use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::auth::PermissionLevel;
use crate::event::Event;
use crate::network::types::*;
use crate::network::NetworkManager;
use crate::ui::components::confirm_dialog::ConfirmDialog;
use crate::ui::components::input_dialog::InputDialog;
use crate::ui::pages::diagnostics::{DiagnosticTool, DiagnosticsState};
use crate::ui::theme::Theme;

// ── Page & Mode enums ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Dashboard,
    Interfaces,
    Wifi,
    Connections,
    Diagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Input,
    Dialog,
    Filtering,
}

// ── Per-page state ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct InterfacesPageState {
    pub selected_index: usize,
}

#[derive(Debug, Clone, Default)]
pub struct WifiPageState {
    pub selected_index: usize,
    pub filter: String,
    pub scanning: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ConnectionsPageState {
    pub selected_index: usize,
}

// ── Pending Action (for confirm dialog) ───────────────────────────────

#[derive(Debug, Clone)]
pub enum PendingAction {
    DeleteConnection(zbus::zvariant::OwnedObjectPath, String),
    DisconnectDevice(zbus::zvariant::OwnedObjectPath, String),
}

// ── Application State ─────────────────────────────────────────────────

pub struct App {
    pub active_page: Page,
    pub mode: Mode,
    pub should_quit: bool,
    pub theme: Theme,
    pub permission_level: PermissionLevel,

    // Network state
    pub network_state: Option<NetworkState>,
    pub nm: Arc<NetworkManager>,

    // Per-page state
    pub interfaces_state: InterfacesPageState,
    pub wifi_state: WifiPageState,
    pub connections_state: ConnectionsPageState,
    pub diagnostics_state: DiagnosticsState,

    // Dialogs
    pub input_dialog: InputDialog,
    pub confirm_dialog: ConfirmDialog,

    // Config
    pub show_help_bar: bool,

    // Toast notification
    pub toast_message: Option<String>,
    pub toast_is_error: bool,
    pub toast_ticks: u8,

    // Pending action for confirm dialog
    pub pending_action: Option<PendingAction>,

    // Event sender for async action results
    pub event_tx: mpsc::UnboundedSender<Event>,
}

impl App {
    pub fn new(
        nm: Arc<NetworkManager>,
        event_tx: mpsc::UnboundedSender<Event>,
        config: &crate::config::Config,
    ) -> Self {
        Self {
            active_page: Page::Dashboard,
            mode: Mode::Normal,
            should_quit: false,
            theme: Theme::default(),
            permission_level: PermissionLevel::Unknown,
            show_help_bar: config.show_help_bar,

            network_state: None,
            nm,

            interfaces_state: InterfacesPageState::default(),
            wifi_state: WifiPageState::default(),
            connections_state: ConnectionsPageState::default(),
            diagnostics_state: DiagnosticsState::default(),

            input_dialog: InputDialog::new("", "", false),
            confirm_dialog: ConfirmDialog::new(),

            toast_message: None,
            toast_is_error: false,
            toast_ticks: 0,

            pending_action: None,
            event_tx,
        }
    }

    /// Handle a key event, dispatching to the appropriate handler
    pub fn handle_key(&mut self, key: KeyEvent) {
        // Handle modal states first
        match self.mode {
            Mode::Input => {
                self.handle_input_key(key);
                return;
            }
            Mode::Dialog => {
                self.handle_dialog_key(key);
                return;
            }
            Mode::Filtering => {
                self.handle_filter_key(key);
                return;
            }
            Mode::Normal => {}
        }

        // Global keybindings
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
                return;
            }
            KeyCode::Tab => {
                self.next_page();
                return;
            }
            KeyCode::BackTab => {
                self.prev_page();
                return;
            }
            KeyCode::Char('1') => {
                self.active_page = Page::Dashboard;
                return;
            }
            KeyCode::Char('2') => {
                self.active_page = Page::Interfaces;
                return;
            }
            KeyCode::Char('3') => {
                self.active_page = Page::Wifi;
                return;
            }
            KeyCode::Char('4') => {
                self.active_page = Page::Connections;
                return;
            }
            KeyCode::Char('5') => {
                self.active_page = Page::Diagnostics;
                return;
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.trigger_refresh();
                return;
            }
            _ => {}
        }

