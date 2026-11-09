//! The optional `config.toml`: defaults, validation, live reload and
//! in-place edits. Nothing here may stop the Daemon from starting.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nexus_ipc::{ErrorCode, Event, Failure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

/// Every option, with its default. A missing file or key means the default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Name of the TUI theme.
    pub theme: String,
    pub connectivity: ConnectivityCheck,
    pub menu: Menu,
    pub prompts: Prompts,
    pub doctor: DoctorConfig,
}

/// What the Doctor probes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DoctorConfig {
    /// The gateway whose answer is tested: `auto` (the primary Link's, else
    /// the default route), `off`, or an IPv4/IPv6 address.
    pub gateway: String,
}

impl Default for DoctorConfig {
    fn default() -> Self {
        Self {
            gateway: "auto".into(),
        }
    }
}

/// Secret prompts (password questions from the Backend).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Prompts {
    /// Seconds a prompt waits for an answer before it expires (1 to 3600).
    pub timeout_secs: u64,
}

impl Default for Prompts {
    fn default() -> Self {
        Self { timeout_secs: 120 }
    }
}

/// The one outside contact Nexus allows itself, and only through the Backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectivityCheck {
    /// `false` switches the check off.
    pub enabled: bool,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Menu {
    /// The launcher `nexus menu` drives; `None` auto-detects.
    pub launcher: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "midnight-bloom".into(),
            connectivity: ConnectivityCheck::default(),
            menu: Menu::default(),
            prompts: Prompts::default(),
            doctor: DoctorConfig::default(),
        }
    }
}

impl Default for ConnectivityCheck {
    fn default() -> Self {
        Self {
            enabled: true,
            url: "https://ping.archlinux.org/nm-check.txt".into(),
        }
    }
}

/// Parses and validates a config file's text. The error names the problem
/// in words a user can act on.
pub fn parse(text: &str) -> Result<Config, String> {
    let config: Config = toml::from_str(text).map_err(|e| {
        let line = e
            .span()
            .map(|s| text[..s.start].matches('\n').count() + 1)
            .map_or(String::new(), |l| format!(" (line {l})"));
        format!("{}{line}", e.message())
    })?;
    let url = &config.connectivity.url;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!(
            "connectivity.url must start with http:// or https://, got `{url}`"
        ));
    }
    let gateway = &config.doctor.gateway;
    if !matches!(gateway.as_str(), "auto" | "off") && gateway.parse::<std::net::IpAddr>().is_err() {
        return Err(format!(
            "doctor.gateway must be `auto`, `off` or an IP address, got `{gateway}`"
        ));
    }
    let secs = config.prompts.timeout_secs;
    if !(1..=3600).contains(&secs) {
        return Err(format!(
            "prompts.timeout_secs must be between 1 and 3600, got {secs}"
        ));
    }
    Ok(config)
}

/// `$XDG_CONFIG_HOME/nexus/config.toml`, else `~/.config/nexus/config.toml`.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("nexus").join("config.toml"))
}

#[derive(Debug)]
struct Loaded {
    /// The file's text when last read; reloads that change nothing are silent.
    raw: Option<String>,
    config: Config,
    warning: Option<String>,
}

/// The live config: what is in force, and why.
#[derive(Debug)]
pub struct Settings {
    path: Option<PathBuf>,
    state: Mutex<Loaded>,
    events: broadcast::Sender<Event>,
}

impl Settings {
    /// Reads the file once. A broken or unreadable file yields defaults and a warning.
    pub fn load(path: Option<PathBuf>, events: broadcast::Sender<Event>) -> Self {
        let settings = Self {
            path,
            state: Mutex::new(Loaded {
                raw: None,
                config: Config::default(),
                warning: None,
            }),
            events,
        };
        settings.refresh(false);
        settings
    }

