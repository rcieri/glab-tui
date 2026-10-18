//! Child process output read up to a byte limit, so a command that writes
//! without end (a CI log, a diff) cannot make glab-tui buffer all of it.

use std::io;
use std::process::{ExitStatus, Stdio};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

/// Stderr only ever feeds an error message.
const STDERR_MAX_BYTES: usize = 64 * 1024;

/// Stdout of a command that succeeded.
#[derive(Debug)]
pub(crate) struct CapturedStdout {
    pub bytes: Vec<u8>,
    /// The command wrote more than the limit: `bytes` holds the first part.
    pub is_truncated: bool,
}

pub(crate) struct BoundedOutput {
    status: ExitStatus,
    pub stdout: CapturedStdout,
    pub stderr: Vec<u8>,
}

impl BoundedOutput {
    /// A child stopped at the limit is killed and exits on that signal. That
    /// is this reader's doing, not a failure of the command.
    pub fn succeeded(&self) -> bool {
        self.stdout.is_truncated || self.status.success()
    }
}

/// Runs `command` like `Command::output`, keeping at most `max_stdout_bytes`
/// of stdout (`None`: all of it). A child that writes more is killed, so the
/// rest is never read.
pub(crate) async fn output_bounded(
    command: &mut Command,
    max_stdout_bytes: Option<usize>,
) -> io::Result<BoundedOutput> {
    output_bounded_within(command, max_stdout_bytes, crate::config::command_timeout()).await
}

pub(crate) async fn output_bounded_within(
    command: &mut Command,
    max_stdout_bytes: Option<usize>,
    timeout: std::time::Duration,
) -> io::Result<BoundedOutput> {
    let invocation = super::subprocess::describe(command);
    // Uncapped runs carry mutations (merge, approve, delete): a CLI already
    // started finishes even when glab-tui quits before it does.
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(max_stdout_bytes.is_some());
    let mut child = command
        .spawn()
        .map_err(|e| io::Error::other(format!("Failed to execute: {invocation}: {e}")))?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(io::Error::other(
            "child process has no stdout or stderr pipe",
        ));
    };

    let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<()>();

    let read_stdout = async move {
        let (bytes, is_truncated) = match max_stdout_bytes {
            Some(limit) => read_prefix(stdout, limit).await?,
            None => (read_all(stdout).await?, false),
        };
        if is_truncated {
            let _ = kill_tx.send(());
        }
        io::Result::Ok(CapturedStdout {
            bytes,
            is_truncated,
        })
    };
    let read_stderr = async move {
        let (kept, has_more) = read_prefix_from(stderr, STDERR_MAX_BYTES).await?;
        if let Some(mut rest) = has_more {
            tokio::io::copy(&mut rest, &mut tokio::io::sink()).await?;
        }
        io::Result::Ok(kept)
    };
    let killer = async {
        if kill_rx.await.is_ok() {
            let _ = child.start_kill();
        }
        io::Result::Ok(())
    };

    let read_all_streams = async {
        let (stdout, stderr, _) = tokio::try_join!(read_stdout, read_stderr, killer)?;
        io::Result::Ok((stdout, stderr))
    };

    let (stdout, stderr) = match tokio::time::timeout(timeout, read_all_streams).await {
        Ok(res) => res?,
        Err(_) => {
            let _ = child.kill().await;
            return Err(io::Error::other(format!(
                "{invocation} timed out after {timeout:?} and was killed \
                 (raise command_timeout_seconds in config.toml for a slow host)"
            )));
        }
    };
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(res) => res?,
        Err(_) => {
            let _ = child.kill().await;
            return Err(io::Error::other(format!(
                "{invocation} timed out after {timeout:?} and was killed \
                 (raise command_timeout_seconds in config.toml for a slow host)"
            )));
        }
    };
    Ok(BoundedOutput {
        status,
        stdout,
        stderr,
    })
}

async fn read_all(mut reader: impl AsyncRead + Unpin) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await?;
    Ok(bytes)
}

/// The first `limit` bytes of `reader`, and whether it had more.
async fn read_prefix(reader: impl AsyncRead + Unpin, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let (kept, rest) = read_prefix_from(reader, limit).await?;
    Ok((kept, rest.is_some()))
}

/// The first `limit` bytes of `reader`, and the reader itself when more
/// follows (one byte of it consumed to find out).
async fn read_prefix_from<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> io::Result<(Vec<u8>, Option<R>)> {
    let mut kept = Vec::new();
    (&mut reader)
        .take(u64::try_from(limit).unwrap_or(u64::MAX))
        .read_to_end(&mut kept)
        .await?;
    let mut probe = [0u8; 1];
    let has_more = reader.read(&mut probe).await? > 0;
    Ok((kept, has_more.then_some(reader)))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn shell(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    #[tokio::test]
    async fn output_within_the_limit_is_read_whole() {
        let output = output_bounded(&mut shell("printf 'hello'"), Some(5))
            .await
            .unwrap();
        assert!(output.succeeded());
        assert_eq!(output.stdout.bytes, b"hello");
        assert!(!output.stdout.is_truncated);
    }

    #[tokio::test]
    async fn endless_output_is_cut_at_the_limit_and_the_child_killed() {
        // Ignoring SIGPIPE keeps the child writing once its stdout is closed,
        // so only the kill ends it. The timeout turns a missing kill into a
        // failure instead of a hang.
        let mut command = shell("trap '' PIPE; while :; do printf 'glab-tui'; done");
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            output_bounded(&mut command, Some(1000)),
        )
        .await
        .expect("the child is killed at the limit")
        .unwrap();
        assert!(output.succeeded());
        assert!(output.stdout.is_truncated);
        assert_eq!(output.stdout.bytes, b"glab-tui".repeat(125));
    }

    #[tokio::test]
    async fn unlimited_read_keeps_everything() {
        let output = output_bounded(&mut shell("head -c 300000 /dev/zero"), None)
            .await
            .unwrap();
        assert_eq!(output.stdout.bytes.len(), 300_000);
        assert!(!output.stdout.is_truncated);
    }

    #[tokio::test]
    async fn failure_reports_stderr_and_stderr_past_its_limit_does_not_block() {
        let output = output_bounded(
            &mut shell("head -c 200000 /dev/zero | tr '\\0' e >&2; printf out; exit 3"),
            Some(10),
        )
        .await
        .unwrap();
        assert!(!output.succeeded());
        assert_eq!(output.stdout.bytes, b"out");
        assert_eq!(output.stderr.len(), STDERR_MAX_BYTES);
        assert!(output.stderr.iter().all(|&b| b == b'e'));
    }
}
