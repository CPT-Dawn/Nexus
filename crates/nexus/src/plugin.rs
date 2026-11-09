//! Plugin VPNs: which NetworkManager plugin reads which provider file, which
//! package ships it, and whether it is installed. Pure apart from the
//! directory listing in [`installed`].

use std::path::{Path, PathBuf};

/// A NetworkManager VPN plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plugin {
    /// The name `nmcli connection import type <name>` takes; also the middle
    /// of the plugin's `nm-<name>-service.name` file.
    pub name: &'static str,
    /// For sentences.
    pub label: &'static str,
    /// The Arch package that ships it.
    pub package: &'static str,
    /// File extensions of the provider files it reads, without the dot.
    pub extensions: &'static [&'static str],
}

const PLUGINS: &[Plugin] = &[
    Plugin {
        name: "openvpn",
        label: "OpenVPN",
        package: "networkmanager-openvpn",
        extensions: &["ovpn"],
    },
    Plugin {
        name: "vpnc",
        label: "Cisco (vpnc)",
        package: "networkmanager-vpnc",
        extensions: &["pcf"],
    },
    Plugin {
        name: "openconnect",
        label: "OpenConnect",
        package: "networkmanager-openconnect",
        extensions: &[],
    },
    Plugin {
        name: "pptp",
        label: "PPTP",
        package: "networkmanager-pptp",
        extensions: &[],
    },
    Plugin {
        name: "l2tp",
        label: "L2TP",
        package: "networkmanager-l2tp",
        extensions: &[],
    },
    Plugin {
        name: "strongswan",
        label: "strongSwan",
        package: "networkmanager-strongswan",
        extensions: &[],
    },
    Plugin {
        name: "fortisslvpn",
        label: "Fortinet SSL VPN",
        package: "networkmanager-fortisslvpn",
        extensions: &[],
    },
];

const SERVICE_PREFIX: &str = "org.freedesktop.NetworkManager.";

pub fn by_name(name: &str) -> Option<&'static Plugin> {
    PLUGINS.iter().find(|p| p.name == name)
}

/// The plugin that reads a provider file, by its extension (any case).
pub fn for_path(path: &str) -> Option<&'static Plugin> {
    let ext = Path::new(path).extension()?.to_str()?.to_lowercase();
    PLUGINS
        .iter()
        .find(|p| p.extensions.contains(&ext.as_str()))
}

/// The plugin name in a Profile's `vpn.service-type`
/// (`org.freedesktop.NetworkManager.openvpn` -> `openvpn`).
pub fn name_of_service(service_type: &str) -> Option<&str> {
    service_type
        .strip_prefix(SERVICE_PREFIX)
        .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// Where NetworkManager looks for plugin descriptions: `NEXUS_VPN_PLUGIN_DIRS`
/// (colon separated, for tests) or the usual directories.
fn dirs() -> Vec<PathBuf> {
    match std::env::var("NEXUS_VPN_PLUGIN_DIRS") {
        Ok(v) => v
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .collect(),
        Err(_) => [
            "/usr/lib/NetworkManager/VPN",
            "/usr/lib64/NetworkManager/VPN",
            "/etc/NetworkManager/VPN",
        ]
        .map(PathBuf::from)
        .to_vec(),
    }
}

/// Whether the plugin called `name` is installed.
pub fn installed(name: &str) -> bool {
    let file = format!("nm-{name}-service.name");
    dirs().iter().any(|d| d.join(&file).is_file())
}

/// What to tell someone whose plugin is missing.
pub fn install_hint(name: &str) -> String {
    match by_name(name) {
        Some(p) => format!(
            "{} needs the {} package. Install it, restart NetworkManager, and try again.",
            p.label, p.package
        ),
        None => format!(
            "The NetworkManager `{name}` VPN plugin is not installed. Install its \
             package (usually named networkmanager-{name}), restart NetworkManager, \
             and try again."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_pick_the_plugin() {
        assert_eq!(for_path("/home/me/work.OVPN").unwrap().name, "openvpn");
        assert_eq!(for_path("cisco.pcf").unwrap().name, "vpnc");
        assert!(for_path("wg0.conf").is_none());
        assert!(for_path("noextension").is_none());
    }

    #[test]
    fn service_types_name_their_plugin() {
        assert_eq!(
            name_of_service("org.freedesktop.NetworkManager.openvpn"),
            Some("openvpn")
        );
        assert_eq!(name_of_service("org.freedesktop.NetworkManager."), None);
        assert_eq!(name_of_service("something.else"), None);
        assert_eq!(
            name_of_service("org.freedesktop.NetworkManager.a/../b"),
            None
        );
    }

    #[test]
    fn the_hint_names_the_package() {
        assert!(install_hint("openvpn").contains("networkmanager-openvpn"));
        assert!(install_hint("mystery").contains("networkmanager-mystery"));
    }
}
