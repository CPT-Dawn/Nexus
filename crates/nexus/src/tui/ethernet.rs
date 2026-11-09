//! State of the Ethernet tab: the Daemon's last picture of the ports and the
//! cursor, plus the commands the tab sends. No I/O: commands run through the
//! app's [`super::wifi::Runner`] (`Runner::call`) and come back as
//! [`super::app::Msg::Wired`].

use nexus_ipc::{Eap, EthernetList, EthernetPort, EthernetState, Op};

#[derive(Debug, Default)]
pub struct EthernetTab {
    list: Option<EthernetList>,
    cursor: usize,
    /// The port (by id) being brought up.
    pub busy: Option<String>,
}

impl EthernetTab {
    /// Takes the Daemon's picture. The cursor stays on the same port when it
    /// is still there. Returns whether anything changed.
    pub fn refresh(&mut self, list: EthernetList) -> bool {
        if self.list.as_ref() == Some(&list) {
            return false;
        }
        let keep = self.selected().map(|p| p.id.clone());
        self.list = Some(list);
        if let Some(id) = keep
            && let Some(i) = self.ports().iter().position(|p| p.id == id)
        {
            self.cursor = i;
        }
        self.cursor = self.cursor.min(self.len().saturating_sub(1));
        true
    }

    /// Whether the Daemon has said anything yet.
    pub fn has_data(&self) -> bool {
        self.list.is_some()
    }

    pub fn ports(&self) -> &[EthernetPort] {
        self.list.as_ref().map_or(&[], |l| &l.ports)
    }

    pub fn len(&self) -> usize {
        self.ports().len()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn selected(&self) -> Option<&EthernetPort> {
        self.ports().get(self.cursor)
    }

    /// Moves the cursor. Returns whether it moved.
    pub fn move_to(&mut self, row: usize) -> bool {
        let row = row.min(self.len().saturating_sub(1));
        let changed = row != self.cursor;
        self.cursor = row;
        changed
    }

    /// The port the Overview card speaks for: the one with a Link, else the
    /// one most in need of attention, else the first.
    pub fn headline(&self) -> Option<&EthernetPort> {
        let rank = |p: &&EthernetPort| match p.state {
            EthernetState::Connected => 0,
            EthernetState::Connecting => 1,
            EthernetState::Misconfigured => 2,
            EthernetState::Unplugged => 4,
            _ => 3,
        };
        self.ports().iter().min_by_key(rank)
    }
}

/// What the tab asks the Daemon to do.
#[derive(Debug, Clone)]
pub enum Cmd {
    Connect {
        port: String,
    },
    Disconnect {
        port: String,
    },
    /// Sign in to an 802.1X port with the form's settings.
    SignIn {
        port: String,
        eap: Box<Eap>,
    },
}

impl Cmd {
    pub fn op(&self) -> Op {
        match self {
            Cmd::Connect { port } => Op::EthernetConnect {
                port: Some(port.clone()),
            },
            Cmd::Disconnect { port } => Op::EthernetDisconnect {
                port: Some(port.clone()),
            },
            Cmd::SignIn { port, eap } => Op::EthernetJoinEap {
                port: Some(port.clone()),
                eap: (**eap).clone(),
            },
        }
    }

    pub fn port(&self) -> &str {
        match self {
            Cmd::Connect { port } | Cmd::Disconnect { port } | Cmd::SignIn { port, .. } => port,
        }
    }
}

/// How a [`Cmd`] ended; `Err` is the Daemon's sentence.
#[derive(Debug, Clone)]
pub struct Done {
    pub cmd: Cmd,
    pub result: Result<(), String>,
}
