//! VPNs on NetworkManager: WireGuard Profiles (type `wireguard`) and plugin
//! VPNs (type `vpn`, listed only). Member names follow NetworkManager's D-Bus
//! reference: `Settings.AddConnection`, `Manager.ActiveConnections`,
//! `ActivateConnection`, `DeactivateConnection`, `Connection.Active`
//! (`Uuid`, `State`), and the `wireguard` setting (`private-key`, `peers`,
//! `listen-port`, `mtu`).

use std::collections::HashMap;
use std::time::Duration;

use futures_util::StreamExt as _;
use nexus_ipc::{
    NewTunnel, Secret, Vpn, VpnAdded, VpnKey, VpnKind, VpnList, VpnShare, VpnState, VpnToggled,
};
use zbus::Variant as Value;
use zbus::proxy::CacheProperties;
use zbus::{Connection, OwnedObjectPath, OwnedVariant};

use super::nm::{ActiveConnectionProxy, NetworkManagerProxy};
use super::wifi::{Settings, SettingsConnectionProxy, SettingsProxy, is_denial, new_uuid, variant};
use crate::text::{self, Failure};
use crate::wireguard::{self, Config, Interface, Peer};

/// `NMActiveConnectionState`.
const ACTIVATING: u32 = 1;
const ACTIVATED: u32 = 2;
const DEACTIVATED: u32 = 4;

/// How long a VPN may take to come up.
const UP_TIMEOUT: Duration = Duration::from_secs(45);

macro_rules! no_cache {
    ($proxy:ident, $conn:expr, $path:expr) => {
        $proxy::builder($conn)
            .path($path)
            .cache_properties(CacheProperties::No)
            .build()
            .await?
    };
}

struct Profile {
    path: OwnedObjectPath,
    uuid: String,
    name: String,
    kind: VpnKind,
    addresses: Vec<String>,
    endpoint: Option<String>,
    /// A plugin VPN's NetworkManager plugin (`openvpn`), when it names one.
    plugin: Option<String>,
}

fn get<T>(all: &Settings, section: &str, key: &str) -> Option<T>
where
    T: TryFrom<OwnedVariant>,
{
    all.get(section)?
        .get(key)?
        .try_clone()
        .ok()?
        .try_into()
        .ok()
}

async fn profiles(conn: &Connection) -> zbus::Result<Vec<Profile>> {
    let settings = SettingsProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let mut out = Vec::new();
    for path in settings.list_connections().await? {
        let proxy = no_cache!(SettingsConnectionProxy, conn, path.as_str());
        let Ok(all) = proxy.get_settings().await else {
            continue;
        };
        let kind = match get::<String>(&all, "connection", "type").as_deref() {
            Some("wireguard") => VpnKind::Wireguard,
            Some("vpn") => VpnKind::Plugin,
            _ => continue,
        };
        let (Some(uuid), Some(name)) = (
            get::<String>(&all, "connection", "uuid"),
            get::<String>(&all, "connection", "id"),
        ) else {
            continue;
        };
        let addresses = address_data(&all, "ipv4")
            .into_iter()
            .chain(address_data(&all, "ipv6"))
            .collect();
        let endpoint = peers_of(&all).into_iter().find_map(|p| p.endpoint);
        let plugin = get::<String>(&all, "vpn", "service-type")
            .and_then(|s| nexus_tui::plugin::name_of_service(&s).map(str::to_owned));
        out.push(Profile {
            path,
            uuid,
            name,
            kind,
            addresses,
            endpoint,
            plugin,
        });
    }
    Ok(out)
}

fn address_data(all: &Settings, section: &str) -> Vec<String> {
    let Some(rows) = get::<Vec<HashMap<String, OwnedVariant>>>(all, section, "address-data") else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            let address: String = row.get("address")?.try_clone().ok()?.try_into().ok()?;
            let prefix: u32 = row.get("prefix")?.try_clone().ok()?.try_into().ok()?;
            Some(format!("{address}/{prefix}"))
        })
        .collect()
}

