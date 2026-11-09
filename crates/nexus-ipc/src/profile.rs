//! Wire types of Profile settings (IP, DNS, MAC, metered, priority) and the
//! validation both the Daemon and Front ends run, so a typo is caught while
//! typing and again before anything reaches the Backend.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use serde::{Deserialize, Serialize};

/// Lowest and highest autoconnect priority NetworkManager accepts.
pub const PRIORITY_MIN: i32 = -999;
pub const PRIORITY_MAX: i32 = 999;

/// How a Profile gets its IPv4 address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Ipv4Method {
    /// DHCP.
    #[default]
    Auto,
    /// The addresses in the Profile.
    Manual,
    #[serde(other)]
    Unknown,
}

/// How a Profile gets its IPv6 address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Ipv6Method {
    /// Router advertisements and DHCPv6.
    #[default]
    Auto,
    Manual,
    /// No IPv6 on this Link at all.
    Disabled,
    #[serde(other)]
    Unknown,
}

/// Which MAC address the Device shows to the Network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MacMode {
    /// A random one, the same each time this Network is joined.
    #[default]
    PerNetwork,
    /// A new random one at every join.
    EveryTime,
    /// The hardware address.
    Permanent,
    #[serde(other)]
    Unknown,
}

/// One address with its prefix length.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressEntry {
    pub address: String,
    pub prefix: u8,
}

/// IPv4 or IPv6 addressing of a Profile.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Ipv4 {
    pub method: Ipv4Method,
    #[serde(default)]
    pub addresses: Vec<AddressEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Ipv6 {
    pub method: Ipv6Method,
    #[serde(default)]
    pub addresses: Vec<AddressEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
}

/// Everything about a Profile that `profile_update` can change.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProfileSettings {
    pub ipv4: Ipv4,
    pub ipv6: Ipv6,
    /// DNS servers, IPv4 and IPv6 alike; empty means "from the Network".
    #[serde(default)]
    pub dns: Vec<String>,
    pub mac: MacMode,
    pub metered: bool,
    /// Higher joins first when several saved Networks are in range.
    pub priority: i32,
    /// The Profile's secret lives in the Secret Service keyring and the
    /// Daemon serves it to NetworkManager, instead of NetworkManager's own
    /// system store (the default).
    #[serde(default)]
    pub keyring: bool,
}

/// The `profile_get` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileView {
    pub ssid: String,
    pub uuid: String,
    /// Pass it back in `profile_update`: the update fails with `conflict`
    /// when the Profile changed since (say in another program).
    pub version: u64,
    /// This Profile carries the current Link, so changes are checkpointed.
    pub active: bool,
    pub settings: ProfileSettings,
}

/// A change waiting for "Keep these settings?".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingChange {
    pub id: u64,
    pub ssid: String,
    /// Seconds until the change rolls back on its own.
    pub rollback_secs: u32,
}

/// The `profile_update` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileApplied {
    pub ssid: String,
    /// The Profile's version after the change.
    pub version: u64,
    /// Present when the change waits for `profile_confirm`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingChange>,
}

/// How a pending change ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Settled {
    Confirmed,
    /// Reverted: nobody confirmed in time, or somebody said so.
    RolledBack,
    #[serde(other)]
    Unknown,
}

/// Which input a [`FieldError`] belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Ipv4Address,
    Ipv4Gateway,
    Ipv6Address,
    Ipv6Gateway,
    Dns,
    Priority,
}

/// One thing wrong with the settings, in words for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub field: Field,
    pub message: String,
}

fn err(field: Field, message: impl Into<String>) -> FieldError {
    FieldError {
        field,
        message: message.into(),
    }
}

/// Parses `192.168.1.20/24, 10.0.0.5/8` into entries. A missing prefix is an
/// error rather than a guess: a wrong guess locks people out.
pub fn parse_addresses(text: &str, v6: bool) -> Result<Vec<AddressEntry>, String> {
    let max = if v6 { 128 } else { 32 };
    text.split([',', ' '])
        .filter(|s| !s.is_empty())
        .map(|item| {
            let (addr, prefix) = item
                .split_once('/')
                .ok_or_else(|| format!("{item}: add a prefix length, like {item}/24"))?;
            let ip: IpAddr = addr
                .parse()
                .map_err(|_| format!("{addr} is not an IP address"))?;
            if ip.is_ipv6() != v6 {
                let want = if v6 { "IPv6" } else { "IPv4" };
                return Err(format!("{addr} is not an {want} address"));
            }
            let prefix: u8 = prefix
                .parse()
                .ok()
                .filter(|p| (1..=max).contains(p))
                .ok_or_else(|| format!("The prefix of {item} must be 1 to {max}"))?;
            Ok(AddressEntry {
                address: ip.to_string(),
                prefix,
            })
        })
        .collect()
}

