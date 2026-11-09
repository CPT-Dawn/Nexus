//! State of the Tethering list (on the Hotspot tab): the Daemon's last
//! picture of Bluetooth and the paired phones, and the cursor. No I/O; the
//! commands it triggers are [`super::wifi::Cmd`]s run by the same `Runner`.

use nexus_ipc::{Bluetooth, Phone, Tethers};

#[derive(Debug, Default)]
pub struct TetherTab {
    tethers: Option<Tethers>,
    cursor: usize,
    /// The phone (by id) a tether is being brought up through.
    pub busy: Option<String>,
}

impl TetherTab {
    /// Takes the Daemon's picture. The cursor stays on the same phone when it
    /// is still there. Returns whether anything changed.
    pub fn refresh(&mut self, tethers: Tethers) -> bool {
        if self.tethers.as_ref() == Some(&tethers) {
            return false;
        }
        let keep = self.selected().map(|p| p.id.clone());
        self.tethers = Some(tethers);
        if let Some(id) = keep
            && let Some(i) = self.phones().iter().position(|p| p.id == id)
        {
            self.cursor = i;
        }
        self.cursor = self.cursor.min(self.len().saturating_sub(1));
        true
    }

    /// Whether the Daemon has said anything yet.
    pub fn has_data(&self) -> bool {
        self.tethers.is_some()
    }

    pub fn bluetooth(&self) -> Bluetooth {
        self.tethers
            .as_ref()
            .map_or(Bluetooth::Absent, |t| t.bluetooth)
    }

    pub fn phones(&self) -> &[Phone] {
        self.tethers.as_ref().map_or(&[], |t| &t.phones)
    }

    pub fn len(&self) -> usize {
        self.phones().len()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn selected(&self) -> Option<&Phone> {
        self.phones().get(self.cursor)
    }

    /// Moves the cursor. Returns whether it moved.
    pub fn move_to(&mut self, row: usize) -> bool {
        let row = row.min(self.len().saturating_sub(1));
        let changed = row != self.cursor;
        self.cursor = row;
        changed
    }
}
