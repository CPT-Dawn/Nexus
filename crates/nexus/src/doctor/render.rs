//! The Doctor's report as plain text, as `nexus doctor` prints it.

use std::fmt::Write as _;

use nexus_ipc::{DoctorReport, Severity};

fn tag(severity: Severity) -> &'static str {
    match severity {
        Severity::Ok => "ok",
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Problem => "problem",
        _ => "?",
    }
}

/// One line per healthy check; cause and fix under everything else.
pub fn render(report: &DoctorReport) -> String {
    let mut out = String::new();
    for f in &report.findings {
        let _ = writeln!(out, "[{:<7}] {}", tag(f.severity), f.title);
        if f.severity == Severity::Ok {
            continue;
        }
        let _ = writeln!(out, "          Why: {}", f.cause);
        if let Some(fix) = &f.fix {
            let _ = writeln!(out, "          Fix: {}", fix.text);
            for command in &fix.commands {
                let _ = writeln!(out, "                 $ {command}");
            }
        }
    }
    if report.worst() >= Severity::Warning {
        out.push_str("\nRun `nexus doctor --fix` to be offered the fixes one by one.\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_ipc::{Finding, Fix};

    #[test]
    fn healthy_checks_take_one_line_and_trouble_explains_itself() {
        let report = DoctorReport {
            findings: vec![
                Finding {
                    id: "a".into(),
                    severity: Severity::Ok,
                    title: "All good".into(),
                    cause: "because".into(),
                    fix: None,
                },
                Finding {
                    id: "b".into(),
                    severity: Severity::Problem,
                    title: "Broken".into(),
                    cause: "it broke".into(),
                    fix: Some(Fix {
                        text: "mend it".into(),
                        commands: vec!["mend".into()],
                        runnable: true,
                        privileged: false,
                    }),
                },
            ],
        };
        let text = render(&report);
        assert!(
            text.contains("[ok     ] All good\n[problem] Broken\n"),
            "{text}"
        );
        assert!(!text.contains("because"));
        assert!(text.contains("Why: it broke") && text.contains("$ mend"));
    }
}
