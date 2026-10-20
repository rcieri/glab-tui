use anyhow::Result;
use std::process::{Output, Stdio};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::time::Duration;

/// Runs `command` to completion like `Command::output`, writing `input` to its
/// stdin, and kills it once it outlives `command_timeout_seconds`.
pub async fn output(command: &mut Command, input: Option<&[u8]>) -> Result<Output> {
    output_within(command, input, None, crate::config::command_timeout()).await
}

/// `output`, keeping at most `max_bytes_per_stream` of stdout and of stderr.
/// The rest is drained rather than the child killed, so a mutation still
/// finishes.
pub(crate) async fn output_capped(
    command: &mut Command,
    input: Option<&[u8]>,
    max_bytes_per_stream: usize,
) -> Result<Output> {
    output_within(
        command,
        input,
        Some(max_bytes_per_stream),
        crate::config::command_timeout(),
    )
    .await
}

async fn output_within(
    command: &mut Command,
    input: Option<&[u8]>,
    max_bytes_per_stream: Option<usize>,
    timeout: Duration,
) -> Result<Output> {
    let invocation = describe(command);
    let stdin = if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    // stdout/stderr must be piped: inherited, they are the TTY under the
    // ratatui alternate screen and the CLI pages JSON through $PAGER.
    command
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to execute: {invocation}: {e}"))?;

    match tokio::time::timeout(
        timeout,
        collect_output(&mut child, input, max_bytes_per_stream),
    )
    .await
    {
        Ok(output) => output.map_err(|e| anyhow::anyhow!("Failed to execute: {invocation}: {e}")),
        Err(_) => {
            // Awaiting `kill` reaps the direct child only. Processes it spawned are not signalled.
            let _ = child.kill().await;
            anyhow::bail!(
                "{invocation} timed out after {timeout:?} and was killed \
                 (raise command_timeout_seconds in config.toml for a slow host)"
            )
        }
    }
}

async fn collect_output(
    child: &mut Child,
    input: Option<&[u8]>,
    max_bytes_per_stream: Option<usize>,
) -> std::io::Result<Output> {
    let stdin = child.stdin.take();
    let stdout_pipe = child.stdout.take().expect("stdout is piped above");
    let stderr_pipe = child.stderr.take().expect("stderr is piped above");

    let feed_stdin = async move {
        if let (Some(mut pipe), Some(bytes)) = (stdin, input) {
            pipe.write_all(bytes).await?;
            pipe.flush().await?;
        }
        Ok::<_, std::io::Error>(())
    };
    let (status, fed, stdout, stderr) = tokio::join!(
        child.wait(),
        feed_stdin,
        read_stream(stdout_pipe, max_bytes_per_stream),
        read_stream(stderr_pipe, max_bytes_per_stream),
    );
    fed?;
    Ok(Output {
        status: status?,
        stdout: stdout?,
        stderr: stderr?,
    })
}

async fn read_stream(
    mut reader: impl AsyncRead + Unpin,
    limit: Option<usize>,
) -> std::io::Result<Vec<u8>> {
    match limit {
        Some(limit) => super::bounded_output::read_prefix_draining(reader, limit).await,
        None => {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await?;
            Ok(bytes)
        }
    }
}

pub(crate) fn describe(command: &Command) -> String {
    let command = command.as_std();
    let mut text = command.get_program().to_string_lossy().into_owned();
    for arg in command.get_args() {
        text.push(' ');
        text.push_str(&arg.to_string_lossy());
    }
    text
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hung_command_is_killed_and_reported_after_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let script = format!("echo $$ > {}; exec sleep 30", pid_file.display());
        let started = std::time::Instant::now();

        let err = output_within(
            Command::new("sh").args(["-c", &script]),
            None,
            None,
            Duration::from_millis(500),
        )
        .await
        .unwrap_err();

        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(
            err.to_string(),
            format!(
                "sh -c {script} timed out after 500ms and was killed \
                 (raise command_timeout_seconds in config.toml for a slow host)"
            )
        );
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        let probe = std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(!probe.success(), "pid {} is still running", pid.trim());
    }

    #[tokio::test]
    async fn input_is_written_to_stdin_and_closed() {
        let output = output_within(
            &mut Command::new("cat"),
            Some(b"{\"body\":\"hi\"}"),
            None,
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        assert!(output.status.success());
        assert_eq!(output.stdout, b"{\"body\":\"hi\"}");
    }

    #[tokio::test]
    async fn capped_output_keeps_a_prefix_of_each_stream_and_lets_the_child_finish() {
        let output = output_within(
            Command::new("sh").args([
                "-c",
                "head -c 200000 /dev/zero | tr '\\0' o; head -c 200000 /dev/zero | tr '\\0' e >&2; exit 3",
            ]),
            None,
            Some(1000),
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        assert_eq!(output.status.code(), Some(3));
        assert_eq!(output.stdout, b"o".repeat(1000));
        assert_eq!(output.stderr, b"e".repeat(1000));
    }
}
