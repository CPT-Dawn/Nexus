//! Seam S2 for event-driven effects and premium surfaces (#29, ADR-0003):
//! keystrokes and Daemon events start effects, `render_fx` draws a frame at a
//! virtual time, and the loop is shown to go back to sleep when they end.
//!
//! The effects themselves are a post-pass, so the final frame of every effect
//! equals the plain frame; the tests snapshot the final frames per colour tier
//! and width, and prove the middle frames differ and the motion rules hold.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_ipc::{
    ActivityEntry, ActivityKind, BackendInfo, Connectivity, Link, LinkKind, Radios, State, Status,
};
use nexus_testkit::scenarios::wifi_busy;
use nexus_testkit::{PATIENCE, World};
use nexus_tui::tui::{self, App, Caps, Client, ColorTier, Glyphs, Movement, Msg};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

const NARROW: (u16, u16) = (60, 20);
const STANDARD: (u16, u16) = (100, 28);
const WIDE: (u16, u16) = (140, 32);
const SIZES: [(&str, (u16, u16)); 3] = [("narrow", NARROW), ("standard", STANDARD), ("wide", WIDE)];

const FRAME: Duration = Duration::from_millis(33);
const LONG: Duration = Duration::from_secs(2);

fn caps(color: ColorTier, glyphs: Glyphs) -> Caps {
    Caps { color, glyphs }
}

fn truecolor() -> Caps {
    caps(ColorTier::TrueColor, Glyphs::Unicode)
}

fn status(state: State) -> Status {
    Status {
        state,
        connectivity: Connectivity::Full,
        link: Some(Link {
            name: "Home-5G".into(),
            kind: LinkKind::Wifi,
        }),
        backend: BackendInfo {
            name: "NetworkManager".into(),
            version: Some("1.50".into()),
        },
        radios: Radios::default(),
    }
}

fn key(app: &mut App, c: char) {
    app.update(Msg::Key(KeyEvent::new(
        KeyCode::Char(c),
        KeyModifiers::NONE,
    )));
}

/// An App at `size`, showing `state`, with no effect left over from setup.
fn app_at(caps: Caps, size: (u16, u16), state: State) -> App {
    let mut app = App::new(caps);
    app.update(Msg::Resize(size.0, size.1));
    app.update(Msg::Status(status(state)));
    settle(&mut app, size);
    app
}

