//! Seam S2 for the radios: Wi-Fi on/off (`w`), airplane mode (`A`), the
//! hardware-switch explanation and the undo that takes each back. Keystrokes
//! drive the TUI, which drives a real Daemon (fake NetworkManager behind it).

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_testkit::scenarios::wifi_busy;
use nexus_testkit::{ActiveConnection, NmConnectivity, NmState, PATIENCE, World};
use nexus_tui::tui::{self, App, Caps, Client, ColorTier, Glyphs, Msg};
use ratatui::buffer::Buffer;

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");
const STANDARD: (u16, u16) = (100, 28);
const NARROW: (u16, u16) = (60, 20);

struct Rig {
    world: World,
    app: App,
    rx: Receiver<Msg>,
    _client: Client,
}

impl Rig {
    fn busy() -> Rig {
        let world = World::start(NEXUS);
        wifi_busy(&world);
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
            _client: client,
        };
        rig.pump_until("the Wi-Fi list", |a| {
            a.wifi().len() == 8 && a.status().is_some()
        });
        rig.key(KeyCode::Char('2'));
        rig
    }

    /// `busy`, but with Home-5G carrying the machine online.
    fn online() -> Rig {
        let world = World::start(NEXUS);
        wifi_busy(&world);
        world
            .nm
            .set_primary(Some(ActiveConnection::wifi("Home-5G")));
        world.nm.set_connectivity(NmConnectivity::Full);
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
            _client: client,
        };
        rig.pump_until("online with a Link", |a| {
            a.wifi().len() == 8
                && a.status()
                    .is_some_and(|s| s.state == nexus_ipc::State::Online && s.link.is_some())
        });
        rig
    }

    fn pump_until(&mut self, what: &str, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(&self.app) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| panic!("timed out waiting for {what}:\n{}", self.text()));
            let msg = self
                .rx
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("timed out waiting for {what}:\n{}", self.text()));
            self.app.update(msg);
        }
    }

    /// Drains messages until the Daemon has been quiet for 400 ms.
    fn settle(&mut self) {
        while let Ok(msg) = self.rx.recv_timeout(Duration::from_millis(400)) {
            self.app.update(msg);
        }
    }

    fn key(&mut self, code: KeyCode) {
        self.app
            .update(Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            self.key(KeyCode::Char(c));
        }
    }

    fn text(&self) -> String {
        self.text_at(STANDARD)
    }

    fn text_at(&self, (w, h): (u16, u16)) -> String {
        text_of(&tui::render(&self.app, w, h))
    }

    fn radio_ready(&mut self, what: &str, done: impl Fn(&nexus_ipc::Radios) -> bool) {
        self.pump_until(what, |a| done(&a.radios()) && a.toast().is_some());
        self.settle();
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
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn w_turns_wifi_off_and_u_turns_it_back_on() {
    let mut rig = Rig::busy();
    assert!(rig.world.nm.wifi_enabled());
    rig.key(KeyCode::Char('w'));
    rig.radio_ready("Wi-Fi off", |r| !r.wifi.enabled);
    assert!(!rig.world.nm.wifi_enabled());
    let frame = rig.text();
    assert!(frame.contains("Wi-Fi is off."), "{frame}");
    assert!(frame.contains("Press w to turn it on."), "{frame}");
    insta::assert_snapshot!("radios_wifi_off_standard", frame);
    insta::assert_snapshot!("radios_wifi_off_narrow", rig.text_at(NARROW));

    rig.key(KeyCode::Char('u'));
    rig.radio_ready("Wi-Fi back on", |r| r.wifi.enabled);
    assert!(rig.world.nm.wifi_enabled());
    assert!(rig.text().contains("Wi-Fi is on."));
}

#[test]
fn with_wifi_off_the_hidden_rows_take_no_keys() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('w'));
    rig.radio_ready("Wi-Fi off", |r| !r.wifi.enabled);
    rig.settle();
    let frame = rig.text();
    let footer = frame.lines().last().unwrap_or_default();
    for word in ["disconnect", "forget", "share", "trust", "settings"] {
        assert!(!footer.contains(word), "footer offers {word}: {footer}");
    }
    // Forget would ask first; with nothing on screen it must not.
    rig.key(KeyCode::Char('f'));
    let frame = rig.text();
    assert!(!frame.contains("Forget"), "{frame}");
    rig.key(KeyCode::Char(':'));
    rig.type_text("forget network");
    rig.key(KeyCode::Enter);
    assert!(
        rig.app
            .toast()
            .is_some_and(|t| t.text.contains("Wi-Fi is off")),
        "{}",
        rig.text()
    );
}

