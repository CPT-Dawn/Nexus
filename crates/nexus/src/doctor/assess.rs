//! From facts to findings: severity, cause and fix, in plain English. Pure,
//! so the wording is easy to read in one place.

use nexus_ipc::{Connectivity, Finding, Fix, Severity, State, Status};

use super::host::{DHCPCD, IWD, NETWORK_MANAGER, NETWORKD, Radio, Units, WPA_SUPPLICANT};
use crate::backend::NetFacts;

/// What the checks found out. `None` means "not asked" or "could not ask".
pub struct Inputs {
    pub status: Status,
    /// Present exactly when NetworkManager answers on the bus.
    pub nm: Option<NetFacts>,
    pub units: Option<Units>,
    /// The kernel's Wi-Fi switches, read only when NetworkManager cannot say.
    pub radio: Option<Radio>,
    /// Whether the connectivity check is switched off (by config or by NetworkManager).
    pub check_off: bool,
    /// The host the check would contact, and what resolving it did.
    pub dns: Option<(String, Result<(), String>)>,
    /// The gateway's address, and whether it answered.
    pub gateway: Option<(String, Result<(), String>)>,
    /// The `ID_LIKE`/`ID` words of `/etc/os-release`, for the install command.
    pub distro: Vec<String>,
}

fn finding(
    id: &str,
    severity: Severity,
    title: impl Into<String>,
    cause: impl Into<String>,
) -> Finding {
    Finding {
        id: id.into(),
        severity,
        title: title.into(),
        cause: cause.into(),
        fix: None,
    }
}

fn with_fix(
    mut f: Finding,
    text: impl Into<String>,
    commands: &[&str],
    privileged: bool,
) -> Finding {
    f.fix = Some(Fix {
        text: text.into(),
        runnable: !commands.is_empty(),
        commands: commands.iter().map(|c| (*c).to_owned()).collect(),
        privileged,
    });
    f
}

/// A fix to read, not to run.
fn advice(mut f: Finding, text: impl Into<String>) -> Finding {
    f.fix = Some(Fix {
        text: text.into(),
        commands: Vec::new(),
        runnable: false,
        privileged: false,
    });
    f
}

const IWD_CONF: &str = "sudo sh -c 'mkdir -p /etc/NetworkManager/conf.d && printf \"[device]\\nwifi.backend=iwd\\n\" > /etc/NetworkManager/conf.d/wifi_backend.conf'";

/// `(unit, name people know it by, finding id suffix)`.
const RIVALS: [(&str, &str, &str); 3] = [
    (NETWORKD, "systemd-networkd", "networkd"),
    (DHCPCD, "dhcpcd", "dhcpcd"),
    (WPA_SUPPLICANT, "wpa_supplicant", "wpa_supplicant"),
];

fn install_command(distro: &[String]) -> Option<&'static str> {
    let is = |names: &[&str]| distro.iter().any(|d| names.contains(&d.as_str()));
    if is(&["arch", "manjaro", "endeavouros", "cachyos"]) {
        Some("sudo pacman -S networkmanager")
    } else if is(&["debian", "ubuntu"]) {
        Some("sudo apt install network-manager")
    } else if is(&["fedora", "rhel"]) {
        Some("sudo dnf install NetworkManager")
    } else if is(&["suse", "opensuse"]) {
        Some("sudo zypper install NetworkManager")
    } else {
        None
    }
}

/// The ordered steps that hand networking to NetworkManager: stop what
/// manages it now, keep iwd as the Wi-Fi backend, start NetworkManager.
fn setup_plan(units: &Units) -> Vec<String> {
    let mut plan = Vec::new();
    for (unit, _, _) in RIVALS {
        // NetworkManager launches its own wpa_supplicant; a standalone one
        // would hold the radio.
        if units.get(unit).active {
            plan.push(format!("sudo systemctl disable --now {unit}"));
        }
    }
    if units.get(IWD).active {
        plan.push(IWD_CONF.to_owned());
    }
    plan.push(format!("sudo systemctl enable --now {NETWORK_MANAGER}"));
    plan
}

