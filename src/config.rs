use std::time::Duration;

/// Runtime configuration for Nexus-NM
#[derive(Debug, Clone)]
pub struct Config {
    /// TUI tick rate for refreshing the UI
    pub tick_rate: Duration,
    /// How often to poll network statistics
    pub stats_poll_interval: Duration,
    /// How often to refresh WiFi scan results
    pub wifi_scan_interval: Duration,
    /// Path for log file output
    pub log_file: Option<String>,
    /// Whether to enable mouse support
    pub mouse_support: bool,
    /// Whether to show the help bar at the bottom
    pub show_help_bar: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tick_rate: Duration::from_millis(250),
            stats_poll_interval: Duration::from_secs(1),
            wifi_scan_interval: Duration::from_secs(10),
            log_file: None,
            mouse_support: true,
            show_help_bar: true,
        }
    }
}
