//! The NetworkManager secret agent: `org.freedesktop.NetworkManager.SecretAgent`
//! served on the Daemon's system-bus connection and registered with the
//! AgentManager, so NetworkManager asks Nexus for secrets it does not have.
//!
//! Every `GetSecrets` becomes a Secret prompt on the [`Broker`]. Secrets are
//! system-owned by default: `SaveSecrets` and `DeleteSecrets` are accepted and
//! do nothing, because NetworkManager keeps system-owned secrets in the
//! Profile itself (research 03). Nothing here logs a settings dictionary or an
//! answer.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use nexus_ipc::PromptKind;
use zbus::{Connection, interface};
use zbus::{OwnedObjectPath, OwnedVariant, Variant};

use crate::secrets::{Broker, Request, Resolution};

pub const AGENT_PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
/// NetworkManager tells agents apart by this reverse-DNS name.
const IDENTIFIER: &str = "dev.cptdawn.Nexus";
const SERVICE: &str = "org.freedesktop.NetworkManager";
const SECURITY: &str = "802-11-wireless-security";
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

fn key(connection_path: &str, setting_name: &str) -> String {
    format!("{connection_path}\n{setting_name}")
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
        let Some(ssid) = wifi_name(&connection).filter(|_| setting_name == SECURITY) else {
            // VPN and 802.1X prompts come with later tickets.
            return Err(AgentError::NoSecrets(
                "Nexus cannot ask for this kind of secret yet".into(),
            ));
        };
        if flags & (ALLOW_INTERACTION | REQUEST_NEW) == 0 {
            // Background request (autoconnect): nobody asked a person.
            return Err(AgentError::NoSecrets(
                "interaction is not allowed for this request".into(),
            ));
        }
        let field = hints
            .first()
            .map_or("psk", |h| h.rsplit('.').next().unwrap_or("psk"))
            .to_owned();
        let request = Request {
            kind: PromptKind::WifiPassword,
            network: String::from_utf8_lossy(&ssid).into_owned(),
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

    /// System-owned secrets are stored by NetworkManager itself.
    async fn save_secrets(&self, _connection: Settings, _connection_path: OwnedObjectPath) {}

    async fn delete_secrets(&self, _connection: Settings, _connection_path: OwnedObjectPath) {}
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
