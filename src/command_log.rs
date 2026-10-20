use std::borrow::Cow;
use std::collections::VecDeque;
use std::time::Duration;

use chrono::{DateTime, Local};

/// Most entries the Terminal tab keeps; pushing past it drops the oldest.
pub const MAX_LOG_ENTRIES: usize = 1000;

/// Most bytes of each output stream an entry keeps. The log holds up to
/// `MAX_LOG_ENTRIES` entries, and one listing or diff can be megabytes.
pub const MAX_CAPTURED_OUTPUT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Program {
    Glab,
    Gh,
    Git,
}

impl Program {
    pub fn binary(self) -> &'static str {
        match self {
            Self::Glab => "glab",
            Self::Gh => "gh",
            Self::Git => "git",
        }
    }
}

/// A CLI call exactly as a backend ran it, so the Terminal tab can run it
/// again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: Program,
    pub args: Vec<String>,
    /// Written to the command's stdin: the request body of `api --input -`.
    pub stdin: Option<String>,
}

impl Invocation {
    pub fn new<S: AsRef<str>>(program: Program, args: &[S]) -> Self {
        Self {
            program,
            args: args.iter().map(|arg| arg.as_ref().to_string()).collect(),
            stdin: None,
        }
    }

    pub fn with_stdin(mut self, stdin: Option<&str>) -> Self {
        self.stdin = stdin.map(str::to_string);
        self
    }

    /// The call as a shell command line. An argument that is not a plain word
    /// is single-quoted, so a copied line runs as recorded and a title such as
    /// `fix $(id)` is not expanded by the shell it is pasted into.
    pub fn command_line(&self) -> String {
        std::iter::once(Cow::Borrowed(self.program.binary()))
            .chain(self.args.iter().map(|arg| shell_quote(arg)))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Characters besides ASCII letters and digits that no interactive shell
/// gives a meaning to inside a word.
const PLAIN_WORD_PUNCTUATION: &str = "-_./:=@%+,";

/// `arg` as one shell word: unchanged when it is a plain word, else
/// single-quoted with each `'` written as `'\''`. A leading `=` is quoted
/// because zsh expands `=name` to the path of the command `name`.
fn shell_quote(arg: &str) -> Cow<'_, str> {
    let is_plain_word = !arg.is_empty()
        && !arg.starts_with('=')
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || PLAIN_WORD_PUNCTUATION.contains(c));
    if is_plain_word {
        Cow::Borrowed(arg)
    } else {
        Cow::Owned(format!("'{}'", arg.replace('\'', r"'\''")))
    }
}

/// How an entry ran. Entries that describe an action rather than a CLI call
/// (`CommandStarted`, custom commands) only have the timing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Execution {
    pub started_at: Option<DateTime<Local>>,
    pub duration: Option<Duration>,
    /// `None` when the command never started or was killed by a signal.
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub invocation: Option<Invocation>,
}

/// `bytes` as text, cut to `MAX_CAPTURED_OUTPUT_BYTES` with a marker when it
/// is longer.
pub fn captured_text(bytes: &[u8]) -> String {
    let kept = &bytes[..bytes.len().min(MAX_CAPTURED_OUTPUT_BYTES)];
    let mut text = String::from_utf8_lossy(kept).trim_end().to_string();
    if bytes.len() > MAX_CAPTURED_OUTPUT_BYTES {
        text.push_str(&format!(
            "\n[TRUNCATED: first {} KiB]",
            MAX_CAPTURED_OUTPUT_BYTES / 1024
        ));
    }
    text
}

#[derive(Debug, Clone, Default)]
pub struct TerminalCommand {
    pub timestamp: String,
    pub command: String,
    pub status: String,
    pub execution: Execution,
}

impl TerminalCommand {
    /// The command as it was run: the recorded CLI call, else the text after
    /// a `Description: ` prefix, or the whole entry when it has none.
    pub fn command_line(&self) -> Cow<'_, str> {
        if let Some(invocation) = &self.execution.invocation {
            return Cow::Owned(invocation.command_line());
        }
        let command = self.command.trim();
        Cow::Borrowed(command.split_once(": ").map_or(command, |(_, line)| line))
    }

    pub fn is_failed(&self) -> bool {
        self.status.starts_with("Failed")
    }

    pub fn is_succeeded(&self) -> bool {
        self.status == "Success"
    }

    pub fn failure_reason(&self) -> Option<&str> {
        self.status
            .strip_prefix("Failed: ")
            .filter(|reason| !reason.trim().is_empty())
    }

    /// What the command printed, stdout before stderr. A failure that printed
    /// nothing gives its reason instead.
    pub fn output(&self) -> Option<String> {
        let printed: Vec<&str> = [&self.execution.stdout, &self.execution.stderr]
            .into_iter()
            .map(|stream| stream.trim_end())
            .filter(|stream| !stream.is_empty())
            .collect();
        if printed.is_empty() {
            return self.failure_reason().map(str::to_string);
        }
        Some(printed.join("\n"))
    }
}

