//! TUI application state

use std::collections::HashMap;

use legion_core::orchestrate::{OrchestrateEngine, WorkerState};
use legion_db::Provider;

use crate::pty::{PtyHandle, SharedParser};

/// Application mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    /// Normal mode - keys go to PTY
    Normal,
    /// Popup menu mode - keys navigate menu
    Popup(PopupMenu),
}

/// Popup menu types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupMenu {
    Main,
    Provider,
    Model,
    Matrix,
}

/// Which column is active in the matrix view
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixCol {
    Provider,
    Model,
}

/// Target for provider/model assignment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTarget {
    Pane(usize),
    AllWorkers,
    AllPanes,
}

/// Main menu items
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainMenuItem {
    Config,
    Quit,
}

impl MainMenuItem {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Config => "Config",
            Self::Quit => "Quit",
        }
    }
}

/// A single pane in the TUI - each runs its own Claude Code instance
pub struct Pane {
    pub pty: Option<PtyHandle>,
    pub proxy_port: u16,
    pub control_port: u16,
    pub label: String,
    pub current_provider: Option<usize>,
    pub current_model: Option<String>,
}

/// Full TUI application state
pub struct App {
    pub mode: AppMode,
    pub should_quit: bool,

    // Provider/model state
    pub providers: Vec<Provider>,
    pub current_provider: Option<usize>,
    pub current_model: Option<String>,
    pub provider_connected: bool,

    // Menu navigation
    pub menu_index: usize,
    pub submenu_index: usize,

    // Matrix navigation
    pub matrix_row: usize,
    pub matrix_col: MatrixCol,
    pub model_target: Option<ModelTarget>,

    // Panes (replaces single pty + ports)
    pub panes: Vec<Pane>,
    pub focused_pane: usize,

    // Layout
    pub leader_ratio: u16,        // leader width percentage (20-80), default 65
    pub dragging_divider: bool,   // mouse drag state
    pub hover_on_divider: bool,   // mouse hovering near divider
    pub term_size: (u16, u16),    // cached (width, height) for resize after ratio change

    // Orchestration
    pub orchestrate: Option<OrchestrateEngine>,
    pub orchestrate_snapshot: Option<Vec<WorkerState>>,
    pub show_dashboard: bool,

    // Saved per-pane configs (label → (provider_id, model))
    saved_pane_configs: HashMap<String, (String, Option<String>)>,
}

impl App {
    pub fn new() -> Self {
        Self {
            mode: AppMode::Normal,
            should_quit: false,
            providers: Vec::new(),
            current_provider: None,
            current_model: None,
            provider_connected: false,
            menu_index: 0,
            submenu_index: 0,
            matrix_row: 0,
            matrix_col: MatrixCol::Provider,
            model_target: None,
            panes: Vec::new(),
            focused_pane: 0,
            leader_ratio: 65,
            dragging_divider: false,
            hover_on_divider: false,
            term_size: (0, 0),
            orchestrate: None,
            orchestrate_snapshot: None,
            show_dashboard: false,
            saved_pane_configs: HashMap::new(),
        }
    }

    /// Add a pane, spawning a Claude Code PTY inside it
    pub fn add_pane(
        &mut self,
        rows: u16,
        cols: u16,
        proxy_port: u16,
        control_port: u16,
        label: String,
        dangerously_skip_permissions: bool,
        worker_id: Option<u16>,
        orchestrate_port: Option<u16>,
        system_prompt: Option<&str>,
        working_dir: Option<&std::path::Path>,
        continue_session: bool,
    ) {
        let use_proxy = self.pane_uses_proxy(&label);
        let pty = match PtyHandle::spawn(rows, cols, proxy_port, control_port, dangerously_skip_permissions, worker_id, orchestrate_port, system_prompt, use_proxy, working_dir, continue_session) {
            Ok(handle) => Some(handle),
            Err(e) => {
                tracing::error!("Failed to spawn Claude for pane '{}': {}", label, e);
                None
            }
        };
        // Check for saved per-pane config
        let (pane_provider, pane_model) = if let Some((saved_pid, saved_model)) = self.saved_pane_configs.get(&label) {
            let provider_idx = self.providers.iter().position(|p| p.id == *saved_pid);
            if provider_idx.is_some() {
                (provider_idx, saved_model.clone())
            } else {
                (self.current_provider, self.current_model.clone())
            }
        } else {
            (self.current_provider, self.current_model.clone())
        };

        self.panes.push(Pane {
            pty,
            proxy_port,
            control_port,
            label,
            current_provider: pane_provider,
            current_model: pane_model,
        });
    }

