use std::collections::VecDeque;

/// Most entries the Terminal tab keeps; pushing past it drops the oldest.
pub const MAX_LOG_ENTRIES: usize = 1000;

#[derive(Debug, Clone)]
pub struct TerminalCommand {
    pub timestamp: String,
    pub command: String,
    pub status: String,
}

impl TerminalCommand {
    /// The command as it was run: the text after a `Description: ` prefix,
    /// or the whole entry when it has none.
    pub fn command_line(&self) -> &str {
        let command = self.command.trim();
        command.split_once(": ").map_or(command, |(_, line)| line)
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
        if let Some(entry) = self.get_mut(row) {
            entry.status = status;
        }
    }

    pub fn clear(&mut self) {
        self.dropped += self.entries.len();
        self.entries.clear();
    }

    pub fn iter(&self) -> std::collections::vec_deque::Iter<'_, TerminalCommand> {
        self.entries.iter()
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
}
