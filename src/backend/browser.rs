use anyhow::Result;
use tokio::sync::mpsc::UnboundedSender;

use super::logged_command::{ChildOutput, LoggedCommand};
use crate::command_log::{Invocation, Program};
use crate::event::Event;

/// Open `url` in the user's browser via `git web--browse`, logged to the
/// Terminal tab under `label` next to the `glab` and `gh` commands. Always
/// returns Ok: a browser helper that fails or exits non-zero is recorded
/// in that log entry instead.
pub(crate) async fn open_url(
    url: &str,
    tx: Option<UnboundedSender<Event>>,
    label: &str,
) -> Result<()> {
    let logged = LoggedCommand::start(label, Invocation::new(Program::Git, &["web--browse", url]));
    let output = tokio::process::Command::new("git")
        .args(["web--browse", url])
        .output()
        .await;
    let (status, child_output) = match &output {
        Ok(out) if out.status.success() => ("Success".to_string(), ChildOutput::from(out)),
        Ok(out) => {
            let child_output = ChildOutput::from(out);
            let reason = logged.failure_reason(&child_output);
            (format!("Failed: {reason}"), child_output)
        }
        Err(error) => (format!("Failed: {error}"), ChildOutput::NOT_STARTED),
    };
    logged.finish(tx.as_ref(), status, child_output);
    Ok(())
}
