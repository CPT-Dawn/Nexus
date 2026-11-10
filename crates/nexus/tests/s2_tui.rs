//! Seam S2: the TUI on a real Daemon (with the fake NetworkManager behind
//! it). Keystrokes go in, `insta` frames come out.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_ipc::Traffic;
use nexus_testkit::{ActiveConnection, NmConnectivity, NmState, PATIENCE, Unit, World};
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
    client: Client,
}

impl Rig {
    fn start(world: World, caps: Caps) -> Rig {
        let (tx, rx) = mpsc::channel();
        let client = Client::spawn(world.socket(), tx);
        let mut rig = Rig {
            world,
            app: App::new(caps),
            rx,
            client,
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
        rig.pump_until("the online status and its first activity", |a| {
            a.status().is_some() && !a.activity().is_empty()
        });
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
            self.sync_watch();
        }
    }

    /// What `nexus` does after every frame: poll traffic only while the
    /// graphs are on screen.
    fn sync_watch(&self) {
        self.client.watch_traffic(self.app.traffic_visible());
        if let Some(id) = self.app.take_fix_request() {
            self.client.run_fix(id);
        }
    }

    fn key(&mut self, code: KeyCode) {
        self.app
            .update(Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        self.sync_watch();
    }

    /// Scripted Daemon traffic, for frames that must not depend on a clock.
    fn traffic(&mut self, rx_bps: u64, tx_bps: u64) {
        self.app.update(Msg::Traffic(Traffic { rx_bps, tx_bps }));
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
fn the_overview_claims_health_only_while_the_doctor_is_clean() {
    use nexus_ipc::{Finding, Severity};
    let mut rig = Rig::online();
    assert!(rig.text(STANDARD).contains("Everything looks healthy."));
    rig.app.update(Msg::Findings(vec![Finding {
        id: "dns".into(),
        severity: Severity::Warning,
        title: "DNS is slow".into(),
        cause: "The resolver takes seconds to answer.".into(),
        fix: None,
    }]));
    let frame = rig.text(STANDARD);
    assert!(!frame.contains("Everything looks healthy."), "{frame}");
    assert!(frame.contains("The Doctor has found a problem."), "{frame}");
    rig.app.update(Msg::Findings(Vec::new()));
    assert!(rig.text(STANDARD).contains("Everything looks healthy."));
}

#[test]
fn overview_online_at_wide_width() {
    let rig = Rig::online();
    insta::assert_snapshot!("overview_online_wide", rig.text(WIDE));
}

#[test]
fn overview_online_at_narrow_width_stacks_everything() {
    let rig = Rig::online();
    let frame = rig.text(NARROW);
    for part in ["Online", "Traffic", "Wi-Fi", "Recent activity"] {
        assert!(frame.contains(part), "missing {part}:\n{frame}");
    }
    insta::assert_snapshot!("overview_online_narrow", frame);
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

    assert!(root.contains("enter open"), "cards open their tab: {root}");
    rig.size(NARROW);
    let narrow = last_line(&rig, NARROW);
    assert!(narrow.contains("enter open"), "{narrow}");
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

// ---- the Doctor: first run, diagnostics, fixes --------------------------------

/// A machine whose NetworkManager is installed but stopped, with networkd
/// in charge: the textbook first run. The connectivity check is off so no
/// lookup depends on the host running the tests.
fn first_run_rig() -> Rig {
    let mut world = World::builder(NEXUS)
        .nm_running(false)
        .daemon(false)
        .start();
    world.write_config("[connectivity]\nenabled = false\n[doctor]\ngateway = \"off\"\n");
    world.start_daemon();
    world
        .systemd
        .set_unit("NetworkManager.service", Unit::Inactive);
    world
        .systemd
        .set_unit("systemd-networkd.service", Unit::Active);
    let mut rig = Rig::start(world, truecolor());
    rig.pump_until("the setup findings", |a| {
        a.findings().iter().any(|f| f.id == "networkmanager")
    });
    rig
}

#[test]
fn first_run_without_networkmanager_shows_the_exact_commands() {
    let mut rig = first_run_rig();
    for (name, size) in [("narrow", NARROW), ("standard", STANDARD), ("wide", WIDE)] {
        let frame = rig.text(size);
        // The narrow box wraps long commands, so look for their start there.
        let (first, last) = if size == NARROW {
            (
                "$ sudo systemctl disable --now",
                "$ sudo systemctl enable --now",
            )
        } else {
            (
                "sudo systemctl disable --now systemd-networkd.service",
                "sudo systemctl enable --now NetworkManager.service",
            )
        };
        assert!(frame.contains(first) && frame.contains(last), "{frame}");
        assert!(!frame.contains('`'), "no raw backticks on screen:\n{frame}");
        insta::assert_snapshot!(format!("first_run_{name}"), frame);
    }
    rig.size(STANDARD);
    assert!(
        rig.text(STANDARD).contains("f fix"),
        "the footer offers the fix"
    );
}

#[test]
fn nothing_privileged_runs_without_a_yes() {
    let mut rig = first_run_rig();
    let log = rig.world.dir().join("pkexec.log");
    rig.world
        .install_tool("pkexec", &format!("echo \"$@\" >> {}", log.display()));
    rig.key(KeyCode::Char('f'));
    let ask = rig.text(STANDARD);
    assert!(
        ask.contains("Run the fix?") && ask.contains("ask for your password"),
        "{ask}"
    );
    insta::assert_snapshot!("first_run_confirm", ask);
    assert_eq!(last_line(&rig, STANDARD).trim(), "y run  n cancel");

    rig.key(KeyCode::Char('n'));
    assert!(!rig.text(STANDARD).contains("Run the fix?"));
    assert_eq!(*rig.app.fix_state(), tui::FixState::Idle);
    assert!(!log.exists(), "a no must run nothing");
}

#[test]
fn a_yes_runs_the_fix_through_pkexec_and_the_screen_says_so() {
    let mut rig = first_run_rig();
    let log = rig.world.dir().join("pkexec.log");
    rig.world
        .install_tool("pkexec", &format!("echo \"$@\" >> {}", log.display()));
    rig.key(KeyCode::Char('f'));
    rig.key(KeyCode::Char('y'));
    rig.pump_until("the fix to finish", |a| {
        *a.fix_state() == tui::FixState::Done
    });
    let ran = std::fs::read_to_string(&log).expect("pkexec ran");
    assert_eq!(
        ran.lines().collect::<Vec<_>>(),
        [
            "/bin/sh -c systemctl disable --now systemd-networkd.service",
            "/bin/sh -c systemctl enable --now NetworkManager.service",
        ]
    );
    assert!(rig.text(STANDARD).contains("Done."));
}

#[test]
fn a_refused_password_is_shown_not_swallowed() {
    let mut rig = first_run_rig();
    rig.world.install_tool("pkexec", "exit 127");
    rig.key(KeyCode::Char('f'));
    rig.key(KeyCode::Char('y'));
    rig.pump_until("the failure", |a| {
        matches!(a.fix_state(), tui::FixState::Failed(_))
    });
    let frame = rig.text(STANDARD);
    assert!(frame.contains("not authorized"), "{frame}");
}

#[test]
fn trouble_on_the_overview_gets_a_diagnostics_panel() {
    let mut rig = portal_rig();
    rig.pump_until("the portal finding", |a| {
        a.findings().iter().any(|f| f.id == "portal")
    });
    for (name, size) in [("narrow", NARROW), ("standard", STANDARD), ("wide", WIDE)] {
        let frame = rig.text(size);
        assert!(frame.contains("Diagnostics"), "{name}:\n{frame}");
        insta::assert_snapshot!(format!("diagnostics_{name}"), frame);
    }
    // Nothing to run for a sign-in page: no `f`.
    assert!(!last_line(&rig, STANDARD).contains("fix"));
}

#[test]
fn a_healthy_overview_has_no_diagnostics_panel() {
    let rig = Rig::online();
    assert!(!rig.text(STANDARD).contains("Diagnostics"));
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
                // Settled: the Daemon's picture is in, and the "You're online"
                // toast's effect has ended (it asks for frames while it runs).
                online.store(
                    app.status().is_some() && !app.activity().is_empty() && !app.animating(),
                    Ordering::SeqCst,
                );
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

// ---- Overview (#12): hero, traffic, cards, activity -----------------------------

/// A rig with a Wi-Fi Device, so the Daemon has counters to turn into rates.
fn online_with_device() -> (Rig, nexus_testkit::DeviceId) {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    go_online(&world);
    let mut rig = Rig::start(world, truecolor());
    rig.pump_until("status and activity", |a| {
        a.status().is_some() && !a.activity().is_empty()
    });
    (rig, wlan0)
}

fn portal_rig() -> Rig {
    let world = World::start(NEXUS);
    world
        .nm
        .set_primary(Some(ActiveConnection::wifi("CafeGuest")));
    world.nm.set_connectivity(NmConnectivity::Portal);
    world.nm.set_state(NmState::ConnectedGlobal);
    let mut rig = Rig::start(world, truecolor());
    rig.pump_until("the portal status and its finding", |a| {
        a.status()
            .is_some_and(|s| s.connectivity == nexus_ipc::Connectivity::Portal)
            && !a.activity().is_empty()
            && a.findings().iter().any(|f| f.id == "portal")
    });
    rig
}

#[test]
fn the_hero_has_big_type_for_every_state() {
    let online = Rig::online();
    insta::assert_snapshot!("hero_online_standard", online.text(STANDARD));

    let portal = portal_rig();
    let frame = portal.text(STANDARD);
    assert!(frame.contains("a sign-in page is"), "{frame}");
    insta::assert_snapshot!("hero_portal_standard", frame);

    let mut offline = Rig::online();
    offline.world.nm.set_primary(None);
    offline.world.nm.set_state(NmState::Disconnected);
    offline.pump_until("offline", |a| {
        a.status()
            .is_some_and(|s| s.state == nexus_ipc::State::Offline)
    });
    insta::assert_snapshot!("hero_offline_standard", offline.text(STANDARD));

    let mut connecting = Rig::online();
    connecting.world.nm.set_state(NmState::Connecting);
    connecting.pump_until("connecting", |a| {
        a.status()
            .is_some_and(|s| s.state == nexus_ipc::State::Connecting)
    });
    insta::assert_snapshot!("hero_connecting_standard", connecting.text(STANDARD));
}

#[test]
fn the_hero_glows_only_where_colour_can_carry_it() {
    let rig = Rig::online_with(truecolor());
    assert!(rig.text(STANDARD).contains('░'), "halo missing");
    let mono = Rig::online_with(caps(ColorTier::None, Glyphs::Unicode));
    assert!(!mono.text(STANDARD).contains('░'), "no halo without colour");
    let ascii = Rig::online_with(caps(ColorTier::TrueColor, Glyphs::Ascii));
    let frame = ascii.text(STANDARD);
    assert!(frame.is_ascii(), "ASCII mode leaked non-ASCII:\n{frame}");
    assert!(frame.contains('#'), "{frame}");
}

#[test]
fn traffic_graphs_at_every_width() {
    for (name, size) in [("narrow", NARROW), ("standard", STANDARD), ("wide", WIDE)] {
        let mut rig = Rig::online();
        rig.size(size);
        let ramp = [
            0, 0, 200_000, 900_000, 2_400_000, 1_200_000, 600_000, 1_800_000,
        ];
        for (i, rx) in ramp.into_iter().enumerate() {
            rig.traffic(rx, rx / 12 + i as u64 * 1_000);
        }
        let frame = rig.text(size);
        assert!(frame.contains("1.8 MB/s"), "{name}:\n{frame}");
        assert!(
            frame
                .chars()
                .any(|c| ('\u{2840}'..='\u{28ff}').contains(&c)),
            "braille"
        );
        insta::assert_snapshot!(format!("traffic_{name}"), frame);
    }
}

#[test]
fn ascii_graphs_use_plain_bars() {
    let mut rig = Rig::online_with(caps(ColorTier::TrueColor, Glyphs::Ascii));
    for rx in [0, 100_000, 1_000_000, 3_000_000] {
        rig.traffic(rx, rx / 10);
    }
    let frame = rig.text(STANDARD);
    assert!(frame.is_ascii(), "{frame}");
    insta::assert_snapshot!("traffic_ascii", frame);
}

#[test]
fn rates_read_in_plain_units() {
    let mut rig = Rig::online();
    for (rx, tx, want) in [
        (0, 0, "0 B/s"),
        (950, 12, "950 B/s"),
        (12_300, 0, "12.3 KB/s"),
        (3_450_000, 0, "3.5 MB/s"),
        (2_100_000_000, 0, "2.1 GB/s"),
    ] {
        rig.traffic(rx, tx);
        let frame = rig.text(STANDARD);
        assert!(frame.contains(want), "{want}:\n{frame}");
    }
}

#[test]
fn traffic_arrives_from_the_daemon_while_the_overview_shows() {
    let (mut rig, wlan0) = online_with_device();
    rig.sync_watch();
    rig.pump_until("a baseline sample", |a| a.traffic_samples() >= 1);
    rig.world.nm.set_traffic(wlan0, 5_000_000, 500_000);
    rig.pump_until("a download rate", |a| {
        a.traffic_now()
            .is_some_and(|t| t.rx_bps > 0 && t.tx_bps > 0)
    });
    assert!(rig.text(STANDARD).contains("B/s"));
}

/// The 1 Hz refresh runs only while the graphs are visible (ADR-0003).
#[test]
fn traffic_polling_stops_when_the_graphs_leave_the_screen() {
    let (mut rig, _wlan0) = online_with_device();
    rig.sync_watch();
    rig.pump_until("polling to start", |a| a.traffic_samples() >= 1);

    // Leave the Overview: nothing may read counters from now on.
    rig.key(KeyCode::Char('l'));
    assert!(!rig.app.traffic_visible());
    let settle = Instant::now() + Duration::from_millis(1300); // one in-flight poll
    while Instant::now() < settle {
        let _ = rig.rx.recv_timeout(Duration::from_millis(50));
    }
    let reads = rig.world.nm.statistics_reads();
    // The one deliberate wait: proving a negative.
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(
        rig.world.nm.statistics_reads(),
        reads,
        "polled while hidden"
    );
    assert_eq!(
        rig.app.traffic_samples(),
        0,
        "history is dropped when hidden"
    );

    // Back on screen: polling resumes.
    rig.key(KeyCode::Char('h'));
    assert!(rig.app.traffic_visible());
    rig.pump_until("polling to resume", |a| a.traffic_samples() >= 1);
    assert!(rig.world.nm.statistics_reads() > reads);
}

#[test]
fn graphs_are_not_visible_behind_overlays_or_in_a_tiny_window() {
    let mut rig = Rig::online();
    assert!(rig.app.traffic_visible());
    rig.type_str("?");
    assert!(!rig.app.traffic_visible());
    rig.key(KeyCode::Esc);
    rig.size((30, 6));
    assert!(!rig.app.traffic_visible());
    rig.size(NARROW);
    assert!(rig.app.traffic_visible());
}

/// ADR-0003 with the graphs running: a flat line costs no frames.
#[test]
fn a_visible_but_flat_graph_draws_no_frames() {
    let world = World::start(NEXUS);
    world.nm.add_wifi_device("wlan0");
    go_online(&world);
    let (tx, rx) = mpsc::channel();
    let client = Client::spawn(world.socket(), tx.clone());
    let frames = Arc::new(AtomicUsize::new(0));
    let looper = {
        let frames = frames.clone();
        std::thread::spawn(move || {
            let mut terminal = Terminal::new(TestBackend::new(100, 28)).unwrap();
            let mut app = App::new(truecolor());
            tui::event_loop(&mut terminal, &mut app, &rx, |app| {
                frames.fetch_add(1, Ordering::SeqCst);
                client.watch_traffic(app.traffic_visible());
            })
            .unwrap();
        })
    };
    // Wait until the Daemon has been asked for counters a few times.
    let deadline = Instant::now() + PATIENCE;
    while world.nm.statistics_reads() < 4 {
        assert!(Instant::now() < deadline, "polling never started");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(200));
    let settled = frames.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(2500)); // proving a negative
    assert_eq!(
        frames.load(Ordering::SeqCst),
        settled,
        "flat graph drew frames"
    );
    tx.send(Msg::Key(KeyEvent::new(
        KeyCode::Char('q'),
        KeyModifiers::NONE,
    )))
    .unwrap();
    looper.join().unwrap();
}

#[test]
fn cards_show_where_you_are_and_enter_opens_their_tab() {
    let mut rig = Rig::online();
    let frame = rig.text(STANDARD);
    for card in ["Wi-Fi", "Ethernet", "VPN", "Hotspot"] {
        assert!(frame.contains(card), "{card}:\n{frame}");
    }
    assert!(
        frame.contains("Home-5G") && frame.contains("Not started"),
        "{frame}"
    );

    rig.type_str("jj");
    rig.key(KeyCode::Enter);
    assert_eq!(rig.app.tab().title(), "VPN");
    rig.key(KeyCode::Char('1'));
    rig.type_str("G");
    rig.key(KeyCode::Enter);
    assert_eq!(rig.app.tab().title(), "Hotspot");

    // Narrow works the same: the cards are in the stack.
    let mut rig = Rig::online();
    rig.size(NARROW);
    rig.key(KeyCode::Enter);
    assert_eq!(rig.app.tab().title(), "Wi-Fi");
}

#[test]
fn the_activity_strip_follows_the_network_live() {
    let mut rig = Rig::online();
    assert!(rig.text(STANDARD).contains("Joined Home-5G (Wi-Fi)"));
    rig.world.nm.set_primary(None);
    rig.world.nm.set_state(NmState::Disconnected);
    rig.pump_until("the loss", |a| {
        a.activity()
            .first()
            .is_some_and(|e| e.text == "Lost Home-5G")
    });
    let frame = rig.text(STANDARD);
    let lost = frame.find("Lost Home-5G").expect("lost line");
    let joined = frame.find("Joined Home-5G").expect("joined line");
    assert!(lost < joined, "newest first:\n{frame}");
    insta::assert_snapshot!("activity_strip_standard", frame);
}

#[test]
fn the_activity_strip_has_a_friendly_empty_state() {
    let world = World::start(NEXUS);
    let mut rig = Rig::start(world, truecolor());
    rig.pump_until("status", |a| a.status().is_some());
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Nothing yet"), "{frame}");
    insta::assert_snapshot!("activity_empty_standard", frame);
}

#[test]
fn the_wide_overview_puts_traffic_beside_activity() {
    let mut rig = Rig::online();
    rig.size(WIDE);
    rig.traffic(1_500_000, 120_000);
    insta::assert_snapshot!("overview_wide_full", rig.text(WIDE));
}

#[test]
fn a_short_window_drops_sections_instead_of_corrupting_them() {
    let rig = Rig::online();
    for size in [(60u16, 10u16), (60, 14), (100, 14), (100, 18)] {
        let frame = rig.text(size);
        assert!(frame.contains("Online"), "{size:?}:\n{frame}");
        insta::assert_snapshot!(format!("overview_short_{}x{}", size.0, size.1), frame);
    }
}
