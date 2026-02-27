use std::path::PathBuf;

use clap::Parser;
use eyre::{Result, WrapErr};
use ratatui::style::Color;
use serde::{Deserialize, Deserializer};
use tracing::info;

// ─── Embedded Default ────────────────────────────────────────────────────
/// Baked into the binary at compile time. The app can never crash due to a
/// missing config file — this is always available as the base layer.
const DEFAULT_CONFIG_TOML: &str = include_str!("../default_config.toml");

// ─── CLI Arguments (override layer) ─────────────────────────────────────
/// Nexus — A beautiful modern TUI WiFi manager
#[derive(Parser, Debug, Clone)]
#[command(name = "nexus", version, about, long_about = None)]
pub struct CliArgs {
    /// WiFi interface to use (overrides config file)
    #[arg(short, long)]
    pub interface: Option<String>,

    /// Log level filter (overrides config file)
    #[arg(short, long)]
    pub log_level: Option<String>,

    /// Disable Nerd Font icons (overrides config file)
    #[arg(long)]
    pub no_nerd_fonts: bool,

    /// Path to a custom config file
    #[arg(short, long)]
    pub config: Option<PathBuf>,

    /// Print the default config to stdout and exit
    #[arg(long)]
    pub print_default_config: bool,

    /// Target FPS for the render loop (overrides config file)
    #[arg(long)]
    pub fps: Option<u16>,
}

// ─── TOML Structs ───────────────────────────────────────────────────────

/// Root configuration — parsed from TOML, then overridden by CLI flags.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub appearance: AppearanceConfig,
    #[serde(default)]
    pub theme: ThemeConfig,
    #[serde(default)]
    pub keys: KeysConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// WiFi interface (empty string = auto-detect)
    #[serde(default)]
    pub interface: String,

    /// Log level
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Polling interval for NM signal listener (seconds)
    #[serde(default = "default_scan_interval")]
    pub scan_interval_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    /// Use Nerd Font icons
    #[serde(default = "default_true")]
    pub nerd_fonts: bool,

    /// Enable animations
    #[serde(default = "default_true")]
    pub animations: bool,

    /// Target FPS
    #[serde(default = "default_fps")]
    pub fps: u16,

    /// Show details panel by default
    #[serde(default = "default_true")]
    pub show_details: bool,

    /// Border style: "rounded", "plain", "thick", "double"
    #[serde(default = "default_border_style")]
    pub border_style: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    #[serde(
        deserialize_with = "deserialize_color",
        default = "default_color_reset"
    )]
    pub bg: Color,

    #[serde(
        deserialize_with = "deserialize_color",
        default = "default_color_white"
    )]
    pub fg: Color,

    #[serde(
        deserialize_with = "deserialize_color",
        default = "default_color_darkgray"
    )]
    pub fg_dim: Color,

    #[serde(deserialize_with = "deserialize_color", default = "default_color_cyan")]
    pub accent: Color,

    #[serde(
        deserialize_with = "deserialize_color",
        default = "default_color_magenta"
    )]
    pub accent_secondary: Color,

    #[serde(
        deserialize_with = "deserialize_color",
        default = "default_color_darkgray"
    )]
    pub border: Color,

    #[serde(deserialize_with = "deserialize_color", default = "default_color_cyan")]
    pub border_focused: Color,

    #[serde(default)]
    pub semantic: SemanticColors,

    #[serde(default)]
    pub signal: SignalColors,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SemanticColors {
    #[serde(
        deserialize_with = "deserialize_color",
        default = "default_color_green"
    )]
    pub connected: Color,

    #[serde(
        deserialize_with = "deserialize_color",
        default = "default_color_yellow"
    )]
    pub warning: Color,

    #[serde(deserialize_with = "deserialize_color", default = "default_color_red")]
    pub error: Color,

    #[serde(
        deserialize_with = "deserialize_color",
        default = "default_color_darkgray"
    )]
    pub selected_bg: Color,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SignalColors {
    #[serde(
        deserialize_with = "deserialize_color",
        default = "default_color_green"
    )]
    pub excellent: Color,

    #[serde(
        deserialize_with = "deserialize_color",
        default = "default_color_green"
    )]
    pub good: Color,

    #[serde(
        deserialize_with = "deserialize_color",
        default = "default_color_yellow"
    )]
    pub fair: Color,

    #[serde(deserialize_with = "deserialize_color", default = "default_color_red")]
    pub weak: Color,

    #[serde(
        deserialize_with = "deserialize_color",
        default = "default_color_darkgray"
    )]
    pub none: Color,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct KeysConfig {
    pub scan: String,
    pub connect: String,
    pub disconnect: String,
    pub forget: String,
    pub hidden: String,
    pub details: String,
    pub refresh: String,
    pub help: String,
    pub quit: String,
}

