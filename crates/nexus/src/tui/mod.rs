//! The TUI: event-driven, themed, one layout engine (ADR-0003).
//!
//! Wiring: a [`Client`] thread and an input thread feed one channel of
//! [`Msg`]; [`event_loop`] blocks on it and draws only when `App::update`
//! says the screen changed. With nothing happening it is asleep.

mod app;
mod bigtype;
mod caps;
mod client;
mod graph;
mod layout;
mod overview;
mod theme;
mod view;
mod wifi;
mod wifi_ui;
mod words;

use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;

use crossterm::event::{self, Event};
use ratatui::Terminal;
use ratatui::backend::{Backend, TestBackend};
use ratatui::buffer::Buffer;

pub use app::{App, Conn, Layer, Msg, Tab};
pub use caps::{Caps, ColorTier, Glyphs};
pub use client::Client;
pub use wifi::{PasswordField, Toast};

/// Draws `app` into a fresh buffer of the given size (what a snapshot sees).
pub fn render(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    terminal.draw(|f| view::draw(f, app)).expect("draw");
    terminal.backend().buffer().clone()
}

/// Blocks on `rx` and redraws only when a message changed the screen.
/// Messages that arrive together are applied together and cost one frame.
/// `on_frame` is called after every frame drawn. Returns when `app` quits.
pub fn event_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    rx: &Receiver<Msg>,
    mut on_frame: impl FnMut(&App),
) -> Result<(), B::Error> {
    terminal.draw(|f| view::draw(f, app))?;
    on_frame(app);
    while !app.should_quit() {
        let first = match app.next_tick() {
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(wait) => rx.recv_timeout(wait),
        };
        let mut dirty = match first {
            Ok(msg) => app.update(msg),
            Err(RecvTimeoutError::Timeout) => app.update(Msg::Tick),
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        };
        while let Ok(msg) = rx.try_recv() {
            dirty |= app.update(msg);
        }
        if dirty {
            terminal.draw(|f| view::draw(f, app))?;
            on_frame(app);
        }
    }
    Ok(())
}

/// Reads terminal input on its own thread. It blocks in the kernel, so it
/// costs nothing while the user is idle.
fn spawn_input(tx: Sender<Msg>) {
    thread::Builder::new()
        .name("nexus-input".into())
        .spawn(move || {
            while let Ok(ev) = event::read() {
                let msg = match ev {
                    Event::Key(k) => Msg::Key(k),
                    Event::Resize(w, h) => Msg::Resize(w, h),
                    _ => continue,
                };
                if tx.send(msg).is_err() {
                    return;
                }
            }
        })
        .expect("spawn the input thread");
}

/// `nexus` with no subcommand: the TUI on the real terminal and socket.
pub fn run(ascii: bool, nerd: bool) -> io::Result<()> {
    let Some(socket) = nexus_ipc::socket_path() else {
        eprintln!("nexus: XDG_RUNTIME_DIR is not set, so there is no daemon socket to find.");
        return Err(io::ErrorKind::NotFound.into());
    };
    let mut app = App::new(Caps::from_env(ascii, nerd));
    let (tx, rx) = mpsc::channel();
    let mut terminal = ratatui::try_init()?;
    let size = terminal.size()?;
    app.update(Msg::Resize(size.width, size.height));
    app.attach(socket.clone(), tx.clone());
    let client = Client::spawn(socket, tx.clone());
    spawn_input(tx);
    // Poll traffic exactly while the graphs are on screen (ADR-0003).
    let result = event_loop(&mut terminal, &mut app, &rx, |app| {
        client.watch_traffic(app.traffic_visible());
    });
    ratatui::restore();
    result
}
