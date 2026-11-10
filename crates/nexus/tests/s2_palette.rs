//! Seam S2 for the command palette (`:`), the scrolling help (`?`) and the
//! mouse: keystrokes and mouse events drive the TUI, which drives a real
//! Daemon over the socket (fake NetworkManager behind it); `insta` frames
//! come out at narrow, standard and wide widths.

use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use nexus_testkit::scenarios::wifi_busy;
use nexus_testkit::{PATIENCE, World};
use nexus_tui::tui::{self, App, Caps, Client, ColorTier, Glyphs, Layer, Msg, Tab};
use ratatui::buffer::Buffer;

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

const NARROW: (u16, u16) = (60, 20);
const STANDARD: (u16, u16) = (100, 28);
const WIDE: (u16, u16) = (140, 32);
const SIZES: [(&str, (u16, u16)); 3] = [("narrow", NARROW), ("standard", STANDARD), ("wide", WIDE)];

struct Rig {
    world: World,
    app: App,
    rx: Receiver<Msg>,
    size: (u16, u16),
    _client: Client,
}

impl Rig {
    /// The busy Wi-Fi scenario with the TUI settled on the Overview.
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
            size: STANDARD,
            _client: client,
        };
        rig.pump_until("the Wi-Fi list", |a| {
            a.wifi().len() == 8 && a.wifi().rows()[0].joined() && a.status().is_some()
        });
        rig
    }

    fn resize(&mut self, size: (u16, u16)) {
        self.size = size;
        self.app.update(Msg::Resize(size.0, size.1));
    }

    fn pump_until(&mut self, what: &str, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(&self.app) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| panic!("timed out waiting for {what}:\n{}", self.text()));
            let msg = self
                .rx
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("timed out waiting for {what}:\n{}", self.text()));
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

    /// Opens the palette and types `query`.
    fn palette(&mut self, query: &str) {
        self.key(KeyCode::Char(':'));
        self.type_str(query);
    }

    fn buffer(&self) -> Buffer {
        tui::render(&self.app, self.size.0, self.size.1)
    }

    fn text(&self) -> String {
        text_of(&self.buffer())
    }

    fn mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) -> bool {
        self.app.update(Msg::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }))
    }

    /// Where `needle` first appears on screen, as (column, row) of its start.
    fn find(&self, needle: &str) -> (u16, u16) {
        self.find_from(needle, 0)
    }

    /// Like [`Rig::find`], skipping the rows above `from`.
    fn find_from(&self, needle: &str, from: usize) -> (u16, u16) {
        let buf = self.buffer();
        let w = buf.area.width as usize;
        for (y, cells) in buf.content().chunks(w).enumerate().skip(from) {
            let mut line = String::new();
            let mut starts = Vec::new();
            for c in cells {
                starts.push(line.len());
                line.push_str(c.symbol());
            }
            if let Some(at) = line.find(needle) {
                let x = starts.iter().position(|s| *s == at).expect("on a cell");
                return (x as u16, y as u16);
            }
        }
        panic!("no {needle:?} on screen:\n{}", self.text());
    }

    /// Clicks on `needle` in the bottom row (the footer's buttons).
    fn click_footer(&mut self, needle: &str) -> bool {
        let (x, y) = self.find_from(needle, self.size.1 as usize - 1);
        self.mouse(MouseEventKind::Down(MouseButton::Left), x, y)
    }

    /// Draws (so the hit areas are current), then clicks on `needle`.
    fn click_on(&mut self, needle: &str) -> bool {
        let (x, y) = self.find(needle);
        self.mouse(MouseEventKind::Down(MouseButton::Left), x, y)
    }

    /// The selected Wi-Fi row's name.
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

    /// On the Wi-Fi tab with `ssid` selected.
    fn select(&mut self, ssid: &str) {
        self.key(KeyCode::Char('2'));
        self.key(KeyCode::Char('g'));
        for _ in 0..self.app.wifi().len() {
            if self.selected() == ssid {
                return;
            }
            self.key(KeyCode::Char('j'));
        }
        assert_eq!(self.selected(), ssid, "no such row");
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

// ---- the palette ----------------------------------------------------------------

#[test]
fn the_palette_lists_every_action_and_shows_its_key() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char(':'));
    let frame = rig.text();
    assert!(frame.contains("Command palette"), "{frame}");
    assert!(frame.contains("Go to Overview"), "{frame}");
    for (name, size) in SIZES {
        rig.resize(size);
        insta::assert_snapshot!(format!("palette_open_{name}"), rig.text());
    }
}

