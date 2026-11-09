//! Seam S2: the TUI on a real Daemon (with the fake NetworkManager behind
//! it). Keystrokes go in, `insta` frames come out.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_testkit::{ActiveConnection, NmConnectivity, NmState, PATIENCE, World};
use nexus_tui::tui::{self, App, Caps, Client, ColorTier, Glyphs, Msg};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

const NARROW: (u16, u16) = (60, 20);
const STANDARD: (u16, u16) = (100, 28);
const WIDE: (u16, u16) = (140, 32);

fn caps(color: ColorTier, glyphs: Glyphs) -> Caps {
    Caps { color, glyphs }
}

fn truecolor() -> Caps {
    caps(ColorTier::TrueColor, Glyphs::Unicode)
}

fn go_online(world: &World) {
    world
        .nm
        .set_primary(Some(ActiveConnection::wifi("Home-5G")));
    world.nm.set_connectivity(NmConnectivity::Full);
    world.nm.set_state(NmState::ConnectedGlobal);
}

/// A TUI wired to a world's Daemon exactly as `nexus` wires it, minus the
/// real terminal.
struct Rig {
    world: World,
    app: App,
    rx: Receiver<Msg>,
    _client: Client,
}

impl Rig {
    fn start(world: World, caps: Caps) -> Rig {
        let (tx, rx) = mpsc::channel();
        let client = Client::spawn(world.socket(), tx);
        let mut rig = Rig {
            world,
            app: App::new(caps),
            rx,
            _client: client,
        };
        rig.size(STANDARD);
        rig
    }

    fn online() -> Rig {
        Rig::online_with(truecolor())
    }

    fn online_with(caps: Caps) -> Rig {
        let world = World::start(NEXUS);
        go_online(&world);
        let mut rig = Rig::start(world, caps);
        rig.pump_until("the online status", |a| a.status().is_some());
        rig
    }

    fn size(&mut self, (w, h): (u16, u16)) {
        self.app.update(Msg::Resize(w, h));
    }

    /// Feeds Daemon messages to the app until `done` holds.
    fn pump_until(&mut self, what: &str, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(&self.app) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| panic!("timed out waiting for {what}"));
            let msg = self
                .rx
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
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

    fn buffer(&self, (w, h): (u16, u16)) -> Buffer {
        tui::render(&self.app, w, h)
    }

