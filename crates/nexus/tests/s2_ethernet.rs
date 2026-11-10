//! Seam S2 for the Ethernet tab: keystrokes drive the TUI, which drives a real
//! Daemon (fake NetworkManager with two wired Devices behind it); `insta`
//! frames come out at narrow, standard and wide widths. The world is the
//! testkit's, so `demo ethernet` shows the same.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_ipc::EthernetState;
use nexus_testkit::scenarios::ethernet_busy;
use nexus_testkit::{DeviceId, PATIENCE, World};
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
    app: App,
    rx: Receiver<Msg>,
    enp3s0: DeviceId,
    enp4s0: DeviceId,
    world: World,
    _client: Client,
}

impl Rig {
    /// The Ethernet scenario, with the TUI on the Ethernet tab and both ports
    /// read.
    fn busy() -> Rig {
        let world = World::start(NEXUS);
        let (enp3s0, enp4s0) = ethernet_busy(&world);
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(truecolor());
        app.attach(world.socket(), tx.clone());
        let client = Client::spawn(world.socket(), tx);
        app.update(Msg::Resize(STANDARD.0, STANDARD.1));
        let mut rig = Rig {
            app,
            rx,
            enp3s0,
            enp4s0,
            world,
            _client: client,
        };
        rig.key(KeyCode::Char('3'));
        rig.pump_until("the ports", |a| {
            let p = a.ethernet().ports();
            p.len() == 2
                && p[0].state == EthernetState::Misconfigured
                && p[1].state == EthernetState::Unplugged
        });
        rig.settle();
        rig
    }

    /// Drains messages until the Daemon has been quiet for 400 ms.
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

    fn text(&self, (w, h): (u16, u16)) -> String {
        text_of(&tui::render(&self.app, w, h))
    }

    fn size(&mut self, (w, h): (u16, u16)) {
        self.app.update(Msg::Resize(w, h));
    }

    fn toast(&self) -> String {
        self.app.toast().map(|t| t.text.clone()).unwrap_or_default()
    }

    /// Opens the sign-in form on the selected port and fills it the way an
    /// employee would, with the right server name.
    fn fill(&mut self, identity: &str, password: &str) {
        self.key(KeyCode::Char('s'));
        assert!(self.app.eap_form().is_some(), "{}", self.text(STANDARD));
        self.type_str(identity); // focus starts on Identity
        self.key(KeyCode::Tab); // anonymous
        self.key(KeyCode::Tab); // password
        self.type_str(password);
        self.key(KeyCode::Tab); // CA certificate
        self.key(KeyCode::Tab); // server name
        self.type_str("radius.office.example");
    }

    fn sign_in(&mut self, identity: &str, password: &str) {
        self.fill(identity, password);
        self.key(KeyCode::Enter);
        self.pump_until("the sign-in to end", |a| {
            a.toast().is_some() || a.eap_form().is_some_and(|f| f.banner.is_some())
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

// ---- the tab, at every width ----------------------------------------------------

#[test]
fn a_plugged_port_that_does_not_work_says_so() {
    let rig = Rig::busy();
    let frame = rig.text(STANDARD);
    assert!(
        frame.contains("enp3s0") && frame.contains("enp4s0"),
        "{frame}"
    );
    assert!(frame.contains("Cable in, not working"), "{frame}");
    assert!(frame.contains("gave enp3s0 no address"), "{frame}");
    assert!(!frame.contains("Cable unplugged"), "{frame}");
    insta::assert_snapshot!("ethernet_misconfigured_standard", frame);
}

#[test]
fn wide_and_narrow_layouts() {
    let mut rig = Rig::busy();
    insta::assert_snapshot!("ethernet_misconfigured_wide", rig.text(WIDE));
    rig.size(NARROW);
    insta::assert_snapshot!("ethernet_narrow_list", rig.text(NARROW));
    rig.key(KeyCode::Char('d'));
    insta::assert_snapshot!("ethernet_narrow_sheet", rig.text(NARROW));
}

#[test]
fn an_unplugged_port_is_not_the_same_picture() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('j'));
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Cable unplugged"), "{frame}");
    assert!(frame.contains("No cable detected on enp4s0"), "{frame}");
    assert!(!frame.contains("Cable in, not working"), "{frame}");
    insta::assert_snapshot!("ethernet_unplugged_standard", frame);
    // Enter explains instead of failing quietly.
    rig.key(KeyCode::Enter);
    assert!(
        rig.toast().starts_with("No cable detected"),
        "{}",
        rig.toast()
    );
    assert!(rig.app.toast().unwrap().bad);
}

#[test]
fn pulling_and_plugging_the_cable_moves_the_picture() {
    let mut rig = Rig::busy();
    rig.world.nm.set_carrier(rig.enp3s0, false);
    rig.pump_until("the cable out", |a| {
        a.ethernet().ports()[0].state == EthernetState::Unplugged
    });
    rig.world.nm.set_carrier(rig.enp4s0, true);
    rig.pump_until("a cable in", |a| {
        a.ethernet().ports()[1].state == EthernetState::Disconnected
    });
    rig.settle();
    insta::assert_snapshot!("ethernet_cables_swapped", rig.text(STANDARD));
}

// ---- joining and wired 802.1X ---------------------------------------------------

#[test]
fn trying_again_without_a_sign_in_fails_with_the_reason() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Enter);
    rig.pump_until("the retry to fail", |a| a.toast().is_some());
    assert!(rig.toast().contains("no address"), "{}", rig.toast());
    assert!(rig.app.toast().unwrap().bad);
    assert_eq!(
        rig.app.ethernet().selected().unwrap().state,
        EthernetState::Misconfigured
    );
}