    /// Kill all PTY child processes (called on exit)
    pub fn kill_all(&mut self) {
        for pane in &mut self.panes {
            if let Some(ref mut pty) = pane.pty {
                pty.kill();
            }
        }
    }

    /// Whether we're in squad (multi-pane) mode
    pub fn is_squad(&self) -> bool {
        self.panes.len() > 1
    }

    /// Get shared parser ref for rendering the focused pane
    pub fn parser(&self) -> Option<&SharedParser> {
        self.panes.get(self.focused_pane)
            .and_then(|pane| pane.pty.as_ref())
            .map(|pty| &pty.parser)
    }

    /// Get shared parser ref for a specific pane
    pub fn parser_at(&self, index: usize) -> Option<&SharedParser> {
        self.panes.get(index)
            .and_then(|pane| pane.pty.as_ref())
            .map(|pty| &pty.parser)
    }

    /// Send bytes to the focused pane's PTY
    pub fn write_to_pty(&mut self, data: &[u8]) {
        if let Some(pane) = self.panes.get_mut(self.focused_pane) {
            if let Some(ref mut pty) = pane.pty {
                let _ = pty.write(data);
            }
        }
    }

    /// Write bytes to a specific pane's PTY (for task injection)
    pub fn write_to_pane(&mut self, pane_index: usize, data: &[u8]) {
        if let Some(pane) = self.panes.get_mut(pane_index) {
            if let Some(ref mut pty) = pane.pty {
                let _ = pty.write(data);
            }
        }
    }

    /// Get orchestration status for a Worker pane
    pub fn worker_task_status(&self, pane_index: usize) -> Option<&WorkerState> {
        if pane_index == 0 || !self.is_squad() {
            return None;
        }
        let worker_id = pane_index as u16;
        self.orchestrate_snapshot.as_ref()
            .and_then(|snap| snap.iter().find(|w| w.worker_id == worker_id))
    }

    /// Get the control port of the focused pane
    pub fn focused_control_port(&self) -> u16 {
        self.panes.get(self.focused_pane)
            .map(|p| p.control_port)
            .unwrap_or(0)
    }

    /// Cycle focus to next pane
    pub fn focus_next(&mut self) {
        if !self.panes.is_empty() {
            self.focused_pane = (self.focused_pane + 1) % self.panes.len();
        }
    }

    /// Cycle focus to previous pane
    pub fn focus_prev(&mut self) {
        if !self.panes.is_empty() {
            self.focused_pane = if self.focused_pane > 0 {
                self.focused_pane - 1
            } else {
                self.panes.len() - 1
            };
        }
    }

    /// Adjust leader ratio by delta (clamped 20-80), then resize panes
    pub fn adjust_leader_ratio(&mut self, delta: i16) {
        self.leader_ratio = (self.leader_ratio as i16 + delta).clamp(20, 80) as u16;
        self.apply_resize();
    }

    /// Set leader ratio from absolute mouse x position
    pub fn set_leader_ratio_from_x(&mut self, x: u16) {
        let (w, _) = self.term_size;
        if w > 0 {
            self.leader_ratio = ((x as u32 * 100) / w as u32).clamp(20, 80) as u16;
        }
    }

    /// Re-apply resize using cached terminal size (after ratio change)
    pub fn apply_resize(&mut self) {
        let (w, h) = self.term_size;
        if w > 0 && h > 0 {
            self.resize_panes(w, h);
        }
    }

