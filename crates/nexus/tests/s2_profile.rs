//! Seam S2 for the Profile settings editor: keystrokes drive the TUI, which
//! drives a real Daemon over the socket (fake NetworkManager behind it);
//! `insta` frames come out at narrow, standard and wide widths.

use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_testkit::scenarios::profile_busy;
use nexus_testkit::{PATIENCE, World};
use nexus_tui::tui::{self, App, Caps, Client, ColorTier, Glyphs, Msg, Phase};
use ratatui::buffer::Buffer;
use serde_json::{Value, json};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

const NARROW: (u16, u16) = (60, 20);
const STANDARD: (u16, u16) = (100, 28);
const WIDE: (u16, u16) = (140, 32);
const SIZES: [(&str, (u16, u16)); 3] = [("narrow", NARROW), ("standard", STANDARD), ("wide", WIDE)];

struct Rig {
    world: World,
    app: App,
    rx: Receiver<Msg>,
    _client: Client,
}

impl Rig {
    /// The profile scenario, the TUI on the Wi-Fi tab.
    fn start() -> Rig {
        let world = World::start(NEXUS);
        profile_busy(&world);
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
        // Wait for the joined list, then for the Daemon to go quiet, and
        // check again.
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

    fn type_str(&mut self, s: &str) {
        for c in s.chars() {
            self.key(KeyCode::Char(c));
        }
    }

    fn select(&mut self, ssid: &str) {
        self.key(KeyCode::Char('g'));
        for _ in 0..self.app.wifi().len() {
            if self.app.wifi().selected().map(|e| e.name()) == Some(ssid) {
                return;
            }
            self.key(KeyCode::Char('j'));
        }
        panic!("no such row {ssid}");
    }

    /// `e` on `ssid`, waiting for the form to be filled in.
    fn open(&mut self, ssid: &str) {
        self.select(ssid);
        self.key(KeyCode::Char('e'));
        self.pump_until("the form", |a| {
            a.editor().is_some_and(|e| e.phase == Phase::Editing)
        });
    }

    fn text(&self, (w, h): (u16, u16)) -> String {
        text_of(&tui::render(&self.app, w, h))
    }

    fn snap_all(&self, name: &str) {
        for (label, size) in SIZES {
            insta::assert_snapshot!(format!("{name}_{label}"), self.text(size));
        }
    }

    fn profile(&self, ssid: &str) -> Value {
        self.world.wire().call(json!({
            "op": "profile_get",
            "network": nexus_ipc::network_id(ssid.as_bytes()),
        }))["ok"]
            .clone()
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
fn e_opens_the_saved_profile_with_its_own_settings() {
    let mut rig = Rig::start();
    rig.open("Parents House");
    let form = rig.text(STANDARD);
    assert!(form.contains("192.168.7.20/24"), "{form}");
    assert!(form.contains("9.9.9.9, 149.112.112.112"), "{form}");
    rig.snap_all("profile_form");
}

#[test]
fn a_network_that_is_not_saved_has_no_settings_to_edit() {
    let mut rig = Rig::start();
    rig.select("Cafe Guest");
    rig.key(KeyCode::Char('e'));
    assert!(rig.app.editor().is_none());
}

#[test]
fn a_typo_is_flagged_inline_and_enter_does_not_send_it() {
    let mut rig = Rig::start();
    rig.open("Parents House");
    // IPv4 is static; move to the address and break it.
    rig.key(KeyCode::Tab);
    rig.type_str("0"); // 192.168.7.20/240
    let wrong = rig.text(STANDARD);
    assert!(
        wrong.contains("The prefix of 192.168.7.20/240 must be 1 to 32"),
        "{wrong}"
    );
    rig.snap_all("profile_form_error");
    let version = rig.profile("Parents House")["version"].clone();
    rig.key(KeyCode::Enter);
    assert_eq!(
        rig.app.editor().map(|e| e.phase.clone()),
        Some(Phase::Editing)
    );
    assert_eq!(rig.profile("Parents House")["version"], version);
}

#[test]
fn a_gateway_off_the_subnet_is_flagged_by_name() {
    let mut rig = Rig::start();
    rig.open("Parents House");
    rig.key(KeyCode::Tab);
    rig.key(KeyCode::Tab); // the gateway row
    rig.app.update(Msg::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    rig.type_str("10.0.0.1");
    let text = rig.text(STANDARD);
    assert!(text.contains("not on the same network"), "{text}");
}

#[test]
fn applying_an_idle_profile_saves_at_once() {
    let mut rig = Rig::start();
    rig.open("Parents House");
    // Move to Metered (IPv4, address, gateway, IPv6, DNS, MAC, Metered) and flip it.
    for _ in 0..6 {
        rig.key(KeyCode::Tab);
    }
    rig.key(KeyCode::Char(' '));
    rig.key(KeyCode::Enter);
    rig.pump_until("the form to close", |a| a.editor().is_none());
    assert_eq!(
        rig.app.toast().map(|t| t.text.as_str()),
        Some("Saved the settings of Parents House.")
    );
    assert_eq!(rig.profile("Parents House")["settings"]["metered"], false);
    assert_eq!(rig.world.nm.checkpoints_open(), 0);
}

#[test]
fn editing_the_profile_in_use_counts_down_and_y_keeps_it() {
    let mut rig = Rig::start();
    rig.open("Home-5G");
    assert!(rig.text(STANDARD).contains("In use: you will be asked"));
    rig.snap_all("profile_form_active");
    // DNS is the fifth row of an automatic profile.
    for _ in 0..2 {
        rig.key(KeyCode::Tab);
    }
    rig.type_str("1.1.1.1");
    rig.key(KeyCode::Enter);
    rig.pump_until("the countdown", |a| {
        a.editor().is_some_and(|e| e.is_pending())
    });
    let counting = rig.text(STANDARD);
    assert!(counting.contains("Keep these settings?"), "{counting}");
    assert!(counting.contains("Reverting in 15s"), "{counting}");
    rig.snap_all("profile_countdown");
    rig.key(KeyCode::Char('y'));
    rig.pump_until("the form to close", |a| a.editor().is_none());
    assert_eq!(
        rig.app.toast().map(|t| t.text.as_str()),
        Some("Keeping the new settings of Home-5G.")
    );
    assert_eq!(
        rig.profile("Home-5G")["settings"]["dns"],
        json!(["1.1.1.1"])
    );
    assert_eq!(rig.world.nm.checkpoint_rollbacks(), 0);
}

#[test]
fn n_reverts_the_change_without_waiting() {
    let mut rig = Rig::start();
    rig.open("Home-5G");
    for _ in 0..2 {
        rig.key(KeyCode::Tab);
    }
    rig.type_str("1.1.1.1");
    rig.key(KeyCode::Enter);
    rig.pump_until("the countdown", |a| {
        a.editor().is_some_and(|e| e.is_pending())
    });
    rig.key(KeyCode::Char('n'));
    rig.pump_until("the form to close", |a| a.editor().is_none());
    let toast = rig
        .app
        .toast()
        .expect("a toast says it was reverted")
        .clone();
    assert!(toast.bad && toast.text.starts_with("Reverted"), "{toast:?}");
    assert_eq!(rig.profile("Home-5G")["settings"]["dns"], json!([]));
    assert_eq!(rig.world.nm.checkpoint_rollbacks(), 1);
}

#[test]
fn a_change_made_elsewhere_meanwhile_is_reported_and_nothing_is_lost() {
    let mut rig = Rig::start();
    rig.open("Parents House");
    rig.world.nm.edit_profile_externally("uuid-parents");
    rig.key(KeyCode::Enter);
    rig.pump_until("the complaint", |a| {
        a.editor()
            .is_some_and(|e| e.phase == Phase::Editing && e.message.is_some())
    });
    let text = rig.text(STANDARD);
    assert!(text.contains("changed elsewhere"), "{text}");
    rig.snap_all("profile_form_conflict");
}

#[test]
fn keeping_a_profile_in_the_keyring_without_one_is_explained_in_the_form() {
    let mut rig = Rig::start();
    rig.open("Parents House");
    // Keyring is the last row (after Priority); this profile is static.
    for _ in 0..8 {
        rig.key(KeyCode::Tab);
    }
    rig.key(KeyCode::Char(' '));
    assert!(rig.text(STANDARD).contains("yes, in your keyring"));
    rig.key(KeyCode::Enter);
    rig.pump_until("the complaint", |a| {
        a.editor()
            .is_some_and(|e| e.phase == Phase::Editing && e.message.is_some())
    });
    let text = rig.text(STANDARD);
    assert!(text.contains("keyring"), "{text}");
    assert_eq!(rig.profile("Parents House")["settings"]["keyring"], false);
    rig.snap_all("profile_form_keyring_unavailable");
}

#[test]
fn esc_closes_the_form_and_changes_nothing() {
    let mut rig = Rig::start();
    rig.open("Parents House");
    let version = rig.profile("Parents House")["version"].clone();
    rig.key(KeyCode::Tab);
    rig.type_str("9");
    rig.key(KeyCode::Esc);
    assert!(rig.app.editor().is_none());
    assert_eq!(rig.profile("Parents House")["version"], version);
}

#[test]
fn the_footer_offers_e_for_saved_rows_and_the_help_lists_it() {
    let mut rig = Rig::start();
    rig.select("Home-5G");
    let footer = rig.text(STANDARD).lines().last().unwrap().to_string();
    assert!(footer.contains("e settings"), "{footer}");
    rig.key(KeyCode::Char('?'));
    assert!(rig.text(STANDARD).contains("Edit the Profile"));
}
