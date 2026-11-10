//! Seam S2 for the Settings tab and the themes: keystrokes drive the TUI,
//! which edits `config.toml` through a real Daemon; `insta` frames come out
//! at narrow, standard and wide widths, for every theme. The world is the
//! testkit's `settings_world`, so `demo settings` shows the same.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_testkit::scenarios::settings_world;
use nexus_testkit::{PATIENCE, World};
use nexus_tui::tui::{self, App, Caps, Client, ColorTier, Glyphs, Movement, Msg};
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use serde_json::json;

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

const NARROW: (u16, u16) = (60, 20);
const STANDARD: (u16, u16) = (100, 28);
const WIDE: (u16, u16) = (140, 32);

const THEMES: [&str; 7] = [
    "midnight-bloom",
    "catppuccin",
    "gruvbox",
    "nord",
    "tokyo-night",
    "rose-pine",
    "high-contrast",
];

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
    /// The Settings scenario with the TUI on the Settings tab and the
    /// Daemon's config received.
    fn new(bad_theme: bool) -> Rig {
        Rig::with(settings_world(NEXUS, bad_theme), truecolor())
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
        rig.pump_until("the config", |a| a.settings().has_data());
        rig.key(KeyCode::Char('6'));
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

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            self.key(KeyCode::Char(c));
        }
    }

    /// Moves the cursor to the row labelled `label`.
    fn go_to(&mut self, label: &str) {
        let rows = tui::settings_labels();
        let want = rows
            .iter()
            .position(|l| *l == label)
            .unwrap_or_else(|| panic!("no row `{label}`"));
        self.key(KeyCode::Char('g'));
        for _ in 0..want {
            self.key(KeyCode::Char('j'));
        }
    }

    fn text(&self, (w, h): (u16, u16)) -> String {
        text_of(&tui::render(&self.app, w, h))
    }

    fn theme(&self) -> &'static str {
        self.app.look().theme.name
    }

    fn config_text(&self) -> String {
        std::fs::read_to_string(self.world.config_path()).expect("config file")
    }

    fn set(&self, key: &str, value: serde_json::Value) {
        let resp = self
            .world
            .wire()
            .call(json!({"op": "config_set", "key": key, "value": value}));
        assert!(resp.get("ok").is_some(), "config_set {key}: {resp}");
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
    format!("{c:?}")
}

/// The frame's text plus a table of every distinct style it uses.
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

// ---- the tab, at every width ------------------------------------------------

#[test]
fn settings_standard() {
    let rig = Rig::new(false);
    let frame = rig.text(STANDARD);
    for want in [
        "Theme",
        "Nerd Font icons",
        "Reduced motion",
        "Notify: joined",
        "Notify: lost",
        "Notify: bad password",
        "Notify: prompt",
        "Notify: login",
        "Notify: VPN drop",
    ] {
        assert!(frame.contains(want), "{want}:\n{frame}");
    }
    insta::assert_snapshot!("settings_standard", frame);
}

#[test]
fn settings_wide() {
    let rig = Rig::new(false);
    insta::assert_snapshot!("settings_wide", rig.text(WIDE));
}

#[test]
fn settings_narrow_list_and_sheet() {
    let mut rig = Rig::new(false);
    rig.app.update(Msg::Resize(NARROW.0, NARROW.1));
    insta::assert_snapshot!("settings_narrow", rig.text(NARROW));
    rig.key(KeyCode::Char('d'));
    insta::assert_snapshot!("settings_narrow_sheet", rig.text(NARROW));
}

#[test]
fn enter_on_a_narrow_list_changes_the_row_instead_of_opening_the_sheet() {
    let mut rig = Rig::new(false);
    rig.app.update(Msg::Resize(NARROW.0, NARROW.1));
    rig.key(KeyCode::Enter);
    rig.pump_until("the next theme", |a| a.look().theme.name == "catppuccin");
    assert_eq!(rig.app.layer(), tui::Layer::Root);
}

// ---- every theme, at every width --------------------------------------------

