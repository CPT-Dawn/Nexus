//! Probes of the host the Doctor reasons about. Each one reads a single
//! thing and decides nothing: [`super::assess`] turns the answers into words.
//!
//! Services come from systemd over the system bus (so tests put a fake
//! systemd on their private bus), names and the gateway are asked of the
//! real resolver and network stack.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use zbus::{Connection, OwnedObjectPath};

/// The services the Doctor looks at.
pub const NETWORK_MANAGER: &str = "NetworkManager.service";
pub const NETWORKD: &str = "systemd-networkd.service";
pub const DHCPCD: &str = "dhcpcd.service";
pub const WPA_SUPPLICANT: &str = "wpa_supplicant.service";
pub const IWD: &str = "iwd.service";

const WATCHED: [&str; 5] = [NETWORK_MANAGER, NETWORKD, DHCPCD, WPA_SUPPLICANT, IWD];

/// How long a name lookup or a gateway probe may take.
const PROBE_WAIT: Duration = Duration::from_secs(3);

/// What systemd says about one unit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnitInfo {
    /// A unit file exists: the package is there.
    pub installed: bool,
    /// Running (or starting).
    pub active: bool,
}

/// The watched units. `get` is total: unknown names are absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Units(HashMap<&'static str, UnitInfo>);

impl Units {
    pub fn get(&self, unit: &str) -> UnitInfo {
        self.0.get(unit).copied().unwrap_or_default()
    }

    #[cfg(test)]
    pub fn with(mut self, unit: &'static str, info: UnitInfo) -> Self {
        self.0.insert(unit, info);
        self
    }
}

type UnitRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    OwnedObjectPath,
    u32,
    String,
    OwnedObjectPath,
);

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait Systemd {
    fn get_unit_file_state(&self, name: &str) -> zbus::Result<String>;
    fn list_units_by_patterns(
        &self,
        states: &[&str],
        patterns: &[&str],
    ) -> zbus::Result<Vec<UnitRow>>;
}

/// The watched units as systemd reports them, `None` when systemd cannot be
/// asked (no systemd, or no system bus).
pub async fn units() -> Option<Units> {
    let conn = Connection::system().await.ok()?;
    let systemd = SystemdProxy::new(&conn).await.ok()?;
    let loaded = systemd.list_units_by_patterns(&[], &WATCHED).await.ok()?;
    let mut units = HashMap::new();
    for name in WATCHED {
        let active = loaded
            .iter()
            .any(|row| row.0 == name && matches!(row.3.as_str(), "active" | "activating"));
        // A running unit has a file; for the rest the file is the proof.
        let installed = active || systemd.get_unit_file_state(name).await.is_ok();
        units.insert(name, UnitInfo { installed, active });
    }
    Some(Units(units))
}

/// The Wi-Fi radio switches the kernel reports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Radio {
    pub soft_blocked: bool,
    pub hard_blocked: bool,
}

/// Reads `rfkill` state from sysfs (`/sys/class/rfkill`), `None` when no
/// Wi-Fi radio is listed. Laptops list phantom switches next to the real
/// radio (an `acer-wireless` that stays blocked while `phy0` works), so Wi-Fi
/// counts as blocked only when no listed radio is free to transmit.
pub fn wifi_radio(root: &Path) -> Option<Radio> {
    let mut radios = Vec::new();
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let dir = entry.path();
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        if read("type").trim() == "wlan" {
            radios.push((read("soft").trim() == "1", read("hard").trim() == "1"));
        }
    }
    if radios.is_empty() {
        return None;
    }
    let usable = radios.iter().any(|(soft, hard)| !soft && !hard);
    Some(Radio {
        soft_blocked: !usable && radios.iter().any(|(soft, _)| *soft),
        hard_blocked: !usable && radios.iter().any(|(_, hard)| *hard),
    })
}