    fn text(&self, size: (u16, u16)) -> String {
        text_of(&self.buffer(size))
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

/// The frame's text plus a table of every distinct style it uses and how
/// many cells wear it, so colour tiers are visible in a text snapshot.
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

fn last_line(rig: &Rig, size: (u16, u16)) -> String {
    rig.text(size).lines().last().unwrap().to_string()
}

// ---- the Overview, at every width -------------------------------------------

#[test]
fn overview_online_at_standard_width() {
    let rig = Rig::online();
    let frame = rig.text(STANDARD);
    assert!(
        frame.contains("You're online via Wi-Fi (Home-5G). Everything looks healthy."),
        "{frame}"
    );
    insta::assert_snapshot!("overview_online_standard", frame);
}

#[test]
fn overview_online_at_wide_width() {
    let rig = Rig::online();
    insta::assert_snapshot!("overview_online_wide", rig.text(WIDE));
}

#[test]
fn overview_online_at_narrow_width_is_a_single_list() {
    let rig = Rig::online();
    let frame = rig.text(NARROW);
    assert!(
        !frame.contains("Everything looks healthy"),
        "detail lives in the sheet:\n{frame}"
    );
    insta::assert_snapshot!("overview_online_narrow", frame);
}

#[test]
fn narrow_enter_slides_in_the_sheet_and_esc_pops_it() {
    let mut rig = Rig::online();
    rig.size(NARROW);
    rig.key(KeyCode::Enter);
    let sheet = rig.text(NARROW);
    assert!(sheet.contains("You're online via Wi-Fi"), "{sheet}");
    insta::assert_snapshot!("overview_sheet_narrow", sheet);
    rig.key(KeyCode::Esc);
    assert!(!rig.text(NARROW).contains("You're online via Wi-Fi"));
    assert!(!rig.app.should_quit(), "Esc must never quit");
}

#[test]
fn a_tiny_terminal_gets_a_hint_not_a_corrupt_layout() {
    let rig = Rig::online();
    insta::assert_snapshot!("too_small", rig.text((30, 6)));
}

#[test]
fn live_daemon_status_changes_reach_the_screen() {
    let mut rig = Rig::online();
    rig.world.nm.set_primary(None);
    rig.world.nm.set_state(NmState::Disconnected);
    rig.pump_until("the offline status", |a| {
        a.status()
            .is_some_and(|s| s.state == nexus_ipc::State::Offline)
    });
    let frame = rig.text(STANDARD);
    assert!(frame.contains("You're offline"), "{frame}");
}

// ---- keys ---------------------------------------------------------------------

#[test]
fn question_mark_lists_every_key_and_esc_closes_it() {
    let mut rig = Rig::online();
    rig.type_str("?");
    insta::assert_snapshot!("help_overlay", rig.text(STANDARD));
    rig.key(KeyCode::Esc);
    assert!(!rig.text(STANDARD).contains("All keys"));
    assert!(!rig.app.should_quit());
}

#[test]
fn the_footer_shows_only_keys_valid_right_now() {
    let mut rig = Rig::online();
    let root = last_line(&rig, STANDARD);
    assert!(root.contains("q quit") && root.contains("? help"), "{root}");
    assert!(
        !root.contains("esc"),
        "nothing to back out of at the root: {root}"
    );

    rig.type_str("?");
    let help = last_line(&rig, STANDARD);
    assert!(
        help.contains("esc close") && !help.contains("q quit"),
        "{help}"
    );
    rig.key(KeyCode::Esc);

    rig.size(NARROW);
    let narrow = last_line(&rig, NARROW);
    assert!(narrow.contains("enter open"), "{narrow}");
    rig.key(KeyCode::Enter);
    let sheet = last_line(&rig, NARROW);
    assert!(
        sheet.contains("esc back") && !sheet.contains("enter open"),
        "{sheet}"
    );
}

#[test]
fn tabs_switch_with_vim_keys_arrows_tab_and_numbers() {
    let mut rig = Rig::online();
    let tab = |rig: &Rig| rig.app.tab().title();
    assert_eq!(tab(&rig), "Overview");
    rig.type_str("l");
    assert_eq!(tab(&rig), "Wi-Fi");
    rig.key(KeyCode::Right);
    assert_eq!(tab(&rig), "Ethernet");
    rig.key(KeyCode::Tab);
    assert_eq!(tab(&rig), "VPN");
    rig.type_str("h");
    assert_eq!(tab(&rig), "Ethernet");
    rig.key(KeyCode::Left);
    rig.key(KeyCode::BackTab);
    assert_eq!(tab(&rig), "Overview");
    rig.type_str("6");
    assert_eq!(tab(&rig), "Settings");
    insta::assert_snapshot!("settings_tab_placeholder", rig.text(STANDARD));
}

#[test]
fn list_selection_moves_with_jk_arrows_and_g() {
    let mut rig = Rig::online();
    assert_eq!(rig.app.selected(), 0);
    rig.type_str("j");
    rig.key(KeyCode::Down);
    assert_eq!(rig.app.selected(), 2);
    rig.type_str("G");
    assert_eq!(rig.app.selected(), 3);
    rig.key(KeyCode::Down);
    assert_eq!(rig.app.selected(), 3, "stops at the end");
    rig.type_str("g");
    assert_eq!(rig.app.selected(), 0);
    rig.type_str("k");
    assert_eq!(rig.app.selected(), 0);
}

#[test]
fn q_quits_from_the_root_only() {
    let mut rig = Rig::online();
    rig.type_str("?");
    rig.type_str("q");
    assert!(!rig.app.should_quit(), "q inside help must not quit");
    rig.key(KeyCode::Esc);
    rig.type_str("q");
    assert!(rig.app.should_quit());

    let mut rig = Rig::online();
    rig.app.update(Msg::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    )));
    assert!(rig.app.should_quit(), "Ctrl-C always quits");
}

// ---- the Daemon going away ----------------------------------------------------

