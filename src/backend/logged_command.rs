//! A CLI call as the Terminal tab records it: what ran, when, for how long,
//! and what it printed.

use std::time::Instant;

use chrono::{DateTime, Local};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

use crate::command_log::{Execution, Invocation, Program, TerminalCommand, captured_text};
use crate::event::Event;

/// What a finished child left behind.
pub(crate) struct ChildOutput<'a> {
    /// `None` when the child never started or was killed by a signal.
    pub exit_code: Option<i32>,
    pub stdout: &'a [u8],
    pub stderr: &'a [u8],
}

impl ChildOutput<'_> {
    /// A child that could not be started.
    pub(crate) const NOT_STARTED: ChildOutput<'static> = ChildOutput {
        exit_code: None,
        stdout: &[],
        stderr: &[],
    };
}

impl<'a> From<&'a std::process::Output> for ChildOutput<'a> {
    fn from(output: &'a std::process::Output) -> Self {
        Self {
            exit_code: output.status.code(),
            stdout: &output.stdout,
            stderr: &output.stderr,
        }
    }
}

/// Started just before the child is spawned; `finish` sends the entry to the
/// Terminal tab.
pub(crate) struct LoggedCommand {
    label: String,
    invocation: Invocation,
    started_at: DateTime<Local>,
    clock: Instant,
}

impl LoggedCommand {
    pub(crate) fn start(desc: &str, invocation: Invocation) -> Self {
        Self {
            label: desc.to_uppercase(),
            invocation,
            started_at: Local::now(),
            clock: Instant::now(),
        }
    }

    pub(crate) fn command_line(&self) -> String {
        self.invocation.command_line()
    }

    pub(crate) fn finish(
        self,
        tx: Option<&UnboundedSender<Event>>,
        status: String,
        output: ChildOutput<'_>,
    ) {
        let Some(tx) = tx else {
            return;
        };
        let command = format!("{}: {}", self.label, self.command_line());
        let entry = TerminalCommand {
            timestamp: Local::now().format("%H:%M:%S").to_string(),
            command,
            status,
            execution: self.into_execution(output),
        };
        let _ = tx.send(Event::TerminalCommandLogged(Box::new(entry)));
    }

    fn into_execution(self, output: ChildOutput<'_>) -> Execution {
        Execution {
            started_at: Some(self.started_at),
            duration: Some(self.clock.elapsed()),
            exit_code: output.exit_code,
            stdout: captured_text(output.stdout),
            stderr: captured_text(output.stderr),
            invocation: Some(self.invocation),
        }
    }
}

/// Runs a recorded call again with the same program, arguments and stdin,
/// returning the status to log and how it ran.
pub(crate) async fn rerun(invocation: Invocation) -> (String, Execution) {
    let mut command = Command::new(invocation.program.binary());
    command.args(&invocation.args);
    if invocation.program == Program::Git {
        // Nothing can answer a credential prompt under the TUI.
        command.env("GIT_TERMINAL_PROMPT", "0");
    }
    let stdin = invocation.stdin.clone();

    let _permit = super::rate_limit::RATE_LIMITER.pace_request().await;
    let logged = LoggedCommand::start("", invocation);
    match super::subprocess::output(&mut command, stdin.as_deref().map(str::as_bytes)).await {
        Ok(output) => {
            let status = if output.status.success() {
                "Success".to_string()
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let reason = match stderr.trim() {
                    "" => format!("{} exited with {}", logged.command_line(), output.status),
                    reason => reason.to_string(),
                };
                format!("Failed: {reason}")
            };
            (status, logged.into_execution(ChildOutput::from(&output)))
        }
        Err(error) => (
            format!("Failed: {error}"),
            logged.into_execution(ChildOutput::NOT_STARTED),
        ),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rerun_replays_arguments_and_stdin_and_records_the_output() {
        let invocation = Invocation {
            program: Program::Git,
            args: vec!["hash-object".to_string(), "--stdin".to_string()],
            stdin: Some("hello\n".to_string()),
        };

        let (status, execution) = rerun(invocation.clone()).await;

        assert_eq!(status, "Success");
        assert_eq!(execution.exit_code, Some(0));
        assert_eq!(execution.stdout, "ce013625030ba8dba906f756967f9e9ca394464a");
        assert_eq!(execution.stderr, "");
        assert_eq!(execution.invocation, Some(invocation));
        assert!(execution.started_at.is_some());
        assert!(execution.duration.is_some());
    }

    #[tokio::test]
    async fn rerun_of_a_failing_call_reports_its_stderr() {
        let invocation = Invocation::new(Program::Git, &["no-such-subcommand-for-glab-tui"]);

        let (status, execution) = rerun(invocation).await;

        assert_eq!(execution.exit_code, Some(1));
        assert_eq!(status, format!("Failed: {}", execution.stderr));
        assert!(status.contains("no-such-subcommand-for-glab-tui"));
    }
}
