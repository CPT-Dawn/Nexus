//! Seam S2 for the Hotspot panel on the Hotspot tab: keystrokes drive the
//! TUI, which drives a real Daemon (fake NetworkManager behind it); `insta`
//! frames come out at narrow, standard and wide widths. The worlds are the
//! testkit's, so `demo hotspot` shows the same.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_testkit::scenarios::{HotspotStart, hotspot_world};
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
    /// The Hotspot scenario, with the TUI on the Hotspot tab and the Daemon's
    /// pictures (phones and Hotspot) received.
    fn new(start: HotspotStart) -> Rig {
        Rig::with(hotspot_world(NEXUS, start), truecolor())
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
        rig.pump_until("the phones and the Hotspot", |a| {
            a.tether().has_data() && a.hotspot().has_data()
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

    fn pump_until_text(&mut self, what: &str, needle: &str) {
        let deadline = Instant::now() + PATIENCE;
        while !self.text(STANDARD).contains(needle) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| panic!("timed out waiting for {what}:\n{}", self.text(STANDARD)));
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

    fn ctrl(&mut self, c: char) {
        self.app
            .update(Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)));
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
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

    /// The bottom row: the footer's keys.
    fn footer(&self) -> String {
        self.text(STANDARD).lines().last().unwrap_or("").to_string()
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

const PASSWORD: &str = "open sesame 42";

// ---- the panel, at every width ------------------------------------------------

#[test]
fn the_panel_sits_above_the_phones_when_off() {
    let rig = Rig::new(HotspotStart::Capable);
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Hotspot") && frame.contains("Off"), "{frame}");
    assert!(frame.contains("Tethering") && frame.contains("Pixel 8"), "{frame}");
    assert!(rig.footer().contains("hotspot"), "{}", rig.footer());
    insta::assert_snapshot!("hotspot_off_standard", frame);
}

#[test]
fn the_panel_when_off_wide_and_narrow() {
    let mut rig = Rig::new(HotspotStart::Capable);
    insta::assert_snapshot!("hotspot_off_wide", rig.text(WIDE));
    rig.size(NARROW);
    insta::assert_snapshot!("hotspot_off_narrow", rig.text(NARROW));
}

#[test]
fn a_running_hotspot_shows_its_name_and_device_count() {
    let mut rig = Rig::new(HotspotStart::Running);
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Garage Wi-Fi"), "{frame}");
    assert!(frame.contains("2 devices joined"), "{frame}");
    assert!(!frame.contains(PASSWORD), "never on the panel");
    insta::assert_snapshot!("hotspot_running_standard", frame);
    insta::assert_snapshot!("hotspot_running_wide", rig.text(WIDE));
    rig.size(NARROW);
    insta::assert_snapshot!("hotspot_running_narrow", rig.text(NARROW));
    let footer = rig.footer();
    assert!(footer.contains("stop hotspot"), "{footer}");
}

#[test]
fn a_card_that_cannot_host_explains_why() {
    let mut rig = Rig::new(HotspotStart::NoCard);
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Not available"), "{frame}");
    let why = rig.app.hotspot().why().expect("a reason").to_owned();
    assert!(why.contains("no access point mode"), "{why}");
    insta::assert_snapshot!("hotspot_nocard_standard", frame);
    rig.size(NARROW);
    insta::assert_snapshot!("hotspot_nocard_narrow", rig.text(NARROW));
    // n says the same instead of trying.
    rig.key(KeyCode::Char('n'));
    assert!(rig.toast().contains("cannot host a Hotspot"), "{}", rig.toast());
    assert!(rig.app.toast().unwrap().bad);
    assert!(!rig.app.hotspot().on());
}

#[test]
fn the_panel_in_plain_ascii() {
    let world = hotspot_world(NEXUS, HotspotStart::Running);
    let rig = Rig::with(
        world,
        Caps {
            color: ColorTier::None,
            glyphs: Glyphs::Ascii,
        },
    );
    insta::assert_snapshot!("hotspot_ascii", rig.text(STANDARD));
}

// ---- start, share, stop -------------------------------------------------------

#[test]
fn n_starts_with_a_made_up_password_and_s_reveals_it_behind_a_question() {
    let mut rig = Rig::new(HotspotStart::Capable);
    rig.key(KeyCode::Char('n'));
    assert_eq!(format!("{:?}", rig.app.hotspot().busy), "Starting");
    rig.pump_until("the Hotspot", |a| a.hotspot().ready());
    rig.pump_until("its toast", |a| a.toast().is_some());
    let name = rig.app.hotspot().ssid().expect("a name").to_owned();
    assert!(name.starts_with("Nexus-"), "{name}");
    assert!(rig.toast().contains(&name), "{}", rig.toast());
    let password = rig.world.nm.hotspot_psk(&name).expect("a password");
    assert_eq!(password.len(), 16);
    let frame = rig.text(STANDARD);
    assert!(!frame.contains(&password), "the password stays hidden: {frame}");
    assert_eq!(rig.world.nm.secret_reads(), 0, "nothing read yet");

    // s only asks.
    rig.key(KeyCode::Char('s'));
    let ask = rig.text(STANDARD);
    assert!(ask.contains("Show the password and a QR code"), "{ask}");
    assert_eq!(rig.world.nm.secret_reads(), 0, "still nothing read");
    insta::assert_snapshot!("hotspot_share_ask", ask);
    // enter reveals: the QR, the password as dots.
    rig.key(KeyCode::Enter);
    rig.pump_until_text("the QR code", "Password");
    let shown = rig.text(STANDARD);
    assert!(!shown.contains(&password), "dots first: {shown}");
    assert!(shown.contains('\u{2022}'), "{shown}");
    rig.key(KeyCode::Char('p'));
    assert!(rig.text(STANDARD).contains(&password));
    // esc drops it all.
    rig.key(KeyCode::Esc);
    assert!(!rig.text(STANDARD).contains("Share Nexus-"));
    assert!(!rig.text(STANDARD).contains(&password));
}

#[test]
fn the_qr_view_shows_the_code_and_the_dots() {
    let mut rig = Rig::new(HotspotStart::Running);
    rig.key(KeyCode::Char('s'));
    rig.key(KeyCode::Enter);
    rig.pump_until_text("the QR code", "Password");
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Share Garage Wi-Fi"), "{frame}");
    assert!(!frame.contains(PASSWORD), "{frame}");
    insta::assert_snapshot!("hotspot_share_shown", frame);
    // Too small a window says what it needs instead of a broken code.
    rig.size(NARROW);
    let small = rig.text(NARROW);
    assert!(small.contains("needs a window"), "{small}");
    insta::assert_snapshot!("hotspot_share_narrow", small);
}

#[test]
fn a_refused_reveal_says_so_and_can_be_retried() {
    let mut rig = Rig::new(HotspotStart::Running);
    rig.world.nm.deny_secret_reads(true);
    rig.key(KeyCode::Char('s'));
    rig.key(KeyCode::Enter);
    rig.pump_until_text("the refusal", "did not authorize");
    rig.world.nm.deny_secret_reads(false);
    rig.key(KeyCode::Enter);
    rig.pump_until_text("the QR code", "Password");
}

#[test]
fn capital_s_stops_it() {
    let mut rig = Rig::new(HotspotStart::Running);
    rig.key(KeyCode::Char('S'));
    rig.pump_until("the stop", |a| !a.hotspot().on());
    rig.pump_until("its toast", |a| a.toast().is_some());
    assert_eq!(rig.toast(), "Hotspot Garage Wi-Fi stopped.");
    assert!(rig.world.nm.hotspots().is_empty());
    assert_eq!(rig.world.nm.hotspot_profiles(), 0);
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Off"), "{frame}");
    insta::assert_snapshot!("hotspot_stopped", frame);
}

#[test]
fn stopping_closes_an_open_qr_view() {
    let mut rig = Rig::new(HotspotStart::Running);
    rig.key(KeyCode::Char('s'));
    assert!(rig.text(STANDARD).contains("Share Garage Wi-Fi"));
    // Somebody stops it from the CLI meanwhile.
    let out = rig.world.run_nexus(&["hotspot", "stop"]);
    assert!(out.status.success());
    rig.pump_until("the view to close", |a| !a.hotspot().on());
    assert!(!rig.text(STANDARD).contains("Share Garage Wi-Fi"));
}

#[test]
fn pressing_n_while_one_runs_says_stop_it_first() {
    let mut rig = Rig::new(HotspotStart::Running);
    rig.key(KeyCode::Char('n'));
    assert!(rig.toast().contains("already running"), "{}", rig.toast());
}

// ---- the form -----------------------------------------------------------------

#[test]
fn c_opens_a_form_and_starts_with_the_name_and_password_typed() {
    let mut rig = Rig::new(HotspotStart::Capable);
    rig.key(KeyCode::Char('c'));
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Start a Hotspot"), "{frame}");
    insta::assert_snapshot!("hotspot_form_empty", frame);
    rig.type_text("Café Wi-Fi");
    rig.key(KeyCode::Enter); // on to the password
    rig.type_text("hunter2hunter2");
    let typed = rig.text(STANDARD);
    assert!(!typed.contains("hunter2"), "masked: {typed}");
    rig.ctrl('r');
    assert!(rig.text(STANDARD).contains("hunter2hunter2"));
    insta::assert_snapshot!("hotspot_form_typed", rig.text(STANDARD));
    rig.key(KeyCode::Enter);
    rig.pump_until("the Hotspot", |a| a.hotspot().ready());
    assert!(rig.app.hotspot().form.is_none(), "the form closes");
    assert_eq!(
        rig.world.nm.hotspot_psk("Café Wi-Fi").as_deref(),
        Some("hunter2hunter2")
    );
}

#[test]
fn a_password_that_is_too_short_stays_in_the_form_with_the_reason() {
    let mut rig = Rig::new(HotspotStart::Capable);
    rig.key(KeyCode::Char('c'));
    rig.key(KeyCode::Enter);
    rig.type_text("short");
    rig.key(KeyCode::Enter);
    rig.pump_until("the refusal", |a| {
        a.hotspot().form.as_ref().is_some_and(|f| f.error.is_some())
    });
    let frame = rig.text(STANDARD);
    assert!(frame.contains("8 to 63 characters"), "{frame}");
    assert!(!rig.app.hotspot().on());
    insta::assert_snapshot!("hotspot_form_error", frame);
    // esc backs out without starting anything.
    rig.key(KeyCode::Esc);
    assert!(rig.app.hotspot().form.is_none());
}

// ---- the Overview card --------------------------------------------------------

#[test]
fn the_overview_card_follows_the_hotspot() {
    let mut rig = Rig::new(HotspotStart::Capable);
    rig.key(KeyCode::Char('1'));
    let idle = rig.text(STANDARD);
    assert!(idle.contains("Not started"), "{idle}");
    assert!(!idle.contains("Nexus-"), "{idle}");
    rig.key(KeyCode::Char('5'));
    rig.key(KeyCode::Char('n'));
    rig.pump_until("the Hotspot", |a| a.hotspot().ready());
    rig.key(KeyCode::Char('1'));
    let running = rig.text(STANDARD);
    assert!(!running.contains("Not started"), "{running}");
    assert!(running.contains("Nexus-"), "{running}");
}

#[test]
fn the_overview_card_says_when_there_is_no_card_to_host_with() {
    let mut rig = Rig::new(HotspotStart::NoCard);
    rig.key(KeyCode::Char('1'));
    assert!(rig.text(STANDARD).contains("Not available"));
}

// ---- the palette --------------------------------------------------------------

#[test]
fn the_palette_finds_the_hotspot_verbs() {
    let mut rig = Rig::new(HotspotStart::Capable);
    rig.key(KeyCode::Char(':'));
    rig.type_text("hotspot");
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Start Hotspot"), "{frame}");
    rig.key(KeyCode::Esc);
    rig.key(KeyCode::Char(':'));
    rig.type_text("stop hotspot");
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Stop Hotspot"), "{frame}");
    // Stop says why it cannot run, rather than vanishing.
    assert!(frame.contains("No Hotspot is running"), "{frame}");
}

// ---- helpers ------------------------------------------------------------------
