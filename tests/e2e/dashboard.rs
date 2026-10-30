//! The Dashboard tab summarizes the lists the other tabs fetch. The GitLab
//! mock serves two open unassigned issues opened by `test-user`, one open MR
//! by `test-user` reviewed by `reviewer-user`, one successful pipeline, two
//! active milestones and two releases. It reports no signed-in user, so the
//! Dashboard starts on everyone's work.
//!
//! Rows, top to bottom: seven MR queues, five issue queues, the
//! contributors (everyone, you, `@reviewer-user`, `@test-user`), then the
//! Pipelines, Milestones and Releases cards. `k` from the first row wraps to
//! the last.
//!
//! The activity feed (`events.json`) holds a push by `test-user` 2 hours
//! ago, a diff comment by `reviewer-user` 30 hours ago, a comment by
//! `test-user` 50 hours ago and an approval by `reviewer-user` 100 hours
//! ago, timed relative to now by the mock.

use crate::TestSession;

const ASCII_ICONS: &str = "icons = \"ascii\"\n";

fn dashboard_session() -> TestSession {
    let mut session = TestSession::with_config_and_args(
        false,
        40,
        150,
        Some(ASCII_ICONS),
        &["--tab", "dashboard"],
    );
    for call in [
        "issue list",
        "mr list",
        "/pipelines?per_page",
        "milestone list",
        "release list",
    ] {
        session
            .wait_for_cli_call(call, 15000)
            .unwrap_or_else(|e| panic!("the dashboard fetches `{call}`: {e}"));
    }
    session
        .wait_for_screen_contains("@reviewer-user", 15000)
        .expect("the contributors card lists the MR's reviewer");
    session
}

fn screen_line_with(session: &TestSession, needle: &str) -> String {
    session
        .emulator
        .get_text()
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_default()
        .to_string()
}

#[test]
fn a_queue_lists_its_items_and_enter_opens_one() {
    let mut session = dashboard_session();
    session
        .wait_for_screen_contains("Open · 1 ", 5000)
        .expect("the first queue, everyone's open MRs, is listed below the cards");
    assert!(
        screen_line_with(&session, "waiting for review").contains(" 1 "),
        "the MR without approval waits for review: {}",
        session.emulator.get_text()
    );
    assert!(
        screen_line_with(&session, "unassigned").contains(" 2 "),
        "both issues are unassigned: {}",
        session.emulator.get_text()
    );

    session.press_keys(b"\r");
    session
        .wait_for_screen_contains("Enter open · Esc back", 5000)
        .expect("Enter moves the keys into the list");
    session.press_keys(b"\r");
    session
        .wait_for_screen_contains("┌ ! MRs ", 5000)
        .expect("Enter on a listed MR opens the MRs tab");
    session
        .wait_for_screen_contains("Preview: MR !2 ", 5000)
        .expect("the MR is selected with its preview open");
}

#[test]
fn choosing_a_contributor_scopes_the_cards_and_lists_their_activity() {
    let mut session = dashboard_session();
    session
        .wait_for_cli_call("/events?after=", 15000)
        .expect("the dashboard reads the project's activity feed");
    session.press_keys(b"kkkk\r");
    session
        .wait_for_screen_contains("Activity of @test-user, last 30 days · 5 ", 5000)
        .expect("Enter on a contributor lists their MRs, issues, commits and comments");
    session
        .wait_for_screen_contains("Paginate the issue list (+1 more)", 5000)
        .expect("their push is listed with its newest commit");
    session
        .wait_for_screen_contains("Picking this up · on Worktree", 5000)
        .expect("their comment is listed with the issue it is on");
    session
        .wait_for_screen_contains("MRs · @test-user", 5000)
        .expect("the MR card counts the contributor's work");
    assert!(
        screen_line_with(&session, "open, involving @test-user").contains(" 1 "),
        "test-user authored the one open MR: {}",
        session.emulator.get_text()
    );
    assert!(
        screen_line_with(&session, "opened by @test-user, unassigned").contains(" 2 "),
        "test-user opened both unassigned issues: {}",
        session.emulator.get_text()
    );

    // Newest first: the push, then the comment on issue #7.
    session.press_keys(b"j\r");
    session
        .wait_for_screen_contains("┌ # Issues ", 5000)
        .expect("Enter on a comment opens the issue it is on");
}

#[test]
fn esc_hands_the_keys_back_from_a_contributors_activity() {
    let mut session = dashboard_session();
    session.press_keys(b"kkkk\r");
    session
        .wait_for_screen_contains("Enter open · Esc back", 5000)
        .expect("Enter on a contributor moves the keys into the list");
    session.press_keys(b"\x1b");
    session
        .wait_for_screen_contains("Enter browse", 5000)
        .expect("Esc hands the keys back to the rows");
}

#[test]
fn the_window_key_counts_each_contributors_activity_in_the_last_day_and_week() {
    let mut session = dashboard_session();
    session
        .wait_for_cli_call("/events?after=", 15000)
        .expect("the dashboard reads the project's activity feed");

    session.press_keys(b"w");
    session
        .wait_for_screen_contains("Contributors · last 24h", 5000)
        .expect("w switches the card to the last day of activity");
    session.settle(300);
    assert!(
        screen_line_with(&session, "@test-user").contains("    1    0    0    0"),
        "only the push of 2 hours ago is inside the last day: {}",
        session.emulator.get_text()
    );

    session.press_keys(b"w");
    session
        .wait_for_screen_contains("Contributors · last 7 days", 5000)
        .expect("w again widens the window to a week");
    session.settle(300);
    assert!(
        screen_line_with(&session, "@test-user").contains("    1    1    0    0"),
        "the week adds test-user's comment: {}",
        session.emulator.get_text()
    );
    assert!(
        screen_line_with(&session, "@reviewer-user").contains("    0    0    1    1"),
        "and reviewer-user's diff comment and approval: {}",
        session.emulator.get_text()
    );
}