/// Parses a comma or space separated list of DNS servers.
pub fn parse_dns(text: &str) -> Result<Vec<String>, String> {
    text.split([',', ' '])
        .filter(|s| !s.is_empty())
        .map(|item| {
            let ip: IpAddr = item
                .parse()
                .map_err(|_| format!("{item} is not an IP address"))?;
            if ip.is_unspecified() || ip.is_multicast() {
                return Err(format!("{item} cannot be a DNS server"));
            }
            Ok(ip.to_string())
        })
        .collect()
}

/// Text of entries as `a/p, a/p`, the inverse of [`parse_addresses`].
pub fn addresses_text(entries: &[AddressEntry]) -> String {
    entries
        .iter()
        .map(|e| format!("{}/{}", e.address, e.prefix))
        .collect::<Vec<_>>()
        .join(", ")
}

fn same_subnet(a: Ipv4Addr, prefix: u8, b: Ipv4Addr) -> bool {
    let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
    u32::from(a) & mask == u32::from(b) & mask
}

fn check_v4(ipv4: &Ipv4, out: &mut Vec<FieldError>) {
    if ipv4.method != Ipv4Method::Manual {
        return;
    }
    if ipv4.addresses.is_empty() {
        out.push(err(
            Field::Ipv4Address,
            "A static IPv4 setup needs an address, like 192.168.1.20/24",
        ));
        return;
    }
    let mut parsed = Vec::new();
    for e in &ipv4.addresses {
        match e.address.parse::<Ipv4Addr>() {
            Err(_) => out.push(err(
                Field::Ipv4Address,
                format!("{} is not an IPv4 address", e.address),
            )),
            Ok(ip) if ip.is_unspecified() || ip.is_multicast() || ip.is_loopback() => {
                out.push(err(
                    Field::Ipv4Address,
                    format!("{ip} cannot be used as an address"),
                ))
            }
            Ok(ip) if e.prefix == 0 || e.prefix > 32 => out.push(err(
                Field::Ipv4Address,
                format!("The prefix of {ip} must be 1 to 32"),
            )),
            Ok(ip) => parsed.push((ip, e.prefix)),
        }
    }
    if let Some(gw) = &ipv4.gateway {
        match gw.parse::<Ipv4Addr>() {
            Err(_) => out.push(err(
                Field::Ipv4Gateway,
                format!("{gw} is not an IPv4 address"),
            )),
            Ok(g) if g.is_unspecified() || g.is_multicast() => {
                out.push(err(Field::Ipv4Gateway, format!("{g} cannot be a gateway")));
            }
            Ok(g) => {
                if !parsed.is_empty() && !parsed.iter().any(|(ip, p)| same_subnet(*ip, *p, g)) {
                    out.push(err(
                        Field::Ipv4Gateway,
                        format!("The gateway {g} is not on the same network as the address"),
                    ));
                }
            }
        }
    }
}

fn check_v6(ipv6: &Ipv6, out: &mut Vec<FieldError>) {
    if ipv6.method != Ipv6Method::Manual {
        return;
    }
    if ipv6.addresses.is_empty() {
        out.push(err(
            Field::Ipv6Address,
            "A static IPv6 setup needs an address, like fd00::20/64",
        ));
        return;
    }
    for e in &ipv6.addresses {
        match e.address.parse::<Ipv6Addr>() {
            Err(_) => out.push(err(
                Field::Ipv6Address,
                format!("{} is not an IPv6 address", e.address),
            )),
            Ok(ip) if ip.is_unspecified() || ip.is_multicast() || ip.is_loopback() => {
                out.push(err(
                    Field::Ipv6Address,
                    format!("{ip} cannot be used as an address"),
                ))
            }
            Ok(_) if e.prefix == 0 || e.prefix > 128 => out.push(err(
                Field::Ipv6Address,
                format!("The prefix of {} must be 1 to 128", e.address),
            )),
            Ok(_) => {}
        }
    }
    if let Some(gw) = &ipv6.gateway {
        match gw.parse::<Ipv6Addr>() {
            Err(_) => out.push(err(
                Field::Ipv6Gateway,
                format!("{gw} is not an IPv6 address"),
            )),
            Ok(g) if g.is_unspecified() || g.is_multicast() => {
                out.push(err(Field::Ipv6Gateway, format!("{g} cannot be a gateway")));
            }
            Ok(_) => {}
        }
    }
}

