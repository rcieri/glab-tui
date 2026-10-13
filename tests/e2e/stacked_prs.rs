//! End-to-end coverage for GitHub stacked PRs: stack data costs GraphQL calls
//! only when something on screen shows it.
//!
//! The mock `gh` serves `tests/fixtures/gh_prs.json` for `pr list`: #11
//! "Stack top" and #10 "Stack bottom" form stack #7, #12 "Lone change" is in
//! no stack. The batch query behind the Stack column asks for `stackEntry`
//! only; the per-PR query asks for `stackEntry` and `entries(first`.

use crate::TestSession;
use std::time::{Duration, Instant};

const FIRST_PR_TITLE: &str = "Stack top";
const STACK_COLUMN_CONFIG: &str = "[mrs]\ncolumns = [\"ID\", \"State\", \"Title\", \"Stack\"]\n";
/// Long enough for every follow-up call a list fetch used to trigger to land
/// in the log. Asserting that a call never happens leaves nothing to wait on.
const FOLLOW_UP_WINDOW_MS: u64 = 1500;

fn session_on_prs_tab(config_toml: Option<&str>) -> TestSession {
    let mut session = TestSession::with_config(true, 40, 160, config_toml);
    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app should reach the Issues tab");
    session.send_input(b"l");
    session
        .wait_for_screen_contains(FIRST_PR_TITLE, 15000)
        .expect("the PRs tab should list the fixture PRs");
    session
}

fn calls_matching(session: &TestSession, pattern: &str) -> Vec<String> {
    session
        .get_cli_calls()
        .lines()
        .filter(|line| line.contains(pattern))
        .map(str::to_string)
        .collect()
}

fn batch_stack_calls(session: &TestSession) -> usize {
    calls_matching(session, "stackEntry")
        .iter()
        .filter(|line| !line.contains("entries(first"))
        .count()
}

/// Feeds the app's output to the emulator until `pattern` has been logged
/// `count` times; an undrained PTY blocks the app mid-redraw.
fn wait_for_calls(session: &mut TestSession, pattern: &str, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while calls_matching(session, pattern).len() < count {
        assert!(
            Instant::now() < deadline,
            "expected {count} `{pattern}` calls, log:\n{}",
            session.get_cli_calls()
        );
        session.settle(20);
    }
}

/// Ctrl+r, then wait until the PR list has been fetched again and every
/// follow-up of that fetch had its chance to run.
fn refresh_prs(session: &mut TestSession) {
    let listed = calls_matching(session, "pr list").len();
    session.send_input(b"\x12");
    wait_for_calls(session, "pr list", listed + 1);
    session.settle(FOLLOW_UP_WINDOW_MS);
}

#[test]
fn hidden_stack_column_costs_no_stack_queries_across_a_refresh() {
    let mut session = session_on_prs_tab(None);
    refresh_prs(&mut session);

    assert_eq!(calls_matching(&session, "stackEntry"), Vec::<String>::new());
    assert_eq!(
        calls_matching(&session, "entries(first"),
        Vec::<String>::new()
    );
}

#[test]
fn inspector_fetches_the_selected_prs_stack_once_and_keeps_it_across_a_refresh() {
    let mut session = session_on_prs_tab(None);
    session.send_input(b"\r");
    session
        .wait_for_screen_contains("#7 (position 2 of 2)", 15000)
        .expect("the inspector should show the Stack field");
    session
        .wait_for_screen_contains("Stack #7 (2 PRs)", 15000)
        .expect("the inspector should show the stack breakdown");

    refresh_prs(&mut session);

    let entries_calls = calls_matching(&session, "entries(first");
    assert_eq!(entries_calls.len(), 1, "{entries_calls:?}");
    assert!(
        entries_calls[0].contains("pullRequest(number:11)"),
        "{entries_calls:?}"
    );
    assert_eq!(batch_stack_calls(&session), 0);
    assert!(
        session.emulator.get_text().contains("#7 (position 2 of 2)"),
        "the refreshed PR keeps its stack:\n{}",
        session.emulator.get_text()
    );
}

