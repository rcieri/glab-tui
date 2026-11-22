//! The Dashboard tab summarizes the lists the other tabs fetch. The GitLab
//! mock serves two open unassigned issues opened by `test-user`, one open MR
//! by `test-user` reviewed by `reviewer-user`, one successful pipeline, two
//! active milestones and two releases. It reports no signed-in user, so the
//! Dashboard starts on everyone's work.
//!
//! At 150 columns the cards sit three to a line: MRs, Issues and
//! Contributors, then Pipelines, Milestones and Releases. `Enter` goes from
//! the tab into the cards, from a card into its rows (the contributors are
//! everyone, you, `@reviewer-user` and `@test-user`, so `k` from the first
//! wraps to `@test-user`), and from a row into its item list.
//!
//! The activity feed (`events.json`) holds a push by `test-user` 2 hours
//! ago, a diff comment by `reviewer-user` 30 hours ago, a comment by
//! `test-user` 50 hours ago and an approval by `reviewer-user` 100 hours
//! ago, timed relative to now by the mock.

use crate::TestSession;
use std::time::{Duration, Instant};

const LINE_WAIT: Duration = Duration::from_secs(5);

const CONFIG: &str = "icons = \"ascii\"\n[dashboard]\nenabled = true\n";

fn dashboard_session() -> TestSession {
    let mut session =
        TestSession::with_config_and_args(false, 40, 150, Some(CONFIG), &["--tab", "dashboard"]);
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

fn wait_for_line(session: &mut TestSession, needle: &str, expected: &str, why: &str) {
    let deadline = Instant::now() + LINE_WAIT;
    while !screen_line_with(session, needle).contains(expected) {
        assert!(
            Instant::now() < deadline,
            "{why}: {}",
            session.emulator.get_text()
        );
        session.settle(50);
    }
}

#[test]
fn a_queue_lists_its_items_and_enter_opens_one() {
    let mut session = dashboard_session();
    session
        .wait_for_screen_contains("Open · 1 ", 5000)
        .expect("the first queue, everyone's open MRs, is listed below the cards");
    assert!(
        screen_line_with(&session, "waiting for review").contains("  0 ? waiting for review"),
        "the mock reports no review state, so the MR waits for nobody: {}",
        session.emulator.get_text()
    );
    wait_for_line(
        &mut session,
        "unassigned",
        "  2 @ unassigned",
        "both issues are unassigned",
    );

    session.press_keys(b"\r\r\r");
    session
        .wait_for_screen_contains("Enter open · Esc back", 5000)
        .expect("Enter goes into the cards, the MR queues, then the list");
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
    session.press_keys(b"\rll\rk\r");
    session
        .wait_for_screen_contains("Activity of @test-user, feed of the last 30 days · 5 ", 5000)
        .expect("Enter on a contributor lists their MRs and issues at any age, with the feed's push and comment");
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
fn esc_climbs_back_one_level_at_a_time() {
    let mut session = dashboard_session();
    session.press_keys(b"\rll\rk\r");
    session
        .wait_for_screen_contains("Enter open · Esc back", 5000)
        .expect("Enter on a contributor moves the keys into the list");
    session.press_keys(b"\x1b");
    session
        .wait_for_screen_contains("j/k pick · Enter list · Esc cards", 5000)
        .expect("Esc hands the keys back to the contributors");
    session.press_keys(b"\x1b");
    session
        .wait_for_screen_contains("h/j/k/l move · Enter open card · Esc leave", 5000)
        .expect("Esc again hands them back to the cards");
    session.press_keys(b"\x1b");
    session
        .wait_for_screen_contains("Enter navigate the cards", 5000)
        .expect("and once more to the tab, where h/l switch tabs");
}

#[test]
fn the_pipelines_card_lists_its_runs_and_opens_one_in_its_tab() {
    let mut session = dashboard_session();
    session
        .wait_for_screen_contains("100% succeeded of the last 1", 5000)
        .expect("the pipelines card counts the fetched run before Enter opens its list");
    session.press_keys(b"\rj\r");
    session
        .wait_for_screen_contains("Enter open · Esc back", 5000)
        .expect("Enter on a card without rows goes straight to its list");
    session.press_keys(b"\r");
    session
        .wait_for_screen_contains("┌ > Pipelines ", 5000)
        .expect("Enter on a run opens the Pipelines tab");
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
    wait_for_line(
        &mut session,
        "@test-user",
        "    1    0    0    0",
        "only the push of 2 hours ago is inside the last day",
    );
    wait_for_line(
        &mut session,
        "@reviewer-user",
        "    0    0    0    0",
        "reviewer-user's diff comment and approval are older than a day",
    );

    session.press_keys(b"w");
    session
        .wait_for_screen_contains("Contributors · last 7 days", 5000)
        .expect("w again widens the window to a week");
    wait_for_line(
        &mut session,
        "@test-user",
        "    1    1    0    0",
        "the week adds test-user's comment",
    );
    wait_for_line(
        &mut session,
        "@reviewer-user",
        "    0    0    1    1",
        "and reviewer-user's diff comment and approval",
    );
}

#[test]
fn github_repository_dashboard_summarizes_prs_issues_and_activity() {
    let mut session =
        TestSession::with_config_and_args(true, 40, 150, Some(CONFIG), &["--tab", "dashboard"]);
    for call in [
        "issue list",
        "pr list",
        "actions/runs",
        "/milestones",
        "/releases",
    ] {
        session
            .wait_for_cli_call(call, 15000)
            .unwrap_or_else(|e| panic!("the dashboard fetches `{call}`: {e}"));
    }
    session
        .wait_for_cli_call("/events?", 15000)
        .expect("the dashboard reads the repository's activity feed");
    session
        .wait_for_screen_contains("@octocat", 15000)
        .expect("the contributors card lists the push event's author");
    session.press_keys(b"ww");
    session
        .wait_for_screen_contains("Contributors · last 7 days", 5000)
        .expect("w twice shows the last week of activity");
    wait_for_line(
        &mut session,
        "@octocat",
        "    1    0    1    0",
        "the week holds octocat's push and review comment",
    );
    session
        .wait_for_screen_contains("unassigned", 5000)
        .expect("the issues card lists its unassigned queue");
    assert!(
        screen_line_with(&session, "unassigned").contains("  1 @ "),
        "the listed issue is unassigned: {}",
        session.emulator.get_text()
    );

    session
        .wait_for_screen_contains("100% succeeded of the last 1", 5000)
        .expect("the pipelines card counts the fetched run before Enter opens its list");
    session.press_keys(b"\rj\r");
    session
        .wait_for_screen_contains("Enter open · Esc back", 5000)
        .expect("Enter on the Pipelines card, which has no rows, goes straight to its list");
    session.press_keys(b"\r");
    session
        .wait_for_screen_contains("┌ > Actions ", 5000)
        .expect("Enter on the listed run opens the Actions tab");
}

#[test]
fn a_short_terminal_shows_the_focused_list_instead_of_hiding_it_under_the_cards() {
    const SHORT_CONFIG: &str =
        "icons = \"ascii\"\n[dashboard]\nenabled = true\n[ui]\nterminal_pane_visible = false\n";
    let mut session = TestSession::with_config_and_args(
        false,
        23,
        150,
        Some(SHORT_CONFIG),
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
        .wait_for_screen_contains("MRs ·", 15000)
        .expect("the card grid renders, exactly as tall as the terminal");

    session.press_keys(b"\r\r\r");
    session
        .wait_for_screen_contains("Enter open · Esc back", 5000)
        .expect("Enter steps down into the list even though it would not fit under the cards");
    session
        .wait_for_screen_contains("!2", 5000)
        .expect("the open MR is visible in the full-area list, not hidden for lack of room");
}

#[test]
fn an_80_by_24_terminal_scrolls_the_cards_to_the_one_selected() {
    let mut session =
        TestSession::with_config_and_args(false, 24, 80, Some(CONFIG), &["--tab", "dashboard"]);
    session
        .wait_for_cli_call("release list", 15000)
        .expect("the dashboard fetches its last source");
    session
        .wait_for_screen_contains("MRs ·", 15000)
        .expect("the first card renders");

    session.press_keys(b"\rjjjj");
    wait_for_line(
        &mut session,
        "┏",
        "Milestones",
        "j walks the one-card-wide grid down to Milestones, scrolled into view",
    );
}
