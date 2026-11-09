//! Seam S2 for Portal login in the TUI: keystrokes in, `insta` frames out,
//! against a real Daemon with a fake NetworkManager behind a Portal. The
//! browser is a stand-in `xdg-open` on the Daemon's PATH.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_testkit::{ActiveConnection, NmConnectivity, NmState, PATIENCE, World};
use nexus_tui::tui::{self, App, Caps, Client, ColorTier, Glyphs, Msg};
use ratatui::buffer::Buffer;

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

const NARROW: (u16, u16) = (60, 20);
const STANDARD: (u16, u16) = (100, 28);
const WIDE: (u16, u16) = (140, 32);

struct Rig {
    world: World,
    app: App,
    rx: Receiver<Msg>,
    log: std::path::PathBuf,
    _client: Client,
}

impl Rig {
    /// Joined to CafeGuest, which holds the traffic behind a Portal.
    fn portal() -> Rig {
        let world = World::start(NEXUS);
        let log = world.dir().join("opened.log");
        world.install_tool("xdg-open", &format!("echo \"$1\" >> '{}'", log.display()));
        world
            .nm
            .set_primary(Some(ActiveConnection::wifi("CafeGuest")));
        world.nm.set_connectivity(NmConnectivity::Portal);
        world.nm.set_state(NmState::ConnectedGlobal);
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(Caps {
            color: ColorTier::TrueColor,
            glyphs: Glyphs::Unicode,
        });
        app.attach(world.socket(), tx.clone());
        let client = Client::spawn(world.socket(), tx);
        app.update(Msg::Resize(STANDARD.0, STANDARD.1));
        let mut rig = Rig {
            world,
            app,
            rx,
            log,
            _client: client,
        };
        rig.pump_until("the Portal and its finding", |a| {
            a.portal().is_some()
                && !a.activity().is_empty()
                && a.findings().iter().any(|f| f.id == "portal")
        });
        rig
    }

    fn pump_until(&mut self, what: &str, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(&self.app) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| {
                    panic!("timed out waiting for {what}:\n{}", self.text(STANDARD))
                });
            let msg = self.rx.recv_timeout(left).unwrap_or_else(|_| {
                panic!("timed out waiting for {what}:\n{}", self.text(STANDARD))
            });
            self.app.update(msg);
        }
    }

    fn key(&mut self, c: char) {
        self.app.update(Msg::Key(KeyEvent::new(
            KeyCode::Char(c),
            KeyModifiers::NONE,
        )));
    }

    fn text(&self, (w, h): (u16, u16)) -> String {
        text_of(&tui::render(&self.app, w, h))
    }

    fn opened(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .map(|t| t.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}

fn text_of(buf: &Buffer) -> String {
    let w = buf.area.width as usize;
    buf.content()
        .chunks(w)
        .map(|row| {
            row.iter()
                .map(|c| c.symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_hero_says_how_to_sign_in_and_other_tabs_get_a_banner() {
    let mut rig = Rig::portal();
    // On the Overview the hero carries the message (and keeps its big type).
    for (name, size) in [("narrow", NARROW), ("standard", STANDARD), ("wide", WIDE)] {
        let frame = rig.text(size);
        assert!(frame.contains("Press o to open it"), "{name}:\n{frame}");
        assert!(!frame.contains("Login required"), "{name}:\n{frame}");
        assert!(
            frame.contains("o login"),
            "the footer lists the key:\n{frame}"
        );
        insta::assert_snapshot!(format!("portal_overview_{name}"), frame);
    }
    // Anywhere else a banner takes the top row.
    rig.key('4');
    for (name, size) in [("narrow", NARROW), ("standard", STANDARD), ("wide", WIDE)] {
        let frame = rig.text(size);
        assert!(
            frame
                .lines()
                .next()
                .unwrap()
                .contains("Login required for CafeGuest"),
            "{name}:\n{frame}"
        );
        insta::assert_snapshot!(format!("portal_banner_{name}"), frame);
    }
}

#[test]
fn o_opens_the_login_page_and_says_so() {
    let mut rig = Rig::portal();
    assert!(rig.opened().is_empty(), "nothing opens by itself");
    rig.key('o');
    rig.pump_until("the toast", |a| a.toast().is_some());
    assert_eq!(
        rig.app.toast().unwrap().text,
        "Opened the login page in your browser."
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while rig.opened().is_empty() {
        assert!(Instant::now() < deadline, "the browser never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(rig.opened(), ["http://ping.archlinux.org/nm-check.txt"]);
}

#[test]
fn signing_in_drops_the_banner_and_welcomes_you_online() {
    let mut rig = Rig::portal();
    rig.world.nm.set_connectivity(NmConnectivity::Full);
    rig.pump_until("the toast", |a| a.portal().is_none() && a.toast().is_some());
    let frame = rig.text(STANDARD);
    assert!(!frame.contains("Login required"), "{frame}");
    assert!(frame.contains("You're online."), "{frame}");
    // `o` is just a key again.
    rig.key('o');
    assert!(rig.opened().is_empty());
}

#[test]
fn help_explains_why_a_browser_may_not_show_the_page() {
    let mut rig = Rig::portal();
    rig.key('?');
    let frame = rig.text(STANDARD);
    assert!(frame.contains("secure DNS"), "{frame}");
    assert!(frame.contains("Open a Portal's login page"), "{frame}");
    insta::assert_snapshot!("portal_help_standard", frame);
}
