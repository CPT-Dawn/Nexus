//! Wire types of the Doctor: findings with a severity, a cause and a fix.

use serde::{Deserialize, Serialize};

/// How much a finding matters. Ordered: later is worse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Severity {
    /// Checked and fine.
    Ok,
    /// Worth knowing, nothing to fix.
    Info,
    /// Something is off and may bite.
    Warning,
    /// This is why the machine is not (fully) online.
    Problem,
    #[serde(other)]
    Unknown,
}

/// What to do about a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fix {
    /// Plain English: what to do and why it helps.
    pub text: String,
    /// The exact commands, in order, as a shell would run them.
    #[serde(default)]
    pub commands: Vec<String>,
    /// Whether the Daemon can run `commands` itself (`doctor_fix`).
    #[serde(default)]
    pub runnable: bool,
    /// Whether `commands` need administrator rights (run through `pkexec`).
    #[serde(default)]
    pub privileged: bool,
}

/// One check's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// Stable id of the check, e.g. `networkmanager`, `dns`, `rfkill`.
    pub id: String,
    pub severity: Severity,
    /// One line, e.g. "NetworkManager is not running".
    pub title: String,
    /// Plain English: why this is so.
    pub cause: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
}

impl Finding {
    /// Whether it deserves the person's attention (`warning` or worse).
    pub fn is_trouble(&self) -> bool {
        self.severity >= Severity::Warning
    }
}

/// The `doctor` result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub findings: Vec<Finding>,
}

impl DoctorReport {
    /// The worst severity found, `Ok` for an empty report.
    pub fn worst(&self) -> Severity {
        self.findings
            .iter()
            .map(|f| f.severity)
            .max()
            .unwrap_or(Severity::Ok)
    }
}

/// The `doctor_fix` result: what ran and how it went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorFixed {
    /// The commands that were started, in order.
    pub ran: Vec<String>,
}
