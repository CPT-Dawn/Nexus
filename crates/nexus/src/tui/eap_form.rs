//! The 802.1X form's state: which rows show, what was typed, which errors are
//! visible. No drawing and no I/O; `eap_ui` draws it, `App` feeds it keys and
//! sends the result. It knows nothing about Wi-Fi beyond a name to show, so
//! wired 802.1X can open the same form.

use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nexus_ipc::{Eap, EapError, EapField, EapMethod, Phase2, Secret};
use zeroize::Zeroizing;

/// One row of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Method,
    Phase2,
    Identity,
    Anonymous,
    Password,
    CaCert,
    Domain,
    SkipCheck,
    ClientCert,
    PrivateKey,
    KeyPassword,
}

impl Row {
    pub fn label(self) -> &'static str {
        match self {
            Row::Method => "Method",
            Row::Phase2 => "Inner method",
            Row::Identity => "Identity",
            Row::Anonymous => "Anonymous id",
            Row::Password => "Password",
            Row::CaCert => "CA certificate",
            Row::Domain => "Server name",
            Row::SkipCheck => "Skip check",
            Row::ClientCert => "Client cert",
            Row::PrivateKey => "Private key",
            Row::KeyPassword => "Key password",
        }
    }

    /// A short example or explanation shown while the row is empty.
    pub fn hint(self) -> &'static str {
        match self {
            Row::Identity => "e.g. you@uni.example",
            Row::Anonymous => "e.g. anonymous@uni.example (optional)",
            Row::CaCert => "empty: use the system CAs",
            Row::Domain => "e.g. radius.uni.example",
            Row::ClientCert => "e.g. /home/you/certs/me.pem",
            Row::PrivateKey => "e.g. /home/you/certs/me.key",
            Row::KeyPassword => "optional",
            _ => "",
        }
    }

    pub fn secret(self) -> bool {
        matches!(self, Row::Password | Row::KeyPassword)
    }

    fn field(self) -> Option<EapField> {
        Some(match self {
            Row::Phase2 => EapField::Phase2,
            Row::Identity => EapField::Identity,
            Row::Password => EapField::Password,
            Row::CaCert => EapField::CaCert,
            Row::Domain => EapField::DomainSuffix,
            Row::ClientCert => EapField::ClientCert,
            Row::PrivateKey => EapField::PrivateKey,
            _ => return None,
        })
    }
}

/// The guided form for an enterprise sign-in.
pub struct EapForm {
    /// The Wi-Fi `network` id the Front end joins when it is submitted.
    pub network: String,
    /// What the form is for, shown in its title.
    pub name: String,
    method: EapMethod,
    phase2: Option<Phase2>,
    identity: String,
    anonymous: String,
    password: Zeroizing<String>,
    ca_cert: String,
    domain: String,
    skip_check: bool,
    client_cert: String,
    private_key: String,
    key_password: Zeroizing<String>,
    focus: usize,
    /// Rows the person has left or tried to submit past: only their errors show.
    seen: Vec<Row>,
    /// Passwords drawn in the clear.
    pub reveal: bool,
    /// The last join's failure, shown at the top.
    pub banner: Option<String>,
}

