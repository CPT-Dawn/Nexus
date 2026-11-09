//! Seam S2 for the 802.1X sign-in form: keystrokes drive the TUI, which drives
//! a real Daemon over the socket (fake NetworkManager behind it); `insta`
//! frames come out at narrow, standard and wide widths.

use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_testkit::scenarios::wifi_enterprise;
use nexus_testkit::{DeviceId, PATIENCE, World};
use nexus_tui::tui::{self, App, Caps, Client, ColorTier, Glyphs, Msg};
use ratatui::buffer::Buffer;
use ratatui::style::Color;

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
    wlan0: Option<DeviceId>,
    app: App,
    rx: Receiver<Msg>,
    _client: Client,
}

impl Rig {
    /// The busy scenario, with the TUI already on the Wi-Fi tab.
    fn busy() -> Rig {
        Rig::busy_with(truecolor())
    }

    fn busy_with(caps: Caps) -> Rig {
        let world = World::start(NEXUS);
        let wlan0 = wifi_enterprise(&world);
        let mut rig = Rig::start(world, Some(wlan0), caps);
        rig.key(KeyCode::Char('2'));
        // Settled: the scenario's join is visible, so no late list moves a frame.
        rig.pump_until("the Wi-Fi list", |a| {
            a.wifi().len() == 9 && a.wifi().rows()[0].joined()
        });
        rig
    }

    fn start(world: World, wlan0: Option<DeviceId>, caps: Caps) -> Rig {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(caps);
        app.attach(world.socket(), tx.clone());
        let client = Client::spawn(world.socket(), tx);
        app.update(Msg::Resize(STANDARD.0, STANDARD.1));
        Rig {
            world,
            wlan0,
            app,
            rx,
            _client: client,
        }
    }

    fn dev(&self) -> DeviceId {
        self.wlan0.expect("the scenario has a Wi-Fi device")
    }

