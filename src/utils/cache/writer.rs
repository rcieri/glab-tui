//! Writes project caches to disk on a thread of their own. Every fetch saves
//! the whole cache, so saves to one file within `COALESCE_WINDOW` of each other
//! become one write. Until its write is done, a save is what `pending` returns,
//! so a load never reads a file older than the last save.

use super::schema::ProjectCache;
use crate::utils::private_fs;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How long the first save to a file waits for more saves to that file
/// before it is written.
const COALESCE_WINDOW: Duration = Duration::from_millis(500);

struct PendingSave {
    snapshot: Arc<ProjectCache>,
    /// The first save since the file was last written.
    queued_at: Instant,
}

struct Queue {
    saves: BTreeMap<PathBuf, PendingSave>,
    is_flushing: bool,
}

static QUEUE: Mutex<Queue> = Mutex::new(Queue {
    saves: BTreeMap::new(),
    is_flushing: false,
});
static QUEUE_CHANGED: Condvar = Condvar::new();
static IS_WRITER_RUNNING: LazyLock<bool> = LazyLock::new(|| {
    std::thread::Builder::new()
        .name("glab-tui-cache-writer".to_string())
        .spawn(write_queued_saves)
        .is_ok()
});

fn lock_queue() -> MutexGuard<'static, Queue> {
    QUEUE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Queues `cache` to be written to `path`, replacing a save to the same path
/// that is not written yet. Writes at once when no writer thread could start.
pub(super) fn enqueue(path: PathBuf, cache: ProjectCache) {
    if !*IS_WRITER_RUNNING {
        write(&path, &cache);
        return;
    }
    let snapshot = Arc::new(cache);
    let mut queue = lock_queue();
    match queue.saves.get_mut(&path) {
        Some(pending) => pending.snapshot = snapshot,
        None => {
            queue.saves.insert(
                path,
                PendingSave {
                    snapshot,
                    queued_at: Instant::now(),
                },
            );
        }
    }
    QUEUE_CHANGED.notify_all();
}

/// The last save to `path` that is not on disk yet.
pub(super) fn pending(path: &Path) -> Option<Arc<ProjectCache>> {
    lock_queue()
        .saves
        .get(path)
        .map(|pending| Arc::clone(&pending.snapshot))
}

/// Writes every queued save now and returns once they are on disk.
pub fn flush() {
    let mut queue = lock_queue();
    if queue.saves.is_empty() {
        return;
    }
    queue.is_flushing = true;
    QUEUE_CHANGED.notify_all();
    while !queue.saves.is_empty() {
        queue = QUEUE_CHANGED
            .wait(queue)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
}

fn write_queued_saves() {
    let mut queue = lock_queue();
    loop {
        let Some(first_queued) = queue.saves.values().map(|pending| pending.queued_at).min() else {
            queue.is_flushing = false;
            queue = QUEUE_CHANGED
                .wait(queue)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            continue;
        };
        let wait = (first_queued + COALESCE_WINDOW).saturating_duration_since(Instant::now());
        if !queue.is_flushing && !wait.is_zero() {
            queue = QUEUE_CHANGED
                .wait_timeout(queue, wait)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
            continue;
        }
        let is_flushing = queue.is_flushing;
        let now = Instant::now();
        let due: Vec<(PathBuf, Arc<ProjectCache>)> = queue
            .saves
            .iter()
            .filter(|(_, pending)| is_flushing || pending.queued_at + COALESCE_WINDOW <= now)
            .map(|(path, pending)| (path.clone(), Arc::clone(&pending.snapshot)))
            .collect();
        drop(queue);

        for (path, snapshot) in &due {
            write(path, snapshot);
        }

        queue = lock_queue();
        for (path, snapshot) in due {
            let is_unchanged = queue
                .saves
                .get(&path)
                .is_some_and(|pending| Arc::ptr_eq(&pending.snapshot, &snapshot));
            if is_unchanged {
                queue.saves.remove(&path);
            }
        }
        QUEUE_CHANGED.notify_all();
    }
}

fn write(path: &Path, cache: &ProjectCache) {
    if let Ok(json) = cache.to_json() {
        let _ = private_fs::write_atomic(path, json);
    }
}
