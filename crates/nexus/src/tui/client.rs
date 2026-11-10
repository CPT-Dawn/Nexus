//! The Daemon connection, on its own thread. It turns the socket into
//! [`Msg`]s and reconnects by itself, so the UI never blocks on I/O.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
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
/// How often the link thread takes a traffic sample, while the graphs are
/// visible. With them hidden it does not wake on a timer at all: it sleeps in
/// `poll` until the Daemon speaks or the UI pokes the wake pipe (the graphs
/// came back, or the UI is going away). It re-reads no status; the Daemon
/// pushes `status_changed`.
const WAKE: Duration = Duration::from_secs(1);

/// Handle to the connection thread; dropping it stops the thread at its
/// next wake-up.
#[derive(Debug)]
pub struct Client {
    stop: Arc<AtomicBool>,
    traffic: Arc<AtomicBool>,
    checks: Arc<Checks>,
    thread: Thread,
    /// Pokes the link thread out of `poll`.
    wake: UnixStream,
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
        let (wake, sleeper) = UnixStream::pair().expect("make the wake pipe");
        wake.set_nonblocking(true).expect("wake pipe");
        sleeper.set_nonblocking(true).expect("wake pipe");
        let handle = {
            let (stop, traffic, checks) = (stop.clone(), traffic.clone(), checks.clone());
            thread::Builder::new()
                .name("nexus-daemon-link".into())
                .spawn(move || run(&socket, &tx, &stop, &traffic, &checks, &sleeper))
                .expect("spawn the daemon link thread")
        };
        Client {
            stop,
            traffic,
            checks,
            thread: handle.thread().clone(),
            wake,
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
        if self.traffic.swap(on, Ordering::SeqCst) != on {
            self.poke();
        }
    }

    /// Wakes the link thread so it re-reads `stop` and `traffic`. A full pipe
    /// already holds a pending poke, so a failed write is fine.
    fn poke(&self) {
        let _ = (&self.wake).write(&[1]);
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.unpark();
        self.poke();
    }
}

fn run(
    socket: &Path,
    tx: &Sender<Msg>,
    stop: &AtomicBool,
    traffic: &AtomicBool,
    checks: &Arc<Checks>,
    sleeper: &UnixStream,
) {
    let mut delay = RETRY_MIN;
    while !stop.load(Ordering::SeqCst) {
        let ended = Connection::connect_to(socket).and_then(|conn| {
            delay = RETRY_MIN;
            session(conn, tx, stop, traffic, checks, sleeper)
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
    sleeper: &UnixStream,
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
    if let Ok(tethers) = conn.tethers()? {
        send(Msg::Tethers(tethers))?;
    }
    if let Ok(got) = conn.call(Op::ConfigGet { key: None })?
        && let Some(config) = got.get("value").cloned()
    {
        send(Msg::Config {
            config,
            warning: got
                .get("warning")
                .and_then(|w| w.as_str())
                .map(String::from),
        })?;
        // Read-only: NetworkManager's own address, set in NetworkManager.conf.
        let uri = got
            .get("connectivity_uri")
            .and_then(|u| u.as_str())
            .map(String::from);
        send(Msg::ConnectivityUri(uri))?;
    }
    if let Ok(list) = conn.call(Op::EthernetList)?
        && let Ok(list) = serde_json::from_value(list)
    {
        send(Msg::Ethernet(list))?;
    }
    if let Ok(hotspot) = conn.call(Op::Hotspot)?
        && let Ok(hotspot) = serde_json::from_value(hotspot)
    {
        send(Msg::Hotspot(hotspot))?;
    }
    send(Msg::Activity(
        conn.activity()?
            .map_err(|f| refused("activity", f))?
            .entries,
    ))?;
    while !stop.load(Ordering::SeqCst) {
        // Sleep until the Daemon speaks or we are poked; wake on a timer
        // only to sample traffic while the graphs are visible.
        if !conn.has_buffered() {
            let sampling = traffic.load(Ordering::SeqCst);
            match sleep(&conn, sleeper, sampling.then_some(WAKE))? {
                Woken::Poked => continue,
                Woken::Timer => {
                    // A refusal (say NetworkManager vanished) skips a sample.
                    if let Ok(t) = conn.traffic()? {
                        send(Msg::Traffic(t))?;
                    }
                    continue;
                }
                Woken::Daemon => {}
            }
        }
        match conn.next_event(WAKE)? {
            Some(Event::StatusChanged { status }) => push(status)?,
            Some(Event::NetworksChanged { wifi }) => send(Msg::Wifi(wifi))?,
            Some(Event::VpnsChanged { vpns }) => send(Msg::Vpns(vpns))?,
            Some(Event::TethersChanged { tethers }) => send(Msg::Tethers(tethers))?,
            Some(Event::EthernetChanged { ethernet }) => send(Msg::Ethernet(ethernet))?,
            Some(Event::HotspotChanged { hotspot }) => send(Msg::Hotspot(hotspot))?,
            Some(event @ (Event::SecretPromptOpened { .. } | Event::SecretPromptClosed { .. })) => {
                send(Msg::Prompt(event))?
            }
            Some(Event::Activity { entry }) => send(Msg::Happened(entry))?,
            Some(Event::DoctorFindings { findings }) => send(Msg::Findings(findings))?,
            Some(Event::ConfigReloaded { config, warning }) => {
                send(Msg::Config { config, warning })?
            }
            Some(event @ (Event::ProfilePending { .. } | Event::ProfileSettled { .. })) => {
                send(Msg::Profile(super::ProfileMsg::Event(event)))?
            }
            Some(_) => {}
            None => {}
        }
    }
    Ok(())
}

enum Woken {
    Daemon,
    Poked,
    Timer,
}

/// Blocks until the Daemon's socket is readable, the wake pipe is poked, or
/// `timeout` passes (`None`: no timer, so an idle TUI is never scheduled).
fn sleep(conn: &Connection, sleeper: &UnixStream, timeout: Option<Duration>) -> io::Result<Woken> {
    use rustix::event::{PollFd, PollFlags, poll};
    use rustix::time::Timespec;
    let timeout = timeout.map(|t| Timespec {
        tv_sec: t.as_secs() as _,
        tv_nsec: t.subsec_nanos() as _,
    });
    let mut fds = [
        PollFd::new(conn, PollFlags::IN),
        PollFd::new(sleeper, PollFlags::IN),
    ];
    match poll(&mut fds, timeout.as_ref()) {
        Ok(0) => return Ok(Woken::Timer),
        Err(rustix::io::Errno::INTR) => return Ok(Woken::Poked),
        Err(e) => return Err(e.into()),
        Ok(_) => {}
    }
    if !fds[1].revents().is_empty() {
        let mut drain = [0u8; 64];
        while (&*sleeper).read(&mut drain).is_ok_and(|n| n > 0) {}
        return Ok(Woken::Poked);
    }
    Ok(Woken::Daemon)
}
