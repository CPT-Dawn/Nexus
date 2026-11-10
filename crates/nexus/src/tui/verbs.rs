//! The one table of things a person can do, in plain English.
//!
//! The command palette (`:`) searches it and the `?` help overlay is
//! generated from it, so a key listed here is findable and documented without
//! further work.
//!
//! Adding an action (a later ticket): write a `static` slice of [`Verb`]s in
//! your own module and add it to [`TABLES`]. `run` says what to do (press a key
//! on a tab, jump to a tab, or call a function), `ready` says whether it can
//! run now or, as `Err`, the sentence that explains why not. Leave `help` empty
//! to keep a row out of the `?` overlay.

use crossterm::event::KeyCode;

use super::app::{App, Tab};
use super::wifi::Entry;
use crate::text;

/// Where a Verb's key works; shown in the help overlay as `Wi-Fi: r`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Any,
    Tab(Tab),
}

/// What running a Verb does.
#[derive(Debug, Clone, Copy)]
pub enum Run {
    /// Press a key as the root layer sees it (works on any tab).
    Root(KeyCode),
    /// Press a key as the given tab's handler sees it (works while another
    /// tab shows; the Verb's `ready` says whether a selection is needed).
    Key(Tab, KeyCode),
    /// Go to a tab.
    Go(Tab),
    /// Anything else. Returns whether the screen changed.
    Call(fn(&mut App) -> bool),
}

/// A Verb's place in the footer hint row. The footer is derived from the
/// Verbs that can run right now, so it can never offer a key the palette
/// would refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    /// Not in the footer.
    None,
    /// Always shown while the Verb can run.
    Core(&'static str),
    /// Shown only when there is room for the less common keys.
    More(&'static str),
}

#[derive(Debug)]
pub struct Verb {
    /// Plain-English imperative, as listed: "Forget network".
    pub verb: &'static str,
    /// Extra words the search also matches ("remove delete saved").
    pub words: &'static str,
    /// The key as shown ("f", "Enter"); empty if there is none.
    pub key: &'static str,
    /// One line for the `?` overlay; empty keeps the Verb out of it.
    pub help: &'static str,
    pub scope: Scope,
    pub run: Run,
    /// Whether, and as what, it appears in the footer.
    pub hint: Hint,
    /// `Ok` when it can run now, else the reason it cannot.
    pub ready: fn(&App) -> Result<(), &'static str>,
}

/// Every table of Verbs, in the order the help overlay lists them.
pub static TABLES: &[&[Verb]] = &[GLOBAL, WIFI, ETHERNET, VPN, HOTSPOT, TETHER, SETTINGS, UNDO];

pub fn all() -> impl Iterator<Item = &'static Verb> {
    TABLES.iter().flat_map(|t| t.iter())
}

fn always(_: &App) -> Result<(), &'static str> {
    Ok(())
}

const fn go(verb: &'static str, key: &'static str, tab: Tab) -> Verb {
    Verb {
        verb,
        words: "open tab show switch",
        key,
        help: "",
        scope: Scope::Any,
        run: Run::Go(tab),
        hint: Hint::None,
        ready: always,
    }
}

