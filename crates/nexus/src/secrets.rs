//! The Secret prompt broker: turns the Backend's requests for a secret into
//! Secret prompts for a Front end, and the answer back into a secret.
//!
//! It knows nothing about NetworkManager (ADR-0001): a Backend calls
//! [`Broker::ask`] and waits. Prompts open and close as events, never carry a
//! secret, and a prompt still open is replayed to every Front end that
//! subscribes later, so one that starts after the request still sees it.
//! The one place a secret travels is [`Resolution::Answered`], in a zeroizing
//! [`Secret`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use nexus_ipc::{ErrorCode, Event, Failure, Prompt, PromptClosed, PromptKind, Secret};
use tokio::sync::{broadcast, oneshot};

use crate::config::Settings;
use crate::text;

/// What a Backend wants to know.
#[derive(Debug, Clone)]
pub struct Request {
    pub kind: PromptKind,
    /// The Network's name, for the sentence.
    pub network: String,
    /// `WifiNetwork.id`.
    pub network_id: String,
    /// The Backend says the last secret was wrong (NetworkManager's
    /// `REQUEST_NEW`).
    pub retry: bool,
}

/// How an [`Broker::ask`] ended.
#[derive(Debug)]
pub enum Resolution {
    Answered(Secret),
    Cancelled,
    Expired,
    /// The Backend gave up on the request.
    Withdrawn,
}

/// [`Resolution`] without the secret, for remembering how a prompt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Answered,
    Cancelled,
    Expired,
    Withdrawn,
}

#[derive(Debug)]
struct Pending {
    prompt: Prompt,
    /// Who is waiting, to withdraw the prompt when they stop caring.
    key: String,
    reply: oneshot::Sender<Resolution>,
}

#[derive(Debug, Default)]
struct State {
    next_id: u64,
    pending: HashMap<u64, Pending>,
    /// Networks whose last secret the Backend found wrong: the next prompt
    /// for them says so even if the Backend did not flag it.
    rejected: Vec<String>,
    /// How the latest prompt for each Network ended.
    ended: HashMap<String, Ended>,
    /// Profiles (by uuid) being joined with a password the caller supplied:
    /// a wrong one is an error for that caller, not a reason to prompt.
    inline: Vec<String>,
}

/// See [`Broker::inline_password`].
#[derive(Debug)]
pub struct InlineGuard<'a> {
    broker: &'a Broker,
    uuid: String,
}

impl Drop for InlineGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.broker.state.lock().expect("broker lock");
        if let Some(i) = state.inline.iter().position(|u| *u == self.uuid) {
            state.inline.remove(i);
        }
    }
}

pub struct Broker {
    state: Mutex<State>,
    events: broadcast::Sender<Event>,
    settings: Arc<Settings>,
}

impl std::fmt::Debug for Broker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Broker").finish_non_exhaustive()
    }
}

impl Broker {
    pub fn new(events: broadcast::Sender<Event>, settings: Arc<Settings>) -> Self {
        Self {
            state: Mutex::new(State::default()),
            events,
            settings,
        }
    }

    /// An event feed that starts with the prompts still open. Taken under the
    /// lock, so a prompt is in the backlog or in the feed, never both or neither.
    pub fn subscribe(&self) -> (broadcast::Receiver<Event>, Vec<Event>) {
        let state = self.state.lock().expect("broker lock");
        let rx = self.events.subscribe();
        let mut open: Vec<&Pending> = state.pending.values().collect();
        open.sort_by_key(|p| p.prompt.id);
        let backlog = open
            .into_iter()
            .map(|p| Event::SecretPromptOpened {
                prompt: p.prompt.clone(),
            })
            .collect();
        (rx, backlog)
    }

    /// Opens a prompt and waits until it is answered, cancelled, withdrawn
    /// or expires. `key` names the asker, so [`Broker::withdraw`] can find
    /// the prompt. Dropping the returned future withdraws the prompt.
    pub async fn ask(&self, key: &str, req: Request) -> Resolution {
        let timeout = self.settings.prompt_timeout();
        let (reply, answer) = oneshot::channel();
        let id = {
            let mut state = self.state.lock().expect("broker lock");
            state.next_id += 1;
            let id = state.next_id;
            let was_rejected = state.rejected.iter().position(|n| n == &req.network_id);
            if let Some(i) = was_rejected {
                state.rejected.remove(i);
            }
            let retry = req.retry || was_rejected.is_some();
            let prompt = Prompt {
                id,
                kind: req.kind,
                message: text::password_prompt(&req.network, retry),
                network: req.network,
                network_id: req.network_id,
                retry,
                expires_in: timeout.as_secs(),
            };
            // A stale prompt for the same asker is replaced, not stacked.
            let stale: Vec<u64> = state
                .pending
                .iter()
                .filter(|(_, p)| p.key == key)
                .map(|(id, _)| *id)
                .collect();
            for old in stale {
                self.close(&mut state, old, Ended::Withdrawn);
            }
            let _ = self.events.send(Event::SecretPromptOpened {
                prompt: prompt.clone(),
            });
            state.pending.insert(
                id,
                Pending {
                    prompt,
                    key: key.to_owned(),
                    reply,
                },
            );
            id
        };
        // If this future is dropped, the guard closes the prompt.
        let guard = Withdraw { broker: self, id };
        let resolution = tokio::select! {
            r = answer => r.unwrap_or(Resolution::Withdrawn),
            () = tokio::time::sleep(timeout) => Resolution::Expired,
        };
        if matches!(resolution, Resolution::Expired) {
            let mut state = self.state.lock().expect("broker lock");
            self.close(&mut state, id, Ended::Expired);
        }
        drop(guard);
        resolution
    }

