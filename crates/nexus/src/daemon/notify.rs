//! Desktop notifications over the session bus (`org.freedesktop.Notifications`).
//!
//! The [`Notifier`] watches the Daemon's own events: a Network joined or
//! lost, a Secret prompt that opens while no Front end is subscribed, and
//! (told by the `wifi_join` handler) a wrong password. Each kind has a switch
//! in `[notifications]`, read when the event happens so edits apply at once.
//!
//! A Secret prompt notification carries an "Enter password" action. Clicking
//! it opens the TUI in a terminal; the TUI subscribes, the Broker replays the
//! open prompt, and the TUI shows it on whatever tab it starts on.
//!
//! The notification server is optional: the session bus is connected lazily,
//! and a missing or vanished server only costs the notifications.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_util::StreamExt as _;
use nexus_ipc::{ActivityKind, Event, Portal, PromptField, PromptKind};
use tokio::sync::{broadcast, mpsc};
use zbus::Variant;

use super::portal::SharedOpener;
use crate::config::Settings;
use crate::secrets::Broker;

const APP_NAME: &str = "Nexus";
const DESKTOP_ENTRY: &str = "nexus";
const ENTER: &str = "enter";
const OPEN_LOGIN: &str = "open-login";
/// Notification urgency levels from the specification.
const URGENCY_NORMAL: u8 = 1;
const URGENCY_CRITICAL: u8 = 2;
/// How long a transient notification stays, in milliseconds.
const BRIEF_MS: i32 = 6000;

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Variant<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;
    fn close_notification(&self, id: u32) -> zbus::Result<()>;
    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;
    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

/// How many Front ends are subscribed right now. A Front end that never
/// subscribes (a one-shot CLI call) does not count: it cannot show a prompt.
#[derive(Debug, Clone, Default)]
pub struct Frontends(Arc<AtomicUsize>);

impl Frontends {
    /// Registers a subscribed connection until the returned guard drops.
    pub fn join(&self) -> Presence {
        self.0.fetch_add(1, Ordering::SeqCst);
        Presence(self.0.clone())
    }

    fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

/// Proof that a Front end is subscribed.
#[derive(Debug)]
pub struct Presence(Arc<AtomicUsize>);

impl Drop for Presence {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// How the Daemon opens the TUI for the user. A trait so tests (and
/// anything without a terminal) never start a real one.
pub trait Terminal: Send + Sync {
    fn open_tui(&self);
}

/// Opens `nexus` through `xdg-terminal-exec`, else `$TERMINAL -e`.
#[derive(Debug)]
pub struct SystemTerminal;

impl Terminal for SystemTerminal {
    fn open_tui(&self) {
        let Ok(exe) = std::env::current_exe() else {
            tracing::warn!("cannot find the nexus binary to open in a terminal");
            return;
        };
        let terminal = std::env::var("TERMINAL").ok();
        let Some(argv) = terminal_command(&exe, on_path, terminal.as_deref()) else {
            tracing::warn!(
                "no terminal to open: install xdg-terminal-exec or set $TERMINAL, then run `nexus`"
            );
            return;
        };
        let mut command = std::process::Command::new(&argv[0]);
        command
            .args(&argv[1..])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        match command.spawn() {
            // Reaped on a thread, so a closed terminal leaves no zombie.
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(e) => tracing::warn!("cannot open a terminal ({}): {e}", argv[0]),
        }
    }
}

/// The command that shows `exe` in a terminal: `xdg-terminal-exec <exe>` when
/// that exists, else `$TERMINAL -e <exe>` (the variable may carry arguments).
fn terminal_command(
    exe: &Path,
    has: impl Fn(&str) -> bool,
    terminal_env: Option<&str>,
) -> Option<Vec<String>> {
    let exe = exe.to_string_lossy().into_owned();
    if has("xdg-terminal-exec") {
        return Some(vec!["xdg-terminal-exec".into(), exe]);
    }
    let mut words = terminal_env?.split_whitespace().map(str::to_owned);
    let program = words.next()?;
    let mut argv = vec![program];
    argv.extend(words);
    argv.extend(["-e".into(), exe]);
    Some(argv)
}

fn on_path(name: &str) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            std::fs::metadata(dir.join(name))
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    })
}

/// What the notifier task is told besides the Daemon's events.
enum Input {
    WrongPassword(String),
    VpnDropped(String),
    Policy {
        network: String,
        body: String,
    },
    Action {
        note: u32,
        key: String,
    },
    Closed(u32),
    /// The session bus connection ended.
    Gone,
}

/// A handle to the notifier task.
#[derive(Debug, Clone)]
pub struct Notifier {
    tx: mpsc::UnboundedSender<Input>,
}

impl std::fmt::Debug for Input {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Input")
    }
}

