//! Seam S2 for the VPN tab: keystrokes drive the TUI, which drives a real
//! Daemon over the socket (fake NetworkManager behind it); `insta` frames
//! come out at narrow, standard and wide widths.

use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_ipc::VpnState;
use nexus_testkit::scenarios::{WG_CONF, vpn_busy};
use nexus_testkit::{PATIENCE, World};
use nexus_tui::tui::{self, App, Caps, Client, ColorTier, Glyphs, Msg};
use ratatui::buffer::Buffer;
use ratatui::style::Color;

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

const NARROW: (u16, u16) = (60, 20);
const STANDARD: (u16, u16) = (100, 28);
const WIDE: (u16, u16) = (140, 32);
/// Tall enough for a config's QR code.
const TALL: (u16, u16) = (120, 46);

const PRIVATE_KEY: &str = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=";

struct Rig {
    world: World,
    app: App,
    rx: Receiver<Msg>,
    _client: Client,
}

impl Rig {
    fn busy() -> Rig {
        let world = World::start(NEXUS);
        vpn_busy(&world);
        Rig::over(world, 3)
    }

    fn empty() -> Rig {
        Rig::over(World::start(NEXUS), 0)
    }

    fn over(world: World, rows: usize) -> Rig {
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
        rig.key(KeyCode::Char('4'));
        rig.pump_until("the VPN list", |a| {
            a.vpn().has_list() && a.vpn().len() == rows
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

    fn key(&mut self, code: KeyCode) {
        self.app
            .update(Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            self.key(KeyCode::Char(c));
        }
    }

    fn select(&mut self, name: &str) {
        self.key(KeyCode::Char('g'));
        for _ in 0..self.app.vpn().len() {
            if self.selected() == name {
                return;
            }
            self.key(KeyCode::Char('j'));
        }
        assert_eq!(self.selected(), name, "no such row");
    }

    fn selected(&self) -> String {
        self.app
            .vpn()
            .selected()
            .map(|v| v.name.clone())
            .unwrap_or_default()
    }

    fn state_of(&self, name: &str) -> VpnState {
        self.app
            .vpn()
            .rows()
            .iter()
            .find(|v| v.name == name)
            .unwrap_or_else(|| panic!("no VPN {name}"))
            .state
    }

    fn size(&mut self, (w, h): (u16, u16)) {
        self.app.update(Msg::Resize(w, h));
    }

    fn text(&self, (w, h): (u16, u16)) -> String {
        text_of(&tui::render(&self.app, w, h))
    }

    fn footer(&self, size: (u16, u16)) -> String {
        self.text(size).lines().last().unwrap().trim().to_string()
    }
}

/// Whether the standard-size frame shows `what`.
fn shows(app: &App, what: &str) -> bool {
    text_of(&tui::render(app, STANDARD.0, STANDARD.1)).contains(what)
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
fn the_list_shows_every_vpn_and_its_state() {
    let mut rig = Rig::busy();
    assert_eq!(rig.selected(), "Home Lab");
    assert_eq!(rig.state_of("Work"), VpnState::Up);
    insta::assert_snapshot!("vpn_standard", rig.text(STANDARD));
    rig.size(NARROW);
    insta::assert_snapshot!("vpn_narrow", rig.text(NARROW));
    rig.size(WIDE);
    insta::assert_snapshot!("vpn_wide", rig.text(WIDE));
}

#[test]
fn an_empty_list_says_how_to_get_a_vpn() {
    let rig = Rig::empty();
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Press i to import"), "{frame}");
    insta::assert_snapshot!("vpn_empty_standard", frame);
}

#[test]
fn enter_brings_a_vpn_up_and_down_with_a_toast() {
    let mut rig = Rig::busy();
    assert_eq!(
        rig.footer(STANDARD),
        "j/k move  enter up  i import  c create  p key  s share  h/l tab  : palette  ? help  q quit"
    );
    rig.key(KeyCode::Enter);
    rig.pump_until("Home Lab up", |a| {
        a.vpn()
            .rows()
            .iter()
            .any(|v| v.name == "Home Lab" && v.state == VpnState::Up)
            && a.toast().is_some()
    });
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Home Lab is up."), "{frame}");
    insta::assert_snapshot!("vpn_toast_up", frame);
    assert!(rig.footer(STANDARD).contains("enter down"));

    rig.key(KeyCode::Enter);
    rig.pump_until("Home Lab down", |a| {
        a.vpn()
            .rows()
            .iter()
            .any(|v| v.name == "Home Lab" && v.state == VpnState::Down)
            && a.toast().is_some_and(|t| t.text.contains("is down"))
    });
}

#[test]
fn a_failed_start_says_why() {
    let mut rig = Rig::busy();
    let uuid = rig.world.nm.vpn_uuid("Home Lab").unwrap();
    rig.world.nm.script_vpn_failure(&uuid, true);
    rig.key(KeyCode::Enter);
    rig.pump_until("the failure", |a| a.toast().is_some_and(|t| t.bad));
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Home Lab did not come up"), "{frame}");
}

#[test]
fn p_shows_the_public_key_of_a_wireguard_vpn_only() {
    let mut rig = Rig::busy();
    rig.select("Work");
    rig.key(KeyCode::Char('p'));
    rig.pump_until("the key", |a| a.vpn().layer.is_some());
    let frame = rig.text(STANDARD);
    let uuid = rig.world.nm.vpn_uuid("Work").unwrap();
    let key = rig
        .world
        .wire()
        .call(serde_json::json!({"op": "vpn_key", "vpn": uuid}));
    let key = key["ok"]["public_key"].as_str().unwrap().to_owned();
    assert!(frame.contains(&key), "{frame}");
    assert!(!frame.contains(PRIVATE_KEY));
    insta::assert_snapshot!("vpn_key_standard", frame);
    assert_eq!(rig.footer(STANDARD), "esc close");
    rig.key(KeyCode::Esc);
    assert!(rig.app.vpn().layer.is_none());

    rig.select("Office OpenVPN");
    rig.key(KeyCode::Char('p'));
    assert!(rig.app.vpn().layer.is_none(), "a plugin VPN has no keys");
    assert!(!rig.footer(STANDARD).contains("p key"));
}

#[test]
fn create_makes_a_tunnel_and_shows_its_public_key() {
    let mut rig = Rig::empty();
    rig.key(KeyCode::Char('c'));
    insta::assert_snapshot!("vpn_create_form", rig.text(STANDARD));
    // An empty form refuses, in words, and stays open.
    for _ in 0..5 {
        rig.key(KeyCode::Enter);
    }
    assert!(rig.text(STANDARD).contains("Give the VPN a name."));
    rig.key(KeyCode::Tab); // from the last line it wraps to the first
    rig.type_text("Cabin");
    rig.key(KeyCode::Enter);
    rig.type_text("10.7.0.2/32");
    for _ in 0..3 {
        rig.key(KeyCode::Enter);
    }
    rig.key(KeyCode::Enter); // last line: send
    rig.pump_until("the key view", |a| shows(a, "Give this public key"));
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Saved. Give this public key"), "{frame}");
    let uuid = rig
        .world
        .nm
        .vpn_uuid("Cabin")
        .expect("the tunnel was saved");
    let private = rig.world.nm.vpn_private_key(&uuid).unwrap();
    assert!(!frame.contains(&private), "the private key never shows");
    rig.key(KeyCode::Esc);
    rig.pump_until("the new row", |a| a.vpn().len() == 1);
    assert_eq!(rig.selected(), "Cabin");
}

#[test]
fn a_bad_config_keeps_the_form_open_with_the_reason() {
    let mut rig = Rig::empty();
    let path = rig.world.dir().join("broken.conf");
    std::fs::write(&path, "[Interface]\nAddress = nope\n").unwrap();
    rig.key(KeyCode::Char('i'));
    rig.type_text(path.to_str().unwrap());
    rig.key(KeyCode::Enter);
    rig.key(KeyCode::Enter);
    rig.pump_until("the refusal", |a| {
        matches!(&a.vpn().layer, Some(l) if format!("{l:?}").contains("not a WireGuard config"))
    });
    let frame = rig.text(STANDARD);
    assert!(frame.contains("line 2"), "{frame}");
    // The temp path differs per run: elide that one line.
    let stable: Vec<&str> = frame
        .lines()
        .map(|l| {
            if l.contains("broken.conf") {
                "<the File line: a temp path>"
            } else {
                l
            }
        })
        .collect();
    insta::assert_snapshot!("vpn_import_refused", stable.join("\n"));
    assert_eq!(rig.world.nm.profile_count(), 0);
}

#[test]
fn import_reads_the_file_and_names_the_vpn_after_it() {
    let mut rig = Rig::empty();
    let path = rig.world.dir().join("travel.conf");
    std::fs::write(&path, WG_CONF).unwrap();
    rig.key(KeyCode::Char('i'));
    rig.type_text(path.to_str().unwrap());
    rig.key(KeyCode::Enter);
    rig.key(KeyCode::Enter);
    rig.pump_until("the key view", |a| shows(a, "Give this public key"));
    assert!(rig.world.nm.vpn_uuid("travel").is_some());
    rig.key(KeyCode::Esc);
    rig.pump_until("the new row", |a| a.vpn().len() == 1);
    assert_eq!(rig.selected(), "travel");
}

#[test]
fn the_config_qr_is_asked_for_and_wiped_on_close() {
    let mut rig = Rig::busy();
    rig.select("Work");
    rig.key(KeyCode::Char('s'));
    let ask = rig.text(STANDARD);
    assert!(ask.contains("PRIVATE key"), "{ask}");
    assert_eq!(
        rig.world.nm.secret_reads(),
        0,
        "nothing is read before enter"
    );
    insta::assert_snapshot!("vpn_share_ask", ask);
    assert_eq!(rig.footer(STANDARD), "enter reveal  esc cancel");

    rig.key(KeyCode::Enter);
    rig.pump_until("the details", |a| a.hints(false) == vec![("esc", "close")]);
    assert_eq!(rig.world.nm.secret_reads(), 1);

    // Too small for the code: says how big it must be, still no private key.
    let small = rig.text(STANDARD);
    assert!(small.contains("needs a window of at least"), "{small}");
    assert!(!small.contains(PRIVATE_KEY));
    insta::assert_snapshot!("vpn_share_too_small", small);
    rig.size(NARROW);
    insta::assert_snapshot!("vpn_share_narrow", rig.text(NARROW));

    // A tall window draws it, black on white.
    rig.size(TALL);
    let frame = rig.text(TALL);
    assert!(
        frame.contains('\u{2588}') || frame.contains('\u{2580}'),
        "{frame}"
    );
    assert!(!frame.contains(PRIVATE_KEY));
    insta::assert_snapshot!("vpn_share_qr", frame);
    let buf = tui::render(&rig.app, TALL.0, TALL.1);
    let painted = buf
        .content()
        .iter()
        .filter(|c| matches!(c.symbol(), "\u{2580}" | "\u{2584}" | "\u{2588}"))
        .filter(|c| c.fg == Color::Black && c.bg == Color::White)
        .count();
    assert!(painted > 200, "only {painted} QR cells are black on white");

    rig.key(KeyCode::Esc);
    assert!(rig.app.vpn().layer.is_none());
    rig.key(KeyCode::Char('s'));
    assert!(
        rig.text(TALL).contains("PRIVATE key"),
        "asks again from scratch"
    );
}

#[test]
fn denied_authorization_says_so_and_enter_tries_again() {
    let mut rig = Rig::busy();
    rig.world.nm.deny_secret_reads(true);
    rig.select("Work");
    rig.key(KeyCode::Char('s'));
    rig.key(KeyCode::Enter);
    rig.pump_until("the refusal", |a| {
        a.hints(false).iter().any(|(_, what)| *what == "try again")
    });
    let frame = rig.text(STANDARD);
    assert!(frame.contains("did not authorize"), "{frame}");
    insta::assert_snapshot!("vpn_share_denied", frame);
    rig.world.nm.deny_secret_reads(false);
    rig.key(KeyCode::Enter);
    rig.pump_until("the details", |a| a.hints(false) == vec![("esc", "close")]);
}

#[test]
fn the_list_follows_changes_made_elsewhere() {
    let mut rig = Rig::busy();
    let uuid = rig.world.nm.vpn_uuid("Work").unwrap();
    rig.world.nm.drop_vpn(&uuid);
    rig.pump_until("Work down", |a| {
        a.vpn()
            .rows()
            .iter()
            .any(|v| v.name == "Work" && v.state == VpnState::Down)
    });
    assert!(!rig.text(STANDARD).contains("up\n"));
}

#[test]
fn a_tunnel_toggles_from_the_narrow_list_too() {
    let mut rig = Rig::busy();
    rig.size(NARROW);
    rig.key(KeyCode::Enter);
    rig.pump_until("Home Lab up", |a| {
        a.vpn()
            .rows()
            .iter()
            .any(|v| v.name == "Home Lab" && v.state == VpnState::Up)
    });
}

// ---- plugin VPNs: prompts, import, the missing plugin ----

/// A world with one OpenVPN Profile that asks for `secrets` when it starts.
fn plugin_rig(secrets: &[&str]) -> Rig {
    let world = World::start(NEXUS);
    world.install_vpn_plugin("openvpn");
    world
        .nm
        .add_plugin_vpn_of("uuid-ovpn", "Office", Some("openvpn"));
    world.nm.script_vpn_asks("uuid-ovpn", secrets);
    Rig::over(world, 1)
}

#[test]
fn a_vpn_that_wants_a_password_asks_in_the_tui() {
    let mut rig = plugin_rig(&["password"]);
    rig.key(KeyCode::Enter);
    rig.pump_until("the prompt", |a| a.field().is_some());
    rig.type_text("hunter2");
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Password for Office"), "{frame}");
    assert!(!frame.contains("hunter2"), "the password is masked: {frame}");
    insta::assert_snapshot!("vpn_prompt_standard", frame);
    rig.size(NARROW);
    insta::assert_snapshot!("vpn_prompt_narrow", rig.text(NARROW));
    rig.size(WIDE);
    insta::assert_snapshot!("vpn_prompt_wide", rig.text(WIDE));

    rig.key(KeyCode::Enter);
    rig.pump_until("Office up", |a| {
        a.toast().is_some_and(|t| t.text.contains("Office is up"))
    });
    assert_eq!(
        rig.world.nm.vpn_answers("uuid-ovpn"),
        [("password".to_owned(), "hunter2".to_owned())]
    );
}

#[test]
fn a_user_name_shows_as_typed_and_a_code_is_asked_after_it() {
    let mut rig = plugin_rig(&["username", "otp"]);
    rig.key(KeyCode::Enter);
    rig.pump_until("the user name prompt", |a| a.field().is_some());
    rig.type_text("ada");
    let frame = rig.text(STANDARD);
    assert!(frame.contains("User name for Office"), "{frame}");
    assert!(frame.contains("User name ada"), "{frame}");
    insta::assert_snapshot!("vpn_prompt_username", frame);
    rig.key(KeyCode::Enter);
    rig.pump_until("the code prompt", |a| {
        a.field()
            .is_some_and(|f| f.prompt.field == nexus_ipc::PromptField::Otp)
    });
    rig.type_text("424242");
    let frame = rig.text(STANDARD);
    assert!(frame.contains("One-time code for Office"), "{frame}");
    assert!(!frame.contains("424242"), "{frame}");
    insta::assert_snapshot!("vpn_prompt_code", frame);
    rig.key(KeyCode::Enter);
    rig.pump_until("Office up", |a| {
        a.toast().is_some_and(|t| t.text.contains("Office is up"))
    });
}

#[test]
fn escape_declines_the_prompt_and_the_vpn_stays_down() {
    let mut rig = plugin_rig(&["password"]);
    rig.key(KeyCode::Enter);
    rig.pump_until("the prompt", |a| a.field().is_some());
    rig.key(KeyCode::Esc);
    rig.pump_until("the failure", |a| a.toast().is_some_and(|t| t.bad));
    assert_eq!(rig.state_of("Office"), VpnState::Down);
}

#[test]
fn importing_an_ovpn_file_adds_a_plugin_row() {
    let world = World::start(NEXUS);
    world.install_vpn_plugin("openvpn");
    world.install_fake_nmcli();
    let mut rig = Rig::over(world, 0);
    let path = rig.world.dir().join("Branch.ovpn");
    std::fs::write(&path, "client\nremote vpn.example.net 1194\n").unwrap();
    rig.key(KeyCode::Char('i'));
    rig.type_text(path.to_str().unwrap());
    rig.key(KeyCode::Enter);
    rig.key(KeyCode::Enter);
    rig.pump_until("the new row", |a| {
        a.vpn().len() == 1 && a.toast().is_some_and(|t| t.text == "Saved Branch.")
    });
    assert_eq!(rig.selected(), "Branch");
    insta::assert_snapshot!("vpn_imported_ovpn", rig.text(STANDARD));
}

#[test]
fn a_missing_plugin_says_which_package_to_install() {
    let world = World::start(NEXUS);
    world
        .nm
        .add_plugin_vpn_of("uuid-ovpn", "Office", Some("openvpn"));
    let mut rig = Rig::over(world, 1);
    rig.key(KeyCode::Enter);
    rig.pump_until("the refusal", |a| a.toast().is_some_and(|t| t.bad));
    let frame = rig.text(STANDARD);
    assert!(frame.contains("networkmanager-openvpn"), "{frame}");
    insta::assert_snapshot!("vpn_missing_plugin", frame);
}
