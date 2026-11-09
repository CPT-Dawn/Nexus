//! State of the Hotspot panel (on the Hotspot tab, above the Tethering list):
//! the Daemon's last picture of whether this machine can host a Hotspot and
//! the one that runs, and the form for choosing a name and password. No I/O;
//! the commands go out through [`super::wifi::Runner::call`].

use nexus_ipc::Hotspot;
use zeroize::Zeroizing;

/// What the person asked for that has not finished yet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Busy {
    #[default]
    No,
    Starting,
    Stopping,
}

/// How a finished command ended; `Err` is the sentence to show.
#[derive(Debug, Clone)]
pub enum Done {
    /// The name of the Hotspot that came up.
    Started(Result<String, String>),
    /// The name of the Hotspot that stopped.
    Stopped(Result<String, String>),
}

/// Which line of the form has the keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Password,
}

/// The "choose a name and password" form. Both lines may stay empty: the
/// Daemon makes up what is missing.
#[derive(Debug)]
pub struct Form {
    pub ssid: String,
    pub password: Zeroizing<String>,
    pub focus: Field,
    /// The password's letters are on screen, not dots.
    pub letters: bool,
    /// Sent, waiting for the Daemon.
    pub busy: bool,
    /// Why the Daemon turned it down.
    pub error: Option<String>,
}

impl Form {
    pub fn new() -> Form {
        Form {
            ssid: String::new(),
            password: Zeroizing::new(String::new()),
            focus: Field::Name,
            letters: false,
            busy: false,
            error: None,
        }
    }

    pub fn focused(&mut self) -> &mut String {
        match self.focus {
            Field::Name => &mut self.ssid,
            Field::Password => &mut self.password,
        }
    }
}

#[derive(Debug, Default)]
pub struct HotspotTab {
    info: Option<Hotspot>,
    pub busy: Busy,
    pub form: Option<Form>,
}

impl HotspotTab {
    /// Takes the Daemon's picture. Returns whether anything changed.
    pub fn refresh(&mut self, info: Hotspot) -> bool {
        if self.info.as_ref() == Some(&info) {
            return false;
        }
        self.info = Some(info);
        true
    }

    /// Whether the Daemon has said anything yet.
    pub fn has_data(&self) -> bool {
        self.info.is_some()
    }

    pub fn info(&self) -> Option<&Hotspot> {
        self.info.as_ref()
    }

    /// A Hotspot is up or coming up.
    pub fn on(&self) -> bool {
        self.info.as_ref().is_some_and(|h| h.on)
    }

    /// This machine can host one (not known yet counts as no).
    pub fn available(&self) -> bool {
        self.info.as_ref().is_some_and(|h| h.available)
    }

    /// Up and carrying devices: details can be read.
    pub fn ready(&self) -> bool {
        self.info.as_ref().is_some_and(|h| h.on && !h.starting)
    }

    pub fn ssid(&self) -> Option<&str> {
        self.info.as_ref().and_then(|h| h.ssid.as_deref())
    }

    /// Why this machine cannot host one, when it cannot.
    pub fn why(&self) -> Option<&str> {
        self.info
            .as_ref()
            .filter(|h| !h.available)
            .and_then(|h| h.why.as_deref())
    }
}
