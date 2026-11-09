//! A whole test world: private buses, fake NetworkManager and a real Daemon.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::{Bus, FakeNm};

/// How long to wait for the Daemon's socket, and for any single reply.
pub const PATIENCE: Duration = Duration::from_secs(10);

/// Builds a [`World`]. `bin` is `env!("CARGO_BIN_EXE_nexus")` in the test.
#[derive(Debug)]
pub struct WorldBuilder {
    bin: PathBuf,
    nm_running: bool,
    start_daemon: bool,
}

impl WorldBuilder {
    /// Whether the fake NetworkManager owns its name at start (default yes).
    pub fn nm_running(mut self, yes: bool) -> Self {
        self.nm_running = yes;
        self
    }

    /// Whether to start the Daemon (default yes). Without it, tests can start
    /// one themselves with [`World::start_daemon`], e.g. under socket activation.
    pub fn daemon(mut self, yes: bool) -> Self {
        self.start_daemon = yes;
        self
    }

    pub fn start(self) -> World {
        let tmp = tempfile::tempdir().expect("tempdir");
        for dir in ["run", "home"] {
            std::fs::create_dir(tmp.path().join(dir)).expect("mkdir");
        }
        let nm_bus = Bus::start(tmp.path(), "system-bus");
        let mut nm = FakeNm::start(&nm_bus.address);
        if !self.nm_running {
            nm.stop();
        }
        let mut world = World {
            bin: self.bin,
            nm,
            nm_bus,
            daemon: None,
            kill_group: false,
            tmp,
        };
        if self.start_daemon {
            world.start_daemon();
        }
        world
    }
}

/// Everything a scenario needs. Drop cleans up processes and files.
#[derive(Debug)]
pub struct World {
    /// The fake NetworkManager; script it from the test.
    pub nm: FakeNm,
    bin: PathBuf,
    nm_bus: Bus,
    daemon: Option<Child>,
    /// The adopted child leads a process group that holds the real Daemon.
    kill_group: bool,
    tmp: TempDir,
}

impl World {
    pub fn builder(bin: impl Into<PathBuf>) -> WorldBuilder {
        WorldBuilder {
            bin: bin.into(),
            nm_running: true,
            start_daemon: true,
        }
    }

    /// Fake NetworkManager running, Daemon started and accepting connections.
    pub fn start(bin: impl Into<PathBuf>) -> World {
        Self::builder(bin).start()
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.tmp.path().join("run")
    }

    pub fn socket(&self) -> PathBuf {
        self.runtime_dir().join("nexus").join("nexus.sock")
    }

    /// Where the user's optional `config.toml` lives in this world.
    pub fn config_path(&self) -> PathBuf {
        self.tmp.path().join("home/.config/nexus/config.toml")
    }

    /// Writes the config file the way a plain editor would (in place).
    pub fn write_config(&self, text: &str) {
        let path = self.config_path();
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir config dir");
        std::fs::write(path, text).expect("write config");
    }

    pub fn daemon_log(&self) -> String {
        std::fs::read_to_string(self.tmp.path().join("daemon.log")).unwrap_or_default()
    }

