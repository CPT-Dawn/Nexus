//! The Daemon connection, on its own thread. It turns the socket into
//! [`Msg`]s and reconnects by itself, so the UI never blocks on I/O.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::thread::{self, Thread};
use std::time::Duration;

use nexus_ipc::{Connection, Connectivity, Event, Op, State, Status};

use super::app::Msg;

/// First retry delay after losing the Daemon; doubles up to [`RETRY_MAX`].
const RETRY_MIN: Duration = Duration::from_millis(100);
const RETRY_MAX: Duration = Duration::from_secs(2);
/// How often the idle link thread wakes to see whether the UI has gone. It
/// re-reads no status (the Daemon pushes `status_changed`); it takes a traffic
/// sample each time, while the graphs are visible.
const WAKE: Duration = Duration::from_secs(1);

/// Handle to the connection thread; dropping it stops the thread at its
/// next wake-up.
#[derive(Debug)]
pub struct Client {
    stop: Arc<AtomicBool>,
    traffic: Arc<AtomicBool>,
    checks: Arc<Checks>,
    thread: Thread,
}

/// The Doctor's calls. They run on threads of their own, because a lookup
/// that times out or a password dialog must never stall status updates.
#[derive(Debug)]
struct Checks {
    socket: PathBuf,
    tx: Sender<Msg>,
    /// Bumped by every request; an older answer is dropped.
    generation: AtomicU64,
}

impl Checks {
    /// Asks the Daemon to run the Doctor and reports the findings.
    fn check(self: &Arc<Self>) {
        let ticket = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let me = self.clone();
        thread::spawn(move || {
            let Ok(mut conn) = Connection::connect_to(&me.socket) else {
                return;
            };
            if let Ok(Ok(report)) = conn.doctor()
                && me.generation.load(Ordering::SeqCst) == ticket
            {
                let _ = me.tx.send(Msg::Findings(report.findings));
            }
        });
    }

    /// Nothing to check: drop what was found and any check in flight.
    fn clear(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let _ = self.tx.send(Msg::Findings(Vec::new()));
    }

    /// Runs a confirmed fix and then looks again.
    fn fix(self: &Arc<Self>, id: String) {
        let me = self.clone();
        thread::spawn(move || {
            let done = Connection::connect_to(&me.socket)
                .and_then(|mut conn| {
                    conn.call(Op::DoctorFix {
                        finding: id,
                        confirm: true,
                    })
                })
                .map_err(|e| format!("Could not reach the daemon: {e}"))
                .and_then(|r| r.map(|_| ()).map_err(|f| f.message));
            let _ = me.tx.send(Msg::FixDone(done));
            me.check();
        });
    }
}

/// Whether the Overview should ask the Doctor why: only when something looks
/// wrong, so a healthy machine costs no lookups at all.
fn worth_checking(status: &Status) -> bool {
    match status.state {
        State::Unavailable | State::Asleep | State::Local | State::Site => true,
        State::Online => matches!(
            status.connectivity,
            Connectivity::Portal | Connectivity::Limited
        ),
        _ => false,
    }
}

impl Client {
    pub fn spawn(socket: PathBuf, tx: Sender<Msg>) -> Client {
        let stop = Arc::new(AtomicBool::new(false));
        let traffic = Arc::new(AtomicBool::new(false));
        let checks = Arc::new(Checks {
            socket: socket.clone(),
            tx: tx.clone(),
            generation: AtomicU64::new(0),
        });
        let handle = {
            let (stop, traffic, checks) = (stop.clone(), traffic.clone(), checks.clone());
            thread::Builder::new()
                .name("nexus-daemon-link".into())
                .spawn(move || run(&socket, &tx, &stop, &traffic, &checks))
                .expect("spawn the daemon link thread")
        };
        Client {
            stop,
            traffic,
            checks,
            thread: handle.thread().clone(),
        }
    }

    /// Runs the fix of a Doctor finding the person confirmed. The outcome
    /// arrives as [`Msg::FixDone`], then fresh findings.
    pub fn run_fix(&self, finding: String) {
        self.checks.fix(finding);
    }

    /// Asks for traffic samples (about one a second) while `on`. The TUI
    /// turns this on exactly while the graphs are visible; off, the Daemon
    /// is never asked for counters.
    pub fn watch_traffic(&self, on: bool) {
        self.traffic.store(on, Ordering::SeqCst);
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.unpark();
    }
}

fn run(
    socket: &Path,
    tx: &Sender<Msg>,
    stop: &AtomicBool,
    traffic: &AtomicBool,
    checks: &Arc<Checks>,
) {
    let mut delay = RETRY_MIN;
    while !stop.load(Ordering::SeqCst) {
        let ended = Connection::connect_to(socket).and_then(|conn| {
            delay = RETRY_MIN;
            session(conn, tx, stop, traffic, checks)
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
fn session(
    mut conn: Connection,
    tx: &Sender<Msg>,
    stop: &AtomicBool,
    traffic: &AtomicBool,
    checks: &Arc<Checks>,
) -> io::Result<()> {
    let refused = |what: &str, f: nexus_ipc::Failure| {
        io::Error::other(format!("daemon refused {what}: {}", f.message))
    };
    conn.call(Op::Subscribe)?
        .map_err(|f| refused("subscribe", f))?;
    let send = |msg| {
        tx.send(msg)
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
    };
    let mut checked: Option<Status> = None;
    let mut push = |status: Status| {
        if checked.as_ref() != Some(&status) {
            if worth_checking(&status) {
                checks.check();
            } else {
                checks.clear();
            }
            checked = Some(status.clone());
        }
        send(Msg::Status(status))
    };
    push(conn.status()?.map_err(|f| refused("status", f))?)?;
    // Without a Backend there is no list yet; `networks_changed` brings it.
    if let Ok(list) = conn.wifi_list()? {
        send(Msg::Wifi(list))?;
    }
    if let Ok(list) = conn.call(Op::VpnList)?
        && let Ok(list) = serde_json::from_value(list)
    {
        send(Msg::Vpns(list))?;
    }
    send(Msg::Activity(
        conn.activity()?
            .map_err(|f| refused("activity", f))?
            .entries,
    ))?;
    while !stop.load(Ordering::SeqCst) {
        match conn.next_event(WAKE)? {
            Some(Event::StatusChanged { status }) => push(status)?,
            Some(Event::NetworksChanged { wifi }) => send(Msg::Wifi(wifi))?,
            Some(Event::VpnsChanged { vpns }) => send(Msg::Vpns(vpns))?,
            Some(event @ (Event::SecretPromptOpened { .. } | Event::SecretPromptClosed { .. })) => {
                send(Msg::Prompt(event))?
            }
            Some(Event::Activity { entry }) => send(Msg::Happened(entry))?,
            Some(Event::DoctorFindings { findings }) => send(Msg::Findings(findings))?,
            Some(_) => {}
            None => {
                // A refusal (say NetworkManager vanished) just skips a sample.
                if traffic.load(Ordering::SeqCst)
                    && let Ok(t) = conn.traffic()?
                {
                    send(Msg::Traffic(t))?;
                }
            }
        }
    }
    Ok(())
}
