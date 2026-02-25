mod app;
mod auth;
mod config;
mod error;
mod event;
mod network;
mod ui;

use std::io;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use color_eyre::eyre::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tracing::info;

use crate::app::App;
use crate::config::Config;
use crate::error::NexusError;
use crate::event::{Event, EventHandler};
use crate::network::stats::StatsPoller;
use crate::network::NetworkManager;

/// nexus-nm — A modern TUI network manager for Arch Linux
#[derive(Parser, Debug)]
#[command(name = "nexus-nm", version, about, long_about = None)]
struct Cli {
    /// Tick rate in milliseconds
    #[arg(short, long, default_value_t = 250)]
    tick_rate: u64,

    /// Log file path (logging disabled if not specified)
    #[arg(short, long)]
    log: Option<String>,

    /// Disable mouse support
    #[arg(long, default_value_t = false)]
    no_mouse: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI args
    let cli = Cli::parse();

    // Initialize color-eyre with custom panic hook that restores terminal
    install_panic_hook();

    // Create config from CLI args
    let config = Config {
        tick_rate: Duration::from_millis(cli.tick_rate),
        mouse_support: !cli.no_mouse,
        log_file: cli.log.clone(),
        ..Default::default()
    };

    // Initialize tracing (uses config.log_file)
    init_logging(&config.log_file);

    info!("nexus-nm starting");

    // Connect to NetworkManager
    let nm = match NetworkManager::new().await {
        Ok(nm) => Arc::new(nm),
        Err(e) => {
            eprintln!("Failed to connect to NetworkManager D-Bus: {}", e);
            eprintln!("Is NetworkManager running? Try: systemctl status NetworkManager");
            std::process::exit(1);
        }
    };

    // Check if NM is running
    if !nm.is_running().await {
        eprintln!("NetworkManager is not running.");
        eprintln!("Start it with: sudo systemctl start NetworkManager");
        std::process::exit(1);
    }

    info!(
        "Connected to NetworkManager v{}",
        nm.version().await.unwrap_or_default()
    );

    // Setup terminal
    enable_raw_mode()
        .map_err(|e| NexusError::Terminal(format!("Failed to enable raw mode: {}", e)))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Setup terminal mouse support based on config
    if !config.mouse_support {
        execute!(io::stdout(), DisableMouseCapture)?;
    }

    // Create event handler
    let mut event_handler = EventHandler::new(config.tick_rate);
    let event_tx = event_handler.sender();

    // Create app
    let mut app = App::new(nm.clone(), event_tx.clone(), &config);

    // Check permissions
    app.permission_level = auth::check_permissions(&nm).await;

    // Initial network state fetch
    let initial_state = nm.snapshot().await;
    app.network_state = Some(initial_state);

    // Spawn background network state poller
    let poller_nm = nm.clone();
    let poller_tx = event_tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            let state = poller_nm.snapshot().await;
            if poller_tx
                .send(Event::NetworkRefresh(Box::new(state)))
                .is_err()
            {
                break;
            }
        }
    });

    // Spawn stats poller (uses configured interval)
    let stats_tx = event_tx.clone();
    let _stats_nm = nm.clone();
    let stats_interval = config.stats_poll_interval;
    tokio::spawn(async move {
        let mut poller = StatsPoller::new();
        let mut interval = tokio::time::interval(stats_interval);
        loop {
            interval.tick().await;
            poller.poll().await;

            // Merge stats into a network refresh
            // We send just the stats update as a special event
            let stats_clone = poller.stats.clone();
            if stats_tx
                .send(Event::ActionSuccess(format!(
                    "STATS:{}",
                    serde_json::to_string(&stats_snapshot(&stats_clone)).unwrap_or_default()
                )))
                .is_err()
            {
                break;
            }
        }
    });

    // ── Main event loop ───────────────────────────────────────────────
    loop {
        // Draw
        terminal.draw(|f| ui::draw(f, &mut app))?;

        // Handle events
        if let Some(event) = event_handler.next().await {
            match &event {
                Event::ActionSuccess(msg) if msg.starts_with("STATS:") => {
                    // Parse stats and merge into network state
                    if let Some(ref mut state) = app.network_state {
                        match serde_json::from_str::<std::collections::HashMap<String, StatsData>>(
                            &msg[6..],
                        ) {
                            Ok(stats) => {
                                for (iface, data) in stats {
                                    let entry = state.stats.entry(iface).or_default();
                                    entry.rx_bytes = data.rx_bytes;
                                    entry.tx_bytes = data.tx_bytes;
                                    entry.rx_packets = data.rx_packets;
                                    entry.tx_packets = data.tx_packets;
                                    entry.rx_errors = data.rx_errors;
                                    entry.tx_errors = data.tx_errors;
                                    entry.rx_dropped = data.rx_dropped;
                                    entry.tx_dropped = data.tx_dropped;
                                    entry.rx_rate = data.rx_rate;
                                    entry.tx_rate = data.tx_rate;
                                    entry.rx_history = data.rx_history.clone();
                                    entry.tx_history = data.tx_history.clone();
                                }
                            }
                            Err(e) => {
                                tracing::trace!(
                                    "{}",
                                    NexusError::Parse(format!("Stats data: {}", e))
                                );
                            }
                        }
                    }
                }
                _ => {
                    app.handle_event(event);
                }
            }

            if app.should_quit {
                break;
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    info!("nexus-nm exiting");
    Ok(())
}

/// Install a panic hook that restores the terminal before printing the panic
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Restore terminal
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        // Call default handler
        default_hook(panic_info);
    }));
    color_eyre::install().ok();
}

/// Initialize tracing to a log file
fn init_logging(log_path: &Option<String>) {
    use tracing_subscriber::EnvFilter;

    if let Some(ref path) = log_path {
        let file = std::fs::File::create(path).expect("Failed to create log file");
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .with_writer(file)
            .with_ansi(false)
            .init();
    } else {
        // No logging if no log path specified (can't log to stdout in a TUI)
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("off"))
            .with_writer(io::sink)
            .init();
    }
}

// ── Stats serialization helper ────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct StatsData {
    rx_bytes: u64,
    tx_bytes: u64,
    rx_packets: u64,
    tx_packets: u64,
    rx_errors: u64,
    tx_errors: u64,
    rx_dropped: u64,
    tx_dropped: u64,
    rx_rate: f64,
    tx_rate: f64,
    rx_history: Vec<f64>,
    tx_history: Vec<f64>,
}

fn stats_snapshot(
    stats: &std::collections::HashMap<String, network::types::InterfaceStats>,
) -> std::collections::HashMap<String, StatsData> {
    stats
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                StatsData {
                    rx_bytes: v.rx_bytes,
                    tx_bytes: v.tx_bytes,
                    rx_packets: v.rx_packets,
                    tx_packets: v.tx_packets,
                    rx_errors: v.rx_errors,
                    tx_errors: v.tx_errors,
                    rx_dropped: v.rx_dropped,
                    tx_dropped: v.tx_dropped,
                    rx_rate: v.rx_rate,
                    tx_rate: v.tx_rate,
                    rx_history: v.rx_history.clone(),
                    tx_history: v.tx_history.clone(),
                },
            )
        })
        .collect()
}
