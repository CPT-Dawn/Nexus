use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::Ipv4Addr;

// ── Device Types ──────────────────────────────────────────────────────

/// NetworkManager device type (maps to NM_DEVICE_TYPE enum)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceType {
    Unknown,
    Ethernet,
    WiFi,
    Bridge,
    Bond,
    Vlan,
    Tunnel,
    Loopback,
    WireGuard,
    Other(u32),
}

impl DeviceType {
    pub fn from_nm(val: u32) -> Self {
        match val {
            0 => Self::Unknown,
            1 => Self::Ethernet,
            2 => Self::WiFi,
            13 => Self::Bridge,
            10 => Self::Bond,
            11 => Self::Vlan,
            16 => Self::Tunnel,
            14 => Self::Loopback,
            29 => Self::WireGuard,
            other => Self::Other(other),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Ethernet => "󰈀 ",
            Self::WiFi => "󰖩 ",
            Self::Bridge => "󰌘 ",
            Self::Bond => "󰌗 ",
            Self::Loopback => "󰑙 ",
            Self::WireGuard => "󰖂 ",
            Self::Tunnel => "󰕥 ",
            Self::Vlan => "󰕓 ",
            _ => "󰛳 ",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Ethernet => "Ethernet",
            Self::WiFi => "WiFi",
            Self::Bridge => "Bridge",
            Self::Bond => "Bond",
            Self::Vlan => "VLAN",
            Self::Tunnel => "Tunnel",
            Self::Loopback => "Loopback",
            Self::WireGuard => "WireGuard",
            Self::Other(_) => "Other",
        }
    }
}

impl fmt::Display for DeviceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ── Device State ──────────────────────────────────────────────────────

/// NetworkManager device state (maps to NM_DEVICE_STATE enum)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceState {
    Unknown,
    Unmanaged,
    Unavailable,
    Disconnected,
    Prepare,
    Config,
    NeedAuth,
    IpConfig,
    IpCheck,
    Secondaries,
    Activated,
    Deactivating,
    Failed,
}

impl DeviceState {
    pub fn from_nm(val: u32) -> Self {
        match val {
            0 => Self::Unknown,
            10 => Self::Unmanaged,
            20 => Self::Unavailable,
            30 => Self::Disconnected,
            40 => Self::Prepare,
            50 => Self::Config,
            60 => Self::NeedAuth,
            70 => Self::IpConfig,
            80 => Self::IpCheck,
            90 => Self::Secondaries,
            100 => Self::Activated,
            110 => Self::Deactivating,
            120 => Self::Failed,
            _ => Self::Unknown,
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Activated)
    }

    pub fn is_connecting(&self) -> bool {
        matches!(
            self,
            Self::Prepare
                | Self::Config
                | Self::NeedAuth
                | Self::IpConfig
                | Self::IpCheck
                | Self::Secondaries
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Unmanaged => "Unmanaged",
            Self::Unavailable => "Unavailable",
            Self::Disconnected => "Disconnected",
            Self::Prepare => "Preparing",
            Self::Config => "Configuring",
            Self::NeedAuth => "Auth Required",
            Self::IpConfig => "Getting IP",
            Self::IpCheck => "Checking IP",
            Self::Secondaries => "Secondaries",
            Self::Activated => "Connected",
            Self::Deactivating => "Disconnecting",
            Self::Failed => "Failed",
        }
    }
}

impl fmt::Display for DeviceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ── Connectivity State ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectivityState {
    Unknown,
    None,
    Portal,
    Limited,
    Full,
}

impl ConnectivityState {
    pub fn from_nm(val: u32) -> Self {
        match val {
            0 => Self::Unknown,
            1 => Self::None,
            2 => Self::Portal,
            3 => Self::Limited,
            4 => Self::Full,
            _ => Self::Unknown,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::None => "No Connectivity",
            Self::Portal => "Captive Portal",
            Self::Limited => "Limited",
            Self::Full => "Full",
        }
    }
}

impl fmt::Display for ConnectivityState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ── Device Info ───────────────────────────────────────────────────────

/// Summarized info about a network device
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub path: zbus::zvariant::OwnedObjectPath,
    pub interface: String,
    pub device_type: DeviceType,
    pub state: DeviceState,
    pub hw_address: String,
    pub ip4_address: Option<String>,
    pub ip4_gateway: Option<String>,
    pub ip4_dns: Vec<String>,
    pub ip4_subnet: Option<String>,
    pub ip6_address: Option<String>,
    pub mtu: u32,
    pub speed: u32, // Mbps, 0 if unknown
    pub driver: String,
    pub active_connection_path: Option<zbus::zvariant::OwnedObjectPath>,
    pub connection_name: Option<String>,
    pub autoconnect: bool,
}

impl DeviceInfo {
    pub fn display_ip(&self) -> String {
        self.ip4_address.clone().unwrap_or_else(|| "—".to_string())
    }
}

// ── WiFi Access Point ─────────────────────────────────────────────────

