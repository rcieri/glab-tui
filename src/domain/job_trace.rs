//! A job log read in bounded pieces: the first read takes it from the start,
//! follow mode appends only what was written since.

/// Most bytes of one job log glab-tui reads. A longer log is cut there and
/// marked truncated rather than buffered whole.
pub const JOB_TRACE_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Bytes an incremental read requests again ahead of the new content. The
/// reply has to start with them, which tells a server that honoured the
/// requested offset apart from one that sent the whole log again.
const OVERLAP_BYTES: usize = 1024;

/// How far a log has been read, and the last bytes read to check that a
/// later read lines up with them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraceCursor {
    read_bytes: usize,
    tail: Vec<u8>,
}

impl TraceCursor {
    pub fn read_bytes(&self) -> usize {
        self.read_bytes
    }

    /// Length of the tail an incremental read re-requests.
    pub fn overlap_len(&self) -> usize {
        self.tail.len()
    }

    /// Offset an incremental read starts at: the re-requested tail first.
    pub fn resume_offset(&self) -> usize {
        self.read_bytes - self.tail.len()
    }

    /// Bytes still allowed under `JOB_TRACE_MAX_BYTES`.
    pub fn remaining_bytes(&self) -> usize {
        JOB_TRACE_MAX_BYTES.saturating_sub(self.read_bytes)
    }

    /// What follows the read part in `reply`, a read from `resume_offset`.
    /// `None` when `reply` does not start with the re-requested tail.
    pub fn new_bytes_in_resumed_read<'a>(&self, reply: &'a [u8]) -> Option<&'a [u8]> {
        reply.strip_prefix(self.tail.as_slice())
    }

    /// What follows the read part in `log`, read from its first byte. `None`
    /// when `log` no longer holds the read part where it was.
    pub fn new_bytes_in_full_read<'a>(&self, log: &'a [u8]) -> Option<&'a [u8]> {
        log.get(self.resume_offset()..)?
            .strip_prefix(self.tail.as_slice())
    }

    pub fn advance(&mut self, bytes: &[u8]) {
        self.read_bytes += bytes.len();
        let kept_from = bytes.len().saturating_sub(OVERLAP_BYTES);
        self.tail.extend_from_slice(&bytes[kept_from..]);
        let excess = self.tail.len().saturating_sub(OVERLAP_BYTES);
        self.tail.drain(..excess);
    }

    /// An update appending `bytes`, which follow this cursor.
    pub fn append_update(&self, bytes: Vec<u8>, is_truncated: bool) -> TraceUpdate {
        TraceUpdate::Append {
            after_bytes: self.read_bytes,
            bytes,
            is_truncated,
        }
    }

    /// An update replacing what this cursor read with `trace`.
    pub fn restart_update(&self, trace: JobTrace) -> TraceUpdate {
        TraceUpdate::Restart {
            after_bytes: self.read_bytes,
            trace,
        }
    }
}

/// The result of reading a log again from a `TraceCursor`.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceUpdate {
    /// `bytes` follow the first `after_bytes` bytes of the log.
    Append {
        after_bytes: usize,
        bytes: Vec<u8>,
        is_truncated: bool,
    },
    /// The log no longer starts with what was read (it was erased or
    /// rewritten). `trace` is a fresh read from its start.
    Restart { after_bytes: usize, trace: JobTrace },
}

impl TraceUpdate {
    fn after_bytes(&self) -> usize {
        match self {
            Self::Append { after_bytes, .. } | Self::Restart { after_bytes, .. } => *after_bytes,
        }
    }

    /// Whether applying this update adds text: an empty append does not.
    pub fn has_new_text(&self) -> bool {
        !matches!(self, Self::Append { bytes, .. } if bytes.is_empty())
    }
}

/// A job log as text, read up to `JOB_TRACE_MAX_BYTES`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobTrace {
    text: String,
    cursor: TraceCursor,
    /// Leading bytes of a character the last read ended in the middle of,
    /// decoded once the rest of it arrives.
    split_char: Vec<u8>,
    is_truncated: bool,
}

