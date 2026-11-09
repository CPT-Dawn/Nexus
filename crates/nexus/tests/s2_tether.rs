//! Seam S2 for Bluetooth Tethering on the Hotspot tab: keystrokes drive the
//! TUI, which drives a real Daemon (fake NetworkManager and fake BlueZ behind
//! it); `insta` frames come out at narrow, standard and wide widths. The
//! worlds are the testkit's, so `demo tether` shows the same.

use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_ipc::Bluetooth;
use nexus_testkit::scenarios::tether_world;
use nexus_testkit::{PATIENCE, World};
use nexus_tui::tui::{self, App, Caps, Client, ColorTier, Glyphs, Msg};
use ratatui::buffer::Buffer;

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

const NARROW: (u16, u16) = (60, 20);
const STANDARD: (u16, u16) = (100, 28);
const WIDE: (u16, u16) = (140, 32);

fn truecolor() -> Caps {
    Caps {
        color: ColorTier::TrueColor,
        glyphs: Glyphs::Unicode,
    }
}

struct Rig {
    world: World,
    app: App,
    rx: Receiver<Msg>,
    _client: Client,
}

impl Rig {
    /// The Tethering scenario, with the TUI on the Hotspot tab and the
    /// Daemon's picture received.
    fn new(powered: bool) -> Rig {
        Rig::with(tether_world(NEXUS, powered), truecolor())
    }

