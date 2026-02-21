//! TUI application state

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use legion_core::orchestrate::OrchestrateEngine;
use legion_db::{Provider, Role, SquadSession, Team};

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
    SessionList,
    CompleteSession,
    NewSessionInput,
    RemoveWorkerList,
    RemoveWorkerConfirm,
    ConnectProvider,
    ProviderApiKeyInput,
    MaxRetries,
    RetryForm,
    DeleteConfirm,
    ClearConfirm,
    SessionDeleteConfirm,
    CompleteRecordChoice,
    FileDiff,
    BranchRecovery,
    BranchList,
    BranchChanged,
    CopilotAuth,
    SetWorkerCount,
    ManageTeams,
    TeamDetail,
    TeamForm,
    RoleList,
    RoleForm,
    AddRoleToTeam,
    BoardDetail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopilotAuthStatus {
    RequestingCode,
    WaitingForAuth,
    Exchanging,
    Success,
    Error,
}

pub enum CopilotAuthMsg {
    DeviceCode { user_code: String, verification_uri: String },
    Authorized,
    SetupComplete { models: Vec<String> },
    Error(String),
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

/// Target for delete confirmation in teams/roles screens
#[derive(Debug, Clone)]
pub enum DeleteTarget {
    Team(String),             // team id
    Role(String),             // role id
    TeamRole(String, String), // (team_id, role_id)
}

/// Main menu items
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainMenuItem {
    SwitchModels,
    ConnectProvider,
    MaxRetries,
    SetWorkers,
    RemoveWorker,
    ManageTeams,
    ManageRoles,
    SwitchSession,
    SwitchBranch,
    CompleteSession,
    Quit,
}

impl MainMenuItem {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SwitchModels => "Switch Models",
            Self::ConnectProvider => "Connect Provider",
            Self::MaxRetries => "Max Retries",
            Self::SetWorkers => "Set Workers",
            Self::RemoveWorker => "Remove Worker",
            Self::ManageTeams => "Manage Teams",
            Self::ManageRoles => "Manage Roles",
            Self::SwitchSession => "Switch Session",
            Self::SwitchBranch => "Switch Branch",
            Self::CompleteSession => "Complete Session",
            Self::Quit => "Quit",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::SwitchModels => "Change AI model for leader or worker panes",
            Self::ConnectProvider => "Add API provider (Anthropic, OpenAI, etc.)",
            Self::MaxRetries => "Set max retry attempts for failed tickets",
            Self::SetWorkers => "Scale worker count from 1 to 8",
            Self::RemoveWorker => "Remove a specific worker from the squad",
            Self::ManageTeams => "View team presets and role compositions",
            Self::ManageRoles => "Create, edit, and delete custom roles",
            Self::SwitchSession => "Switch to a different squad session",
            Self::SwitchBranch => "Change the git branch for this session",
            Self::CompleteSession => "Mark current session as complete",
            Self::Quit => "Exit Legion",
        }
    }
}

/// Predefined provider templates
#[derive(Debug, Clone)]
pub struct ProviderTemplate {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub api_format: &'static str,
    pub models: &'static [&'static str],
    pub env_var: &'static str,
    pub auth_method: &'static str, // "api_key" or "device_flow"
}

pub const PROVIDER_TEMPLATES: &[ProviderTemplate] = &[
    ProviderTemplate {
        id: "__default__",
        name: "Native",
        base_url: "",
        api_format: "anthropic",
        models: &["claude-opus-4-6", "claude-sonnet-4-5-20250929", "claude-haiku-4-5-20251001"],
        env_var: "",
        auth_method: "none",
    },
    ProviderTemplate {
        id: "github_copilot",
        name: "GitHub Copilot",
        base_url: "https://api.githubcopilot.com",
        api_format: "github_copilot",
        models: &["claude-sonnet-4-5-20250929", "claude-opus-4-6", "gpt-4o", "gpt-5.2-codex"],
        env_var: "GITHUB_TOKEN",
        auth_method: "device_flow",
    },
    ProviderTemplate {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        api_format: "openai_chat",
        models: &["anthropic/claude-opus-4-6", "openai/gpt-4o", "google/gemini-2.5-pro"],
        env_var: "OPENROUTER_API_KEY",
        auth_method: "api_key",
    },
    ProviderTemplate {
        id: "minimax",
        name: "MiniMax",
        base_url: "https://api.minimax.io/v1",
        api_format: "openai_chat",
        models: &["MiniMax-M2.5", "MiniMax-M2.5-highspeed", "MiniMax-M2.1", "MiniMax-M2"],
        env_var: "MINIMAX_API_KEY",
        auth_method: "api_key",
    },
];

/// Maximum number of workers (not including leader)
pub const MAX_WORKERS: u16 = 8;

/// Worker timeout: kill SDK process if no output for this many seconds
pub const WORKER_TIMEOUT_SECS: u64 = 300;
/// Scrollback buffer size for vt100 parser (number of lines kept in history).
pub const SCROLLBACK_LINES: usize = 100_000;
/// Max scroll input offset to prevent unbounded accumulation from fast trackpad scrolling.
pub const MAX_SCROLL_OFFSET: usize = 100_000;

/// A single pane in the TUI - each runs its own Claude Code instance
pub struct Pane {
    pub pty: Option<PtyHandle>,
    pub proxy_port: u16,
    pub control_port: u16,
    pub label: String,
    pub current_provider: Option<usize>,
    pub current_model: Option<String>,
    /// If true, pane was spawned with --continue; monitor for fallback
    pub spawned_with_continue: bool,

    // SDK execution state (workers only)
    pub sdk_task: Option<crate::sdk::SdkHandle>,
    pub sdk_parser: Option<crate::pty::SharedParser>,
    pub sdk_entries: Vec<crate::sdk::ProgressEntry>,
    pub current_ticket_id: Option<usize>,
    /// Full log buffer for this pane's SDK execution (all formatted output lines)
    pub sdk_log_buffer: Option<std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
    /// Last time SDK produced output (for timeout detection)
    pub last_sdk_activity: Option<Instant>,
    /// Scrollback offset (0 = normal view, >0 = scrolled up)
    pub scroll_offset: usize,
}

impl Pane {
    /// Get the parser for this pane (prefer SDK parser for workers, fallback to PTY for leader)
    pub fn parser(&self) -> Option<&SharedParser> {
        self.sdk_parser.as_ref().or_else(|| self.pty.as_ref().map(|pty| &pty.parser))
    }
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

    // Right panel (embedded task board)
    pub right_panel_focused: bool,
    pub ticket_snapshot: Option<Vec<legion_core::TicketSnapshot>>,
    pub queue_stats: Option<(usize, usize, usize, usize, usize)>,