impl JobTrace {
    /// A log read from its start. `is_truncated` when the read stopped at
    /// the byte limit with more to come.
    pub fn from_read(bytes: &[u8], is_truncated: bool) -> Self {
        let mut trace = Self::default();
        trace.append(bytes, is_truncated);
        trace
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> &TraceCursor {
        &self.cursor
    }

    pub fn is_truncated(&self) -> bool {
        self.is_truncated
    }

    /// Index of the first line after `current` (from the top when `None`)
    /// that reports a failure, wrapping around to the first such line.
    pub fn next_failure_line(&self, current: Option<usize>) -> Option<usize> {
        let is_ahead = |index: usize| current.is_none_or(|current| index > current);
        let mut failures = self
            .text
            .lines()
            .enumerate()
            .filter(|(_, line)| is_failure_line(line))
            .map(|(index, _)| index);
        let first = failures.next()?;
        if is_ahead(first) {
            return Some(first);
        }
        Some(failures.find(|&index| is_ahead(index)).unwrap_or(first))
    }

    /// Applies an update read from this trace's cursor. Returns `false`, and
    /// changes nothing, when the update was read from another position (the
    /// trace changed while it was being read).
    pub fn apply(&mut self, update: TraceUpdate) -> bool {
        if update.after_bytes() != self.cursor.read_bytes {
            return false;
        }
        match update {
            TraceUpdate::Append {
                bytes,
                is_truncated,
                ..
            } => self.append(&bytes, is_truncated),
            TraceUpdate::Restart { trace, .. } => *self = trace,
        }
        true
    }

    fn append(&mut self, bytes: &[u8], is_truncated: bool) {
        let kept = &bytes[..bytes.len().min(self.cursor.remaining_bytes())];
        self.is_truncated |= is_truncated || kept.len() < bytes.len();
        self.cursor.advance(kept);
        if self.split_char.is_empty() {
            self.split_char = push_utf8_lossy(&mut self.text, kept).to_vec();
        } else {
            let mut joined = std::mem::take(&mut self.split_char);
            joined.extend_from_slice(kept);
            self.split_char = push_utf8_lossy(&mut self.text, &joined).to_vec();
        }
    }
}

/// Words a CI log line reports a failure with: runner verdicts (`ERROR: Job
/// failed`, `##[error]`), compiler and test diagnostics (`--- FAIL:`), panics
/// and stack traces. Matched as whole words, so `errors: 0`, `allow_failure`
/// and `proc-macro-error` are not, and not with a zero count next to them, so
/// `0 failed`, `Failed: 0` and `failed=0` are not either.
const FAILURE_WORDS: &[&str] = &[
    "error",
    "fatal",
    "fail",
    "failed",
    "failure",
    "panicked",
    "traceback",
    "exception",
];

fn is_failure_line(line: &str) -> bool {
    let line =
        crate::utils::format::sanitize_untrusted(crate::utils::format::trace_line_content(line));
    words(&line).any(|(start, word)| {
        FAILURE_WORDS
            .iter()
            .any(|failure| word.eq_ignore_ascii_case(failure))
            && !is_zero_count_before(&line[..start])
            && !is_zero_count_after(&line[start + word.len()..])
    })
}

/// Each word of `text` with its byte offset, less a sentence's trailing `.`.
fn words(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut searched = 0;
    std::iter::from_fn(move || {
        let start = searched + text[searched..].find(is_word_char)?;
        let end = text[start..]
            .find(|c: char| !is_word_char(c))
            .map_or(text.len(), |len| start + len);
        searched = end;
        Some((start, text[start..end].trim_end_matches('.')))
    })
}

/// `0 failed`, but not Ansible's `unreachable=0 failed=1`.
fn is_zero_count_before(before: &str) -> bool {
    before.trim_end().rsplit(char::is_whitespace).next() == Some("0")
}

/// `Failed: 0`, `failed=0` or `# fail 0`, but not cargo's `FAILED. 0 passed`
/// or `1 failed; 0 ignored`.
fn is_zero_count_after(after: &str) -> bool {
    after
        .trim_start_matches(|c: char| matches!(c, ':' | '=') || c.is_whitespace())
        .split(|c: char| !is_word_char(c))
        .next()
        .is_some_and(|count| count.trim_end_matches('.') == "0")
}

/// Characters a word is made of. `-`, `.` and `/` keep crate names and paths
/// such as `quick-error` or `src/error.rs` whole.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/')
}