/// Plays frames until every effect has ended.
fn settle(app: &mut App, size: (u16, u16)) {
    for _ in 0..200 {
        if !app.animating() {
            return;
        }
        tui::render_fx(app, size.0, size.1, FRAME * 4);
    }
    panic!("an effect never ended");
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

// ---- tabs glide ------------------------------------------------------------------

#[test]
fn a_tab_change_glides_in_and_ends_on_the_plain_frame() {
    let mut app = app_at(truecolor(), STANDARD, State::Online);
    key(&mut app, '3');
    assert!(app.animating(), "a tab change starts an effect");
    assert!(app.next_tick().is_some(), "and asks for frames");

    let plain = tui::render(&app, STANDARD.0, STANDARD.1);
    let first = tui::render_fx(&mut app, STANDARD.0, STANDARD.1, Duration::ZERO);
    let mid = tui::render_fx(&mut app, STANDARD.0, STANDARD.1, FRAME * 4);
    assert_ne!(
        styled_of(&first),
        styled_of(&plain),
        "the first frame is mid-glide"
    );
    assert_ne!(styled_of(&mid), styled_of(&plain), "so is the middle");

    let done = tui::render_fx(&mut app, STANDARD.0, STANDARD.1, LONG);
    assert_eq!(
        text_of(&done),
        text_of(&plain),
        "the glide lands on the real frame"
    );
    assert!(!app.animating());
    assert!(app.next_tick().is_none(), "nothing asks for another frame");
}

#[test]
fn a_key_that_changes_no_tab_starts_no_effect() {
    let mut app = app_at(truecolor(), STANDARD, State::Online);
    key(&mut app, '1'); // already on the Overview
    assert!(!app.animating());
    key(&mut app, 'j');
    assert!(!app.animating());
}

// ---- joined bloom, connecting pulse -----------------------------------------------

#[test]
fn coming_online_blooms_once_and_settles() {
    let mut app = app_at(truecolor(), STANDARD, State::Online);
    app.update(Msg::Happened(ActivityEntry {
        at: 1,
        kind: ActivityKind::Online,
        text: "Online".into(),
    }));
    assert!(app.animating(), "the join blooms");
    let plain = tui::render(&app, STANDARD.0, STANDARD.1);
    let start = tui::render_fx(&mut app, STANDARD.0, STANDARD.1, Duration::ZERO);
    assert_ne!(
        styled_of(&start),
        styled_of(&plain),
        "the bloom recolours the surface"
    );
    settle(&mut app, STANDARD);
}

#[test]
fn connecting_pulses_until_the_state_changes() {
    let mut app = app_at(truecolor(), STANDARD, State::Offline);
    app.update(Msg::Status(status(State::Connecting)));
    assert!(app.animating(), "the focal element pulses while connecting");

    // The pulse breathes: frames a half beat apart differ, and it never ends
    // by itself.
    let a = tui::render_fx(&mut app, STANDARD.0, STANDARD.1, Duration::ZERO);
    let b = tui::render_fx(&mut app, STANDARD.0, STANDARD.1, Duration::from_millis(650));
    assert_ne!(styled_of(&a), styled_of(&b));
    for _ in 0..40 {
        tui::render_fx(&mut app, STANDARD.0, STANDARD.1, Duration::from_millis(90));
    }
    assert!(app.animating());

    // A repeat of the same state changes nothing.
    app.update(Msg::Status(status(State::Connecting)));
    assert!(app.animating());

    // Connected: it settles and stops asking for frames.
    app.update(Msg::Status(status(State::Online)));
    settle(&mut app, STANDARD);
    assert!(app.next_tick().is_none());
}

// ---- errors shake once, toasts slide ----------------------------------------------

struct Rig {
    world: World,
    app: App,
    rx: Receiver<Msg>,
    _client: Client,
}

impl Rig {
    fn busy(caps: Caps, size: (u16, u16)) -> Rig {
        let world = World::start(NEXUS);
        wifi_busy(&world);
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(caps);
        app.attach(world.socket(), tx.clone());
        let client = Client::spawn(world.socket(), tx);
        app.update(Msg::Resize(size.0, size.1));
        let mut rig = Rig {
            world,
            app,
            rx,
            _client: client,
        };
        // Settled: the scenario's join is visible, and no late list is coming.
        loop {
            rig.pump_until("the Wi-Fi list", |a| {
                a.wifi().len() == 8 && a.wifi().rows()[0].joined() && a.status().is_some()
            });
            while let Ok(msg) = rig.rx.recv_timeout(Duration::from_millis(400)) {
                rig.app.update(msg);
            }
            if rig.app.wifi().rows()[0].joined() {
                break;
            }
        }
        settle(&mut rig.app, size);
        rig
    }

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

    fn select(&mut self, ssid: &str) {
        self.key(KeyCode::Char('2'));
        self.key(KeyCode::Char('g'));
        for _ in 0..self.app.wifi().len() {
            if self.app.wifi().selected().map(|e| e.name()) == Some(ssid) {
                return;
            }
            self.key(KeyCode::Char('j'));
        }
        panic!("no row {ssid}");
    }
}

#[test]
fn a_failed_join_shakes_its_toast_once_and_it_settles() {
    let mut rig = Rig::busy(truecolor(), STANDARD);
    // Cafe Guest is open: the join dies in activation (state 60, reason 2).
    rig.world.nm.script_join_failure("Cafe Guest", 60, 2);
    rig.select("Cafe Guest");
    settle(&mut rig.app, STANDARD);
    rig.key(KeyCode::Enter);
    rig.pump_until("the problem toast", |a| a.toast().is_some_and(|t| t.bad));

    assert!(rig.app.animating(), "an error toast moves");
    let (w, h) = STANDARD;
    // Frames across the effect: the toast fades in (colour only, so a
    // transparent terminal stays clear), then its columns shake.
    let mut texts = std::collections::BTreeSet::new();
    for _ in 0..12 {
        let frame = tui::render_fx(&mut rig.app, w, h, Duration::from_millis(40));
        texts.insert(text_of(&frame));
    }
    assert!(texts.len() > 1, "the toast shook: {} frames", texts.len());
    settle(&mut rig.app, STANDARD);
    let plain = tui::render(&rig.app, w, h);
    let after = tui::render_fx(&mut rig.app, w, h, LONG);
    assert_eq!(text_of(&after), text_of(&plain), "it ends in place");
    assert!(
        text_of(&plain).contains("Problem"),
        "the toast stays legible:\n{}",
        text_of(&plain)
    );
}

// ---- reduced motion and tiers ------------------------------------------------------

#[test]
fn reduced_motion_moves_nothing_but_keeps_state_changes_legible() {
    let mut app = App::new(truecolor());
    app.reduce_motion(true);
    assert_eq!(app.movement(), Movement::Reduced);
    app.update(Msg::Resize(STANDARD.0, STANDARD.1));
    app.update(Msg::Status(status(State::Online)));

    // No glide.
    key(&mut app, '3');
    assert!(!app.animating(), "tabs change at once");
    // No pulse.
    app.update(Msg::Status(status(State::Connecting)));
    assert!(!app.animating(), "no pulsing while connecting");
    assert!(
        text_of(&tui::render(&app, STANDARD.0, STANDARD.1)).contains("Connecting"),
        "the state is still said in words"
    );
    // The bloom's colour change stays, without the spreading: one fade.
    app.update(Msg::Status(status(State::Online)));
    app.update(Msg::Happened(ActivityEntry {
        at: 1,
        kind: ActivityKind::Online,
        text: "Online".into(),
    }));
    assert!(app.animating());
    let plain = tui::render(&app, STANDARD.0, STANDARD.1);
    let start = tui::render_fx(&mut app, STANDARD.0, STANDARD.1, Duration::ZERO);
    assert_eq!(
        text_of(&start),
        text_of(&plain),
        "colour only: no cell moves or disappears"
    );
    settle(&mut app, STANDARD);
}

#[test]
fn the_motion_environment_switch() {
    let on = |pairs: &'static [(&'static str, &'static str)]| {
        tui::reduced_motion(move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_string())
        })
    };
    assert!(!on(&[]));
    assert!(on(&[("NEXUS_REDUCE_MOTION", "1")]));
    assert!(on(&[("REDUCE_MOTION", "yes")]));
    assert!(!on(&[("NEXUS_REDUCE_MOTION", "0")]));
    assert!(!on(&[("NEXUS_REDUCE_MOTION", "")]));
}