    // Session management
    pub current_session: Option<SquadSession>,
    pub project_path: Option<PathBuf>,

    // Session list state
    pub session_list: Vec<SquadSession>,
    pub session_list_index: usize,
    pub complete_merge_index: usize,
    pub session_branch_status: std::collections::HashMap<String, bool>, // session_name → branch_exists

    // Session delete/complete state
    pub session_delete_target: Option<String>,
    pub session_delete_pending_count: usize,
    pub session_delete_ticket_count: usize,
    pub session_delete_log_count: usize,
    pub complete_record_choice: usize,
    pub complete_session_name: Option<String>,
    pub creating_default_session: bool,

    // Branch detection at startup
    pub detected_branch: Option<String>,
    pub detected_commit: Option<String>,

    // Session error message (shown in NewSessionInput popup)
    pub session_error: Option<String>,

    // Deferred session spawning (for in-TUI session selection)
    pub session_name_input: String,
    pub base_port: u16,
    pub requested_workers: u16,
    pub pending_orchestrate_port: Option<u16>,

    // Dynamic worker management
    pub pending_worker_proxies: Vec<(u16, u16, String)>,  // (proxy_port, control_port, label) queued by start_session
    pub pending_set_worker_count: Option<u16>,  // target worker count to apply
    pub set_worker_count_selection: u16,         // currently selected number in popup
    pub pending_sync_max_iterations: bool,
    pub pending_remove_worker: Option<(usize, String)>, // (pane index, strategy: "merge"/"keep"/"discard")
    pub next_worker_id: u16,
    pub remove_worker_target: usize,
    pub remove_worker_confirming: bool,
    pub remove_worker_strategy_index: usize,

    // Saved per-pane configs (label → (provider_id, model))
    saved_pane_configs: HashMap<String, (String, Option<String>)>,

    // Board state (squad task board)
    pub board_selected: usize,          // selected ticket id in board view
    pub board_detail_scroll: usize,     // scroll offset in detail popup

    // Per-ticket log buffers (ticket_id → log lines)
    pub ticket_logs: HashMap<usize, std::sync::Arc<std::sync::Mutex<Vec<String>>>>,

    // Per-ticket team activity timeline (ticket_id → activities)
    pub ticket_team_activities: HashMap<usize, Vec<crate::sdk::TeamActivity>>,

    // Connect Provider state
    pub connect_provider_index: usize,       // selected template index
    pub api_key_input: String,               // text input buffer for API key
    pub default_max_iterations: u16,         // default retry count for new tickets

    // Retry form state
    pub retry_target_id: usize,
    pub retry_form_fields: [String; 4],      // [prompt, context, criteria, feedback]
    pub retry_form_focus: u8,                // 0-3 for which field is focused
    pub retry_error_summary: String,         // ticket error summary for display
    pub retry_form_scroll: u16,              // scroll offset for long content

    // Delete confirm state
    pub delete_confirm_id: usize,

    // Leader context status (from statusLine hook)
    pub leader_context_pct: Option<u8>,
    pub leader_git_branch: Option<String>,
    pub leader_output_style: Option<String>,

    // Branch recovery dialog state
    pub recovery_session: Option<SquadSession>,  // session being recovered
    pub recovery_choice: usize,                   // 0-3 selected option
    pub branch_list: Vec<String>,                 // cached local branches
    pub branch_list_index: usize,                 // selected branch in list
    pub new_session_branch_index: usize,          // selected branch index in new session popup
    pub new_session_branch_focused: bool,         // true = branch selector focused, false = name input

    // Runtime branch change detection
    pub last_branch_check: Option<std::time::Instant>,
    pub branch_changed_to: Option<String>,

    // File diff popup state
    pub diff_ticket_id: usize,
    pub diff_data: Option<crate::diff::DiffData>,
    pub diff_file_selected: usize,
    pub diff_scroll: usize,
    pub diff_loading: bool,
    pub diff_error: Option<String>,

    // Orchestrate API shutdown handle
    pub orchestrate_shutdown: Option<tokio::sync::oneshot::Sender<()>>,

    // Copilot device auth flow state
    pub copilot_auth_status: CopilotAuthStatus,
    pub copilot_auth_rx: Option<tokio::sync::mpsc::UnboundedReceiver<CopilotAuthMsg>>,
    pub copilot_user_code: Option<String>,
    pub copilot_verification_uri: Option<String>,
    pub copilot_auth_error: Option<String>,
    pub copilot_models_result: Option<Vec<String>>,

    // Manage Teams state
    pub team_list: Vec<Team>,
    pub team_list_index: usize,
    pub team_detail_team: Option<Team>,
    pub team_detail_roles: Vec<Role>,
    pub team_detail_index: usize,

    // Team form (new/edit)
    pub team_form_fields: [String; 3],    // [name, description, team_prompt]
    pub team_form_focus: u8,              // 0=name, 1=description, 2=team_prompt, 3=role selection
    pub team_form_cursor: usize,          // cursor position within current field (char index)
    pub team_form_editing: Option<String>, // Some(id)=editing existing, None=new
    pub team_form_clone_roles: Vec<String>, // role_ids to copy when cloning a builtin team
    pub team_form_role_list: Vec<Role>,       // all available roles for selection
    pub team_form_role_selections: Vec<bool>, // parallel to team_form_role_list
    pub team_form_role_scroll: usize,         // currently highlighted index in role list

    // Role list
    pub role_list: Vec<Role>,
    pub role_list_index: usize,

    // Role form (new/edit)
    pub role_form_fields: [String; 3],    // [name, description, prompt_template]
    pub role_form_focus: u8,              // 0-2
    pub role_form_cursor: usize,          // cursor position within current field (char index)
    pub role_form_editing: Option<String>, // Some(id)=editing, None=new
    pub role_form_clone_source: Option<String>, // Some(builtin_id) when cloning a builtin role

    // Add role to team
    pub add_role_available: Vec<Role>,    // roles not yet in current team
    pub add_role_index: usize,
    pub add_role_selections: Vec<bool>,  // parallel to add_role_available

    // Delete confirmation (inline footer)
    pub confirm_delete: Option<(String, DeleteTarget)>, // (display_name, target)