impl Notifier {
    /// Starts the task. It feeds on the same events Front ends get.
    pub fn spawn(
        settings: Arc<Settings>,
        broker: &Broker,
        frontends: Frontends,
        terminal: Arc<dyn Terminal>,
        opener: SharedOpener,
    ) -> Notifier {
        let (tx, rx) = mpsc::unbounded_channel();
        let (feed, backlog) = broker.subscribe();
        let task = Task {
            settings,
            frontends,
            terminal,
            opener,
            tx: tx.clone(),
            session: None,
            shown: HashMap::new(),
            portal: None,
        };
        tokio::spawn(task.run(feed, backlog, rx));
        Notifier { tx }
    }

    /// A VPN went down without anyone asking.
    pub fn vpn_dropped(&self, name: &str) {
        let _ = self.tx.send(Input::VpnDropped(name.to_owned()));
    }

    /// A Trust policy acted on joining an Untrusted Network; `body` says what
    /// it did.
    pub fn policy(&self, network: &str, body: &str) {
        let _ = self.tx.send(Input::Policy {
            network: network.to_owned(),
            body: body.to_owned(),
        });
    }

    /// A join was refused for a wrong password; `message` is the sentence
    /// the caller got.
    pub fn wrong_password(&self, message: &str) {
        let _ = self.tx.send(Input::WrongPassword(message.to_owned()));
    }
}

struct Task {
    settings: Arc<Settings>,
    frontends: Frontends,
    terminal: Arc<dyn Terminal>,
    opener: SharedOpener,
    tx: mpsc::UnboundedSender<Input>,
    session: Option<NotificationsProxy<'static>>,
    /// Secret prompt id -> the notification offering to answer it.
    shown: HashMap<u64, u32>,
    /// The "Login required" notification and the Portal it is about.
    portal: Option<(u32, Portal)>,
}

impl Task {
    async fn run(
        mut self,
        mut feed: broadcast::Receiver<Event>,
        backlog: Vec<Event>,
        mut rx: mpsc::UnboundedReceiver<Input>,
    ) {
        for event in backlog {
            self.event(event).await;
        }
        loop {
            tokio::select! {
                got = feed.recv() => match got {
                    Ok(event) => self.event(event).await,
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                Some(input) = rx.recv() => self.input(input).await,
            }
        }
    }

    async fn event(&mut self, event: Event) {
        let config = self.settings.notifications();
        match event {
            Event::Activity { entry } => {
                let wanted = match entry.kind {
                    ActivityKind::Joined => config.joined,
                    ActivityKind::Lost => config.lost,
                    _ => false,
                };
                if wanted {
                    self.show(&entry.text, "", URGENCY_NORMAL, &[], BRIEF_MS)
                        .await;
                }
                if entry.kind == ActivityKind::Online && config.portal {
                    self.show("You're online", &entry.text, URGENCY_NORMAL, &[], BRIEF_MS)
                        .await;
                }
            }
            Event::PortalChanged { portal } => self.portal_changed(portal, config.portal).await,
            Event::SecretPromptOpened { prompt }
                if config.secret_prompt
                    && matches!(
                        prompt.kind,
                        PromptKind::WifiPassword | PromptKind::VpnSecret
                    )
                    && self.frontends.count() == 0 =>
            {
                let millis =
                    i32::try_from(prompt.expires_in.saturating_mul(1000)).unwrap_or(i32::MAX);
                let title = match (prompt.kind, prompt.field) {
                    (PromptKind::VpnSecret, PromptField::Otp) => "One-time code needed",
                    (PromptKind::VpnSecret, PromptField::Username) => "VPN user name needed",
                    (PromptKind::VpnSecret, _) => "VPN password needed",
                    _ => "Password needed",
                };
                let id = self
                    .show(
                        title,
                        &prompt.message,
                        URGENCY_CRITICAL,
                        &[ENTER, "Enter password"],
                        millis,
                    )
                    .await;
                if let Some(note) = id {
                    self.shown.insert(prompt.id, note);
                }
            }
            Event::SecretPromptClosed { id, .. } => {
                if let (Some(note), Some(session)) = (self.shown.remove(&id), &self.session) {
                    let _ = session.close_notification(note).await;
                }
            }
            _ => {}
        }
    }

    async fn portal_changed(&mut self, portal: Option<Portal>, wanted: bool) {
        // Whatever was offered before is stale now: another Network, or signed in.
        if let Some((note, _)) = self.portal.take()
            && let Some(session) = &self.session
        {
            let _ = session.close_notification(note).await;
        }
        let Some(portal) = portal.filter(|_| wanted) else {
            return;
        };
        let id = self
            .show(
                &format!("Login required for {}", portal.network),
                "This network wants you to sign in before it gives internet access.",
                URGENCY_CRITICAL,
                &[OPEN_LOGIN, "Open login page"],
                0,
            )
            .await;
        if let Some(note) = id {
            self.portal = Some((note, portal));
        }
    }

    async fn input(&mut self, input: Input) {
        match input {
            Input::WrongPassword(message) => {
                if self.settings.notifications().wrong_password {
                    self.show("Wrong password", &message, URGENCY_NORMAL, &[], BRIEF_MS)
                        .await;
                }
            }
            Input::VpnDropped(name) => {
                if self.settings.notifications().vpn_dropped {
                    self.show(
                        "VPN dropped",
                        &crate::text::vpn_dropped(&name),
                        URGENCY_CRITICAL,
                        &[],
                        0,
                    )
                    .await;
                }
            }
            Input::Policy { network, body } => {
                if self.settings.notifications().policy {
                    self.show(
                        &format!("{network} is untrusted"),
                        &body,
                        URGENCY_NORMAL,
                        &[],
                        BRIEF_MS,
                    )
                    .await;
                }
            }
            Input::Action { note, key } => {
                if key == OPEN_LOGIN
                    && let Some((_, portal)) = self.portal.as_ref().filter(|(n, _)| *n == note)
                    && let Err(e) = self.opener.open(&portal.url)
                {
                    tracing::warn!("cannot open the login page: {e}");
                }
                if key == ENTER && self.shown.values().any(|n| *n == note) {
                    self.terminal.open_tui();
                }
            }
            Input::Closed(note) => {
                self.shown.retain(|_, n| *n != note);
                if self.portal.as_ref().is_some_and(|(n, _)| *n == note) {
                    self.portal = None;
                }
            }
            Input::Gone => self.session = None,
        }
    }

    /// Sends one notification; `None` when there is no server to take it.
    async fn show(
        &mut self,
        summary: &str,
        body: &str,
        urgency: u8,
        actions: &[&str],
        expire_ms: i32,
    ) -> Option<u32> {
        let session = self.connect().await?;
        let mut hints: HashMap<&str, Variant<'_>> = HashMap::new();
        hints.insert("desktop-entry", Variant::from(DESKTOP_ENTRY));
        hints.insert("urgency", Variant::from(urgency));
        match session
            .notify(
                APP_NAME,
                0,
                "network-wireless",
                summary,
                body,
                actions,
                hints,
                expire_ms,
            )
            .await
        {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::debug!("notification not delivered: {e}");
                self.session = None;
                None
            }
        }
    }