#[test]
fn the_palette_turns_wifi_off_and_on() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char(':'));
    rig.type_text("turn wifi off");
    rig.key(KeyCode::Enter);
    rig.radio_ready("Wi-Fi off", |r| !r.wifi.enabled);
    rig.key(KeyCode::Char(':'));
    rig.type_text("turn wifi on");
    rig.key(KeyCode::Enter);
    rig.radio_ready("Wi-Fi on", |r| r.wifi.enabled);
    assert!(rig.world.nm.wifi_enabled());
}

#[test]
fn airplane_mode_turns_every_radio_off_and_u_takes_it_back() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('A'));
    rig.radio_ready("airplane mode", |r| r.airplane);
    assert!(!rig.world.nm.wifi_enabled());
    let frame = rig.text();
    assert!(frame.contains("Airplane mode is on"), "{frame}");
    insta::assert_snapshot!("radios_airplane_standard", frame);
    rig.key(KeyCode::Char('1'));
    let overview = rig.text();
    assert!(overview.contains("Airplane mode"), "{overview}");

    rig.key(KeyCode::Char('u'));
    rig.radio_ready("airplane off", |r| !r.airplane);
    assert!(rig.world.nm.wifi_enabled());
}

#[test]
fn a_hardware_switch_is_explained_and_w_cannot_override_it() {
    let mut rig = Rig::busy();
    rig.world.nm.set_wifi_hardware_blocked(true);
    rig.pump_until("the block", |a| a.radios().wifi.hardware_blocked);
    rig.settle();
    let frame = rig.text();
    assert!(frame.contains("hardware switch"), "{frame}");
    insta::assert_snapshot!("radios_hardware_blocked_standard", frame);

    rig.key(KeyCode::Char('w'));
    let frame = rig.text();
    assert!(frame.contains("hardware switch"), "{frame}");
    assert!(rig.app.toast().is_some_and(|t| t.bad), "{frame}");

    // The palette says why too.
    rig.key(KeyCode::Char(':'));
    rig.type_text("turn wifi on");
    rig.key(KeyCode::Enter);
    assert!(
        rig.app
            .toast()
            .is_some_and(|t| t.text.contains("hardware switch"))
    );
}

#[test]
fn the_help_lists_the_radio_keys() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('?'));
    let mut seen = String::new();
    for _ in 0..12 {
        seen.push_str(&rig.text());
        rig.key(KeyCode::PageDown);
    }
    assert!(seen.contains("Wi-Fi off or on"), "{seen}");
    assert!(seen.contains("airplane mode"), "{seen}");
}

#[test]
fn airplane_mode_takes_the_overview_offline_and_u_brings_the_link_back() {
    let mut rig = Rig::online();
    let before = rig.text();
    assert!(before.contains("Everything looks healthy"), "{before}");

    rig.key(KeyCode::Char('A'));
    rig.radio_ready("airplane mode", |r| r.airplane);
    rig.pump_until("the Link to drop", |a| {
        a.status()
            .is_some_and(|s| s.link.is_none() && s.state != nexus_ipc::State::Online)
    });
    rig.settle();
    let overview = rig.text();
    assert!(!overview.contains("ONLINE"), "{overview}");
    assert!(!overview.contains("Everything looks healthy"), "{overview}");

    rig.key(KeyCode::Char('u'));
    rig.pump_until("the Link to return", |a| {
        a.status()
            .is_some_and(|s| s.link.is_some() && s.state == nexus_ipc::State::Online)
    });
}