/// The peers in a `wireguard` setting, secrets (pre-shared keys) not included.
fn peers_of(all: &Settings) -> Vec<Peer> {
    let Some(rows) = get::<Vec<HashMap<String, OwnedVariant>>>(all, "wireguard", "peers") else {
        return Vec::new();
    };
    let text = |row: &HashMap<String, OwnedVariant>, key: &str| -> Option<String> {
        row.get(key)?.try_clone().ok()?.try_into().ok()
    };
    rows.iter()
        .filter_map(|row| {
            Some(Peer {
                public_key: text(row, "public-key")?,
                preshared_key: None,
                allowed_ips: row
                    .get("allowed-ips")
                    .and_then(|v| v.try_clone().ok()?.try_into().ok())
                    .unwrap_or_default(),
                endpoint: text(row, "endpoint"),
                keepalive: row
                    .get("persistent-keepalive")
                    .and_then(|v| v.try_clone().ok()?.try_into().ok())
                    .filter(|k: &u32| *k > 0)
                    .and_then(|k| u16::try_from(k).ok()),
            })
        })
        .collect()
}

/// Profile UUID -> state, for every VPN that is up or coming up.
async fn states(conn: &Connection) -> zbus::Result<HashMap<String, VpnState>> {
    let nm = NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let mut out = HashMap::new();
    for path in nm.active_connections().await? {
        let active = no_cache!(ActiveConnectionProxy, conn, path.as_str());
        // An active connection can vanish between the two reads.
        let (Ok(uuid), Ok(state)) = (active.uuid().await, active.state().await) else {
            continue;
        };
        match state {
            ACTIVATING => out.insert(uuid, VpnState::Activating),
            ACTIVATED => out.insert(uuid, VpnState::Up),
            _ => None,
        };
    }
    Ok(out)
}

pub async fn list(conn: &Connection) -> zbus::Result<VpnList> {
    let states = states(conn).await?;
    let mut vpns: Vec<Vpn> = profiles(conn)
        .await?
        .into_iter()
        .map(|p| Vpn {
            state: states.get(&p.uuid).copied().unwrap_or(VpnState::Down),
            id: p.uuid,
            name: p.name,
            kind: p.kind,
            addresses: p.addresses,
            endpoint: p.endpoint,
        })
        .collect();
    vpns.sort_by_key(|v| v.name.to_lowercase());
    Ok(VpnList { vpns })
}

async fn find(conn: &Connection, id: &str) -> zbus::Result<Option<Profile>> {
    Ok(profiles(conn).await?.into_iter().find(|p| p.uuid == id))
}

fn root() -> OwnedObjectPath {
    OwnedObjectPath::try_from("/").expect("root path")
}

/// Brings a VPN up and waits until it is carrying traffic or has failed.
pub async fn up(conn: &Connection, id: &str) -> zbus::Result<Result<VpnToggled, Failure>> {
    let Some(p) = find(conn, id).await? else {
        return Ok(Err(text::unknown_vpn()));
    };
    if let Some(plugin) = p.plugin.as_deref()
        && !nexus_tui::plugin::installed(plugin)
    {
        return Ok(Err(text::missing_plugin(plugin)));
    }
    let nm = NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let active = match nm.activate_connection(&p.path, &root(), &root()).await {
        Ok(path) => path,
        Err(e) if is_denial(&e) => {
            return Ok(Err(text::vpn_failed(
                &p.name,
                "NetworkManager did not authorize it",
            )));
        }
        Err(zbus::Error::MethodError(_, Some(why), _)) => {
            return Ok(Err(text::vpn_failed(&p.name, why.trim_end_matches('.'))));
        }
        Err(e) => return Err(e),
    };
    // The stream is opened before the state is read, so no change slips by.
    let proxy = ActiveConnectionProxy::builder(conn)
        .path(active.as_str())
        .build()
        .await?;
    let mut changes = proxy.receive_state_changed().await;
    let outcome = tokio::time::timeout(UP_TIMEOUT, async {
        let mut state = proxy.state().await.unwrap_or(DEACTIVATED);
        loop {
            match state {
                ACTIVATED => return true,
                ACTIVATING => {}
                _ => return false,
            }
            match changes.next().await {
                Some(change) => state = change.get().await.unwrap_or(DEACTIVATED),
                None => return false,
            }
        }
    })
    .await;
    Ok(match outcome {
        Ok(true) => Ok(VpnToggled {
            id: p.uuid,
            name: p.name,
            state: VpnState::Up,
        }),
        Ok(false) => Err(text::vpn_failed(&p.name, "NetworkManager gave up on it")),
        Err(_) => Err(text::vpn_timed_out(&p.name)),
    })
}