    /// Resize all panes to match new terminal dimensions
    pub fn resize_panes(&mut self, term_width: u16, term_height: u16) {
        self.term_size = (term_width, term_height);
        let content_height = term_height.saturating_sub(2); // header + footer

        if self.is_squad() {
            let worker_count = (self.panes.len() - 1) as u16;

            // Leader: leader_ratio% width, full content height
            let leader_width = (term_width as u32 * self.leader_ratio as u32 / 100) as u16;
            let leader_rows = content_height.saturating_sub(2);
            let leader_cols = leader_width.saturating_sub(2);
            if let Some(pane) = self.panes.get_mut(0) {
                if let Some(ref mut pty) = pane.pty {
                    let _ = pty.resize(leader_rows, leader_cols);
                }
            }

            // Workers: remaining width minus 1 for divider column, vertically split
            let worker_width = term_width.saturating_sub(leader_width).saturating_sub(1);
            let worker_height = if worker_count > 0 { content_height / worker_count } else { 0 };
            let worker_rows = worker_height.saturating_sub(2);
            let worker_cols = worker_width.saturating_sub(2);
            for i in 1..self.panes.len() {
                if let Some(ref mut pty) = self.panes[i].pty {
                    let _ = pty.resize(worker_rows, worker_cols);
                }
            }
        } else {
            // Single pane: full width minus border
            let rows = content_height.saturating_sub(2);
            let cols = term_width.saturating_sub(2);
            if let Some(pane) = self.panes.get_mut(0) {
                if let Some(ref mut pty) = pane.pty {
                    let _ = pty.resize(rows, cols);
                }
            }
        }
    }

    /// Load providers from database, prepend "Native" option
    pub fn load_from_db(&mut self) {
        // "Default" provider: no proxy, Claude Code uses its own native auth (OAuth for Claude Max)
        let default_provider = Provider {
            id: "__default__".to_string(),
            name: "Default".to_string(),
            base_url: String::new(),
            api_key: None,
            api_format: "anthropic".to_string(),
            models: None,
            is_default: false,
            created_at: 0,
        };
        self.providers = vec![default_provider];

        if let Ok(repo) = legion_db::open_db() {
            if let Ok(mut providers) = repo.list_providers() {
                self.providers.append(&mut providers);
            }
            if let Ok(Some(default)) = repo.get_default_provider() {
                self.current_provider =
                    self.providers.iter().position(|p| p.id == default.id);
                self.current_model =
                    default.models.as_ref().and_then(|m| m.first().cloned());
                self.provider_connected = true;
            }
            // Load saved per-pane configs
            if let Ok(pane_configs) = repo.list_pane_configs() {
                for pc in pane_configs {
                    self.saved_pane_configs.insert(
                        pc.pane_label,
                        (pc.provider_id, pc.model),
                    );
                }
            }
        }

        // Default to Native (index 0) if no default provider set
        if self.current_provider.is_none() {
            self.current_provider = Some(0);
            self.provider_connected = true;
        }
    }

    /// Get saved pane config for a given label (provider_id, model)
    pub fn get_saved_pane_config(&self, label: &str) -> Option<&(String, Option<String>)> {
        self.saved_pane_configs.get(label)
    }

    /// Check if a pane (by label) should use the proxy, or Default (no proxy) mode
    pub fn pane_uses_proxy(&self, label: &str) -> bool {
        match self.saved_pane_configs.get(label) {
            Some((pid, _)) => pid != "__default__",
            None => {
                // No saved config — use app-level default
                self.current_provider
                    .and_then(|i| self.providers.get(i))
                    .map(|p| p.id != "__default__")
                    .unwrap_or(false)
            }
        }
    }

    // --- Menu navigation ---

