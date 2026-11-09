//! 802.1X (EAP) settings: one description for Wi-Fi Enterprise now and wired
//! 802.1X later. Front ends validate with [`Eap::validate`] as the person
//! types; the Daemon runs the same check before it touches the Backend.

use serde::{Deserialize, Serialize};

use crate::Secret;

/// The outer method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EapMethod {
    Peap,
    Ttls,
    Tls,
}

impl EapMethod {
    pub const ALL: [EapMethod; 3] = [EapMethod::Peap, EapMethod::Ttls, EapMethod::Tls];

    pub fn label(self) -> &'static str {
        match self {
            EapMethod::Peap => "PEAP",
            EapMethod::Ttls => "TTLS",
            EapMethod::Tls => "TLS",
        }
    }

    /// The inner methods that go with this outer one; the first is the default.
    pub fn inner(self) -> &'static [Phase2] {
        match self {
            EapMethod::Peap => &[Phase2::Mschapv2, Phase2::Gtc, Phase2::Md5],
            EapMethod::Ttls => &[Phase2::Mschapv2, Phase2::Pap, Phase2::Mschap, Phase2::Chap],
            EapMethod::Tls => &[],
        }
    }
}

/// The inner (phase 2) method of PEAP and TTLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase2 {
    Mschapv2,
    Mschap,
    Pap,
    Chap,
    Gtc,
    Md5,
}

impl Phase2 {
    pub fn label(self) -> &'static str {
        match self {
            Phase2::Mschapv2 => "MSCHAPv2",
            Phase2::Mschap => "MSCHAP",
            Phase2::Pap => "PAP",
            Phase2::Chap => "CHAP",
            Phase2::Gtc => "GTC",
            Phase2::Md5 => "MD5",
        }
    }
}

/// A form field, so a Front end can put an error beside the right input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EapField {
    Phase2,
    Identity,
    Password,
    CaCert,
    DomainSuffix,
    ClientCert,
    PrivateKey,
}

/// One thing wrong with the settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EapError {
    pub field: EapField,
    pub message: String,
}

/// Enterprise sign-in settings. Certificates are file paths NetworkManager
/// reads itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Eap {
    pub method: EapMethod,
    /// Inner method; required for PEAP and TTLS, absent for TLS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase2: Option<Phase2>,
    pub identity: String,
    /// The outer identity sent in the clear (eduroam: `anonymous@uni.example`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anonymous_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<Secret>,
    /// CA certificate file (PEM or DER). Without one the system's CAs are
    /// used, which needs `domain_suffix`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_cert: Option<String>,
    /// The server name (or its tail) the certificate must match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain_suffix: Option<String>,
    /// Explicitly do not check the server at all. Anyone can then pose as it.
    #[serde(default)]
    pub no_ca_validation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_cert: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key_password: Option<Secret>,
}

fn blank(s: &Option<String>) -> bool {
    s.as_deref().is_none_or(|s| s.trim().is_empty())
}

fn missing_file(path: &str) -> bool {
    std::fs::File::open(path).is_err()
}

impl Eap {
    /// A form with the usual defaults for `method`: eduroam-style PEAP with
    /// MSCHAPv2 and server checking on.
    pub fn new(method: EapMethod) -> Eap {
        Eap {
            method,
            phase2: method.inner().first().copied(),
            identity: String::new(),
            anonymous_identity: None,
            password: None,
            ca_cert: None,
            domain_suffix: None,
            no_ca_validation: false,
            client_cert: None,
            private_key: None,
            private_key_password: None,
        }
    }

    /// Whether the server's certificate goes unchecked.
    pub fn skips_validation(&self) -> bool {
        self.no_ca_validation
    }

    /// Everything wrong with the settings, in form order; empty when they can
    /// be tried. Looks at the files named, so run it where the person's files are.
    pub fn validate(&self) -> Vec<EapError> {
        let mut out = Vec::new();
        let mut bad = |field, message: &str| {
            out.push(EapError {
                field,
                message: message.into(),
            });
        };
        let inner = self.method.inner();
        match (self.phase2, inner.is_empty()) {
            (None, false) => bad(EapField::Phase2, "Pick the inner method."),
            (Some(p), false) if !inner.contains(&p) => bad(
                EapField::Phase2,
                &format!("{} cannot run inside {}.", p.label(), self.method.label()),
            ),
            (Some(_), true) => bad(EapField::Phase2, "TLS has no inner method."),
            _ => {}
        }
        if self.identity.trim().is_empty() {
            bad(
                EapField::Identity,
                "Enter your identity, usually your email or username.",
            );
        }
        if self.method != EapMethod::Tls
            && self.password.as_ref().is_none_or(|p| p.expose().is_empty())
        {
            bad(EapField::Password, "Enter your password.");
        }
        if !self.no_ca_validation {
            match self.ca_cert.as_deref().filter(|p| !p.trim().is_empty()) {
                Some(path) if missing_file(path) => {
                    bad(EapField::CaCert, "That CA certificate file cannot be read.");
                }
                Some(_) => {}
                None if blank(&self.domain_suffix) => bad(
                    EapField::DomainSuffix,
                    "Enter the server name (ask your IT desk), or pick a CA certificate.",
                ),
                None => {}
            }
        }
        if self.method == EapMethod::Tls {
            for (field, path, what) in [
                (
                    EapField::ClientCert,
                    &self.client_cert,
                    "client certificate",
                ),
                (EapField::PrivateKey, &self.private_key, "private key"),
            ] {
                match path.as_deref().filter(|p| !p.trim().is_empty()) {
                    None => bad(field, &format!("Pick your {what} file.")),
                    Some(p) if missing_file(p) => {
                        bad(field, &format!("That {what} file cannot be read."));
                    }
                    Some(_) => {}
                }
            }
        }
        out
    }
}