        // Page-specific keybindings
        match self.active_page {
            Page::Dashboard => self.handle_dashboard_key(key),
            Page::Interfaces => self.handle_interfaces_key(key),
            Page::Wifi => self.handle_wifi_key(key),
            Page::Connections => self.handle_connections_key(key),
            Page::Diagnostics => self.handle_diagnostics_key(key),
        }
    }

    // ── Navigation ────────────────────────────────────────────────────

    fn next_page(&mut self) {
        self.active_page = match self.active_page {
            Page::Dashboard => Page::Interfaces,
            Page::Interfaces => Page::Wifi,
            Page::Wifi => Page::Connections,
            Page::Connections => Page::Diagnostics,
            Page::Diagnostics => Page::Dashboard,
        };
    }

    fn prev_page(&mut self) {
        self.active_page = match self.active_page {
            Page::Dashboard => Page::Diagnostics,
            Page::Interfaces => Page::Dashboard,
            Page::Wifi => Page::Interfaces,
            Page::Connections => Page::Wifi,
            Page::Diagnostics => Page::Connections,
        };
    }

    // ── Toast Management ──────────────────────────────────────────────

    pub fn show_toast(&mut self, message: &str, is_error: bool) {
        self.toast_message = Some(message.to_string());
        self.toast_is_error = is_error;
        self.toast_ticks = 12; // ~3 seconds at 250ms tick
    }

    pub fn tick_toast(&mut self) {
        if self.toast_ticks > 0 {
            self.toast_ticks -= 1;
            if self.toast_ticks == 0 {
                self.toast_message = None;
            }
        }
    }

    // ── Dashboard Keys ────────────────────────────────────────────────

    fn handle_dashboard_key(&mut self, _key: KeyEvent) {
        // Dashboard is mostly read-only, navigation handled globally
    }

    // ── Interfaces Keys ───────────────────────────────────────────────

    fn handle_interfaces_key(&mut self, key: KeyEvent) {
        let device_count = self
            .network_state
            .as_ref()
            .map(|s| s.devices.len())
            .unwrap_or(0);

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.interfaces_state.selected_index > 0 {
                    self.interfaces_state.selected_index -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if device_count > 0 && self.interfaces_state.selected_index < device_count - 1 {
                    self.interfaces_state.selected_index += 1;
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.interfaces_state.selected_index = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                if device_count > 0 {
                    self.interfaces_state.selected_index = device_count - 1;
                }
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.disconnect_selected_interface();
            }
            _ => {}
        }
    }

    fn disconnect_selected_interface(&mut self) {
        if !self.permission_level.can_write() {
            self.show_toast("Read-only mode — cannot disconnect", true);
            return;
        }

        let device = match self
            .network_state
            .as_ref()
            .and_then(|s| s.devices.get(self.interfaces_state.selected_index))
        {
            Some(d) => d,
            None => return,
        };

        if !device.state.is_connected() {
            self.show_toast("Device is not connected", true);
            return;
        }

        let path = device.path.clone();
        let name = device.interface.clone();

        self.pending_action = Some(PendingAction::DisconnectDevice(path, name.clone()));
        self.confirm_dialog
            .show("Disconnect", &format!("Disconnect {}?", name), None);
        self.mode = Mode::Dialog;
    }

    // ── WiFi Keys ─────────────────────────────────────────────────────

    fn handle_wifi_key(&mut self, key: KeyEvent) {
        let ap_count = self
            .network_state
            .as_ref()
            .map(|s| {
                if self.wifi_state.filter.is_empty() {
                    s.wifi_access_points.len()
                } else {
                    let fl = self.wifi_state.filter.to_lowercase();
                    s.wifi_access_points
                        .iter()
                        .filter(|ap| ap.ssid.to_lowercase().contains(&fl))
                        .count()
                }
            })
            .unwrap_or(0);

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.wifi_state.selected_index > 0 {
                    self.wifi_state.selected_index -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if ap_count > 0 && self.wifi_state.selected_index < ap_count - 1 {
                    self.wifi_state.selected_index += 1;
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.wifi_state.selected_index = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                if ap_count > 0 {
                    self.wifi_state.selected_index = ap_count - 1;
                }
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.trigger_wifi_scan();
            }
            KeyCode::Char('/') => {
                self.wifi_state.filter.clear();
                self.mode = Mode::Filtering;
            }
            KeyCode::Char('w') | KeyCode::Char('W') => {
                self.toggle_wireless();
            }
            KeyCode::Enter => {
                self.connect_selected_wifi();
            }
            _ => {}
        }
    }

    fn trigger_wifi_scan(&mut self) {
        self.wifi_state.scanning = true;
        let nm = self.nm.clone();
        let tx = self.event_tx.clone();

        tokio::spawn(async move {
            // Find WiFi device
            let devices = nm.list_devices().await.unwrap_or_default();
            for dev in &devices {
                if dev.device_type == DeviceType::WiFi {
                    let _ = nm.request_wifi_scan(&dev.path).await;
                }
            }
            // Wait for scan to complete, then trigger a refresh
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let _ = tx.send(Event::ActionSuccess("WiFi scan complete".into()));
        });
    }

    fn toggle_wireless(&mut self) {
        if !self.permission_level.can_write() {
            self.show_toast("Read-only mode — cannot toggle WiFi", true);
            return;
        }

        let currently_enabled = self
            .network_state
            .as_ref()
            .map(|s| s.wireless_enabled)
            .unwrap_or(false);

        let nm = self.nm.clone();
        let tx = self.event_tx.clone();
        let enable = !currently_enabled;

        tokio::spawn(async move {
            match nm.set_wireless_enabled(enable).await {
                Ok(()) => {
                    let msg = if enable {
                        "WiFi enabled"
                    } else {
                        "WiFi disabled"
                    };
                    let _ = tx.send(Event::ActionSuccess(msg.into()));
                }
                Err(e) => {
                    let _ = tx.send(Event::ActionError(format!("Failed to toggle WiFi: {}", e)));
                }
            }
        });
    }

    fn connect_selected_wifi(&mut self) {
        if !self.permission_level.can_write() {
            self.show_toast("Read-only mode — cannot connect", true);
            return;
        }

        let state = match &self.network_state {
            Some(s) => s,
            None => return,
        };

        // Get the filtered AP list (same logic as rendering)
        let filtered_aps: Vec<&WifiAccessPoint> = if self.wifi_state.filter.is_empty() {
            state.wifi_access_points.iter().collect()
        } else {
            let fl = self.wifi_state.filter.to_lowercase();
            state
                .wifi_access_points
                .iter()
                .filter(|ap| ap.ssid.to_lowercase().contains(&fl))
                .collect()
        };

        let ap = match filtered_aps.get(self.wifi_state.selected_index) {
            Some(ap) => (*ap).clone(),
            None => return,
        };

        if ap.is_active {
            self.show_toast("Already connected to this network", false);
            return;
        }

        // If it's a saved network, activate directly
        if ap.is_saved {
            self.activate_saved_wifi(&ap);
            return;
        }

        // If open network, connect without password
        if ap.security == WifiSecurity::Open {
            self.connect_wifi_direct(&ap, None);
            return;
        }

        // Need password — show input dialog
        self.input_dialog =
            InputDialog::new(&format!("Connect to {}", ap.ssid), "Enter password:", true);
        self.input_dialog.show();
        self.mode = Mode::Input;
    }

    fn activate_saved_wifi(&mut self, ap: &WifiAccessPoint) {
        let nm = self.nm.clone();
        let tx = self.event_tx.clone();
        let ssid = ap.ssid.clone();

        // We need to find the saved connection path and WiFi device path
        let state = self.network_state.as_ref().unwrap();
        let wifi_device = state
            .devices
            .iter()
            .find(|d| d.device_type == DeviceType::WiFi)
            .map(|d| d.path.clone());

        let saved_conn = state
            .saved_connections
            .iter()
            .find(|c| c.id == ap.ssid && c.conn_type == "802-11-wireless")
            .map(|c| c.path.clone());

        if let (Some(device_path), Some(conn_path)) = (wifi_device, saved_conn) {
            tokio::spawn(async move {
                match nm.activate_connection(&conn_path, &device_path).await {
                    Ok(_) => {
                        let _ = tx.send(Event::ActionSuccess(format!("Connecting to {}", ssid)));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::ActionError(format!(
                            "Failed to connect to {}: {}",
                            ssid, e
                        )));
                    }
                }
            });
        }
    }

    fn connect_wifi_direct(&mut self, ap: &WifiAccessPoint, password: Option<&str>) {
        let nm = self.nm.clone();
        let tx = self.event_tx.clone();
        let ssid = ap.ssid.clone();
        let ap_path = ap.path.clone();
        let security = ap.security;
        let password = password.map(String::from);

        let state = self.network_state.as_ref().unwrap();
        let wifi_device = state
            .devices
            .iter()
            .find(|d| d.device_type == DeviceType::WiFi)
            .map(|d| d.path.clone());

        if let Some(device_path) = wifi_device {
            tokio::spawn(async move {
                match nm
                    .connect_wifi(&device_path, &ap_path, &ssid, password.as_deref(), security)
                    .await
                {
                    Ok(_) => {
                        let _ = tx.send(Event::ActionSuccess(format!("Connecting to {}", ssid)));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::ActionError(format!(
                            "Failed to connect to {}: {}",
                            ssid, e
                        )));
                    }
                }
            });
        }
    }

    // ── Connections Keys ──────────────────────────────────────────────

    fn handle_connections_key(&mut self, key: KeyEvent) {
        let conn_count = self
            .network_state
            .as_ref()
            .map(|s| s.saved_connections.len())
            .unwrap_or(0);

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.connections_state.selected_index > 0 {
                    self.connections_state.selected_index -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if conn_count > 0 && self.connections_state.selected_index < conn_count - 1 {
                    self.connections_state.selected_index += 1;
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.connections_state.selected_index = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                if conn_count > 0 {
                    self.connections_state.selected_index = conn_count - 1;
                }
            }
            KeyCode::Enter => {
                self.activate_selected_connection();
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.deactivate_selected_connection();
            }
            KeyCode::Char('x') | KeyCode::Char('X') => {
                self.delete_selected_connection();
            }
            _ => {}
        }
    }

    fn activate_selected_connection(&mut self) {
        if !self.permission_level.can_write() {
            self.show_toast("Read-only mode — cannot activate", true);
            return;
        }

        let state = match &self.network_state {
            Some(s) => s,
            None => return,
        };

        let conn = match state
            .saved_connections
            .get(self.connections_state.selected_index)
        {
            Some(c) => c,
            None => return,
        };

        // Already active?
        let active_uuids: std::collections::HashSet<String> = state
            .active_connections
            .iter()
            .map(|ac| ac.uuid.clone())
            .collect();
        if active_uuids.contains(&conn.uuid) {
            self.show_toast("Connection is already active", false);
            return;
        }

        // Find a suitable device
        let device_path = state
            .devices
            .iter()
            .find(|d| {
                let type_matches = match conn.conn_type.as_str() {
                    "802-11-wireless" => d.device_type == DeviceType::WiFi,
                    "802-3-ethernet" => d.device_type == DeviceType::Ethernet,
                    _ => false,
                };
                type_matches
                    && (conn.interface.is_none() || conn.interface.as_deref() == Some(&d.interface))
            })
            .map(|d| d.path.clone());

        let device_path = match device_path {
            Some(p) => p,
            None => {
                self.show_toast("No suitable device found for this connection", true);
                return;
            }
        };

        let nm = self.nm.clone();
        let tx = self.event_tx.clone();
        let conn_path = conn.path.clone();
        let conn_name = conn.id.clone();

        tokio::spawn(async move {
            match nm.activate_connection(&conn_path, &device_path).await {
                Ok(_) => {
                    let _ = tx.send(Event::ActionSuccess(format!("Activating {}", conn_name)));
                }
                Err(e) => {
                    let _ = tx.send(Event::ActionError(format!(
                        "Failed to activate {}: {}",
                        conn_name, e
                    )));
                }
            }
        });
    }

    fn deactivate_selected_connection(&mut self) {
        if !self.permission_level.can_write() {
            self.show_toast("Read-only mode — cannot deactivate", true);
            return;
        }

        let state = match &self.network_state {
            Some(s) => s,
            None => return,
        };

        let conn = match state
            .saved_connections
            .get(self.connections_state.selected_index)
        {
            Some(c) => c,
            None => return,
        };

        // Find active connection for this UUID
        let active = state
            .active_connections
            .iter()
            .find(|ac| ac.uuid == conn.uuid);

        let active = match active {
            Some(ac) => ac,
            None => {
                self.show_toast("Connection is not active", true);
                return;
            }
        };

        let nm = self.nm.clone();
        let tx = self.event_tx.clone();
        let path = active.path.clone();
        let name = conn.id.clone();

        tokio::spawn(async move {
            match nm.deactivate_connection(&path).await {
                Ok(_) => {
                    let _ = tx.send(Event::ActionSuccess(format!("Deactivated {}", name)));
                }
                Err(e) => {
                    let _ = tx.send(Event::ActionError(format!(
                        "Failed to deactivate {}: {}",
                        name, e
                    )));
                }
            }
        });
    }

    fn delete_selected_connection(&mut self) {
        if !self.permission_level.can_write() {
            self.show_toast("Read-only mode — cannot delete", true);
            return;
        }

        let state = match &self.network_state {
            Some(s) => s,
            None => return,
        };

        let conn = match state
            .saved_connections
            .get(self.connections_state.selected_index)
        {
            Some(c) => c,
            None => return,
        };

        let path = conn.path.clone();
        let name = conn.id.clone();

        self.pending_action = Some(PendingAction::DeleteConnection(path, name.clone()));
        self.confirm_dialog.show(
            "Delete Connection",
            &format!("Delete '{}'? This cannot be undone.", name),
            None,
        );
        self.mode = Mode::Dialog;
    }

    // ── Diagnostics Keys ──────────────────────────────────────────────

    fn handle_diagnostics_key(&mut self, key: KeyEvent) {
        let tool_count = DiagnosticTool::all().len();

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.diagnostics_state.selected_tool > 0 {
                    self.diagnostics_state.selected_tool -= 1;
                    self.diagnostics_state.output.clear();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.diagnostics_state.selected_tool < tool_count - 1 {
                    self.diagnostics_state.selected_tool += 1;
                    self.diagnostics_state.output.clear();
                }
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.diagnostics_state.output.clear();
            }
            KeyCode::Enter => {
                self.run_diagnostic();
            }
            _ => {}
        }
    }

    fn run_diagnostic(&mut self) {
        let tool = DiagnosticTool::all()
            .get(self.diagnostics_state.selected_tool)
            .copied()
            .unwrap_or(DiagnosticTool::Ping);

        match tool {
            DiagnosticTool::Ping | DiagnosticTool::DnsLookup => {
                // Need target input
                let (title, prompt) = match tool {
                    DiagnosticTool::Ping => ("Ping", "Enter host to ping:"),
                    DiagnosticTool::DnsLookup => ("DNS Lookup", "Enter domain to resolve:"),
                    _ => unreachable!(),
                };
                self.input_dialog = InputDialog::new(title, prompt, false);
                self.input_dialog.show();
                self.mode = Mode::Input;
            }
            DiagnosticTool::RouteTable => {
                self.run_route_table();
            }
            DiagnosticTool::DnsServers => {
                self.show_dns_servers();
            }
            DiagnosticTool::InterfaceStats => {
                self.show_interface_stats();
            }
        }
    }

    fn run_route_table(&mut self) {
        self.diagnostics_state.output.clear();
        self.diagnostics_state
            .output
            .push_back(">>> Route Table".into());
        self.diagnostics_state.output.push_back("---".into());

        self.diagnostics_state.running = true;
        let tx = self.event_tx.clone();

        tokio::spawn(async move {
            let output = tokio::process::Command::new("ip")
                .args(["route", "show"])
                .output()
                .await;

            match output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    for line in stdout.lines() {
                        let _ = tx.send(Event::ActionSuccess(format!("DIAG:{}", line)));
                    }
                    let _ = tx.send(Event::ActionSuccess("DIAG_DONE:route".into()));
                }
                Err(e) => {
                    let _ = tx.send(Event::ActionError(format!("Failed: {}", e)));
                }
            }
        });
    }

    fn show_dns_servers(&mut self) {
        self.diagnostics_state.output.clear();
        self.diagnostics_state
            .output
            .push_back(">>> DNS Servers".into());
        self.diagnostics_state.output.push_back("---".into());

        if let Some(ref state) = self.network_state {
            for server in &state.dns.servers {
                self.diagnostics_state
                    .output
                    .push_back(format!("  {}", server));
            }
            if !state.dns.search_domains.is_empty() {
                self.diagnostics_state.output.push_back("".into());
                self.diagnostics_state
                    .output
                    .push_back(">>> Search Domains".into());
                for domain in &state.dns.search_domains {
                    self.diagnostics_state
                        .output
                        .push_back(format!("  {}", domain));
                }
            }
        }
    }

    fn show_interface_stats(&mut self) {
        self.diagnostics_state.output.clear();
        self.diagnostics_state
            .output
            .push_back(">>> Interface Statistics".into());
        self.diagnostics_state.output.push_back("---".into());

        if let Some(ref state) = self.network_state {
            for dev in &state.devices {
                if let Some(stats) = state.stats.get(&dev.interface) {
                    self.diagnostics_state.output.push_back(format!(
                        "  {} ({}):",
                        dev.interface,
                        dev.device_type.label()
                    ));
                    self.diagnostics_state.output.push_back(format!(
                        "    RX: {} ({}/s)  Packets: {}  Errors: {}  Dropped: {}",
                        InterfaceStats::format_bytes(stats.rx_bytes),
                        InterfaceStats::format_rate(stats.rx_rate),
                        stats.rx_packets,
                        stats.rx_errors,
                        stats.rx_dropped,
                    ));
                    self.diagnostics_state.output.push_back(format!(
                        "    TX: {} ({}/s)  Packets: {}  Errors: {}  Dropped: {}",
                        InterfaceStats::format_bytes(stats.tx_bytes),
                        InterfaceStats::format_rate(stats.tx_rate),
                        stats.tx_packets,
                        stats.tx_errors,
                        stats.tx_dropped,
                    ));
                    self.diagnostics_state.output.push_back("".into());
                }
            }
        }
    }

    fn run_ping(&mut self, target: &str) {
        self.diagnostics_state.target = target.to_string();
        self.diagnostics_state.output.clear();
        self.diagnostics_state
            .output
            .push_back(format!(">>> Ping {}", target));
        self.diagnostics_state.output.push_back("---".into());
        self.diagnostics_state.running = true;

        let tx = self.event_tx.clone();
        let target = target.to_string();

        tokio::spawn(async move {
            let output = tokio::process::Command::new("ping")
                .args(["-c", "5", "-W", "2", &target])
                .output()
                .await;

            match output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    for line in stdout.lines().chain(stderr.lines()) {
                        let _ = tx.send(Event::ActionSuccess(format!("DIAG:{}", line)));
                    }
                    let _ = tx.send(Event::ActionSuccess("DIAG_DONE:ping".into()));
                }
                Err(e) => {
                    let _ = tx.send(Event::ActionError(format!("Ping failed: {}", e)));
                }
            }
        });
    }

    fn run_dns_lookup(&mut self, domain: &str) {
        self.diagnostics_state.target = domain.to_string();
        self.diagnostics_state.output.clear();
        self.diagnostics_state
            .output
            .push_back(format!(">>> DNS Lookup: {}", domain));
        self.diagnostics_state.output.push_back("---".into());

        match dns_lookup::lookup_host(domain) {
            Ok(addrs) => {
                for addr in addrs {
                    self.diagnostics_state
                        .output
                        .push_back(format!("  {}", addr));
                }
            }
            Err(e) => {
                self.diagnostics_state
                    .output
                    .push_back(format!("  Error: {}", e));
            }
        }
    }

    // ── Input Dialog Handler ──────────────────────────────────────────

    fn handle_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input_dialog.hide();
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                let value = self.input_dialog.value().to_string();
                let title = self.input_dialog.title.clone();
                self.input_dialog.hide();
                self.mode = Mode::Normal;
                self.on_input_submit(&title, &value);
            }
            KeyCode::Backspace => {
                self.input_dialog.delete_char();
            }
            KeyCode::Delete => {
                self.input_dialog.delete_forward();
            }
            KeyCode::Left => {
                self.input_dialog.move_left();
            }
            KeyCode::Right => {
                self.input_dialog.move_right();
            }
            KeyCode::Home => {
                self.input_dialog.move_start();
            }
            KeyCode::End => {
                self.input_dialog.move_end();
            }
            KeyCode::Char(c) => {
                self.input_dialog.insert_char(c);
            }
            _ => {}
        }
    }

    fn on_input_submit(&mut self, title: &str, value: &str) {
        if value.is_empty() {
            return;
        }

        match title {
            t if t.starts_with("Connect to ") => {
                // WiFi password submission
                let ssid = t.strip_prefix("Connect to ").unwrap_or("");
                if let Some(state) = &self.network_state {
                    if let Some(ap) = state
                        .wifi_access_points
                        .iter()
                        .find(|a| a.ssid == ssid)
                        .cloned()
                    {
                        self.connect_wifi_direct(&ap, Some(value));
                    }
                }
            }
            "Ping" => {
                self.run_ping(value);
            }
            "DNS Lookup" => {
                self.run_dns_lookup(value);
            }
            _ => {}
        }
    }

    // ── Confirm Dialog Handler ────────────────────────────────────────

    fn handle_dialog_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.confirm_dialog.hide();
                self.mode = Mode::Normal;
                self.on_confirm();
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.confirm_dialog.hide();
                self.pending_action = None;
                self.mode = Mode::Normal;
            }
            _ => {}
        }
    }

    fn on_confirm(&mut self) {
        let action = match self.pending_action.take() {
            Some(a) => a,
            None => return,
        };

        match action {
            PendingAction::DeleteConnection(path, name) => {
                let nm = self.nm.clone();
                let tx = self.event_tx.clone();
                tokio::spawn(async move {
                    match nm.delete_connection(&path).await {
                        Ok(_) => {
                            let _ = tx.send(Event::ActionSuccess(format!("Deleted {}", name)));
                        }
                        Err(e) => {
                            let _ = tx.send(Event::ActionError(format!(
                                "Failed to delete {}: {}",
                                name, e
                            )));
                        }
                    }
                });
            }
            PendingAction::DisconnectDevice(path, name) => {
                let nm = self.nm.clone();
                let tx = self.event_tx.clone();
                tokio::spawn(async move {
                    match nm.disconnect_device(&path).await {
                        Ok(_) => {
                            let _ = tx.send(Event::ActionSuccess(format!("Disconnected {}", name)));
                        }
                        Err(e) => {
                            let _ = tx.send(Event::ActionError(format!(
                                "Failed to disconnect {}: {}",
                                name, e
                            )));
                        }
                    }
                });
            }
        }
    }

    // ── Filter Handler ────────────────────────────────────────────────

    fn handle_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.wifi_state.filter.clear();
                self.wifi_state.selected_index = 0;
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                self.wifi_state.selected_index = 0;
                self.mode = Mode::Normal;
            }
            KeyCode::Backspace => {
                self.wifi_state.filter.pop();
                self.wifi_state.selected_index = 0;
            }
            KeyCode::Char(c) => {
                self.wifi_state.filter.push(c);
                self.wifi_state.selected_index = 0;
            }
            _ => {}
        }
    }

    // ── Refresh ───────────────────────────────────────────────────────

    fn trigger_refresh(&self) {
        let nm = self.nm.clone();
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let state = nm.snapshot().await;
            let _ = tx.send(Event::NetworkRefresh(Box::new(state)));
        });
    }

    /// Handle events from the event loop
    pub fn handle_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => self.handle_key(key),
            Event::Tick => {
                self.tick_toast();
            }
            Event::NetworkRefresh(state) => {
                self.network_state = Some(*state);
                self.wifi_state.scanning = false;

                // Clamp selection indices
                if let Some(ref s) = self.network_state {
                    if self.interfaces_state.selected_index >= s.devices.len()
                        && !s.devices.is_empty()
                    {
                        self.interfaces_state.selected_index = s.devices.len() - 1;
                    }
                    if self.wifi_state.selected_index >= s.wifi_access_points.len()
                        && !s.wifi_access_points.is_empty()
                    {
                        self.wifi_state.selected_index = s.wifi_access_points.len() - 1;
                    }
                    if self.connections_state.selected_index >= s.saved_connections.len()
                        && !s.saved_connections.is_empty()
                    {
                        self.connections_state.selected_index = s.saved_connections.len() - 1;
                    }
                }
            }
            Event::ActionSuccess(msg) => {
                // Handle diagnostic output streaming
                if let Some(stripped) = msg.strip_prefix("DIAG:") {
                    self.diagnostics_state
                        .output
                        .push_back(stripped.to_string());
                    // Keep output bounded
                    while self.diagnostics_state.output.len() > 200 {
                        self.diagnostics_state.output.pop_front();
                    }
                } else if msg.starts_with("DIAG_DONE:") {
                    self.diagnostics_state.running = false;
                } else {
                    self.show_toast(&msg, false);
                    self.trigger_refresh();
                }
            }
            Event::ActionError(msg) => {
                if msg.contains("PermissionDenied") || msg.contains("permission") {
                    self.permission_level = PermissionLevel::ReadOnly;
                }
                self.show_toast(&msg, true);
                self.diagnostics_state.running = false;
            }
            Event::Mouse(_) => {
                // Mouse events are captured but not yet handled;
                // reserved for future interactive click support
            }
            Event::Resize(_w, _h) => {
                // Terminal resized — ratatui redraws automatically
            }
        }
    }
}