#[test]
fn unreachable_daemon_is_a_clear_state_and_reconnects_by_itself() {
    let world = World::builder(NEXUS).daemon(false).start();
    go_online(&world);
    let mut rig = Rig::start(world, truecolor());
    rig.pump_until("the unreachable state", |a| a.is_down());
    let down = rig.text(STANDARD);
    assert!(down.contains("Can't reach the Nexus daemon"), "{down}");
    insta::assert_snapshot!("daemon_unreachable", down);

    rig.world.start_daemon();
    rig.pump_until("the reconnect", |a| a.status().is_some());
    let up = rig.text(STANDARD);
    assert!(up.contains("You're online via Wi-Fi"), "{up}");
}

#[test]
fn a_restarting_daemon_shows_the_state_then_recovers() {
    let mut rig = Rig::online();
    rig.world.stop_daemon();
    rig.pump_until("the unreachable state", |a| a.is_down());
    rig.world.start_daemon();
    rig.pump_until("the reconnect", |a| !a.is_down() && a.status().is_some());
}

#[test]
fn a_missing_network_manager_is_a_status_not_a_crash() {
    let world = World::builder(NEXUS).nm_running(false).start();
    let mut rig = Rig::start(world, truecolor());
    rig.pump_until("the status", |a| a.status().is_some());
    let frame = rig.text(STANDARD);
    assert!(frame.contains("NetworkManager is not running"), "{frame}");
}

// ---- colour tiers, glyphs, transparency ----------------------------------------

#[test]
fn colour_tiers_each_have_a_designed_frame() {
    for (name, c) in [
        ("truecolor", caps(ColorTier::TrueColor, Glyphs::Unicode)),
        ("256", caps(ColorTier::Ansi256, Glyphs::Unicode)),
        ("16", caps(ColorTier::Ansi16, Glyphs::Unicode)),
        ("none", caps(ColorTier::None, Glyphs::Unicode)),
        ("ascii", caps(ColorTier::None, Glyphs::Ascii)),
    ] {
        let rig = Rig::online_with(c);
        insta::assert_snapshot!(format!("tier_{name}"), styled_of(&rig.buffer(STANDARD)));
    }
}

#[test]
fn midnight_bloom_is_the_default_and_the_background_is_transparent() {
    let rig = Rig::online();
    let buf = rig.buffer(STANDARD);
    assert!(
        buf.content()
            .iter()
            .any(|c| c.fg == Color::Rgb(0x8d, 0xe3, 0xdc)),
        "mint accent #8de3dc is on screen"
    );
    let bare = buf
        .content()
        .iter()
        .filter(|c| c.bg == Color::Reset)
        .count();
    assert!(
        bare * 10 > buf.content().len() * 8,
        "most cells leave the background to the terminal"
    );
    assert!(
        !buf.content()
            .iter()
            .any(|c| c.bg == Color::Rgb(0x0b, 0x12, 0x14)),
        "the palette background is not painted unless opaque"
    );
}

#[test]
fn no_color_keeps_selection_visible_with_reverse_video() {
    let rig = Rig::online_with(caps(ColorTier::None, Glyphs::Unicode));
    let buf = rig.buffer(STANDARD);
    assert!(
        buf.content()
            .iter()
            .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
    );
    assert!(
        buf.content()
            .iter()
            .any(|c| c.modifier.contains(Modifier::REVERSED))
    );
}

#[test]
fn ascii_glyphs_stay_in_ascii() {
    let mut rig = Rig::online_with(caps(ColorTier::None, Glyphs::Ascii));
    for size in [NARROW, STANDARD, WIDE] {
        rig.size(size);
        let frame = rig.text(size);
        assert!(frame.is_ascii(), "non-ASCII in {size:?}:\n{frame}");
        rig.type_str("?");
        let help = rig.text(size);
        assert!(help.is_ascii(), "non-ASCII help in {size:?}:\n{help}");
        rig.key(KeyCode::Esc);
    }
}

#[test]
fn nerd_font_icons_are_opt_in() {
    let private_use = |s: &str| s.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c));
    assert!(!private_use(&Rig::online().text(STANDARD)));
    let nerd = Rig::online_with(caps(ColorTier::TrueColor, Glyphs::Nerd)).text(STANDARD);
    assert!(private_use(&nerd));
}

