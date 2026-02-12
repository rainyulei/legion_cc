//! TUI application state and logic

use std::sync::Arc;

use legion_core::{ProxyConfig, ProxyServer};
use legion_core::session::{discover_sessions, ClaudeSession};
use legion_db::{Provider, Session};

use crate::pty::PtyHandle;

/// Popup menu types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupMenu {
    Main,
    Provider,
    Model,
    Session,
}

/// Application mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Normal,
    Popup(PopupMenu),
}

/// Main menu items
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainMenuItem {
    Provider,
    Model,
    Session,
    Settings,
    Quit,
}

impl MainMenuItem {
    pub fn label(&self) -> &'static str {
        match self {
            MainMenuItem::Provider => "Provider",
            MainMenuItem::Model => "Model",
            MainMenuItem::Session => "Session",
            MainMenuItem::Settings => "Settings",
            MainMenuItem::Quit => "Quit",
        }
    }
}

/// TUI application state
pub struct App {
    /// Current application mode
    pub mode: AppMode,
    /// Whether the app should quit
    pub should_quit: bool,

    /// Available providers
    pub providers: Vec<Provider>,
    /// Currently selected provider index
    pub current_provider: Option<usize>,
    /// Currently selected model name
    pub current_model: Option<String>,
    /// Whether we're connected to the provider
    pub provider_connected: bool,

    /// Available sessions (from database)
    pub sessions: Vec<Session>,
    /// Currently selected session index
    pub current_session: Option<usize>,

    /// Discovered Claude Code sessions (from ~/.claude/projects/)
    pub claude_sessions: Vec<ClaudeSession>,
    /// Currently active Claude session
    pub current_claude_session: Option<ClaudeSession>,

    /// Current menu selection index (main menu)
    pub menu_index: usize,
    /// Current submenu selection index
    pub submenu_index: usize,

    /// Proxy server instance
    pub proxy: Arc<ProxyServer>,
    /// Proxy server port
    pub proxy_port: u16,

    /// PTY handle for Claude Code process
    pub pty: Option<PtyHandle>,
    /// PTY output buffer as string
    pub pty_output: String,
}

impl Default for App {
    fn default() -> Self {
        Self::new(18080)
    }
}

impl App {
    /// Create a new App instance with specified proxy port
    pub fn new(proxy_port: u16) -> Self {
        Self {
            mode: AppMode::Normal,
            should_quit: false,
            providers: Vec::new(),
            current_provider: None,
            current_model: None,
            provider_connected: false,
            sessions: Vec::new(),
            current_session: None,
            claude_sessions: Vec::new(),
            current_claude_session: None,
            menu_index: 0,
            submenu_index: 0,
            proxy: Arc::new(ProxyServer::new(proxy_port)),
            proxy_port,
            pty: None,
            pty_output: String::new(),
        }
    }

