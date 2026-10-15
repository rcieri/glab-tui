use serde::Deserialize;
use std::collections::{HashMap, HashSet};

/// The viewer's GitHub "Viewed" checkbox state for one file of a pull request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FileViewedState {
    Viewed,
    Unviewed,
    /// Viewed once, but the file has new changes since.
    Dismissed,
}

/// Server-side viewed state of a pull request's files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrViewedFiles {
    /// GraphQL node ID, the handle the viewed-state mutations take.
    pub pull_request_id: String,
    pub files: HashMap<String, FileViewedState>,
}

/// Digest of a file's diff text, persisted next to each reviewed mark.
///
/// FNV-1a rather than `std::hash::DefaultHasher`: the value outlives the
/// process, and the standard hasher's algorithm may change between Rust
/// releases, which would silently reset every mark after an upgrade.
#[derive(Clone, Copy, Debug)]
pub struct Fingerprint(u64);

impl Default for Fingerprint {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fingerprint {
    pub fn update(&mut self, line: &str) {
        for byte in line.bytes().chain(std::iter::once(b'\n')) {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    pub fn finish(self) -> String {
        format!("{:016x}", self.0)
    }
}

/// Pushes local reviewed-mark changes to GitHub's "Viewed" state.
///
/// Toggles are queued and flushed after a short debounce, so rapid toggles of
/// one file collapse into its last state, and a file toggled back to what the
/// server already holds sends nothing.
#[derive(Clone, Debug)]
pub struct ViewedSync {
    pull_request_id: String,
    /// Paths the server is known to hold as viewed.
    confirmed: HashSet<String>,
    /// Latest unsent intent per path.
    pending: HashMap<String, bool>,
    flush_scheduled: bool,
}

impl ViewedSync {
    pub fn new(pull_request_id: String, confirmed: HashSet<String>) -> Self {
        Self {
            pull_request_id,
            confirmed,
            pending: HashMap::new(),
            flush_scheduled: false,
        }
    }

    pub fn pull_request_id(&self) -> &str {
        &self.pull_request_id
    }

    /// Records the user's latest intent for `paths`. Returns true when the
    /// caller must schedule a flush; one already scheduled picks these up.
    pub fn queue(&mut self, paths: &[String], viewed: bool) -> bool {
        for path in paths {
            self.pending.insert(path.clone(), viewed);
        }
        !std::mem::replace(&mut self.flush_scheduled, true)
    }

    /// Drains the queued intents that differ from the server state, sorted.
    pub fn take_changes(&mut self) -> Vec<(String, bool)> {
        self.flush_scheduled = false;
        let confirmed = &self.confirmed;
        let mut changes: Vec<(String, bool)> = self
            .pending
            .drain()
            .filter(|(path, viewed)| confirmed.contains(path) != *viewed)
            .collect();
        changes.sort();
        changes
    }

    /// Settles a finished request and returns the local state each of its
    /// paths must show: the server's. Paths toggled again since are left out;
    /// their newer intent is still on its way.
    pub fn settle(&mut self, changes: &[(String, bool)], succeeded: bool) -> Vec<(String, bool)> {
        changes
            .iter()
            .filter_map(|(path, viewed)| {
                if succeeded {
                    if *viewed {
                        self.confirmed.insert(path.clone());
                    } else {
                        self.confirmed.remove(path);
                    }
                }
                (!self.pending.contains_key(path))
                    .then(|| (path.clone(), self.confirmed.contains(path)))
            })
            .collect()
    }

    /// Takes over the unsent intents of the sync this one replaces (the diff
    /// was re-fetched before they were flushed) and returns them, so the
    /// caller re-applies them on top of the freshly loaded server state.
    pub fn adopt_pending(&mut self, previous: ViewedSync) -> Vec<(String, bool)> {
        if previous.pull_request_id != self.pull_request_id {
            return Vec::new();
        }
        // The flush event the previous sync scheduled is still queued.
        self.flush_scheduled |= previous.flush_scheduled;
        let carried: Vec<(String, bool)> = previous.pending.into_iter().collect();
        self.pending.extend(carried.iter().cloned());
        carried
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn sync_with_viewed(names: &[&str]) -> ViewedSync {
        ViewedSync::new("PR_1".to_string(), paths(names).into_iter().collect())
    }

    #[test]
    fn fingerprint_depends_on_line_boundaries_and_content() {
        let digest = |lines: &[&str]| {
            let mut fp = Fingerprint::default();
            for line in lines {
                fp.update(line);
            }
            fp.finish()
        };
        assert_eq!(digest(&["+a", "-b"]), digest(&["+a", "-b"]));
        assert_ne!(digest(&["+a", "-b"]), digest(&["+a", "-c"]));
        assert_ne!(digest(&["+ab"]), digest(&["+a", "b"]));
        // Persisted in the cache: the value must not drift between builds.
        assert_eq!(digest(&[]), "cbf29ce484222325");
    }

    #[test]
    fn only_the_first_queue_asks_for_a_flush() {
        let mut sync = sync_with_viewed(&[]);
        assert!(sync.queue(&paths(&["a.rs"]), true));
        assert!(!sync.queue(&paths(&["b.rs"]), true));
        sync.take_changes();
        assert!(sync.queue(&paths(&["a.rs"]), false));
    }

    #[test]
    fn rapid_toggles_collapse_to_the_last_state() {
        let mut sync = sync_with_viewed(&["kept.rs"]);
        sync.queue(&paths(&["a.rs"]), true);
        sync.queue(&paths(&["a.rs"]), false);
        sync.queue(&paths(&["a.rs"]), true);
        // Toggled back to what the server already has: nothing to send.
        sync.queue(&paths(&["kept.rs"]), false);
        sync.queue(&paths(&["kept.rs"]), true);
        assert_eq!(sync.take_changes(), vec![("a.rs".to_string(), true)]);
        assert!(sync.take_changes().is_empty());
    }

    #[test]
    fn failure_reverts_to_the_server_state_unless_toggled_again() {
        let mut sync = sync_with_viewed(&["old.rs"]);
        sync.queue(&paths(&["a.rs", "b.rs", "old.rs"]), true);
        sync.queue(&paths(&["old.rs"]), false);
        let changes = sync.take_changes();
        assert_eq!(
            changes,
            vec![
                ("a.rs".to_string(), true),
                ("b.rs".to_string(), true),
                ("old.rs".to_string(), false)
            ]
        );
        // b.rs is toggled again while the request is in flight.
        sync.queue(&paths(&["b.rs"]), false);

        let corrections = sync.settle(&changes, false);
        assert_eq!(
            corrections,
            vec![("a.rs".to_string(), false), ("old.rs".to_string(), true)]
        );
    }

    #[test]
    fn success_confirms_the_new_state() {
        let mut sync = sync_with_viewed(&[]);
        sync.queue(&paths(&["a.rs"]), true);
        let changes = sync.take_changes();
        assert_eq!(
            sync.settle(&changes, true),
            vec![("a.rs".to_string(), true)]
        );
        // The server now holds it, so toggling it twice sends nothing.
        sync.queue(&paths(&["a.rs"]), false);
        sync.queue(&paths(&["a.rs"]), true);
        assert!(sync.take_changes().is_empty());
    }

    #[test]
    fn a_replacement_adopts_unsent_intents_of_the_same_pull_request() {
        let mut previous = sync_with_viewed(&[]);
        assert!(previous.queue(&paths(&["a.rs"]), true));

        let mut fresh = sync_with_viewed(&[]);
        assert_eq!(
            fresh.adopt_pending(previous),
            vec![("a.rs".to_string(), true)]
        );
        assert!(
            !fresh.queue(&paths(&["b.rs"]), true),
            "the previous flush is still scheduled"
        );
        assert_eq!(fresh.take_changes().len(), 2);

        let mut other = ViewedSync::new("PR_2".to_string(), HashSet::new());
        other.queue(&paths(&["a.rs"]), true);
        let mut fresh = sync_with_viewed(&[]);
        assert!(fresh.adopt_pending(other).is_empty());
    }
}
