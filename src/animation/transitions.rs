use super::ease_out;
use crate::network::types::WiFiNetwork;

/// Smooth signal strength display values toward their actual values.
/// Call this every tick for each visible network.
pub fn smooth_signals(networks: &mut [WiFiNetwork], factor: f32) {
    for net in networks.iter_mut() {
        let target = net.signal_strength as f32;
        net.display_signal = ease_out(net.display_signal, target, factor);

        // Increment seen ticks for fade-in (cap at 60 to avoid overflow)
        if net.seen_ticks < 60 {
            net.seen_ticks = net.seen_ticks.saturating_add(1);
        }
    }
}

/// Calculate opacity (0.0 - 1.0) for a newly discovered network based on seen_ticks.
/// Used to fade in new networks over ~10 ticks.
pub fn fade_in_opacity(seen_ticks: u16) -> f32 {
    if seen_ticks >= 10 {
        1.0
    } else {
        seen_ticks as f32 / 10.0
    }
}
