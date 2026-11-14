//! End-to-end coverage for the Assignees and Reviewers fields of the GitLab
//! edit menu: the `glab issue update` / `glab mr update` call they lead to.
//!
//! The mock `glab` serves `tests/fixtures/members.json` (`test-user`,
//! `reviewer-user`, `other-user`) as the project members. `mrs.json` holds
//! MR !2, assigned to `test-user` with `reviewer-user` as reviewer.
//! `TEST_GLAB_ISSUES=assigned_issues.json` lists issue #7, assigned to
//! `test-user`. glab's `--assignee` and `--reviewer` add a user with a `+`
//! prefix, remove one with `!` and replace all of them without a prefix.

use crate::TestSession;

const MR_TITLE: &str = "Feature: Add pagination support";
const ASSIGNED_ISSUE_TITLE: &str = "Assigned to test-user";
const ISSUE_UPDATE: &str = "glab issue update 7 -R test-owner/test-repo";
const MR_UPDATE: &str = "glab mr update 2 -R test-owner/test-repo";

/// Launch on the Issues tab listing issue #7, assigned to `test-user`, and
/// open its edit menu.
fn edit_assigned_issue() -> TestSession {
    let mut session = TestSession::with_envs(
        false,
        40,
        140,
        &[("TEST_GLAB_ISSUES", "assigned_issues.json")],
    );
    session
        .wait_for_screen_contains(ASSIGNED_ISSUE_TITLE, 30000)
        .expect("issue #7 listed");
    session.press_keys(b"e");
    session
        .wait_for_screen_contains("Edit Issue #7", 5000)
        .expect("e opens the edit menu");
    session
}

/// Launch, switch to the MRs tab and open the edit menu of MR !2.
fn edit_mr() -> TestSession {
    let mut session = TestSession::new(false, 40, 140);
    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app reaches the Issues tab");
    session.send_input(b"l");
    session
        .wait_for_screen_contains(MR_TITLE, 15000)
        .expect("MRs tab lists MR !2");
    session.press_keys(b"e");
    session
        .wait_for_screen_contains("Edit MR #2", 5000)
        .expect("e opens the edit menu");
    session
}

/// Opens the field `keys` move to, toggles each user in `users` in its
/// selector and confirms the selection.
fn toggle_people(session: &mut TestSession, keys: &[u8], selector: &str, users: &[&str]) {
    session.press_keys(keys);
    session
        .wait_for_screen_contains(selector, 5000)
        .unwrap_or_else(|e| panic!("{selector} opens: {e}"));
    session
        .wait_for_screen_contains("@other-user", 5000)
        .expect("the selector lists the project members");
    for user in users {
        session.press_keys(b"/");
        session.press_keys(user.as_bytes());
        session.press_keys(b"\r ");
        session.press_keys(b"\x1b");
    }
    session.press_keys(b"\r");
    session
        .wait_for_screen_without(selector, 5000)
        .expect("Enter confirms the selection");
}

/// Submits the edit menu and returns the arguments after `call` of every
/// recorded call starting with it, once `count` such calls ran.
fn submit_and_read_calls(session: &mut TestSession, call: &str, count: usize) -> Vec<String> {
    session.press_keys(b"\x18"); // Ctrl+x
    session
        .wait_for_cli_call_count(call, count, 5000)
        .expect("the edit menu submits the update");
    session
        .get_cli_calls()
        .lines()
        .filter_map(|line| line.strip_prefix(call))
        .map(|rest| rest.trim().to_string())
        .collect()
}

/// The arguments of `call`, sorted: glab reads each flag on its own, so their
/// order does not matter.
fn sorted_args(call: &str) -> Vec<&str> {
    let mut args: Vec<&str> = call.split_whitespace().collect();
    args.sort();
    args
}

/// Asserts that exactly one call was recorded, with `expected` as its
/// arguments in any order.
fn assert_one_call_with(calls: &[String], expected: &[&str]) {
    assert_eq!(calls.len(), 1, "one update call: {calls:?}");
    let mut expected = expected.to_vec();
    expected.sort();
    assert_eq!(
        sorted_args(&calls[0]),
        expected,
        "recorded call: {}",
        calls[0]
    );
}

#[test]
fn test_adding_an_issue_assignee_keeps_the_others() {
    let mut session = edit_assigned_issue();
    toggle_people(&mut session, b"jj\r", "Select Assignees", &["other"]);

    let calls = submit_and_read_calls(&mut session, ISSUE_UPDATE, 1);
    assert_one_call_with(&calls, &["--assignee=+other-user"]);
}

#[test]
fn test_removing_an_issue_assignee_unassigns_only_that_user() {
    let mut session = edit_assigned_issue();
    toggle_people(&mut session, b"jj\r", "Select Assignees", &["test"]);

    let calls = submit_and_read_calls(&mut session, ISSUE_UPDATE, 1);
    assert_one_call_with(&calls, &["--assignee=!test-user"]);
}

#[test]
fn test_adding_and_removing_issue_assignees_in_one_submit_sends_both_changes() {
    let mut session = edit_assigned_issue();
    toggle_people(
        &mut session,
        b"jj\r",
        "Select Assignees",
        &["other", "test"],
    );

    let calls = submit_and_read_calls(&mut session, ISSUE_UPDATE, 1);
    assert_one_call_with(&calls, &["--assignee=+other-user", "--assignee=!test-user"]);
}

#[test]
fn test_adding_and_removing_mr_assignees_in_one_submit_sends_both_changes() {
    let mut session = edit_mr();
    toggle_people(
        &mut session,
        b"jjj\r",
        "Select Assignees",
        &["other", "test"],
    );

    let calls = submit_and_read_calls(&mut session, MR_UPDATE, 1);
    assert_one_call_with(&calls, &["--assignee=+other-user", "--assignee=!test-user"]);
}

#[test]
fn test_adding_an_mr_reviewer_keeps_the_others() {
    let mut session = edit_mr();
    toggle_people(&mut session, b"jjjj\r", "Select Reviewers", &["other"]);

    let calls = submit_and_read_calls(&mut session, MR_UPDATE, 1);
    assert_one_call_with(&calls, &["--reviewer=+other-user"]);
}

#[test]
fn test_removing_an_mr_reviewer_removes_only_that_reviewer() {
    let mut session = edit_mr();
    toggle_people(&mut session, b"jjjj\r", "Select Reviewers", &["reviewer"]);

    let calls = submit_and_read_calls(&mut session, MR_UPDATE, 1);
    assert_one_call_with(&calls, &["--reviewer=!reviewer-user"]);
}

#[test]
fn test_bulk_editing_assignees_adds_them_to_every_selected_issue() {
    let mut session = TestSession::new(false, 40, 140);
    session
        .wait_for_screen_contains("Worktree for seven", 30000)
        .expect("issues listed");
    session.press_keys(b" j ");
    session.press_keys(b"e");
    session
        .wait_for_screen_contains("Bulk Edit 2 Issues", 5000)
        .expect("e with two selected issues opens the bulk edit menu");
    toggle_people(&mut session, b"\r", "Select Assignees", &["other"]);

    let mut calls = submit_and_read_calls(&mut session, "glab issue update", 2);
    calls.sort();
    assert_eq!(calls.len(), 2, "one update call per issue: {calls:?}");
    for (call, iid) in calls.iter().zip(["7", "8"]) {
        let mut expected = vec![iid, "-R", "test-owner/test-repo", "--assignee=+other-user"];
        expected.sort();
        assert_eq!(sorted_args(call), expected, "recorded call: {call}");
    }
}
