//! Seam S2 for the Wi-Fi tab: keystrokes drive the TUI, which drives a real
//! Daemon over the socket (fake NetworkManager behind it); `insta` frames
//! come out. The scenarios are the testkit's, so `demo wifi` shows the same.

use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_testkit::scenarios::{wifi_busy, wifi_with_open_prompt};
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
        let wlan0 = wifi_busy(&world);
        let mut rig = Rig::start(world, Some(wlan0), caps);
        rig.key(KeyCode::Char('2'));
        // Settled: the scenario's join is visible, so no late list moves a frame.
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

    /// Drops the scenario's link and waits for it to show, so a later join
    /// cannot change the Home-5G row halfway through a snapshot.
    fn drop_link(&mut self) {
        self.select("Home-5G");
        self.key(KeyCode::Char('x'));
        self.pump_until("the link to drop", |a| {
            a.toast().is_some()
                && a.wifi().rows()[0].name() == "Home-5G"
                && !a.wifi().rows()[0].joined()
        });
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

    fn names(&self) -> Vec<String> {
        self.app
            .wifi()
            .rows()
            .iter()
            .map(|e| e.name().to_owned())
            .collect()
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

    fn footer(&self, size: (u16, u16)) -> String {
        self.text(size).lines().last().unwrap().to_string()
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

// ---- the list, at every width -------------------------------------------------

#[test]
fn the_list_shows_bars_security_band_and_a_saved_section() {
    let rig = Rig::busy();
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Saved, out of range"), "{frame}");
    assert!(frame.contains("Parents House") && frame.contains("Hotel Lobby"));
    insta::assert_snapshot!("wifi_standard", frame);
}

#[test]
fn the_list_at_wide_width_adds_the_band() {
    let rig = Rig::busy();
    let frame = rig.text(WIDE);
    assert!(frame.contains("5G") && frame.contains("2.4G"), "{frame}");
    insta::assert_snapshot!("wifi_wide", frame);
}

#[test]
fn the_list_at_narrow_width_is_one_list_and_d_opens_the_sheet() {
    let mut rig = Rig::busy();
    rig.size(NARROW);
    insta::assert_snapshot!("wifi_narrow", rig.text(NARROW));
    rig.key(KeyCode::Char('d'));
    let sheet = rig.text(NARROW);
    assert!(
        sheet.contains("Connected") && sheet.contains("Access points"),
        "{sheet}"
    );
    insta::assert_snapshot!("wifi_sheet_narrow", sheet);
    rig.key(KeyCode::Esc);
    assert!(!rig.text(NARROW).contains("Access points"));
}

#[test]
fn the_detail_pane_describes_the_selected_network() {
    let mut rig = Rig::busy();
    rig.select("Work-Secure");
    let frame = rig.text(STANDARD);
    assert!(
        frame.contains("WPA3 personal") && frame.contains("5 GHz"),
        "{frame}"
    );
    assert!(
        frame.contains("You will be asked for the password"),
        "{frame}"
    );
    insta::assert_snapshot!("wifi_detail_secured", frame);
    rig.select("Parents House");
    let frame = rig.text(STANDARD);
    assert!(
        frame.contains("Saved, out of range") && frame.contains("Autoconnect"),
        "{frame}"
    );
    insta::assert_snapshot!("wifi_detail_saved", frame);
}

#[test]
fn colour_tiers_each_have_a_wifi_frame() {
    let tiers = [
        ("truecolor", ColorTier::TrueColor, Glyphs::Unicode),
        ("16", ColorTier::Ansi16, Glyphs::Unicode),
        ("none", ColorTier::None, Glyphs::Unicode),
        ("ascii", ColorTier::Ansi256, Glyphs::Ascii),
    ];
    for (name, color, glyphs) in tiers {
        let mut rig = Rig::busy_with(Caps { color, glyphs });
        rig.select("Neighbour-2G");
        let frame = rig.styled(STANDARD);
        if glyphs == Glyphs::Ascii {
            assert!(frame.is_ascii(), "ascii tier leaked non-ascii:\n{frame}");
        }
        insta::assert_snapshot!(format!("wifi_tier_{name}"), frame);
    }
}

#[test]
fn without_an_adapter_the_tab_says_so() {
    let world = World::start(NEXUS);
    let mut rig = Rig::start(world, None, truecolor());
    rig.key(KeyCode::Char('2'));
    rig.pump_until("the empty list", |a| a.wifi().has_list());
    let frame = rig.text(STANDARD);
    assert!(frame.contains("No Wi-Fi adapter found."), "{frame}");
}

// ---- the order never moves under the cursor -------------------------------------

#[test]
fn signal_changes_never_reshuffle_the_list_or_move_the_cursor_off_its_row() {
    let mut rig = Rig::busy();
    let before = rig.names();
    rig.select("Neighbour-2G");
    rig.world.nm.set_strength_of("Neighbour-2G", 99);
    rig.pump_until("the new signal", |a| {
        a.wifi().signal_of("Neighbour-2G") == Some(99)
    });
    assert_eq!(rig.names(), before, "same order despite the new signal");
    assert_eq!(rig.selected(), "Neighbour-2G");
}

#[test]
fn a_network_that_appears_goes_to_the_end_of_its_section() {
    let mut rig = Rig::busy();
    rig.select("Cafe Guest");
    rig.world.nm.add_ap(
        rig.dev(),
        nexus_testkit::Ap::wpa2("Brand New", "AA:00:00:00:00:77", 97),
    );
    rig.pump_until("the new Network", |a| a.wifi().len() == 9);
    let names = rig.names();
    assert_eq!(names[6], "Brand New", "{names:?}");
    assert_eq!(
        names[7], "Parents House",
        "saved ones stay below: {names:?}"
    );
    assert_eq!(rig.selected(), "Cafe Guest");
}

#[test]
fn a_rescan_is_when_the_order_is_redone_and_the_cursor_still_follows_its_row() {
    let mut rig = Rig::busy();
    rig.world.nm.set_last_scan_ago(rig.dev(), 60_000);
    rig.select("Cafe Guest");
    rig.key(KeyCode::Char('r'));
    rig.world.nm.set_strength_of("Rooftop-6E", 99);
    rig.pump_until("the resorted list", |a| {
        a.wifi().rows()[0].name() == "Rooftop-6E"
    });
    assert_eq!(rig.selected(), "Cafe Guest");
}

// ---- scanning -------------------------------------------------------------------------

#[test]
fn a_rescan_shows_the_sweep_until_the_new_list_arrives() {
    let mut rig = Rig::busy();
    rig.world.nm.set_last_scan_ago(rig.dev(), 60_000);
    assert_eq!(rig.app.next_tick(), None, "nothing animates while idle");
    rig.key(KeyCode::Char('r'));
    let frame = rig.text(STANDARD);
    assert!(frame.contains("scanning"), "{frame}");
    insta::assert_snapshot!("wifi_scanning", frame);
    assert!(rig.app.next_tick().is_some(), "the sweep asks for frames");

    assert!(rig.app.update(Msg::Tick), "a tick moves the sweep");
    assert_ne!(rig.text(STANDARD), frame);

    rig.world.nm.set_strength_of("Neighbour-2G", 40);
    rig.pump_until("the answer and the new list", |a| {
        a.wifi().signal_of("Neighbour-2G") == Some(40) && a.wifi().scan_acked()
    });
    assert!(rig.app.wifi().scanning(), "results may still be coming");
    for _ in 0..20 {
        rig.app.update(Msg::Tick);
    }
    assert!(!rig.text(STANDARD).contains("scanning"));
    assert_eq!(rig.app.next_tick(), None);
}

#[test]
fn a_rescan_inside_the_rate_limit_says_the_list_is_fresh() {
    let mut rig = Rig::busy();
    rig.world.nm.set_last_scan_ago(rig.dev(), 100);
    rig.key(KeyCode::Char('r'));
    rig.pump_until("the answer", |a| a.toast().is_some());
    assert!(
        rig.toast().contains("Scanned a moment ago"),
        "{}",
        rig.toast()
    );
    assert!(!rig.app.wifi().scanning());
    assert_eq!(rig.world.nm.scan_requests(), 0);
}

// ---- joining ----------------------------------------------------------------------------

#[test]
fn enter_joins_an_open_network_and_the_list_keeps_its_order() {
    let mut rig = Rig::busy();
    let before = rig.names();
    rig.select("Cafe Guest");
    rig.key(KeyCode::Enter);
    assert!(rig.text(STANDARD).contains("Joining"), "busy shows at once");
    rig.pump_until("the join", |a| a.toast().is_some());
    assert_eq!(rig.toast(), "Joined Cafe Guest.");
    assert_eq!(
        rig.world.nm.joined_ssid(rig.dev()).as_deref(),
        Some("Cafe Guest")
    );
    rig.pump_until("the joined marker", |a| {
        a.wifi().selected().is_some_and(|e| e.joined())
    });
    assert_eq!(rig.names(), before);
}

#[test]
fn a_secured_network_asks_for_its_password_inline_masked_with_a_reveal_key() {
    let mut rig = Rig::busy();
    rig.drop_link();
    rig.select("Work-Secure");
    rig.key(KeyCode::Enter);
    rig.pump_until("the prompt", |a| a.field().is_some());
    rig.type_str("hunter22");
    let masked = rig.text(STANDARD);
    assert!(!masked.contains("hunter22"), "{masked}");
    assert!(masked.contains("••••••••"), "{masked}");
    insta::assert_snapshot!("wifi_prompt_masked", masked);
    insta::assert_snapshot!("wifi_prompt_masked_narrow", {
        rig.size(NARROW);
        let narrow = rig.text(NARROW);
        rig.size(STANDARD);
        narrow
    });

    rig.ctrl('r');
    let shown = rig.text(STANDARD);
    assert!(shown.contains("hunter22"), "{shown}");
    insta::assert_snapshot!("wifi_prompt_revealed", shown);
    rig.ctrl('r');
    assert!(!rig.text(STANDARD).contains("hunter22"));

    rig.key(KeyCode::Backspace);
    rig.type_str("2");
    rig.key(KeyCode::Enter);
    assert!(rig.app.field().is_none(), "the field is gone once answered");
    rig.pump_until("the join", |a| a.toast().is_some());
    assert_eq!(rig.toast(), "Joined Work-Secure.");
    assert_eq!(
        rig.world.nm.joined_ssid(rig.dev()).as_deref(),
        Some("Work-Secure")
    );
}

#[test]
fn a_wrong_password_reopens_the_field_with_the_message() {
    let mut rig = Rig::busy();
    rig.drop_link();
    rig.world.nm.set_psk("Work-Secure", "correct-horse");
    rig.select("Work-Secure");
    rig.key(KeyCode::Enter);
    rig.pump_until("the prompt", |a| a.field().is_some());
    rig.type_str("battery-staple");
    rig.key(KeyCode::Enter);
    rig.pump_until("the prompt again", |a| {
        a.field().is_some_and(|f| f.prompt.retry)
    });
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Wrong password"), "{frame}");
    assert_eq!(rig.app.field().unwrap().text(), "", "starts empty again");
    insta::assert_snapshot!("wifi_prompt_retry", frame);

    rig.type_str("correct-horse");
    rig.key(KeyCode::Enter);
    rig.pump_until("the join", |a| a.toast().is_some());
    assert_eq!(rig.toast(), "Joined Work-Secure.");
}

#[test]
fn esc_cancels_the_prompt_and_leaves_nothing_saved() {
    let mut rig = Rig::busy();
    let profiles = rig.world.nm.profile_count();
    rig.select("Work-Secure");
    rig.key(KeyCode::Enter);
    rig.pump_until("the prompt", |a| a.field().is_some());
    rig.type_str("abc");
    rig.key(KeyCode::Esc);
    assert!(rig.app.field().is_none());
    rig.pump_until("the failure", |a| a.toast().is_some());
    assert!(rig.toast().contains("Work-Secure"), "{}", rig.toast());
    assert_eq!(rig.world.nm.profile_count(), profiles);
    assert!(!rig.app.should_quit());
}

#[test]
fn a_prompt_that_opened_before_the_tui_started_is_there_on_any_tab() {
    let world = World::start(NEXUS);
    let join = wifi_with_open_prompt(&world);
    let mut rig = Rig::start(world, None, truecolor());
    rig.pump_until("the replayed prompt", |a| a.field().is_some());
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Join Work-Secure"), "{frame}");
    insta::assert_snapshot!("wifi_prompt_on_overview", frame);
    assert_eq!(
        rig.footer(STANDARD).trim(),
        "enter join  ctrl-r show  esc cancel"
    );

    rig.type_str("hunter22");
    rig.key(KeyCode::Enter);
    join.join().expect("the waiting join ends once answered");
}

// ---- disconnect, Forget, autoconnect, undo --------------------------------------------------------

#[test]
fn disconnect_shows_a_toast_and_undo_joins_again() {
    let mut rig = Rig::busy();
    assert_eq!(rig.selected(), "Home-5G");
    rig.key(KeyCode::Char('x'));
    rig.pump_until("the disconnect", |a| a.toast().is_some());
    assert_eq!(rig.toast(), "Disconnected from Home-5G.");
    assert_eq!(rig.world.nm.joined_ssid(rig.dev()), None);
    assert!(rig.footer(STANDARD).contains("u undo"));
    rig.pump_until("the joined marker to go", |a| {
        a.wifi().selected().is_some_and(|e| !e.joined())
    });
    insta::assert_snapshot!("wifi_disconnected_toast", rig.text(STANDARD));

    rig.key(KeyCode::Char('u'));
    rig.pump_until("the rejoin", |a| {
        a.toast().is_some_and(|t| t.text.starts_with("Joined"))
    });
    assert_eq!(
        rig.world.nm.joined_ssid(rig.dev()).as_deref(),
        Some("Home-5G")
    );
}

#[test]
fn forget_asks_by_name_and_only_a_yes_deletes() {
    let mut rig = Rig::busy();
    let profiles = rig.world.nm.profile_count();
    rig.select("Parents House");
    rig.key(KeyCode::Char('f'));
    let frame = rig.text(STANDARD);
    assert!(frame.contains("Forget Parents House?"), "{frame}");
    insta::assert_snapshot!("wifi_forget_confirm", frame);
    assert_eq!(rig.footer(STANDARD).trim(), "y forget  esc keep");

    rig.key(KeyCode::Enter);
    assert!(rig.app.confirm().is_some(), "Enter is not a yes");
    rig.key(KeyCode::Esc);
    assert!(rig.app.confirm().is_none());
    assert_eq!(rig.world.nm.profile_count(), profiles);

    rig.key(KeyCode::Char('f'));
    rig.key(KeyCode::Char('y'));
    rig.pump_until("the forget", |a| a.toast().is_some());
    assert_eq!(rig.toast(), "Forgot Parents House.");
    assert_eq!(rig.world.nm.profile_count(), profiles - 1);
    rig.pump_until("the shorter list", |a| a.wifi().len() == 7);
    assert!(!rig.names().contains(&"Parents House".to_string()));
}

#[test]
fn forget_is_not_offered_for_a_network_that_is_not_saved() {
    let mut rig = Rig::busy();
    rig.select("Cafe Guest");
    rig.key(KeyCode::Char('f'));
    assert!(rig.app.confirm().is_none());
    assert!(!rig.footer(STANDARD).contains("forget"));
}

#[test]
fn autoconnect_toggles_with_a_toast_and_undo() {
    let mut rig = Rig::busy();
    rig.drop_link();
    rig.select("Parents House");
    rig.key(KeyCode::Char('a'));
    rig.pump_until("the toggle", |a| a.toast().is_some());
    assert_eq!(rig.toast(), "Parents House will not join automatically.");
    assert_eq!(
        rig.world.nm.profile_autoconnect("uuid-parents"),
        Some(false)
    );
    insta::assert_snapshot!("wifi_autoconnect_toast", rig.text(STANDARD));

    rig.key(KeyCode::Char('u'));
    rig.pump_until("the undo", |a| {
        a.toast()
            .is_some_and(|t| t.text.contains("will join automatically"))
    });
    assert_eq!(rig.world.nm.profile_autoconnect("uuid-parents"), Some(true));
}

#[test]
fn autoconnect_works_on_a_saved_network_that_is_in_range_too() {
    let mut rig = Rig::busy();
    assert_eq!(rig.selected(), "Home-5G");
    rig.key(KeyCode::Char('a'));
    rig.pump_until("the detail to say off", |a| {
        text_of(&tui::render(a, STANDARD.0, STANDARD.1)).contains("Autoconnect   off")
    });
}

#[test]
fn footer_keys_follow_the_selected_row() {
    let mut rig = Rig::busy();
    let joined = rig.footer(STANDARD);
    assert!(
        joined.contains("x disconnect") && joined.contains("f forget"),
        "{joined}"
    );
    assert!(!joined.contains("enter join"), "already joined: {joined}");
    rig.select("Cafe Guest");
    let open = rig.footer(STANDARD);
    assert!(
        open.contains("enter join") && !open.contains("forget"),
        "{open}"
    );
    rig.size(NARROW);
    let narrow = rig.footer(NARROW);
    assert!(
        narrow.contains("enter join") && narrow.contains("r scan"),
        "{narrow}"
    );
}

#[test]
fn the_help_overlay_lists_the_wifi_keys() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('?'));
    let help = rig.text(STANDARD);
    assert!(help.contains("Forget (asks first)"), "{help}");
}
