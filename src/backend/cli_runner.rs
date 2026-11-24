//! How both backends run `glab`, `gh` and `git`: every call is logged to the
//! Terminal tab, and every `glab`/`gh` call takes a `RATE_LIMITER` slot and
//! is retried on a rate-limit error.

use anyhow::Result;
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinSet;

use super::bounded_output::{CapturedStdout, output_bounded};
use super::logged_command::{ChildOutput, LoggedCommand};
use crate::command_log::{Invocation, Program};
use crate::event::Event;

#[derive(Clone, Debug)]
pub struct CliRunner {
    program: Program,
    tx: Option<UnboundedSender<Event>>,
}

impl CliRunner {
    pub(crate) fn new(program: Program) -> Self {
        Self { program, tx: None }
    }

    pub(crate) fn set_tx(&mut self, tx: UnboundedSender<Event>) {
        self.tx = Some(tx);
    }

    pub(crate) fn tx(&self) -> Option<&UnboundedSender<Event>> {
        self.tx.as_ref()
    }

    /// The same Terminal tab, running `git` in the working directory.
    pub(crate) fn git(&self) -> Self {
        Self {
            program: Program::Git,
            tx: self.tx.clone(),
        }
    }

    pub(crate) async fn run(&self, args: &[&str], desc: &str) -> Result<String> {
        let stdout = self.run_bounded(args, desc, None).await?;
        Ok(String::from_utf8(stdout.bytes)?)
    }

    /// `run` keeping at most `max_stdout_bytes` of the output.
    pub(crate) async fn run_capped(
        &self,
        args: &[&str],
        desc: &str,
        max_stdout_bytes: usize,
    ) -> Result<CapturedStdout> {
        self.run_bounded(args, desc, Some(max_stdout_bytes)).await
    }

    async fn run_bounded(
        &self,
        args: &[&str],
        desc: &str,
        max_stdout_bytes: Option<usize>,
    ) -> Result<CapturedStdout> {
        if !self.is_api_client() {
            return self.run_once(args, desc, max_stdout_bytes).await;
        }
        super::rate_limit::execute_with_retry(|| self.run_once(args, desc, max_stdout_bytes)).await
    }

    /// Local `git` reads reach no API, so they take no rate-limit slot.
    fn is_api_client(&self) -> bool {
        self.program != Program::Git
    }

    async fn run_once(
        &self,
        args: &[&str],
        desc: &str,
        max_stdout_bytes: Option<usize>,
    ) -> Result<CapturedStdout> {
        let binary = self.program.binary();
        let logged = LoggedCommand::start(desc, Invocation::new(self.program, args));
        let output = match output_bounded(Command::new(binary).args(args), max_stdout_bytes).await {
            Ok(output) => output,
            Err(e) => {
                logged.finish(self.tx(), format!("Failed: {e}"), ChildOutput::NOT_STARTED);
                return Err(e.into());
            }
        };
        if output.succeeded() {
            logged.finish(self.tx(), "Success".to_string(), output.child_output());
            Ok(output.stdout)
        } else {
            let child_output = output.child_output();
            let reason = logged.failure_reason(&child_output);
            logged.finish(self.tx(), format!("Failed: {reason}"), child_output);
            anyhow::bail!("{binary} command failed: {reason}")
        }
    }

    /// Runs every request concurrently and returns each response paired with
    /// the index of the request that produced it, in ascending index order.
    ///
    /// At most `MAX_CONCURRENT_REQUESTS` subprocesses run at once; beyond that
    /// the fetch degrades to waves rather than a subprocess storm. A task that
    /// panics or is cancelled yields an `Err` for its index rather than a
    /// silently missing response, so no caller can mistake it for a short
    /// page.
    pub(crate) async fn run_concurrent(
        &self,
        requests: Vec<Vec<String>>,
        desc: &str,
    ) -> Vec<(usize, Result<String>)> {
        let total = requests.len();
        let mut tasks = JoinSet::new();

        for (index, args) in requests.into_iter().enumerate() {
            let runner = self.clone();
            let desc = desc.to_string();
            tasks.spawn(async move {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                (index, runner.run(&args, &desc).await)
            });
        }

        let mut slots: Vec<Option<Result<String>>> = (0..total).map(|_| None).collect();
        let mut join_failure: Option<String> = None;
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((index, response)) => slots[index] = Some(response),
                // A panicked or cancelled task never reports its index; the empty
                // slot it leaves behind is filled in below.
                Err(e) => join_failure = Some(e.to_string()),
            }
        }

        slots
            .into_iter()
            .enumerate()
            .map(|(index, slot)| {
                let response = slot.unwrap_or_else(|| {
                    let cause = join_failure
                        .clone()
                        .unwrap_or_else(|| "task did not complete".to_string());
                    Err(anyhow::anyhow!("request {} failed: {}", index + 1, cause))
                });
                (index, response)
            })
            .collect()
    }

    /// `<program> api [-X <method>] [--input -] <endpoint>`, with `body`
    /// written to stdin when there is one.
    pub(crate) async fn raw_api(
        &self,
        endpoint: &str,
        method: &str,
        body: Option<&str>,
        desc: &str,
    ) -> Result<String> {
        super::rate_limit::execute_with_retry(|| self.raw_api_once(endpoint, method, body, desc))
            .await
    }

    async fn raw_api_once(
        &self,
        endpoint: &str,
        method: &str,
        body: Option<&str>,
        desc: &str,
    ) -> Result<String> {
        let binary = self.program.binary();
        let input = body.filter(|b| !b.is_empty());
        let mut args = vec!["api"];
        if method != "GET" {
            args.extend(["-X", method]);
        }
        if input.is_some() {
            args.extend(["--input", "-"]);
        }
        args.push(endpoint);
        let logged =
            LoggedCommand::start(desc, Invocation::new(self.program, &args).with_stdin(input));

        let output =
            super::subprocess::output(Command::new(binary).args(&args), input.map(str::as_bytes))
                .await;
        match output {
            Ok(out) if out.status.success() => {
                logged.finish(self.tx(), "Success".to_string(), (&out).into());
                Ok(String::from_utf8(out.stdout)?)
            }
            Ok(out) => {
                let child_output = ChildOutput::from(&out);
                let reason = logged.failure_reason(&child_output);
                logged.finish(self.tx(), format!("Failed: {reason}"), child_output);
                anyhow::bail!("{binary} api failed: {reason}")
            }
            Err(e) => {
                logged.finish(self.tx(), format!("Failed: {e}"), ChildOutput::NOT_STARTED);
                Err(e)
            }
        }
    }

    /// Opens `url` in the user's browser via `git web--browse`, logged under
    /// `label`. Always returns Ok: a browser helper that fails or exits
    /// non-zero is recorded in that log entry instead.
    pub(crate) async fn open_url(&self, url: &str, label: &str) -> Result<()> {
        let args = ["web--browse", url];
        let logged = LoggedCommand::start(label, Invocation::new(Program::Git, &args));
        let output = Command::new(Program::Git.binary())
            .args(args)
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
        logged.finish(self.tx(), status, child_output);
        Ok(())
    }
}