    /// How long a Secret prompt waits for an answer.
    pub fn prompt_timeout(&self) -> std::time::Duration {
        let state = self.state.lock().expect("settings lock");
        std::time::Duration::from_secs(state.config.prompts.timeout_secs)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Re-reads the file; announces a `config_reloaded` event if it changed.
    pub fn reload(&self) {
        self.refresh(true);
    }

    fn refresh(&self, announce: bool) {
        let read = match &self.path {
            None => Ok(None),
            Some(p) => match std::fs::read_to_string(p) {
                Ok(text) => Ok(Some(text)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(format!("cannot read {}: {e}", p.display())),
            },
        };
        let mut state = self.state.lock().expect("settings lock");
        let (raw, config, warning) = match read {
            Ok(None) => (None, Config::default(), None),
            Ok(Some(text)) => match parse(&text) {
                Ok(config) => (Some(text), config, None),
                Err(problem) => (
                    Some(text),
                    Config::default(),
                    Some(format!("config.toml is being ignored: {problem}")),
                ),
            },
            Err(problem) => (None, Config::default(), Some(problem)),
        };
        if state.raw == raw && state.warning == warning {
            return;
        }
        *state = Loaded {
            raw,
            config,
            warning,
        };
        if let Some(w) = &state.warning {
            tracing::warn!("{w}");
        }
        if announce {
            // No subscribers is fine.
            let _ = self.events.send(Event::ConfigReloaded {
                config: to_value(&state.config),
                warning: state.warning.clone(),
            });
        }
    }

    /// The `config_get` result: `{"value": ..., "warning"?: ...}`.
    pub fn get(&self, key: Option<&str>) -> Result<Value, Failure> {
        let state = self.state.lock().expect("settings lock");
        answer(&state, key)
    }

    /// Sets `key` (`null` resets it to the default) by editing the file in
    /// place, so comments and ordering survive.
    pub fn set(&self, key: &str, value: &Value) -> Result<Value, Failure> {
        let bad = |message: String| Failure {
            code: ErrorCode::BadRequest,
            message,
        };
        let path = self
            .path
            .as_deref()
            .ok_or_else(|| bad("no config location: HOME is not set".into()))?;
        let mut state = self.state.lock().expect("settings lock");
        let old = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(bad(format!("cannot read {}: {e}", path.display()))),
        };
        if let Err(problem) = parse(&old) {
            return Err(bad(format!(
                "config.toml is invalid, fix it first: {problem}"
            )));
        }
        let mut doc: toml_edit::DocumentMut = old
            .parse()
            .map_err(|e| bad(format!("config.toml is invalid, fix it first: {e}")))?;
        edit(&mut doc, key, value).map_err(bad)?;
        let new = doc.to_string();
        let config = parse(&new).map_err(|p| bad(format!("cannot set `{key}`: {p}")))?;
        write_atomically(path, &new).map_err(|e| Failure {
            code: ErrorCode::Internal,
            message: format!("cannot write {}: {e}", path.display()),
        })?;
        *state = Loaded {
            raw: Some(new),
            config,
            warning: None,
        };
        let _ = self.events.send(Event::ConfigReloaded {
            config: to_value(&state.config),
            warning: None,
        });
        answer(&state, Some(key))
    }
}

fn to_value(config: &Config) -> Value {
    serde_json::to_value(config).expect("config serializes")
}

fn answer(state: &Loaded, key: Option<&str>) -> Result<Value, Failure> {
    let mut value = to_value(&state.config);
    for part in key.into_iter().flat_map(|k| k.split('.')) {
        value = value.get(part).cloned().ok_or_else(|| Failure {
            code: ErrorCode::BadRequest,
            message: format!("unknown config key `{}`", key.unwrap_or_default()),
        })?;
    }
    let mut out = serde_json::json!({ "value": value });
    if let Some(w) = &state.warning {
        out["warning"] = Value::String(w.clone());
    }
    Ok(out)
}

fn edit(doc: &mut toml_edit::DocumentMut, key: &str, value: &Value) -> Result<(), String> {
    let mut parts: Vec<&str> = key.split('.').collect();
    let last = parts.pop().filter(|l| !l.is_empty()).ok_or("empty key")?;
    let mut table = doc.as_table_mut();
    for part in parts {
        table = table
            .entry(part)
            .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
            .as_table_mut()
            .ok_or_else(|| format!("`{part}` is not a section"))?;
    }
    let item = match value {
        Value::Null => {
            table.remove(last);
            return Ok(());
        }
        Value::Bool(b) => toml_edit::value(*b),
        Value::String(s) => toml_edit::value(s.as_str()),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => toml_edit::value(i),
            (None, Some(f)) => toml_edit::value(f),
            _ => return Err("unsupported number".into()),
        },
        _ => return Err("only strings, numbers, booleans and null can be set".into()),
    };
    // Replace the value but keep the key's comments and position.
    match table.get_mut(last) {
        Some(existing) if existing.is_value() => {
            let decor = existing.as_value().map(|v| v.decor().clone());
            *existing = item;
            if let (Some(decor), Some(v)) = (decor, existing.as_value_mut()) {
                *v.decor_mut() = decor;
            }
        }
        _ => {
            table.insert(last, item);
        }
    }
    Ok(())
}

fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    let dir = path.parent().expect("config path has a parent");
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(".config.toml.nexus-tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Reloads the config whenever its directory changes. Watching the directory
/// (not the file) survives editors that save by renaming a temp file over it.
/// Failing to watch only costs live reload, never startup.
pub async fn watch(settings: std::sync::Arc<Settings>) {
    if let Err(e) = watch_inner(&settings).await {
        tracing::warn!("config changes will not be picked up live: {e}");
    }
}

/// How long the directory must stay quiet before the file is re-read.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

async fn watch_inner(settings: &Settings) -> io::Result<()> {
    use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
    use std::mem::MaybeUninit;

    let Some(dir) = settings.path().and_then(Path::parent) else {
        return Ok(());
    };
    std::fs::create_dir_all(dir)?;
    let fd = inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC)?;
    inotify::add_watch(
        &fd,
        dir,
        WatchFlags::CLOSE_WRITE
            | WatchFlags::MOVED_TO
            | WatchFlags::MOVED_FROM
            | WatchFlags::CREATE
            | WatchFlags::MODIFY
            | WatchFlags::DELETE,
    )?;
    // tokio deprecates the safe constructor; `register` is `unsafe`, which this
    // workspace forbids. An owned inotify fd satisfies what the lint is about.
    #[allow(deprecated)]
    let fd = tokio::io::unix::AsyncFd::new(fd)?;
    let mut buf = [MaybeUninit::<u8>::uninit(); 4096];
    loop {
        // Wait for a change, then for the directory to go quiet: an editor
        // that writes in place leaves a truncated or half-written file until
        // it finishes, and reading that would report a bogus parse warning.
        let mut changed = true;
        while changed {
            let mut guard = fd.readable().await?;
            // Drain what is queued; the reload compares file contents, so the
            // individual events do not matter.
            let mut reader = inotify::Reader::new(fd.get_ref(), &mut buf);
            loop {
                match reader.next() {
                    Ok(_) => {}
                    Err(rustix::io::Errno::AGAIN) => {
                        guard.clear_ready();
                        break;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            drop(guard);
            changed = tokio::time::timeout(SETTLE, fd.readable()).await.is_ok();
        }
        settings.reload();
    }
}