/// The default IPv4 gateway from the kernel's route table text
/// (`/proc/net/route`).
pub fn default_gateway(route_table: &str) -> Option<String> {
    const RTF_GATEWAY: u32 = 0x2;
    route_table.lines().skip(1).find_map(|line| {
        let mut cols = line.split_whitespace();
        let (_iface, dest, gateway, flags) =
            (cols.next()?, cols.next()?, cols.next()?, cols.next()?);
        let flags = u32::from_str_radix(flags, 16).ok()?;
        if dest != "00000000" || flags & RTF_GATEWAY == 0 {
            return None;
        }
        // Printed in host order: the bytes in memory are the address.
        let raw = u32::from_str_radix(gateway, 16).ok()?;
        Some(Ipv4Addr::from(raw.to_ne_bytes()).to_string())
    })
}

/// The host part of an `http(s)://` URL.
pub fn url_host(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next()?
    } else {
        authority.split(':').next()?
    };
    (!host.is_empty()).then_some(host)
}

/// Resolves `host` with the system resolver (what every program on the
/// machine would get). The error is a short reason.
pub async fn resolve(host: &str) -> Result<(), String> {
    let name = host.to_owned();
    let lookup = tokio::task::spawn_blocking(move || {
        (name.as_str(), 0).to_socket_addrs().map(|mut a| a.next())
    });
    match tokio::time::timeout(PROBE_WAIT, lookup).await {
        Ok(Ok(Ok(Some(_)))) => Ok(()),
        Ok(Ok(Ok(None))) => Err("the answer was empty".into()),
        Ok(Ok(Err(e))) => Err(e.to_string()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("no answer in time".into()),
    }
}

/// Whether the gateway answers at all. It opens a TCP connection to its DNS
/// port: a connection or an outright refusal both prove a machine is there,
/// while silence or "no route" prove it is not. No raw sockets, no
/// privileges, and nothing leaves the local network.
pub async fn gateway_answers(address: &str) -> Result<(), String> {
    let ip: IpAddr = address
        .parse()
        .map_err(|_| format!("`{address}` is not an IP address"))?;
    match tokio::time::timeout(PROBE_WAIT, tokio::net::TcpStream::connect((ip, 53))).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => Ok(()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("no answer in time".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_route_table_yields_the_default_gateway() {
        let table = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
                     wlan0\t00000000\t0100A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
                     wlan0\t0000A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\t0\t0\t0\n";
        let expected = Ipv4Addr::from(0x0100A8C0u32.to_ne_bytes()).to_string();
        assert_eq!(default_gateway(table), Some(expected));
        assert_eq!(default_gateway("Iface\tDestination\n"), None);
    }

    #[test]
    fn hosts_come_out_of_urls() {
        assert_eq!(url_host("http://a.example/x?y"), Some("a.example"));
        assert_eq!(url_host("https://u:p@a.example:8080/"), Some("a.example"));
        assert_eq!(url_host("http://[::1]:80/"), Some("::1"));
        assert_eq!(url_host("not a url"), None);
    }

    #[test]
    fn rfkill_in_sysfs_is_read_for_wifi_only() {
        let dir = tempfile::tempdir().unwrap();
        let radio = |name: &str, kind: &str, soft: &str, hard: &str| {
            let d = dir.path().join(name);
            std::fs::create_dir(&d).unwrap();
            for (f, v) in [("type", kind), ("soft", soft), ("hard", hard)] {
                std::fs::write(d.join(f), format!("{v}\n")).unwrap();
            }
        };
        radio("rfkill0", "bluetooth", "1", "1");
        assert_eq!(wifi_radio(dir.path()), None, "bluetooth is not Wi-Fi");
        radio("rfkill1", "wlan", "1", "0");
        assert_eq!(
            wifi_radio(dir.path()),
            Some(Radio {
                soft_blocked: true,
                hard_blocked: false
            })
        );
        // A phantom platform switch stays blocked while the real radio works.
        radio("rfkill2", "wlan", "0", "0");
        assert_eq!(wifi_radio(dir.path()), Some(Radio::default()));
    }
}
