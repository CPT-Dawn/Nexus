//! Running a finding's fix. Only commands the Doctor itself wrote for a
//! finding it just found are ever run, never anything a Front end sent, and
//! never without an explicit `confirm`.

use nexus_ipc::{DoctorFixed, Failure};
use tokio::process::Command;

use crate::backend::Backend;
use crate::config::Settings;
use crate::text;

/// `pkexec` exits 126 when the dialog was dismissed and 127 when the user is
/// not authorized (or no agent could ask).
const PKEXEC_DISMISSED: i32 = 126;
const PKEXEC_REFUSED: i32 = 127;

/// Runs the fix of finding `id`, in order, stopping at the first failure.
pub async fn run_fix<B: Backend>(
    backend: &B,
    settings: &Settings,
    id: &str,
    confirm: bool,
) -> Result<DoctorFixed, Failure> {
    if !confirm {
        return Err(text::fix_not_confirmed().into());
    }
    // Always against a fresh look: what was true a minute ago may not be now.
    let findings = super::check(backend, settings).await;
    let fix = findings
        .into_iter()
        .find(|f| f.id == id)
        .and_then(|f| f.fix)
        .filter(|fix| fix.runnable)
        .ok_or_else(|| Failure::from(text::nothing_to_fix(id)))?;
    let mut ran = Vec::new();
    for command in &fix.commands {
        ran.push(command.clone());
        let mut cmd = match (fix.privileged, command.strip_prefix("sudo ")) {
            (true, Some(rest)) => {
                let mut c = Command::new("pkexec");
                c.args(["/bin/sh", "-c", rest]);
                c
            }
            _ => {
                let mut c = Command::new("/bin/sh");
                c.args(["-c", command]);
                c
            }
        };
        let output = cmd
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .map_err(|e| Failure::from(text::fix_not_started(command, &e)))?;
        if output.status.success() {
            continue;
        }
        let message = match output.status.code() {
            Some(PKEXEC_DISMISSED | PKEXEC_REFUSED) if fix.privileged => format!(
                "`{command}` was cancelled or not authorized, so nothing was changed by it."
            ),
            code => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let last = stderr.lines().last().unwrap_or("").trim();
                let code = code.map_or("a signal".to_owned(), |c| format!("exit code {c}"));
                format!(
                    "`{command}` failed ({code}){}",
                    if last.is_empty() {
                        String::new()
                    } else {
                        format!(": {last}")
                    }
                )
            }
        };
        return Err(text::fix_failed(&message).into());
    }
    Ok(DoctorFixed { ran })
}