    pub fn main_menu_items() -> &'static [MainMenuItem] {
        &[MainMenuItem::Config, MainMenuItem::Quit]
    }

    pub fn toggle_popup(&mut self) {
        match self.mode {
            AppMode::Normal => {
                self.mode = AppMode::Popup(PopupMenu::Matrix);
                self.matrix_row = 0;
                self.matrix_col = MatrixCol::Provider;
            }
            AppMode::Popup(_) => {
                self.mode = AppMode::Normal;
            }
        }
    }

    pub fn enter_submenu(&mut self) {
        if let AppMode::Popup(PopupMenu::Main) = self.mode {
            let items = Self::main_menu_items();
            if self.menu_index < items.len() {
                match items[self.menu_index] {
                    MainMenuItem::Config => {
                        self.mode = AppMode::Popup(PopupMenu::Matrix);
                        self.matrix_row = 0;
                        self.matrix_col = MatrixCol::Provider;
                    }
                    MainMenuItem::Quit => {
                        self.should_quit = true;
                    }
                }
            }
        }
    }

    pub fn select_submenu_item(&mut self) {
        match self.mode {
            AppMode::Popup(PopupMenu::Provider) => {
                if self.submenu_index < self.providers.len() {
                    let first_model = self.providers.get(self.submenu_index)
                        .and_then(|p| p.models.as_ref())
                        .and_then(|m| m.first().cloned());

                    match self.model_target {
                        Some(ModelTarget::Pane(i)) => {
                            if let Some(pane) = self.panes.get_mut(i) {
                                pane.current_provider = Some(self.submenu_index);
                                pane.current_model = first_model;
                            }
                        }
                        Some(ModelTarget::AllWorkers) => {
                            for pane in self.panes.iter_mut().skip(1) {
                                pane.current_provider = Some(self.submenu_index);
                                pane.current_model = first_model.clone();
                            }
                        }
                        Some(ModelTarget::AllPanes) | None => {
                            self.current_provider = Some(self.submenu_index);
                            self.current_model = first_model.clone();
                            for pane in self.panes.iter_mut() {
                                pane.current_provider = Some(self.submenu_index);
                                pane.current_model = first_model.clone();
                            }
                        }
                    }
                    self.provider_connected = true;
                }
                if self.model_target.is_some() {
                    self.back_to_matrix();
                } else {
                    self.mode = AppMode::Popup(PopupMenu::Main);
                }
            }
            AppMode::Popup(PopupMenu::Model) => {
                let model_name = self.target_provider_models()
                    .and_then(|models| models.get(self.submenu_index).cloned());

                if let Some(model) = model_name {
                    match self.model_target {
                        Some(ModelTarget::Pane(i)) => {
                            if let Some(pane) = self.panes.get_mut(i) {
                                pane.current_model = Some(model);
                            }
                        }
                        Some(ModelTarget::AllWorkers) => {
                            for pane in self.panes.iter_mut().skip(1) {
                                pane.current_model = Some(model.clone());
                            }
                        }
                        Some(ModelTarget::AllPanes) | None => {
                            self.current_model = Some(model.clone());
                            for pane in self.panes.iter_mut() {
                                pane.current_model = Some(model.clone());
                            }
                        }
                    }
                }
                if self.model_target.is_some() {
                    self.back_to_matrix();
                } else {
                    self.mode = AppMode::Popup(PopupMenu::Main);
                }
            }
            _ => {}
        }
    }

    pub fn back_to_main_menu(&mut self) {
        self.mode = AppMode::Popup(PopupMenu::Main);
    }

    pub fn menu_up(&mut self) {
        let len = match self.mode {
            AppMode::Popup(PopupMenu::Main) => Self::main_menu_items().len(),
            AppMode::Popup(PopupMenu::Provider) => self.providers.len(),
            AppMode::Popup(PopupMenu::Model) => {
                self.target_provider_models().map(|m| m.len()).unwrap_or(0)
            }
            AppMode::Popup(PopupMenu::Matrix) => self.matrix_row_count(),
            _ => return,
        };
        let idx = match self.mode {
            AppMode::Popup(PopupMenu::Main) => &mut self.menu_index,
            AppMode::Popup(PopupMenu::Matrix) => &mut self.matrix_row,
            _ => &mut self.submenu_index,
        };
        *idx = if *idx > 0 { *idx - 1 } else { len.saturating_sub(1) };
    }

    pub fn menu_down(&mut self) {
        let len = match self.mode {
            AppMode::Popup(PopupMenu::Main) => Self::main_menu_items().len(),
            AppMode::Popup(PopupMenu::Provider) => self.providers.len(),
            AppMode::Popup(PopupMenu::Model) => {
                self.target_provider_models().map(|m| m.len()).unwrap_or(0)
            }
            AppMode::Popup(PopupMenu::Matrix) => self.matrix_row_count(),
            _ => return,
        };
        let idx = match self.mode {
            AppMode::Popup(PopupMenu::Main) => &mut self.menu_index,
            AppMode::Popup(PopupMenu::Matrix) => &mut self.matrix_row,
            _ => &mut self.submenu_index,
        };
        *idx = if *idx < len.saturating_sub(1) { *idx + 1 } else { 0 };
    }

    pub fn get_current_provider(&self) -> Option<&Provider> {
        self.current_provider.and_then(|i| self.providers.get(i))
    }

    pub fn get_current_provider_models(&self) -> Option<&Vec<String>> {
        self.get_current_provider().and_then(|p| p.models.as_ref())
    }

    // --- Matrix navigation ---

    /// Total selectable rows in matrix: panes + batch options (squad only)
    pub fn matrix_row_count(&self) -> usize {
        if self.is_squad() {
            self.panes.len() + 2
        } else {
            self.panes.len()
        }
    }

    /// Convert current matrix_row to a ModelTarget
    pub fn matrix_target(&self) -> ModelTarget {
        let pane_count = self.panes.len();
        if self.matrix_row < pane_count {
            ModelTarget::Pane(self.matrix_row)
        } else if self.matrix_row == pane_count {
            ModelTarget::AllWorkers
        } else {
            ModelTarget::AllPanes
        }
    }

    /// From the matrix, open the provider or model picker for the current cell
    pub fn matrix_enter(&mut self) {
        let target = self.matrix_target();
        self.model_target = Some(target);
        match self.matrix_col {
            MatrixCol::Provider => {
                self.mode = AppMode::Popup(PopupMenu::Provider);
                self.submenu_index = match target {
                    ModelTarget::Pane(i) => self.panes.get(i)
                        .and_then(|p| p.current_provider)
                        .unwrap_or(0),
                    _ => self.current_provider.unwrap_or(0),
                };
            }
            MatrixCol::Model => {
                self.mode = AppMode::Popup(PopupMenu::Model);
                self.submenu_index = match target {
                    ModelTarget::Pane(i) => {
                        let pane_model = self.panes.get(i).and_then(|p| p.current_model.as_ref());
                        let pane_provider = self.panes.get(i).and_then(|p| p.current_provider);
                        pane_provider
                            .and_then(|pi| self.providers.get(pi))
                            .and_then(|p| p.models.as_ref())
                            .and_then(|models| {
                                pane_model.and_then(|m| models.iter().position(|x| x == m))
                            })
                            .unwrap_or(0)
                    }
                    _ => {
                        self.current_provider
                            .and_then(|pi| self.providers.get(pi))
                            .and_then(|p| p.models.as_ref())
                            .and_then(|models| {
                                self.current_model
                                    .as_ref()
                                    .and_then(|m| models.iter().position(|x| x == m))
                            })
                            .unwrap_or(0)
                    }
                };
            }
        }
    }

    /// Return from Provider/Model sub-menu back to the matrix view
    pub fn back_to_matrix(&mut self) {
        self.model_target = None;
        self.mode = AppMode::Popup(PopupMenu::Matrix);
    }

    /// Toggle the active column in the matrix view
    pub fn matrix_col_toggle(&mut self) {
        self.matrix_col = match self.matrix_col {
            MatrixCol::Provider => MatrixCol::Model,
            MatrixCol::Model => MatrixCol::Provider,
        };
    }

    /// Get models for the current target's provider
    pub fn target_provider_models(&self) -> Option<&Vec<String>> {
        let provider_idx = match self.model_target {
            Some(ModelTarget::Pane(i)) => self.panes.get(i).and_then(|p| p.current_provider),
            _ => self.current_provider,
        };
        provider_idx
            .and_then(|i| self.providers.get(i))
            .and_then(|p| p.models.as_ref())
    }

    /// Get the target label for sub-menu titles
    pub fn target_label(&self) -> &str {
        match self.model_target {
            Some(ModelTarget::Pane(i)) => self.panes.get(i)
                .map(|p| p.label.as_str())
                .unwrap_or("Pane"),
            Some(ModelTarget::AllWorkers) => "All Workers",
            Some(ModelTarget::AllPanes) => "All Panes",
            None => "All",
        }
    }
}
