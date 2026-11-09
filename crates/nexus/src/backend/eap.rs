//! NetworkManager's `802-1x` setting for a set of [`Eap`] choices. Shared by
//! Wi-Fi Enterprise and, later, wired 802.1X: both put this section beside
//! their own.

use std::collections::HashMap;

use nexus_ipc::{Eap, EapMethod, Phase2};
use zbus::OwnedVariant;

fn variant<'a>(v: impl Into<zbus::Variant<'a>>) -> OwnedVariant {
    OwnedVariant::try_from(v.into()).expect("plain values have no file descriptors")
}

fn phase2(p: Phase2) -> &'static str {
    match p {
        Phase2::Mschapv2 => "mschapv2",
        Phase2::Mschap => "mschap",
        Phase2::Pap => "pap",
        Phase2::Chap => "chap",
        Phase2::Gtc => "gtc",
        Phase2::Md5 => "md5",
    }
}

/// A certificate or key setting: NetworkManager wants `file://<path>` and a
/// trailing NUL as bytes (the "path" scheme).
fn path_bytes(path: &str) -> Vec<u8> {
    let mut v = format!("file://{}", path.trim()).into_bytes();
    v.push(0);
    v
}

/// The `802-1x` section. Secrets are system-owned (flags 0), so they are
/// stored with the Profile and autoconnect works before login.
pub fn setting(eap: &Eap) -> HashMap<String, OwnedVariant> {
    let method = match eap.method {
        EapMethod::Peap => "peap",
        EapMethod::Ttls => "ttls",
        EapMethod::Tls => "tls",
    };
    let mut s = HashMap::from([
        ("eap".to_owned(), variant(vec![method.to_owned()])),
        ("identity".to_owned(), variant(eap.identity.trim())),
    ]);
    let text = |s: &mut HashMap<String, OwnedVariant>, key: &str, value: &Option<String>| {
        if let Some(v) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            s.insert(key.to_owned(), variant(v));
        }
    };
    text(&mut s, "anonymous-identity", &eap.anonymous_identity);
    if let Some(p) = eap.phase2 {
        s.insert("phase2-auth".to_owned(), variant(phase2(p)));
    }
    if eap.method != EapMethod::Tls
        && let Some(pw) = &eap.password
    {
        s.insert("password".to_owned(), variant(pw.expose()));
        s.insert("password-flags".to_owned(), variant(0u32));
    }
    // Server checking: the given CA, else the system's CAs pinned by name,
    // else (only when asked for) nothing.
    let ca = eap
        .ca_cert
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    if !eap.no_ca_validation {
        match ca {
            Some(path) => {
                s.insert("ca-cert".to_owned(), variant(path_bytes(path)));
                s.insert("system-ca-certs".to_owned(), variant(false));
            }
            None => {
                s.insert("system-ca-certs".to_owned(), variant(true));
            }
        }
        text(&mut s, "domain-suffix-match", &eap.domain_suffix);
    } else {
        s.insert("system-ca-certs".to_owned(), variant(false));
    }
    if eap.method == EapMethod::Tls {
        for (key, path) in [
            ("client-cert", &eap.client_cert),
            ("private-key", &eap.private_key),
        ] {
            if let Some(p) = path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
                s.insert(key.to_owned(), variant(path_bytes(p)));
            }
        }
        if let Some(pw) = &eap.private_key_password {
            s.insert("private-key-password".to_owned(), variant(pw.expose()));
            s.insert("private-key-password-flags".to_owned(), variant(0u32));
        } else {
            // An unencrypted key: without this NetworkManager asks for a
            // password nobody has (NM_SETTING_SECRET_FLAG_NOT_REQUIRED).
            s.insert("private-key-password-flags".to_owned(), variant(4u32));
        }
    }
    s
}