#[test]
fn sixteen_colours_and_no_colour_have_no_effects_only_their_static_frames() {
    for tier in [ColorTier::Ansi16, ColorTier::None] {
        let mut app = app_at(caps(tier, Glyphs::Unicode), STANDARD, State::Online);
        assert_eq!(app.movement(), Movement::Off);
        key(&mut app, '3');
        assert!(!app.animating(), "{tier:?}: a tab change is instant");
        app.update(Msg::Status(status(State::Connecting)));
        assert!(!app.animating(), "{tier:?}: no pulse");
    }
}

#[test]
fn on_256_colours_effects_blend_into_palette_indexes_only() {
    let mut app = app_at(
        caps(ColorTier::Ansi256, Glyphs::Unicode),
        STANDARD,
        State::Online,
    );
    key(&mut app, '3');
    assert!(app.animating());
    for step in [Duration::ZERO, FRAME * 3, FRAME * 6] {
        let frame = tui::render_fx(&mut app, STANDARD.0, STANDARD.1, step);
        for cell in frame.content() {
            assert!(
                !matches!(cell.fg, Color::Rgb(..)) && !matches!(cell.bg, Color::Rgb(..)),
                "a 256-colour terminal never sees RGB: {cell:?}"
            );
        }
    }
}

// ---- zero idle frames after effects ----------------------------------------------

