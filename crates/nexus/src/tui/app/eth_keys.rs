//! Keys and results of the Ethernet tab, kept apart from the rest of `App`.

use crossterm::event::{KeyCode, KeyEvent};
use nexus_ipc::EthernetState;

use super::{App, Msg};
use crate::tui::eap_form::EapForm;
use crate::tui::ethernet::{Cmd, Done};
use crate::tui::profile::Editor;

impl App {
    /// Keys of the Ethernet list: enter joins, x drops, s signs in with
    /// 802.1X, e edits the Profile.
    pub(super) fn eth_key(&mut self, key: KeyEvent) -> bool {
        let Some(port) = self.ethernet.selected().cloned() else {
            return false;
        };
        let busy = self.ethernet.busy.is_some();
        match key.code {
            KeyCode::Enter if port.state == EthernetState::Unplugged => {
                // Nothing can be joined: say why instead of failing quietly.
                self.say(nexus_ipc::unplugged_message(&port.id), true, None);
                true
            }
            KeyCode::Enter
                if !busy
                    && matches!(
                        port.state,
                        EthernetState::Disconnected | EthernetState::Misconfigured
                    ) =>
            {
                self.run_eth(Cmd::Connect { port: port.id });
                true
            }
            KeyCode::Char('x')
                if matches!(
                    port.state,
                    EthernetState::Connected | EthernetState::Connecting
                ) =>
            {
                self.run_eth(Cmd::Disconnect { port: port.id });
                true
            }
            KeyCode::Char('s') if !busy && port.state != EthernetState::Unplugged => {
                self.toast = None;
                self.eap = Some(EapForm::wired(port.id));
                true
            }
            KeyCode::Char('e') if port.profile.is_some() => {
                if let Some(profile) = port.profile {
                    self.start_editor(Editor::loading_wired(profile.uuid, profile.name));
                }
                true
            }
            _ => false,
        }
    }

    /// Sends an Ethernet command, marking a join as in flight.
    pub(super) fn run_eth(&mut self, cmd: Cmd) {
        if !matches!(cmd, Cmd::Disconnect { .. }) {
            self.ethernet.busy = Some(cmd.port().to_owned());
        }
        self.toast = None;
        if let Some(runner) = &self.runner {
            let sent = cmd.clone();
            runner.call(cmd.op(), move |r| {
                Msg::Wired(Box::new(Done {
                    cmd: sent,
                    result: r.map(|_| ()),
                }))
            });
        }
    }

    /// A command finished.
    pub(super) fn eth_done(&mut self, done: Done) -> bool {
        let Done { cmd, result } = done;
        if !matches!(cmd, Cmd::Disconnect { .. }) {
            self.ethernet.busy = None;
        }
        match (cmd, result) {
            (Cmd::Connect { port }, Ok(())) => self.say(format!("{port} is up."), false, None),
            (Cmd::SignIn { port, .. }, Ok(())) => {
                self.say(format!("Signed in. {port} is up."), false, None);
            }
            (Cmd::Disconnect { port }, Ok(())) => {
                let undo = Cmd::Connect { port: port.clone() };
                self.say(format!("Dropped {port}."), false, Some(undo.into()));
            }
            // Back to the form, nothing typed lost, to try again.
            (Cmd::SignIn { port, eap }, Err(message)) => {
                let mut form = EapForm::reopened(port.clone(), port, &eap, message);
                form.wired = true;
                self.eap = Some(form);
            }
            (_, Err(message)) => self.say(message, true, None),
        }
        true
    }
}
