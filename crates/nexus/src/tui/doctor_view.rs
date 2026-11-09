//! What the Doctor shows in the TUI: the first-run setup screen when
//! NetworkManager is missing, the Diagnostics panel on the Overview, and the
//! confirmation before anything privileged runs.

use nexus_ipc::{Finding, Severity};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::app::{App, FixState};
use super::theme::Look;
use super::view::Sym;

/// The style of a finding's marker and title.
fn severity_style(look: &Look, severity: Severity) -> Style {
    match severity {
        Severity::Problem => look.err(),
        Severity::Warning => look.warn(),
        Severity::Ok => look.ok(),
        _ => look.muted(),
    }
}

/// Finding text for the screen: the backticks that mark commands and names
/// in `nexus doctor` output are noise here.
fn plain(text: &str) -> String {
    text.replace('`', "")
}

/// `text` cut to `width` columns, ending in `...` when it had to be.
fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let kept: String = text.chars().take(width.saturating_sub(3)).collect();
    format!("{}...", kept.trim_end())
}

/// The lines saying how the fix is going, if it has been started.
fn fix_status(app: &App, look: &Look) -> Option<Line<'static>> {
    match app.fix_state() {
        FixState::Idle | FixState::Asking => None,
        FixState::Running => Some(Line::styled("Running the fix...", look.warn())),
        FixState::Done => Some(Line::styled(
            "Done. Checking your system again...",
            look.ok(),
        )),
        FixState::Failed(why) => Some(Line::styled(why.clone(), look.err())),
    }
}

/// The first-run screen: why Nexus cannot work yet, and the exact commands.
pub fn setup(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let block = sym.block(look, "Set up NetworkManager", true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines: Vec<Line> = Vec::new();
    let trouble: Vec<&Finding> = app.findings().iter().filter(|f| f.is_trouble()).collect();
    let Some(main) = trouble.iter().find(|f| f.id == "networkmanager") else {
        lines.push(Line::styled(
            "NetworkManager is not running.",
            look.err().bold(),
        ));
        lines.push(Line::default());
        lines.push(Line::styled("Checking your system...", look.dim()));
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    };
    lines.push(Line::from(vec![
        Span::styled(format!("{} ", sym.dot()), look.err()),
        Span::styled(plain(&main.title), look.err().bold()),
    ]));
    lines.push(Line::styled(plain(&main.cause), look.text()));
    for other in trouble.iter().filter(|f| f.id != main.id) {
        // Titles only: the cause is one `nexus doctor` away, and the
        // commands are what this screen is for.
        if lines.len() == 2 {
            lines.push(Line::default());
        }
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", sym.dot()),
                severity_style(look, other.severity),
            ),
            Span::styled(plain(&other.title), look.muted()),
        ]));
    }
    if let Some(fix) = &main.fix {
        lines.push(Line::default());
        lines.push(Line::styled(plain(&fix.text), look.text()));
        for command in &fix.commands {
            lines.push(Line::from(vec![
                Span::styled("  $ ", look.dim()),
                Span::styled(command.clone(), look.accent()),
            ]));
        }
        lines.push(Line::default());
        lines.push(Line::styled(
            if fix.runnable {
                "Press f to run these steps for you. Administrator steps ask for your password first."
            } else {
                "Run the commands in a terminal, then Nexus notices on its own."
            },
            look.muted(),
        ));
    }
    if let Some(status) = fix_status(app, look) {
        lines.push(status);
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// How many rows the Diagnostics panel wants for these findings.
pub fn panel_height(app: &App) -> u16 {
    let n = app
        .findings()
        .iter()
        .filter(|f| f.is_trouble())
        .count()
        .min(3) as u16;
    if n == 0 { 0 } else { 2 + n * 2 }
}

/// The Diagnostics panel: trouble first, each with what to do about it.
pub fn panel(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let block = sym.block(look, "Diagnostics", false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut trouble: Vec<&Finding> = app.findings().iter().filter(|f| f.is_trouble()).collect();
    trouble.sort_by_key(|f| std::cmp::Reverse(f.severity));
    let roomy = inner.height as usize >= trouble.len().min(3) * 2;
    let mut lines: Vec<Line> = Vec::new();
    for finding in trouble.iter().take(3) {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", sym.dot()),
                severity_style(look, finding.severity),
            ),
            Span::styled(
                fit(
                    &plain(&finding.title),
                    (inner.width as usize).saturating_sub(2),
                ),
                look.text(),
            ),
        ]));
        if roomy {
            let hint = finding
                .fix
                .as_ref()
                .map_or(finding.cause.as_str(), |fix| fix.text.as_str());
            let room = (inner.width as usize).saturating_sub(2);
            lines.push(Line::styled(
                format!("  {}", fit(&plain(hint), room)),
                look.muted(),
            ));
        }
    }
    if let Some(status) = fix_status(app, look) {
        lines.push(status);
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// The yes/no before a fix runs.
pub fn confirm(f: &mut Frame, app: &App, area: Rect, look: &Look, sym: &Sym) {
    let Some(finding) = app.runnable_fix() else {
        return;
    };
    let Some(fix) = &finding.fix else { return };
    let mut lines = vec![
        Line::styled(plain(&finding.title), look.title()),
        Line::default(),
        Line::styled("These commands will run:", look.text()),
    ];
    for command in &fix.commands {
        lines.push(Line::from(vec![
            Span::styled("  $ ", look.dim()),
            Span::styled(command.clone(), look.accent()),
        ]));
    }
    lines.push(Line::default());
    if fix.privileged {
        lines.push(Line::styled(
            "They run as administrator: your system will ask for your password.",
            look.warn(),
        ));
    }
    lines.push(Line::from(vec![
        Span::styled("y", look.accent()),
        Span::styled(" run them   ", look.muted()),
        Span::styled("n", look.accent()),
        Span::styled(" cancel", look.muted()),
    ]));
    let width = 72.min(area.width);
    let wrapped: u16 = lines
        .iter()
        .map(|l| {
            (l.width() as u16)
                .div_ceil(width.saturating_sub(4).max(1))
                .max(1)
        })
        .sum();
    let height = (wrapped + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    f.render_widget(Clear, rect);
    let block = sym.block(look, "Run the fix?", true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