static GLOBAL: &[Verb] = &[
    go("Go to Overview", "1", Tab::Overview),
    go("Go to Wi-Fi", "2", Tab::WiFi),
    go("Go to Ethernet", "3", Tab::Ethernet),
    go("Go to VPN", "4", Tab::Vpn),
    go("Go to Hotspot", "5", Tab::Hotspot),
    go("Go to Settings", "6", Tab::Settings),
    Verb {
        verb: "Open login page",
        words: "portal captive sign in browser",
        key: "o",
        help: "Open a Portal's login page",
        scope: Scope::Any,
        run: Run::Root(KeyCode::Char('o')),
        hint: Hint::Core("login"),
        ready: |app| app.portal().map(|_| ()).ok_or("No login page is waiting."),
    },
    Verb {
        verb: "Run the suggested fix",
        words: "doctor repair diagnostics problem",
        key: "f",
        help: "Overview: run the fix (asks first)",
        scope: Scope::Any,
        run: Run::Root(KeyCode::Char('f')),
        hint: Hint::Core("fix"),
        ready: |app| {
            if app.tab() != Tab::Overview {
                Err("Fixes are offered on the Overview.")
            } else if app.runnable_fix().is_none() {
                Err("Nothing to fix right now.")
            } else {
                Ok(())
            }
        },
    },
    Verb {
        verb: "Filter the list",
        words: "search find narrow type name networks vpns",
        key: "/",
        help: "Filter the Wi-Fi or VPN list by name (Esc clears)",
        scope: Scope::Any,
        run: Run::Root(KeyCode::Char('/')),
        hint: Hint::None,
        ready: |app| {
            if app.filterable() {
                Ok(())
            } else {
                Err("Open the Wi-Fi or VPN tab to filter its list.")
            }
        },
    },
    Verb {
        verb: "Turn Wi-Fi off",
        words: "wifi wi-fi radio disable switch power",
        key: "w",
        help: "Turn Wi-Fi off or on (undo with u)",
        scope: Scope::Any,
        run: Run::Root(KeyCode::Char('w')),
        hint: Hint::None,
        ready: |app| {
            if app.radios().wifi.enabled {
                Ok(())
            } else {
                Err(text::tui::WIFI_ALREADY_OFF)
            }
        },
    },
    Verb {
        verb: "Turn Wi-Fi on",
        words: "wifi wi-fi radio enable switch power hardware blocked",
        key: "w",
        help: "",
        scope: Scope::Any,
        run: Run::Root(KeyCode::Char('w')),
        hint: Hint::None,
        ready: |app| {
            let r = app.radios();
            if r.wifi.hardware_blocked {
                Err(text::HARD_BLOCKED)
            } else if r.wifi.enabled {
                Err(text::tui::WIFI_ALREADY_ON)
            } else {
                Ok(())
            }
        },
    },
    Verb {
        verb: "Turn airplane mode on",
        words: "flight radios offline wifi bluetooth off",
        key: "A",
        help: "Turn airplane mode on or off: every radio off (undo with u)",
        scope: Scope::Any,
        run: Run::Root(KeyCode::Char('A')),
        hint: Hint::None,
        ready: |app| {
            if app.radios().airplane {
                Err(text::tui::AIRPLANE_ALREADY_ON)
            } else {
                Ok(())
            }
        },
    },
    Verb {
        verb: "Turn airplane mode off",
        words: "flight radios online wifi bluetooth on",
        key: "A",
        help: "",
        scope: Scope::Any,
        run: Run::Root(KeyCode::Char('A')),
        hint: Hint::None,
        ready: |app| {
            if app.radios().airplane {
                Ok(())
            } else {
                Err(text::tui::AIRPLANE_ALREADY_OFF)
            }
        },
    },
    Verb {
        verb: "Show all keys",
        words: "help shortcuts keyboard",
        key: "?",
        help: "Show or hide this help",
        scope: Scope::Any,
        run: Run::Root(KeyCode::Char('?')),
        hint: Hint::None,
        ready: always,
    },
    Verb {
        verb: "Quit Nexus",
        words: "exit close",
        key: "q",
        help: "Quit (Ctrl-C works anywhere)",
        scope: Scope::Any,
        run: Run::Root(KeyCode::Char('q')),
        hint: Hint::None,
        ready: always,
    },
];

// ---- Wi-Fi ----------------------------------------------------------------------

fn picked(app: &App) -> Result<&Entry, &'static str> {
    if app.tab() != Tab::WiFi {
        return Err("Open the Wi-Fi tab and pick a Network first.");
    }
    app.wifi().selected().ok_or("No Network is selected.")
}

fn saved(app: &App) -> Result<(), &'static str> {
    if picked(app)?.saved() {
        Ok(())
    } else {
        Err("That Network isn't saved.")
    }
}

const fn wifi(
    verb: &'static str,
    words: &'static str,
    key: &'static str,
    help: &'static str,
    code: KeyCode,
    hint: Hint,
    ready: fn(&App) -> Result<(), &'static str>,
) -> Verb {
    Verb {
        verb,
        words,
        key,
        help,
        scope: Scope::Tab(Tab::WiFi),
        run: Run::Key(Tab::WiFi, code),
        hint,
        ready,
    }
}

