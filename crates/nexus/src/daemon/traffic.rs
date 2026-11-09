//! Turns the Backend's byte counters into rates. The Daemon keeps the last
//! sample, so any number of Front ends can ask and each answer covers the
//! time since the previous sample, whoever took it.

use std::time::{Duration, Instant};

use nexus_ipc::Traffic;
use tokio::sync::Mutex;

use crate::backend::Counters;

/// Asked again sooner than this, the previous rates are still the answer:
/// counters move about once a second and a short window is mostly noise.
const MIN_WINDOW: Duration = Duration::from_millis(800);

#[derive(Debug)]
struct Sample {
    at: Instant,
    counters: Counters,
    rates: Traffic,
}

#[derive(Debug, Default)]
pub struct Sampler {
    last: Mutex<Option<Sample>>,
}

impl Sampler {
    /// Rates for the given counters; `None` (no Link) clears the baseline.
    pub async fn rates(&self, counters: Option<Counters>) -> Traffic {
        let mut last = self.last.lock().await;
        let Some(now) = counters else {
            *last = None;
            return Traffic::default();
        };
        let at = Instant::now();
        let rates = match &*last {
            Some(s) if s.counters.device == now.device && at - s.at < MIN_WINDOW => {
                return s.rates;
            }
            Some(s) if s.counters.device == now.device => {
                let secs = (at - s.at).as_secs_f64();
                // A counter that went backwards was reset: count from zero.
                let rate = |new: u64, old: u64| {
                    new.checked_sub(old)
                        .map_or(0, |d| (d as f64 / secs).round() as u64)
                };
                Traffic {
                    rx_bps: rate(now.rx_bytes, s.counters.rx_bytes),
                    tx_bps: rate(now.tx_bytes, s.counters.tx_bytes),
                }
            }
            // A different Link (or the first sample) is only a baseline.
            _ => Traffic::default(),
        };
        *last = Some(Sample {
            at,
            counters: now,
            rates,
        });
        rates
    }
}