#[test]
fn capability_detection_honours_the_environment() {
    type Pairs<'a> = &'a [(&'a str, &'a str)];
    fn env(pairs: Pairs<'_>) -> impl Fn(&str) -> Option<String> + '_ {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }
    let utf8 = ("LANG", "en_US.UTF-8");
    let detect = |pairs: Pairs<'_>| Caps::detect(env(pairs), false, false);
    assert_eq!(
        detect(&[("COLORTERM", "truecolor"), utf8]).color,
        ColorTier::TrueColor
    );
    assert_eq!(
        detect(&[("COLORTERM", "24bit"), utf8]).color,
        ColorTier::TrueColor
    );
    assert_eq!(
        detect(&[("TERM", "xterm-256color"), utf8]).color,
        ColorTier::Ansi256
    );
    assert_eq!(detect(&[("TERM", "xterm"), utf8]).color, ColorTier::Ansi16);
    assert_eq!(detect(&[("TERM", "dumb"), utf8]).color, ColorTier::None);
    // NO_COLOR wins over everything, but empty means unset.
    assert_eq!(
        detect(&[("COLORTERM", "truecolor"), ("NO_COLOR", "1"), utf8]).color,
        ColorTier::None
    );
    assert_eq!(
        detect(&[("COLORTERM", "truecolor"), ("NO_COLOR", ""), utf8]).color,
        ColorTier::TrueColor
    );
    // Glyphs: Unicode by default, ASCII without a UTF-8 locale or on a dumb
    // terminal, Nerd Fonts only on request, and ASCII beats Nerd.
    assert_eq!(detect(&[utf8]).glyphs, Glyphs::Unicode);
    assert_eq!(detect(&[("LANG", "C")]).glyphs, Glyphs::Ascii);
    assert_eq!(detect(&[("TERM", "dumb"), utf8]).glyphs, Glyphs::Ascii);
    assert_eq!(
        Caps::detect(env(&[utf8]), true, false).glyphs,
        Glyphs::Ascii
    );
    assert_eq!(Caps::detect(env(&[utf8]), false, true).glyphs, Glyphs::Nerd);
    assert_eq!(
        Caps::detect(env(&[("LANG", "C")]), false, true).glyphs,
        Glyphs::Ascii
    );
}

// ---- zero idle frames -----------------------------------------------------------

/// ADR-0003: with nothing happening the TUI draws nothing.
#[test]
fn an_idle_tui_draws_no_frames() {
    let world = World::start(NEXUS);
    go_online(&world);

    let (tx, rx) = mpsc::channel();
    let _client = Client::spawn(world.socket(), tx.clone());
    let frames = Arc::new(AtomicUsize::new(0));
    let online = Arc::new(AtomicBool::new(false));
    let looper = {
        let (frames, online) = (frames.clone(), online.clone());
        std::thread::spawn(move || {
            let mut terminal = Terminal::new(TestBackend::new(100, 28)).unwrap();
            let mut app = App::new(truecolor());
            tui::event_loop(&mut terminal, &mut app, &rx, |app| {
                frames.fetch_add(1, Ordering::SeqCst);
                online.store(app.status().is_some(), Ordering::SeqCst);
            })
            .unwrap();
            text_of(terminal.backend().buffer())
        })
    };

    // Wait, polling, for the first frame that shows the Daemon's status.
    let deadline = Instant::now() + PATIENCE;
    while !online.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "no status frame");
        std::thread::sleep(Duration::from_millis(10));
    }
    let settled = frames.load(Ordering::SeqCst);

    // The one deliberate wait: proving a negative takes a quiet period that
    // outlasts the client's status poll and any timer the TUI might hold.
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(
        frames.load(Ordering::SeqCst),
        settled,
        "frames drawn while nothing happened"
    );

    // Input still gets through, and each event draws.
    tx.send(Msg::Key(KeyEvent::new(
        KeyCode::Char('l'),
        KeyModifiers::NONE,
    )))
    .unwrap();
    tx.send(Msg::Key(KeyEvent::new(
        KeyCode::Char('q'),
        KeyModifiers::NONE,
    )))
    .unwrap();
    let last = looper.join().unwrap();
    assert!(frames.load(Ordering::SeqCst) > settled);
    assert!(last.contains("Wi-Fi"), "{last}");
}
