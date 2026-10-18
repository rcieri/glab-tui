//! End-to-end coverage for the State column filter narrowing GitHub's
//! `gh issue list` and `gh pr list` through `--state`.
//!
//! The mock `gh` logs every call. `pr list --state merged` serves
//! `tests/fixtures/gh_prs_merged.json` (#20 "Merged change"). Every other PR
//! listing serves `tests/fixtures/gh_prs.json`, whose PRs are all open.

use crate::TestSession;
use std::time::{Duration, Instant};

const OPEN_PR_TITLE: &str = "Stack top";
const MERGED_PR_TITLE: &str = "Merged change";

fn state_filter_config(pane: &str, states: &[&str]) -> String {
    let quoted: Vec<String> = states.iter().map(|state| format!("\"{state}\"")).collect();
    format!("[{pane}.column_filters]\nState = [{}]\n", quoted.join(", "))
}

/// The `--state` value of every logged `gh <subcommand>` call, in call order.
fn state_flags(session: &TestSession, subcommand: &str) -> Vec<String> {
    let prefix = format!("gh {subcommand} ");
    session
        .get_cli_calls()
        .lines()
        .filter(|line| line.starts_with(&prefix))
        .map(|line| {
            let mut args = line.split_whitespace();
            args.by_ref().find(|arg| *arg == "--state");
            args.next().unwrap_or("<none>").to_string()
        })
        .collect()
}

/// Pumps the app's output until `subcommand` has been logged `count` times;
/// an undrained PTY blocks the app mid-redraw.
fn wait_for_state_flags(session: &mut TestSession, subcommand: &str, count: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let flags = state_flags(session, subcommand);
        if flags.len() >= count {
            return flags;
        }
        assert!(
            Instant::now() < deadline,
            "expected {count} `gh {subcommand}` calls, log:\n{}",
            session.get_cli_calls()
        );
        session.settle(20);
    }
}

fn first_issue_list_state(states: &[&str]) -> String {
    let config = (!states.is_empty()).then(|| state_filter_config("issues", states));
    let mut session = TestSession::with_config(true, 40, 160, config.as_deref());
    wait_for_state_flags(&mut session, "issue list", 1).remove(0)
}

fn launch_on_prs_tab(states: &[&str]) -> TestSession {
    let config = (!states.is_empty()).then(|| state_filter_config("mrs", states));
    let mut session = TestSession::with_config(true, 40, 160, config.as_deref());
    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app should reach the Issues tab");
    session.send_input(b"l");
    session
}

fn first_pr_list_state(states: &[&str]) -> String {
    let mut session = launch_on_prs_tab(states);
    wait_for_state_flags(&mut session, "pr list", 1).remove(0)
}

fn press_keys(session: &mut TestSession, keys: &[u8]) {
    for key in keys {
        session.send_input(&[*key]);
        session.settle(150);
    }
}

#[test]
fn issue_list_asks_for_the_single_state_the_filter_shows() {
    assert_eq!(first_issue_list_state(&["opened"]), "open");
    assert_eq!(first_issue_list_state(&["closed"]), "closed");
}

#[test]
fn issue_list_asks_for_every_state_with_both_states_or_no_filter() {
    assert_eq!(first_issue_list_state(&["opened", "closed"]), "all");
    assert_eq!(first_issue_list_state(&[]), "all");
}

#[test]
fn pr_list_asks_for_the_single_state_the_filter_shows() {
    assert_eq!(first_pr_list_state(&["opened"]), "open");
    assert_eq!(first_pr_list_state(&["merged"]), "merged");
    assert_eq!(first_pr_list_state(&["closed"]), "closed");
}

#[test]
fn pr_list_asks_for_every_state_with_several_states_or_no_filter() {
    assert_eq!(first_pr_list_state(&["opened", "merged"]), "all");
    assert_eq!(first_pr_list_state(&["opened", "merged", "closed"]), "all");

    let mut session = launch_on_prs_tab(&[]);
    assert_eq!(
        wait_for_state_flags(&mut session, "pr list", 1),
        vec!["all"]
    );
    session
        .wait_for_screen_contains(OPEN_PR_TITLE, 15000)
        .expect("an unfiltered list shows the fixture PRs");
}

#[test]
fn picking_merged_in_the_state_filter_refetches_merged_prs() {
    let mut session = launch_on_prs_tab(&["OPEN"]);
    session
        .wait_for_screen_contains(OPEN_PR_TITLE, 15000)
        .expect("the open-only list shows the open fixture PRs");
    assert_eq!(
        wait_for_state_flags(&mut session, "pr list", 1),
        vec!["open"]
    );

    // Configure popup, State column (second row), value picker.
    press_keys(&mut session, b",j\r");
    session
        .wait_for_screen_contains("Filter by State", 5000)
        .expect("Enter on the State column opens its value picker");
    // Picker rows are CLOSED, MERGED, OPEN: select MERGED, drop OPEN, confirm.
    press_keys(&mut session, b"j j \r\x1b");

    assert_eq!(
        wait_for_state_flags(&mut session, "pr list", 2),
        vec!["open", "merged"]
    );
    session
        .wait_for_screen_contains(MERGED_PR_TITLE, 15000)
        .expect("the refetched merged PRs replace the open ones");
    assert!(
        !session.emulator.get_text().contains(OPEN_PR_TITLE),
        "open PRs must not stay listed under the MERGED filter:\n{}",
        session.emulator.get_text()
    );
}
