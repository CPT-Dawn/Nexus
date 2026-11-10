//! Portal support: what the Link is behind, the page that logs in, and the
//! browser launcher.
//!
//! NetworkManager says "portal" but not where the login page is. A Portal
//! redirects plain `http://` requests to its page, so the page to open is
//! the connectivity check URL on `http://`. An `https://` request would
//! only meet a certificate error.

use std::sync::Arc;

use nexus_ipc::{Connectivity, Portal, Status};

use crate::config::Settings;

/// How the Daemon opens a page for the user. A trait so tests never start a
/// real browser.
pub trait Opener: Send + Sync + std::fmt::Debug {
    fn open(&self, url: &str) -> Result<(), String>;
}

/// Shared handle to the opener.
pub type SharedOpener = Arc<dyn Opener>;

/// Opens pages with `xdg-open`.
#[derive(Debug)]
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, url: &str) -> Result<(), String> {
        crate::sysutil::spawn_detached(std::process::Command::new("xdg-open").arg(url))
            .map_err(|e| format!("Could not start xdg-open ({e}). Open {url} in a browser."))?;
        Ok(())
    }
}

/// The Portal the status says the Link is behind. `None` while the
/// connectivity check is switched off in the config: nothing is probed, so
/// a Portal is not Nexus's to announce.
pub fn of_status(status: &Status, settings: &Settings) -> Option<Portal> {
    let check = settings.connectivity();
    if !check.enabled || status.connectivity != Connectivity::Portal {
        return None;
    }
    let link = status.link.as_ref()?;
    Some(Portal {
        network: link.name.clone(),
        url: login_url(&check.url),
    })
}

/// The check URL on plain `http://`.
pub fn login_url(check_url: &str) -> String {
    match check_url.strip_prefix("https://") {
        Some(rest) => format!("http://{rest}"),
        None => check_url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_login_page_is_the_check_url_on_plain_http() {
        assert_eq!(
            login_url("https://ping.archlinux.org/nm-check.txt"),
            "http://ping.archlinux.org/nm-check.txt"
        );
        assert_eq!(login_url("http://x.test/a"), "http://x.test/a");
    }
}