#[test]
fn the_sign_in_form_is_the_enterprise_form_titled_for_the_port() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('s'));
    let frame = rig.text(STANDARD);
    assert!(
        frame.contains("Sign in to enp3s0") && frame.contains("PEAP") && frame.contains("MSCHAPv2"),
        "{frame}"
    );
    insta::assert_snapshot!("ethernet_eap_form", frame);
    // Esc backs out without touching anything.
    rig.key(KeyCode::Esc);
    assert!(rig.app.eap_form().is_none());
    assert!(rig.world.nm.ethernet_profiles().len() == 1);
}

#[test]
fn signing_in_brings_the_link_up_and_shows_its_details() {
    let mut rig = Rig::busy();
    rig.sign_in("alice@office.example", "hunter2");
    assert_eq!(rig.toast(), "Signed in. enp3s0 is up.");
    assert!(rig.app.eap_form().is_none());
    assert!(rig.world.nm.ethernet_up(rig.enp3s0));
    rig.pump_until("the connected port", |a| {
        a.ethernet().ports()[0].state == EthernetState::Connected
            && !a.ethernet().ports()[0].addresses.is_empty()
    });
    rig.settle();
    let frame = rig.text(STANDARD);
    for want in [
        "Connected",
        "1 Gbps",
        "10.20.30.41/24",
        "10.20.30.1",
        "10.20.30.2",
        "Office wall socket",
        "(Enterprise)",
    ] {
        assert!(frame.contains(want), "{want}:\n{frame}");
    }
    insta::assert_snapshot!("ethernet_connected_standard", frame);
    rig.size(WIDE);
    insta::assert_snapshot!("ethernet_connected_wide", rig.text(WIDE));
    rig.size(NARROW);
    rig.key(KeyCode::Char('d'));
    insta::assert_snapshot!("ethernet_connected_narrow", rig.text(NARROW));
}

#[test]
fn the_overview_card_carries_the_link_details() {
    let mut rig = Rig::busy();
    // Before: plugged in but not working.
    rig.key(KeyCode::Char('1'));
    let before = rig.text(STANDARD);
    assert!(before.contains("No address"), "{before}");
    rig.key(KeyCode::Char('3'));
    rig.sign_in("alice@office.example", "hunter2");
    rig.pump_until("the connected port", |a| {
        a.ethernet().ports()[0].state == EthernetState::Connected
            && !a.ethernet().ports()[0].addresses.is_empty()
    });
    rig.settle();
    rig.key(KeyCode::Char('1'));
    let after = rig.text(STANDARD);
    assert!(
        after.contains("1 Gbps, 10.20.30.41"),
        "the card is missing its Link details:\n{after}"
    );
    insta::assert_snapshot!("ethernet_overview_card", after);
}