/// Everything wrong with `settings`, in field order. Empty means fine.
pub fn validate(settings: &ProfileSettings) -> Vec<FieldError> {
    let mut out = Vec::new();
    check_v4(&settings.ipv4, &mut out);
    check_v6(&settings.ipv6, &mut out);
    for server in &settings.dns {
        match server.parse::<IpAddr>() {
            Err(_) => out.push(err(Field::Dns, format!("{server} is not an IP address"))),
            Ok(ip) if ip.is_unspecified() || ip.is_multicast() => {
                out.push(err(Field::Dns, format!("{server} cannot be a DNS server")));
            }
            Ok(_) => {}
        }
    }
    if !(PRIORITY_MIN..=PRIORITY_MAX).contains(&settings.priority) {
        out.push(err(
            Field::Priority,
            format!("Priority must be {PRIORITY_MIN} to {PRIORITY_MAX}"),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manual4(addr: &str, prefix: u8, gw: Option<&str>) -> ProfileSettings {
        ProfileSettings {
            ipv4: Ipv4 {
                method: Ipv4Method::Manual,
                addresses: vec![AddressEntry {
                    address: addr.into(),
                    prefix,
                }],
                gateway: gw.map(Into::into),
            },
            ..Default::default()
        }
    }

    #[test]
    fn defaults_are_valid() {
        assert!(validate(&ProfileSettings::default()).is_empty());
    }

    #[test]
    fn static_ipv4_is_checked() {
        assert!(validate(&manual4("192.168.1.20", 24, Some("192.168.1.1"))).is_empty());
        let e = validate(&manual4("192.168.1.300", 24, None));
        assert_eq!(e[0].field, Field::Ipv4Address);
        let e = validate(&manual4("192.168.1.20", 33, None));
        assert_eq!(e[0].field, Field::Ipv4Address);
        let e = validate(&manual4("192.168.1.20", 24, Some("10.0.0.1")));
        assert_eq!(e[0].field, Field::Ipv4Gateway);
        assert!(e[0].message.contains("not on the same network"));
        let mut s = manual4("192.168.1.20", 24, None);
        s.ipv4.addresses.clear();
        assert_eq!(validate(&s)[0].field, Field::Ipv4Address);
    }

    #[test]
    fn auto_ignores_leftover_addresses() {
        let mut s = manual4("nonsense", 99, Some("x"));
        s.ipv4.method = Ipv4Method::Auto;
        assert!(validate(&s).is_empty());
    }

    #[test]
    fn static_ipv6_and_dns_and_priority() {
        let mut s = ProfileSettings::default();
        s.ipv6.method = Ipv6Method::Manual;
        s.ipv6.addresses = vec![AddressEntry {
            address: "fd00::20".into(),
            prefix: 64,
        }];
        s.ipv6.gateway = Some("fe80::1".into());
        assert!(validate(&s).is_empty());
        s.ipv6.gateway = Some("1.2.3.4".into());
        assert_eq!(validate(&s)[0].field, Field::Ipv6Gateway);
        s.ipv6.gateway = None;
        s.dns = vec![
            "1.1.1.1".into(),
            "2606:4700:4700::1111".into(),
            "bad".into(),
        ];
        assert_eq!(validate(&s)[0].field, Field::Dns);
        s.dns.clear();
        s.priority = 1000;
        assert_eq!(validate(&s)[0].field, Field::Priority);
    }

    #[test]
    fn text_parsing() {
        let a = parse_addresses("192.168.1.20/24, 10.0.0.2/8", false).unwrap();
        assert_eq!(addresses_text(&a), "192.168.1.20/24, 10.0.0.2/8");
        assert!(
            parse_addresses("192.168.1.20", false)
                .unwrap_err()
                .contains("prefix")
        );
        assert!(parse_addresses("fd00::1/64", false).is_err());
        assert!(parse_addresses("1.2.3.4/0", false).is_err());
        assert_eq!(parse_dns("1.1.1.1 8.8.8.8").unwrap().len(), 2);
        assert!(parse_dns("one.one.one.one").is_err());
    }
}
