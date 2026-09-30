use crate::domain::mr::{DiscussionNote, NotePosition};
use std::collections::HashMap;

/// Where a review thread points to, relative to the diff currently loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadAnchor {
    /// A plain MR/PR comment with no file position.
    General,
    /// The anchor line is present in the loaded diff.
    InDiff {
        file_path: String,
        line: Option<u64>,
    },
    /// The note has a file position, but that line is no longer in the diff
    /// (force-push, rebase, or a position without a line).
    Outdated {
        file_path: String,
        line: Option<u64>,
    },
}

impl ThreadAnchor {
    fn classify(
        position: Option<&NotePosition>,
        is_in_diff: &impl Fn(&NotePosition) -> bool,
    ) -> Self {
        let Some(position) = position else {
            return Self::General;
        };
        let Some(file_path) = position
            .new_path
            .clone()
            .or_else(|| position.old_path.clone())
        else {
            return Self::General;
        };
        let line = position.new_line.or(position.old_line);
        if is_in_diff(position) {
            Self::InDiff { file_path, line }
        } else {
            Self::Outdated { file_path, line }
        }
    }
}

/// One discussion: the root note followed by its replies, oldest first.
#[derive(Debug, Clone)]
pub struct ReviewThread {
    /// Never empty; the first note is the one that opened the thread.
    pub notes: Vec<DiscussionNote>,
    pub anchor: ThreadAnchor,
}

impl ReviewThread {
    pub fn root(&self) -> &DiscussionNote {
        &self.notes[0]
    }

    pub fn reply_count(&self) -> usize {
        self.notes.len() - 1
    }

    pub fn is_resolvable(&self) -> bool {
        self.notes.iter().any(|n| n.resolvable.unwrap_or(false))
    }

    /// Same rule as the diff header's unresolved counter: a thread stays open
    /// while any of its resolvable notes is unresolved.
    pub fn is_unresolved(&self) -> bool {
        self.notes
            .iter()
            .any(|n| n.resolvable.unwrap_or(false) && !n.resolved.unwrap_or(false))
    }
}

/// Groups non-system notes into threads by discussion id, oldest thread first.
/// `is_in_diff` decides whether a thread's anchor still exists in the diff.
pub fn group_threads(
    notes: &[DiscussionNote],
    is_in_diff: impl Fn(&NotePosition) -> bool,
) -> Vec<ReviewThread> {
    let mut thread_index: HashMap<String, usize> = HashMap::new();
    let mut grouped: Vec<Vec<DiscussionNote>> = Vec::new();

    for note in notes.iter().filter(|n| !n.system) {
        let key = note
            .discussion_id
            .clone()
            .unwrap_or_else(|| note.id.to_string());
        let idx = *thread_index.entry(key).or_insert_with(|| {
            grouped.push(Vec::new());
            grouped.len() - 1
        });
        grouped[idx].push(note.clone());
    }

    let mut threads: Vec<ReviewThread> = grouped
        .into_iter()
        .map(|mut thread_notes| {
            thread_notes.sort_by(|a, b| a.created_at.cmp(&b.created_at));
            let anchor = ThreadAnchor::classify(thread_notes[0].position.as_ref(), &is_in_diff);
            ReviewThread {
                notes: thread_notes,
                anchor,
            }
        })
        .collect();
    threads.sort_by(|a, b| a.root().created_at.cmp(&b.root().created_at));
    threads
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::mr::Author;

    fn note(id: u64, discussion: &str, created_at: &str) -> DiscussionNote {
        DiscussionNote {
            id,
            body: format!("note {id}"),
            author: Author {
                username: "alice".to_string(),
            },
            created_at: created_at.to_string(),
            system: false,
            position: None,
            discussion_id: Some(discussion.to_string()),
            resolved: None,
            resolvable: None,
        }
    }

    fn at_line(mut n: DiscussionNote, path: &str, line: u64) -> DiscussionNote {
        n.position = Some(NotePosition {
            new_path: Some(path.to_string()),
            old_path: Some(path.to_string()),
            new_line: Some(line),
            old_line: None,
            start_line: None,
            line_range: None,
        });
        n
    }

    fn resolvable(mut n: DiscussionNote, resolved: bool) -> DiscussionNote {
        n.resolvable = Some(true);
        n.resolved = Some(resolved);
        n
    }

    #[test]
    fn groups_replies_under_the_oldest_note_and_orders_threads_by_root() {
        let notes = vec![
            note(3, "b", "2026-01-03T00:00:00Z"),
            note(2, "a", "2026-01-02T00:00:00Z"),
            note(1, "a", "2026-01-01T00:00:00Z"),
        ];
        let threads = group_threads(&notes, |_| true);

        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].root().id, 1);
        assert_eq!(threads[0].reply_count(), 1);
        assert_eq!(threads[1].root().id, 3);
    }

    #[test]
    fn system_notes_are_dropped_and_missing_discussion_ids_stay_separate() {
        let mut system = note(1, "a", "2026-01-01T00:00:00Z");
        system.system = true;
        let mut lone_a = note(2, "", "2026-01-02T00:00:00Z");
        lone_a.discussion_id = None;
        let mut lone_b = note(3, "", "2026-01-03T00:00:00Z");
        lone_b.discussion_id = None;

        let threads = group_threads(&[system, lone_a, lone_b], |_| true);

        assert_eq!(threads.len(), 2);
        assert!(threads.iter().all(|t| t.reply_count() == 0));
    }

    #[test]
    fn anchor_is_general_in_diff_or_outdated() {
        let notes = vec![
            note(1, "general", "2026-01-01T00:00:00Z"),
            at_line(note(2, "live", "2026-01-02T00:00:00Z"), "src/a.rs", 10),
            at_line(note(3, "gone", "2026-01-03T00:00:00Z"), "src/a.rs", 99),
        ];
        let threads = group_threads(&notes, |p| p.new_line == Some(10));

        assert_eq!(threads[0].anchor, ThreadAnchor::General);
        assert_eq!(
            threads[1].anchor,
            ThreadAnchor::InDiff {
                file_path: "src/a.rs".to_string(),
                line: Some(10)
            }
        );
        assert_eq!(
            threads[2].anchor,
            ThreadAnchor::Outdated {
                file_path: "src/a.rs".to_string(),
                line: Some(99)
            }
        );
    }

    #[test]
    fn thread_is_unresolved_while_any_resolvable_note_is_open() {
        let open = group_threads(
            &[
                resolvable(note(1, "t", "2026-01-01T00:00:00Z"), true),
                resolvable(note(2, "t", "2026-01-02T00:00:00Z"), false),
            ],
            |_| true,
        );
        assert!(open[0].is_unresolved());

        let closed = group_threads(
            &[resolvable(note(1, "t", "2026-01-01T00:00:00Z"), true)],
            |_| true,
        );
        assert!(closed[0].is_resolvable());
        assert!(!closed[0].is_unresolved());

        let plain = group_threads(&[note(1, "t", "2026-01-01T00:00:00Z")], |_| true);
        assert!(!plain[0].is_resolvable());
        assert!(!plain[0].is_unresolved());
    }
}