#[test]
fn every_theme_renders_at_every_width() {
    let mut rig = Rig::new(false);
    for slug in THEMES {
        rig.set("theme", json!(slug));
        rig.pump_until(slug, |a| a.look().theme.name == slug);
        rig.settle();
        for (label, size) in [("narrow", NARROW), ("standard", STANDARD), ("wide", WIDE)] {
            rig.app.update(Msg::Resize(size.0, size.1));
            let frame = styled_of(&tui::render(&rig.app, size.0, size.1));
            insta::assert_snapshot!(format!("theme_{slug}_{label}"), frame);
        }
    }
}

#[test]
fn every_theme_works_with_the_gradient_border() {
    let mut rig = Rig::new(false);
    for slug in THEMES {
        rig.set("theme", json!(slug));
        rig.pump_until(slug, |a| a.look().theme.name == slug);
        let accent = rig.app.look().theme.accent.rgb;
        let buf = tui::render(&rig.app, STANDARD.0, STANDARD.1);
        let shades: std::collections::BTreeSet<_> = buf
            .content()
            .iter()
            .filter(|c| "╭╮╰╯─│".contains(c.symbol()))
            .filter_map(|c| match c.fg {
                Color::Rgb(r, g, b) => Some((r, g, b)),
                _ => None,
            })
            .collect();
        // The focused pane's border is a run of hues around the accent, and
        // the unfocused pane keeps the theme's quiet border colour.
        assert!(shades.len() > 3, "{slug}: {shades:?}");
        assert!(shades.contains(&rig.app.look().theme.border.rgb), "{slug}");
        assert!(!shades.contains(&accent) || shades.len() > 3, "{slug}");
    }
}

#[test]
fn the_ladder_of_colour_tiers_still_themes() {
    for (tier, glyphs) in [
        (ColorTier::Ansi256, Glyphs::Unicode),
        (ColorTier::Ansi16, Glyphs::Unicode),
        (ColorTier::None, Glyphs::Ascii),
    ] {
        let rig = Rig::with(
            settings_world(NEXUS, false),
            Caps {
                color: tier,
                glyphs,
            },
        );
        let name = format!("{tier:?}").to_lowercase();
        insta::assert_snapshot!(
            format!("settings_tier_{name}"),
            styled_of(&tui::render(&rig.app, STANDARD.0, STANDARD.1))
        );
    }
}

// ---- changing things -------------------------------------------------------

#[test]
fn enter_cycles_the_theme_and_the_file_keeps_its_comments() {
    let mut rig = Rig::new(false);
    assert_eq!(rig.theme(), "midnight-bloom");
    rig.key(KeyCode::Enter);
    rig.pump_until("Catppuccin", |a| a.look().theme.name == "catppuccin");
    let file = rig.config_text();
    assert!(file.contains("theme = \"catppuccin\""), "{file}");
    assert!(file.contains("# Nexus settings, edited by hand"), "{file}");
    assert!(
        file.contains("# try `nexus config set theme nord`"),
        "{file}"
    );
    assert!(file.contains("# too chatty on the train"), "{file}");
    assert!(rig.text(STANDARD).contains("Catppuccin"));
}

#[test]
fn cycling_reaches_the_custom_theme_and_wraps() {
    let mut rig = Rig::new(false);
    for slug in &THEMES[1..] {
        rig.key(KeyCode::Enter);
        rig.pump_until(slug, |a| a.look().theme.name == *slug);
    }
    rig.key(KeyCode::Enter);
    rig.pump_until("the custom theme", |a| a.look().theme.name == "custom");
    // sunrise = Gruvbox with a coral accent.
    assert_eq!(rig.app.look().theme.accent.rgb, (0xff, 0x7a, 0x59));
    assert_eq!(
        rig.app.look().theme.fg.rgb,
        tui::builtin_theme("gruvbox").unwrap().fg.rgb
    );
    assert!(rig.text(STANDARD).contains("sunrise"));
    rig.key(KeyCode::Enter);
    rig.pump_until("the first theme again", |a| {
        a.look().theme.name == "midnight-bloom"
    });
}

#[test]
fn reset_puts_the_default_back() {
    let mut rig = Rig::new(false);
    rig.key(KeyCode::Enter);
    rig.pump_until("Catppuccin", |a| a.look().theme.name == "catppuccin");
    rig.key(KeyCode::Char('r'));
    rig.pump_until("the default", |a| a.look().theme.name == "midnight-bloom");
    assert!(!rig.config_text().contains("theme = \"catppuccin\""));
}

