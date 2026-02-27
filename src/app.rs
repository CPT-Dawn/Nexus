use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;

use crate::animation::AnimationState;
use crate::animation::transitions::smooth_signals;
use crate::config::Config;
use crate::event::Event;
use crate::network::types::*;
use crate::ui::theme::Theme;

/// Application mode / state machine
#[derive(Debug, Clone)]
pub enum AppMode {
    /// Normal browsing mode
    Normal,
    /// Scan in progress
    Scanning,
    /// Password input dialog (for the given SSID)
    PasswordInput { ssid: String },
    /// Connecting to a network
    Connecting,
    /// Disconnecting
    Disconnecting,
    /// Hidden network dialog
    Hidden,
    /// Help overlay
    Help,
    /// Error dialog
    Error(String),
}

/// Main application state
pub struct App {
    pub mode: AppMode,
    pub networks: Vec<WiFiNetwork>,
    pub selected_index: usize,
    pub connection_status: ConnectionStatus,
    pub password_input: String,
    pub password_visible: bool,
    pub hidden_ssid_input: String,
    pub hidden_password_input: String,
    pub hidden_field_focus: u8, // 0 = SSID, 1 = password
    pub animation: AnimationState,
    pub should_quit: bool,
    pub detail_visible: bool,
    pub config: Config,
    pub theme: Theme,
    pub interface_name: String,
    event_tx: mpsc::UnboundedSender<Event>,
}

impl App {
    pub fn new(
        config: Config,
        theme: Theme,
        interface_name: String,
        event_tx: mpsc::UnboundedSender<Event>,
    ) -> Self {
        let detail_visible = config.appearance.show_details;
        Self {
            mode: AppMode::Normal,
            networks: Vec::new(),
            selected_index: 0,
            connection_status: ConnectionStatus::default(),
            password_input: String::new(),
            password_visible: false,
            hidden_ssid_input: String::new(),
            hidden_password_input: String::new(),
            hidden_field_focus: 0,
            animation: AnimationState::default(),
            should_quit: false,
            detail_visible,
            config,
            theme,
            interface_name,
            event_tx,
        }
    }

    /// Process a key event
    pub fn handle_key(&mut self, key: KeyEvent) {
        match &self.mode {
            AppMode::Normal | AppMode::Scanning => self.handle_key_normal(key),
            AppMode::PasswordInput { .. } => self.handle_key_password(key),
            AppMode::Hidden => self.handle_key_hidden(key),
            AppMode::Help => self.handle_key_help(key),
            AppMode::Error(_) => self.handle_key_error(key),
            AppMode::Connecting | AppMode::Disconnecting => {
                // Only allow quit during busy states
                if key.code == KeyCode::Char('q') {
                    self.should_quit = true;
                }
            }
        }
    }

    /// Handle keys in normal/scanning mode
    fn handle_key_normal(&mut self, key: KeyEvent) {
        match key.code {
            // Navigation
            KeyCode::Up | KeyCode::Char('k') => self.select_prev(),
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Char('g') => self.select_first(),
            KeyCode::Char('G') => self.select_last(),
            KeyCode::Home => self.select_first(),
            KeyCode::End => self.select_last(),

            // Actions
            KeyCode::Enter => self.action_connect(),
            KeyCode::Char('d') => self.action_disconnect(),
            KeyCode::Char('s') => self.action_scan(),
            KeyCode::Char('f') => self.action_forget(),
            KeyCode::Char('h') => self.action_hidden(),
            KeyCode::Char('r') => self.action_refresh(),
            KeyCode::Char('i') => {
                self.detail_visible = !self.detail_visible;
            }

            // Help
            KeyCode::Char('?') | KeyCode::Char('/') => {
                self.mode = AppMode::Help;
                self.animation.start_dialog_slide();
            }

            // Quit
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => self.should_quit = true,

            _ => {}
        }
    }