impl fmt::Debug for EapForm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EapForm")
            .field("name", &self.name)
            .field("method", &self.method)
            .field("secrets", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl EapForm {
    pub fn new(network: String, name: String) -> EapForm {
        let method = EapMethod::Peap;
        EapForm {
            network,
            name,
            method,
            phase2: method.inner().first().copied(),
            identity: String::new(),
            anonymous: String::new(),
            password: Zeroizing::new(String::new()),
            ca_cert: String::new(),
            domain: String::new(),
            skip_check: false,
            client_cert: String::new(),
            private_key: String::new(),
            key_password: Zeroizing::new(String::new()),
            focus: 2, // Identity: where typing starts
            seen: Vec::new(),
            reveal: false,
            banner: None,
        }
    }

    /// The form again after a failed join, with everything the person typed.
    pub fn reopened(network: String, name: String, eap: &Eap, banner: String) -> EapForm {
        let mut form = EapForm::new(network, name);
        form.method = eap.method;
        form.phase2 = eap.phase2;
        form.identity.clone_from(&eap.identity);
        form.anonymous = eap.anonymous_identity.clone().unwrap_or_default();
        form.password = Zeroizing::new(
            eap.password
                .as_ref()
                .map(|p| p.expose().to_owned())
                .unwrap_or_default(),
        );
        form.ca_cert = eap.ca_cert.clone().unwrap_or_default();
        form.domain = eap.domain_suffix.clone().unwrap_or_default();
        form.skip_check = eap.no_ca_validation;
        form.client_cert = eap.client_cert.clone().unwrap_or_default();
        form.private_key = eap.private_key.clone().unwrap_or_default();
        form.key_password = Zeroizing::new(
            eap.private_key_password
                .as_ref()
                .map(|p| p.expose().to_owned())
                .unwrap_or_default(),
        );
        form.banner = Some(banner);
        form
    }

    pub fn method(&self) -> EapMethod {
        self.method
    }

    pub fn phase2(&self) -> Option<Phase2> {
        self.phase2
    }

    pub fn skips_check(&self) -> bool {
        self.skip_check
    }

    /// The rows on screen for the chosen method, in order.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Method];
        if self.method == EapMethod::Tls {
            rows.extend([
                Row::Identity,
                Row::ClientCert,
                Row::PrivateKey,
                Row::KeyPassword,
            ]);
        } else {
            rows.extend([Row::Phase2, Row::Identity, Row::Anonymous, Row::Password]);
        }
        rows.extend([Row::CaCert, Row::Domain, Row::SkipCheck]);
        rows
    }

    pub fn focused(&self) -> Row {
        let rows = self.rows();
        rows[self.focus.min(rows.len() - 1)]
    }

    /// The text of a row (secrets included: the drawing masks them).
    pub fn text(&self, row: Row) -> &str {
        match row {
            Row::Identity => &self.identity,
            Row::Anonymous => &self.anonymous,
            Row::Password => &self.password,
            Row::CaCert => &self.ca_cert,
            Row::Domain => &self.domain,
            Row::ClientCert => &self.client_cert,
            Row::PrivateKey => &self.private_key,
            Row::KeyPassword => &self.key_password,
            Row::Method | Row::Phase2 | Row::SkipCheck => "",
        }
    }

    fn buffer(&mut self, row: Row) -> Option<&mut String> {
        Some(match row {
            Row::Identity => &mut self.identity,
            Row::Anonymous => &mut self.anonymous,
            Row::Password => &mut self.password,
            Row::CaCert => &mut self.ca_cert,
            Row::Domain => &mut self.domain,
            Row::ClientCert => &mut self.client_cert,
            Row::PrivateKey => &mut self.private_key,
            Row::KeyPassword => &mut self.key_password,
            Row::Method | Row::Phase2 | Row::SkipCheck => return None,
        })
    }

    /// The settings as typed.
    pub fn to_eap(&self) -> Eap {
        let some = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_owned());
        let tls = self.method == EapMethod::Tls;
        Eap {
            method: self.method,
            phase2: self.phase2,
            identity: self.identity.trim().to_owned(),
            anonymous_identity: some(&self.anonymous).filter(|_| !tls),
            password: (!tls && !self.password.is_empty()).then(|| Secret::new(&**self.password)),
            ca_cert: some(&self.ca_cert),
            domain_suffix: some(&self.domain),
            no_ca_validation: self.skip_check,
            client_cert: some(&self.client_cert).filter(|_| tls),
            private_key: some(&self.private_key).filter(|_| tls),
            private_key_password: (tls && !self.key_password.is_empty())
                .then(|| Secret::new(&**self.key_password)),
        }
    }

    /// The error to show under `row`, if the person has been past it.
    pub fn error(&self, row: Row) -> Option<String> {
        if !self.seen.contains(&row) {
            return None;
        }
        let field = row.field()?;
        self.to_eap()
            .validate()
            .into_iter()
            .find(|e: &EapError| e.field == field)
            .map(|e| e.message)
    }

    fn see(&mut self, row: Row) {
        if !self.seen.contains(&row) {
            self.seen.push(row);
        }
    }

    fn go(&mut self, by: isize) {
        let leaving = self.focused();
        self.see(leaving);
        let n = self.rows().len() as isize;
        self.focus = (self.focus as isize + by).rem_euclid(n) as usize;
    }

    fn cycle(&mut self, forward: bool) {
        match self.focused() {
            Row::Method => {
                let all = EapMethod::ALL;
                let at = all.iter().position(|m| *m == self.method).unwrap_or(0);
                let next = all[(at + if forward { 1 } else { all.len() - 1 }) % all.len()];
                self.method = next;
                self.phase2 = next.inner().first().copied();
                self.focus = 0;
            }
            Row::Phase2 => {
                let inner = self.method.inner();
                let at = inner.iter().position(|p| Some(*p) == self.phase2).unwrap_or(0);
                self.phase2 = Some(inner[(at + if forward { 1 } else { inner.len() - 1 }) % inner.len()]);
            }
            Row::SkipCheck => self.skip_check = !self.skip_check,
            _ => {}
        }
    }

    /// Tries to submit: the settings when they are valid, else every error
    /// shows and the first bad row has the focus.
    pub fn submit(&mut self) -> Option<Eap> {
        let eap = self.to_eap();
        let errors = eap.validate();
        if errors.is_empty() {
            return Some(eap);
        }
        let rows = self.rows();
        for row in &rows {
            self.see(*row);
        }
        if let Some(at) = rows
            .iter()
            .position(|r| r.field() == Some(errors[0].field))
        {
            self.focus = at;
        }
        None
    }

    /// Handles a key. `Typed::Submit(eap)` and `Typed::Cancel` end the form.
    pub fn key(&mut self, key: KeyEvent) -> Typed {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let row = self.focused();
        match key.code {
            KeyCode::Esc => return Typed::Cancel,
            KeyCode::Enter => {
                if let Some(eap) = self.submit() {
                    return Typed::Submit(Box::new(eap));
                }
            }
            KeyCode::Tab | KeyCode::Down => self.go(1),
            KeyCode::BackTab | KeyCode::Up => self.go(-1),
            KeyCode::Left => self.cycle(false),
            KeyCode::Right => self.cycle(true),
            KeyCode::Char('r') if ctrl => self.reveal = !self.reveal,
            KeyCode::Char('u') if ctrl => {
                if let Some(b) = self.buffer(row) {
                    b.clear();
                }
            }
            KeyCode::Char(' ') if self.buffer(row).is_none() => self.cycle(true),
            KeyCode::Backspace => {
                if let Some(b) = self.buffer(row) {
                    b.pop();
                }
            }
            KeyCode::Char(c) if !ctrl => {
                if let Some(b) = self.buffer(row) {
                    b.push(c);
                }
            }
            _ => {}
        }
        Typed::Editing
    }
}

/// What a key did to the form.
#[derive(Debug)]
pub enum Typed {
    Editing,
    Cancel,
    Submit(Box<Eap>),
}
