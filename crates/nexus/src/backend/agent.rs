//! The NetworkManager secret agent: `org.freedesktop.NetworkManager.SecretAgent`
//! served on the Daemon's system-bus connection and registered with the
//! AgentManager, so NetworkManager asks Nexus for secrets it does not have.
//!
//! Every `GetSecrets` becomes a Secret prompt on the [`Broker`]. Secrets are
//! system-owned by default: `SaveSecrets` and `DeleteSecrets` are accepted and
//! do nothing, because NetworkManager keeps system-owned secrets in the
//! Profile itself (research 03).
//!
//! A Profile the person chose to "keep in keyring" has its `psk-flags` set to
//! agent-owned: NetworkManager never stores the secret, so it asks here, and
//! the agent answers from the Secret Service without a prompt (also for
//! autoconnect). Without the keyring it falls back to the Secret prompt, so
//! the join still works. Nothing here logs a settings dictionary or an
//! answer.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use nexus_ipc::{PromptField, PromptKind};
use zbus::{Connection, interface};
use zbus::{OwnedObjectPath, OwnedVariant, Variant};

use crate::keyring::Keyring;
use crate::secrets::{Broker, Request, Resolution};

pub const AGENT_PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
/// NetworkManager tells agents apart by this reverse-DNS name.
const IDENTIFIER: &str = "dev.cptdawn.Nexus";
const SERVICE: &str = "org.freedesktop.NetworkManager";
const SECURITY: &str = "802-11-wireless-security";
const VPN: &str = "vpn";
/// The hint prefix a VPN plugin uses for a sentence to show the person.
const MESSAGE_HINT: &str = "x-vpn-message:";
/// How long to wait before registering again after a failure.
const RETRY: Duration = Duration::from_secs(2);

/// `NM_SECRET_AGENT_GET_SECRETS_FLAGS_ALLOW_INTERACTION`: a person is there
/// to answer. Without it an agent must not prompt.
const ALLOW_INTERACTION: u32 = 0x1;
/// `NM_SECRET_AGENT_GET_SECRETS_FLAGS_REQUEST_NEW`: the last secret was wrong.
const REQUEST_NEW: u32 = 0x2;

type Settings = HashMap<String, HashMap<String, OwnedVariant>>;

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.NetworkManager.SecretAgent")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NoSecrets(String),
    UserCanceled(String),
    AgentCanceled(String),
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.AgentManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/AgentManager"
)]
trait AgentManager {
    fn register(&self, identifier: &str) -> zbus::Result<()>;
}

struct Agent {
    broker: Arc<Broker>,
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent").finish_non_exhaustive()
    }
}

/// What a request is for, read from the Profile NetworkManager sent along.
fn wifi_name(connection: &Settings) -> Option<Vec<u8>> {
    connection
        .get("802-11-wireless")?
        .get("ssid")?
        .try_clone()
        .ok()?
        .try_into()
        .ok()
}

fn connection_uuid(connection: &Settings) -> Option<String> {
    connection
        .get("connection")?
        .get("uuid")?
        .try_clone()
        .ok()?
        .try_into()
        .ok()
}

/// Whether the Wi-Fi secret of this Profile is the agent's to supply
/// (`psk-flags` has the agent-owned bit): the "keep in keyring" Profiles.
fn keyring_owned(connection: &Settings) -> bool {
    connection
        .get(SECURITY)
        .and_then(|s| s.get("psk-flags"))
        .and_then(|v| u32::try_from(v.try_clone().ok()?).ok())
        .is_some_and(|f| f & 0x1 != 0)
}

fn psk_of(connection: &Settings) -> Option<String> {
    connection
        .get(SECURITY)?
        .get("psk")?
        .try_clone()
        .ok()?
        .try_into()
        .ok()
}

fn key(connection_path: &str, setting_name: &str) -> String {
    format!("{connection_path}\n{setting_name}")
}

fn connection_text(connection: &Settings, section: &str, key: &str) -> Option<String> {
    connection
        .get(section)?
        .get(key)?
        .try_clone()
        .ok()?
        .try_into()
        .ok()
}

/// What a VPN secret called `name` is, from its name.
fn field_of(name: &str) -> PromptField {
    let n = name.to_lowercase();
    if ["otp", "challenge", "token", "2fa", "code"]
        .iter()
        .any(|k| n.contains(k))
    {
        PromptField::Otp
    } else if n.contains("user") {
        PromptField::Username
    } else {
        PromptField::Password
    }
}

