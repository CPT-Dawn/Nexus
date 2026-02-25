use std::collections::HashMap;
use tokio::fs;

use crate::network::types::InterfaceStats;

const HISTORY_MAX: usize = 60;

/// Reads /sys/class/net/<iface>/statistics/ counters
pub struct StatsPoller {
    previous: HashMap<String, (u64, u64)>, // iface -> (rx_bytes, tx_bytes)
    pub stats: HashMap<String, InterfaceStats>,
}

impl StatsPoller {
    pub fn new() -> Self {
        Self {
            previous: HashMap::new(),
            stats: HashMap::new(),
        }
    }

    /// Poll all interfaces and compute rate deltas.
    /// Call this once per second for accurate Bps calculations.
    pub async fn poll(&mut self) {
        let interfaces = match self.list_interfaces().await {
            Ok(ifaces) => ifaces,
            Err(_) => return,
        };

        for iface in &interfaces {
            let rx = read_stat(iface, "rx_bytes").await.unwrap_or(0);
            let tx = read_stat(iface, "tx_bytes").await.unwrap_or(0);
            let rx_packets = read_stat(iface, "rx_packets").await.unwrap_or(0);
            let tx_packets = read_stat(iface, "tx_packets").await.unwrap_or(0);
            let rx_errors = read_stat(iface, "rx_errors").await.unwrap_or(0);
            let tx_errors = read_stat(iface, "tx_errors").await.unwrap_or(0);
            let rx_dropped = read_stat(iface, "rx_dropped").await.unwrap_or(0);
            let tx_dropped = read_stat(iface, "tx_dropped").await.unwrap_or(0);

            let entry = self.stats.entry(iface.clone()).or_default();
            entry.rx_bytes = rx;
            entry.tx_bytes = tx;
            entry.rx_packets = rx_packets;
            entry.tx_packets = tx_packets;
            entry.rx_errors = rx_errors;
            entry.tx_errors = tx_errors;
            entry.rx_dropped = rx_dropped;
            entry.tx_dropped = tx_dropped;

            // Compute rate if we have a previous sample
            if let Some((prev_rx, prev_tx)) = self.previous.get(iface) {
                let rx_delta = rx.saturating_sub(*prev_rx) as f64;
                let tx_delta = tx.saturating_sub(*prev_tx) as f64;
                entry.rx_rate = rx_delta;
                entry.tx_rate = tx_delta;

                // Append to history ring buffer
                entry.rx_history.push(rx_delta);
                entry.tx_history.push(tx_delta);
                if entry.rx_history.len() > HISTORY_MAX {
                    entry.rx_history.remove(0);
                }
                if entry.tx_history.len() > HISTORY_MAX {
                    entry.tx_history.remove(0);
                }
            }

            self.previous.insert(iface.clone(), (rx, tx));
        }

        // Remove stats for interfaces that no longer exist
        self.stats.retain(|k, _| interfaces.contains(k));
        self.previous.retain(|k, _| interfaces.contains(k));
    }

    async fn list_interfaces(&self) -> Result<Vec<String>, std::io::Error> {
        let mut entries = fs::read_dir("/sys/class/net").await?;
        let mut interfaces = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            if let Ok(name) = entry.file_name().into_string() {
                interfaces.push(name);
            }
        }
        Ok(interfaces)
    }
}

async fn read_stat(iface: &str, stat: &str) -> Result<u64, std::io::Error> {
    let path = format!("/sys/class/net/{}/statistics/{}", iface, stat);
    let content = fs::read_to_string(&path).await?;
    content
        .trim()
        .parse::<u64>()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
