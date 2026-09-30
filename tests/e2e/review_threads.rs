//! End-to-end coverage for the diff view's review threads overlay (`T`).
//!
//! The mock `glab` serves `tests/fixtures/mrs.json` for `mr list`,
//! `mr_diff.txt` for `mr diff` and `mr_notes.json` for `mr note list`. The
//! notes hold one anchored thread with a reply, one general comment, one
//! comment whose line is gone from the diff, and a system note that must never
//! be listed.

use crate::TestSession;
use std::time::{Duration, Instant};

const MR_TITLE: &str = "Feature: Add pagination support";
/// Diff row of the anchored thread in `mr_diff.txt`.
const ANCHORED_CODE: &str = "anchored_call();";
/// Marker the diff pane draws in front of the cursor row.
const CURSOR_MARKER: &str = "❯";

/// Launch, switch to the MRs tab and open the fixture MR's diff.
fn session_in_diff_view() -> TestSession {
    let mut session = TestSession::new(false, 40, 140);
    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app should reach the Issues tab");
    session.send_input(b"l");
    session
        .wait_for_screen_contains(MR_TITLE, 15000)
        .expect("MRs tab should list the fixture MR");
    session.send_input(b"D");
    session
        .wait_for_screen_contains("Merge Request Diff #2", 15000)
        .expect("D should open the MR diff");
    session
}

/// Feeds pending output to the emulator until a screen row satisfies
/// `matches`, returning that row.
fn wait_for_row(
    session: &mut TestSession,
    matches: impl Fn(&str) -> bool,
    timeout_ms: u64,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let bytes = session.pty.read_output();
        if !bytes.is_empty() {
            session.emulator.write_bytes(&bytes);
        }
        let screen = session.emulator.get_text();
        if let Some(row) = screen.lines().find(|row| matches(row)) {
            return Ok(row.to_string());
        }
        if Instant::now() >= deadline {
            return Err(format!("no matching row. Current screen:\n{screen}"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Sends keys one at a time: the app reads a burst of bytes as one sequence.
fn press_keys(session: &TestSession, keys: &[u8]) {
    for key in keys {
        session.send_input(&[*key]);
        std::thread::sleep(Duration::from_millis(150));
    }
}

#[test]
fn test_review_threads_list_general_and_outdated_threads() {
    let mut session = session_in_diff_view();

    session.send_input(b"T");

    session
        .wait_for_screen_contains("Review Threads — 2 unresolved / 3 total", 5000)
        .expect("system notes are dropped; the other three notes form three threads");
    for expected in [
        "GENERAL",
        "General remark without a line.",
        "OUTDATED",
        "src/lib.rs:90",
        "src/lib.rs:3",
        "+1 reply",
    ] {
        session
            .wait_for_screen_contains(expected, 2000)
            .unwrap_or_else(|e| panic!("overlay should show {expected:?}: {e}"));
    }
    assert!(
        !session.emulator.get_text().contains("added 1 commit"),
        "system notes must not be listed"
    );
}

#[test]
fn test_review_threads_jump_reaches_a_reviewed_and_hidden_file() {
    let mut session = session_in_diff_view();

    // The tree opens focused on `docs/`; move to `src/`, mark it reviewed
    // (which folds it) and hide reviewed files, so `src/lib.rs` has no tree row.
    press_keys(&session, b"jjmM");
    session
        .wait_for_screen_contains("readme.md", 5000)
        .expect("docs/ stays in the tree");

    session.send_input(b"T");
    session
        .wait_for_screen_contains("Review Threads", 5000)
        .expect("T should open the overlay");
    // Threads are oldest first, so the anchored one is already selected.
    session.send_input(b"\r");

    let row = wait_for_row(
        &mut session,
        |row| row.contains(CURSOR_MARKER) && row.contains(ANCHORED_CODE),
        5000,
    )
    .expect("Enter should land the diff cursor on the anchored line");
    assert!(
        row.contains(" 3 "),
        "cursor row should be new line 3: {row}"
    );
    assert!(
        !session.emulator.get_text().contains("Review Threads"),
        "a successful jump closes the overlay"
    );
}