impl Agent {
    /// A plugin VPN's secrets: one Secret prompt per name in the hints
    /// (`password`, `cert-pass`, ...), answers returned as `vpn.secrets`.
    async fn vpn_secrets(
        &self,
        connection: Settings,
        connection_path: OwnedObjectPath,
        hints: Vec<String>,
        flags: u32,
    ) -> Result<Settings, AgentError> {
        if flags & (ALLOW_INTERACTION | REQUEST_NEW) == 0 {
            return Err(AgentError::NoSecrets(
                "interaction is not allowed for this request".into(),
            ));
        }
        let message = hints
            .iter()
            .find_map(|h| h.strip_prefix(MESSAGE_HINT))
            .map(str::to_owned);
        let mut names: Vec<&str> = hints
            .iter()
            .map(String::as_str)
            .filter(|h| !h.starts_with(MESSAGE_HINT))
            .collect();
        if names.is_empty() {
            names.push("password");
        }
        let vpn = connection_text(&connection, "connection", "id").unwrap_or_else(|| "VPN".into());
        let uuid = connection_uuid(&connection).unwrap_or_default();
        let mut answers: HashMap<String, String> = HashMap::new();
        for name in names {
            let request = Request {
                kind: PromptKind::VpnSecret,
                field: field_of(name),
                message: message.clone(),
                network: vpn.clone(),
                network_id: uuid.clone(),
                retry: flags & REQUEST_NEW != 0,
            };
            match self
                .broker
                .ask(&key(connection_path.as_str(), VPN), request)
                .await
            {
                Resolution::Answered(secret) => {
                    answers.insert(name.to_owned(), secret.expose().to_owned());
                }
                Resolution::Cancelled => {
                    return Err(AgentError::UserCanceled("the user declined".into()));
                }
                Resolution::Expired => {
                    return Err(AgentError::NoSecrets("nobody answered in time".into()));
                }
                Resolution::Withdrawn => {
                    return Err(AgentError::AgentCanceled("request withdrawn".into()));
                }
            }
        }
        let value = OwnedVariant::try_from(Variant::new(answers)).map_err(AgentError::ZBus)?;
        Ok(HashMap::from([(
            VPN.to_owned(),
            HashMap::from([("secrets".to_owned(), value)]),
        )]))
    }
}