pub fn assess(i: &Inputs) -> Vec<Finding> {
    let mut out = Vec::new();
    let running = i.nm.is_some();
    network_manager(i, &mut out);
    conflicts(i, running, &mut out);
    if running {
        link(i, &mut out);
    }
    radio(i, &mut out);
    if i.check_off {
        let mut cause = String::from(
            "Without it Nexus cannot tell a sign-in page or an outage from a working connection, \
             and the checks that rely on it are skipped.",
        );
        if let Some(uri) = i.nm.as_ref().and_then(|n| n.connectivity_uri.as_deref()) {
            // Read-only here: the address lives in NetworkManager.conf.
            cause.push_str(&format!(
                " NetworkManager would check {uri} (set in NetworkManager.conf, not by Nexus)."
            ));
        }
        out.push(advice(
            finding(
                "connectivity-check",
                Severity::Info,
                "The connectivity check is switched off",
                cause,
            ),
            "Turn it on with `nexus config set connectivity.enabled true`.",
        ));
    }
    reachability(i, &mut out);
    if let (true, Some(nm), Some(_)) = (running, &i.nm, &i.status.link) {
        out.push(if nm.ipv6 {
            finding(
                "ipv6",
                Severity::Ok,
                "IPv6 is available",
                "This network handed out an IPv6 address.",
            )
        } else {
            finding(
                "ipv6",
                Severity::Info,
                "No IPv6 address",
                "This network did not hand out an IPv6 address. Everything still works over IPv4; \
                 many networks are IPv4 only.",
            )
        });
    }
    out
}

/// The oldest NetworkManager Nexus is built and tested against.
const MIN_NM_MAJOR: u32 = 1;
const MIN_NM_MINOR: u32 = 46;

/// Whether a NetworkManager version string (`1.44.2`, `1.47.90-dev`) is older
/// than the supported one. A string that does not parse is not called old.
fn is_too_old(version: &str) -> bool {
    let mut parts = version.split(['.', '-']).map(|p| p.parse::<u32>().ok());
    let (Some(Some(major)), Some(Some(minor))) = (parts.next(), parts.next()) else {
        return false;
    };
    (major, minor) < (MIN_NM_MAJOR, MIN_NM_MINOR)
}

fn network_manager(i: &Inputs, out: &mut Vec<Finding>) {
    if let Some(nm) = &i.nm {
        let version = nm
            .version
            .clone()
            .or_else(|| i.status.backend.version.clone());
        let title = match version {
            Some(v) => format!("NetworkManager {v} is running"),
            None => "NetworkManager is running".into(),
        };
        out.push(finding(
            "networkmanager",
            Severity::Ok,
            title,
            "NetworkManager manages your connections, which is what Nexus drives.",
        ));
        if let Some(v) = nm.version.as_deref().filter(|v| is_too_old(v)) {
            out.push(advice(
                finding(
                    "networkmanager-version",
                    Severity::Problem,
                    format!("NetworkManager {v} is older than Nexus supports"),
                    format!(
                        "Nexus needs NetworkManager {MIN_NM_MAJOR}.{MIN_NM_MINOR} or newer for \
                         Hotspots, WireGuard and safe Profile changes; some of them will not work."
                    ),
                ),
                "Update NetworkManager with your distribution's package manager.",
            ));
        }
        return;
    }
    let cause_base =
        "Nexus manages networks through NetworkManager and it is not answering on the system bus";
    let Some(units) = &i.units else {
        let f = finding(
            "networkmanager",
            Severity::Problem,
            "NetworkManager is not running",
            format!("{cause_base}. Nexus could not ask systemd whether it is installed."),
        );
        out.push(with_fix(
            f,
            "Start it now and at every boot.",
            &["sudo systemctl enable --now NetworkManager.service"],
            true,
        ));
        out.push(advice(
            finding(
                "services",
                Severity::Info,
                "Could not ask systemd about services",
                "systemd did not answer, so conflicting network services could not be checked.",
            ),
            "On a system without systemd, check by hand that only one program manages your network.",
        ));
        return;
    };
    let plan = setup_plan(units);
    if units.get(NETWORK_MANAGER).installed {
        let f = finding(
            "networkmanager",
            Severity::Problem,
            "NetworkManager is not running",
            format!("{cause_base}, although it is installed."),
        );
        let text = if plan.len() > 1 {
            "Switch off what manages your network now, then start NetworkManager. The steps are in order."
        } else {
            "Start it now and at every boot."
        };
        let plan: Vec<&str> = plan.iter().map(String::as_str).collect();
        out.push(with_fix(f, text, &plan, true));
    } else {
        let f = finding(
            "networkmanager",
            Severity::Problem,
            "NetworkManager is not installed",
            format!("{cause_base}, because the package is not installed."),
        );
        let mut steps: Vec<String> = install_command(&i.distro)
            .map(str::to_owned)
            .into_iter()
            .collect();
        steps.extend(plan);
        let mut f = advice(
            f,
            "Install the NetworkManager package with your package manager (package `networkmanager`, \
             or `network-manager` on Debian and Ubuntu), then run `nexus doctor` again for the \
             remaining steps.",
        );
        if let Some(fix) = &mut f.fix {
            fix.commands = steps;
        }
        out.push(f);
    }
}