#[test]
fn view_stack_opens_the_selector_for_a_stacked_pr_and_refuses_a_lone_one() {
    let mut session = session_on_prs_tab(None);
    session.send_input(b"Y");
    session
        .wait_for_screen_contains("Stack #7 — 2 PRs", 15000)
        .expect("Y on a stacked PR should open the stack selector");

    session.send_input(b"\x1b");
    // A lone ESC followed at once by `j` reads as Alt+j.
    session.settle(300);
    session.send_input(b"jj");
    session.settle(300);
    session.send_input(b"Y");
    session
        .wait_for_screen_contains("This PR is not part of a stack.", 15000)
        .expect("Y on a PR outside any stack should say so");

    let entries_calls = calls_matching(&session, "entries(first");
    assert_eq!(entries_calls.len(), 2, "{entries_calls:?}");
    assert!(
        entries_calls[0].contains("pullRequest(number:11)"),
        "{entries_calls:?}"
    );
    assert!(
        entries_calls[1].contains("pullRequest(number:12)"),
        "{entries_calls:?}"
    );
}

#[test]
fn visible_stack_column_costs_one_batch_query_per_list_fetch() {
    let mut session = session_on_prs_tab(Some(STACK_COLUMN_CONFIG));
    session
        .wait_for_screen_contains("#7 2/2", 15000)
        .expect("the Stack column should show #11's position");
    session
        .wait_for_screen_contains("#7 1/2", 15000)
        .expect("the Stack column should show #10's position");

    refresh_prs(&mut session);

    assert_eq!(
        batch_stack_calls(&session),
        calls_matching(&session, "pr list").len()
    );
    assert_eq!(
        calls_matching(&session, "entries(first"),
        Vec::<String>::new()
    );
}

fn request_bodies(session: &TestSession) -> Vec<String> {
    std::fs::read_to_string(format!("{}.stdin", session.sandbox.log_path.display()))
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn merging_a_stacked_pr_merges_its_downstack_through_the_async_endpoint() {
    let mut session = session_on_prs_tab(None);
    session.send_input(b"m");
    session
        .wait_for_screen_contains("Merge Stack #7 through #11", 15000)
        .expect("m on a stacked PR should open the stack merge dialog");
    let dialog = session.emulator.get_text();
    assert!(dialog.contains("#10: Stack bottom"), "{dialog}");
    assert!(dialog.contains("#11: Stack top"), "{dialog}");

    session.send_input(b"\r");
    wait_for_calls(&mut session, "merge-async/mock-merge-uuid", 1);

    let requests = calls_matching(&session, "-X PUT");
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert!(
        requests[0].ends_with("/pulls/11/merge-async"),
        "{requests:?}"
    );
    assert_eq!(
        request_bodies(&session),
        vec![r#"{"merge_method":"squash"}"#]
    );
    assert_eq!(calls_matching(&session, "pr merge"), Vec::<String>::new());
}

#[test]
fn merging_a_pr_outside_any_stack_keeps_using_gh_pr_merge() {
    let mut session = session_on_prs_tab(None);
    session.send_input(b"jj");
    session.settle(300);
    session.send_input(b"m");
    session
        .wait_for_screen_contains("Delete source branch", 15000)
        .expect("m on a lone PR should open the plain merge dialog");

    session.send_input(b"\r");
    wait_for_calls(&mut session, "pr merge 12", 1);

    assert_eq!(
        calls_matching(&session, "merge-async"),
        Vec::<String>::new()
    );
}

#[test]
fn bulk_merge_sends_stacked_prs_through_the_async_endpoint() {
    let mut session = session_on_prs_tab(None);
    session.send_input(b" ");
    session.settle(300);
    session.send_input(b"jj");
    session.settle(300);
    session.send_input(b" ");
    session.settle(300);
    session.send_input(b"m");
    session
        .wait_for_screen_contains("Merge 2 ", 15000)
        .expect("m with two PRs selected should open the bulk merge dialog");

    session.send_input(b"\r");
    wait_for_calls(&mut session, "pr merge 12", 1);
    wait_for_calls(&mut session, "merge-async/mock-merge-uuid", 1);

    let requests = calls_matching(&session, "-X PUT");
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert!(
        requests[0].ends_with("/pulls/11/merge-async"),
        "{requests:?}"
    );
    assert_eq!(
        calls_matching(&session, "pr merge 11"),
        Vec::<String>::new()
    );
}