#[test]
fn typing_filters_by_plain_english_at_every_width() {
    let mut rig = Rig::busy();
    rig.select("Parents House");
    rig.palette("forget net");
    assert!(rig.text().contains("Forget network"));
    for (name, size) in SIZES {
        rig.resize(size);
        insta::assert_snapshot!(format!("palette_forget_{name}"), rig.text());
    }
}

#[test]
fn an_unavailable_action_is_dimmed_and_says_why() {
    let mut rig = Rig::busy();
    rig.palette("forget");
    let frame = rig.text();
    assert!(
        frame.contains("Open the Wi-Fi tab and pick a Network first."),
        "{frame}"
    );
    insta::assert_snapshot!("palette_blocked", frame);
    // Enter explains it in a toast instead of doing something odd.
    rig.key(KeyCode::Enter);
    assert!(rig.app.palette().is_none());
    assert!(rig.toast().contains("Wi-Fi tab"), "{}", rig.toast());
    assert_eq!(rig.app.tab(), Tab::Overview);
}

#[test]
fn no_match_says_so() {
    let mut rig = Rig::busy();
    rig.palette("zzzzzz");
    insta::assert_snapshot!("palette_no_match", rig.text());
}

#[test]
fn forget_network_from_the_palette_asks_first() {
    let mut rig = Rig::busy();
    rig.select("Parents House");
    rig.palette("forget network");
    rig.key(KeyCode::Enter);
    assert!(rig.app.palette().is_none());
    let frame = rig.text();
    assert!(
        frame.contains("Forget") && frame.contains("Parents House"),
        "{frame}"
    );
    assert!(rig.app.confirm().is_some());
    // Nothing is deleted until a yes.
    assert_eq!(rig.world.nm.profile_count(), 3);
    rig.key(KeyCode::Esc);
    assert_eq!(rig.world.nm.profile_count(), 3);
}

#[test]
fn share_wifi_from_the_palette_opens_the_share_view() {
    let mut rig = Rig::busy();
    rig.select("Parents House");
    rig.palette("share wi");
    rig.key(KeyCode::Enter);
    let frame = rig.text();
    assert!(frame.contains("Parents House"), "{frame}");
    assert!(
        frame.contains("Share") || frame.contains("share"),
        "{frame}"
    );
    // The secret is only read after the person says so.
    assert_eq!(rig.world.nm.secret_reads(), 0);
}

#[test]
fn rescan_works_from_any_tab() {
    let mut rig = Rig::busy();
    assert_eq!(rig.app.tab(), Tab::Overview);
    rig.palette("rescan");
    rig.key(KeyCode::Enter);
    assert!(rig.app.wifi().scanning());
}

#[test]
fn go_to_a_tab_by_name() {
    let mut rig = Rig::busy();
    rig.palette("go hotspot");
    rig.key(KeyCode::Enter);
    assert_eq!(rig.app.tab(), Tab::Hotspot);
}

#[test]
fn arrows_move_and_escape_closes_without_quitting() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char(':'));
    rig.key(KeyCode::Down);
    rig.key(KeyCode::Down);
    assert_eq!(rig.app.palette().unwrap().selected, 2);
    rig.key(KeyCode::Up);
    assert_eq!(rig.app.palette().unwrap().selected, 1);
    // `q` is text inside the palette, not quit.
    rig.type_str("q");
    assert!(!rig.app.should_quit());
    assert_eq!(rig.app.palette().unwrap().query, "q");
    rig.ctrl('u');
    assert_eq!(rig.app.palette().unwrap().query, "");
    rig.key(KeyCode::Esc);
    assert!(rig.app.palette().is_none());
    assert!(!rig.app.should_quit());
}

#[test]
fn the_palette_does_not_steal_a_password_typed_in_the_field() {
    let mut rig = Rig::busy();
    rig.select("Neighbour-2G");
    rig.key(KeyCode::Enter); // opens a Secret prompt through the Daemon
    rig.pump_until("the password field", |a| a.field().is_some());
    rig.key(KeyCode::Char(':'));
    assert!(rig.app.palette().is_none(), "':' belongs to the password");
    rig.key(KeyCode::Esc);
}

#[test]
fn the_palette_hint_is_in_the_footer() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('2'));
    for (_, size) in SIZES {
        rig.resize(size);
        let frame = rig.text();
        let footer = frame.lines().last().unwrap();
        assert!(footer.contains(": palette"), "{footer}");
    }
}

// ---- help that scrolls ----------------------------------------------------------