#[test]
fn a_refused_sign_in_comes_back_to_the_form_with_everything_typed() {
    let mut rig = Rig::busy();
    rig.sign_in("alice@office.example", "wrong-password");
    rig.settle();
    let form = rig.app.eap_form().expect("the form is back");
    let banner = form.banner.clone().unwrap_or_default();
    assert!(banner.contains("refused the 802.1X sign-in"), "{banner}");
    let frame = rig.text(STANDARD);
    assert!(frame.contains("alice@office.example"), "{frame}");
    assert!(frame.contains("Sign in to enp3s0"), "{frame}");
    assert!(!frame.contains("wrong-password"), "{frame}");
    insta::assert_snapshot!("ethernet_eap_refused", frame);
    // The mistake is fixed in place: clear the password and retry.
    rig.key(KeyCode::BackTab); // domain .. back to password
    rig.key(KeyCode::BackTab);
    rig.key(KeyCode::BackTab);
    assert!(rig.app.eap_form().is_some());
}

#[test]
fn x_drops_the_link_and_enter_brings_it_back() {
    let mut rig = Rig::busy();
    rig.sign_in("alice@office.example", "hunter2");
    rig.pump_until("the connected port", |a| {
        a.ethernet().ports()[0].state == EthernetState::Connected
    });
    rig.key(KeyCode::Char('x'));
    rig.pump_until("the Link to drop", |a| {
        a.ethernet().ports()[0].state == EthernetState::Disconnected
    });
    assert_eq!(rig.toast(), "Dropped enp3s0.");
    // The Profile kept its sign-in, so Enter joins at once.
    rig.key(KeyCode::Enter);
    rig.pump_until("the Link back", |a| {
        a.ethernet().ports()[0].state == EthernetState::Connected
    });
    assert_eq!(rig.toast(), "enp3s0 is up.");
}

#[test]
fn u_takes_a_dropped_link_back() {
    let mut rig = Rig::busy();
    rig.sign_in("alice@office.example", "hunter2");
    rig.pump_until("the connected port", |a| {
        a.ethernet().ports()[0].state == EthernetState::Connected
    });
    rig.key(KeyCode::Char('x'));
    rig.pump_until("the Link to drop", |a| {
        a.ethernet().ports()[0].state == EthernetState::Disconnected
            && a.toast().is_some_and(|t| t.undo.is_some())
    });
    rig.key(KeyCode::Char('u'));
    rig.pump_until("the Link back", |a| {
        a.ethernet().ports()[0].state == EthernetState::Connected
    });
}

// ---- Profile settings -----------------------------------------------------------

#[test]
fn e_opens_the_profile_editor_for_the_ports_profile() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('e'));
    rig.pump_until("the Profile to load", |a| {
        a.editor().is_some_and(|e| e.view().is_some())
    });
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Office wall socket"), "{frame}");
    assert!(frame.contains("IPv4"), "{frame}");
    insta::assert_snapshot!("ethernet_profile_editor", frame);
    rig.key(KeyCode::Esc);
    assert!(rig.app.editor().is_none());
}

#[test]
fn a_port_without_a_profile_has_nothing_to_edit() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('j'));
    rig.key(KeyCode::Char('e'));
    assert!(rig.app.editor().is_none());
}

// ---- palette and help -----------------------------------------------------------

#[test]
fn the_palette_finds_the_sign_in_and_explains_when_it_cannot_run() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('j')); // the unplugged port
    rig.key(KeyCode::Char(':'));
    rig.type_str("802.1x");
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Sign in to Ethernet"), "{frame}");
    assert!(frame.contains("No cable is plugged in."), "{frame}");
    insta::assert_snapshot!("ethernet_palette_blocked", frame);
}

#[test]
fn the_footer_offers_what_the_port_can_do() {
    let mut rig = Rig::busy();
    let hints: Vec<_> = rig.app.hints(false).iter().map(|h| h.0).collect();
    assert!(hints.contains(&"enter") && hints.contains(&"s") && hints.contains(&"e"));
    rig.key(KeyCode::Char('j'));
    let hints: Vec<_> = rig.app.hints(false).iter().map(|h| h.0).collect();
    assert!(
        !hints.contains(&"enter") && !hints.contains(&"s"),
        "{hints:?}"
    );
}