    fn size(&mut self, (w, h): (u16, u16)) {
        self.app.update(Msg::Resize(w, h));
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

    fn ctrl(&mut self, c: char) {
        self.app.update(Msg::Key(KeyEvent::new(
            KeyCode::Char(c),
            KeyModifiers::CONTROL,
        )));
    }

    fn type_str(&mut self, s: &str) {
        for c in s.chars() {
            self.key(KeyCode::Char(c));
        }
    }

    /// Moves the cursor onto the row named `ssid`.
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

    fn toast(&self) -> String {
        self.app.toast().map(|t| t.text.clone()).unwrap_or_default()
    }

    fn text(&self, (w, h): (u16, u16)) -> String {
        text_of(&tui::render(&self.app, w, h))
    }

    fn styled(&self, (w, h): (u16, u16)) -> String {
        styled_of(&tui::render(&self.app, w, h))
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

fn color_name(c: Color) -> String {
    match c {
        Color::Reset => "-".into(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(i) => format!("idx{i}"),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// The frame's text plus a table of every distinct style and its cell count.
fn styled_of(buf: &Buffer) -> String {
    let mut styles: std::collections::BTreeMap<String, usize> = Default::default();
    for c in buf.content() {
        let mods: Vec<_> = c
            .modifier
            .iter_names()
            .map(|(n, _)| n.to_lowercase())
            .collect();
        let key = format!(
            "fg={} bg={} mods=[{}]",
            color_name(c.fg),
            color_name(c.bg),
            mods.join(",")
        );
        *styles.entry(key).or_default() += 1;
    }
    let table: String = styles
        .iter()
        .map(|(k, n)| format!("{n:5} cells  {k}\n"))
        .collect();
    format!("{}\n\n{table}", text_of(buf))
}

// ---- the form -----------------------------------------------------------------

impl Rig {
    /// Opens the form on eduroam.
    fn open_form(&mut self) {
        self.select("eduroam");
        self.key(KeyCode::Enter);
        assert!(self.app.eap_form().is_some(), "{}", self.text(STANDARD));
    }

    /// Fills the form the way a student would, with the right server name.
    fn fill(&mut self, identity: &str, password: &str) {
        self.type_str(identity); // focus starts on Identity
        self.key(KeyCode::Tab); // anonymous
        self.key(KeyCode::Tab); // password
        self.type_str(password);
        self.key(KeyCode::Tab); // CA certificate
        self.key(KeyCode::Tab); // server name
        self.type_str("radius.uni.example");
    }
}

#[test]
fn enter_on_an_enterprise_network_opens_the_guided_form() {
    let mut rig = Rig::busy();
    rig.open_form();
    let frame = rig.text(STANDARD);
    assert!(
        frame.contains("Sign in to eduroam")
            && frame.contains("PEAP")
            && frame.contains("MSCHAPv2"),
        "{frame}"
    );
    insta::assert_snapshot!("eap_form_standard", frame);
    rig.size(WIDE);
    insta::assert_snapshot!("eap_form_wide", rig.text(WIDE));
    rig.size(NARROW);
    insta::assert_snapshot!("eap_form_narrow", rig.text(NARROW));
}

#[test]
fn leaving_a_field_empty_shows_its_error_there() {
    let mut rig = Rig::busy();
    rig.open_form();
    assert!(!rig.text(STANDARD).contains("Enter your identity"));
    rig.key(KeyCode::Tab); // leave Identity empty
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Enter your identity"), "{frame}");
    // Not yet visited: the password is not nagged about.
    assert!(!frame.contains("Enter your password"), "{frame}");
    insta::assert_snapshot!("eap_form_inline_error", frame);
    rig.size(NARROW);
    insta::assert_snapshot!("eap_form_inline_error_narrow", rig.text(NARROW));
}

#[test]
fn enter_with_a_bad_form_stays_open_and_shows_every_error() {
    let mut rig = Rig::busy();
    rig.open_form();
    rig.key(KeyCode::Enter);
    let frame = rig.text(STANDARD);
    assert!(rig.app.eap_form().is_some());
    for want in ["Enter your identity", "Enter your password", "server name"] {
        assert!(frame.contains(want), "{want}:\n{frame}");
    }
    assert_eq!(
        rig.world.nm.joined_ssid(rig.dev()).as_deref(),
        Some("Home-5G")
    );
}

#[test]
fn tls_asks_for_certificates_not_a_password() {
    let mut rig = Rig::busy();
    rig.open_form();
    rig.key(KeyCode::BackTab);
    rig.key(KeyCode::BackTab); // Method
    rig.key(KeyCode::Right); // TTLS
    rig.key(KeyCode::Right); // TLS
    let frame = rig.text(STANDARD);
    assert!(
        frame.contains("Client cert") && frame.contains("Private key"),
        "{frame}"
    );
    assert!(
        !frame.contains("Password  ") && !frame.contains("Inner method"),
        "{frame}"
    );
    insta::assert_snapshot!("eap_form_tls", frame);
}

#[test]
fn skipping_the_server_check_shows_an_explicit_warning() {
    let mut rig = Rig::busy();
    rig.open_form();
    for _ in 0..5 {
        rig.key(KeyCode::Tab); // Identity .. Skip check
    }
    assert!(!rig.text(STANDARD).contains("pose as this network"));
    rig.key(KeyCode::Char(' '));
    let frame = rig.text(STANDARD);
    assert!(frame.contains("pose as this network"), "{frame}");
    insta::assert_snapshot!("eap_form_skip_check", frame);
    insta::assert_snapshot!("eap_form_skip_check_styles", rig.styled(STANDARD));
}

#[test]
fn a_filled_form_joins_and_the_password_never_shows() {
    let mut rig = Rig::busy();
    rig.open_form();
    rig.fill("you@uni.example", "correct-horse");
    let frame = rig.text(STANDARD);
    assert!(!frame.contains("correct-horse"), "{frame}");
    assert!(frame.contains("•••••••••••••"), "{frame}");
    rig.ctrl('r');
    assert!(rig.text(STANDARD).contains("correct-horse"));
    rig.key(KeyCode::Enter);
    rig.pump_until("the join", |a| {
        a.eap_form().is_none() && a.toast().is_some()
    });
    assert_eq!(rig.toast(), "Joined eduroam.");
    assert_eq!(
        rig.world.nm.joined_ssid(rig.dev()).as_deref(),
        Some("eduroam")
    );
    assert_eq!(
        rig.world
            .nm
            .eap_setting("eduroam", "domain-suffix-match")
            .as_deref(),
        Some("radius.uni.example")
    );
}

#[test]
fn a_refused_sign_in_reopens_the_form_with_everything_typed() {
    let mut rig = Rig::busy();
    rig.open_form();
    rig.fill("you@uni.example", "typo-horse");
    rig.key(KeyCode::Enter);
    rig.pump_until("the form to come back", |a| {
        a.eap_form().is_some_and(|f| f.banner.is_some())
    });
    let frame = rig.text(STANDARD);
    assert!(frame.contains("refused the sign-in"), "{frame}");
    assert!(
        frame.contains("you@uni.example") && frame.contains("radius.uni.example"),
        "{frame}"
    );
    insta::assert_snapshot!("eap_form_refused", frame);
}

#[test]
fn escape_closes_the_form_and_joins_nothing() {
    let mut rig = Rig::busy();
    rig.open_form();
    rig.type_str("you");
    rig.key(KeyCode::Esc);
    assert!(rig.app.eap_form().is_none());
    assert_eq!(
        rig.world.nm.joined_ssid(rig.dev()).as_deref(),
        Some("Home-5G")
    );
}
