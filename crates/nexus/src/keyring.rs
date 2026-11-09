//! The Secret Service keyring (gnome-keyring, KWallet, KeePassXC...) as a
//! home for a Profile's Wi-Fi secret, through `oo7`. Opt-in per Profile: the
//! system store of NetworkManager stays the default.
//!
//! Nothing is cached and nothing stays connected: every call opens a session
//! on the user's session bus, does its one job and drops it, so a keyring that
//! starts, locks or dies later is simply seen the next time. Items are found
//! by attributes (never by label), and the secret is zeroized after use.

use std::collections::HashMap;
use std::time::Duration;

use oo7::dbus::{Collection, Service};
use zeroize::Zeroizing;

const APPLICATION: &str = "dev.cptdawn.Nexus";
/// A locked keyring may ask the person to unlock it; do not wait forever.
const PATIENCE: Duration = Duration::from_secs(20);

/// Why the keyring could not be used; the text is for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unavailable(pub String);

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Handle on the user's Secret Service. Cheap to copy around.
#[derive(Debug, Clone, Copy, Default)]
pub struct Keyring;

fn attributes(uuid: &str) -> HashMap<&'static str, String> {
    HashMap::from([
        ("application", APPLICATION.to_owned()),
        ("kind", "wifi-psk".to_owned()),
        ("profile-uuid", uuid.to_owned()),
    ])
}

fn why(e: impl std::fmt::Display) -> Unavailable {
    Unavailable(e.to_string())
}

async fn open() -> Result<Collection, Unavailable> {
    let work = async {
        let cnx = zbus::connection::Builder::session()
            .method_timeout(PATIENCE)
            .build()
            .await
            .map_err(|e| {
                tracing::debug!("no session bus for the keyring: {e}");
                Unavailable("there is no session bus to find it on".into())
            })?;
        // Encrypted if the service can, plain (over the private session bus)
        // if it cannot, whatever way it says no.
        let service = match Service::encrypted_with_connection(&cnx).await {
            Ok(service) => service,
            Err(_) => Service::plain_with_connection(&cnx).await.map_err(|e| {
                tracing::debug!("no Secret Service: {e}");
                Unavailable("no Secret Service is running".into())
            })?,
        };
        let collection = service.default_collection().await.map_err(why)?;
        if collection.is_locked().await.map_err(why)? {
            collection.unlock(None).await.map_err(why)?;
        }
        Ok::<_, Unavailable>(collection)
    };
    match tokio::time::timeout(PATIENCE, work).await {
        Ok(opened) => opened,
        Err(_) => Err(Unavailable("it did not answer in time".into())),
    }
}

impl Keyring {
    /// Keeps `secret` for the Profile with this UUID (replacing an older one).
    pub async fn store(&self, uuid: &str, ssid: &str, secret: &str) -> Result<(), Unavailable> {
        let collection = open().await?;
        let label = format!("Nexus: Wi-Fi password of {ssid}");
        collection
            .create_item(&label, &attributes(uuid), secret, true, None)
            .await
            .map(|_| ())
            .map_err(why)
    }

    /// The kept secret of the Profile, if the keyring holds one.
    pub async fn lookup(&self, uuid: &str) -> Result<Option<Zeroizing<String>>, Unavailable> {
        let collection = open().await?;
        let found = collection
            .search_items(&attributes(uuid))
            .await
            .map_err(why)?;
        let Some(item) = found.first() else {
            return Ok(None);
        };
        let secret = item.secret().await.map_err(why)?;
        Ok(Some(Zeroizing::new(
            String::from_utf8_lossy(secret.as_bytes()).into_owned(),
        )))
    }

    /// Forgets the Profile's secret. Nothing kept is not an error.
    pub async fn remove(&self, uuid: &str) -> Result<(), Unavailable> {
        let collection = open().await?;
        let found = collection
            .search_items(&attributes(uuid))
            .await
            .map_err(why)?;
        for item in found {
            item.delete(None).await.map_err(why)?;
        }
        Ok(())
    }
}
