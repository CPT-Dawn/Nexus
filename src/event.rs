use crossterm::event::{Event as CrosstermEvent, EventStream, KeyEvent, KeyEventKind};
use futures::StreamExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

use crate::network::types::{ConnectionStatus, WiFiNetwork};

/// Commands dispatched from the UI to the network backend.
/// Replaces the old stringly-typed `Event::Error("CONNECT:...")` hack.
#[derive(Debug, Clone)]
pub enum NetworkCommand {
    /// Connect to a known/open network
    Connect {
        ssid: String,
        password: Option<String>,
    },
    /// Connect to a hidden network
    ConnectHidden {
        ssid: String,
        password: Option<String>,
    },
    /// Disconnect the active connection
    Disconnect,
    /// Forget a saved network profile
    Forget { ssid: String },
    /// Trigger a WiFi scan
    Scan,
    /// Refresh connection info
    RefreshConnection,
}

/// Application-level events
#[derive(Debug, Clone)]
pub enum Event {
    /// User key press
    Key(KeyEvent),
    /// Animation / render tick
    Tick,
    /// Terminal resize
    Resize(u16, u16),
    /// WiFi scan results arrived
    NetworkScan(Vec<WiFiNetwork>),
    /// Connection status change
    ConnectionChanged(ConnectionStatus),
    /// A network command dispatched by the UI (processed by main loop)
    Command(NetworkCommand),
    /// An error from an async operation
    Error(String),
}

/// Handles event collection from multiple sources.
///
/// Uses crossterm's async `EventStream` (via `futures::StreamExt`) instead of
/// blocking `event::poll()` / `event::read()`, so no tokio worker thread is
/// ever blocked.
pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Event>,
    _tx: mpsc::UnboundedSender<Event>,
    stop: Arc<AtomicBool>,
}

impl EventHandler {
    /// Create a new event handler. Spawns background tasks for async input and tick generation.
    pub fn new(tick_rate_ms: u64) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let _tx = tx.clone();
        let stop = Arc::new(AtomicBool::new(false));

        // Async input task — uses crossterm's EventStream (non-blocking)
        let input_tx = tx.clone();
        let input_stop = stop.clone();
        tokio::spawn(async move {
            let mut reader = EventStream::new();
            loop {
                if input_stop.load(Ordering::Relaxed) {
                    return;
                }
                let maybe_event = reader.next().await;
                if input_stop.load(Ordering::Relaxed) {
                    return;
                }
                match maybe_event {
                    Some(Ok(CrosstermEvent::Key(key))) => {
                        if key.kind == KeyEventKind::Press
                            && input_tx.send(Event::Key(key)).is_err()
                        {
                            return;
                        }
                    }
                    Some(Ok(CrosstermEvent::Resize(w, h))) => {
                        if input_tx.send(Event::Resize(w, h)).is_err() {
                            return;
                        }
                    }
                    Some(Err(_)) | None => {
                        // Stream ended or errored — exit gracefully
                        return;
                    }
                    _ => {}
                }
            }
        });

        // Tick task
        let tick_tx = tx.clone();
        let tick_stop = stop.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(tick_rate_ms));
            loop {
                interval.tick().await;
                if tick_stop.load(Ordering::Relaxed) {
                    return;
                }
                if tick_tx.send(Event::Tick).is_err() {
                    return;
                }
            }
        });

        Self { rx, _tx: tx, stop }
    }

    /// Get a clone of the sender for forwarding network events
    pub fn sender(&self) -> mpsc::UnboundedSender<Event> {
        self._tx.clone()
    }

    /// Receive the next event
    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }

    /// Signal all background tasks to stop
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
