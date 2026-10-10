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
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(io::Error::other(
            "child process has no stdout or stderr pipe",
        ));
    };

    let read_stdout = async {
        let (bytes, is_truncated) = match max_stdout_bytes {
            Some(limit) => read_prefix(stdout, limit).await?,
            None => (read_all(stdout).await?, false),
        };
        if is_truncated {
            child.start_kill()?;
        }
        io::Result::Ok(CapturedStdout {
            bytes,
            is_truncated,
        })
    };
    // Stderr past its limit is drained, not left in the pipe: a child blocked
    // on a full stderr pipe would never finish writing stdout.
    let read_stderr = async {
        let (kept, has_more) = read_prefix_from(stderr, STDERR_MAX_BYTES).await?;
        if let Some(mut rest) = has_more {
            tokio::io::copy(&mut rest, &mut tokio::io::sink()).await?;
        }
        io::Result::Ok(kept)
    };
    let (stdout, stderr) = tokio::try_join!(read_stdout, read_stderr)?;
    let status = child.wait().await?;
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
        let output = output_bounded(&mut shell("yes glab-tui"), Some(1000))
            .await
            .unwrap();
        assert!(output.succeeded());
        assert!(output.stdout.is_truncated);
        assert_eq!(output.stdout.bytes.len(), 1000);
        assert!(output.stdout.bytes.starts_with(b"glab-tui\nglab-tui\n"));
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