/// ADR-0003: effects end, and then the loop draws nothing at all.
#[test]
fn no_frames_are_drawn_once_the_effects_have_finished() {
    let (tx, rx) = mpsc::channel();
    let frames = Arc::new(AtomicUsize::new(0));
    let animating = Arc::new(Mutex::new(true));
    let looper = {
        let (frames, animating) = (frames.clone(), animating.clone());
        std::thread::spawn(move || {
            let mut terminal = Terminal::new(TestBackend::new(100, 28)).unwrap();
            let mut app = App::new(truecolor());
            tui::event_loop(&mut terminal, &mut app, &rx, |app| {
                frames.fetch_add(1, Ordering::SeqCst);
                *animating.lock().unwrap() = app.animating();
            })
            .unwrap();
        })
    };
    tx.send(Msg::Resize(100, 28)).unwrap();
    tx.send(Msg::Status(status(State::Online))).unwrap();
    // A tab change, then a join bloom: two effects.
    key_msg(&tx, '3');
    tx.send(Msg::Happened(ActivityEntry {
        at: 1,
        kind: ActivityKind::Online,
        text: "Online".into(),
    }))
    .unwrap();

    // Wait, polling, for the frame on which the last effect ended.
    let deadline = Instant::now() + PATIENCE;
    loop {
        let (n, busy) = (frames.load(Ordering::SeqCst), *animating.lock().unwrap());
        if n > 3 && !busy {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "effects never ended ({n} frames)"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let settled = frames.load(Ordering::SeqCst);

    // Longer than any effect and the 6 s toast's neighbours: proving a negative.
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(
        frames.load(Ordering::SeqCst),
        settled,
        "frames drawn after the effects finished"
    );
    key_msg(&tx, 'q');
    looper.join().unwrap();
}

fn key_msg(tx: &mpsc::Sender<Msg>, c: char) {
    tx.send(Msg::Key(KeyEvent::new(
        KeyCode::Char(c),
        KeyModifiers::NONE,
    )))
    .unwrap();
}

// ---- premium surfaces ---------------------------------------------------------------

fn border_cells(buf: &Buffer) -> Vec<(u16, u16, Color)> {
    let mut out = Vec::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let c = &buf[(x, y)];
            if matches!(c.symbol(), "╭" | "╮" | "╰" | "╯" | "─" | "│") && c.fg != Color::Reset
            {
                out.push((x, y, c.fg));
            }
        }
    }
    out
}

#[test]
fn truecolor_borders_run_a_gradient() {
    let app = app_at(truecolor(), STANDARD, State::Online);
    let buf = tui::render(&app, STANDARD.0, STANDARD.1);
    let focused: std::collections::BTreeSet<_> = border_cells(&buf)
        .into_iter()
        .map(|(_, _, c)| color_name(c))
        .collect();
    assert!(
        focused.len() > 6,
        "the focused border shifts hue across the screen: {focused:?}"
    );
}

#[test]
fn a_transparent_terminal_shows_through_everywhere_but_the_selection() {
    // kitty's background_opacity, foot's alpha and compositor blur only apply
    // to cells left on the default background: any other cell is opaque. The
    // selection tint (the active tab and the selected row) is the one exception.
    for state in [State::Online, State::Offline] {
        let app = app_at(truecolor(), STANDARD, state);
        let buf = tui::render(&app, STANDARD.0, STANDARD.1);
        let painted: Vec<(u16, u16, Color)> = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .filter_map(|(x, y)| {
                let bg = buf[(x, y)].bg;
                (bg != Color::Reset).then_some((x, y, bg))
            })
            .collect();
        let colours: std::collections::BTreeSet<_> =
            painted.iter().map(|(_, _, c)| color_name(*c)).collect();
        let rows: std::collections::BTreeSet<_> = painted.iter().map(|(_, y, _)| *y).collect();
        assert!(
            colours.len() <= 1 && rows.len() <= 3,
            "{state:?}: only the selection may paint a background, \
             got {colours:?} on rows {rows:?}"
        );
    }
}

#[test]
fn the_other_tiers_have_their_own_designed_borders() {
    // 256 colours: the same gradient, stepped to palette indexes, no glow.
    let app = app_at(
        caps(ColorTier::Ansi256, Glyphs::Unicode),
        STANDARD,
        State::Online,
    );
    let buf = tui::render(&app, STANDARD.0, STANDARD.1);
    let steps: std::collections::BTreeSet<_> = border_cells(&buf)
        .into_iter()
        .map(|(_, _, c)| color_name(c))
        .collect();
    assert!(steps.len() > 1, "{steps:?}");
    assert!(
        steps.iter().all(|s| s.starts_with("idx") || s == "-"),
        "{steps:?}"
    );
    assert!(
        buf.content()
            .iter()
            .all(|c| !matches!(c.fg, Color::Rgb(..)) && !matches!(c.bg, Color::Rgb(..))),
        "no RGB on a 256-colour terminal"
    );

    // 16 colours: the named border colours (focused and plain), no glow.
    let app = app_at(
        caps(ColorTier::Ansi16, Glyphs::Unicode),
        STANDARD,
        State::Online,
    );
    let buf = tui::render(&app, STANDARD.0, STANDARD.1);
    let one: std::collections::BTreeSet<_> = border_cells(&buf)
        .into_iter()
        .map(|(_, _, c)| color_name(c))
        .collect();
    assert!(
        one.iter()
            .all(|c| matches!(c.as_str(), "cyan" | "lightcyan")),
        "named colours only, no gradient: {one:?}"
    );
    assert!(
        buf.content()
            .iter()
            .all(|c| !matches!(c.bg, Color::Rgb(..)))
    );
}

