//! Smoke coverage for the secondary tabs on GitLab: each tab lists every row
//! of its fixture, narrows to one row under a `/` search, and runs one action
//! against the row the search left selected.
//!
//! The mock `glab` serves `runners.json` for `runner list`, `releases.json`
//! for `release list`, `todos.json` for `todo list`, `milestones.json` for
//! `milestone list`, `branches.json` for the branches endpoint and
//! `environments.json` / `deployments.json` for the environment endpoints.
//! Every fixture holds two rows, so a search that matches one of them is
//! visible on screen.

use crate::TestSession;

const ASCII_ICONS: &str = "icons = \"ascii\"\n";

/// Launch with ASCII icons (the test emulator gives a multi-byte glyph one
/// cell per byte) and move right `tab_index` tabs from Issues.
fn session_on_tab(tab_index: usize, first_row: &str) -> TestSession {
    let mut session = TestSession::with_config(false, 24, 140, Some(ASCII_ICONS));
    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app should reach the Issues tab");
    session.press_keys(&b"l".repeat(tab_index));
    session
        .wait_for_screen_contains(first_row, 15000)
        .expect("the tab should list its fixture rows");
    session
}

/// Applies `query` as the list search and waits for the filtered list.
fn search(session: &mut TestSession, query: &str) {
    session.press_keys(b"/");
    session.press_keys(query.as_bytes());
    session.press_keys(b"\r");
    session
        .wait_for_screen_contains("FILTERED", 5000)
        .expect("Enter should keep the search as a filter");
    session.settle(300);
}

/// The screen above the Terminal pane, whose command log echoes arguments such
/// as milestone titles back onto the screen.
fn list_pane(session: &TestSession) -> String {
    session
        .emulator
        .get_text()
        .lines()
        .take_while(|row| !row.contains(" Terminal "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_lists(session: &TestSession, rows: &[&str]) {
    let pane = list_pane(session);
    for row in rows {
        assert!(pane.contains(row), "{row:?} should be listed:\n{pane}");
    }
}

fn assert_hides(session: &TestSession, row: &str) {
    let pane = list_pane(session);
    assert!(
        !pane.contains(row),
        "{row:?} should be filtered out:\n{pane}"
    );
}

/// Moves the focus of an open confirmation dialog to Submit and activates it.
fn confirm_dialog(session: &mut TestSession, title: &str) {
    session
        .wait_for_screen_contains(title, 5000)
        .expect("the action should ask for confirmation");
    session.press_keys(b"h\r");
}

#[test]
fn runners_tab_lists_filters_and_pauses_the_matching_runner() {
    let mut session = session_on_tab(4, "Shared Runner 1");
    assert_lists(
        &session,
        &["#9876", "Shared Runner 1", "#9877", "Docker Runner 2"],
    );

    search(&mut session, "Docker");
    assert_hides(&session, "Shared Runner 1");

    session.press_keys(b"p");
    session
        .wait_for_cli_call(
            "glab runner update 9877 --pause -R test-owner/test-repo",
            5000,
        )
        .expect("p should pause the runner the search selected");
}

#[test]
fn releases_tab_lists_filters_and_deletes_the_matching_release() {
    let mut session = session_on_tab(5, "Second release");
    assert_lists(
        &session,
        &["v1.1.0", "Second release", "v1.0.0", "First release"],
    );

    search(&mut session, "First");
    assert_hides(&session, "Second release");

    session.press_keys(b"d");
    confirm_dialog(&mut session, "Delete Release v1.0.0");
    session
        .wait_for_cli_call(
            "glab release delete v1.0.0 -R test-owner/test-repo -y",
            5000,
        )
        .expect("confirming should delete the release the search selected");
}

#[test]
fn todos_tab_lists_filters_and_marks_the_matching_todo_done() {
    let mut session = session_on_tab(6, "Todo Item 1");
    assert_lists(&session, &["Todo Item 1", "Review pagination"]);

    search(&mut session, "Review");
    assert_hides(&session, "Todo Item 1");

    session.press_keys(b"\r");
    session
        .wait_for_cli_call("glab todo done 2", 5000)
        .expect("Enter should mark the todo the search selected as done");
    session
        .wait_for_screen_contains(" MRs ", 5000)
        .expect("a merge request todo should open the MRs tab");
}

#[test]
fn milestones_tab_lists_filters_and_closes_the_matching_milestone() {
    let mut session = session_on_tab(7, "v1.0 Milestone");
    assert_lists(&session, &["v1.0 Milestone", "v2.0 Roadmap"]);

    search(&mut session, "Roadmap");
    assert_hides(&session, "v1.0 Milestone");

    session.press_keys(b"c");
    confirm_dialog(&mut session, "Close Milestone #2");
    session
        .wait_for_cli_call("glab milestone close 2 -R test-owner/test-repo", 5000)
        .expect("confirming should close the milestone the search selected");
}

#[test]
fn branches_tab_lists_filters_and_deletes_the_matching_branch() {
    let mut session = session_on_tab(8, "feature/pagination");
    let pane = list_pane(&session);
    assert!(
        pane.lines()
            .any(|row| row.contains("main") && row.contains("YES")),
        "the default branch should be listed as default and protected:\n{pane}"
    );

    search(&mut session, "feature");
    let pane = list_pane(&session);
    assert!(
        !pane.lines().any(|row| row.contains("YES")),
        "the default branch should be filtered out:\n{pane}"
    );

    session.press_keys(b"d");
    confirm_dialog(&mut session, "Delete Branch 'feature/pagination'");
    session
        .wait_for_cli_call(
            "glab api -X DELETE /projects/test-owner%2Ftest-repo/repository/branches/feature",
            5000,
        )
        .expect("confirming should delete the branch the search selected");
}

#[test]
fn environments_tab_lists_filters_and_fetches_the_matching_deployments() {
    let mut session = session_on_tab(9, "production");
    assert_lists(&session, &["production", "available", "staging", "stopped"]);

    search(&mut session, "prod");
    assert_hides(&session, "staging");

    session.press_keys(b"\r");
    session
        .wait_for_cli_call(
            "glab api /projects/test-owner%2Ftest-repo/deployments?per_page=100&environment=production",
            5000,
        )
        .expect("Enter should fetch the deployments of the environment the search selected");
}