static WIFI: &[Verb] = &[
    wifi(
        "Join network",
        "connect wifi wi-fi",
        "Enter",
        "Join the selected Network",
        KeyCode::Enter,
        Hint::Core("join"),
        |app| {
            let entry = picked(app)?;
            if entry.joined() {
                Err("Already connected to that Network.")
            } else if app.wifi().busy.is_some() {
                Err("A join is already running.")
            } else {
                Ok(())
            }
        },
    ),
    wifi(
        "Rescan Wi-Fi",
        "scan refresh search networks",
        "r",
        "Rescan",
        KeyCode::Char('r'),
        Hint::Core("scan"),
        always,
    ),
    wifi(
        "Disconnect",
        "leave drop wifi wi-fi",
        "x",
        "Disconnect",
        KeyCode::Char('x'),
        Hint::More("disconnect"),
        |app| {
            if picked(app)?.joined() {
                Ok(())
            } else {
                Err("Not connected to that Network.")
            }
        },
    ),
    wifi(
        "Forget network",
        "remove delete saved profile",
        "f",
        "Forget (asks first)",
        KeyCode::Char('f'),
        Hint::More("forget"),
        saved,
    ),
    wifi(
        "Share Wi-Fi",
        "qr code password phone",
        "s",
        "Share as a QR code (asks first)",
        KeyCode::Char('s'),
        Hint::More("share"),
        saved,
    ),
    wifi(
        "Toggle autoconnect",
        "automatic auto join",
        "a",
        "Toggle autoconnect",
        KeyCode::Char('a'),
        Hint::More("auto"),
        |app| {
            if picked(app)?.autoconnect().is_some() {
                Ok(())
            } else {
                Err("That Network isn't saved.")
            }
        },
    ),
    wifi(
        "Trust or distrust network",
        "untrusted vpn metered safe",
        "t",
        "Mark Trusted or Untrusted",
        KeyCode::Char('t'),
        Hint::More("trust"),
        |app| {
            if picked(app)?.trust().is_some() {
                Ok(())
            } else {
                Err("That Network isn't in range.")
            }
        },
    ),
    wifi(
        "Edit network settings",
        "profile ip dns address mac metered priority",
        "e",
        "Edit the Profile's settings",
        KeyCode::Char('e'),
        Hint::More("settings"),
        saved,
    ),
    wifi(
        "Join hidden network",
        "other name ssid connect wifi wi-fi",
        "n",
        "Join a hidden Network by name",
        KeyCode::Char('n'),
        Hint::More("other"),
        |app| {
            if app.tab() == Tab::WiFi {
                Ok(())
            } else {
                Err("Open the Wi-Fi tab first.")
            }
        },
    ),
    wifi(
        "Choose Wi-Fi card",
        "adapter device interface radio",
        "c",
        "Pick the Wi-Fi card joins use",
        KeyCode::Char('c'),
        Hint::More("card"),
        |app| {
            if app.tab() != Tab::WiFi {
                Err("Open the Wi-Fi tab first.")
            } else if app.wifi().cards().is_empty() {
                Err("There is only one Wi-Fi card.")
            } else {
                Ok(())
            }
        },
    ),
    wifi(
        "Show access points",
        "details info bssid channel frequency band rate",
        "i",
        "Access point details of the Network",
        KeyCode::Char('i'),
        Hint::None,
        |app| match picked(app)? {
            Entry::Net(_) => Ok(()),
            Entry::Saved(_) => Err("That Network is out of range."),
        },
    ),
];

// ---- VPN ------------------------------------------------------------------------

fn vpn_open(app: &App) -> Result<(), &'static str> {
    if app.tab() == Tab::Vpn {
        Ok(())
    } else {
        Err("Open the VPN tab first.")
    }
}

fn vpn_picked(app: &App) -> Result<&nexus_ipc::Vpn, &'static str> {
    vpn_open(app)?;
    app.vpn().selected().ok_or("No VPN is selected.")
}

fn vpn_wireguard(app: &App) -> Result<(), &'static str> {
    if vpn_picked(app)?.kind == nexus_ipc::VpnKind::Wireguard {
        Ok(())
    } else {
        Err("Only WireGuard VPNs have keys to show.")
    }
}

const fn vpn(
    verb: &'static str,
    words: &'static str,
    key: &'static str,
    help: &'static str,
    code: KeyCode,
    hint: Hint,
    ready: fn(&App) -> Result<(), &'static str>,
) -> Verb {
    Verb {
        verb,
        words,
        key,
        help,
        scope: Scope::Tab(Tab::Vpn),
        run: Run::Key(Tab::Vpn, code),
        hint,
        ready,
    }
}

