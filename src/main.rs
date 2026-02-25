mod animation;
mod app;
mod config;
mod event;
mod network;
mod ui;

use std::io;
use std::panic;
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
use config::Config;
use event::{Event, EventHandler};
use network::NetworkBackend;
use network::manager::NmBackend;
use network::types::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let config = Config::parse();

    // Initialize error reporting
    color_eyre::install()?;

    // Set up logging to file
    let log_dir = Config::log_dir();
    let file_appender = tracing_appender::rolling::daily(&log_dir, "nexus.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.log_level)),
        )
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

    info!("Nexus starting up");

    // Install custom panic hook that restores terminal
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        // Restore terminal before printing panic
        let _ = disable_raw_mode();
        let _ = execute!(io::stderr(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // Initialize network backend
    let nm_backend = match NmBackend::new(config.interface.as_deref()).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: {}", e);
            eprintln!("\nNexus requires NetworkManager to be running.");
            eprintln!("Install: sudo pacman -S networkmanager");
            eprintln!("Start:   sudo systemctl start NetworkManager");
            std::process::exit(1);
        }
    };

    let interface_name = nm_backend.interface_name().to_string();

    // Start D-Bus signal listeners
    let signal_conn = nm_backend.connection().clone();
    let signal_device = nm_backend.device_path();
    let signal_tx = nm_backend.event_sender();

    network::signals::start_signal_listener(signal_conn, signal_device, signal_tx).await;

    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    terminal.hide_cursor()?;

    // Set up event handler (33ms tick = ~30 FPS)
    let mut events = EventHandler::new(33);
    let event_tx = events.sender();

    // Create app state
    let mut app = App::new(config, interface_name, event_tx.clone());

    // Perform initial scan
    app.mode = AppMode::Scanning;
    app.animation.start_spinner();

    let scan_iface = nm_backend.interface_name().to_string();
    let scan_tx = event_tx.clone();

    tokio::spawn(async move {
        if let Ok(backend) = NmBackend::new(Some(&scan_iface)).await {
            match backend.scan().await {
                Ok(networks) => {
                    let _ = scan_tx.send(Event::NetworkScan(networks));
                }
                Err(e) => {
                    let _ = scan_tx.send(Event::Error(format!("Scan failed: {}", e)));
                }
            }
        }
    });

    // Also fetch current connection
    let conn_tx = event_tx.clone();
    let conn_iface = nm_backend.interface_name().to_string();
    tokio::spawn(async move {
        if let Ok(backend) = NmBackend::new(Some(&conn_iface)).await {
            match backend.current_connection().await {
                Ok(Some(info)) => {
                    let _ =
                        conn_tx.send(Event::ConnectionChanged(ConnectionStatus::Connected(info)));
                }
                Ok(None) => {
                    let _ = conn_tx.send(Event::ConnectionChanged(ConnectionStatus::Disconnected));
                }
                Err(e) => {
                    tracing::warn!("Failed to get connection info: {}", e);
                }
            }
        }
    });

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

                Event::NetworkEvent(net_event) => {
                    handle_network_event(&mut app, &nm_backend, net_event, &event_tx).await;
                }

                Event::Error(msg) => {
                    handle_error_event(&mut app, &nm_backend, &msg, &event_tx).await;
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
    // Give tasks a moment to exit (they poll every ~33ms)
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

/// Handle network events from D-Bus signals
async fn handle_network_event(
    _app: &mut App,
    nm: &NmBackend,
    event: NetworkEvent,
    tx: &tokio::sync::mpsc::UnboundedSender<Event>,
) {
    match event {
        NetworkEvent::ScanComplete(networks) => {
            if !networks.is_empty() {
                let _ = tx.send(Event::NetworkScan(networks));
                return;
            }
            // If empty, trigger a fresh scan
            let iface = nm.interface_name().to_string();
            let scan_tx = tx.clone();
            tokio::spawn(async move {
                if let Ok(backend) = NmBackend::new(Some(&iface)).await {
                    match backend.scan().await {
                        Ok(networks) => {
                            let _ = scan_tx.send(Event::NetworkScan(networks));
                        }
                        Err(e) => {
                            let _ = scan_tx.send(Event::Error(format!("Scan failed: {}", e)));
                        }
                    }
                }
            });
        }
        NetworkEvent::ConnectionChanged(status) => {
            let _ = tx.send(Event::ConnectionChanged(status));
            // Also refresh full connection info
            let iface = nm.interface_name().to_string();
            let conn_tx = tx.clone();
            tokio::spawn(async move {
                if let Ok(backend) = NmBackend::new(Some(&iface)).await {
                    match backend.current_connection().await {
                        Ok(Some(info)) => {
                            let _ = conn_tx
                                .send(Event::ConnectionChanged(ConnectionStatus::Connected(info)));
                        }
                        Ok(None) => {
                            let _ = conn_tx
                                .send(Event::ConnectionChanged(ConnectionStatus::Disconnected));
                        }
                        _ => {}
                    }
                }
            });
        }
    }
}

/// Handle custom error/command events (connect, forget, disconnect)
async fn handle_error_event(
    app: &mut App,
    nm: &NmBackend,
    msg: &str,
    tx: &tokio::sync::mpsc::UnboundedSender<Event>,
) {
    if let Some(rest) = msg.strip_prefix("CONNECT:") {
        let parts: Vec<&str> = rest.splitn(2, ':').collect();
        let ssid = parts[0].to_string();
        let password = parts.get(1).and_then(|p| {
            if p.is_empty() {
                None
            } else {
                Some(p.to_string())
            }
        });

        let iface = nm.interface_name().to_string();
        let connect_tx = tx.clone();
        tokio::spawn(async move {
            if let Ok(backend) = NmBackend::new(Some(&iface)).await {
                match backend.connect(&ssid, password.as_deref()).await {
                    Ok(()) => {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        match backend.current_connection().await {
                            Ok(Some(info)) => {
                                let _ = connect_tx.send(Event::ConnectionChanged(
                                    ConnectionStatus::Connected(info),
                                ));
                            }
                            _ => {
                                let _ = connect_tx
                                    .send(Event::ConnectionChanged(ConnectionStatus::Disconnected));
                            }
                        }
                        if let Ok(networks) = backend.scan().await {
                            let _ = connect_tx.send(Event::NetworkScan(networks));
                        }
                    }
                    Err(e) => {
                        let _ = connect_tx.send(Event::ConnectionChanged(
                            ConnectionStatus::Failed(format!("{}", e)),
                        ));
                    }
                }
            }
        });
    } else if let Some(rest) = msg.strip_prefix("CONNECT_HIDDEN:") {
        let parts: Vec<&str> = rest.splitn(2, ':').collect();
        let ssid = parts[0].to_string();
        let password = parts.get(1).and_then(|p| {
            if p.is_empty() {
                None
            } else {
                Some(p.to_string())
            }
        });

        let iface = nm.interface_name().to_string();
        let connect_tx = tx.clone();
        tokio::spawn(async move {
            if let Ok(backend) = NmBackend::new(Some(&iface)).await {
                match backend.connect_hidden(&ssid, password.as_deref()).await {
                    Ok(()) => {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        match backend.current_connection().await {
                            Ok(Some(info)) => {
                                let _ = connect_tx.send(Event::ConnectionChanged(
                                    ConnectionStatus::Connected(info),
                                ));
                            }
                            _ => {
                                let _ = connect_tx
                                    .send(Event::ConnectionChanged(ConnectionStatus::Disconnected));
                            }
                        }
                        if let Ok(networks) = backend.scan().await {
                            let _ = connect_tx.send(Event::NetworkScan(networks));
                        }
                    }
                    Err(e) => {
                        let _ = connect_tx.send(Event::ConnectionChanged(
                            ConnectionStatus::Failed(format!("{}", e)),
                        ));
                    }
                }
            }
        });
    } else if let Some(ssid) = msg.strip_prefix("FORGET:") {
        let ssid = ssid.to_string();
        let iface = nm.interface_name().to_string();
        let forget_tx = tx.clone();
        tokio::spawn(async move {
            if let Ok(backend) = NmBackend::new(Some(&iface)).await {
                match backend.forget_network(&ssid).await {
                    Ok(()) => {
                        if let Ok(networks) = backend.scan().await {
                            let _ = forget_tx.send(Event::NetworkScan(networks));
                        }
                    }
                    Err(e) => {
                        let _ = forget_tx.send(Event::Error(format!("Failed to forget: {}", e)));
                    }
                }
            }
        });
    } else if msg.starts_with("DISCONNECT:") {
        let iface = nm.interface_name().to_string();
        let dc_tx = tx.clone();
        tokio::spawn(async move {
            if let Ok(backend) = NmBackend::new(Some(&iface)).await {
                match backend.disconnect().await {
                    Ok(()) => {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        let _ =
                            dc_tx.send(Event::ConnectionChanged(ConnectionStatus::Disconnected));
                        if let Ok(networks) = backend.scan().await {
                            let _ = dc_tx.send(Event::NetworkScan(networks));
                        }
                    }
                    Err(e) => {
                        let _ = dc_tx.send(Event::ConnectionChanged(ConnectionStatus::Failed(
                            format!("{}", e),
                        )));
                    }
                }
            }
        });
    } else {
        // Generic error — display it
        app.mode = AppMode::Error(msg.to_string());
        app.animation.start_dialog_slide();
    }
}
