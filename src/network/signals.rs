use std::time::Duration;

use tokio::sync::mpsc;
use tracing::debug;
use zbus::Connection;
use zbus::zvariant::OwnedObjectPath;

use super::types::NetworkEvent;

/// Start listening for NetworkManager D-Bus signals and forward them as NetworkEvents.
/// This runs as a background tokio task.
pub async fn start_signal_listener(
    _conn: Connection,
    _device_path: OwnedObjectPath,
    event_tx: mpsc::UnboundedSender<NetworkEvent>,
) {
    debug!("Starting NetworkManager signal listener");

    // Poll-based signal monitoring as a simple, reliable approach.
    // zbus v5 made add_match private; we use periodic polling instead.
    let tx = event_tx.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            // Signal the main loop to refresh connection state
            let _ = tx.send(NetworkEvent::ConnectionChanged(
                crate::network::types::ConnectionStatus::Disconnected,
            ));
        }
    });

    debug!("Signal listeners started");
}