fn conflicts(i: &Inputs, running: bool, out: &mut Vec<Finding>) {
    let Some(units) = &i.units else { return };
    for (unit, name, id) in RIVALS {
        if !units.get(unit).active || (running && unit == WPA_SUPPLICANT) {
            continue;
        }
        let f = finding(
            &format!("conflict-{id}"),
            Severity::Warning,
            format!("{name} is also managing your network"),
            format!(
                "{name} is running and configures network interfaces by itself, so it fights \
                 NetworkManager over them: connections drop, addresses flap, or one of them wins at random."
            ),
        );
        out.push(if running {
            with_fix(
                f,
                format!("Switch {name} off, now and at boot."),
                &[&format!("sudo systemctl disable --now {unit}")],
                true,
            )
        } else {
            advice(
                f,
                format!("The NetworkManager steps above switch {name} off before starting NetworkManager."),
            )
        });
    }
    // NetworkManager starts wpa_supplicant for Wi-Fi unless iwd is its backend,
    // so iwd running beside it with no wpa_supplicant means iwd is the backend.
    let iwd_is_backend = running && !units.get(WPA_SUPPLICANT).active;
    if units.get(IWD).active && !iwd_is_backend {
        let f = finding(
            "conflict-iwd",
            Severity::Info,
            "iwd is running on its own",
            "iwd drives the Wi-Fi radio. NetworkManager can use it as its Wi-Fi backend, but \
             when NetworkManager uses wpa_supplicant instead, the two fight over the radio.",
        );
        out.push(advice(
            f,
            if running {
                "Make iwd NetworkManager's Wi-Fi backend: put `[device]` and `wifi.backend=iwd` in \
                 /etc/NetworkManager/conf.d/wifi_backend.conf, then restart NetworkManager."
            } else {
                "The NetworkManager steps above keep iwd and make it NetworkManager's Wi-Fi backend."
            },
        ));
    }
}

fn link(i: &Inputs, out: &mut Vec<Finding>) {
    match i.status.state {
        State::Offline => out.push(advice(
            finding(
                "link",
                Severity::Problem,
                "Not connected to any network",
                "NetworkManager is running, but no network is connected.",
            ),
            "Join a Wi-Fi network with `nexus wifi join <name>`, or plug in a cable.",
        )),
        State::Asleep => out.push(with_fix(
            finding(
                "link",
                Severity::Problem,
                "Networking is switched off",
                "NetworkManager has been told to turn all networking off (airplane mode or `nmcli networking off`).",
            ),
            "Turn networking back on.",
            &["nmcli networking on"],
            false,
        )),
        _ => {}
    }
}

fn radio(i: &Inputs, out: &mut Vec<Finding>) {
    let (radio, via_nm) = match (&i.nm, i.radio) {
        (Some(nm), _) => (
            Some(Radio {
                soft_blocked: !nm.wireless_enabled,
                hard_blocked: !nm.wireless_hw_enabled,
            }),
            true,
        ),
        (None, radio) => (radio, false),
    };
    let Some(radio) = radio else { return };
    let f = if radio.hard_blocked {
        advice(
            finding(
                "rfkill",
                Severity::Problem,
                "Wi-Fi is switched off by a hardware switch",
                "A physical switch or a function key has turned the Wi-Fi radio off, and software cannot undo that.",
            ),
            "Flip the Wi-Fi switch on your laptop or press its function key (often marked with an antenna).",
        )
    } else if radio.soft_blocked {
        let f = finding(
            "rfkill",
            Severity::Problem,
            "Wi-Fi is switched off in software",
            "The Wi-Fi radio is blocked (airplane mode or `rfkill block`), so no Wi-Fi network can be found.",
        );
        let command = if via_nm {
            "nmcli radio wifi on"
        } else {
            "rfkill unblock wifi"
        };
        with_fix(f, "Switch the radio back on.", &[command], false)
    } else {
        finding(
            "rfkill",
            Severity::Ok,
            "The Wi-Fi radio is not blocked",
            "Neither a software nor a hardware switch is off.",
        )
    };
    out.push(f);
}