static VPN: &[Verb] = &[
    vpn(
        "Bring VPN up",
        "toggle vpn connect start wireguard",
        "Enter",
        "Bring the VPN up or down",
        KeyCode::Enter,
        Hint::Core("up"),
        |app| {
            if vpn_picked(app)?.state != nexus_ipc::VpnState::Down {
                Err("That VPN is already up.")
            } else if app.vpn().busy.is_some() {
                Err("A VPN is already changing.")
            } else {
                Ok(())
            }
        },
    ),
    vpn(
        "Bring VPN down",
        "toggle vpn disconnect stop wireguard",
        "Enter",
        "",
        KeyCode::Enter,
        Hint::Core("down"),
        |app| {
            if vpn_picked(app)?.state == nexus_ipc::VpnState::Down {
                Err("That VPN is already down.")
            } else if app.vpn().busy.is_some() {
                Err("A VPN is already changing.")
            } else {
                Ok(())
            }
        },
    ),
    vpn(
        "Import WireGuard config",
        "vpn wireguard conf file add",
        "i",
        "Import a WireGuard .conf",
        KeyCode::Char('i'),
        Hint::Core("import"),
        vpn_open,
    ),
    vpn(
        "Create WireGuard VPN",
        "vpn wireguard new tunnel generate keys add",
        "c",
        "Create a WireGuard VPN with new keys",
        KeyCode::Char('c'),
        Hint::Core("create"),
        vpn_open,
    ),
    vpn(
        "Show VPN public key",
        "vpn wireguard key peer",
        "p",
        "Show a VPN's public key",
        KeyCode::Char('p'),
        Hint::More("key"),
        vpn_wireguard,
    ),
    vpn(
        "Share VPN config",
        "vpn wireguard qr code phone private key",
        "s",
        "Share a VPN's config as a QR code (asks first)",
        KeyCode::Char('s'),
        Hint::More("share"),
        vpn_wireguard,
    ),
];

// ---- Hotspot: sharing this machine's connection ---------------------------------

fn can_start(app: &App) -> Result<(), &'static str> {
    let tab = app.hotspot();
    if !tab.has_data() {
        Err("Still checking whether this machine can host a Hotspot.")
    } else if !tab.available() {
        Err("This machine cannot host a Hotspot; the Hotspot tab says why.")
    } else if tab.on() {
        Err("A Hotspot is already running.")
    } else {
        Ok(())
    }
}

static HOTSPOT: &[Verb] = &[
    hotspot(
        "Start Hotspot",
        "share connection access point password",
        "n",
        "Share this connection with a made-up password",
        KeyCode::Char('n'),
        Hint::Core("hotspot"),
        can_start,
    ),
    hotspot(
        "Start Hotspot with my own name and password",
        "share connection access point name",
        "c",
        "Start a Hotspot with a name and password you choose",
        KeyCode::Char('c'),
        Hint::Core("custom"),
        can_start,
    ),
    hotspot(
        "Share Hotspot as QR code",
        "hotspot password phone join",
        "s",
        "Show the Hotspot's QR code and password (asks first)",
        KeyCode::Char('s'),
        Hint::Core("hotspot QR"),
        |app| {
            if app.hotspot().ready() {
                Ok(())
            } else {
                Err("No Hotspot is up to share.")
            }
        },
    ),
    hotspot(
        "Stop Hotspot",
        "share connection access point end",
        "S",
        "Stop the Hotspot",
        KeyCode::Char('S'),
        Hint::Core("stop hotspot"),
        |app| {
            if app.hotspot().on() {
                Ok(())
            } else {
                Err("No Hotspot is running.")
            }
        },
    ),
];

// ---- Hotspot: Tethering ---------------------------------------------------------

fn phone(app: &App) -> Result<&nexus_ipc::Phone, &'static str> {
    if app.tab() != Tab::Hotspot {
        return Err("Open the Hotspot tab and pick a phone first.");
    }
    app.tether().selected().ok_or("No phone is selected.")
}