/// Which entries the Terminal tab lists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StatusFilter {
    #[default]
    All,
    Failed,
    Succeeded,
}

impl StatusFilter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Failed,
            Self::Failed => Self::Succeeded,
            Self::Succeeded => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Failed => "Errors",
            Self::Succeeded => "Success",
        }
    }

    pub fn admits(self, entry: &TerminalCommand) -> bool {
        match self {
            Self::All => true,
            Self::Failed => entry.is_failed(),
            Self::Succeeded => entry.is_succeeded(),
        }
    }
}

/// Handle to a log entry that stays valid while older entries are dropped or
/// the log is cleared; it simply stops resolving once its entry is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogRow(usize);

/// The Terminal tab's command history, bounded to `MAX_LOG_ENTRIES`.
#[derive(Debug, Default)]
pub struct CommandLog {
    entries: VecDeque<TerminalCommand>,
    /// Entries removed from the front so far: `LogRow(n)` lives at index
    /// `n - dropped`.
    dropped: usize,
}

impl CommandLog {
    pub fn push(&mut self, entry: TerminalCommand) -> LogRow {
        if self.entries.len() == MAX_LOG_ENTRIES {
            self.entries.pop_front();
            self.dropped += 1;
        }
        self.entries.push_back(entry);
        LogRow(self.dropped + self.entries.len() - 1)
    }

    pub fn get(&self, row: LogRow) -> Option<&TerminalCommand> {
        self.entries.get(row.0.checked_sub(self.dropped)?)
    }

    pub fn get_mut(&mut self, row: LogRow) -> Option<&mut TerminalCommand> {
        self.entries.get_mut(row.0.checked_sub(self.dropped)?)
    }

    /// The most recent entry matching `predicate`.
    pub fn find_latest(&self, predicate: impl Fn(&TerminalCommand) -> bool) -> Option<LogRow> {
        self.entries
            .iter()
            .rposition(predicate)
            .map(|index| LogRow(self.dropped + index))
    }

    pub fn set_status(&mut self, row: LogRow, status: String) {
        self.settle(row, status, None);
    }