    fn with(world: World, caps: Caps) -> Rig {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(caps);
        app.attach(world.socket(), tx.clone());
        let client = Client::spawn(world.socket(), tx);
        app.update(Msg::Resize(STANDARD.0, STANDARD.1));
        let mut rig = Rig {
            world,
            app,
            rx,
            _client: client,
        };
        rig.key(KeyCode::Char('5'));
        rig.pump_until("the phones", |a| a.tether().has_data());
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

    fn key(&mut self, code: KeyCode) {
        self.app
            .update(Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    fn text(&self, (w, h): (u16, u16)) -> String {
        text_of(&tui::render(&self.app, w, h))
    }

    fn size(&mut self, (w, h): (u16, u16)) {
        self.app.update(Msg::Resize(w, h));
    }

    fn toast(&self) -> String {
        self.app.toast().map(|t| t.text.clone()).unwrap_or_default()
    }

    fn selected(&self) -> String {
        self.app
            .tether()
            .selected()
            .map(|p| p.name.clone())
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
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---- the list, at every width -------------------------------------------------

#[test]
fn phones_standard() {
    let rig = Rig::new(true);
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Pixel 8") && frame.contains("Sam's iPho"), "{frame}");
    insta::assert_snapshot!("tether_standard", frame);
}

#[test]
fn phones_wide() {
    let rig = Rig::new(true);
    insta::assert_snapshot!("tether_wide", rig.text(WIDE));
}

#[test]
fn phones_narrow_list_and_sheet() {
    let mut rig = Rig::new(true);
    rig.size(NARROW);
    insta::assert_snapshot!("tether_narrow", rig.text(NARROW));
    rig.key(KeyCode::Char('d'));
    insta::assert_snapshot!("tether_narrow_sheet", rig.text(NARROW));
}

#[test]
fn phones_in_plain_ascii() {
    let world = tether_world(NEXUS, true);
    let rig = Rig::with(
        world,
        Caps {
            color: ColorTier::None,
            glyphs: Glyphs::Ascii,
        },
    );
    insta::assert_snapshot!("tether_ascii", rig.text(STANDARD));
}

// ---- Bluetooth off, or no phone -----------------------------------------------

#[test]
fn bluetooth_off_says_so_in_the_list_and_the_detail() {
    let rig = Rig::new(false);
    assert_eq!(rig.app.tether().bluetooth(), Bluetooth::Off);
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Bluetooth is off"), "{frame}");
    insta::assert_snapshot!("tether_bluetooth_off", frame);
}

#[test]
fn enter_with_bluetooth_off_toasts_how_to_turn_it_on() {
    let mut rig = Rig::new(false);
    rig.key(KeyCode::Enter);
    rig.pump_until("the refusal", |a| a.toast().is_some());
    assert!(rig.toast().contains("Bluetooth is off"), "{}", rig.toast());
    assert!(rig.app.toast().unwrap().bad);
    assert!(rig.app.tether().busy.is_none());
    insta::assert_snapshot!("tether_bluetooth_off_toast", rig.text(STANDARD));
}

#[test]
fn no_paired_phone_explains_how_to_pair() {
    let world = World::builder(NEXUS).bluez(true).start();
    let rig = Rig::with(world, truecolor());
    let frame = rig.text(STANDARD);
    assert!(frame.contains("No paired phone"), "{frame}");
    insta::assert_snapshot!("tether_no_phone", frame);
}

#[test]
fn no_bluetooth_service_is_not_a_crash() {
    let world = World::start(NEXUS);
    let rig = Rig::with(world, truecolor());
    assert_eq!(rig.app.tether().bluetooth(), Bluetooth::Absent);
    insta::assert_snapshot!("tether_no_bluetooth", rig.text(STANDARD));
}

#[test]
fn switching_bluetooth_off_and_on_updates_the_tab() {
    let mut rig = Rig::new(true);
    rig.world.bluez().set_powered(false);
    rig.pump_until("Bluetooth off", |a| a.tether().bluetooth() == Bluetooth::Off);
    assert!(rig.text(STANDARD).contains("Bluetooth is off"));
    rig.world.bluez().set_powered(true);
    rig.pump_until("Bluetooth on", |a| a.tether().bluetooth() == Bluetooth::On);
    assert!(rig.text(STANDARD).contains("Bluetooth is on"));
}

// ---- tethering and dropping ---------------------------------------------------

#[test]
fn enter_tethers_x_drops_and_u_undoes() {
    let mut rig = Rig::new(true);
    assert_eq!(rig.selected(), "Galaxy S24 Ultra of the whole family");
    rig.key(KeyCode::Char('j'));
    assert_eq!(rig.selected(), "Pixel 8");
    rig.key(KeyCode::Enter);
    assert!(rig.app.tether().busy.is_some(), "marked as in flight");
    rig.pump_until("the tether", |a| a.toast().is_some());
    assert_eq!(rig.toast(), "Tethered through Pixel 8.");
    rig.pump_until("the Link in the list", |a| {
        a.tether().selected().is_some_and(|p| p.connected)
    });
    insta::assert_snapshot!("tether_connected", rig.text(STANDARD));
    // Dropping needs the person to have the footer's x.
    let footer = rig.text(STANDARD).lines().last().unwrap().to_string();
    assert!(footer.contains("drop tether"), "{footer}");

    rig.key(KeyCode::Char('x'));
    rig.pump_until("the drop", |a| {
        a.toast().is_some_and(|t| t.text.starts_with("Dropped"))
    });
    assert_eq!(rig.toast(), "Dropped the tether to Pixel 8.");
    rig.pump_until("the Link gone", |a| {
        a.tether().selected().is_some_and(|p| !p.connected)
    });

    rig.key(KeyCode::Char('u'));
    rig.pump_until("the undo", |a| {
        a.tether().selected().is_some_and(|p| p.connected)
    });
}

#[test]
fn a_failed_tether_toasts_the_reason_and_leaves_no_profile() {
    let mut rig = Rig::new(true);
    rig.world.nm.script_tether_failure("AA:BB:CC:DD:EE:02", 70, 5);
    rig.key(KeyCode::Char('j'));
    rig.key(KeyCode::Char('j'));
    assert_eq!(rig.selected(), "Sam's iPhone");
    rig.key(KeyCode::Enter);
    rig.pump_until("the failure", |a| a.toast().is_some_and(|t| t.bad));
    assert!(rig.toast().contains("Sam's iPhone"), "{}", rig.toast());
    assert_eq!(rig.world.nm.phone_profiles("AA:BB:CC:DD:EE:02"), 0);
    assert!(rig.app.tether().busy.is_none());
    insta::assert_snapshot!("tether_failed", rig.text(STANDARD));
}