const fn hotspot(
    verb: &'static str,
    words: &'static str,
    key: &'static str,
    help: &'static str,
    code: KeyCode,
    hint: Hint,
    ready: fn(&App) -> Result<(), &'static str>,
) -> Verb {
    Verb {
        verb,
        words,
        key,
        help,
        scope: Scope::Tab(Tab::Hotspot),
        run: Run::Key(Tab::Hotspot, code),
        hint,
        ready,
    }
}

static TETHER: &[Verb] = &[
    hotspot(
        "Tether to phone",
        "bluetooth internet connect share connection",
        "Enter",
        "Use the selected phone's data",
        KeyCode::Enter,
        Hint::Core("tether"),
        |app| {
            if phone(app)?.connected {
                Err("Already tethered to that phone.")
            } else if app.tether().busy.is_some() {
                Err("A tether is already starting.")
            } else {
                Ok(())
            }
        },
    ),
    hotspot(
        "Drop tether",
        "bluetooth disconnect stop phone",
        "x",
        "Drop the phone's tether",
        KeyCode::Char('x'),
        Hint::Core("drop tether"),
        |app| {
            if phone(app)?.connected {
                Ok(())
            } else {
                Err("Not tethered to that phone.")
            }
        },
    ),
];

// ---- Settings -------------------------------------------------------------------

fn loaded(app: &App) -> Result<(), &'static str> {
    if app.settings().has_data() {
        Ok(())
    } else {
        Err("The daemon has not sent its settings yet.")
    }
}

static SETTINGS: &[Verb] = &[
    Verb {
        verb: "Switch theme",
        words: "colours colors palette look dark nord gruvbox catppuccin tokyo rose pine contrast",
        key: "",
        help: "Use the next theme",
        scope: Scope::Any,
        run: Run::Call(|app| app.change_setting_by_key("theme")),
        hint: Hint::None,
        ready: loaded,
    },
    Verb {
        verb: "Toggle Nerd Font icons",
        words: "icons glyphs font symbols",
        key: "",
        help: "",
        scope: Scope::Any,
        run: Run::Call(|app| app.change_setting_by_key("ui.nerd_font")),
        hint: Hint::None,
        ready: loaded,
    },
    Verb {
        verb: "Toggle reduced motion",
        words: "animation effects accessibility calm",
        key: "",
        help: "",
        scope: Scope::Any,
        run: Run::Call(|app| app.change_setting_by_key("ui.reduce_motion")),
        hint: Hint::None,
        ready: loaded,
    },
    Verb {
        verb: "Change setting",
        words: "toggle switch notification option",
        key: "Enter",
        help: "Change the selected setting",
        scope: Scope::Tab(Tab::Settings),
        run: Run::Key(Tab::Settings, KeyCode::Enter),
        hint: Hint::Core("change"),
        ready: |app| {
            if app.tab() != Tab::Settings {
                Err("Open the Settings tab and pick a setting first.")
            } else {
                loaded(app)
            }
        },
    },
    Verb {
        verb: "Reset setting",
        words: "default restore",
        key: "r",
        help: "Put the default back",
        scope: Scope::Tab(Tab::Settings),
        run: Run::Key(Tab::Settings, KeyCode::Char('r')),
        hint: Hint::Core("reset"),
        ready: |app| {
            if app.tab() != Tab::Settings {
                Err("Open the Settings tab and pick a setting first.")
            } else {
                loaded(app)
            }
        },
    },
];

// ---- Ethernet -------------------------------------------------------------------

fn port(app: &App) -> Result<&nexus_ipc::EthernetPort, &'static str> {
    if app.tab() != Tab::Ethernet {
        return Err("Open the Ethernet tab and pick a port first.");
    }
    app.ethernet()
        .selected()
        .ok_or("No Ethernet port is selected.")
}

const fn wired(
    verb: &'static str,
    words: &'static str,
    key: &'static str,
    help: &'static str,
    code: KeyCode,
    hint: Hint,
    ready: fn(&App) -> Result<(), &'static str>,
) -> Verb {
    Verb {
        verb,
        words,
        key,
        help,
        scope: Scope::Tab(Tab::Ethernet),
        run: Run::Key(Tab::Ethernet, code),
        hint,
        ready,
    }
}

