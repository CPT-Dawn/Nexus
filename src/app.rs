use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;

use crate::animation::AnimationState;
use crate::animation::transitions::smooth_signals;
use crate::config::Config;
use crate::event::{Event, NetworkCommand};
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
    /// Inline search / filter mode
    Search,
    /// Error dialog
    Error(String),
}

/// Sort ordering for the network list
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Signal,
    Alphabetical,
    Security,
    Band,
}

impl SortMode {
    /// Cycle to the next sort mode
    pub fn next(self) -> Self {
        match self {
            Self::Signal => Self::Alphabetical,
            Self::Alphabetical => Self::Security,
            Self::Security => Self::Band,
            Self::Band => Self::Signal,
        }
    }

    /// Human-readable label for the title bar
    pub fn label(self) -> &'static str {
        match self {
            Self::Signal => "↓Signal",
            Self::Alphabetical => "↓A-Z",
            Self::Security => "↓Security",
            Self::Band => "↓Band",
        }
    }
}

/// Main application state
pub struct App {
    pub mode: AppMode,
    pub networks: Vec<WiFiNetwork>,
    /// Filtered view indices into `networks`
    pub filtered_indices: Vec<usize>,
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
    pub sort_mode: SortMode,
    pub search_query: String,
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
            filtered_indices: Vec::new(),
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
            sort_mode: SortMode::Signal,
            search_query: String::new(),
            event_tx,
        }
    }

    /// Get the list of networks to display (filtered view).
    /// Returns references via index.
    pub fn visible_networks(&self) -> Vec<&WiFiNetwork> {
        self.filtered_indices
            .iter()
            .filter_map(|&i| self.networks.get(i))
            .collect()
    }

    /// Get the currently selected network (accounting for filter)
    pub fn selected_network(&self) -> Option<&WiFiNetwork> {
        self.filtered_indices
            .get(self.selected_index)
            .and_then(|&i| self.networks.get(i))
    }

    /// Rebuild the filtered indices based on search query
    fn rebuild_filter(&mut self) {
        let query = self.search_query.to_lowercase();
        self.filtered_indices = self
            .networks
            .iter()
            .enumerate()
            .filter(|(_, net)| {
                if query.is_empty() {
                    return true;
                }
                net.ssid.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();

        // Clamp selection
        if self.filtered_indices.is_empty() {
            self.selected_index = 0;
        } else {
            self.selected_index = self.selected_index.min(self.filtered_indices.len() - 1);
        }
    }

    // ─── Key Matching Helpers ───────────────────────────────────────

    /// Check if a key event matches a config-defined keybinding.
    /// Supports single-char keys and special key names.
    fn key_matches(&self, key: &KeyEvent, binding: &str) -> bool {
        match binding {
            "enter" => key.code == KeyCode::Enter,
            "esc" => key.code == KeyCode::Esc,
            "tab" => key.code == KeyCode::Tab,
            "backtab" => key.code == KeyCode::BackTab,
            "up" => key.code == KeyCode::Up,
            "down" => key.code == KeyCode::Down,
            "left" => key.code == KeyCode::Left,
            "right" => key.code == KeyCode::Right,
            "home" => key.code == KeyCode::Home,
            "end" => key.code == KeyCode::End,
            "backspace" => key.code == KeyCode::Backspace,
            "delete" => key.code == KeyCode::Delete,
            s if s.len() == 1 => {
                let ch = s.chars().next().unwrap();
                key.code == KeyCode::Char(ch)
            }
            _ => false,
        }
    }

    /// Process a key event
    pub fn handle_key(&mut self, key: KeyEvent) {
        match &self.mode {
            AppMode::Normal | AppMode::Scanning => self.handle_key_normal(key),
            AppMode::PasswordInput { .. } => self.handle_key_password(key),
            AppMode::Hidden => self.handle_key_hidden(key),
            AppMode::Help => self.handle_key_help(key),
            AppMode::Search => self.handle_key_search(key),
            AppMode::Error(_) => self.handle_key_error(key),
            AppMode::Connecting | AppMode::Disconnecting => {
                // Only allow quit during busy states
                if key.code == KeyCode::Char('q') {
                    self.should_quit = true;
                }
            }
        }
    }

    /// Handle keys in normal/scanning mode — uses config keybindings
    fn handle_key_normal(&mut self, key: KeyEvent) {
        let keys = self.config.keys.clone();

        // Hard-coded navigation (vim + arrows)
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.select_prev();
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.select_next();
                return;
            }
            KeyCode::Char('g') if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.select_first();
                return;
            }
            KeyCode::Char('G') => {
                self.select_last();
                return;
            }
            KeyCode::Home => {
                self.select_first();
                return;
            }
            KeyCode::End => {
                self.select_last();
                return;
            }
            _ => {}
        }

        // Config-driven action keys
        if self.key_matches(&key, &keys.connect) {
            self.action_connect();
        } else if self.key_matches(&key, &keys.disconnect) {
            self.action_disconnect();
        } else if self.key_matches(&key, &keys.scan) {
            self.action_scan();
        } else if self.key_matches(&key, &keys.forget) {
            self.action_forget();
        } else if self.key_matches(&key, &keys.hidden) {
            self.action_hidden();
        } else if self.key_matches(&key, &keys.refresh) {
            self.action_refresh();
        } else if self.key_matches(&key, &keys.details) {
            self.detail_visible = !self.detail_visible;
        } else if self.key_matches(&key, &keys.help) {
            self.mode = AppMode::Help;
            self.animation.start_dialog_slide();
        } else if self.key_matches(&key, &keys.sort) {
            self.sort_mode = self.sort_mode.next();
            self.apply_sort();
            self.rebuild_filter();
        } else if self.key_matches(&key, &keys.search) {
            self.search_query.clear();
            self.mode = AppMode::Search;
        } else if self.key_matches(&key, &keys.quit) {
            self.should_quit = true;
        } else if key.code == KeyCode::Esc {
            // Clear filter if active, otherwise quit
            if !self.search_query.is_empty() {
                self.search_query.clear();
                self.rebuild_filter();
            } else {
                self.should_quit = true;
            }
        }
    }

    /// Handle keys in search/filter mode
    fn handle_key_search(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                // Keep the current query but exit search mode
                self.mode = AppMode::Normal;
            }
            KeyCode::Enter => {
                self.mode = AppMode::Normal;
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.rebuild_filter();
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.rebuild_filter();
            }
            KeyCode::Up => self.select_prev(),
            KeyCode::Down => self.select_next(),
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

                    let pwd = if password.is_empty() {
                        None
                    } else {
                        Some(password)
                    };
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
        if !self.filtered_indices.is_empty() {
            self.selected_index = self.selected_index.saturating_sub(1);
        }
    }

    fn select_next(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.selected_index = (self.selected_index + 1).min(self.filtered_indices.len() - 1);
        }
    }

    fn select_first(&mut self) {
        self.selected_index = 0;
    }

    fn select_last(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.selected_index = self.filtered_indices.len() - 1;
        }
    }

    // ─── Sorting ────────────────────────────────────────────────────

    /// Apply the current sort mode to `self.networks`
    fn apply_sort(&mut self) {
        match self.sort_mode {
            SortMode::Signal => {
                self.networks.sort_by(|a, b| {
                    b.is_active
                        .cmp(&a.is_active)
                        .then(b.signal_strength.cmp(&a.signal_strength))
                });
            }
            SortMode::Alphabetical => {
                self.networks.sort_by(|a, b| {
                    b.is_active
                        .cmp(&a.is_active)
                        .then(a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
                });
            }
            SortMode::Security => {
                self.networks.sort_by(|a, b| {
                    b.is_active
                        .cmp(&a.is_active)
                        .then(security_rank(&b.security).cmp(&security_rank(&a.security)))
                        .then(b.signal_strength.cmp(&a.signal_strength))
                });
            }
            SortMode::Band => {
                self.networks.sort_by(|a, b| {
                    b.is_active
                        .cmp(&a.is_active)
                        .then(b.frequency.cmp(&a.frequency))
                        .then(b.signal_strength.cmp(&a.signal_strength))
                });
            }
        }
    }

    // ─── Actions ────────────────────────────────────────────────────

    fn action_connect(&mut self) {
        let net = match self.selected_network() {
            Some(n) => n,
            None => return,
        };

        // Already connected
        if net.is_active {
            return;
        }

        if net.security.needs_password() && !net.is_saved {
            let ssid = net.ssid.clone();
            self.password_input.clear();
            self.password_visible = false;
            self.mode = AppMode::PasswordInput { ssid };
            self.animation.start_dialog_slide();
        } else {
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
        self.mode = AppMode::Disconnecting;
        self.connection_status = ConnectionStatus::Disconnecting;
        self.animation.start_spinner();
        let _ = self
            .event_tx
            .send(Event::Command(NetworkCommand::Disconnect));
    }

    fn action_scan(&mut self) {
        if matches!(self.mode, AppMode::Scanning) {
            return;
        }
        self.mode = AppMode::Scanning;
        self.animation.start_spinner();
        let _ = self.event_tx.send(Event::Command(NetworkCommand::Scan));
    }

    fn action_forget(&mut self) {
        let net = match self.selected_network() {
            Some(n) => n,
            None => return,
        };
        if !net.is_saved {
            self.mode = AppMode::Error("Network is not saved".to_string());
            self.animation.start_dialog_slide();
            return;
        }
        let ssid = net.ssid.clone();
        let _ = self
            .event_tx
            .send(Event::Command(NetworkCommand::Forget { ssid }));
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
        let _ = self
            .event_tx
            .send(Event::Command(NetworkCommand::RefreshConnection));
    }

    fn dispatch_connect(&mut self, ssid: String, password: Option<String>) {
        let _ = self
            .event_tx
            .send(Event::Command(NetworkCommand::Connect { ssid, password }));
    }

    fn dispatch_connect_hidden(&mut self, ssid: String, password: Option<String>) {
        let _ = self
            .event_tx
            .send(Event::Command(NetworkCommand::ConnectHidden {
                ssid,
                password,
            }));
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

        // Apply current sort
        self.apply_sort();
        // Rebuild filter
        self.rebuild_filter();

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

/// Rank security types for sorting (higher = more secure)
fn security_rank(sec: &SecurityType) -> u8 {
    match sec {
        SecurityType::Open => 0,
        SecurityType::Wep => 1,
        SecurityType::Wpa => 2,
        SecurityType::WPA2 => 3,
        SecurityType::WPA2Enterprise => 4,
        SecurityType::WPA3 => 5,
        SecurityType::Unknown => 0,
    }
}
