//! What a toast's `u` does. Every list that can take a change back
//! (Wi-Fi, Ethernet, VPN, Settings, Hotspot) records the command that does it,
//! so there is one `u` and one place that replays it.

use serde_json::Value;

use super::{ethernet, vpn, wifi};

/// The command that takes a change back.
#[derive(Debug, Clone)]
pub enum Undo {
    Wifi(wifi::Cmd),
    Ethernet(ethernet::Cmd),
    Vpn(vpn::Cmd),
    /// Put a config key back to what it was (`Null` removes it).
    Setting {
        key: String,
        value: Value,
    },
}

impl From<wifi::Cmd> for Undo {
    fn from(cmd: wifi::Cmd) -> Self {
        Undo::Wifi(cmd)
    }
}

impl From<ethernet::Cmd> for Undo {
    fn from(cmd: ethernet::Cmd) -> Self {
        Undo::Ethernet(cmd)
    }
}

impl From<vpn::Cmd> for Undo {
    fn from(cmd: vpn::Cmd) -> Self {
        Undo::Vpn(cmd)
    }
}
