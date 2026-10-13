use anyhow::Result;
use std::process::{Output, Stdio};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::time::Duration;

/// Runs `command` to completion like `Command::output`, writing `input` to its
/// stdin, and kills it once it outlives `command_timeout_seconds`.
pub async fn output(command: &mut Command, input: Option<&[u8]>) -> Result<Output> {
    output_within(command, input, crate::config::command_timeout()).await
}

async fn output_within(
    command: &mut Command,
    input: Option<&[u8]>,
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

    match tokio::time::timeout(timeout, collect_output(&mut child, input)).await {
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

async fn collect_output(child: &mut Child, input: Option<&[u8]>) -> std::io::Result<Output> {
    let stdin = child.stdin.take();
    let mut stdout_pipe = child.stdout.take().expect("stdout is piped above");
    let mut stderr_pipe = child.stderr.take().expect("stderr is piped above");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let feed_stdin = async move {
        if let (Some(mut pipe), Some(bytes)) = (stdin, input) {
            pipe.write_all(bytes).await?;
            pipe.flush().await?;
        }
        Ok::<_, std::io::Error>(())
    };
    let (status, fed, read_stdout, read_stderr) = tokio::join!(
        child.wait(),
        feed_stdin,
        stdout_pipe.read_to_end(&mut stdout),
        stderr_pipe.read_to_end(&mut stderr),
    );
    fed?;
    read_stdout?;
    read_stderr?;
    Ok(Output {
        status: status?,
        stdout,
        stderr,
    })
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
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        assert!(output.status.success());
        assert_eq!(output.stdout, b"{\"body\":\"hi\"}");
    }
}