    /// Get the main menu items
    pub fn main_menu_items() -> &'static [MainMenuItem] {
        &[
            MainMenuItem::Provider,
            MainMenuItem::Model,
            MainMenuItem::Session,
            MainMenuItem::Settings,
            MainMenuItem::Quit,
        ]
    }

    /// Toggle popup menu
    pub fn toggle_popup(&mut self) {
        match self.mode {
            AppMode::Normal => {
                self.mode = AppMode::Popup(PopupMenu::Main);
                self.menu_index = 0;
            }
            AppMode::Popup(_) => {
                self.mode = AppMode::Normal;
            }
        }
    }

    /// Enter a submenu based on current selection
    pub fn enter_submenu(&mut self) {
        if let AppMode::Popup(PopupMenu::Main) = self.mode {
            let items = Self::main_menu_items();
            if self.menu_index < items.len() {
                match items[self.menu_index] {
                    MainMenuItem::Provider => {
                        self.mode = AppMode::Popup(PopupMenu::Provider);
                        self.submenu_index = self.current_provider.unwrap_or(0);
                    }
                    MainMenuItem::Model => {
                        self.mode = AppMode::Popup(PopupMenu::Model);
                        // Find current model index
                        self.submenu_index = self.get_current_model_index().unwrap_or(0);
                    }
                    MainMenuItem::Session => {
                        self.refresh_sessions();
                        self.mode = AppMode::Popup(PopupMenu::Session);
                        // Find current claude session index
                        self.submenu_index = self
                            .current_claude_session
                            .as_ref()
                            .and_then(|current| {
                                self.claude_sessions.iter().position(|s| s.id == current.id)
                            })
                            .unwrap_or(0);
                    }
                    MainMenuItem::Settings => {
                        // TODO: Settings submenu
                    }
                    MainMenuItem::Quit => {
                        self.should_quit = true;
                    }
                }
            }
        }
    }

    /// Select item in submenu
    pub fn select_submenu_item(&mut self) {
        match self.mode {
            AppMode::Popup(PopupMenu::Provider) => {
                if self.submenu_index < self.providers.len() {
                    self.current_provider = Some(self.submenu_index);
                    // Reset model when provider changes
                    self.current_model = self.get_current_provider()
                        .and_then(|p| p.models.as_ref())
                        .and_then(|m| m.first().cloned());
                    self.provider_connected = true;
                }
                self.back_to_main_menu();
            }
            AppMode::Popup(PopupMenu::Model) => {
                if let Some(models) = self.get_current_provider_models() {
                    if self.submenu_index < models.len() {
                        self.current_model = Some(models[self.submenu_index].clone());
                    }
                }
                self.back_to_main_menu();
            }
            AppMode::Popup(PopupMenu::Session) => {
                if self.submenu_index < self.claude_sessions.len() {
                    let session = self.claude_sessions[self.submenu_index].clone();
                    let _ = self.switch_session(&session);
                }
                self.back_to_main_menu();
            }
            _ => {}
        }
    }

    /// Go back to main menu
    pub fn back_to_main_menu(&mut self) {
        self.mode = AppMode::Popup(PopupMenu::Main);
    }

    /// Move menu selection up
    pub fn menu_up(&mut self) {
        match self.mode {
            AppMode::Popup(PopupMenu::Main) => {
                let items = Self::main_menu_items();
                if self.menu_index > 0 {
                    self.menu_index -= 1;
                } else {
                    self.menu_index = items.len().saturating_sub(1);
                }
            }
            AppMode::Popup(PopupMenu::Provider) => {
                if self.submenu_index > 0 {
                    self.submenu_index -= 1;
                } else {
                    self.submenu_index = self.providers.len().saturating_sub(1);
                }
            }
            AppMode::Popup(PopupMenu::Model) => {
                let count = self.get_current_provider_models().map(|m| m.len()).unwrap_or(0);
                if self.submenu_index > 0 {
                    self.submenu_index -= 1;
                } else {
                    self.submenu_index = count.saturating_sub(1);
                }
            }
            AppMode::Popup(PopupMenu::Session) => {
                if self.submenu_index > 0 {
                    self.submenu_index -= 1;
                } else {
                    self.submenu_index = self.claude_sessions.len().saturating_sub(1);
                }
            }
            _ => {}
        }
    }

    /// Move menu selection down
    pub fn menu_down(&mut self) {
        match self.mode {
            AppMode::Popup(PopupMenu::Main) => {
                let items = Self::main_menu_items();
                if self.menu_index < items.len().saturating_sub(1) {
                    self.menu_index += 1;
                } else {
                    self.menu_index = 0;
                }
            }
            AppMode::Popup(PopupMenu::Provider) => {
                if self.submenu_index < self.providers.len().saturating_sub(1) {
                    self.submenu_index += 1;
                } else {
                    self.submenu_index = 0;
                }
            }
            AppMode::Popup(PopupMenu::Model) => {
                let count = self.get_current_provider_models().map(|m| m.len()).unwrap_or(0);
                if self.submenu_index < count.saturating_sub(1) {
                    self.submenu_index += 1;
                } else {
                    self.submenu_index = 0;
                }
            }
            AppMode::Popup(PopupMenu::Session) => {
                if self.submenu_index < self.claude_sessions.len().saturating_sub(1) {
                    self.submenu_index += 1;
                } else {
                    self.submenu_index = 0;
                }
            }
            _ => {}
        }
    }

    /// Get current provider
    pub fn get_current_provider(&self) -> Option<&Provider> {
        self.current_provider.and_then(|i| self.providers.get(i))
    }

    /// Get current provider's models
    pub fn get_current_provider_models(&self) -> Option<&Vec<String>> {
        self.get_current_provider().and_then(|p| p.models.as_ref())
    }

    /// Get current model index in the provider's model list
    fn get_current_model_index(&self) -> Option<usize> {
        let models = self.get_current_provider_models()?;
        let current = self.current_model.as_ref()?;
        models.iter().position(|m| m == current)
    }

    /// Get current session
    pub fn get_current_session(&self) -> Option<&Session> {
        self.current_session.and_then(|i| self.sessions.get(i))
    }

    /// Refresh discovered Claude sessions from ~/.claude/projects/
    pub fn refresh_sessions(&mut self) {
        self.claude_sessions = discover_sessions().unwrap_or_default();
    }

    /// Switch to a discovered Claude session
    pub fn switch_session(&mut self, session: &ClaudeSession) -> anyhow::Result<()> {
        self.current_claude_session = Some(session.clone());

        // Restart Claude with --resume pointing to the session file
        // This will require modifying PTY spawn
        Ok(())
    }

    /// Load data from repository
    pub fn load_from_repo(&mut self, repo: &legion_db::Repository) {
        if let Ok(providers) = repo.list_providers() {
            self.providers = providers;
            // Set default provider if available
            if let Ok(Some(default)) = repo.get_default_provider() {
                self.current_provider = self.providers.iter().position(|p| p.id == default.id);
                self.current_model = default.models.as_ref().and_then(|m| m.first().cloned());
                self.provider_connected = true;
            }
        }

        if let Ok(sessions) = repo.list_sessions() {
            self.sessions = sessions;
            if !self.sessions.is_empty() {
                self.current_session = Some(0);
            }
        }
    }

    /// Connect to the currently selected provider and update proxy configuration
    pub async fn connect_provider(&self) -> anyhow::Result<()> {
        if let Some(provider) = self.get_current_provider() {
            let config = ProxyConfig {
                target_url: Some(provider.base_url.clone()),
                api_key: provider.api_key.clone(),
                api_format: Some(provider.api_format.clone()),
                model: self.current_model.clone(),
            };
            self.proxy.update_config(config).await;
        }
        Ok(())
    }

    /// Update proxy configuration when model changes
    pub async fn update_proxy_model(&self) {
        if let Some(provider) = self.get_current_provider() {
            let config = ProxyConfig {
                target_url: Some(provider.base_url.clone()),
                api_key: provider.api_key.clone(),
                api_format: Some(provider.api_format.clone()),
                model: self.current_model.clone(),
            };
            self.proxy.update_config(config).await;
        }
    }

    /// Start Claude Code in a PTY
    pub fn start_claude(&mut self) -> anyhow::Result<()> {
        let pty = PtyHandle::spawn_claude(self.proxy_port)?;
        self.pty = Some(pty);
        Ok(())
    }

    /// Update PTY output buffer from the PTY handle
    pub fn update_pty_output(&mut self) {
        if let Some(pty) = &self.pty {
            let output = pty.read_output();
            self.pty_output = String::from_utf8_lossy(&output).to_string();
        }
    }

    /// Send data to the PTY
    pub fn send_to_pty(&mut self, data: &[u8]) {
        if let Some(pty) = &mut self.pty {
            let _ = pty.write(data);
        }
    }
}