    /// A `nexus` command with the world's environment and nothing inherited,
    /// so the real buses and the real runtime dir are unreachable.
    pub fn nexus(&self) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.tmp.path().join("home"))
            .env("XDG_RUNTIME_DIR", self.runtime_dir())
            .env("DBUS_SYSTEM_BUS_ADDRESS", &self.nm_bus.address)
            // A session bus that does not exist: the Daemon must cope.
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!(
                    "unix:path={}",
                    self.tmp.path().join("no-session-bus").display()
                ),
            );
        if let Some(log) = std::env::var_os("RUST_LOG") {
            cmd.env("RUST_LOG", log);
        }
        cmd
    }

    /// Starts `nexus daemon` in the foreground and waits for its socket.
    pub fn start_daemon(&mut self) {
        let log = std::fs::File::create(self.tmp.path().join("daemon.log")).expect("log file");
        let mut cmd = self.nexus();
        cmd.arg("daemon")
            .stdin(Stdio::null())
            .stdout(log.try_clone().expect("clone log"))
            .stderr(log);
        let child = cmd.spawn().expect("spawn nexus daemon");
        self.adopt_daemon(child);
    }

    /// Kills the Daemon, as a crash or restart would. A later
    /// [`World::start_daemon`] brings a fresh one up on the same socket.
    pub fn stop_daemon(&mut self) {
        if let Some(mut d) = self.daemon.take() {
            let _ = d.kill();
            let _ = d.wait();
        }
    }

    /// Takes over a Daemon started by the test (e.g. under
    /// `systemd-socket-activate`) and waits until its socket accepts.
    pub fn adopt_daemon(&mut self, mut child: Child) {
        let socket = self.socket();
        let start = Instant::now();
        loop {
            if UnixStream::connect(&socket).is_ok() {
                self.daemon = Some(child);
                return;
            }
            if let Ok(Some(status)) = child.try_wait() {
                panic!("daemon exited early ({status}):\n{}", self.daemon_log());
            }
            assert!(
                start.elapsed() < PATIENCE,
                "daemon socket never appeared:\n{}",
                self.daemon_log()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Like [`World::adopt_daemon`], for a child spawned with
    /// `process_group(0)` that forks the Daemon (`systemd-socket-activate`):
    /// on drop the whole group is killed, not just the child.
    pub fn adopt_daemon_group(&mut self, child: Child) {
        self.kill_group = true;
        self.adopt_daemon(child);
    }

    /// A raw JSON-lines connection, for asserting on the wire itself.
    pub fn wire(&self) -> Wire {
        Wire::connect(&self.socket())
    }

    /// Runs `nexus <args>` to completion and returns its output.
    pub fn run_nexus(&self, args: &[&str]) -> std::process::Output {
        self.nexus().args(args).output().expect("run nexus")
    }
}

impl Drop for World {
    fn drop(&mut self) {
        if let Some(mut d) = self.daemon.take() {
            if self.kill_group {
                let _ = Command::new("kill")
                    .args(["-KILL", "--", &format!("-{}", d.id())])
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = d.kill();
            let _ = d.wait();
        }
    }
}

/// A raw client. Responses come back as whole JSON values so tests check the
/// wire, not the crate's types.
#[derive(Debug)]
pub struct Wire {
    reader: BufReader<UnixStream>,
    next_id: u64,
    patience: Duration,
}

impl Wire {
    pub fn connect(socket: &Path) -> Wire {
        Self::connect_patiently(socket, PATIENCE)
    }

    /// Like [`Wire::connect`], waiting up to `patience` for each reply (a
    /// real join, with a wrong password, takes tens of seconds).
    pub fn connect_patiently(socket: &Path, patience: Duration) -> Wire {
        let stream = UnixStream::connect(socket).expect("connect to the daemon socket");
        stream.set_read_timeout(Some(patience)).expect("timeout");
        Wire {
            reader: BufReader::new(stream),
            next_id: 1,
            patience,
        }
    }

    /// Sends one raw line (newline added) and returns the next line parsed.
    pub fn raw(&mut self, line: &str) -> Value {
        let stream = self.reader.get_mut();
        stream.write_all(line.as_bytes()).expect("write");
        stream.write_all(b"\n").expect("write");
        self.read_line()
    }

    /// Reads one line; `Value::Null` if the peer hung up without a word.
    pub fn read_line(&mut self) -> Value {
        let mut buf = String::new();
        match self.reader.read_line(&mut buf) {
            Ok(0) | Err(_) => Value::Null,
            Ok(_) => serde_json::from_str(&buf).expect("daemon replies in JSON"),
        }
    }

    /// Reads one line, waiting at most `wait`; `None` when nothing arrived.
    pub fn try_read_line(&mut self, wait: Duration) -> Option<Value> {
        self.reader
            .get_ref()
            .set_read_timeout(Some(wait))
            .expect("timeout");
        let mut buf = String::new();
        let got = matches!(self.reader.read_line(&mut buf), Ok(n) if n > 0);
        self.reader
            .get_ref()
            .set_read_timeout(Some(self.patience))
            .expect("timeout");
        got.then(|| serde_json::from_str(&buf).expect("daemon speaks JSON"))
    }

    /// Subscribes this connection to events; later [`Wire::next_event`] calls
    /// see everything the Daemon pushes from now on.
    pub fn subscribe(&mut self) {
        let resp = self.call(json!({"op": "subscribe"}));
        assert!(resp.get("ok").is_some(), "subscribe refused: {resp}");
    }

    /// Waits (up to [`PATIENCE`]) for the next event named `name`, skipping
    /// other lines, and returns it whole. Panics if none arrives.
    pub fn next_event(&mut self, name: &str) -> Value {
        loop {
            let line = self.read_line();
            assert!(!line.is_null(), "no `{name}` event arrived in time");
            if line["event"] == name {
                return line;
            }
        }
    }

    /// Sends `{"v":1,"id":<n>,...op}` and returns the whole response.
    pub fn call(&mut self, op: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let mut req = json!({"v": 1, "id": id});
        req.as_object_mut()
            .unwrap()
            .extend(op.as_object().expect("op is an object").clone());
        let resp = self.raw(&req.to_string());
        assert_eq!(resp["id"], id, "response id: {resp}");
        resp
    }
}
