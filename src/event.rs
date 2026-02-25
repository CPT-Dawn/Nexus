use crossterm::event::{Event as CrosstermEvent, KeyEvent, MouseEvent};
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::network::types::NetworkState;

/// Unified application event
#[derive(Debug)]
pub enum Event {
    /// Terminal key press
    Key(KeyEvent),
    /// Terminal mouse event
    Mouse(MouseEvent),
    /// Periodic tick for UI refresh
    Tick,
    /// Terminal resize
    Resize(u16, u16),
    /// Network state has been refreshed
    NetworkRefresh(Box<NetworkState>),
    /// A network action completed (success message)
    ActionSuccess(String),
    /// A network action failed (error message)
    ActionError(String),
}

/// Handles terminal input and tick events, sending them through a channel
pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Event>,
    _tx: mpsc::UnboundedSender<Event>,
}

impl EventHandler {
    /// Create a new event handler with the given tick rate.
    /// Spawns an async task that polls crossterm events and sends ticks.
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let event_tx = tx.clone();

        tokio::spawn(async move {
            let mut reader = crossterm::event::EventStream::new();
            let mut tick_interval = tokio::time::interval(tick_rate);

            loop {
                tokio::select! {
                    // Terminal event
                    maybe_event = reader.next() => {
                        match maybe_event {
                            Some(Ok(evt)) => {
                                let app_event = match evt {
                                    CrosstermEvent::Key(key) => Event::Key(key),
                                    CrosstermEvent::Mouse(mouse) => Event::Mouse(mouse),
                                    CrosstermEvent::Resize(w, h) => Event::Resize(w, h),
                                    _ => continue,
                                };
                                if event_tx.send(app_event).is_err() {
                                    break;
                                }
                            }
                            Some(Err(_)) => break,
                            None => break,
                        }
                    }
                    // Tick
                    _ = tick_interval.tick() => {
                        if event_tx.send(Event::Tick).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Self { rx, _tx: tx }
    }

    /// Get a clone of the sender for external event injection
    /// (e.g., from network poller tasks)
    pub fn sender(&self) -> mpsc::UnboundedSender<Event> {
        self._tx.clone()
    }

    /// Receive the next event
    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}
