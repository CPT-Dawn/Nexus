//! The Daemon connection, on its own thread. It turns the socket into
//! [`Msg`]s and reconnects by itself, so the UI never blocks on I/O.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::{self, Thread};
use std::time::Duration;

use nexus_ipc::{Connection, Event, Op};

use super::app::Msg;

/// First retry delay after losing the Daemon; doubles up to [`RETRY_MAX`].
const RETRY_MIN: Duration = Duration::from_millis(100);
const RETRY_MAX: Duration = Duration::from_secs(2);
/// How often the idle link thread wakes to see whether the UI has gone. It
/// reads nothing: the Daemon pushes `status_changed` for every change.
const WAKE: Duration = Duration::from_secs(1);

/// Handle to the connection thread; dropping it stops the thread at its
/// next wake-up.
#[derive(Debug)]
pub struct Client {
    stop: Arc<AtomicBool>,
    thread: Thread,
}

impl Client {
    pub fn spawn(socket: PathBuf, tx: Sender<Msg>) -> Client {
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let stop = stop.clone();
            thread::Builder::new()
                .name("nexus-daemon-link".into())
                .spawn(move || run(&socket, &tx, &stop))
                .expect("spawn the daemon link thread")
        };
        Client {
            stop,
            thread: handle.thread().clone(),
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.unpark();
    }
}

fn run(socket: &Path, tx: &Sender<Msg>, stop: &AtomicBool) {
    let mut delay = RETRY_MIN;
    while !stop.load(Ordering::SeqCst) {
        let ended = Connection::connect_to(socket).and_then(|conn| {
            delay = RETRY_MIN;
            session(conn, tx, stop)
        });
        // A closed channel means the UI is gone.
        if ended.is_err_and(|e| e.kind() == io::ErrorKind::BrokenPipe) {
            return;
        }
        // `Down` is idempotent: retrying while down redraws nothing.
        if tx.send(Msg::Down).is_err() {
            return;
        }
        thread::park_timeout(delay);
        delay = (delay * 2).min(RETRY_MAX);
    }
}

/// Runs until the connection fails (an `Err`) or the UI goes away
/// (`BrokenPipe`).
fn session(mut conn: Connection, tx: &Sender<Msg>, stop: &AtomicBool) -> io::Result<()> {
    let refused = |what: &str, f: nexus_ipc::Failure| {
        io::Error::other(format!("daemon refused {what}: {}", f.message))
    };
    conn.call(Op::Subscribe)?
        .map_err(|f| refused("subscribe", f))?;
    let push = |status| {
        tx.send(Msg::Status(status))
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
    };
    push(conn.status()?.map_err(|f| refused("status", f))?)?;
    while !stop.load(Ordering::SeqCst) {
        if let Some(Event::StatusChanged { status }) = conn.next_event(WAKE)? {
            push(status)?;
        }
    }
    Ok(())
}