/// Gateway, DNS, the sign-in page and "connected but no internet".
fn reachability(i: &Inputs, out: &mut Vec<Finding>) {
    let mut broken = false;
    if let Some((address, result)) = &i.gateway {
        out.push(match result {
            Ok(()) => finding(
                "gateway",
                Severity::Ok,
                format!("Your router ({address}) answers"),
                "The first hop out of your network is alive.",
            ),
            Err(why) => {
                broken = true;
                advice(
                    finding(
                        "gateway",
                        Severity::Problem,
                        format!("Your router ({address}) does not answer"),
                        format!(
                            "This machine is connected, but the router it should send traffic through \
                             is silent ({why}). The link may be weak, or the router stuck."
                        ),
                    ),
                    "Move closer to the access point or check the cable, then restart your router.",
                )
            }
        });
    }
    if let Some((host, result)) = &i.dns {
        out.push(match result {
            Ok(()) => finding(
                "dns",
                Severity::Ok,
                "Names resolve",
                format!("The system resolver found {host}."),
            ),
            Err(why) => {
                let severity = if i.status.connectivity == Connectivity::Full {
                    Severity::Warning
                } else {
                    broken = true;
                    Severity::Problem
                };
                advice(
                    finding(
                        "dns",
                        severity,
                        format!("Cannot look up {host}"),
                        format!(
                            "The system resolver could not find {host} ({why}), so programs cannot \
                             reach anything by name even if the link is up."
                        ),
                    ),
                    "Check your DNS servers with `resolvectl status` or /etc/resolv.conf, or try a \
                     public one such as 9.9.9.9.",
                )
            }
        });
    }
    if i.status.connectivity == Connectivity::Portal {
        out.push(advice(
            finding(
                "portal",
                Severity::Warning,
                "A sign-in page is blocking the internet",
                "This network (a hotel, cafe or campus) holds all traffic until you sign in on its web page.",
            ),
            "Press o in Nexus to open the sign-in page, or visit any plain http:// page in a browser. If nothing appears, turn off the browser's secure DNS (DNS over HTTPS) until you have signed in: it hides the sign-in page.",
        ));
    }
    let linked_without_internet = matches!(i.status.state, State::Local | State::Site)
        || (i.status.state == State::Online && i.status.connectivity == Connectivity::Limited);
    if linked_without_internet && !broken {
        out.push(advice(
            finding(
                "internet",
                Severity::Problem,
                "Connected, but the internet is unreachable",
                "Your router and DNS answer, so the break is past your router: your provider is down, \
                 or a firewall blocks the connectivity check.",
            ),
            "Check whether other devices on this network are online; if not, restart your router or call your provider.",
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::host::UnitInfo;

    fn inputs() -> Inputs {
        Inputs {
            status: Status::unavailable(),
            nm: None,
            units: None,
            radio: None,
            check_off: true,
            dns: None,
            gateway: None,
            distro: Vec::new(),
        }
    }

    #[test]
    fn old_networkmanagers_are_told_apart_from_supported_ones() {
        assert!(is_too_old("1.44.2"));
        assert!(is_too_old("0.9.10"));
        assert!(!is_too_old("1.46.0"));
        assert!(!is_too_old("1.47.90-dev"));
        assert!(!is_too_old("2.0.0"));
        assert!(!is_too_old("garbage"));
    }

    #[test]
    fn the_install_command_follows_the_distribution() {
        let d = |s: &str| vec![s.to_owned()];
        assert_eq!(
            install_command(&d("arch")),
            Some("sudo pacman -S networkmanager")
        );
        assert_eq!(
            install_command(&d("ubuntu")),
            Some("sudo apt install network-manager")
        );
        assert_eq!(install_command(&d("nixos")), None);
    }

    #[test]
    fn without_systemd_the_plan_is_just_starting_networkmanager() {
        let found = assess(&inputs());
        let nm = found.iter().find(|f| f.id == "networkmanager").unwrap();
        assert_eq!(
            nm.fix.as_ref().unwrap().commands,
            ["sudo systemctl enable --now NetworkManager.service"]
        );
        assert!(found.iter().any(|f| f.id == "services"));
    }

    #[test]
    fn the_plan_stops_rivals_before_starting_networkmanager() {
        let on = UnitInfo {
            installed: true,
            active: true,
        };
        let units = Units::default().with(NETWORKD, on).with(DHCPCD, on);
        let plan = setup_plan(&units);
        assert_eq!(plan.len(), 3);
        assert!(plan[0].contains("systemd-networkd") && plan[1].contains("dhcpcd"));
        assert!(plan[2].contains("enable --now NetworkManager"));
    }
}