    /// The answer to a prompt, from a Front end.
    pub fn answer(&self, id: u64, secret: Secret) -> Result<(), Failure> {
        let mut state = self.state.lock().expect("broker lock");
        let pending = self.take(&mut state, id, Ended::Answered)?;
        let _ = pending.reply.send(Resolution::Answered(secret));
        Ok(())
    }

    /// A Front end declining a prompt.
    pub fn cancel(&self, id: u64) -> Result<(), Failure> {
        let mut state = self.state.lock().expect("broker lock");
        let pending = self.take(&mut state, id, Ended::Cancelled)?;
        let _ = pending.reply.send(Resolution::Cancelled);
        Ok(())
    }

    /// The Backend no longer needs what `key` asked for.
    pub fn withdraw(&self, key: &str) {
        let mut state = self.state.lock().expect("broker lock");
        let ids: Vec<u64> = state
            .pending
            .iter()
            .filter(|(_, p)| p.key == key)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.close(&mut state, id, Ended::Withdrawn);
        }
    }

    /// Closes every open prompt: the Backend that asked is gone (or came back
    /// without remembering it asked).
    pub fn withdraw_all(&self) {
        let mut state = self.state.lock().expect("broker lock");
        let ids: Vec<u64> = state.pending.keys().copied().collect();
        for id in ids {
            self.close(&mut state, id, Ended::Withdrawn);
        }
    }

    /// Notes that the Backend rejected the last secret for this Network, so
    /// the next prompt for it reads as a retry.
    pub fn mark_rejected(&self, network_id: &str) {
        let mut state = self.state.lock().expect("broker lock");
        if !state.rejected.iter().any(|n| n == network_id) {
            state.rejected.push(network_id.to_owned());
        }
    }

    /// How the latest prompt for this Network ended, once.
    pub fn take_ended(&self, network_id: &str) -> Option<Ended> {
        self.state
            .lock()
            .expect("broker lock")
            .ended
            .remove(network_id)
    }

    /// Marks the Profile `uuid` as joined with a password the caller
    /// supplied, until the guard drops. While marked, [`Broker::is_inline`]
    /// is true and the Backend should not prompt for a replacement.
    pub fn inline_password(&self, uuid: &str) -> InlineGuard<'_> {
        self.state
            .lock()
            .expect("broker lock")
            .inline
            .push(uuid.to_owned());
        InlineGuard {
            broker: self,
            uuid: uuid.to_owned(),
        }
    }

    pub fn is_inline(&self, uuid: &str) -> bool {
        let state = self.state.lock().expect("broker lock");
        state.inline.iter().any(|u| u == uuid)
    }

    /// Forgets how earlier prompts ended, before a fresh attempt.
    pub fn forget_ended(&self, network_id: &str) {
        let mut state = self.state.lock().expect("broker lock");
        state.ended.remove(network_id);
        state.rejected.retain(|n| n != network_id);
    }

    /// Removes a prompt for a Front end's reply; `unknown_prompt` if it is gone.
    fn take(&self, state: &mut State, id: u64, ended: Ended) -> Result<Pending, Failure> {
        let Some(pending) = state.pending.remove(&id) else {
            return Err(Failure {
                code: ErrorCode::UnknownPrompt,
                message: text::unknown_prompt(),
            });
        };
        self.finished(state, &pending.prompt, ended);
        Ok(pending)
    }

    /// Closes a prompt nobody replied to.
    fn close(&self, state: &mut State, id: u64, ended: Ended) {
        if let Some(pending) = state.pending.remove(&id) {
            self.finished(state, &pending.prompt, ended);
            let _ = pending.reply.send(Resolution::Withdrawn);
        }
    }

    fn finished(&self, state: &mut State, prompt: &Prompt, ended: Ended) {
        state.ended.insert(prompt.network_id.clone(), ended);
        let _ = self.events.send(Event::SecretPromptClosed {
            id: prompt.id,
            reason: match ended {
                Ended::Answered => PromptClosed::Answered,
                Ended::Cancelled => PromptClosed::Cancelled,
                Ended::Expired => PromptClosed::Expired,
                Ended::Withdrawn => PromptClosed::Withdrawn,
            },
        });
    }
}

/// Closes the prompt if the asker stops waiting.
struct Withdraw<'a> {
    broker: &'a Broker,
    id: u64,
}

impl Drop for Withdraw<'_> {
    fn drop(&mut self) {
        let mut state = self.broker.state.lock().expect("broker lock");
        self.broker.close(&mut state, self.id, Ended::Withdrawn);
    }
}