#[test]
fn help_never_clips_it_scrolls() {
    let mut rig = Rig::busy();
    for (name, size) in SIZES {
        rig.resize(size);
        rig.key(KeyCode::Char('?'));
        assert_eq!(rig.app.layer(), Layer::Help);
        let top = rig.text();
        assert!(top.contains("Open the command palette"), "{top}");
        insta::assert_snapshot!(format!("help_top_{name}"), top);
        rig.key(KeyCode::Char('G'));
        let bottom = rig.text();
        assert!(
            bottom.contains("plain http://"),
            "the note's last words:\n{bottom}"
        );
        insta::assert_snapshot!(format!("help_bottom_{name}"), bottom);
        rig.key(KeyCode::Esc);
        assert_eq!(rig.app.layer(), Layer::Root);
    }
}

#[test]
fn help_lists_what_the_palette_can_do() {
    let mut rig = Rig::busy();
    rig.resize((100, 60)); // tall enough to see it all at once
    rig.key(KeyCode::Char('?'));
    let frame = rig.text();
    for keys in [
        "Wi-Fi: f",
        "Wi-Fi: s",
        "Wi-Fi: r",
        "Wi-Fi: Enter",
        "o ",
        "q",
    ] {
        assert!(frame.contains(keys), "{keys} missing:\n{frame}");
    }
    assert!(
        !frame.contains("(j/k scroll)"),
        "it fits, so no scrolling:\n{frame}"
    );
}

#[test]
fn help_scrolls_by_key_page_and_wheel() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('?'));
    assert_eq!(rig.app.help_scroll(), 0);
    rig.key(KeyCode::Char('j'));
    assert_eq!(rig.app.help_scroll(), 1);
    rig.key(KeyCode::Char('k'));
    assert_eq!(rig.app.help_scroll(), 0);
    rig.key(KeyCode::PageDown);
    assert!(rig.app.help_scroll() > 1);
    let before = rig.app.help_scroll();
    rig.mouse(MouseEventKind::ScrollUp, 50, 10);
    let after_up = rig.app.help_scroll();
    assert!(after_up < before, "the wheel scrolls up");
    rig.mouse(MouseEventKind::ScrollDown, 50, 10);
    assert_eq!(rig.app.help_scroll(), before, "and back down");
    rig.key(KeyCode::End);
    let end = rig.app.help_scroll();
    rig.key(KeyCode::Down);
    assert_eq!(rig.app.help_scroll(), end, "stops at the end");
    rig.key(KeyCode::Home);
    assert_eq!(rig.app.help_scroll(), 0);
}

// ---- the mouse ------------------------------------------------------------------

#[test]
fn clicking_a_tab_label_switches_tabs() {
    let mut rig = Rig::busy();
    assert!(rig.click_on("Hotspot"));
    assert_eq!(rig.app.tab(), Tab::Hotspot);
    assert!(rig.click_on("Wi-Fi"));
    assert_eq!(rig.app.tab(), Tab::WiFi);
    assert!(!rig.click_on("Wi-Fi"), "already there: no redraw");
    insta::assert_snapshot!("mouse_tab_wifi_standard", rig.text());
}

#[test]
fn clicking_the_narrow_tab_name_steps_to_the_next_tab() {
    let mut rig = Rig::busy();
    rig.resize(NARROW);
    assert!(rig.click_on("Overview"));
    assert_eq!(rig.app.tab(), Tab::WiFi);
}

#[test]
fn clicking_an_overview_card_opens_its_tab() {
    let mut rig = Rig::busy();
    for size in [STANDARD, NARROW, WIDE] {
        rig.resize(size);
        rig.key(KeyCode::Char('1'));
        // Below the header, so the tab label of the same name is not hit.
        let (x, y) = rig.find_from("Hotspot", 1);
        assert!(rig.mouse(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(rig.app.tab(), Tab::Hotspot, "{size:?}");
    }
}

#[test]
fn clicking_a_wifi_row_selects_and_a_second_click_acts() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('2'));
    assert_eq!(rig.selected(), "Home-5G");
    assert!(rig.click_on("Cafe Guest"));
    assert_eq!(rig.selected(), "Cafe Guest");
    assert!(rig.app.wifi().busy.is_none(), "one click only selects");
    insta::assert_snapshot!("mouse_row_selected", rig.text());
    // The detail pane's title repeats the name: click the list row.
    let (x, y) = rig.find_from("Cafe Guest", 2);
    rig.mouse(MouseEventKind::Down(MouseButton::Left), x, y);
    assert_eq!(rig.app.wifi().busy.as_deref(), Some("Cafe Guest"));
    rig.pump_until("the join", |a| {
        a.wifi().busy.is_none()
            && a.wifi()
                .rows()
                .iter()
                .any(|e| e.name() == "Cafe Guest" && e.joined())
    });
}

#[test]
fn clicking_below_the_saved_rule_picks_the_right_row() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('2'));
    rig.click_on("Hotel Lobby");
    assert_eq!(rig.selected(), "Hotel Lobby");
    rig.click_on("Parents House");
    assert_eq!(rig.selected(), "Parents House");
}