static ETHERNET: &[Verb] = &[
    wired(
        "Connect Ethernet",
        "wired cable port join retry",
        "Enter",
        "Join the selected port",
        KeyCode::Enter,
        Hint::Core("connect"),
        |app| {
            use nexus_ipc::EthernetState::{Connected, Connecting, Unplugged};
            let p = port(app)?;
            if p.state == Unplugged {
                Err("No cable is plugged in.")
            } else if matches!(p.state, Connected | Connecting) {
                Err("That port is already up.")
            } else if app.ethernet().busy.is_some() {
                Err("A port is already coming up.")
            } else {
                Ok(())
            }
        },
    ),
    wired(
        "Disconnect Ethernet",
        "wired cable port drop stop",
        "x",
        "Drop the port's Link",
        KeyCode::Char('x'),
        Hint::Core("disconnect"),
        |app| {
            use nexus_ipc::EthernetState::{Connected, Connecting};
            if matches!(port(app)?.state, Connected | Connecting) {
                Ok(())
            } else {
                Err("That port is not connected.")
            }
        },
    ),
    wired(
        "Sign in to Ethernet",
        "wired 802.1x enterprise office campus identity password certificate",
        "s",
        "Sign in to an 802.1X port",
        KeyCode::Char('s'),
        Hint::Core("sign in"),
        |app| {
            if port(app)?.state == nexus_ipc::EthernetState::Unplugged {
                Err("No cable is plugged in.")
            } else if app.ethernet().busy.is_some() {
                Err("A port is already coming up.")
            } else {
                Ok(())
            }
        },
    ),
    wired(
        "Ethernet settings",
        "wired profile ip address dns mac static metered",
        "e",
        "Edit the port's Profile settings",
        KeyCode::Char('e'),
        Hint::Core("settings"),
        |app| {
            port(app)?
                .profile
                .as_ref()
                .map(|_| ())
                .ok_or("That port has no saved Profile yet. Connect it first.")
        },
    ),
];

// ---- both lists -----------------------------------------------------------------

static UNDO: &[Verb] = &[Verb {
    verb: "Undo last change",
    words: "revert",
    key: "u",
    help: "Wi-Fi, Hotspot: undo the toast",
    scope: Scope::Any,
    run: Run::Call(App::undo),
    hint: Hint::Core("undo"),
    ready: |app| {
        if app.toast().is_some_and(|t| t.undo.is_some()) {
            Ok(())
        } else {
            Err("Nothing to undo.")
        }
    },
}];

// ---- searching ------------------------------------------------------------------

/// One match of a palette search.
#[derive(Debug, Clone, Copy)]
pub struct Found {
    pub verb: &'static Verb,
    /// `Some(reason)` when it cannot run right now.
    pub blocked: Option<&'static str>,
}

/// How well `token` matches `hay` (both lower case): a substring beats a
/// scattered match, and an early or word-start one beats a late one.
fn score(hay: &str, token: &str) -> Option<i32> {
    if let Some(p) = hay.find(token) {
        let word_start = p == 0 || hay.as_bytes()[p - 1] == b' ';
        return Some(200 - p as i32 + if word_start { 50 } else { 0 });
    }
    let mut chars = hay.chars().enumerate();
    let (mut last, mut gaps) = (None::<usize>, 0usize);
    for want in token.chars() {
        loop {
            match chars.next() {
                Some((i, c)) if c == want => {
                    if let Some(l) = last {
                        gaps += i - l - 1;
                    }
                    last = Some(i);
                    break;
                }
                Some(_) => {}
                None => return None,
            }
        }
    }
    Some(100 - gaps.min(90) as i32)
}

/// Verbs matching `query` (every word of it must match), best first. With
/// no query, the ones that can run now come before the ones that cannot.
pub fn search(app: &App, query: &str) -> Vec<Found> {
    let query = query.to_lowercase();
    let tokens: Vec<&str> = query.split_whitespace().collect();
    let mut found: Vec<(Found, i32)> = all()
        .filter_map(|verb| {
            let hay = format!("{} {}", verb.verb, verb.words).to_lowercase();
            let mut total = 0;
            for t in &tokens {
                total += score(&hay, t)?;
            }
            let blocked = (verb.ready)(app).err();
            Some((Found { verb, blocked }, total))
        })
        .collect();
    // Stable sort: ties keep table order.
    // A blocked Verb ranks a little lower, but a clearly better match still
    // wins so its reason can be read.
    found.sort_by_key(|(f, s)| -(s - if f.blocked.is_some() { 30 } else { 0 }));
    found.into_iter().map(|(f, _)| f).collect()
}