#[test]
fn nerd_font_icons_follow_the_switch() {
    let mut rig = Rig::new(false);
    assert!(!rig.text(STANDARD).contains('\u{f013}'));
    rig.go_to("Nerd Font icons");
    rig.key(KeyCode::Enter);
    rig.pump_until("Nerd Font glyphs", |a| a.look().caps.glyphs == Glyphs::Nerd);
    assert!(rig.text(STANDARD).contains('\u{f013}'));
    assert!(rig.config_text().contains("nerd_font = true"));
    rig.key(KeyCode::Enter);
    rig.pump_until("Unicode glyphs", |a| {
        a.look().caps.glyphs == Glyphs::Unicode
    });
}

#[test]
fn ascii_stays_ascii_whatever_the_config_says() {
    let mut rig = Rig::with(
        settings_world(NEXUS, false),
        Caps {
            color: ColorTier::TrueColor,
            glyphs: Glyphs::Ascii,
        },
    );
    rig.set("ui.nerd_font", json!(true));
    rig.pump_until("the config", |a| {
        a.settings()
            .config()
            .is_some_and(|c| c["ui"]["nerd_font"] == true)
    });
    assert_eq!(rig.app.look().caps.glyphs, Glyphs::Ascii);
}

#[test]
fn reduced_motion_follows_the_switch() {
    let mut rig = Rig::new(false);
    assert_eq!(rig.app.movement(), Movement::Full);
    rig.go_to("Reduced motion");
    rig.key(KeyCode::Enter);
    rig.pump_until("reduced motion", |a| a.movement() == Movement::Reduced);
    assert!(rig.config_text().contains("reduce_motion = true"));
    rig.key(KeyCode::Enter);
    rig.pump_until("full motion", |a| a.movement() == Movement::Full);
}

#[test]
fn the_environment_keeps_motion_reduced() {
    let mut rig = Rig::new(false);
    rig.app.reduce_motion(true);
    assert_eq!(rig.app.movement(), Movement::Reduced);
    rig.set("ui.reduce_motion", json!(false));
    rig.set("theme", json!("nord"));
    rig.pump_until("Nord", |a| a.look().theme.name == "nord");
    assert_eq!(rig.app.movement(), Movement::Reduced);
    rig.go_to("Reduced motion");
    assert!(rig.text(STANDARD).contains("NEXUS_REDUCE_MOTION"));
}

#[test]
fn each_notification_switch_edits_its_key() {
    let mut rig = Rig::new(false);
    for (label, key, from) in [
        ("Notify: joined", "joined", true),
        ("Notify: lost", "lost", false),
        ("Notify: bad password", "wrong_password", true),
        ("Notify: prompt", "secret_prompt", true),
        ("Notify: login", "portal", true),
        ("Notify: VPN drop", "vpn_dropped", true),
    ] {
        rig.go_to(label);
        rig.key(KeyCode::Enter);
        rig.pump_until(label, |a| {
            a.settings()
                .config()
                .is_some_and(|c| c["notifications"][key] == !from)
        });
        let got = rig
            .world
            .wire()
            .call(json!({"op": "config_get", "key": format!("notifications.{key}")}));
        assert_eq!(got["ok"]["value"], json!(!from), "{key}: {got}");
    }
    insta::assert_snapshot!("settings_notifications_flipped", rig.text(STANDARD));
}

#[test]
fn the_connectivity_row_shows_networkmanagers_address_read_only() {
    let mut rig = Rig::new(false);
    rig.go_to("Connectivity check");
    rig.pump_until("the address", |a| a.settings().connectivity_uri().is_some());
    let frame = rig.text(STANDARD);
    let uri = rig.app.settings().connectivity_uri().unwrap().to_owned();
    assert!(frame.contains(&uri), "{frame}");
    assert!(frame.contains("NetworkManager.conf"), "{frame}");
    assert!(frame.contains("ConnectivityCheckEnabled"), "{frame}");
}