    /// Handle keys in password input mode
    fn handle_key_password(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                let password = self.password_input.clone();
                if let AppMode::PasswordInput { ssid } = &self.mode {
                    let ssid = ssid.clone();
                    self.mode = AppMode::Connecting;
                    self.connection_status = ConnectionStatus::Connecting(ssid.clone());
                    self.animation.start_spinner();

                    let _tx = self.event_tx.clone();
                    let pwd = if password.is_empty() {
                        None
                    } else {
                        Some(password)
                    };
                    // Fire connect in background
                    self.dispatch_connect(ssid, pwd);
                }
            }
            KeyCode::Esc => {
                self.password_input.clear();
                self.password_visible = false;
                self.mode = AppMode::Normal;
            }
            KeyCode::Backspace => {
                self.password_input.pop();
            }
            KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.password_visible = !self.password_visible;
            }
            KeyCode::Char(c) => {
                self.password_input.push(c);
            }
            _ => {}
        }
    }

    /// Handle keys in hidden network dialog
    fn handle_key_hidden(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                self.hidden_field_focus = if self.hidden_field_focus == 0 { 1 } else { 0 };
            }
            KeyCode::Enter => {
                if !self.hidden_ssid_input.is_empty() {
                    let ssid = self.hidden_ssid_input.clone();
                    let pwd = if self.hidden_password_input.is_empty() {
                        None
                    } else {
                        Some(self.hidden_password_input.clone())
                    };
                    self.mode = AppMode::Connecting;
                    self.connection_status = ConnectionStatus::Connecting(ssid.clone());
                    self.animation.start_spinner();
                    self.dispatch_connect_hidden(ssid, pwd);
                }
            }
            KeyCode::Esc => {
                self.hidden_ssid_input.clear();
                self.hidden_password_input.clear();
                self.hidden_field_focus = 0;
                self.password_visible = false;
                self.mode = AppMode::Normal;
            }
            KeyCode::Backspace => {
                if self.hidden_field_focus == 0 {
                    self.hidden_ssid_input.pop();
                } else {
                    self.hidden_password_input.pop();
                }
            }
            KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.password_visible = !self.password_visible;
            }
            KeyCode::Char(c) => {
                if self.hidden_field_focus == 0 {
                    self.hidden_ssid_input.push(c);
                } else {
                    self.hidden_password_input.push(c);
                }
            }
            _ => {}
        }
    }

    /// Handle keys in help overlay
    fn handle_key_help(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('?') | KeyCode::Char('/') | KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = AppMode::Normal;
            }
            _ => {}
        }
    }

    /// Handle keys in error dialog
    fn handle_key_error(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                self.mode = AppMode::Normal;
            }
            _ => {}
        }
    }

    // ─── Navigation ─────────────────────────────────────────────────

    fn select_prev(&mut self) {
        if !self.networks.is_empty() {
            self.selected_index = self.selected_index.saturating_sub(1);
        }
    }

    fn select_next(&mut self) {
        if !self.networks.is_empty() {
            self.selected_index = (self.selected_index + 1).min(self.networks.len() - 1);
        }
    }

    fn select_first(&mut self) {
        self.selected_index = 0;
    }

    fn select_last(&mut self) {
        if !self.networks.is_empty() {
            self.selected_index = self.networks.len() - 1;
        }
    }

    // ─── Actions ────────────────────────────────────────────────────

    fn action_connect(&mut self) {
        if self.networks.is_empty() {
            return;
        }

        let net = &self.networks[self.selected_index];

        // Already connected
        if net.is_active {
            return;
        }

        if net.security.needs_password() && !net.is_saved {
            // Need password — open dialog
            self.password_input.clear();
            self.password_visible = false;
            self.mode = AppMode::PasswordInput {
                ssid: net.ssid.clone(),
            };
            self.animation.start_dialog_slide();
        } else {
            // Open network or saved network — connect directly
            let ssid = net.ssid.clone();
            self.mode = AppMode::Connecting;
            self.connection_status = ConnectionStatus::Connecting(ssid.clone());
            self.animation.start_spinner();
            self.dispatch_connect(ssid, None);
        }
    }

    fn action_disconnect(&mut self) {
        if !self.connection_status.is_connected() || self.connection_status.is_busy() {
            return;
        }
        let _active_ssid = self.connection_status.ssid().map(|s| s.to_string());
        self.mode = AppMode::Disconnecting;
        self.connection_status = ConnectionStatus::Disconnecting;
        self.animation.start_spinner();

        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(Event::Error("DISCONNECT:".to_string()));
        });
    }

    fn action_scan(&mut self) {
        if matches!(self.mode, AppMode::Scanning) {
            return;
        }
        self.mode = AppMode::Scanning;
        self.animation.start_spinner();

        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            // Signal the main loop to perform a scan
            let _ = tx.send(Event::NetworkEvent(NetworkEvent::ScanComplete(Vec::new())));
        });
    }

    fn action_forget(&mut self) {
        if self.networks.is_empty() {
            return;
        }
        let net = &self.networks[self.selected_index];
        if !net.is_saved {
            self.mode = AppMode::Error("Network is not saved".to_string());
            self.animation.start_dialog_slide();
            return;
        }

        let tx = self.event_tx.clone();
        let ssid = net.ssid.clone();
        tokio::spawn(async move {
            let _ = tx.send(Event::Error(format!("FORGET:{}", ssid)));
        });
    }

    fn action_hidden(&mut self) {
        self.hidden_ssid_input.clear();
        self.hidden_password_input.clear();
        self.hidden_field_focus = 0;
        self.password_visible = false;
        self.mode = AppMode::Hidden;
        self.animation.start_dialog_slide();
    }

    fn action_refresh(&mut self) {
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(Event::NetworkEvent(NetworkEvent::ConnectionChanged(
                ConnectionStatus::Disconnected,
            )));
        });
    }

    fn dispatch_connect(&mut self, ssid: String, password: Option<String>) {
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(Event::Error(format!(
                "CONNECT:{}:{}",
                ssid,
                password.unwrap_or_default()
            )));
        });
    }

    fn dispatch_connect_hidden(&mut self, ssid: String, password: Option<String>) {
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(Event::Error(format!(
                "CONNECT_HIDDEN:{}:{}",
                ssid,
                password.unwrap_or_default()
            )));
        });
    }

    // ─── Tick / Animation Updates ───────────────────────────────────

    /// Called every tick to advance animations and smooth values
    pub fn tick(&mut self) {
        // Only advance animations if enabled in config
        if self.config.animations() {
            self.animation.tick();
        }

        // Smooth signal strength display values
        smooth_signals(&mut self.networks, 0.2);
    }

    /// Update network list from scan results
    pub fn update_networks(&mut self, mut networks: Vec<WiFiNetwork>) {
        // Preserve seen_ticks and display_signal for networks that were already visible
        for new_net in networks.iter_mut() {
            if let Some(existing) = self.networks.iter().find(|n| n.ssid == new_net.ssid) {
                new_net.seen_ticks = existing.seen_ticks;
                new_net.display_signal = existing.display_signal;
            }
        }

        self.networks = networks;

        // Clamp selected index
        if !self.networks.is_empty() {
            self.selected_index = self.selected_index.min(self.networks.len() - 1);
        } else {
            self.selected_index = 0;
        }

        // Return to normal mode if we were scanning
        if matches!(self.mode, AppMode::Scanning) {
            self.mode = AppMode::Normal;
            self.animation.stop_spinner();
        }
    }

    /// Update connection status
    pub fn update_connection_status(&mut self, status: ConnectionStatus) {
        self.connection_status = status;

        // If we were connecting/disconnecting, return to normal
        if matches!(self.mode, AppMode::Connecting | AppMode::Disconnecting) {
            self.mode = AppMode::Normal;
            self.animation.stop_spinner();
        }
    }
}