#[interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl Agent {
    async fn get_secrets(
        &self,
        connection: Settings,
        connection_path: OwnedObjectPath,
        setting_name: String,
        hints: Vec<String>,
        flags: u32,
    ) -> Result<Settings, AgentError> {
        if setting_name == VPN {
            return self
                .vpn_secrets(connection, connection_path, hints, flags)
                .await;
        }
        let Some(ssid) = wifi_name(&connection).filter(|_| setting_name == SECURITY) else {
            // 802.1X prompts come with a later ticket.
            return Err(AgentError::NoSecrets(
                "Nexus cannot ask for this kind of secret yet".into(),
            ));
        };
        let kept = keyring_owned(&connection);
        if kept
            && flags & REQUEST_NEW == 0
            && let Some(uuid) = connection_uuid(&connection)
        {
            // The keyring is asked first, and quietly: this is also the
            // answer to an autoconnect nobody is watching.
            match Keyring.lookup(&uuid).await {
                Ok(Some(secret)) => {
                    let value = OwnedVariant::try_from(Variant::from(secret.as_str()))
                        .map_err(AgentError::ZBus)?;
                    return Ok(HashMap::from([(
                        setting_name,
                        HashMap::from([("psk".to_owned(), value)]),
                    )]));
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("the keyring cannot give a password: {e}"),
            }
        }
        if flags & (ALLOW_INTERACTION | REQUEST_NEW) == 0 {
            // Background request (autoconnect): nobody asked a person.
            return Err(AgentError::NoSecrets(
                "interaction is not allowed for this request".into(),
            ));
        }
        if flags & REQUEST_NEW != 0
            && connection_uuid(&connection).is_some_and(|u| self.broker.is_inline(&u))
        {
            // The caller typed this password into the join itself: tell
            // NetworkManager there is no better one, so the join fails as a
            // wrong password and the caller decides what to ask next.
            return Err(AgentError::NoSecrets(
                "the password given with the join was wrong".into(),
            ));
        }
        let field = hints
            .first()
            .map_or("psk", |h| h.rsplit('.').next().unwrap_or("psk"))
            .to_owned();
        let request = Request {
            kind: PromptKind::WifiPassword,
            field: PromptField::Password,
            message: None,
            network: nexus_ipc::display_ssid(&ssid),
            network_id: nexus_ipc::network_id(&ssid),
            retry: flags & REQUEST_NEW != 0,
        };
        match self
            .broker
            .ask(&key(connection_path.as_str(), &setting_name), request)
            .await
        {
            Resolution::Answered(secret) => {
                let value = OwnedVariant::try_from(Variant::from(secret.expose()))
                    .map_err(AgentError::ZBus)?;
                Ok(HashMap::from([(
                    setting_name,
                    HashMap::from([(field, value)]),
                )]))
            }
            Resolution::Cancelled => Err(AgentError::UserCanceled("the user declined".into())),
            Resolution::Expired => Err(AgentError::NoSecrets("nobody answered in time".into())),
            Resolution::Withdrawn => Err(AgentError::AgentCanceled("request withdrawn".into())),
        }
    }

    async fn cancel_get_secrets(&self, connection_path: OwnedObjectPath, setting_name: String) {
        self.broker
            .withdraw(&key(connection_path.as_str(), &setting_name));
    }

    /// System-owned secrets are stored by NetworkManager itself; a "keep in
    /// keyring" Profile's secret, once a join with it worked, goes to the keyring.
    async fn save_secrets(&self, connection: Settings, _connection_path: OwnedObjectPath) {
        if !keyring_owned(&connection) {
            return;
        }
        let (Some(uuid), Some(psk)) = (connection_uuid(&connection), psk_of(&connection)) else {
            return;
        };
        let psk = zeroize::Zeroizing::new(psk);
        let ssid = wifi_name(&connection)
            .map(|s| nexus_ipc::display_ssid(&s))
            .unwrap_or_default();
        if let Err(e) = Keyring.store(&uuid, &ssid, &psk).await {
            tracing::warn!("cannot keep the password of {ssid} in the keyring: {e}");
        }
    }

    /// A forgotten "keep in keyring" Profile takes its keyring entry along.
    async fn delete_secrets(&self, connection: Settings, _connection_path: OwnedObjectPath) {
        if !keyring_owned(&connection) {
            return;
        }
        if let Some(uuid) = connection_uuid(&connection)
            && let Err(e) = Keyring.remove(&uuid).await
        {
            tracing::warn!("cannot clear a keyring entry: {e}");
        }
    }
}

/// Serves the agent on `conn` and keeps it registered: now, and again every
/// time NetworkManager (re)appears, because a restart forgets its agents.
/// Returns only when the bus connection fails.
pub async fn keep_registered(conn: &Connection, broker: Arc<Broker>) -> zbus::Result<()> {
    use zbus::{MatchRule, MessageStream, message::Type};
    conn.object_server()
        .at(
            AGENT_PATH,
            Agent {
                broker: broker.clone(),
            },
        )
        .await?;
    // Watch first, so an owner change during registration is not missed.
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .interface("org.freedesktop.DBus")
        .member("NameOwnerChanged")
        .arg(0, SERVICE)
        .build()?;
    let mut owner = MessageStream::for_match_rule(rule, conn, None).await?;
    let mut registered = false;
    loop {
        let present = has_owner(conn).await;
        if present && !registered {
            match register(conn).await {
                Ok(()) => {
                    tracing::info!("registered as the NetworkManager secret agent");
                    registered = true;
                }
                Err(e) => tracing::debug!("cannot register the secret agent yet: {e}"),
            }
        }
        let retry = present && !registered;
        tokio::select! {
            changed = owner.next() => {
                if changed.is_none() {
                    return Err(zbus::Error::Failure("bus connection closed".into()));
                }
                registered = false;
                // A restarted NetworkManager no longer waits for any answer.
                broker.withdraw_all();
            }
            () = async {
                if retry {
                    tokio::time::sleep(RETRY).await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
        }
    }
}

async fn register(conn: &Connection) -> zbus::Result<()> {
    AgentManagerProxy::new(conn)
        .await?
        .register(IDENTIFIER)
        .await
}

async fn has_owner(conn: &Connection) -> bool {
    let Ok(bus) = zbus::fdo::DBusProxy::new(conn).await else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from(SERVICE) else {
        return false;
    };
    bus.name_has_owner(name).await.unwrap_or(false)
}
