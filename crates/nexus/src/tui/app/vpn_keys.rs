//! Keys and results of the VPN tab, kept apart from the rest of `App`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_ipc::VpnState;

use super::App;
use crate::tui::vpn::{Cmd, Done, Form, Layer, Reply, ShareView, Stage};

impl App {
    /// Keys of the VPN list (no layer open).
    pub(super) fn vpn_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('i') => {
                self.toast = None;
                self.vpn.layer = Some(Layer::Form(Form::import()));
                true
            }
            KeyCode::Char('c') => {
                self.toast = None;
                self.vpn.layer = Some(Layer::Form(Form::create()));
                true
            }
            _ => {
                let Some(vpn) = self.vpn.selected().cloned() else {
                    return false;
                };
                match key.code {
                    KeyCode::Enter if self.vpn.busy.is_none() => {
                        self.toast = None;
                        self.vpn.busy = Some(vpn.id.clone());
                        self.vpn_send(Cmd::Toggle {
                            id: vpn.id,
                            name: vpn.name,
                            up: vpn.state == VpnState::Down,
                        });
                        true
                    }
                    KeyCode::Char('p') if vpn.kind == nexus_ipc::VpnKind::Wireguard => {
                        self.toast = None;
                        self.vpn_send(Cmd::Key {
                            id: vpn.id,
                            name: vpn.name,
                        });
                        true
                    }
                    KeyCode::Char('s') if vpn.kind == nexus_ipc::VpnKind::Wireguard => {
                        self.toast = None;
                        self.vpn.layer = Some(Layer::Share(ShareView {
                            id: vpn.id,
                            name: vpn.name,
                            stage: Stage::Ask,
                        }));
                        true
                    }
                    _ => false,
                }
            }
        }
    }

    fn vpn_send(&self, cmd: Cmd) {
        if let Some(runner) = &self.vpn_runner {
            runner.run(cmd);
        }
    }

    /// Keys while a form, the key view or the QR view is open.
    pub(super) fn vpn_layer_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(layer) = self.vpn.layer.as_mut() else {
            return false;
        };
        match layer {
            Layer::Key { .. } => match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.vpn.layer = None;
                    true
                }
                _ => false,
            },
            Layer::Share(view) => match (&view.stage, key.code) {
                (_, KeyCode::Esc) => {
                    self.vpn.layer = None;
                    true
                }
                (Stage::Ask | Stage::Failed(_), KeyCode::Enter) => {
                    view.stage = Stage::Reading;
                    let cmd = Cmd::Reveal {
                        id: view.id.clone(),
                        name: view.name.clone(),
                    };
                    self.vpn_send(cmd);
                    true
                }
                _ => false,
            },
            Layer::Form(form) => {
                match key.code {
                    KeyCode::Esc => {
                        self.vpn.layer = None;
                        return true;
                    }
                    _ if form.busy => return false,
                    KeyCode::Enter if form.focus + 1 < form.fields.len() => form.focus += 1,
                    KeyCode::Enter => match form.command() {
                        Ok(cmd) => {
                            form.busy = true;
                            form.error = None;
                            self.vpn_send(cmd);
                        }
                        Err(why) => form.error = Some(why),
                    },
                    KeyCode::Tab | KeyCode::Down => {
                        form.focus = (form.focus + 1) % form.fields.len();
                    }
                    KeyCode::BackTab | KeyCode::Up => {
                        form.focus = (form.focus + form.fields.len() - 1) % form.fields.len();
                    }
                    KeyCode::Backspace => {
                        form.fields[form.focus].value.pop();
                    }
                    KeyCode::Char('u') if ctrl => form.fields[form.focus].value.clear(),
                    KeyCode::Char(c) if !ctrl => form.fields[form.focus].value.push(c),
                    _ => return false,
                }
                true
            }
        }
    }

    /// A command finished.
    pub(super) fn vpn_done(&mut self, done: Done) -> bool {
        let Done { cmd, result } = done;
        match (cmd, result) {
            (Cmd::Toggle { name, .. }, result) => {
                self.vpn.busy = None;
                match result {
                    Ok(Reply::Toggled(t)) => {
                        let word = if t.state == VpnState::Up {
                            "up"
                        } else {
                            "down"
                        };
                        self.say(format!("{name} is {word}."), false, None);
                    }
                    Ok(_) => {}
                    Err(message) => self.say(message, true, None),
                }
            }
            (Cmd::Import { .. } | Cmd::Create(_), Ok(Reply::Added(added))) => {
                // The new tunnel's public key is the next thing the person needs.
                self.vpn.layer = Some(Layer::Key {
                    name: added.name,
                    key: added.public_key,
                    saved: true,
                });
            }
            (Cmd::Import { .. } | Cmd::Create(_), Err(message)) => match self.vpn.layer.as_mut() {
                Some(Layer::Form(form)) => {
                    form.busy = false;
                    form.error = Some(message);
                }
                _ => self.say(message, true, None),
            },
            (Cmd::Key { .. }, Ok(Reply::Key(k))) => {
                self.vpn.layer = Some(Layer::Key {
                    name: k.name,
                    key: k.public_key,
                    saved: false,
                });
            }
            (Cmd::Key { .. }, Err(message)) => self.say(message, true, None),
            (Cmd::Reveal { id, .. }, result) => match self.vpn.layer.as_mut() {
                Some(Layer::Share(view))
                    if view.id == id && matches!(view.stage, Stage::Reading) =>
                {
                    view.arrived(match result {
                        Ok(Reply::Share(share)) => Ok(share),
                        Ok(_) => Err("The daemon sent something unexpected.".into()),
                        Err(message) => Err(message),
                    });
                }
                // Closed meanwhile: the details drop here, wiped.
                _ => return false,
            },
            _ => return false,
        }
        true
    }

    /// Footer hints of the VPN tab. `more` adds the less common keys.
    pub(super) fn vpn_hints(&self, more: bool) -> Vec<(&'static str, &'static str)> {
        match &self.vpn.layer {
            Some(Layer::Form(form)) => {
                return if form.busy {
                    vec![("esc", "close")]
                } else if form.focus + 1 < form.fields.len() {
                    vec![("enter", "next"), ("tab", "move"), ("esc", "cancel")]
                } else {
                    vec![("enter", "save"), ("tab", "move"), ("esc", "cancel")]
                };
            }
            Some(Layer::Key { .. }) => return vec![("esc", "close")],
            Some(Layer::Share(view)) => {
                return match view.stage {
                    Stage::Ask => vec![("enter", "reveal"), ("esc", "cancel")],
                    Stage::Reading => vec![("esc", "cancel")],
                    Stage::Shown { .. } => vec![("esc", "close")],
                    Stage::Failed(_) => vec![("enter", "try again"), ("esc", "close")],
                };
            }
            None => {}
        }
        let mut v = Vec::new();
        if let Some(vpn) = self.vpn.selected() {
            v.push((
                "enter",
                if vpn.state == VpnState::Down {
                    "up"
                } else {
                    "down"
                },
            ));
        }
        v.push(("i", "import"));
        v.push(("c", "create"));
        if more
            && self
                .vpn
                .selected()
                .is_some_and(|v| v.kind == nexus_ipc::VpnKind::Wireguard)
        {
            v.push(("p", "key"));
            v.push(("s", "share"));
        }
        v
    }
}
