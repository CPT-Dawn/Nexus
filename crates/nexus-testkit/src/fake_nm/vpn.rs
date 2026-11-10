//! VPN activations in the fake NetworkManager: the `VPN.Connection` objects and their state walk.

#![allow(clippy::wildcard_imports)]

use super::*;

pub(super) fn vpn_active_path(n: usize) -> String {
    format!("{MANAGER_PATH}/ActiveConnection/vpn{n}")
}

/// A VPN's `Connection.Active` object, served while it is up or coming up.
pub(super) struct VpnActive(pub(super) Shared, pub(super) usize);

impl VpnActive {
    fn read<T>(&self, f: impl FnOnce(&ProfileModel, &VpnModel) -> T) -> Option<T> {
        let m = self.0.lock().unwrap();
        let p = m.profiles[self.1].as_ref()?;
        Some(f(p, p.vpn.as_ref()?))
    }
}

#[interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl VpnActive {
    #[zbus(property)]
    fn id(&self) -> String {
        self.read(|p, _| p.id.clone()).unwrap_or_default()
    }

    #[zbus(property, name = "Type")]
    fn kind(&self) -> String {
        self.read(|_, v| v.kind.clone()).unwrap_or_default()
    }

    #[zbus(property)]
    fn uuid(&self) -> String {
        self.read(|p, _| p.uuid.clone()).unwrap_or_default()
    }

    #[zbus(property)]
    fn state(&self) -> u32 {
        self.read(|_, v| v.active).unwrap_or(4)
    }
}

/// Sets a VPN's `State` and tells whoever listens.
pub(super) async fn vpn_state(conn: &Connection, model: &Shared, n: usize, state: u32) {
    if let Some(v) = model.lock().unwrap().profiles[n]
        .as_mut()
        .and_then(|p| p.vpn.as_mut())
    {
        v.active = state;
    }
    if let Ok(iface) = conn
        .object_server()
        .interface::<_, VpnActive>(vpn_active_path(n))
        .await
    {
        let guard = iface.get().await;
        let _ = guard.state_changed(iface.signal_emitter()).await;
    }
}

/// A VPN stops being active: observers see state 4, then the object goes.
pub(super) async fn end_vpn(conn: &Connection, model: &Shared, n: usize) {
    let was_active = model.lock().unwrap().profiles[n]
        .as_ref()
        .and_then(|p| p.vpn.as_ref())
        .is_some_and(|v| v.active > 0);
    if !was_active {
        return;
    }
    vpn_state(conn, model, n, 4).await;
    if let Some(v) = model.lock().unwrap().profiles[n]
        .as_mut()
        .and_then(|p| p.vpn.as_mut())
    {
        v.active = 0;
    }
    let _ = conn
        .object_server()
        .remove::<VpnActive, _>(vpn_active_path(n))
        .await;
    manager_changed(conn).await;
}

/// `ActivateConnection` on a VPN Profile: coming up (1), then up (2) or
/// failed, on the fake's own task.
pub(super) async fn start_vpn(
    conn: &Connection,
    model: &Shared,
    n: usize,
) -> zbus::fdo::Result<()> {
    {
        let mut m = model.lock().unwrap();
        let Some(v) = m.profiles[n].as_mut().and_then(|p| p.vpn.as_mut()) else {
            return Err(zbus::fdo::Error::InvalidArgs("no such connection".into()));
        };
        if v.active > 0 {
            return Ok(());
        }
        v.active = 1;
    }
    conn.object_server()
        .at(vpn_active_path(n), VpnActive(model.clone(), n))
        .await?;
    manager_changed(conn).await;
    let (c, m) = (conn.clone(), model.clone());
    let task = tokio::spawn(async move {
        tokio::task::yield_now().await;
        let (fails, asks, settings) = {
            let guard = m.lock().unwrap();
            let v = guard.profiles[n].as_ref().and_then(|p| p.vpn.as_ref());
            (
                v.is_some_and(|v| v.fails),
                v.and_then(|v| v.asks.clone()),
                v.map(|v| clone_settings(&v.settings)),
            )
        };
        let mut refused = false;
        if let (Some(hints), Some(settings)) = (asks, settings) {
            let body = (
                settings,
                opath(&profile_path(n)),
                "vpn",
                hints,
                ALLOW_INTERACTION,
            );
            match call_agent(&c, &m, "GetSecrets", &body).await {
                Ok(reply) => {
                    let secrets: Option<HashMap<String, String>> = reply
                        .body()
                        .deserialize::<SettingsDict>()
                        .ok()
                        .and_then(|mut all| {
                            all.remove("vpn")?
                                .remove("secrets")?
                                .try_clone()
                                .ok()?
                                .try_into()
                                .ok()
                        });
                    if let Some(v) = m.lock().unwrap().profiles[n]
                        .as_mut()
                        .and_then(|p| p.vpn.as_mut())
                    {
                        v.answers = secrets;
                    }
                }
                Err(_) => refused = true,
            }
        }
        if fails || refused {
            end_vpn(&c, &m, n).await;
        } else {
            vpn_state(&c, &m, n, 2).await;
        }
    });
    model.lock().unwrap().tasks.push(task.abort_handle());
    Ok(())
}