/// WiFi security type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WifiSecurity {
    Open,
    WEP,
    WPA,
    WPA2,
    WPA3,
    Enterprise,
    Unknown,
}

impl WifiSecurity {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::WEP => "WEP",
            Self::WPA => "WPA",
            Self::WPA2 => "WPA2",
            Self::WPA3 => "WPA3",
            Self::Enterprise => "802.1X",
            Self::Unknown => "?",
        }
    }

    /// Determine security from NM AP flags
    pub fn from_nm_flags(flags: u32, wpa_flags: u32, rsn_flags: u32) -> Self {
        if rsn_flags != 0 {
            if rsn_flags & 0x200 != 0 {
                return Self::Enterprise;
            }
            // RSN with SAE = WPA3
            if rsn_flags & 0x400 != 0 {
                return Self::WPA3;
            }
            return Self::WPA2;
        }
        if wpa_flags != 0 {
            if wpa_flags & 0x200 != 0 {
                return Self::Enterprise;
            }
            return Self::WPA;
        }
        if flags & 0x1 != 0 {
            return Self::WEP;
        }
        Self::Open
    }
}

impl fmt::Display for WifiSecurity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Information about a WiFi access point
#[derive(Debug, Clone)]
pub struct WifiAccessPoint {
    pub path: zbus::zvariant::OwnedObjectPath,
    pub ssid: String,
    pub bssid: String,
    pub frequency: u32, // MHz
    pub channel: u32,
    pub strength: u8, // 0-100
    pub security: WifiSecurity,
    pub is_saved: bool,
    pub is_active: bool,
}

impl WifiAccessPoint {
    /// Convert frequency in MHz to channel number
    pub fn freq_to_channel(freq: u32) -> u32 {
        match freq {
            2412 => 1,
            2417 => 2,
            2422 => 3,
            2427 => 4,
            2432 => 5,
            2437 => 6,
            2442 => 7,
            2447 => 8,
            2452 => 9,
            2457 => 10,
            2462 => 11,
            2467 => 12,
            2472 => 13,
            2484 => 14,
            f if (5170..=5825).contains(&f) => (f - 5000) / 5,
            f if (5955..=7115).contains(&f) => (f - 5950) / 5, // WiFi 6E
            _ => 0,
        }
    }

    /// Get signal strength as bars (1-4)
    pub fn signal_bars(&self) -> &'static str {
        match self.strength {
            0..=24 => "▂   ",
            25..=49 => "▂▄  ",
            50..=74 => "▂▄▆ ",
            _ => "▂▄▆█",
        }
    }

    /// Get the band label
    pub fn band(&self) -> &'static str {
        match self.frequency {
            2400..=2500 => "2.4G",
            5000..=5899 => "5G",
            5900..=7200 => "6G",
            _ => "?",
        }
    }
}

// ── Connection Profile ────────────────────────────────────────────────

/// A saved NetworkManager connection profile
#[derive(Debug, Clone)]
pub struct ConnectionProfile {
    pub path: zbus::zvariant::OwnedObjectPath,
    pub id: String, // Human-readable name
    pub uuid: String,
    pub conn_type: String, // "802-11-wireless", "802-3-ethernet", etc.
    pub interface: Option<String>,
    pub autoconnect: bool,
    pub timestamp: u64, // Last used (Unix timestamp)
}

impl ConnectionProfile {
    pub fn type_label(&self) -> &'static str {
        match self.conn_type.as_str() {
            "802-11-wireless" => "WiFi",
            "802-3-ethernet" => "Ethernet",
            "vpn" => "VPN",
            "wireguard" => "WireGuard",
            "bridge" => "Bridge",
            "bond" => "Bond",
            "vlan" => "VLAN",
            "loopback" => "Loopback",
            _ => "Other",
        }
    }

    pub fn type_icon(&self) -> &'static str {
        match self.conn_type.as_str() {
            "802-11-wireless" => "󰖩 ",
            "802-3-ethernet" => "󰈀 ",
            "vpn" => "󰖂 ",
            "wireguard" => "󰖂 ",
            "bridge" => "󰌘 ",
            _ => "󰛳 ",
        }
    }
}

// ── Active Connection ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ActiveConnection {
    pub path: zbus::zvariant::OwnedObjectPath,
    pub id: String,
    pub uuid: String,
    pub conn_type: String,
    pub state: ActiveConnectionState,
    pub devices: Vec<zbus::zvariant::OwnedObjectPath>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveConnectionState {
    Unknown,
    Activating,
    Activated,
    Deactivating,
    Deactivated,
}

impl ActiveConnectionState {
    pub fn from_nm(val: u32) -> Self {
        match val {
            0 => Self::Unknown,
            1 => Self::Activating,
            2 => Self::Activated,
            3 => Self::Deactivating,
            4 => Self::Deactivated,
            _ => Self::Unknown,
        }
    }
}