#[test]
fn u_puts_a_switch_back_and_a_reset_too() {
    let mut rig = Rig::new(false);
    rig.go_to("Notify: joined");
    rig.key(KeyCode::Enter);
    rig.pump_until("joined off", |a| {
        a.settings()
            .config()
            .is_some_and(|c| c["notifications"]["joined"] == false)
    });
    assert!(rig.app.toast().is_some_and(|t| t.undo.is_some()));
    rig.key(KeyCode::Char('u'));
    rig.pump_until("joined back on", |a| {
        a.settings()
            .config()
            .is_some_and(|c| c["notifications"]["joined"] != false)
    });

    // Reset undoes too: "Notify: lost" starts switched off in this world.
    rig.go_to("Notify: lost");
    rig.key(KeyCode::Char('r'));
    rig.pump_until("lost reset", |a| {
        a.settings()
            .config()
            .is_some_and(|c| c["notifications"]["lost"] != false)
    });
    rig.key(KeyCode::Char('u'));
    rig.pump_until("lost off again", |a| {
        a.settings()
            .config()
            .is_some_and(|c| c["notifications"]["lost"] == false)
    });
}

#[test]
fn a_hand_edit_changes_the_running_tui() {
    let mut rig = Rig::new(false);
    rig.world.write_config("theme = \"tokyo-night\"\n");
    rig.pump_until("Tokyo Night", |a| a.look().theme.name == "tokyo-night");
}

#[test]
fn the_palette_switches_themes() {
    let mut rig = Rig::new(false);
    rig.key(KeyCode::Char('1'));
    rig.key(KeyCode::Char(':'));
    rig.type_text("switch theme");
    rig.key(KeyCode::Enter);
    rig.pump_until("Catppuccin", |a| a.look().theme.name == "catppuccin");
}

// ---- a theme that cannot be used --------------------------------------------

#[test]
fn an_invalid_custom_theme_falls_back_with_a_warning() {
    let mut rig = Rig::new(true);
    assert_eq!(rig.theme(), "midnight-bloom");
    let note = rig.app.theme_note().expect("a warning").to_owned();
    assert!(note.contains("accent = teal"), "{note}");
    assert!(
        rig.app
            .toast()
            .is_some_and(|t| t.bad && t.text.contains("Using Midnight Bloom")),
        "{:?}",
        rig.app.toast()
    );
    insta::assert_snapshot!("settings_bad_theme", rig.text(STANDARD));
    // Fixing it by hand clears the warning.
    rig.world.write_config("theme = \"nord\"\n");
    rig.pump_until("Nord", |a| a.look().theme.name == "nord");
    assert!(rig.app.theme_note().is_none());
}

#[test]
fn an_unknown_theme_name_falls_back_too() {
    let mut rig = Rig::new(false);
    rig.set("theme", json!("vaporwave"));
    rig.pump_until("the warning", |a| a.theme_note().is_some());
    assert_eq!(rig.theme(), "midnight-bloom");
    assert!(rig.app.theme_note().unwrap().contains("`vaporwave`"));
}

#[test]
fn a_broken_config_file_leaves_the_tui_themed_and_says_why() {
    let mut rig = Rig::new(false);
    rig.set("theme", json!("nord"));
    rig.pump_until("Nord", |a| a.look().theme.name == "nord");
    rig.world.write_config("theme = [oops\n");
    rig.pump_until("the Daemon's warning", |a| a.settings().warning().is_some());
    let frame = rig.text(STANDARD);
    assert!(frame.contains("config.toml is being ignored"), "{frame}");
}

#[test]
fn clicking_a_row_selects_then_changes_it() {
    let mut rig = Rig::new(false);
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Reduced motion"));
    // Render once so the click rectangles are recorded.
    let _ = tui::render(&rig.app, STANDARD.0, STANDARD.1);
    let (x, y) = find(&rig, "Reduced motion");
    click(&mut rig, x, y);
    assert_eq!(rig.app.selected(), 2);
    let _ = tui::render(&rig.app, STANDARD.0, STANDARD.1);
    click(&mut rig, x, y);
    rig.pump_until("reduced motion", |a| a.movement() == Movement::Reduced);
}

fn find(rig: &Rig, needle: &str) -> (u16, u16) {
    let buf = tui::render(&rig.app, STANDARD.0, STANDARD.1);
    for y in 0..buf.area.height {
        let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
        if let Some(at) = row.find(needle) {
            return (row[..at].chars().count() as u16, y);
        }
    }
    panic!("`{needle}` is not on screen");
}

fn click(rig: &mut Rig, x: u16, y: u16) {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    rig.app.update(Msg::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
}