#[test]
fn final_frames_per_tier_and_width() {
    let tiers: [(&str, Caps); 5] = [
        ("truecolor", caps(ColorTier::TrueColor, Glyphs::Unicode)),
        ("256", caps(ColorTier::Ansi256, Glyphs::Unicode)),
        ("16", caps(ColorTier::Ansi16, Glyphs::Unicode)),
        ("none", caps(ColorTier::None, Glyphs::Unicode)),
        ("ascii", caps(ColorTier::TrueColor, Glyphs::Ascii)),
    ];
    for (tier, c) in tiers {
        for (name, size) in SIZES {
            let mut app = app_at(c, size, State::Online);
            key(&mut app, '3');
            key(&mut app, '1'); // back on the Overview, hero and all
            settle(&mut app, size);
            let buf = tui::render_fx(&mut app, size.0, size.1, LONG);
            // Narrow and wide get text only; the standard frame carries the
            // style table, which is what tells the tiers apart.
            let frame = if name == "standard" {
                styled_of(&buf)
            } else {
                text_of(&buf)
            };
            insta::assert_snapshot!(format!("final_{tier}_{name}"), frame);
        }
    }
}

// ---- overlays leave their neighbours alone ------------------------------------------

/// The cells of `buf` inside the body rows (between header and footer) and
/// outside `except`, as text.
fn body_outside(buf: &Buffer, except: Rect) -> Vec<(u16, u16, String)> {
    let mut out = Vec::new();
    for y in 1..buf.area.height - 1 {
        for x in 0..buf.area.width {
            if !except.contains((x, y).into()) {
                out.push((x, y, buf[(x, y)].symbol().to_string()));
            }
        }
    }
    out
}

/// The rectangle an overlay drew: the cells that changed.
fn changed_rect(before: &Buffer, after: &Buffer) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (u16::MAX, u16::MAX, 0, 0);
    for y in 1..before.area.height - 1 {
        for x in 0..before.area.width {
            if before[(x, y)].symbol() != after[(x, y)].symbol() {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    assert!(x0 != u16::MAX, "the overlay changed nothing");
    Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1)
}

#[test]
fn the_palette_covers_only_its_own_box_and_never_the_list() {
    for (name, size) in [("standard", STANDARD), ("wide", WIDE)] {
        let mut rig = Rig::busy(truecolor(), size);
        rig.select("Home-5G");
        let before = tui::render(&rig.app, size.0, size.1);
        rig.key(KeyCode::Char(':'));
        let after = tui::render(&rig.app, size.0, size.1);
        let area = changed_rect(&before, &after);
        let list_width = if name == "wide" { 40 } else { 30 };
        assert!(
            area.x >= list_width,
            "{name}: the palette starts at column {} inside the {list_width}-column list",
            area.x
        );
        assert_eq!(
            body_outside(&before, area),
            body_outside(&after, area),
            "{name}: something outside the palette's box changed"
        );
        // The list's band column survives whole.
        let text = text_of(&after);
        assert!(
            name == "standard" || text.contains("5G │") || text.contains("5G│"),
            "{text}"
        );
    }
}

#[test]
fn the_share_dialog_covers_only_its_own_box_and_never_the_list() {
    for (name, size) in [("standard", STANDARD), ("wide", WIDE)] {
        let mut rig = Rig::busy(truecolor(), size);
        rig.select("Home-5G");
        let before = tui::render(&rig.app, size.0, size.1);
        rig.key(KeyCode::Char('s'));
        let after = tui::render(&rig.app, size.0, size.1);
        let area = changed_rect(&before, &after);
        let list_width = if name == "wide" { 40 } else { 30 };
        assert!(
            area.x >= list_width,
            "{name}: the dialog starts at column {} inside the list",
            area.x
        );
    }
}
