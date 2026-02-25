use tracing::{info, warn};

/// Permission level detected for the current session
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionLevel {
    /// Full access (root or PolicyKit authorized)
    Full,
    /// Read-only mode (cannot modify connections)
    ReadOnly,
    /// Unknown (haven't checked yet)
    Unknown,
}

impl PermissionLevel {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Full => "Full Access",
            Self::ReadOnly => "Read-Only",
            Self::Unknown => "Checking...",
        }
    }

    pub fn can_write(&self) -> bool {
        matches!(self, Self::Full)
    }
}

/// Check current permission level by testing a D-Bus operation
pub async fn check_permissions(nm: &crate::network::NetworkManager) -> PermissionLevel {
    // Check if we're root
    if unsafe { libc::geteuid() } == 0 {
        info!("Running as root — full access");
        return PermissionLevel::Full;
    }

    // Try to read wireless_enabled (a property that requires minimal permissions)
    // Then try a write-like check — wireless_enabled is settable
    match nm.wireless_enabled().await {
        Ok(_) => {
            // Reading works. For write check, we'd need to actually try a privileged op.
            // NM uses PolicyKit — if a polkit agent is running, write calls will trigger auth.
            // We optimistically assume Full if we can read, and downgrade on first PermissionDenied.
            info!(
                "NetworkManager accessible — assuming full access (polkit will prompt if needed)"
            );
            PermissionLevel::Full
        }
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("org.freedesktop.DBus.Error.AccessDenied")
                || err_str.contains("PermissionDenied")
            {
                warn!("Permission denied — running in read-only mode");
                warn!("Tip: Run with sudo or ensure a PolicyKit agent is running");
                PermissionLevel::ReadOnly
            } else {
                warn!("Error checking permissions: {}", e);
                PermissionLevel::ReadOnly
            }
        }
    }
}
