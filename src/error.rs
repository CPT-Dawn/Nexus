use thiserror::Error;

/// Unified error type for Nexus-NM
#[derive(Error, Debug)]
pub enum NexusError {
    #[error("D-Bus error: {0}")]
    Dbus(#[from] zbus::Error),

    #[error("D-Bus fdo error: {0}")]
    DbusFdo(#[from] zbus::fdo::Error),

    #[error("NetworkManager error: {0}")]
    NetworkManager(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("WiFi error: {0}")]
    Wifi(String),

    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Terminal error: {0}")]
    Terminal(String),

    #[error("DNS error: {0}")]
    Dns(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Not supported: {0}")]
    NotSupported(String),
}

pub type NexusResult<T> = Result<T, NexusError>;