    /// Stamps a finished entry's status. `execution` replaces what the entry
    /// knew about its run; without one, the time since it started becomes its
    /// duration.
    pub fn settle(&mut self, row: LogRow, status: String, execution: Option<Execution>) {
        let Some(entry) = self.get_mut(row) else {
            return;
        };
        entry.status = status;
        match execution {
            Some(execution) => entry.execution = execution,
            None => {
                if let (None, Some(started_at)) =
                    (entry.execution.duration, entry.execution.started_at)
                {
                    entry.execution.duration = (Local::now() - started_at).to_std().ok();
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.dropped += self.entries.len();
        self.entries.clear();
    }

    pub fn iter(&self) -> std::collections::vec_deque::Iter<'_, TerminalCommand> {
        self.entries.iter()
    }

    /// Every entry with the row that keeps pointing at it, oldest first.
    pub fn rows(&self) -> impl Iterator<Item = (LogRow, &TerminalCommand)> {
        let dropped = self.dropped;
        self.entries
            .iter()
            .enumerate()
            .map(move |(index, entry)| (LogRow(dropped + index), entry))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(command: &str) -> TerminalCommand {
        TerminalCommand {
            timestamp: String::new(),
            command: command.to_string(),
            status: "Running".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn pushing_past_the_cap_drops_the_oldest_entries() {
        let mut log = CommandLog::default();
        for index in 0..MAX_LOG_ENTRIES + 5 {
            log.push(entry(&format!("cmd {index}")));
        }

        assert_eq!(log.len(), MAX_LOG_ENTRIES);
        assert_eq!(log.iter().next().map(|e| e.command.as_str()), Some("cmd 5"));
        assert_eq!(
            log.iter().last().map(|e| e.command.as_str()),
            Some(format!("cmd {}", MAX_LOG_ENTRIES + 4).as_str())
        );
    }

    #[test]
    fn a_row_keeps_pointing_at_its_entry_after_older_entries_are_dropped() {
        let mut log = CommandLog::default();
        log.push(entry("first"));
        let row = log.push(entry("second"));
        for index in 0..MAX_LOG_ENTRIES - 1 {
            log.push(entry(&format!("filler {index}")));
        }

        log.set_status(row, "Success".to_string());

        let settled = log.get(row).expect("second is still within the cap");
        assert_eq!(settled.command, "second");
        assert_eq!(settled.status, "Success");
    }

    #[test]
    fn a_row_whose_entry_was_dropped_no_longer_resolves() {
        let mut log = CommandLog::default();
        let row = log.push(entry("evicted"));
        for index in 0..MAX_LOG_ENTRIES {
            log.push(entry(&format!("filler {index}")));
        }

        log.set_status(row, "Success".to_string());

        assert!(log.get(row).is_none());
        assert!(log.iter().all(|e| e.status == "Running"));
    }

    #[test]
    fn clearing_empties_the_log_and_invalidates_earlier_rows() {
        let mut log = CommandLog::default();
        let before = log.push(entry("before"));
        log.clear();
        let after = log.push(entry("after"));

        log.set_status(before, "Success".to_string());

        assert_eq!(log.len(), 1);
        assert!(log.get(before).is_none());
        assert_eq!(log.get(after).map(|e| e.status.as_str()), Some("Running"));
    }

    #[test]
    fn find_latest_returns_the_most_recent_match() {
        let mut log = CommandLog::default();
        log.push(entry("glab mr list"));
        let latest = log.push(entry("glab mr list"));
        log.push(entry("gh pr list"));

        assert_eq!(
            log.find_latest(|e| e.command == "glab mr list"),
            Some(latest)
        );
        assert_eq!(log.find_latest(|e| e.command == "missing"), None);
    }

    #[test]
    fn command_line_drops_the_description_prefix() {
        assert_eq!(
            entry("Fetch issues: glab issue list -R a/b").command_line(),
            "glab issue list -R a/b"
        );
        assert_eq!(entry("  glab mr list  ").command_line(), "glab mr list");
    }

    #[test]
    fn command_line_prefers_the_recorded_invocation() {
        let mut logged = entry("Creating branch: x from main");
        logged.execution.invocation = Some(
            Invocation::new(Program::Glab, &["api", "-X", "POST", "--input", "-", "b"])
                .with_stdin(Some("{}")),
        );

        assert_eq!(logged.command_line(), "glab api -X POST --input - b");
    }

    #[test]
    fn command_line_quotes_arguments_that_are_not_plain_words() {
        let invocation = Invocation::new(
            Program::Glab,
            &[
                "mr",
                "note",
                "create",
                "-m",
                "fix $(id) it's",
                "--label=",
                "",
                "=ls",
                "a,b@c:1/d.e%f+g",
            ],
        );

        assert_eq!(
            invocation.command_line(),
            r"glab mr note create -m 'fix $(id) it'\''s' --label= '' '=ls' a,b@c:1/d.e%f+g"
        );
    }

    #[test]
    fn settling_without_an_execution_times_the_entry_from_its_start() {
        let mut log = CommandLog::default();
        let mut running = entry("Creating branch: x");
        running.execution.started_at = Some(Local::now() - chrono::Duration::seconds(3));
        let row = log.push(running);

        log.set_status(row, "Success".to_string());

        let duration = log.get(row).and_then(|e| e.execution.duration);
        assert!(duration.is_some_and(|d| d >= Duration::from_secs(3)));
    }

    #[test]
    fn settling_with_an_execution_replaces_what_the_entry_knew() {
        let mut log = CommandLog::default();
        let row = log.push(entry("FETCH: glab mr list"));
        let execution = Execution {
            duration: Some(Duration::from_millis(5)),
            exit_code: Some(0),
            stdout: "[]".to_string(),
            invocation: Some(Invocation::new(Program::Glab, &["mr", "list"])),
            ..Default::default()
        };

        log.settle(row, "Success".to_string(), Some(execution.clone()));

        let settled = log.get(row).expect("row resolves");
        assert_eq!(settled.status, "Success");
        assert_eq!(settled.execution, execution);
    }

    #[test]
    fn captured_text_keeps_short_output_and_marks_cut_output() {
        assert_eq!(captured_text(b"ok\n"), "ok");

        let long = vec![b'a'; MAX_CAPTURED_OUTPUT_BYTES + 1];
        let text = captured_text(&long);

        assert_eq!(
            text,
            format!(
                "{}\n[TRUNCATED: first 16 KiB]",
                "a".repeat(MAX_CAPTURED_OUTPUT_BYTES)
            )
        );
    }
}
