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

/// A change of a viewed-state batch GitHub did not apply, and why.
/// GraphQL runs aliased mutations independently, so one refused path leaves
/// the rest of its batch applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedViewedPath {
    pub path: String,
    pub reason: String,
}

/// Digest of a file's diff text, persisted next to each reviewed mark.
///
/// FNV-1a rather than `std::hash::DefaultHasher`: the value outlives the
/// process, and the standard hasher's algorithm may change between Rust
/// releases, which would silently reset every mark after an upgrade.
#[derive(Clone, Copy, Debug)]
pub struct Fingerprint(u64);

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

impl Default for Fingerprint {
    fn default() -> Self {
        Self(FNV_OFFSET_BASIS)
    }
}

impl Fingerprint {
    pub fn update(&mut self, line: &str) {
        for byte in line.bytes().chain(std::iter::once(b'\n')) {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }

    pub fn finish(self) -> String {
        format!("{:016x}", self.0)
    }
}

/// Files per "Viewed" mutation; every path is a `gh` argument, and Windows
/// caps a command line at 32 KiB.
pub const VIEWED_SYNC_BATCH: usize = 50;

/// Pushes local reviewed-mark changes to GitHub's "Viewed" state.
///
/// Toggles are queued and flushed after a short debounce, so rapid toggles of
/// one file collapse into its last state, and a file toggled back to what the
/// server already holds sends nothing. One lives per pull request for the
/// whole session, so closing the diff loses neither queued intents nor the
/// request in flight.
#[derive(Clone, Debug)]
pub struct ViewedSync {
    pull_request_id: String,
    /// Paths the server is known to hold as viewed.
    confirmed: HashSet<String>,
    /// Latest unsent intent per path.
    pending: HashMap<String, bool>,
    flush_scheduled: bool,
    /// The batch sent and not settled yet. Requests go out one at a time: two
    /// in flight could reach GitHub in either order, and an intent queued
    /// meanwhile can only be told apart from a no-op once the request ahead
    /// of it has landed.
    in_flight: Vec<(String, bool)>,
}

impl ViewedSync {
    pub fn new(pull_request_id: String, confirmed: HashSet<String>) -> Self {
        Self {
            pull_request_id,
            confirmed,
            pending: HashMap::new(),
            flush_scheduled: false,
            in_flight: Vec::new(),
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

    /// Drains up to `VIEWED_SYNC_BATCH` queued intents that differ from the
    /// server state, sorted. Drains nothing while a request is in flight: the
    /// caller flushes again once it settles (`claim_follow_up_flush`).
    pub fn take_changes(&mut self) -> Vec<(String, bool)> {
        self.flush_scheduled = false;
        if !self.in_flight.is_empty() {
            return Vec::new();
        }
        let confirmed = &self.confirmed;
        self.pending
            .retain(|path, viewed| confirmed.contains(path) != *viewed);
        let mut batch: Vec<(&String, bool)> = self
            .pending
            .iter()
            .map(|(path, viewed)| (path, *viewed))
            .collect();
        batch.sort();
        batch.truncate(VIEWED_SYNC_BATCH);
        let changes: Vec<(String, bool)> = batch
            .into_iter()
            .map(|(path, viewed)| (path.clone(), viewed))
            .collect();
        for (path, _) in &changes {
            self.pending.remove(path);
        }
        self.in_flight.clone_from(&changes);
        changes
    }

    /// Settles a finished request and returns the local state each of its
    /// paths must show: the server's. GitHub applied every change but the
    /// `rejected` paths. Paths toggled again since are left out; their newer
    /// intent is still on its way.
    pub fn settle(
        &mut self,
        changes: &[(String, bool)],
        rejected: &HashSet<&str>,
    ) -> Vec<(String, bool)> {
        self.in_flight.clear();
        changes
            .iter()
            .filter_map(|(path, viewed)| {
                if !rejected.contains(path.as_str()) {
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

    /// Returns true when intents left over by the settled request (queued
    /// while it was in flight, or past its batch) need a flush the caller
    /// must schedule. A flush already scheduled picks them up.
    pub fn claim_follow_up_flush(&mut self) -> bool {
        !self.pending.is_empty() && !std::mem::replace(&mut self.flush_scheduled, true)
    }

    /// Replaces the server state with a freshly fetched one; queued intents
    /// and the request in flight are kept.
    pub fn reload(&mut self, confirmed: HashSet<String>) {
        self.confirmed = confirmed;
    }

    /// The state the user asked for and the server may not show yet: the
    /// batch in flight, overridden by intents queued since, sorted.
    pub fn unsettled(&self) -> Vec<(String, bool)> {
        let mut intents: HashMap<&str, bool> = self
            .in_flight
            .iter()
            .map(|(path, viewed)| (path.as_str(), *viewed))
            .collect();
        intents.extend(
            self.pending
                .iter()
                .map(|(path, viewed)| (path.as_str(), *viewed)),
        );
        let mut unsettled: Vec<(String, bool)> = intents
            .into_iter()
            .map(|(path, viewed)| (path.to_string(), viewed))
            .collect();
        unsettled.sort();
        unsettled
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

    fn applied() -> HashSet<&'static str> {
        HashSet::new()
    }

    fn every_path(changes: &[(String, bool)]) -> HashSet<&str> {
        changes.iter().map(|(path, _)| path.as_str()).collect()
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
        assert_eq!(digest(&[""]), "af63c74c8601c8dd");
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

        let corrections = sync.settle(&changes, &every_path(&changes));
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
            sync.settle(&changes, &applied()),
            vec![("a.rs".to_string(), true)]
        );
        // The server now holds it, so toggling it twice sends nothing.
        sync.queue(&paths(&["a.rs"]), false);
        sync.queue(&paths(&["a.rs"]), true);
        assert!(sync.take_changes().is_empty());
    }

    #[test]
    fn an_intent_queued_during_a_request_is_sent_once_it_lands() {
        let mut sync = sync_with_viewed(&[]);
        sync.queue(&paths(&["a.rs"]), true);
        let marked = sync.take_changes();
        assert_eq!(marked, vec![("a.rs".to_string(), true)]);
        // Unmarked while the mark is on its way: against the server state
        // before the mark this is a no-op, against the one after it is not.
        assert!(sync.queue(&paths(&["a.rs"]), false));
        assert!(sync.take_changes().is_empty());

        assert!(sync.settle(&marked, &applied()).is_empty());
        assert!(sync.claim_follow_up_flush());
        let unmarked = sync.take_changes();
        assert_eq!(unmarked, vec![("a.rs".to_string(), false)]);
        assert_eq!(
            sync.settle(&unmarked, &applied()),
            vec![("a.rs".to_string(), false)]
        );
        assert!(!sync.claim_follow_up_flush());
    }

    #[test]
    fn a_large_toggle_goes_out_one_batch_at_a_time() {
        let names: Vec<String> = (0..=VIEWED_SYNC_BATCH)
            .map(|i| format!("f{i:03}.rs"))
            .collect();
        let mut sync = sync_with_viewed(&[]);
        sync.queue(&names, true);

        let first = sync.take_changes();
        let expected: Vec<(String, bool)> = names[..VIEWED_SYNC_BATCH]
            .iter()
            .map(|name| (name.clone(), true))
            .collect();
        assert_eq!(first, expected);
        assert!(sync.take_changes().is_empty(), "one request at a time");

        sync.settle(&first, &applied());
        assert!(sync.claim_follow_up_flush());
        assert_eq!(
            sync.take_changes(),
            vec![(names[VIEWED_SYNC_BATCH].clone(), true)]
        );
    }

    #[test]
    fn unsettled_is_the_batch_in_flight_overridden_by_newer_intents() {
        let mut sync = sync_with_viewed(&[]);
        sync.queue(&paths(&["a.rs", "b.rs"]), true);
        sync.take_changes();
        sync.queue(&paths(&["b.rs", "c.rs"]), false);
        assert_eq!(
            sync.unsettled(),
            vec![
                ("a.rs".to_string(), true),
                ("b.rs".to_string(), false),
                ("c.rs".to_string(), false),
            ]
        );
    }

    #[test]
    fn a_reload_keeps_the_request_in_flight_and_judges_intents_against_the_new_state() {
        let mut sync = sync_with_viewed(&[]);
        sync.queue(&paths(&["a.rs"]), true);
        let in_flight = sync.take_changes();

        // Viewed in the browser meanwhile: unmarking it is a real change now.
        sync.reload(HashSet::from(["b.rs".to_string()]));
        sync.queue(&paths(&["b.rs"]), false);
        assert!(sync.take_changes().is_empty(), "one request at a time");

        sync.settle(&in_flight, &applied());
        assert!(sync.claim_follow_up_flush());
        assert_eq!(sync.take_changes(), vec![("b.rs".to_string(), false)]);
    }

    #[test]
    fn a_partly_rejected_batch_confirms_what_github_applied_and_reverts_the_rest() {
        let mut sync = sync_with_viewed(&["old.rs"]);
        sync.queue(&paths(&["a.rs", "gone.rs"]), true);
        sync.queue(&paths(&["old.rs"]), false);
        let changes = sync.take_changes();

        let corrections = sync.settle(&changes, &HashSet::from(["gone.rs"]));
        assert_eq!(
            corrections,
            vec![
                ("a.rs".to_string(), true),
                ("gone.rs".to_string(), false),
                ("old.rs".to_string(), false),
            ]
        );
        // a.rs is confirmed now: toggling it back and forth sends nothing.
        sync.queue(&paths(&["a.rs"]), false);
        sync.queue(&paths(&["a.rs"]), true);
        assert!(sync.take_changes().is_empty());
    }
}