    /// The session bus proxy, connecting (and listening for clicks) if needed.
    async fn connect(&mut self) -> Option<NotificationsProxy<'static>> {
        if let Some(s) = &self.session {
            return Some(s.clone());
        }
        let conn = zbus::Connection::session()
            .await
            .map_err(|e| tracing::debug!("no session bus, no notifications: {e}"))
            .ok()?;
        let proxy = NotificationsProxy::new(&conn)
            .await
            .map_err(|e| tracing::debug!("no notification server: {e}"))
            .ok()?;
        let (mut actions, mut closed) = match (
            proxy.receive_action_invoked().await,
            proxy.receive_notification_closed().await,
        ) {
            (Ok(a), Ok(c)) => (a, c),
            _ => return None,
        };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            loop {
                let input = tokio::select! {
                    Some(signal) = actions.next() => match signal.args() {
                        Ok(a) => Input::Action { note: a.id, key: a.action_key },
                        Err(_) => continue,
                    },
                    Some(signal) = closed.next() => match signal.args() {
                        Ok(a) => Input::Closed(a.id),
                        Err(_) => continue,
                    },
                    else => break,
                };
                if tx.send(input).is_err() {
                    return;
                }
            }
            let _ = tx.send(Input::Gone);
        });
        self.session = Some(proxy.clone());
        Some(proxy)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn exe() -> PathBuf {
        PathBuf::from("/usr/bin/nexus")
    }

    #[test]
    fn xdg_terminal_exec_wins_when_present() {
        let argv = terminal_command(&exe(), |n| n == "xdg-terminal-exec", Some("foot")).unwrap();
        assert_eq!(argv, ["xdg-terminal-exec", "/usr/bin/nexus"]);
    }

    #[test]
    fn terminal_variable_is_the_fallback_and_may_carry_arguments() {
        let argv = terminal_command(&exe(), |_| false, Some("kitty --single-instance")).unwrap();
        assert_eq!(argv, ["kitty", "--single-instance", "-e", "/usr/bin/nexus"]);
    }

    #[test]
    fn no_launcher_at_all_is_none() {
        assert!(terminal_command(&exe(), |_| false, None).is_none());
        assert!(terminal_command(&exe(), |_| false, Some("  ")).is_none());
    }
}
