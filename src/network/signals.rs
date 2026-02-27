use std::time::Duration;

use tokio::sync::mpsc;
use tracing::{debug, warn};
use zbus::Connection;
use zbus::zvariant::OwnedObjectPath;

use crate::event::Event;

/// Start listening for NetworkManager D-Bus signals and forward them as Events.
/// Uses zbus `MessageStream` to get real-time property change notifications
/// from NetworkManager instead of blind polling.
pub async fn start_signal_listener(
    conn: Connection,
    device_path: OwnedObjectPath,
    event_tx: mpsc::UnboundedSender<Event>,
) {
    debug!("Starting NetworkManager signal listener");

    // Attempt to subscribe to PropertiesChanged signals on our WiFi device.
    // If subscription fails, fall back to periodic polling.
    let sub_result =
        subscribe_device_signals(conn.clone(), device_path.clone(), event_tx.clone()).await;

    if let Err(e) = sub_result {
        warn!(
            "D-Bus signal subscription failed ({}), falling back to polling",
            e
        );
        let tx = event_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                // Signal the main loop to refresh connection state
                if tx
                    .send(Event::Command(
                        crate::event::NetworkCommand::RefreshConnection,
                    ))
                    .is_err()
                {
                    return;
                }
            }
        });
    }

    debug!("Signal listeners started");
}

/// Subscribe to D-Bus PropertiesChanged signals on the WiFi device.
/// Sends a RefreshConnection command whenever a property change is detected.
async fn subscribe_device_signals(
    conn: Connection,
    device_path: OwnedObjectPath,
    event_tx: mpsc::UnboundedSender<Event>,
) -> eyre::Result<()> {
    use futures::StreamExt;
    use zbus::MatchRule;

    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path(device_path.as_str())?
        .build();

    let proxy = zbus::fdo::DBusProxy::new(&conn).await?;
    proxy.add_match_rule(rule).await?;

    let mut stream = zbus::MessageStream::from(&conn);
    let tx = event_tx.clone();

    tokio::spawn(async move {
        // Debounce: don't send more than one refresh per 2 seconds
        let mut last_signal = tokio::time::Instant::now();
        let debounce = Duration::from_secs(2);

        while let Some(msg) = stream.next().await {
            if let Ok(msg) = msg {
                // Check if it's a signal related to our device
                let header = msg.header();
                let is_props_changed = header
                    .member()
                    .is_some_and(|m| m.as_str() == "PropertiesChanged");

                if is_props_changed && last_signal.elapsed() >= debounce {
                    last_signal = tokio::time::Instant::now();
                    debug!("D-Bus PropertiesChanged signal received, refreshing");
                    if tx
                        .send(Event::Command(
                            crate::event::NetworkCommand::RefreshConnection,
                        ))
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    });

    // Also keep a slower fallback poll for changes that don't trigger signals
    // (e.g., AP list changes after roaming)
    let tx2 = event_tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        loop {
            interval.tick().await;
            if tx2
                .send(Event::Command(
                    crate::event::NetworkCommand::RefreshConnection,
                ))
                .is_err()
            {
                return;
            }
        }
    });

    Ok(())
}