// ─── Defaults ───────────────────────────────────────────────────────────

impl Default for Config {
    fn default() -> Self {
        // Parse the embedded TOML — this cannot fail since we control it
        toml::from_str(DEFAULT_CONFIG_TOML)
            .expect("BUG: embedded default_config.toml is invalid TOML")
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            interface: String::new(),
            log_level: "info".into(),
            scan_interval_secs: 5,
        }
    }
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            nerd_fonts: true,
            animations: true,
            fps: 30,
            show_details: true,
            border_style: "rounded".into(),
        }
    }
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            bg: Color::Reset,
            fg: Color::White,
            fg_dim: Color::DarkGray,
            accent: Color::Cyan,
            accent_secondary: Color::Magenta,
            border: Color::DarkGray,
            border_focused: Color::Cyan,
            semantic: SemanticColors::default(),
            signal: SignalColors::default(),
        }
    }
}

impl Default for SemanticColors {
    fn default() -> Self {
        Self {
            connected: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            selected_bg: Color::DarkGray,
        }
    }
}

impl Default for SignalColors {
    fn default() -> Self {
        Self {
            excellent: Color::Green,
            good: Color::Green,
            fair: Color::Yellow,
            weak: Color::Red,
            none: Color::DarkGray,
        }
    }
}

impl Default for KeysConfig {
    fn default() -> Self {
        Self {
            scan: "s".into(),
            connect: "enter".into(),
            disconnect: "d".into(),
            forget: "f".into(),
            hidden: "h".into(),
            details: "i".into(),
            refresh: "r".into(),
            help: "/".into(),
            quit: "q".into(),
        }
    }
}

// ─── Color Deserializer ─────────────────────────────────────────────────

fn deserialize_color<'de, D>(deserializer: D) -> std::result::Result<Color, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    parse_color(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid color: \"{s}\"")))
}

/// Parse a color string into a ratatui Color.
/// Supports: named colors, "reset", "#RRGGBB" hex.
pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim().to_lowercase();
    match s.as_str() {
        "reset" | "default" | "transparent" => Some(Color::Reset),
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "white" => Some(Color::White),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "darkgrey" | "dark_gray" | "dark_grey" => Some(Color::DarkGray),
        "lightred" | "light_red" => Some(Color::LightRed),
        "lightgreen" | "light_green" => Some(Color::LightGreen),
        "lightyellow" | "light_yellow" => Some(Color::LightYellow),
        "lightblue" | "light_blue" => Some(Color::LightBlue),
        "lightmagenta" | "light_magenta" => Some(Color::LightMagenta),
        "lightcyan" | "light_cyan" => Some(Color::LightCyan),
        hex if hex.starts_with('#') && hex.len() == 7 => {
            let r = u8::from_str_radix(&hex[1..3], 16).ok()?;
            let g = u8::from_str_radix(&hex[3..5], 16).ok()?;
            let b = u8::from_str_radix(&hex[5..7], 16).ok()?;
            Some(Color::Rgb(r, g, b))
        }
        _ => None,
    }
}

// ─── Serde Default Helpers ──────────────────────────────────────────────