// ── Interface Statistics ──────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct InterfaceStats {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
    pub rx_dropped: u64,
    pub tx_dropped: u64,
    /// Bytes per second (computed from deltas)
    pub rx_rate: f64,
    pub tx_rate: f64,
    /// Historical rates for sparkline (last 60 samples)
    pub rx_history: Vec<f64>,
    pub tx_history: Vec<f64>,
}

impl InterfaceStats {
    pub fn format_bytes(bytes: u64) -> String {
        const KIB: u64 = 1024;
        const MIB: u64 = 1024 * 1024;
        const GIB: u64 = 1024 * 1024 * 1024;

        if bytes >= GIB {
            format!("{:.1} GiB", bytes as f64 / GIB as f64)
        } else if bytes >= MIB {
            format!("{:.1} MiB", bytes as f64 / MIB as f64)
        } else if bytes >= KIB {
            format!("{:.1} KiB", bytes as f64 / KIB as f64)
        } else {
            format!("{} B", bytes)
        }
    }

    pub fn format_rate(bytes_per_sec: f64) -> String {
        if bytes_per_sec >= 1_073_741_824.0 {
            format!("{:.1} GiB/s", bytes_per_sec / 1_073_741_824.0)
        } else if bytes_per_sec >= 1_048_576.0 {
            format!("{:.1} MiB/s", bytes_per_sec / 1_048_576.0)
        } else if bytes_per_sec >= 1024.0 {
            format!("{:.1} KiB/s", bytes_per_sec / 1024.0)
        } else {
            format!("{:.0} B/s", bytes_per_sec)
        }
    }
}

// ── DNS Info ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DnsInfo {
    pub servers: Vec<String>,
    pub search_domains: Vec<String>,
}

// ── Route Entry ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RouteEntry {
    pub destination: String,
    pub gateway: String,
    pub metric: u32,
    pub interface: String,
}

impl RouteEntry {
    /// Parse a route line from `ip route show` output
    pub fn parse_route_line(line: &str) -> Option<Self> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        let destination = parts[0].to_string();
        let gateway = parts
            .iter()
            .position(|&p| p == "via")
            .and_then(|i| parts.get(i + 1))
            .map(|s| s.to_string())
            .unwrap_or_else(|| "*".to_string());
        let metric = parts
            .iter()
            .position(|&p| p == "metric")
            .and_then(|i| parts.get(i + 1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let interface = parts
            .iter()
            .position(|&p| p == "dev")
            .and_then(|i| parts.get(i + 1))
            .map(|s| s.to_string())
            .unwrap_or_default();

        Some(RouteEntry {
            destination,
            gateway,
            metric,
            interface,
        })
    }

    /// Convert legacy NM IPv4 address tuples to route entries
    pub fn from_legacy_nm_addresses(addrs: &[(u32, u32, u32)]) -> Vec<Self> {
        nm_ip4_addresses_to_strings(addrs)
            .into_iter()
            .map(|(addr, prefix, gw)| RouteEntry {
                destination: format!("{}/{}", addr, prefix),
                gateway: gw,
                metric: 0,
                interface: String::new(),
            })
            .collect()
    }
}

// ── Network State Snapshot ────────────────────────────────────────────

/// Complete snapshot of network state, cached and refreshed periodically
#[derive(Debug, Clone)]
pub struct NetworkState {
    pub connectivity: ConnectivityState,
    pub wireless_enabled: bool,
    pub networking_enabled: bool,
    pub devices: Vec<DeviceInfo>,
    pub active_connections: Vec<ActiveConnection>,
    pub wifi_access_points: Vec<WifiAccessPoint>,
    pub saved_connections: Vec<ConnectionProfile>,
    pub stats: std::collections::HashMap<String, InterfaceStats>,
    pub dns: DnsInfo,
    pub hostname: String,
    pub nm_version: String,
    pub nm_running: bool,
}

impl Default for NetworkState {
    fn default() -> Self {
        Self {
            connectivity: ConnectivityState::Unknown,
            wireless_enabled: false,
            networking_enabled: false,
            devices: Vec::new(),
            active_connections: Vec::new(),
            wifi_access_points: Vec::new(),
            saved_connections: Vec::new(),
            stats: std::collections::HashMap::new(),
            dns: DnsInfo {
                servers: Vec::new(),
                search_domains: Vec::new(),
            },
            hostname: String::new(),
            nm_version: String::new(),
            nm_running: false,
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────

/// Convert NM's IP address format (u32 in network byte order) to dotted notation
pub fn nm_ip4_to_string(addr: u32) -> String {
    Ipv4Addr::from(addr.to_be()).to_string()
}

/// Convert NM's IP address array format: [[addr, prefix, gateway], ...]
/// Each element is (address: u32, prefix: u32, gateway: u32) in network byte order
pub fn nm_ip4_addresses_to_strings(addrs: &[(u32, u32, u32)]) -> Vec<(String, u32, String)> {
    addrs
        .iter()
        .map(|(addr, prefix, gw)| (nm_ip4_to_string(*addr), *prefix, nm_ip4_to_string(*gw)))
        .collect()
}
