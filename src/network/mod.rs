pub mod manager;
pub mod signals;
pub mod types;

use eyre::Result;
use types::{ConnectionInfo, WiFiNetwork};

/// Abstract network backend trait.
/// Allows swapping implementations (NetworkManager, iwd, mock) cleanly.
pub trait NetworkBackend: Send + Sync {
    /// Trigger a WiFi scan and return discovered networks
    async fn scan(&self) -> Result<Vec<WiFiNetwork>>;

    /// Connect to a network by SSID, optionally with a password
    async fn connect(&self, ssid: &str, password: Option<&str>) -> Result<()>;

    /// Disconnect from the currently active WiFi connection
    async fn disconnect(&self) -> Result<()>;

    /// Forget (delete) a saved network profile
    async fn forget_network(&self, ssid: &str) -> Result<()>;

    /// Get current active WiFi connection info (None if disconnected)
    async fn current_connection(&self) -> Result<Option<ConnectionInfo>>;

    /// Connect to a hidden network
    async fn connect_hidden(&self, ssid: &str, password: Option<&str>) -> Result<()>;

    /// Get the interface name being used
    fn interface_name(&self) -> &str;
}
