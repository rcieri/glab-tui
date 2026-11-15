//! End-to-end coverage for the merge dialog: the method and options picked
//! in the form are the flags the merge call carries.
//!
//! The mock `glab` lists !2 without a head SHA, so no `--sha` is sent, and
//! takes the project's `merge_method` from `TEST_GLAB_MERGE_METHOD`. The
//! mock `gh` lists #12 "Lone change" third, outside any stack, so it gets the
//! plain merge dialog.

use crate::{Sandbox, TestSession};

const GITLAB_MR_TITLE: &str = "Feature: Add pagination support";
const GITHUB_FIRST_PR_TITLE: &str = "Stack top";
const ASCII_ICONS: &str = "icons = \"ascii\"\n";

/// Launches on `is_github`'s fixture repository with `envs` for the CLI
/// mocks, moves to the MRs/PRs tab, presses `row_keys` to reach the MR to
/// merge and opens its merge dialog.
fn open_merge_dialog(
    is_github: bool,
    envs: &[(&str, &str)],
    first_title: &str,
    row_keys: &[u8],
    dialog_title: &str,
) -> TestSession {
    let sandbox = Sandbox::new(is_github).unwrap();
    let conf_dir = sandbox.config_dir.join("glab-tui");
    std::fs::create_dir_all(&conf_dir).unwrap();
    std::fs::write(conf_dir.join("config.toml"), ASCII_ICONS).unwrap();
    let mut session = TestSession::launch(sandbox, 40, 160, envs);
    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app should reach the Issues tab");
    session.send_input(b"l");
    session
        .wait_for_screen_contains(first_title, 15000)
        .expect("the MRs tab should list the fixture MRs");
    session.press_keys(row_keys);
    session.send_input(b"m");
    session
        .wait_for_screen_contains(dialog_title, 15000)
        .expect("m should open the merge dialog of the highlighted MR");
    session
        .wait_for_screen_without("loading…", 15000)
        .expect("the dialog should list the methods the project allows");
    session
        .wait_for_screen_contains("h/l button", 5000)
        .expect("the dialog opens on a button");
    session
        .wait_for_screen_contains("Enter merge", 5000)
        .expect("the dialog opens on its Merge button");
    session
}

/// Waits for the first call starting with `prefix` and returns every call
/// that does, once the dialog has closed.
fn merge_calls(session: &mut TestSession, prefix: &str) -> Vec<String> {
    session
        .wait_for_cli_call(prefix, 10000)
        .expect("confirming should run the merge");
    session
        .wait_for_screen_without("Enter merge", 5000)
        .expect("the dialog closes once confirmed");
    session
        .get_cli_calls()
        .lines()
        .filter(|call| call.starts_with(prefix))
        .map(str::to_string)
        .collect()
}

/// Presses `keys` one at a time from the current focus, then waits
/// until every entry of `shown` is drawn, so Enter confirms what was picked.
fn pick(session: &mut TestSession, keys: &[u8], shown: &[&str]) {
    session.press_keys(keys);
    for choice in shown {
        session
            .wait_for_screen_contains(choice, 5000)
            .unwrap_or_else(|err| panic!("the dialog should show `{choice}`: {err}"));
    }
}

#[test]
fn gitlab_merge_dialog_squashes_and_removes_the_source_branch_by_default() {
    let mut session =
        open_merge_dialog(false, &[], GITLAB_MR_TITLE, b"", "Merge Merge Request #2");
    session.send_input(b"\r");

    assert_eq!(
        merge_calls(&mut session, "glab mr merge"),
        ["glab mr merge 2 -R test-owner/test-repo --squash --remove-source-branch --yes"]
    );
}

#[test]
fn github_merge_dialog_squashes_and_deletes_the_branch_by_default() {
    let mut session =
        open_merge_dialog(true, &[], GITHUB_FIRST_PR_TITLE, b"jj", "Merge Pull Request #12");
    session.send_input(b"\r");

    assert_eq!(
        merge_calls(&mut session, "gh pr merge"),
        ["gh pr merge 12 -R test-owner/test-repo --squash --delete-branch"]
    );
}

/// The mocked GitLab project merges with merge commits, so it offers Merge
/// commit and Squash, not Rebase.
#[test]
fn gitlab_merge_dialog_sends_the_picked_method_and_options() {
    let mut session =
        open_merge_dialog(false, &[], GITLAB_MR_TITLE, b"", "Merge Merge Request #2");
    pick(&mut session, b"\th ", &["* Merge commit"]);
    pick(&mut session, b"\t ", &["- Delete source branch"]);
    pick(&mut session, b"j ", &["+ Auto-merge"]);
    pick(&mut session, b"\t", &["Enter cancel"]);
    pick(&mut session, b"h", &["Enter merge"]);
    session.send_input(b"\r");

    assert_eq!(
        merge_calls(&mut session, "glab mr merge"),
        ["glab mr merge 2 -R test-owner/test-repo --auto-merge=true --yes"]
    );
}

/// A fast-forward project names its plain merge Fast-forward and offers
/// Rebase, which is sent without `--sha`.
#[test]
fn gitlab_merge_dialog_rebases_on_a_fast_forward_project() {
    let mut session = open_merge_dialog(
        false,
        &[("TEST_GLAB_MERGE_METHOD", "ff")],
        GITLAB_MR_TITLE,
        b"",
        "Merge Merge Request #2",
    );
    pick(&mut session, b"", &["Fast-forward", "* Squash", "Rebase"]);
    pick(&mut session, b"\tl ", &["* Rebase"]);
    pick(&mut session, b"\t\th", &["Enter merge"]);
    session.send_input(b"\r");

    assert_eq!(
        merge_calls(&mut session, "glab mr merge"),
        ["glab mr merge 2 -R test-owner/test-repo --remove-source-branch --rebase --yes"]
    );
}

#[test]
fn github_merge_dialog_sends_the_picked_method_and_options() {
    let mut session =
        open_merge_dialog(true, &[], GITHUB_FIRST_PR_TITLE, b"jj", "Merge Pull Request #12");
    pick(&mut session, b"\th ", &["* Merge commit"]);
    pick(&mut session, b"\t ", &["- Delete source branch"]);
    pick(&mut session, b"j ", &["+ Auto-merge"]);
    pick(&mut session, b"\t", &["Enter cancel"]);
    pick(&mut session, b"h", &["Enter merge"]);
    session.send_input(b"\r");

    assert_eq!(
        merge_calls(&mut session, "gh pr merge"),
        ["gh pr merge 12 -R test-owner/test-repo --merge --auto"]
    );
}
