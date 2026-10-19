//! A CLI call as the Terminal tab records it: what ran, when, for how long,
//! and what it printed.

use std::time::Instant;

use chrono::{DateTime, Local};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

use crate::command_log::{
    Execution, Invocation, MAX_CAPTURED_OUTPUT_BYTES, Program, TerminalCommand, captured_text,
};
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

    /// Why the call failed, for its `Failed:` status and its error: the
    /// trimmed stderr, or how the command ended when it printed nothing.
    pub(crate) fn failure_reason(&self, output: &ChildOutput<'_>) -> String {
        let stderr = String::from_utf8_lossy(output.stderr);
        let mut reason = match (stderr.trim(), output.exit_code) {
            ("", Some(code)) => format!("{} exited with code {code}", self.command_line()),
            ("", None) => format!("{} was killed by a signal", self.command_line()),
            (stderr, _) => stderr.to_string(),
        };
        // `gh api` prints only the HTTP status on stderr; GitHub's reason
        // (e.g. "Can not approve your own pull request") is in the body on stdout.
        if self.is_gh_api()
            && let Some(details) = github_error_details(output.stdout)
        {
            reason = format!("{reason}: {details}");
        }
        reason
    }

    fn is_gh_api(&self) -> bool {
        self.invocation.program == Program::Gh
            && self.invocation.args.first().is_some_and(|arg| arg == "api")
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

fn github_error_details(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let details: Vec<String> = value
        .get("errors")?
        .as_array()?
        .iter()
        .filter_map(|e| {
            let text = |key| e.get(key).and_then(serde_json::Value::as_str);
            match (
                e.as_str().or_else(|| text("message")),
                text("field"),
                text("code"),
            ) {
                (Some(message), _, _) => Some(message.to_string()),
                (None, Some(field), Some(code)) => Some(format!("{field} {code}")),
                _ => None,
            }
        })
        .collect();
    (!details.is_empty()).then(|| details.join("; "))
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
    // One byte past what an entry keeps lets `captured_text` mark it truncated.
    let output = super::subprocess::output_capped(
        &mut command,
        stdin.as_deref().map(str::as_bytes),
        MAX_CAPTURED_OUTPUT_BYTES + 1,
    )
    .await;
    match output {
        Ok(output) => {
            let child_output = ChildOutput::from(&output);
            let status = if output.status.success() {
                "Success".to_string()
            } else {
                format!("Failed: {}", logged.failure_reason(&child_output))
            };
            (status, logged.into_execution(child_output))
        }
        Err(error) => (
            format!("Failed: {error}"),
            logged.into_execution(ChildOutput::NOT_STARTED),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(
        exit_code: Option<i32>,
        stdout: &'static [u8],
        stderr: &'static [u8],
    ) -> ChildOutput<'static> {
        ChildOutput {
            exit_code,
            stdout,
            stderr,
        }
    }

    #[test]
    fn github_error_details_reads_string_and_object_errors() {
        let body = br#"{"message":"Unprocessable Entity","errors":["Review Can not approve your own pull request",{"resource":"PullRequestReviewComment","code":"custom","message":"line must be part of the diff"},{"resource":"PullRequestReviewComment","field":"in_reply_to","code":"invalid"}],"status":"422"}"#;
        assert_eq!(
            github_error_details(body).as_deref(),
            Some(
                "Review Can not approve your own pull request; line must be part of the diff; in_reply_to invalid"
            )
        );
        assert_eq!(github_error_details(br#"{"message":"Not Found"}"#), None);
        assert_eq!(github_error_details(b"not json"), None);
    }

    #[test]
    fn failure_reason_adds_github_error_details_to_gh_api_calls_only() {
        let body = br#"{"errors":["Review Can not approve your own pull request"]}"#;
        let output = failed(Some(1), body, b"gh: Unprocessable Entity (HTTP 422)\n");
        let api = LoggedCommand::start(
            "",
            Invocation::new(Program::Gh, &["api", "repos/a/b/pulls/1/reviews"]),
        );
        let pr_view = LoggedCommand::start("", Invocation::new(Program::Gh, &["pr", "view", "1"]));

        assert_eq!(
            api.failure_reason(&output),
            "gh: Unprocessable Entity (HTTP 422): Review Can not approve your own pull request"
        );
        assert_eq!(
            pr_view.failure_reason(&output),
            "gh: Unprocessable Entity (HTTP 422)"
        );
    }

    #[test]
    fn failure_reason_without_stderr_names_how_the_command_ended() {
        let logged = LoggedCommand::start("", Invocation::new(Program::Glab, &["mr", "list"]));

        assert_eq!(
            logged.failure_reason(&failed(Some(2), b"", b"  \n")),
            "glab mr list exited with code 2"
        );
        assert_eq!(
            logged.failure_reason(&failed(None, b"", b"")),
            "glab mr list was killed by a signal"
        );
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
    #[tokio::test]
    async fn rerun_of_a_failing_call_reports_its_stderr() {
        let invocation = Invocation::new(Program::Git, &["no-such-subcommand-for-glab-tui"]);

        let (status, execution) = rerun(invocation).await;

        assert_eq!(execution.exit_code, Some(1));
        assert_eq!(status, format!("Failed: {}", execution.stderr));
        assert!(status.contains("no-such-subcommand-for-glab-tui"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rerun_of_a_large_output_keeps_the_logged_prefix_and_marks_it_truncated() {
        let invocation = Invocation::new(
            Program::Git,
            &[
                "-c",
                "alias.big=!head -c 100000 /dev/zero | tr '\\0' z",
                "big",
            ],
        );

        let (status, execution) = rerun(invocation).await;

        assert_eq!(status, "Success");
        assert_eq!(
            execution.stdout,
            format!(
                "{}\n[TRUNCATED: first 16 KiB]",
                "z".repeat(MAX_CAPTURED_OUTPUT_BYTES)
            )
        );
    }
}
