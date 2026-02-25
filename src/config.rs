use clap::Parser;

/// Nexus — A beautiful modern TUI WiFi manager
#[derive(Parser, Debug, Clone)]
#[command(name = "nexus", version, about, long_about = None)]
pub struct Config {
    /// WiFi interface to use (default: auto-detect)
    #[arg(short, long)]
    pub interface: Option<String>,

    /// Log level filter (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    pub log_level: String,

    /// Disable Nerd Font icons, use plain Unicode instead
    #[arg(long, default_value_t = false)]
    pub no_nerd_fonts: bool,
}

impl Config {
    pub fn log_dir() -> std::path::PathBuf {
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("nexus");
        std::fs::create_dir_all(&data_dir).ok();
        data_dir
    }
}