    // Project-local DB path (set when project_path is known)
    pub project_db_path: Option<PathBuf>,
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
            right_panel_focused: false,
            ticket_snapshot: None,
            queue_stats: None,
            current_session: None,
            project_path: None,
            session_list: Vec::new(),
            session_list_index: 0,
            complete_merge_index: 0,
            session_branch_status: std::collections::HashMap::new(),
            session_delete_target: None,
            session_delete_pending_count: 0,
            session_delete_ticket_count: 0,
            session_delete_log_count: 0,
            complete_record_choice: 0,
            complete_session_name: None,
            creating_default_session: false,
            detected_branch: None,
            detected_commit: None,
            session_error: None,
            session_name_input: String::new(),
            base_port: 0,
            requested_workers: 0,
            pending_orchestrate_port: None,
            pending_worker_proxies: Vec::new(),
            pending_set_worker_count: None,
            set_worker_count_selection: 2,
            pending_sync_max_iterations: false,
            pending_remove_worker: None,
            next_worker_id: 1,
            remove_worker_target: 0,
            remove_worker_confirming: false,
            remove_worker_strategy_index: 0,
            saved_pane_configs: HashMap::new(),
            board_selected: 0,
            // board_detail is now AppMode::Popup(PopupMenu::BoardDetail)
            board_detail_scroll: 0,
            ticket_logs: HashMap::new(),
            ticket_team_activities: HashMap::new(),
            connect_provider_index: 0,
            api_key_input: String::new(),
            default_max_iterations: 5,
            retry_target_id: 0,
            retry_form_fields: [String::new(), String::new(), String::new(), String::new()],
            retry_form_focus: 0,
            retry_error_summary: String::new(),
            retry_form_scroll: 0,
            delete_confirm_id: 0,
            leader_context_pct: None,
            leader_git_branch: None,
            leader_output_style: None,
            recovery_session: None,
            recovery_choice: 0,
            branch_list: Vec::new(),
            branch_list_index: 0,
            new_session_branch_index: 0,
            new_session_branch_focused: false,
            last_branch_check: None,
            branch_changed_to: None,
            diff_ticket_id: 0,
            diff_data: None,
            diff_file_selected: 0,
            diff_scroll: 0,
            diff_loading: false,
            diff_error: None,
            orchestrate_shutdown: None,
            copilot_auth_status: CopilotAuthStatus::RequestingCode,
            copilot_auth_rx: None,
            copilot_user_code: None,
            copilot_verification_uri: None,
            copilot_auth_error: None,
            copilot_models_result: None,
            team_list: Vec::new(),
            team_list_index: 0,
            team_detail_team: None,
            team_detail_roles: Vec::new(),
            team_detail_index: 0,
            team_form_fields: [String::new(), String::new(), String::new()],
            team_form_focus: 0,
            team_form_cursor: 0,
            team_form_editing: None,
            team_form_clone_roles: Vec::new(),
            team_form_role_list: Vec::new(),
            team_form_role_selections: Vec::new(),
            team_form_role_scroll: 0,
            role_list: Vec::new(),
            role_list_index: 0,
            role_form_fields: [String::new(), String::new(), String::new()],
            role_form_focus: 0,
            role_form_cursor: 0,
            role_form_editing: None,
            role_form_clone_source: None,
            add_role_available: Vec::new(),
            add_role_index: 0,
            add_role_selections: Vec::new(),
            confirm_delete: None,
            project_db_path: None,
        }
    }

    /// Open the project-local database (squad_sessions, tickets, etc.)
    pub fn open_project_db(&self) -> Option<legion_db::Repository> {
        self.project_db_path.as_ref()
            .and_then(|p| legion_db::open_project_db(p).ok())
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
            spawned_with_continue: continue_session,
            sdk_task: None,
            sdk_parser: None,
            sdk_entries: Vec::new(),
            current_ticket_id: None,
            sdk_log_buffer: None,
            last_sdk_activity: None,
            scroll_offset: 0,
        });
    }

    /// Kill all PTY and SDK child processes (called on exit)
    pub fn kill_all(&mut self) {
        for pane in &mut self.panes {
            if let Some(ref mut pty) = pane.pty {
                pty.kill();
            }
            if let Some(ref mut sdk) = pane.sdk_task {
                sdk.kill();
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
            .and_then(|pane| pane.parser())
    }

    /// Get shared parser ref for a specific pane
    pub fn parser_at(&self, index: usize) -> Option<&SharedParser> {
        self.panes.get(index)
            .and_then(|pane| pane.parser())
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
            // Leader: leader_ratio% width, full content height
            // Workers are SDK-based (no PTY), so only the leader pane needs resize.
            let leader_width = (term_width as u32 * self.leader_ratio as u32 / 100) as u16;
            let leader_rows = content_height.saturating_sub(2);
            let leader_cols = leader_width.saturating_sub(2);
            if let Some(pane) = self.panes.get_mut(0) {
                if let Some(ref mut pty) = pane.pty {
                    let _ = pty.resize(leader_rows, leader_cols);
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

    /// Save a provider from template + API key to DB, then reload providers
    pub fn save_provider_from_template(&mut self, template_index: usize, api_key: &str) {
        if template_index >= PROVIDER_TEMPLATES.len() { return; }
        let tmpl = &PROVIDER_TEMPLATES[template_index];
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let provider = Provider {
            id: tmpl.id.to_string(),
            name: tmpl.name.to_string(),
            base_url: tmpl.base_url.to_string(),
            api_key: if api_key.is_empty() { None } else { Some(api_key.to_string()) },
            api_format: tmpl.api_format.to_string(),
            models: Some(tmpl.models.iter().map(|m| m.to_string()).collect()),
            is_default: false,
            created_at: now,
        };
        if let Ok(repo) = legion_db::open_db() {
            let _ = repo.upsert_provider(&provider);
        }
        // Reload providers
        self.load_from_db();
    }

    /// Check if a provider template is already connected (exists in providers list)
    pub fn is_provider_connected(&self, template_id: &str) -> bool {
        self.providers.iter().any(|p| p.id == template_id)
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

    /// Get the current session name for display
    pub fn session_name(&self) -> &str {
        self.current_session
            .as_ref()
            .map(|s| s.name.as_str())
            .unwrap_or("(no session)")
    }

    /// Create a new session: create worktrees, save to DB
    pub fn create_session(&mut self, name: &str, worker_count: u16, is_default: bool) -> anyhow::Result<Vec<PathBuf>> {
        let project_path = self.project_path.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No project path set"))?;

        let paths = if is_default {
            crate::worktree::create_default_session_worktrees(project_path, name, worker_count)?
        } else {
            crate::worktree::create_session_worktrees(project_path, name, worker_count)?
        };

        // Detect branch if not already set (non-startup creation)
        if self.detected_branch.is_none() {
            self.detected_branch = crate::worktree::current_branch(project_path);
            self.detected_commit = crate::worktree::current_commit(project_path);
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let session = SquadSession {
            name: name.to_string(),
            project_path: project_path.to_string_lossy().to_string(),
            worker_count: worker_count as i64,
            status: "active".to_string(),
            created_at: now,
            completed_at: None,
            is_default,
            base_branch: self.detected_branch.clone(),
            base_commit: self.detected_commit.clone(),
            last_active_at: Some(now),
            max_iterations: Some(self.default_max_iterations as i64),
        };

        if let Some(repo) = self.open_project_db() {
            repo.upsert_squad_session(&session)?;
        }

        self.current_session = Some(session);
        Ok(paths)
    }

    /// Load teams from the orchestrate DB for the leader prompt
    fn load_teams_for_leader(&self) -> Vec<(String, String, Vec<String>)> {
        if let Some(ref engine) = self.orchestrate {
            if let Some(db) = engine.db() {
                if let Ok(db_lock) = db.lock() {
                    if let Ok(teams) = db_lock.list_teams() {
                        return teams.into_iter().map(|t| {
                            let role_names: Vec<String> = t.role_ids.iter().filter_map(|rid| {
                                db_lock.get_role(rid).ok().flatten().map(|r| r.name)
                            }).collect();
                            (t.id, t.name, role_names)
                        }).collect();
                    }
                }
            }
        }
        Vec::new()
    }

    /// Get worktree path for a pane in the current session
    pub fn pane_worktree(&self, pane_label: &str) -> Option<PathBuf> {
        let project_path = self.project_path.as_ref()?;
        let session = self.current_session.as_ref()?;
        Some(crate::worktree::pane_worktree_path(project_path, &session.name, pane_label))
    }

    /// Get the default session name based on the git default branch
    pub fn default_session_name_for_default(&self) -> String {
        if let Some(ref project_path) = self.project_path {
            crate::worktree::default_branch(project_path)
        } else {
            "main".to_string()
        }
    }

    /// Phase 1 of session completion: merge branches to git default branch, remove worktrees.
    /// Does NOT take/clear current_session (that happens in phase 2 via complete_session_records).
    /// Returns Ok(false) if no current session.
    pub fn complete_session_merge(&mut self) -> anyhow::Result<bool> {
        let session = match self.current_session.as_ref() {
            Some(s) => s,
            None => return Ok(false),
        };
        let project_path = match self.project_path.as_ref() {
            Some(p) => p.clone(),
            None => return Ok(false),
        };

        let default_branch = crate::worktree::default_branch(&project_path);
        let _ = std::process::Command::new("git")
            .args(["checkout", &default_branch])
            .current_dir(&project_path)
            .output();

        if session.is_default {
            // Default session: only merge worker branches (Leader is on the main repo)
            for i in 1..=session.worker_count {
                let label = format!("Worker {}", i);
                if let Err(e) = crate::worktree::merge_branch(&project_path, &session.name, &label) {
                    tracing::error!("Merge failed for {}: {}", label, e);
                    return Err(e);
                }
            }
            crate::worktree::remove_default_session_worktrees(
                &project_path, &session.name, session.worker_count as u16, false,
            )?;
        } else {
            // Non-default session: merge Leader + all workers
            let pane_labels = std::iter::once("Leader".to_string())
                .chain((1..=session.worker_count).map(|i| format!("Worker {}", i)));

            for label in pane_labels {
                if let Err(e) = crate::worktree::merge_branch(&project_path, &session.name, &label) {
                    tracing::error!("Merge failed for {}: {}", label, e);
                    return Err(e);
                }
            }
            crate::worktree::remove_session_worktrees(
                &project_path, &session.name, session.worker_count as u16, false,
            )?;
        }

        Ok(true)
    }

    /// Phase 2 of session completion: handle ticket records (migrate or delete), mark session completed.
    /// Takes current_session and clears in-memory ticket data.
    pub fn complete_session_records(&mut self, migrate: bool) -> anyhow::Result<()> {
        let session = match self.current_session.take() {
            Some(s) => s,
            None => return Ok(()),
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        if let Some(repo) = self.open_project_db() {
            if migrate {
                let default_session_name = self.default_session_name_for_default();
                repo.migrate_tickets_to_session(&session.name, &default_session_name)?;
            } else {
                repo.delete_session_tickets(&session.name)?;
            }
            repo.complete_squad_session(&session.name, now)?;
        }

        // Clear in-memory ticket data
        self.ticket_logs.clear();
        self.ticket_team_activities.clear();
        self.ticket_snapshot = None;
        self.queue_stats = None;

        Ok(())
    }

    /// Delete a session entirely: remove worktrees, delete all tickets/logs, delete DB record.
    /// Cannot delete a default session.
    pub fn delete_session(&mut self, session_name: &str) -> anyhow::Result<()> {
        let repo = self.open_project_db()
            .ok_or_else(|| anyhow::anyhow!("No project database available"))?;
        let session = repo.get_squad_session(session_name)?
            .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", session_name))?;

        if session.is_default {
            anyhow::bail!("Cannot delete the default session '{}'", session_name);
        }

        // Remove worktrees with force
        if let Some(ref project_path) = self.project_path {
            let _ = crate::worktree::remove_session_worktrees(
                project_path, session_name, session.worker_count as u16, true,
            );
        }

        // Delete all tickets/logs from DB
        repo.delete_session_tickets(session_name)?;

        // Delete squad_session record from DB
        repo.delete_squad_session(session_name)?;

        // If this is the current session, clear it
        if self.current_session.as_ref().map(|s| s.name.as_str()) == Some(session_name) {
            self.current_session = None;
        }

        // Clear in-memory ticket data
        self.ticket_logs.clear();
        self.ticket_team_activities.clear();
        self.ticket_snapshot = None;
        self.queue_stats = None;

        Ok(())
    }

    pub fn load_session_list(&mut self) {
        if let Some(repo) = self.open_project_db() {
            self.session_list = repo.list_squad_sessions().unwrap_or_default();
        }
        // Check branch status for all sessions
        self.session_branch_status.clear();
        if let Some(ref project_path) = self.project_path {
            for session in &self.session_list {
                if let Some(ref branch) = session.base_branch {
                    let exists = crate::worktree::branch_exists(project_path, branch);
                    self.session_branch_status.insert(session.name.clone(), exists);
                }
            }
        }
    }

    /// Generate a default session name like "session-1", "session-2", etc.
    pub fn default_session_name(&self) -> String {
        let mut n = 1u32;
        let existing: std::collections::HashSet<&str> = self.session_list.iter()
            .map(|s| s.name.as_str())
            .collect();
        loop {
            let candidate = format!("session-{}", n);
            if !existing.contains(candidate.as_str()) {
                return candidate;
            }
            n += 1;
        }
    }

    /// Start a session: create/verify worktrees, spawn PTY panes, set up orchestration
    pub fn start_session(&mut self, name: &str, worker_count: u16, is_resume: bool, is_default: bool) -> anyhow::Result<()> {
        // Kill existing panes if switching sessions
        self.kill_all();
        self.panes.clear();
        self.focused_pane = 0;

        let project_path = self.project_path.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No project path"))?
            .clone();

        // Create or verify worktrees
        let worktree_paths = if is_resume {
            // Load session from DB and update last_active_at
            if let Some(repo) = self.open_project_db() {
                self.current_session = repo.get_squad_session(name).ok().flatten();
                let _ = repo.touch_squad_session(name);
            }
            let session_is_default = self.current_session.as_ref().map(|s| s.is_default).unwrap_or(is_default);
            // Restore max_iterations from session config
            if let Some(mi) = self.current_session.as_ref().and_then(|s| s.max_iterations) {
                self.default_max_iterations = mi as u16;
                self.pending_sync_max_iterations = true;
            }

            let mut paths = Vec::new();
            if session_is_default {
                // Default session: Leader path = project_path (no worktree)
                paths.push(project_path.clone());
            } else {
                let wt = crate::worktree::pane_worktree_path(&project_path, name, "Leader");
                if !crate::worktree::worktree_exists(&wt) {
                    let _ = crate::worktree::create_worktree(&project_path, name, "Leader");
                }
                paths.push(wt);
            }
            for i in 1..=worker_count {
                let label = format!("Worker {}", i);
                let wt = crate::worktree::pane_worktree_path(&project_path, name, &label);
                if !crate::worktree::worktree_exists(&wt) {
                    let _ = crate::worktree::create_worktree(&project_path, name, &label);
                }
                paths.push(wt);
            }
            paths
        } else {
            self.create_session(name, worker_count, is_default)?
        };

        // Calculate PTY sizes from cached terminal dimensions
        let (term_width, term_height) = self.term_size;
        let content_height = term_height.saturating_sub(2);
        let leader_width = (term_width as u32 * self.leader_ratio as u32 / 100) as u16;
        let leader_pty_rows = content_height.saturating_sub(2);
        let leader_pty_cols = leader_width.saturating_sub(2);
        // (Worker PTY sizes no longer needed — workers use SDK mode)

        // Generate system prompts (leader only — workers get prompts via SDK dispatch)
        let teams_for_leader = self.load_teams_for_leader();
        let leader_prompt = crate::claudemd::leader_instructions(worker_count, &teams_for_leader);

        // Write /split-tickets command to leader worktree
        if let Err(e) = crate::claudemd::write_leader_commands(&worktree_paths[0], worker_count) {
            tracing::warn!("Failed to write leader commands: {}", e);
        }

        // Port assignments
        let base_port = self.base_port;
        let orchestrate_port = base_port + 2000;

        // Spawn leader pane (pass --continue on resume; fallback handles failure)
        self.add_pane(
            leader_pty_rows, leader_pty_cols, base_port, base_port + 1000,
            "Leader".into(), false, None, Some(orchestrate_port), Some(&leader_prompt),
            Some(worktree_paths[0].as_path()), is_resume,
        );

        // Workers: create panes without PTY (SDK will be used when ticket assigned)
        for i in 0..worker_count {
            let proxy = base_port + i + 1;
            let control = base_port + 1000 + i + 1;
            let label = format!("Worker {}", i + 1);
            // Restore saved per-pane config on restart
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
                pty: None,
                proxy_port: proxy,
                control_port: control,
                label,
                current_provider: pane_provider,
                current_model: pane_model,
                spawned_with_continue: false,
                sdk_task: None,
                sdk_parser: None,
                sdk_entries: Vec::new(),
                current_ticket_id: None,
                sdk_log_buffer: None,
                last_sdk_activity: None,
                scroll_offset: 0,
            });
        }

        // Queue proxy creation for each worker pane (consumed in event loop)
        for i in 0..worker_count {
            let proxy = base_port + i + 1;
            let control = base_port + 1000 + i + 1;
            let label = format!("Worker {}", i + 1);
            self.pending_worker_proxies.push((proxy, control, label));
        }

        // Track next worker ID for dynamic add
        self.next_worker_id = worker_count + 1;

        // Set up orchestration engine with DB persistence
        let session_name = self.current_session.as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "default".to_string());
        let engine = if let Some(repo) = self.open_project_db() {
            let repo = std::sync::Arc::new(std::sync::Mutex::new(repo));
            legion_core::OrchestrateEngine::with_db(worker_count, repo, session_name)
        } else {
            legion_core::OrchestrateEngine::new(worker_count)
        };
        self.orchestrate = Some(engine);
        self.pending_orchestrate_port = Some(orchestrate_port);

        Ok(())
    }

    /// Check if any pane spawned with --continue failed, and respawn without it
    pub fn check_continue_fallback(&mut self) {
        let worker_count = if self.panes.len() > 1 { (self.panes.len() - 1) as u16 } else { 0 };

        // Collect indices that need respawn
        let mut respawn: Vec<usize> = Vec::new();
        for (i, pane) in self.panes.iter().enumerate() {
            if !pane.spawned_with_continue { continue; }
            let has_error = pane.pty.as_ref()
                .and_then(|pty| pty.parser.lock().ok())
                .map(|p| {
                    let screen = p.screen();
                    let (_, cols) = screen.size();
                    screen.rows(0, cols).any(|row| row.contains("No conversation found"))
                })
                .unwrap_or(false);
            if has_error {
                respawn.push(i);
            }
        }

        for i in respawn {
            let label = self.panes[i].label.clone();
            let proxy_port = self.panes[i].proxy_port;
            let control_port = self.panes[i].control_port;
            tracing::info!("Pane '{}': --continue failed, respawning without it", label);

            // Kill existing PTY
            if let Some(ref mut pty) = self.panes[i].pty {
                pty.kill();
            }

            // Calculate PTY size from layout
            let (tw, th) = self.term_size;
            let ch = th.saturating_sub(2);
            let (rows, cols) = if worker_count > 0 {
                if i == 0 {
                    let lw = (tw as u32 * self.leader_ratio as u32 / 100) as u16;
                    (ch.saturating_sub(2), lw.saturating_sub(2))
                } else {
                    let lw = (tw as u32 * self.leader_ratio as u32 / 100) as u16;
                    let ww = tw.saturating_sub(lw).saturating_sub(1);
                    let wh = ch / worker_count;
                    (wh.saturating_sub(2), ww.saturating_sub(2))
                }
            } else {
                (ch.saturating_sub(2), tw.saturating_sub(2))
            };

            // Derive spawn params
            let orchestrate_port = self.base_port + 2000;
            let worker_id: Option<u16> = if i > 0 { Some(i as u16) } else { None };
            let skip_perms = i > 0;
            let use_proxy = self.pane_uses_proxy(&label);
            let working_dir = self.pane_worktree(&label);
            let prompt = if i == 0 {
                let teams_for_leader = self.load_teams_for_leader();
                crate::claudemd::leader_instructions(worker_count, &teams_for_leader)
            } else {
                let wd_str = working_dir.as_ref().map(|p| p.to_string_lossy().to_string());
                crate::claudemd::worker_instructions(i as u16, wd_str.as_deref(), None, None)
            };

            match crate::pty::PtyHandle::spawn(
                rows, cols, proxy_port, control_port,
                skip_perms, worker_id, Some(orchestrate_port),
                Some(&prompt), use_proxy,
                working_dir.as_deref(), false, // no --continue
            ) {
                Ok(handle) => self.panes[i].pty = Some(handle),
                Err(e) => {
                    tracing::error!("Failed to respawn pane '{}': {}", label, e);
                    self.panes[i].pty = None;
                }
            }
            self.panes[i].spawned_with_continue = false;
        }
    }

    // --- Menu navigation ---

    pub fn main_menu_items(&self) -> Vec<MainMenuItem> {
        let mut items = vec![MainMenuItem::SwitchModels, MainMenuItem::ConnectProvider];
        if self.is_squad() {
            items.push(MainMenuItem::MaxRetries);
            items.push(MainMenuItem::SetWorkers);
            let wc = self.panes.len().saturating_sub(1) as u16;
            if wc > 0 {
                items.push(MainMenuItem::RemoveWorker);
            }
            if self.current_session.as_ref().and_then(|s| s.base_branch.as_ref()).is_some() {
                items.push(MainMenuItem::SwitchBranch);
            }
        }
        items.push(MainMenuItem::ManageTeams);
        items.push(MainMenuItem::ManageRoles);
        items.push(MainMenuItem::SwitchSession);
        items.push(MainMenuItem::CompleteSession);
        items
    }

    /// Number of worker panes (excludes leader)
    pub fn worker_count(&self) -> usize {
        if self.is_squad() { self.panes.len() - 1 } else { 0 }
    }

    /// Persist the current worker count to the DB session record.
    pub fn persist_worker_count(&mut self) {
        let wc = self.worker_count() as i64;
        let repo = self.open_project_db();
        if let Some(ref mut session) = self.current_session {
            session.worker_count = wc;
            if let Some(repo) = repo {
                let _ = repo.upsert_squad_session(session);
            }
        }
    }

    pub fn toggle_popup(&mut self) {
        match self.mode {
            AppMode::Normal => {
                self.mode = AppMode::Popup(PopupMenu::Main);
                self.menu_index = 0;
            }
            AppMode::Popup(_) => {
                // Don't close popup if no panes exist (startup session selection)
                if !self.panes.is_empty() {
                    self.mode = AppMode::Normal;
                }
            }
        }
    }

    pub fn enter_submenu(&mut self) {
        if let AppMode::Popup(PopupMenu::Main) = self.mode {
            let items = self.main_menu_items();
            if self.menu_index < items.len() {
                match items[self.menu_index] {
                    MainMenuItem::SwitchModels => {
                        self.mode = AppMode::Popup(PopupMenu::Matrix);
                        self.matrix_row = 0;
                        self.matrix_col = MatrixCol::Provider;
                    }
                    MainMenuItem::ConnectProvider => {
                        self.connect_provider_index = 0;
                        self.mode = AppMode::Popup(PopupMenu::ConnectProvider);
                    }
                    MainMenuItem::MaxRetries => {
                        self.submenu_index = (self.default_max_iterations as usize).saturating_sub(1);
                        self.mode = AppMode::Popup(PopupMenu::MaxRetries);
                    }
                    MainMenuItem::SetWorkers => {
                        self.set_worker_count_selection = self.worker_count() as u16;
                        self.mode = AppMode::Popup(PopupMenu::SetWorkerCount);
                    }
                    MainMenuItem::RemoveWorker => {
                        self.remove_worker_target = 0;
                        self.remove_worker_confirming = false;
                        self.mode = AppMode::Popup(PopupMenu::RemoveWorkerList);
                    }
                    MainMenuItem::SwitchBranch => {
                        if let Some(ref project_path) = self.project_path {
                            self.branch_list = crate::worktree::list_local_branches(project_path);
                        }
                        self.branch_list_index = 0;
                        self.recovery_session = None;
                        self.mode = AppMode::Popup(PopupMenu::BranchList);
                    }
                    MainMenuItem::ManageTeams => {
                        if let Ok(repo) = legion_db::open_db() {
                            self.team_list = repo.list_teams().unwrap_or_default();
                        }
                        self.team_list_index = 0;
                        self.mode = AppMode::Popup(PopupMenu::ManageTeams);
                    }
                    MainMenuItem::ManageRoles => {
                        if let Ok(repo) = legion_db::open_db() {
                            self.role_list = repo.list_roles().unwrap_or_default();
                        }
                        self.role_list_index = 0;
                        self.mode = AppMode::Popup(PopupMenu::RoleList);
                    }
                    MainMenuItem::SwitchSession => {
                        self.load_session_list();
                        self.mode = AppMode::Popup(PopupMenu::SessionList);
                        self.session_list_index = 0;
                    }
                    MainMenuItem::CompleteSession => {
                        if let Some(ref session) = self.current_session {
                            if !session.is_default {
                                self.complete_session_name = Some(session.name.clone());
                                self.mode = AppMode::Popup(PopupMenu::CompleteSession);
                                self.complete_merge_index = 0;
                            }
                        }
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
            AppMode::Popup(PopupMenu::Main) => self.main_menu_items().len(),
            AppMode::Popup(PopupMenu::Provider) => self.providers.len(),
            AppMode::Popup(PopupMenu::Model) => {
                self.target_provider_models().map(|m| m.len()).unwrap_or(0)
            }
            AppMode::Popup(PopupMenu::Matrix) => self.matrix_row_count(),
            AppMode::Popup(PopupMenu::SessionList) => self.session_list.len() + 1,
            AppMode::Popup(PopupMenu::CompleteSession) => 3,
            AppMode::Popup(PopupMenu::RemoveWorkerList) => self.worker_count(),
            AppMode::Popup(PopupMenu::RemoveWorkerConfirm) => 0,
            AppMode::Popup(PopupMenu::ManageTeams) => self.team_list.len(),
            AppMode::Popup(PopupMenu::TeamDetail) => self.team_detail_roles.len(),
            AppMode::Popup(PopupMenu::RoleList) => self.role_list.len(),
            AppMode::Popup(PopupMenu::AddRoleToTeam) => self.add_role_available.len(),
            _ => return,
        };
        let idx = match self.mode {
            AppMode::Popup(PopupMenu::Main) => &mut self.menu_index,
            AppMode::Popup(PopupMenu::Matrix) => &mut self.matrix_row,
            AppMode::Popup(PopupMenu::SessionList) => &mut self.session_list_index,
            AppMode::Popup(PopupMenu::CompleteSession) => &mut self.complete_merge_index,
            AppMode::Popup(PopupMenu::RemoveWorkerList) => &mut self.remove_worker_target,
            AppMode::Popup(PopupMenu::RemoveWorkerConfirm) => &mut self.remove_worker_strategy_index,
            AppMode::Popup(PopupMenu::ManageTeams) => &mut self.team_list_index,
            AppMode::Popup(PopupMenu::TeamDetail) => &mut self.team_detail_index,
            AppMode::Popup(PopupMenu::RoleList) => &mut self.role_list_index,
            AppMode::Popup(PopupMenu::AddRoleToTeam) => &mut self.add_role_index,
            _ => &mut self.submenu_index,
        };
        *idx = if *idx > 0 { *idx - 1 } else { len.saturating_sub(1) };
    }

    pub fn menu_down(&mut self) {
        let len = match self.mode {
            AppMode::Popup(PopupMenu::Main) => self.main_menu_items().len(),
            AppMode::Popup(PopupMenu::Provider) => self.providers.len(),
            AppMode::Popup(PopupMenu::Model) => {
                self.target_provider_models().map(|m| m.len()).unwrap_or(0)
            }
            AppMode::Popup(PopupMenu::Matrix) => self.matrix_row_count(),
            AppMode::Popup(PopupMenu::SessionList) => self.session_list.len() + 1,
            AppMode::Popup(PopupMenu::CompleteSession) => 3,
            AppMode::Popup(PopupMenu::RemoveWorkerList) => self.worker_count(),
            AppMode::Popup(PopupMenu::RemoveWorkerConfirm) => 0,
            AppMode::Popup(PopupMenu::ManageTeams) => self.team_list.len(),
            AppMode::Popup(PopupMenu::TeamDetail) => self.team_detail_roles.len(),
            AppMode::Popup(PopupMenu::RoleList) => self.role_list.len(),
            AppMode::Popup(PopupMenu::AddRoleToTeam) => self.add_role_available.len(),
            _ => return,
        };
        let idx = match self.mode {
            AppMode::Popup(PopupMenu::Main) => &mut self.menu_index,
            AppMode::Popup(PopupMenu::Matrix) => &mut self.matrix_row,
            AppMode::Popup(PopupMenu::SessionList) => &mut self.session_list_index,
            AppMode::Popup(PopupMenu::CompleteSession) => &mut self.complete_merge_index,
            AppMode::Popup(PopupMenu::RemoveWorkerList) => &mut self.remove_worker_target,
            AppMode::Popup(PopupMenu::RemoveWorkerConfirm) => &mut self.remove_worker_strategy_index,
            AppMode::Popup(PopupMenu::ManageTeams) => &mut self.team_list_index,
            AppMode::Popup(PopupMenu::TeamDetail) => &mut self.team_detail_index,
            AppMode::Popup(PopupMenu::RoleList) => &mut self.role_list_index,
            AppMode::Popup(PopupMenu::AddRoleToTeam) => &mut self.add_role_index,
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

    /// Resolve team roles from the database for the given team_id
    fn resolve_team_roles(&self, team_id: &str) -> Vec<(String, String, String)> {
        if let Some(ref engine) = self.orchestrate {
            if let Some(db) = engine.db() {
                if let Ok(db_lock) = db.lock() {
                    if let Ok(roles) = db_lock.get_team_roles(team_id) {
                        return roles.into_iter()
                            .map(|r| (r.id, r.name, r.prompt_template))
                            .collect();
                    }
                }
            }
        }
        Vec::new()
    }

    /// Resolve team_prompt from the database for the given team_id
    fn resolve_team_prompt(&self, team_id: &str) -> Option<String> {
        if let Some(ref engine) = self.orchestrate {
            if let Some(db) = engine.db() {
                if let Ok(db_lock) = db.lock() {
                    if let Ok(Some(team)) = db_lock.get_team(team_id) {
                        if !team.team_prompt.is_empty() {
                            return Some(team.team_prompt);
                        }
                    }
                }
            }
        }
        None
    }

    /// Start an SDK task on a worker pane
    pub fn start_sdk_task(
        &mut self,
        pane_index: usize,
        ticket_id: usize,
        prompt: &str,
        team_mode: &legion_core::TeamMode,
        iteration: u16,
        feedback: Option<&str>,
        title: &str,
        context: Option<&str>,
        criteria: Option<&str>,
        structure_plan: Option<&str>,
    ) {
        // Compute values before mutable borrow
        let pane_label = self.panes[pane_index].label.clone();
        let working_dir = self.pane_worktree(&pane_label);
        tracing::info!(
            "SDK task: pane_index={}, label={:?}, working_dir={:?}, project_path={:?}, session={:?}",
            pane_index, pane_label, working_dir,
            self.project_path, self.current_session.as_ref().map(|s| &s.name)
        );
        let use_proxy = self.pane_uses_proxy(&pane_label);
        let proxy_port = self.panes[pane_index].proxy_port;

        // Create SDK parser
        let (term_w, term_h) = self.term_size;
        let lw = (term_w as u32 * self.leader_ratio as u32 / 100) as u16;
        let ww = term_w.saturating_sub(lw).saturating_sub(1);
        let parser = std::sync::Arc::new(std::sync::Mutex::new(
            vt100::Parser::new(term_h.saturating_sub(4), ww.saturating_sub(2), SCROLLBACK_LINES)
        ));

        // Generate system prompt based on team mode
        let wd_str = working_dir.as_ref().map(|p| p.to_string_lossy().to_string());
        let sys_prompt = match team_mode {
            legion_core::TeamMode::Solo => {
                crate::claudemd::worker_instructions(pane_index as u16, wd_str.as_deref(), Some(&[]), None)
            }
            legion_core::TeamMode::TechLeadTeam | legion_core::TeamMode::Custom(_) => {
                let team_name = match team_mode {
                    legion_core::TeamMode::TechLeadTeam => "tech_lead_team",
                    legion_core::TeamMode::Custom(s) => s.as_str(),
                    _ => unreachable!(),
                };
                let team_roles = self.resolve_team_roles(team_name);
                if team_roles.is_empty() {
                    crate::claudemd::worker_instructions(pane_index as u16, wd_str.as_deref(), None, None)
                } else {
                    let team_prompt_str = self.resolve_team_prompt(team_name);
                    crate::claudemd::worker_instructions(
                        pane_index as u16,
                        wd_str.as_deref(),
                        Some(&team_roles),
                        team_prompt_str.as_deref(),
                    )
                }
            }
        };

        // Gather project file tree from worker worktree via git ls-files
        let file_tree = working_dir.as_ref().and_then(|wd| {
            std::process::Command::new("git")
                .args(["ls-files"])
                .current_dir(wd)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|s| !s.is_empty())
        });

        // Build structured effective prompt with title, context, and criteria
        // Prepend working directory instruction directly in the user prompt (higher priority than system prompt)
        let wd_prefix = if let Some(ref wd) = working_dir {
            format!(
                "IMPORTANT: Your working directory is `{}`. Run `pwd` first to confirm. Create ALL files here using relative paths (e.g. `./heart.py`). NEVER use `cd ~` or `cd /Users/...` or any absolute path outside this directory.\n\n",
                wd.display()
            )
        } else {
            String::new()
        };
        let structured_prompt = format!("{}{}", wd_prefix, build_structured_prompt(
            title, prompt, context, criteria, structure_plan, file_tree.as_deref(),
        ));

        let wd = working_dir.unwrap_or_else(|| std::path::PathBuf::from("."));

        // Per-ticket log buffer: reuse for retry iterations (and retries), create new for fresh ticket
        let existing_log = self.ticket_logs.get(&ticket_id).cloned();
        // If we have existing log and starting a fresh run (iteration 1), add retry separator
        if iteration == 1 {
            if let Some(ref buf) = existing_log {
                if let Ok(mut logs) = buf.lock() {
                    if !logs.is_empty() {
                        logs.push(format!("\n--- Retry ---\n"));
                    }
                }
            }
        }

        match crate::sdk::SdkHandle::spawn(
            &wd, &structured_prompt, parser.clone(), use_proxy, proxy_port,
            Some(&sys_prompt), iteration, feedback, existing_log,
        ) {
            Ok(handle) => {
                // Store log buffer in ticket_logs map (keyed by ticket_id, not pane)
                self.ticket_logs.insert(ticket_id, handle.log_buffer.clone());
                let pane = &mut self.panes[pane_index];
                pane.sdk_log_buffer = Some(handle.log_buffer.clone());
                pane.sdk_task = Some(handle);
                pane.sdk_parser = Some(parser);
                pane.current_ticket_id = Some(ticket_id);
                pane.last_sdk_activity = Some(Instant::now());
                if iteration == 1 {
                    pane.sdk_entries.clear();
                }
                tracing::info!("SDK task started for pane {} (ticket {}, iter {})", pane_index, ticket_id, iteration);
            }
            Err(e) => {
                tracing::error!("Failed to start SDK task for pane {}: {}", pane_index, e);
            }
        }
    }

    /// Remove a single worker: kill SDK task, handle git worktree, remove pane
    pub fn remove_single_worker(&mut self, pane_index: usize, strategy: &str) -> anyhow::Result<()> {
        if pane_index == 0 || pane_index >= self.panes.len() {
            return Err(anyhow::anyhow!("Invalid pane index for removal"));
        }

        let label = self.panes[pane_index].label.clone();

        // Kill PTY
        if let Some(ref mut pty) = self.panes[pane_index].pty {
            pty.kill();
        }

        // Kill SDK task
        if let Some(ref mut sdk) = self.panes[pane_index].sdk_task {
            sdk.kill();
        }

        // Handle git worktree
        if let (Some(ref project_path), Some(ref session)) = (&self.project_path, &self.current_session) {
            match strategy {
                "merge" => {
                    let default_branch = crate::worktree::default_branch(project_path);
                    let _ = std::process::Command::new("git")
                        .args(["checkout", &default_branch])
                        .current_dir(project_path)
                        .output();
                    crate::worktree::merge_branch(project_path, &session.name, &label)?;
                    let _ = crate::worktree::remove_worktree(project_path, &session.name, &label, false);
                }
                "discard" => {
                    let _ = crate::worktree::remove_worktree(project_path, &session.name, &label, true);
                }
                _ => {
                    // "keep" — leave worktree as-is
                }
            }
        }

        // Remove pane
        self.panes.remove(pane_index);

        // Adjust focused pane
        if self.focused_pane >= self.panes.len() {
            self.focused_pane = self.panes.len().saturating_sub(1);
        }

        // Resize remaining panes
        self.apply_resize();

        tracing::info!("Removed worker '{}' with strategy '{}'", label, strategy);
        Ok(())
    }
}

/// Build a structured prompt from ticket fields for SDK execution
fn build_structured_prompt(
    title: &str, prompt: &str, context: Option<&str>, criteria: Option<&str>,
    structure_plan: Option<&str>, file_tree: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    parts.push(format!("# Task: {}", title));

    if let Some(plan) = structure_plan {
        if !plan.is_empty() {
            parts.push(String::new());
            parts.push("## Architecture & Structure Plan".to_string());
            parts.push(plan.to_string());
            parts.push(String::new());
            parts.push("**IMPORTANT**: Follow the file paths and conventions above exactly.".to_string());
        }
    }

    if let Some(ctx) = context {
        if !ctx.is_empty() {
            parts.push(String::new());
            parts.push("## Context".to_string());
            parts.push(ctx.to_string());
        }
    }

    if let Some(crit) = criteria {
        if !crit.is_empty() {
            parts.push(String::new());
            parts.push("## Success Criteria".to_string());
            parts.push(crit.to_string());
        }
    }

    if let Some(tree) = file_tree {
        if !tree.is_empty() {
            parts.push(String::new());
            parts.push("## Current Project Structure".to_string());
            parts.push(format!("```\n{}\n```", tree));
        }
    }

    parts.push(String::new());
    parts.push("## Task Description".to_string());
    parts.push(prompt.to_string());

    parts.push(String::new());
    parts.push("## Instructions".to_string());
    parts.push("- Follow the team workflow described in your system prompt".to_string());
    parts.push("- Output <promise>DONE</promise> when ALL criteria are met".to_string());

    parts.join("\n")
}