// ---- footer hints ---------------------------------------------------------------

/// The footer key as shown ("Enter" reads "enter"; the rest are as written).
fn shown(key: &'static str) -> &'static str {
    if key == "Enter" { "enter" } else { key }
}

fn hint_of(app: &App, verb: &Verb, more: bool) -> Option<(&'static str, &'static str)> {
    let text = match verb.hint {
        Hint::None => return None,
        Hint::More(_) if !more => return None,
        Hint::Core(t) | Hint::More(t) => t,
    };
    (verb.ready)(app).ok()?;
    Some((shown(verb.key), text))
}

/// The footer keys of `tab`'s own Verbs that can run now, in table order.
/// `more` adds the less common ones.
pub fn tab_hints(app: &App, tab: Tab, more: bool) -> Vec<(&'static str, &'static str)> {
    all()
        .filter(|v| v.scope == Scope::Tab(tab))
        .filter_map(|v| hint_of(app, v, more))
        .collect()
}

/// The footer keys of the Verbs that work on any tab and can run now (undo,
/// the login page, the fix).
pub fn global_hints(app: &App) -> Vec<(&'static str, &'static str)> {
    all()
        .filter(|v| v.scope == Scope::Any)
        .filter_map(|v| hint_of(app, v, true))
        .collect()
}

// ---- help -----------------------------------------------------------------------

/// Keys that are not Verbs (movement and backing out), listed first.
const NAV: &[(&str, &str)] = &[
    (":", "Open the command palette"),
    ("j / k, Down / Up", "Move in the list"),
    ("g / G", "Jump to top / bottom"),
    ("h / l, Left / Right", "Previous / next tab"),
    ("Tab / Shift-Tab", "Next / previous tab"),
    ("1 - 6", "Jump to a tab"),
    ("Enter", "Open the selected card's tab"),
    ("d", "Details sheet (small windows)"),
    ("Esc", "Back out; never quits"),
];

/// The rows of the `?` overlay: `(keys, what it does)`.
pub fn help_rows() -> Vec<(String, &'static str)> {
    let mut rows: Vec<(String, &'static str)> =
        NAV.iter().map(|(k, w)| ((*k).to_owned(), *w)).collect();
    for v in all().filter(|v| !v.help.is_empty() && !v.key.is_empty()) {
        let keys = match v.scope {
            Scope::Any => v.key.to_owned(),
            Scope::Tab(tab) => format!("{}: {}", tab.title(), v.key),
        };
        rows.push((keys, v.help));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::{App, Caps};

    fn app() -> App {
        App::new(Caps::from_env(true, false))
    }

    fn top(query: &str) -> Option<&'static str> {
        search(&app(), query).first().map(|f| f.verb.verb)
    }

    #[test]
    fn plain_english_finds_the_verb() {
        assert_eq!(top("forget network"), Some("Forget network"));
        assert_eq!(top("share wi-fi"), Some("Share Wi-Fi"));
        assert_eq!(top("rescan"), Some("Rescan Wi-Fi"));
        assert_eq!(top("scan"), Some("Rescan Wi-Fi"));
    }

    #[test]
    fn scattered_letters_still_match() {
        assert_eq!(top("frgt"), Some("Forget network"));
        assert_eq!(top("zzzz"), None);
    }

    #[test]
    fn synonyms_in_words_match() {
        assert_eq!(top("qr"), Some("Share Wi-Fi"));
        assert_eq!(top("captive"), Some("Open login page"));
    }

    #[test]
    fn blocked_verbs_sort_after_ready_ones() {
        let found = search(&app(), "");
        let first_blocked = found.iter().position(|f| f.blocked.is_some()).unwrap();
        assert!(found[first_blocked..].iter().all(|f| f.blocked.is_some()));
        assert!(
            first_blocked > 0,
            "the tab jumps and rescan are always ready"
        );
    }

    #[test]
    fn help_lists_every_keyed_verb_once() {
        let rows = help_rows();
        assert!(
            rows.iter()
                .any(|(k, w)| k == "Wi-Fi: f" && w.contains("Forget"))
        );
        assert!(rows.iter().all(|(k, _)| !k.is_empty()));
    }
}
