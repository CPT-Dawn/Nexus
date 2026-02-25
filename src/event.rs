use crossterm::event::{self, Event as CrosstermEvent, KeyEvent, KeyEventKind};
use std::time::Duration;
use tokio::sync::mpsc;

use crate::network::types::{ConnectionStatus, NetworkEvent, WiFiNetwork};

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
    /// A network event from the backend
    NetworkEvent(NetworkEvent),
    /// An error from an async operation
    Error(String),
}

/// Handles event collection from multiple sources
pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Event>,
    _tx: mpsc::UnboundedSender<Event>,
}

impl EventHandler {
    /// Create a new event handler. Spawns background tasks for input polling and tick generation.
    pub fn new(tick_rate_ms: u64) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let _tx = tx.clone();

        // Input polling task
        let input_tx = tx.clone();
        tokio::spawn(async move {
            loop {
                if event::poll(Duration::from_millis(tick_rate_ms)).unwrap_or(false)
                    && let Ok(evt) = event::read()
                {
                    match evt {
                        CrosstermEvent::Key(key) => {
                            // Only forward key press events, not release/repeat
                            if key.kind == KeyEventKind::Press
                                && input_tx.send(Event::Key(key)).is_err()
                            {
                                return;
                            }
                        }
                        CrosstermEvent::Resize(w, h) => {
                            if input_tx.send(Event::Resize(w, h)).is_err() {
                                return;
                            }
                        }
                        _ => {}
                    }
                }
            }
        });

        // Tick task
        let tick_tx = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(tick_rate_ms));
            loop {
                interval.tick().await;
                if tick_tx.send(Event::Tick).is_err() {
                    return;
                }
            }
        });

        Self { rx, _tx: tx }
    }

    /// Get a clone of the sender for forwarding network events
    pub fn sender(&self) -> mpsc::UnboundedSender<Event> {
        self._tx.clone()
    }

    /// Receive the next event
    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}
