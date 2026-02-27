mod animation;
mod app;
mod config;
mod event;
mod network;
mod ui;

use std::io;
use std::panic;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use color_eyre::eyre::Result;
use crossterm::{
    cursor, execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tracing::info;

use app::{App, AppMode};
use config::CliArgs;
use event::{Event, EventHandler, NetworkCommand};
use network::NetworkBackend;
use network::manager::NmBackend;
use network::types::*;
use ui::theme::Theme;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments (thin layer — just flags)
    let cli = CliArgs::parse();

    // Handle --print-default-config early exit
    if cli.print_default_config {
        print!("{}", config::default_config_toml());
        return Ok(());
    }

    // Initialize error reporting
    color_eyre::install()?;

    // Load configuration (TOML + CLI overrides)
    let config = config::load(&cli)?;

    // Build the runtime theme from config
    let theme = Theme::from_config(&config);

    // Set up logging to file
    let log_dir = config::Config::log_dir();
    let file_appender = tracing_appender::rolling::daily(&log_dir, "nexus.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.general.log_level)),
        )
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

    info!("Nexus starting up");
    info!(
        "Config: fps={}, animations={}, nerd_fonts={}, scan_interval={}s, help_key={}",
        config.appearance.fps,
        config.animations(),
        config.nerd_fonts(),
        config.scan_interval().as_secs(),
        config.keys().help
    );

    // Install custom panic hook that restores terminal
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        // Restore terminal before printing panic
        let _ = disable_raw_mode();
        let _ = execute!(io::stderr(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // Initialize network backend (shared via Arc — no more re-creating per operation)
    let nm_backend = match NmBackend::new(config.interface()).await {
        Ok(b) => Arc::new(b),
        Err(e) => {
            eprintln!("Error: {}", e);
            eprintln!("\nNexus requires NetworkManager to be running.");
            eprintln!("Install: sudo pacman -S networkmanager");
            eprintln!("Start:   sudo systemctl start NetworkManager");
            std::process::exit(1);
        }
    };

    let interface_name = nm_backend.interface_name().to_string();

    // Set up event handler (tick rate from config FPS)
    let mut events = EventHandler::new(config.tick_rate_ms());
    let event_tx = events.sender();

    // Start D-Bus signal listeners — now sends events directly via event_tx
    let signal_conn = nm_backend.connection().clone();
    let signal_device = nm_backend.device_path();

    network::signals::start_signal_listener(signal_conn, signal_device, event_tx.clone()).await;

    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    terminal.hide_cursor()?;

    // Create app state
    let mut app = App::new(config, theme, interface_name, event_tx.clone());

    // Perform initial scan
    app.mode = AppMode::Scanning;
    app.animation.start_spinner();

    {
        let nm = Arc::clone(&nm_backend);
        let tx = event_tx.clone();
        tokio::spawn(async move {
            match nm.scan().await {
                Ok(networks) => {
                    let _ = tx.send(Event::NetworkScan(networks));
                }
                Err(e) => {
                    let _ = tx.send(Event::Error(format!("Scan failed: {}", e)));
                }
            }
        });
    }

    // Also fetch current connection
    {
        let nm = Arc::clone(&nm_backend);
        let tx = event_tx.clone();
        tokio::spawn(async move {
            match nm.current_connection().await {
                Ok(Some(info)) => {
                    let _ = tx.send(Event::ConnectionChanged(ConnectionStatus::Connected(info)));
                }
                Ok(None) => {
                    let _ = tx.send(Event::ConnectionChanged(ConnectionStatus::Disconnected));
                }
                Err(e) => {
                    tracing::warn!("Failed to get connection info: {}", e);
                }
            }
        });
    }

    // ─── Main Event Loop ────────────────────────────────────────────
    info!("Entering main event loop");

    loop {
        // Render
        terminal.draw(|frame| ui::render(frame, &app))?;

        // Wait for next event
        if let Some(event) = events.next().await {
            match event {
                Event::Key(key) => {
                    app.handle_key(key);
                }

                Event::Tick => {
                    app.tick();
                }

                Event::Resize(w, h) => {
                    tracing::debug!("Terminal resized to {}x{}", w, h);
                }

                Event::NetworkScan(networks) => {
                    app.update_networks(networks);
                }

                Event::ConnectionChanged(status) => {
                    app.update_connection_status(status);
                }

                Event::Command(cmd) => {
                    handle_command(&nm_backend, cmd, &event_tx);
                }

                Event::Error(msg) => {
                    app.mode = AppMode::Error(msg);
                    app.animation.start_dialog_slide();
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    // ─── Cleanup ────────────────────────────────────────────────────
    info!("Nexus shutting down");

    // Stop background event tasks first so they release stdin
    events.stop();
    // Give tasks a moment to exit
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Restore terminal state
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, cursor::Show)?;
    terminal.show_cursor()?;

    // Flush to ensure all escape sequences are written immediately
    use std::io::Write;
    io::stdout().flush()?;

    Ok(())
}

/// Handle typed network commands dispatched from the UI.
/// Each command spawns an async task that reuses the shared Arc<NmBackend>.
fn handle_command(
    nm: &Arc<NmBackend>,
    cmd: NetworkCommand,
    tx: &tokio::sync::mpsc::UnboundedSender<Event>,
) {
    match cmd {
        NetworkCommand::Scan => {
            let nm = Arc::clone(nm);
            let tx = tx.clone();
            tokio::spawn(async move {
                match nm.scan().await {
                    Ok(networks) => {
                        let _ = tx.send(Event::NetworkScan(networks));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::Error(format!("Scan failed: {}", e)));
                    }
                }
            });
        }

        NetworkCommand::Connect { ssid, password } => {
            let nm = Arc::clone(nm);
            let tx = tx.clone();
            tokio::spawn(async move {
                match nm.connect(&ssid, password.as_deref()).await {
                    Ok(()) => {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        match nm.current_connection().await {
                            Ok(Some(info)) => {
                                let _ = tx.send(Event::ConnectionChanged(
                                    ConnectionStatus::Connected(info),
                                ));
                            }
                            _ => {
                                let _ = tx
                                    .send(Event::ConnectionChanged(ConnectionStatus::Disconnected));
                            }
                        }
                        if let Ok(networks) = nm.scan().await {
                            let _ = tx.send(Event::NetworkScan(networks));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Event::ConnectionChanged(ConnectionStatus::Failed(
                            format!("{}", e),
                        )));
                    }
                }
            });
        }

        NetworkCommand::ConnectHidden { ssid, password } => {
            let nm = Arc::clone(nm);
            let tx = tx.clone();
            tokio::spawn(async move {
                match nm.connect_hidden(&ssid, password.as_deref()).await {
                    Ok(()) => {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        match nm.current_connection().await {
                            Ok(Some(info)) => {
                                let _ = tx.send(Event::ConnectionChanged(
                                    ConnectionStatus::Connected(info),
                                ));
                            }
                            _ => {
                                let _ = tx
                                    .send(Event::ConnectionChanged(ConnectionStatus::Disconnected));
                            }
                        }
                        if let Ok(networks) = nm.scan().await {
                            let _ = tx.send(Event::NetworkScan(networks));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Event::ConnectionChanged(ConnectionStatus::Failed(
                            format!("{}", e),
                        )));
                    }
                }
            });
        }

        NetworkCommand::Disconnect => {
            let nm = Arc::clone(nm);
            let tx = tx.clone();
            tokio::spawn(async move {
                match nm.disconnect().await {
                    Ok(()) => {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        let _ = tx.send(Event::ConnectionChanged(ConnectionStatus::Disconnected));
                        if let Ok(networks) = nm.scan().await {
                            let _ = tx.send(Event::NetworkScan(networks));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Event::ConnectionChanged(ConnectionStatus::Failed(
                            format!("{}", e),
                        )));
                    }
                }
            });
        }

        NetworkCommand::Forget { ssid } => {
            let nm = Arc::clone(nm);
            let tx = tx.clone();
            tokio::spawn(async move {
                match nm.forget_network(&ssid).await {
                    Ok(()) => {
                        if let Ok(networks) = nm.scan().await {
                            let _ = tx.send(Event::NetworkScan(networks));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Event::Error(format!("Failed to forget: {}", e)));
                    }
                }
            });
        }

        NetworkCommand::RefreshConnection => {
            let nm = Arc::clone(nm);
            let tx = tx.clone();
            tokio::spawn(async move {
                match nm.current_connection().await {
                    Ok(Some(info)) => {
                        let _ =
                            tx.send(Event::ConnectionChanged(ConnectionStatus::Connected(info)));
                    }
                    Ok(None) => {
                        let _ = tx.send(Event::ConnectionChanged(ConnectionStatus::Disconnected));
                    }
                    Err(e) => {
                        tracing::warn!("Refresh failed: {}", e);
                    }
                }
            });
        }
    }
}
