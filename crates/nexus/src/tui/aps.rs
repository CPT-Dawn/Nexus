//! The access point details of a Network (`i` on the Wi-Fi tab): address,
//! channel, frequency, band and link rate of every access point behind it.
//! State and drawing; the Daemon call goes through `Runner::call`.

use nexus_ipc::ApList;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::caps::Glyphs;
use super::theme::Look;
use super::view::Sym;

#[derive(Debug)]
pub enum Stage {
    Loading,
    Shown(ApList),
    Failed(String),
}

/// The open details view.
#[derive(Debug)]
pub struct Aps {
    pub ssid: String,
    pub stage: Stage,
}

impl Aps {
    pub fn loading(ssid: String) -> Self {
        Self {
            ssid,
            stage: Stage::Loading,
        }
    }
}

/// Rows from this inner width on also say which card hears the access point.
const CARD_AT: usize = 60;

pub(super) fn draw(f: &mut Frame, aps: &Aps, body: Rect, look: &Look, sym: &Sym) {
    let rows = match &aps.stage {
        Stage::Shown(list) => list.aps.len(),
        _ => 1,
    };
    let width = body.width.min(70);
    let height = body.height.min(rows as u16 + 6);
    let rect = Rect {
        x: body.x + (body.width - width) / 2,
        y: body.y + (body.height - height) / 2,
        width,
        height,
    };
    f.render_widget(Clear, rect);
    let title = format!("Access points of {}", aps.ssid);
    let block = sym.block(look, &title, true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    if inner.height < 3 {
        return;
    }
    let mut lines: Vec<Line> = Vec::new();
    match &aps.stage {
        Stage::Loading => lines.push(Line::styled("Reading...", look.dim())),
        Stage::Failed(message) => lines.push(Line::styled(message.clone(), look.err())),
        Stage::Shown(list) => {
            let cards = inner.width as usize >= CARD_AT;
            let head = format!(
                "  {:<17} {:>3} {:>5} {:<5} {:>9} {:>4}{}",
                "Address",
                "Ch",
                "MHz",
                "Band",
                "Rate",
                "Sig",
                if cards { "  Card" } else { "" }
            );
            lines.push(Line::styled(head, look.muted()));
            let room = (inner.height as usize).saturating_sub(3);
            for ap in list.aps.iter().take(room) {
                let marker = if ap.joined { sym.dot() } else { " " };
                let rate = if ap.rate_mbps == 0 {
                    "?".to_owned()
                } else {
                    format!("{} Mb/s", ap.rate_mbps)
                };
                let band = match ap.band {
                    nexus_ipc::Band::Ghz2_4 => "2.4G",
                    nexus_ipc::Band::Ghz5 => "5G",
                    nexus_ipc::Band::Ghz6 => "6G",
                    _ => "?",
                };
                let text = format!(
                    "{:<17} {:>3} {:>5} {:<5} {:>9} {:>3}%{}",
                    ap.bssid,
                    ap.channel,
                    ap.frequency_mhz,
                    band,
                    rate,
                    ap.signal,
                    if cards {
                        format!("  {}", ap.device)
                    } else {
                        String::new()
                    }
                );
                let style = if ap.joined { look.ok() } else { look.text() };
                lines.push(Line::from(vec![
                    Span::styled(format!("{marker} "), look.ok()),
                    Span::styled(text, style),
                ]));
            }
            if list.aps.len() > room {
                lines.push(Line::styled(
                    format!("  and {} more", list.aps.len() - room),
                    look.dim(),
                ));
            }
        }
    }
    let list_area = Rect {
        height: inner.height - 1,
        ..inner
    };
    let keys = Rect {
        y: inner.bottom() - 1,
        height: 1,
        ..inner
    };
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), list_area);
    let glyph = if sym.glyphs == Glyphs::Ascii {
        "*"
    } else {
        "●"
    };
    f.render_widget(
        Paragraph::new(Line::styled(
            format!("{glyph} connected   esc close"),
            look.dim(),
        )),
        keys,
    );
}
