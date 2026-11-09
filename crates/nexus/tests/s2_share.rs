//! Seam S2 for sharing a Network as a QR code: keystrokes drive the TUI, which
//! drives a real Daemon over the socket (fake NetworkManager behind it);
//! `insta` frames come out at narrow, standard and wide widths.

use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_testkit::scenarios::wifi_busy;
use nexus_testkit::{PATIENCE, World};
use nexus_tui::tui::{self, App, Caps, Client, ColorTier, Glyphs, Msg};
use ratatui::buffer::Buffer;
use ratatui::style::Color;

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

const NARROW: (u16, u16) = (60, 20);
const STANDARD: (u16, u16) = (100, 28);
const WIDE: (u16, u16) = (140, 32);

const PARENTS_PASSWORD: &str = "parents-wifi-2024";

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
        rig.key(KeyCode::Char('2'));
        // Settled: the join's signals can arrive as several lists, and a frame
        // snapshotted between them shows Home-5G without its joined marker.
        loop {
            rig.pump_until("the Wi-Fi list", |a| {
                a.wifi().len() == 8 && a.wifi().rows()[0].joined()
            });
            while let Ok(msg) = rig.rx.recv_timeout(std::time::Duration::from_millis(400)) {
                rig.app.update(msg);
            }
            if rig.app.wifi().rows()[0].joined() {
                break;
            }
        }
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

    fn size(&mut self, (w, h): (u16, u16)) {
        self.app.update(Msg::Resize(w, h));
    }

    /// Reveals the share view of `ssid` and waits for the QR code.
    fn reveal(&mut self, ssid: &str) {
        self.select(ssid);
        self.key(KeyCode::Char('s'));
        self.key(KeyCode::Enter);
        self.pump_until("the revealed details", |a| {
            a.hints(false).contains(&("esc", "close"))
                && !a.hints(false).iter().any(|(k, _)| *k == "enter")
        });
    }

    fn text(&self, (w, h): (u16, u16)) -> String {
        text_of(&tui::render(&self.app, w, h))
    }

    fn footer(&self, size: (u16, u16)) -> String {
        self.text(size).lines().last().unwrap().trim().to_string()
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
fn s_asks_first_and_reads_no_secret_until_the_reveal_key() {
    let mut rig = Rig::busy();
    rig.select("Parents House");
    rig.key(KeyCode::Char('s'));
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Share Parents House"), "{frame}");
    assert!(!frame.contains(PARENTS_PASSWORD));
    assert_eq!(rig.world.nm.secret_reads(), 0);
    assert_eq!(rig.footer(STANDARD), "enter reveal  esc cancel");
    insta::assert_snapshot!("share_ask_standard", frame);

    rig.key(KeyCode::Esc);
    assert!(!rig.text(STANDARD).contains("Share Parents House"));
    assert_eq!(rig.world.nm.secret_reads(), 0, "cancelling reads nothing");
}

#[test]
fn the_qr_code_is_drawn_in_text_cells_with_the_password_masked() {
    let mut rig = Rig::busy();
    rig.reveal("Parents House");
    assert_eq!(rig.world.nm.secret_reads(), 1);
    let frame = rig.text(STANDARD);
    assert!(
        frame.contains('\u{2588}') || frame.contains('\u{2580}'),
        "{frame}"
    );
    assert!(frame.contains("Password  \u{2022}"), "{frame}");
    assert!(
        !frame.contains(PARENTS_PASSWORD),
        "masked until asked:\n{frame}"
    );
    insta::assert_snapshot!("share_qr_standard", frame);

    rig.size(WIDE);
    insta::assert_snapshot!("share_qr_wide", rig.text(WIDE));
}

#[test]
fn p_shows_the_password_and_esc_wipes_the_view() {
    let mut rig = Rig::busy();
    rig.reveal("Parents House");
    assert_eq!(rig.footer(STANDARD), "p show password  esc close");
    rig.key(KeyCode::Char('p'));
    let frame = rig.text(STANDARD);
    assert!(
        frame.contains(&format!("Password  {PARENTS_PASSWORD}")),
        "{frame}"
    );
    rig.key(KeyCode::Char('p'));
    assert!(!rig.text(STANDARD).contains(PARENTS_PASSWORD));

    rig.key(KeyCode::Esc);
    let after = rig.text(STANDARD);
    assert!(
        !after.contains("Password") && !after.contains(PARENTS_PASSWORD),
        "{after}"
    );
    // Asking again starts from the question, not from the old details.
    rig.key(KeyCode::Char('s'));
    assert!(rig.text(STANDARD).contains("Show the password"));
}

#[test]
fn the_code_is_black_on_white_whatever_the_theme() {
    let mut rig = Rig::busy();
    rig.reveal("Parents House");
    let buf = tui::render(&rig.app, STANDARD.0, STANDARD.1);
    let painted = buf
        .content()
        .iter()
        .filter(|c| {
            c.symbol() == "\u{2580}" || c.symbol() == "\u{2584}" || c.symbol() == "\u{2588}"
        })
        .filter(|c| c.fg == Color::Black && c.bg == Color::White)
        .count();
    assert!(painted > 100, "only {painted} QR cells are black on white");
}

#[test]
fn a_window_too_small_for_the_code_says_how_big_it_must_be() {
    let mut rig = Rig::busy();
    rig.reveal("Parents House");
    rig.size(NARROW);
    let frame = rig.text(NARROW);
    assert!(frame.contains("needs a window of at least"), "{frame}");
    assert!(!frame.contains(PARENTS_PASSWORD));
    insta::assert_snapshot!("share_qr_narrow", frame);
}

#[test]
fn an_open_network_has_a_code_and_no_password() {
    let mut rig = Rig::busy();
    rig.reveal("Hotel Lobby");
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Open network: no password."), "{frame}");
    assert_eq!(rig.footer(STANDARD), "esc close");
    assert_eq!(rig.world.nm.secret_reads(), 0);
}

#[test]
fn denied_authorization_says_so_and_enter_tries_again() {
    let mut rig = Rig::busy();
    rig.world.nm.deny_secret_reads(true);
    rig.select("Parents House");
    rig.key(KeyCode::Char('s'));
    rig.key(KeyCode::Enter);
    rig.pump_until("the refusal", |a| {
        a.hints(false).iter().any(|(_, what)| *what == "try again")
    });
    let frame = rig.text(STANDARD);
    assert!(frame.contains("did not authorize"), "{frame}");
    assert!(!frame.contains(PARENTS_PASSWORD));
    insta::assert_snapshot!("share_denied_standard", frame);

    rig.world.nm.deny_secret_reads(false);
    rig.key(KeyCode::Enter);
    rig.pump_until("the details", |a| {
        a.hints(false).iter().any(|(k, _)| *k == "p")
    });
}

#[test]
fn s_is_only_offered_for_saved_networks() {
    let mut rig = Rig::busy();
    rig.select("Cafe Guest");
    rig.key(KeyCode::Char('s'));
    assert!(!rig.text(STANDARD).contains("Share Cafe Guest"));
    assert!(!rig.footer(STANDARD).contains("share"));
    rig.select("Parents House");
    assert!(
        rig.footer(STANDARD).contains("s share"),
        "{}",
        rig.footer(STANDARD)
    );
}