#[test]
fn footer_hints_are_buttons() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('2'));
    assert!(!rig.app.wifi().scanning());
    rig.click_footer("r scan");
    assert!(rig.app.wifi().scanning());
    // The palette hint opens the palette; its own hints close it.
    rig.click_footer(": palette");
    assert!(rig.app.palette().is_some());
    rig.click_footer("esc close");
    assert!(rig.app.palette().is_none());
    // `? help` opens the help; `esc close` shuts it.
    rig.click_footer("? help");
    assert_eq!(rig.app.layer(), Layer::Help);
    rig.click_footer("esc close");
    assert_eq!(rig.app.layer(), Layer::Root);
}

#[test]
fn clicking_a_palette_match_runs_it_and_outside_dismisses() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char(':'));
    assert!(rig.click_on("Go to Settings"));
    assert!(rig.app.palette().is_none());
    assert_eq!(rig.app.tab(), Tab::Settings);

    rig.key(KeyCode::Char(':'));
    let (x, y) = rig.find("Command palette");
    assert!(
        !rig.mouse(MouseEventKind::Down(MouseButton::Left), x, y),
        "the box itself"
    );
    assert!(rig.app.palette().is_some());
    assert!(rig.mouse(MouseEventKind::Down(MouseButton::Left), 0, 20));
    assert!(rig.app.palette().is_none());
}

#[test]
fn the_wheel_scrolls_lists_and_the_palette() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char('2'));
    assert_eq!(rig.selected(), "Home-5G");
    assert!(rig.mouse(MouseEventKind::ScrollDown, 10, 5));
    assert_eq!(rig.app.wifi().cursor(), 1);
    assert!(rig.mouse(MouseEventKind::ScrollUp, 10, 5));
    assert_eq!(rig.app.wifi().cursor(), 0);
    assert!(
        !rig.mouse(MouseEventKind::ScrollUp, 10, 5),
        "already at the top"
    );

    rig.key(KeyCode::Char(':'));
    assert!(rig.mouse(MouseEventKind::ScrollDown, 50, 6));
    assert_eq!(rig.app.palette().unwrap().selected, 1);
    rig.key(KeyCode::Esc);
}

#[test]
fn a_long_palette_list_scrolls_to_keep_the_selection_in_view() {
    let mut rig = Rig::busy();
    rig.resize((100, 14));
    rig.key(KeyCode::Char(':'));
    for _ in 0..15 {
        rig.key(KeyCode::Down);
    }
    let frame = rig.text();
    let selected = rig.app.matches()[rig.app.palette().unwrap().selected]
        .verb
        .verb;
    assert!(frame.contains(selected), "{selected} not visible:\n{frame}");
    insta::assert_snapshot!("palette_scrolled_short", frame);
}

#[test]
fn modals_swallow_clicks_but_not_their_own_buttons() {
    let mut rig = Rig::busy();
    rig.select("Parents House");
    rig.key(KeyCode::Char('f'));
    assert!(rig.app.confirm().is_some());
    // A click on the list behind does nothing.
    assert!(!rig.mouse(MouseEventKind::Down(MouseButton::Left), 4, 3));
    assert!(rig.app.confirm().is_some());
    // The footer's `esc keep` button backs out.
    rig.click_footer("esc keep");
    assert!(rig.app.confirm().is_none());
    assert_eq!(rig.world.nm.profile_count(), 3);
}

// ---- idle -----------------------------------------------------------------------

#[test]
fn nothing_moves_by_itself_while_the_palette_or_help_is_open() {
    let mut rig = Rig::busy();
    rig.key(KeyCode::Char(':'));
    assert!(rig.app.next_tick().is_none(), "no timer behind the palette");
    // Mouse motion, release and drag cost no frame.
    for kind in [
        MouseEventKind::Moved,
        MouseEventKind::Up(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Left),
    ] {
        assert!(!rig.mouse(kind, 10, 10), "{kind:?} must not redraw");
    }
    rig.key(KeyCode::Esc);
    rig.key(KeyCode::Char('?'));
    assert!(rig.app.next_tick().is_none(), "no timer behind the help");
    assert!(!rig.mouse(MouseEventKind::Moved, 3, 3));
}

#[test]
fn hotspot_actions_are_listed_and_say_where_they_work() {
    let mut rig = Rig::busy();
    rig.palette("tether");
    let frame = rig.text();
    assert!(frame.contains("Tether to phone"), "{frame}");
    assert!(
        frame.contains("Open the Hotspot tab and pick a phone first."),
        "{frame}"
    );
}