fn default_true() -> bool {
    true
}
fn default_log_level() -> String {
    "info".into()
}
fn default_scan_interval() -> u64 {
    5
}
fn default_fps() -> u16 {
    30
}
fn default_border_style() -> String {
    "rounded".into()
}
fn default_color_reset() -> Color {
    Color::Reset
}
fn default_color_white() -> Color {
    Color::White
}
fn default_color_darkgray() -> Color {
    Color::DarkGray
}
fn default_color_cyan() -> Color {
    Color::Cyan
}
fn default_color_magenta() -> Color {
    Color::Magenta
}
fn default_color_green() -> Color {
    Color::Green
}
fn default_color_yellow() -> Color {
    Color::Yellow
}
fn default_color_red() -> Color {
    Color::Red
}

// ─── Path Resolution ────────────────────────────────────────────────────

impl Config {
    /// Standard config file path: ~/.config/nexus/config.toml
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("nexus")
            .join("config.toml")
    }

    /// Log directory: ~/.local/share/nexus/
    pub fn log_dir() -> PathBuf {
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("nexus");
        std::fs::create_dir_all(&data_dir).ok();
        data_dir
    }

    /// Convenience: interface as Option<&str> (empty = None)
    pub fn interface(&self) -> Option<&str> {
        let iface = self.general.interface.trim();
        if iface.is_empty() { None } else { Some(iface) }
    }

    /// Convenience: tick interval from FPS
    pub fn tick_rate_ms(&self) -> u64 {
        let fps = self.appearance.fps.max(1);
        1000 / fps as u64
    }

    /// Check if nerd fonts are enabled
    pub fn nerd_fonts(&self) -> bool {
        self.appearance.nerd_fonts
    }

    /// Check if animations are enabled
    pub fn animations(&self) -> bool {
        self.appearance.animations
    }

    /// Scan polling interval as Duration
    pub fn scan_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.general.scan_interval_secs)
    }

    /// Access keybinding config
    pub fn keys(&self) -> &KeysConfig {
        &self.keys
    }
}

// ─── Bootloader ─────────────────────────────────────────────────────────

/// The single entry point for configuration. Called exactly once at startup.
///
/// 1. Parse CLI args
/// 2. If `--print-default-config`, dump and exit
/// 3. Resolve config file path (CLI override or default)
/// 4. If config file doesn't exist, create directory tree + write defaults
/// 5. Parse TOML from disk into Config
/// 6. Apply CLI overrides on top
pub fn load(cli: &CliArgs) -> Result<Config> {
    // Determine which config file to read
    let config_path = cli.config.clone().unwrap_or_else(Config::config_path);

    // Bootstrap: ensure the file exists on disk
    if !config_path.exists() {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent).wrap_err_with(|| {
                format!("Failed to create config directory: {}", parent.display())
            })?;
        }
        std::fs::write(&config_path, DEFAULT_CONFIG_TOML).wrap_err_with(|| {
            format!(
                "Failed to write default config to {}",
                config_path.display()
            )
        })?;
        info!("Created default config at {}", config_path.display());
    }

    // Read and parse
    let toml_str = std::fs::read_to_string(&config_path)
        .wrap_err_with(|| format!("Failed to read config from {}", config_path.display()))?;

    let mut config: Config = toml::from_str(&toml_str).wrap_err_with(|| {
        format!(
            "Failed to parse config at {}.\n\
             Delete the file to regenerate defaults, or run:\n  \
             nexus --print-default-config > {:?}",
            config_path.display(),
            config_path
        )
    })?;

    // ── CLI overrides ───────────────────────────────────────────────
    if let Some(ref iface) = cli.interface {
        config.general.interface = iface.clone();
    }
    if let Some(ref level) = cli.log_level {
        config.general.log_level = level.clone();
    }
    if cli.no_nerd_fonts {
        config.appearance.nerd_fonts = false;
    }
    if let Some(fps) = cli.fps {
        config.appearance.fps = fps;
    }

    Ok(config)
}

/// Returns the embedded default config TOML string.
pub fn default_config_toml() -> &'static str {
    DEFAULT_CONFIG_TOML
}
