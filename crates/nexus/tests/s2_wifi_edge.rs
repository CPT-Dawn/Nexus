//! Seam S2 for the awkward Wi-Fi cases on the TUI: the hidden-network form,
//! the access point details, the card chooser and SSIDs that are not UTF-8.
//! Keystrokes in, `insta` frames out, against a real Daemon (the testkit's
//! `wifi_edge` scenario, which `demo wifi-edge` shows).

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_testkit::scenarios::wifi_edge;
use nexus_testkit::{DeviceId, PATIENCE, World};
use nexus_tui::tui::{self, App, Caps, Client, ColorTier, Glyphs, Msg};
use ratatui::buffer::Buffer;

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

const NARROW: (u16, u16) = (60, 20);
const STANDARD: (u16, u16) = (100, 28);
const WIDE: (u16, u16) = (140, 32);

struct Rig {
    world: World,
    wlan0: DeviceId,
    wlan1: DeviceId,
    app: App,
    rx: Receiver<Msg>,
    _client: Client,
}

impl Rig {
    fn start() -> Rig {
        let world = World::start(NEXUS);
        let (wlan0, wlan1) = wifi_edge(&world);
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
            wlan0,
            wlan1,
            app,
            rx,
            _client: client,
        };
        rig.key(KeyCode::Char('2'));
        rig.pump_until("the Wi-Fi list", |a| a.wifi().len() == 4);
        rig.settle();
        rig
    }

    /// Drains the Daemon until it is quiet, so no late list moves a frame.
    fn settle(&mut self) {
        while let Ok(msg) = self.rx.recv_timeout(Duration::from_millis(400)) {
            self.app.update(msg);
        }
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

    fn type_str(&mut self, s: &str) {
        for c in s.chars() {
            self.key(KeyCode::Char(c));
        }
    }

    fn select(&mut self, ssid: &str) {
        self.key(KeyCode::Char('g'));
        for _ in 0..self.app.wifi().len() {
            if self.selected() == ssid {
                return;
            }
            self.key(KeyCode::Char('j'));
        }
        assert_eq!(self.selected(), ssid, "no such row");
    }

    fn selected(&self) -> String {
        self.app
            .wifi()
            .selected()
            .map(|e| e.name().to_owned())
            .unwrap_or_default()
    }

    fn text(&self, (w, h): (u16, u16)) -> String {
        text_of(&tui::render(&self.app, w, h))
    }

    fn toast(&self) -> String {
        self.app.toast().map(|t| t.text.clone()).unwrap_or_default()
    }

    /// Waits until the frame at standard width shows `needle`.
    fn until_shown(&mut self, what: &str, needle: &'static str) {
        self.pump_until(what, move |a| {
            text_of(&tui::render(a, STANDARD.0, STANDARD.1)).contains(needle)
        });
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

// ---- odd names and the list -----------------------------------------------------

#[test]
fn a_name_that_is_not_utf8_shows_a_replacement_mark_never_a_control_code() {
    let rig = Rig::start();
    let frame = rig.text(WIDE);
    assert!(frame.contains("caf\u{fffd} \u{fffd}[2Jwifi"), "{frame}");
    assert!(!frame.contains('\u{1b}'), "{frame:?}");
    // The hidden network is not listed.
    assert!(!frame.contains("Ghost-Net"));
    insta::assert_snapshot!("edge_list_wide", frame);
}

// ---- the hidden-network form ----------------------------------------------------

#[test]
fn the_hidden_form_at_every_width() {
    let mut rig = Rig::start();
    rig.key(KeyCode::Char('n'));
    for (name, size) in [
        ("edge_hidden_form_narrow", NARROW),
        ("edge_hidden_form_standard", STANDARD),
        ("edge_hidden_form_wide", WIDE),
    ] {
        rig.app.update(Msg::Resize(size.0, size.1));
        let frame = rig.text(size);
        assert!(frame.contains("Join a hidden network"), "{frame}");
        insta::assert_snapshot!(name, frame);
    }
}

#[test]
fn the_form_complains_inline_and_hides_the_password_row_for_open_networks() {
    let mut rig = Rig::start();
    rig.key(KeyCode::Char('n'));
    rig.key(KeyCode::Enter);
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Type the network's name."), "{frame}");
    insta::assert_snapshot!("edge_hidden_form_name_missing", frame);
    rig.type_str("Quiet");
    rig.key(KeyCode::Tab);
    for _ in 0..4 {
        rig.key(KeyCode::Right); // WPA2 -> WPA3 -> WPA2/WPA3 -> OWE -> Open
    }
    let frame = rig.text(STANDARD);
    assert!(
        frame.contains("Open") && !frame.contains("Password"),
        "{frame}"
    );
    insta::assert_snapshot!("edge_hidden_form_open", frame);
}

#[test]
fn a_hidden_network_joins_from_the_form_with_its_password() {
    let mut rig = Rig::start();
    rig.key(KeyCode::Char('n'));
    rig.type_str("Ghost-Net");
    rig.key(KeyCode::Tab);
    rig.key(KeyCode::Tab);
    rig.type_str("boo-boo-boo");
    let frame = rig.text(STANDARD);
    assert!(!frame.contains("boo-boo-boo"), "the password is masked");
    rig.key(KeyCode::Enter);
    rig.pump_until("the join", |a| {
        a.toast().is_some_and(|t| t.text.contains("Joined"))
    });
    assert_eq!(rig.toast(), "Joined Ghost-Net.");
    assert!(rig.app.hidden_form().is_none());
    assert_eq!(
        rig.world.nm.joined_ssid(rig.wlan0).as_deref(),
        Some("Ghost-Net")
    );
}

#[test]
fn a_hidden_network_nobody_hears_brings_the_form_back_with_the_reason() {
    let mut rig = Rig::start();
    rig.key(KeyCode::Char('n'));
    rig.type_str("Nowhere");
    rig.key(KeyCode::Tab);
    for _ in 0..4 {
        rig.key(KeyCode::Right);
    }
    rig.key(KeyCode::Enter);
    rig.pump_until("the form to come back", |a| a.hidden_form().is_some());
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Nowhere is not in range"), "{frame}");
    assert!(
        frame.contains("Network name"),
        "what was typed stays: {frame}"
    );
    insta::assert_snapshot!("edge_hidden_form_failed", frame);
    rig.key(KeyCode::Esc);
    assert!(rig.app.hidden_form().is_none());
}

// ---- access point details -------------------------------------------------------

#[test]
fn access_point_details_at_every_width() {
    let mut rig = Rig::start();
    rig.select("Mesh-Home");
    rig.key(KeyCode::Char('i'));
    rig.until_shown("the access points", "AA:00:00:00:10:02");
    for (name, size) in [
        ("edge_aps_narrow", NARROW),
        ("edge_aps_standard", STANDARD),
        ("edge_aps_wide", WIDE),
    ] {
        rig.app.update(Msg::Resize(size.0, size.1));
        let frame = rig.text(size);
        assert!(frame.contains("AA:00:00:00:10:01"), "{frame}");
        assert!(frame.contains("2.4G") && frame.contains("5G"), "{frame}");
        assert!(frame.contains("6G"), "6 GHz is shown: {frame}");
        insta::assert_snapshot!(name, frame);
    }
    rig.key(KeyCode::Esc);
    assert!(!rig.text(STANDARD).contains("Access points of"));
}

// ---- several cards --------------------------------------------------------------

#[test]
fn the_card_chooser_pins_joins_to_one_card() {
    let mut rig = Rig::start();
    rig.select("Mesh-Home");
    assert!(rig.text(STANDARD).contains("any (press c)"));
    rig.key(KeyCode::Char('c')); // wlan0
    rig.key(KeyCode::Char('c')); // wlan1
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Wi-Fi · wlan1"), "{frame}");
    insta::assert_snapshot!("edge_card_pinned", frame);
    rig.key(KeyCode::Enter);
    // Mesh-Home is secured and no password was given: the Daemon asks.
    rig.pump_until("the password prompt", |a| a.field().is_some());
    rig.type_str("mesh-passphrase");
    rig.key(KeyCode::Enter);
    rig.pump_until("the join", |a| {
        a.toast().is_some_and(|t| t.text.contains("Joined"))
    });
    assert_eq!(
        rig.world.nm.joined_ssid(rig.wlan1).as_deref(),
        Some("Mesh-Home"),
        "the pinned card joined, though wlan0 hears it better"
    );
    assert_eq!(rig.world.nm.joined_ssid(rig.wlan0), None);
    rig.key(KeyCode::Char('c')); // back to any card
    assert!(rig.text(STANDARD).contains("any (press c)"));
}

#[test]
fn a_card_that_cannot_hear_the_network_says_so() {
    let mut rig = Rig::start();
    rig.select("Airport Free");
    rig.key(KeyCode::Char('c'));
    rig.key(KeyCode::Char('c')); // wlan1 hears only Mesh-Home and Mixed-Mode
    rig.key(KeyCode::Enter);
    rig.pump_until("the refusal", |a| a.toast().is_some_and(|t| t.bad));
    assert!(
        rig.toast().contains("wlan1 cannot hear Airport Free"),
        "{}",
        rig.toast()
    );
}

#[test]
fn the_palette_lists_the_new_actions() {
    let mut rig = Rig::start();
    rig.key(KeyCode::Char(':'));
    rig.type_str("hidden");
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Join hidden network"), "{frame}");
}