/// Takes a VPN down.
pub async fn down(conn: &Connection, id: &str) -> zbus::Result<Result<VpnToggled, Failure>> {
    let Some(p) = find(conn, id).await? else {
        return Ok(Err(text::unknown_vpn()));
    };
    let nm = NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let mut stopped = false;
    for path in nm.active_connections().await? {
        let active = no_cache!(ActiveConnectionProxy, conn, path.as_str());
        if active.uuid().await.is_ok_and(|u| u == p.uuid) {
            nm.deactivate_connection(&path).await?;
            stopped = true;
        }
    }
    Ok(if stopped {
        Ok(VpnToggled {
            id: p.uuid,
            name: p.name,
            state: VpnState::Down,
        })
    } else {
        Err(text::vpn_not_up(&p.name))
    })
}

fn string_list(items: &[String]) -> Value<'static> {
    Value::new(items.to_vec())
}

fn dict_rows(rows: Vec<Vec<(&str, Value<'static>)>>) -> OwnedVariant {
    let rows: Vec<HashMap<String, Value<'static>>> = rows
        .into_iter()
        .map(|row| row.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
        .collect();
    OwnedVariant::try_from(Value::new(rows)).expect("no file descriptors")
}

fn split_prefix(cidr: &str) -> (String, u32) {
    match cidr.split_once('/') {
        Some((a, p)) => (a.to_owned(), p.parse().unwrap_or(32)),
        None => (cidr.to_owned(), 32),
    }
}

/// A short, valid interface name for a Profile called `name`.
fn interface_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(15)
        .collect();
    if out.is_empty() {
        out = "wg0".into();
    }
    out
}

/// The NetworkManager settings for a WireGuard Profile. The private key goes
/// only into this dictionary, which is handed over and dropped.
fn to_settings(config: &Config, name: &str, uuid: &str) -> Settings {
    let connection = HashMap::from([
        ("id".to_owned(), variant(name)),
        ("type".to_owned(), variant("wireguard")),
        ("uuid".to_owned(), variant(uuid)),
        ("interface-name".to_owned(), variant(interface_name(name))),
        ("autoconnect".to_owned(), variant(false)),
    ]);
    let i = &config.interface;
    let mut wg = HashMap::from([
        ("private-key".to_owned(), variant(i.private_key.expose())),
        ("private-key-flags".to_owned(), variant(0u32)),
    ]);
    if let Some(p) = i.listen_port {
        wg.insert("listen-port".to_owned(), variant(u32::from(p)));
    }
    if let Some(m) = i.mtu {
        wg.insert("mtu".to_owned(), variant(m));
    }
    if !config.peers.is_empty() {
        let rows = config
            .peers
            .iter()
            .map(|p| {
                let mut row = vec![
                    ("public-key", Value::from(p.public_key.clone())),
                    ("allowed-ips", string_list(&p.allowed_ips)),
                ];
                if let Some(e) = &p.endpoint {
                    row.push(("endpoint", Value::from(e.clone())));
                }
                if let Some(k) = &p.preshared_key {
                    row.push(("preshared-key", Value::from(k.expose().to_owned())));
                    row.push(("preshared-key-flags", Value::from(0u32)));
                }
                if let Some(k) = p.keepalive {
                    row.push(("persistent-keepalive", Value::from(u32::from(k))));
                }
                row
            })
            .collect();
        wg.insert("peers".to_owned(), dict_rows(rows));
    }
    let mut all = HashMap::from([
        ("connection".to_owned(), connection),
        ("wireguard".to_owned(), wg),
    ]);
    for (section, v6) in [("ipv4", false), ("ipv6", true)] {
        let mine: Vec<(String, u32)> = i
            .addresses
            .iter()
            .filter(|a| a.contains(':') == v6)
            .map(|a| split_prefix(a))
            .collect();
        let mut s = HashMap::new();
        if mine.is_empty() {
            s.insert("method".to_owned(), variant("disabled"));
        } else {
            s.insert("method".to_owned(), variant("manual"));
            s.insert(
                "address-data".to_owned(),
                dict_rows(
                    mine.into_iter()
                        .map(|(a, p)| vec![("address", Value::from(a)), ("prefix", Value::from(p))])
                        .collect(),
                ),
            );
            let dns: Vec<String> = i
                .dns
                .iter()
                .filter(|d| d.contains(':') == v6)
                .cloned()
                .collect();
            if !dns.is_empty() {
                s.insert(
                    "dns-data".to_owned(),
                    OwnedVariant::try_from(string_list(&dns)).expect("strings"),
                );
            }
            if !i.search.is_empty() {
                s.insert(
                    "dns-search".to_owned(),
                    OwnedVariant::try_from(string_list(&i.search)).expect("strings"),
                );
            }
        }
        all.insert(section.to_owned(), s);
    }
    all
}

async fn add(
    conn: &Connection,
    config: &Config,
    name: &str,
) -> zbus::Result<Result<VpnAdded, Failure>> {
    let Some(public_key) = wireguard::public_of(config.interface.private_key.expose()) else {
        return Ok(Err(text::invalid_config("the private key is not a key")));
    };
    let uuid = new_uuid();
    let settings = SettingsProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    settings
        .add_connection(to_settings(config, name, &uuid))
        .await?;
    Ok(Ok(VpnAdded {
        id: uuid,
        name: name.to_owned(),
        public_key,
    }))
}

/// A name for an imported config that came without one.
fn default_name() -> String {
    "WireGuard".to_owned()
}

/// How long `nmcli` may take to import a file.
const IMPORT_TIMEOUT: Duration = Duration::from_secs(30);

/// A private directory that is removed, files and all, when dropped. A
/// provider file can hold keys, so it never sits where others can read it.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> std::io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt as _;
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(std::env::temp_dir);
        let dir = base.join(format!("nexus-import-{}", new_uuid()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A file name `nmcli` will turn into the Profile's name.
fn file_stem(name: &str, fallback: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let clean = clean.trim().trim_start_matches('.');
    if clean.is_empty() {
        fallback.to_owned()
    } else {
        clean.to_owned()
    }
}

/// `Connection 'Office' (uuid) successfully added.` -> `("Office", "uuid")`.
fn parse_added(stdout: &str) -> Option<(String, String)> {
    let line = stdout
        .lines()
        .find(|l| l.contains("successfully added"))?
        .trim();
    let rest = line.strip_prefix("Connection '")?;
    let at = rest.rfind("' (")?;
    let (name, rest) = (&rest[..at], &rest[at + 3..]);
    let uuid = rest.split(')').next()?;
    Some((name.to_owned(), uuid.to_owned()))
}

/// Imports a provider file (`.ovpn`, ...) with `nmcli connection import`:
/// the plugin's own importer is not reachable over D-Bus.
async fn import_plugin(
    name: Option<&str>,
    plugin: &str,
    config: &Secret,
) -> Result<VpnAdded, Failure> {
    let Some(known) = nexus_tui::plugin::by_name(plugin) else {
        return Err(text::unknown_plugin(plugin));
    };
    if !nexus_tui::plugin::installed(plugin) {
        return Err(text::missing_plugin(plugin));
    }
    let internal = |e: std::io::Error| Failure {
        code: nexus_ipc::ErrorCode::Internal,
        message: format!("Could not run nmcli: {e}."),
    };
    let scratch = Scratch::new().map_err(internal)?;
    let ext = known.extensions.first().copied().unwrap_or(known.name);
    let stem = file_stem(name.unwrap_or(""), known.label);
    let file = scratch.0.join(format!("{stem}.{ext}"));
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file)
            .map_err(internal)?;
        f.write_all(config.expose().as_bytes()).map_err(internal)?;
    }
    let run = tokio::process::Command::new("nmcli")
        .args(["connection", "import", "type", plugin, "file"])
        .arg(&file)
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = match tokio::time::timeout(IMPORT_TIMEOUT, run).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(internal(e)),
        Err(_) => {
            return Err(text::invalid_provider_file(
                plugin,
                "nmcli took too long to import it",
            ));
        }
    };
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        let said = said.trim().trim_start_matches("Error:").trim();
        // The path is a scratch name; the person does not need it.
        let said = said.replace(&file.display().to_string(), "the file");
        return Err(text::invalid_provider_file(
            plugin,
            if said.is_empty() {
                "nmcli refused it"
            } else {
                said.trim_end_matches('.')
            },
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some((name, id)) = parse_added(&stdout) else {
        return Err(text::invalid_provider_file(
            plugin,
            "nmcli did not say what it saved",
        ));
    };
    Ok(VpnAdded {
        id,
        name,
        public_key: String::new(),
    })
}

pub async fn import(
    conn: &Connection,
    name: Option<&str>,
    config: &Secret,
    plugin: Option<&str>,
) -> zbus::Result<Result<VpnAdded, Failure>> {
    if let Some(plugin) = plugin {
        return Ok(import_plugin(name, plugin, config).await);
    }
    let parsed = match wireguard::parse(config.expose()) {
        Ok(c) => c,
        Err(e) => return Ok(Err(text::invalid_config(&e.to_string()))),
    };
    let name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map_or_else(default_name, str::to_owned);
    add(conn, &parsed, &name).await
}

pub async fn create(
    conn: &Connection,
    tunnel: &NewTunnel,
) -> zbus::Result<Result<VpnAdded, Failure>> {
    let name = tunnel.name.trim();
    if name.is_empty() {
        return Ok(Err(text::invalid_config("the tunnel needs a name")));
    }
    let mut addresses = Vec::new();
    for a in &tunnel.addresses {
        match wireguard::cidr(a) {
            Some(c) => addresses.push(c),
            None => {
                return Ok(Err(text::invalid_config(&format!(
                    "`{a}` is not an address"
                ))));
            }
        }
    }
    if addresses.is_empty() {
        return Ok(Err(text::invalid_config("the tunnel needs an address")));
    }
    let (mut dns, mut search) = (Vec::new(), Vec::new());
    for d in &tunnel.dns {
        match d.parse::<std::net::IpAddr>() {
            Ok(ip) => dns.push(ip.to_string()),
            Err(_) => search.push(d.clone()),
        }
    }
    let mut peers = Vec::new();
    if let Some(key) = &tunnel.peer_key {
        if !wireguard::is_key(key) {
            return Ok(Err(text::invalid_config(
                "the peer's public key is not a WireGuard key (44 base64 characters)",
            )));
        }
        let mut allowed_ips = Vec::new();
        for a in &tunnel.allowed_ips {
            match wireguard::cidr(a) {
                Some(c) => allowed_ips.push(c),
                None => {
                    return Ok(Err(text::invalid_config(&format!(
                        "`{a}` is not a network"
                    ))));
                }
            }
        }
        if let Some(e) = &tunnel.endpoint
            && !wireguard::is_endpoint(e)
        {
            return Ok(Err(text::invalid_config(
                "the endpoint must look like host:port",
            )));
        }
        peers.push(Peer {
            public_key: key.clone(),
            preshared_key: None,
            allowed_ips,
            endpoint: tunnel.endpoint.clone(),
            keepalive: Some(25),
        });
    }
    let (private_key, _) = wireguard::generate();
    let config = Config {
        interface: Interface {
            private_key,
            addresses,
            dns,
            search,
            listen_port: None,
            mtu: None,
        },
        peers,
    };
    add(conn, &config, name).await
}

/// The stored private key of a WireGuard Profile and its pre-shared keys,
/// merged with the visible settings into a whole config. A polkit refusal
/// is `reveal_denied`.
async fn full_config(conn: &Connection, p: &Profile) -> zbus::Result<Result<Config, Failure>> {
    if p.kind != VpnKind::Wireguard {
        return Ok(Err(text::not_wireguard(&p.name)));
    }
    let proxy = no_cache!(SettingsConnectionProxy, conn, p.path.as_str());
    let all = proxy.get_settings().await?;
    let secrets = match proxy.get_secrets("wireguard").await {
        Ok(s) => s,
        Err(e) if is_denial(&e) => return Ok(Err(text::vpn_reveal_denied(&p.name))),
        Err(e) => return Err(e),
    };
    let Some(private_key) =
        get::<String>(&secrets, "wireguard", "private-key").filter(|k| wireguard::is_key(k))
    else {
        return Ok(Err(text::no_stored_key(&p.name)));
    };
    let mut peers = peers_of(&all);
    if let Some(rows) = get::<Vec<HashMap<String, OwnedVariant>>>(&secrets, "wireguard", "peers") {
        for row in rows {
            let text =
                |k: &str| -> Option<String> { row.get(k)?.try_clone().ok()?.try_into().ok() };
            if let (Some(key), Some(psk)) = (text("public-key"), text("preshared-key"))
                && let Some(peer) = peers.iter_mut().find(|q| q.public_key == key)
            {
                peer.preshared_key = Some(Secret::new(psk));
            }
        }
    }
    let mut interface = Interface {
        private_key: Secret::new(private_key),
        addresses: address_data(&all, "ipv4")
            .into_iter()
            .chain(address_data(&all, "ipv6"))
            .collect(),
        dns: get::<Vec<String>>(&all, "ipv4", "dns-data")
            .unwrap_or_default()
            .into_iter()
            .chain(get::<Vec<String>>(&all, "ipv6", "dns-data").unwrap_or_default())
            .collect(),
        search: get::<Vec<String>>(&all, "ipv4", "dns-search").unwrap_or_default(),
        listen_port: get::<u32>(&all, "wireguard", "listen-port")
            .filter(|p| *p > 0)
            .and_then(|p| u16::try_from(p).ok()),
        mtu: get::<u32>(&all, "wireguard", "mtu").filter(|m| *m > 0),
    };
    interface.addresses.dedup();
    Ok(Ok(Config { interface, peers }))
}

pub async fn key(conn: &Connection, id: &str) -> zbus::Result<Result<VpnKey, Failure>> {
    let Some(p) = find(conn, id).await? else {
        return Ok(Err(text::unknown_vpn()));
    };
    Ok(full_config(conn, &p).await?.and_then(|c| {
        wireguard::public_of(c.interface.private_key.expose())
            .map(|public_key| VpnKey {
                name: p.name.clone(),
                public_key,
            })
            .ok_or_else(|| text::no_stored_key(&p.name))
    }))
}

pub async fn reveal(conn: &Connection, id: &str) -> zbus::Result<Result<VpnShare, Failure>> {
    let Some(p) = find(conn, id).await? else {
        return Ok(Err(text::unknown_vpn()));
    };
    Ok(full_config(conn, &p).await?.and_then(|c| {
        let public_key = wireguard::public_of(c.interface.private_key.expose())
            .ok_or_else(|| text::no_stored_key(&p.name))?;
        Ok(VpnShare {
            name: p.name.clone(),
            public_key,
            config: Secret::new(wireguard::render(&c).as_str()),
        })
    }))
}