/// Decodes `bytes` onto `text`, invalid sequences as U+FFFD, and returns the
/// bytes of a character cut off at the end, which a later read completes.
fn push_utf8_lossy<'a>(text: &mut String, mut bytes: &'a [u8]) -> &'a [u8] {
    loop {
        match std::str::from_utf8(bytes) {
            Ok(valid) => {
                text.push_str(valid);
                return &[];
            }
            Err(error) => {
                let (valid, rest) = bytes.split_at(error.valid_up_to());
                text.push_str(&String::from_utf8_lossy(valid));
                match error.error_len() {
                    None => return rest,
                    Some(invalid_len) => {
                        text.push(char::REPLACEMENT_CHARACTER);
                        bytes = &rest[invalid_len..];
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `len` bytes of numbered lines, so no two stretches of it are equal.
    fn log_of(len: usize) -> Vec<u8> {
        (0..)
            .flat_map(|line: usize| format!("{line:07}\n").into_bytes())
            .take(len)
            .collect()
    }

    #[test]
    fn resumed_read_appends_only_the_bytes_after_the_overlap() {
        let log = log_of(5000);
        let mut trace = JobTrace::from_read(&log[..3000], false);
        let cursor = trace.cursor().clone();
        assert_eq!(cursor.resume_offset(), 3000 - OVERLAP_BYTES);

        let reply = &log[cursor.resume_offset()..];
        let new = cursor.new_bytes_in_resumed_read(reply).unwrap();
        assert!(trace.apply(cursor.append_update(new.to_vec(), false)));

        assert_eq!(trace.text().as_bytes(), log.as_slice());
        assert_eq!(trace.cursor().read_bytes(), 5000);
    }

    #[test]
    fn whole_log_sent_for_a_resumed_read_does_not_line_up() {
        let log = log_of(5000);
        let trace = JobTrace::from_read(&log[..3000], false);
        let cursor = trace.cursor();

        assert_eq!(cursor.new_bytes_in_resumed_read(&log), None);
        assert_eq!(cursor.new_bytes_in_full_read(&log), Some(&log[3000..]));
    }

    #[test]
    fn full_read_of_a_rewritten_log_does_not_line_up() {
        let trace = JobTrace::from_read(&log_of(3000), false);
        let rewritten = vec![b'z'; 5000];

        assert_eq!(trace.cursor().new_bytes_in_full_read(&rewritten), None);
        assert_eq!(trace.cursor().new_bytes_in_full_read(b"short"), None);
    }

    #[test]
    fn first_read_of_an_empty_log_lines_up_with_anything() {
        let cursor = TraceCursor::default();
        assert_eq!(cursor.resume_offset(), 0);
        assert_eq!(cursor.new_bytes_in_resumed_read(b"abc"), Some(&b"abc"[..]));
        assert_eq!(cursor.new_bytes_in_full_read(b"abc"), Some(&b"abc"[..]));
    }

    #[test]
    fn character_split_between_reads_is_decoded_once_complete() {
        let log = "build ✓ done".as_bytes();
        let split = log.iter().position(|&b| b == 0xE2).unwrap() + 1;
        let mut trace = JobTrace::from_read(&log[..split], false);
        assert_eq!(trace.text(), "build ");

        let update = trace.cursor().append_update(log[split..].to_vec(), false);
        assert!(trace.apply(update));
        assert_eq!(trace.text(), "build ✓ done");
        assert_eq!(trace.cursor().read_bytes(), log.len());
    }

    #[test]
    fn invalid_bytes_are_replaced_and_still_counted_as_read() {
        let trace = JobTrace::from_read(b"ok\xFFok", false);
        assert_eq!(trace.text(), "ok\u{FFFD}ok");
        assert_eq!(trace.cursor().read_bytes(), 5);
    }

    #[test]
    fn bytes_past_the_limit_are_dropped_and_mark_the_trace_truncated() {
        let mut trace = JobTrace::from_read(&log_of(JOB_TRACE_MAX_BYTES - 10), false);
        assert!(!trace.is_truncated());

        let update = trace.cursor().append_update(log_of(25), false);
        assert!(trace.apply(update));

        assert!(trace.is_truncated());
        assert_eq!(trace.cursor().read_bytes(), JOB_TRACE_MAX_BYTES);
        assert_eq!(trace.cursor().remaining_bytes(), 0);
        assert_eq!(trace.text().len(), JOB_TRACE_MAX_BYTES);
    }

    #[test]
    fn update_read_from_another_position_is_ignored() {
        let mut trace = JobTrace::from_read(b"first", false);
        let stale = TraceCursor::default().append_update(b"other".to_vec(), false);

        assert!(!trace.apply(stale));
        assert_eq!(trace.text(), "first");
    }

    #[test]
    fn restart_replaces_the_trace() {
        let mut trace = JobTrace::from_read(b"erased log", false);
        let fresh = JobTrace::from_read(b"new", false);
        let update = trace.cursor().restart_update(fresh.clone());

        assert!(trace.apply(update));
        assert_eq!(trace, fresh);
    }

    const FAILING_LOG: &str = "\
$ cargo test
test result: ok. 12 passed; 0 failed
errors: 0, allow_failure: true
\u{1b}[0;31merror[E0308]\u{1b}[0m: mismatched types
compiling more
##[error]Process completed with exit code 1.
";

    #[test]
    fn next_failure_line_skips_lines_that_only_mention_failure_counts() {
        let trace = JobTrace::from_read(FAILING_LOG.as_bytes(), false);
        assert_eq!(trace.next_failure_line(None), Some(3));
    }

    #[test]
    fn next_failure_line_moves_past_the_current_line_and_wraps_around() {
        let trace = JobTrace::from_read(FAILING_LOG.as_bytes(), false);
        assert_eq!(trace.next_failure_line(Some(3)), Some(5));
        assert_eq!(trace.next_failure_line(Some(5)), Some(3));
    }

    #[test]
    fn next_failure_line_includes_the_first_line_when_starting_from_the_top() {
        let trace = JobTrace::from_read(b"fatal: not a git repository\nok\n", false);
        assert_eq!(trace.next_failure_line(None), Some(0));
        assert_eq!(trace.next_failure_line(Some(0)), Some(0));
    }

    #[test]
    fn next_failure_line_is_none_for_a_clean_log() {
        let trace = JobTrace::from_read(b"$ make\nall good\n", false);
        assert_eq!(trace.next_failure_line(None), None);
    }

    #[test]
    fn failure_words_inside_names_and_paths_do_not_match() {
        for line in [
            "   Compiling proc-macro-error v1.0.4",
            "   Compiling quick-error v2.0.1",
            "PASS src/components/Error.test.tsx",
            "  --> src/error.rs:10:5",
        ] {
            assert!(!is_failure_line(line), "{line}");
        }
    }

    #[test]
    fn failure_word_ending_a_sentence_matches() {
        assert!(is_failure_line(
            "error: could not compile `app` due to previous error."
        ));
        assert!(is_failure_line("Build failed."));
    }

    #[test]
    fn test_failures_and_nonzero_failure_counts_match() {
        for line in [
            "--- FAIL: TestParse (0.00s)",
            "FAIL\texample.com/pkg\t0.012s",
            "test result: FAILED. 0 passed; 1 failed; 0 ignored",
            "Failed!  - Failed:     1, Passed:    10, Skipped:     0, Total:    11",
            "host1 : ok=3 changed=0 unreachable=0 failed=1 skipped=0",
            "Error: 0.1.2 is not a published version",
        ] {
            assert!(is_failure_line(line), "{line}");
        }
    }

    #[test]
    fn zero_failure_counts_do_not_match() {
        for line in [
            "Passed!  - Failed:     0, Passed:    11, Skipped:     0, Total:    11",
            "Test Count: 11, Passed: 11, Failed: 0, Warnings: 0",
            "host1 : ok=3 changed=0 unreachable=0 failed=0 skipped=0",
            "test result: ok. 12 passed; 0 failed; 0 ignored",
            "# fail 0",
            "fail-fast: false",
        ] {
            assert!(!is_failure_line(line), "{line}");
        }
    }

    #[test]
    fn github_step_names_are_not_searched() {
        let step = "build\tUpload logs on failure\t2024-05-14T11:19:20.0000000Z";
        assert!(!is_failure_line(&format!("{step} Uploading 3 files")));
        assert!(is_failure_line(&format!("{step} Error: no such file")));
    }
}
