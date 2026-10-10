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
}
