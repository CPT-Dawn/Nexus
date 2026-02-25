use std::fmt;

/// Security type of a WiFi network
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SecurityType {
    Open,
    WEP,
    WPA,
    WPA2,
    WPA3,
    WPA2Enterprise,
    Unknown,
}

impl fmt::Display for SecurityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open => write!(f, "Open"),
            Self::WEP => write!(f, "WEP"),
            Self::WPA => write!(f, "WPA"),
            Self::WPA2 => write!(f, "WPA2"),
            Self::WPA3 => write!(f, "WPA3"),
            Self::WPA2Enterprise => write!(f, "WPA2-EAP"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

impl SecurityType {
    pub fn needs_password(&self) -> bool {
        !matches!(self, Self::Open)
    }

    pub fn from_flags(flags: u32, wpa_flags: u32, rsn_flags: u32) -> Self {
        if rsn_flags != 0 {
            // RSN = WPA2/WPA3
            if rsn_flags & 0x200 != 0 {
                return Self::WPA2Enterprise;
            }
            // SAE = WPA3
            if rsn_flags & 0x400 != 0 {
                return Self::WPA3;
            }
            return Self::WPA2;
        }
        if wpa_flags != 0 {
            if wpa_flags & 0x200 != 0 {
                return Self::WPA2Enterprise;
            }
            return Self::WPA;
        }
        // NM80211ApFlags: Privacy = 0x1
        if flags & 0x1 != 0 {
            return Self::WEP;
        }
        Self::Open
    }
}

/// Signal strength level for display
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SignalLevel {
    None,
    Weak,
    Fair,
    Good,
    Excellent,
}

impl SignalLevel {
    pub fn from_percentage(pct: u8) -> Self {
        match pct {
            0..=19 => Self::None,
            20..=39 => Self::Weak,
            40..=59 => Self::Fair,
            60..=79 => Self::Good,
            80..=100 => Self::Excellent,
            _ => Self::Excellent,
        }
    }
}

/// Frequency band
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrequencyBand {
    TwoGhz,
    FiveGhz,
    SixGhz,
    Unknown,
}

impl FrequencyBand {
    pub fn from_mhz(freq: u32) -> Self {
        match freq {
            2400..=2500 => Self::TwoGhz,
            5000..=5900 => Self::FiveGhz,
            5925..=7125 => Self::SixGhz,
            _ => Self::Unknown,
        }
    }
}

impl fmt::Display for FrequencyBand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TwoGhz => write!(f, "2.4 GHz"),
            Self::FiveGhz => write!(f, "5 GHz"),
            Self::SixGhz => write!(f, "6 GHz"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Compute WiFi channel from frequency in MHz
pub fn channel_from_frequency(freq: u32) -> u32 {
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
        // 5 GHz: channel = (freq - 5000) / 5
        f if (5000..=5900).contains(&f) => (f - 5000) / 5,
        // 6 GHz: channel = (freq - 5950) / 5
        f if (5925..=7125).contains(&f) => (f - 5950) / 5,
        _ => 0,
    }
}

/// A visible WiFi network (access point)
#[derive(Debug, Clone)]
pub struct WiFiNetwork {
    pub ssid: String,
    pub bssid: String,
    pub signal_strength: u8,
    pub frequency: u32,
    pub security: SecurityType,
    pub is_saved: bool,
    pub is_active: bool,
    /// D-Bus object path for the AP
    pub ap_path: String,
    /// Animation: ticks since this network was first seen (for fade-in)
    pub seen_ticks: u16,
    /// Smoothed signal strength for animation
    pub display_signal: f32,
}

impl WiFiNetwork {
    pub fn channel(&self) -> u32 {
        channel_from_frequency(self.frequency)
    }

    pub fn band(&self) -> FrequencyBand {
        FrequencyBand::from_mhz(self.frequency)
    }

    pub fn signal_level(&self) -> SignalLevel {
        SignalLevel::from_percentage(self.signal_strength)
    }
}

/// Information about the current active connection
#[derive(Debug, Clone, Default)]
pub struct ConnectionInfo {
    pub ssid: String,
    pub bssid: String,
    pub ip4: Option<String>,
    pub ip6: Option<String>,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
    pub mac: String,
    pub speed: u32,
    pub frequency: u32,
    pub signal: u8,
    pub interface: String,
}

/// Overall connection status
#[derive(Debug, Clone)]
pub enum ConnectionStatus {
    Connected(ConnectionInfo),
    Connecting(String),
    Disconnecting,
    Disconnected,
    Failed(String),
}

impl Default for ConnectionStatus {
    fn default() -> Self {
        Self::Disconnected
    }
}

impl ConnectionStatus {
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected(_))
    }

    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Connecting(_) | Self::Disconnecting)
    }

    pub fn ssid(&self) -> Option<&str> {
        match self {
            Self::Connected(info) => Some(&info.ssid),
            Self::Connecting(ssid) => Some(ssid),
            _ => None,
        }
    }
}

/// Events from the network backend
#[derive(Debug, Clone)]
pub enum NetworkEvent {
    ScanComplete(Vec<WiFiNetwork>),
    ConnectionChanged(ConnectionStatus),
    AccessPointAdded,
    AccessPointRemoved,
    Error(String),
}
