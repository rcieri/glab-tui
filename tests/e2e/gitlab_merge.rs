//! A GitLab merge that must not squash clears the MR's own squash flag first,
//! and sets it again when the merge fails: GitLab follows that flag whatever
//! `glab mr merge` is asked, and glab's `--squash-before-merge` flips it
//! rather than setting it.
//!
//! The mock `glab` lists `mrs.json` (!2 "Feature: Add pagination support"),
//! sets the MR's squash flag with `TEST_GLAB_MR_SQUASH`, and takes the
//! project's `squash_option` from `TEST_GLAB_SQUASH_OPTION`, which picks the
//! preselected method: Squash on `default_on`, Merge commit on `default_off`,
//! and Squash for an MR whose squash flag is set.

use crate::{Sandbox, TestSession};

const MR_TITLE: &str = "Feature: Add pagination support";

/// How the merge dialog is confirmed.
enum Pick {
    /// Submit at once, with the preselected method.
    Preselected,
    /// Pick Merge commit, the first of the radio, then Submit.
    MergeCommit,
}

fn merge_the_listed_mr(extra: &[(&str, &str)], pick: Pick) -> TestSession {
    let sandbox = Sandbox::new(false).unwrap();
    let conf_dir = sandbox.config_dir.join("glab-tui");
    std::fs::create_dir_all(&conf_dir).unwrap();
    std::fs::write(conf_dir.join("config.toml"), "icons = \"ascii\"\n").unwrap();
    let mut session = TestSession::launch(sandbox, 40, 160, extra);
    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app should reach the Issues tab");
    session.send_input(b"l");
    session
        .wait_for_screen_contains(MR_TITLE, 15000)
        .expect("the MRs tab should list the fixture MR");
    session.send_input(b"m");
    session
        .wait_for_screen_contains("Merge method", 15000)
        .expect("the merge dialog should show its method radio");
    session
        .wait_for_screen_without("loading…", 15000)
        .expect("the merge dialog should list the methods the project allows");
    if let Pick::MergeCommit = pick {
        session.press_keys(b"\th ");
        session
            .wait_for_screen_contains("* Merge commit", 5000)
            .expect("Merge commit should be picked");
        session.press_keys(b"\t\th");
        session
            .wait_for_screen_contains("Enter merge", 5000)
            .expect("the focus should be back on Submit");
    }
    session.send_input(b"\r");
    session
        .wait_for_cli_call("glab mr merge 2", 15000)
        .expect("Submit should merge the MR");
    session
}

fn calls(session: &TestSession) -> Vec<String> {
    session
        .get_cli_calls()
        .lines()
        .map(str::to_string)
        .collect()
}

fn position(calls: &[String], prefix: &str) -> Option<usize> {
    calls.iter().position(|call| call.starts_with(prefix))
}

#[test]
fn a_merge_commit_clears_the_mr_squash_flag_before_merging() {
    let session = merge_the_listed_mr(
        &[
            ("TEST_GLAB_SQUASH_OPTION", "default_off"),
            ("TEST_GLAB_MR_SQUASH", "1"),
        ],
        Pick::MergeCommit,
    );
    let calls = calls(&session);
    let update = position(&calls, "glab mr update 2 --squash-before-merge -R ")
        .unwrap_or_else(|| panic!("no squash flag update in {calls:#?}"));
    let merge = position(&calls, "glab mr merge 2 ").unwrap();
    assert!(update < merge, "{calls:#?}");
    assert!(!calls[merge].contains("--squash"), "{}", calls[merge]);
}

#[test]
fn a_squash_merge_leaves_the_mr_squash_flag_alone() {
    let session = merge_the_listed_mr(&[("TEST_GLAB_MR_SQUASH", "1")], Pick::Preselected);
    let calls = calls(&session);
    assert_eq!(position(&calls, "glab mr update "), None, "{calls:#?}");
    let merge = position(&calls, "glab mr merge 2 ").unwrap();
    assert!(calls[merge].contains(" --squash"), "{}", calls[merge]);
}

#[test]
fn a_merge_commit_of_an_mr_without_the_squash_flag_sends_no_update() {
    let session = merge_the_listed_mr(
        &[("TEST_GLAB_SQUASH_OPTION", "default_off")],
        Pick::Preselected,
    );
    let calls = calls(&session);
    assert_eq!(position(&calls, "glab mr update "), None, "{calls:#?}");
}

#[test]
fn an_mr_whose_author_asked_for_a_squash_is_squashed_by_default() {
    let session = merge_the_listed_mr(
        &[
            ("TEST_GLAB_SQUASH_OPTION", "default_off"),
            ("TEST_GLAB_MR_SQUASH", "1"),
        ],
        Pick::Preselected,
    );
    let calls: Vec<String> = calls(&session)
        .into_iter()
        .filter(|call| call.starts_with("glab mr update") || call.starts_with("glab mr merge"))
        .collect();
    assert_eq!(
        calls,
        ["glab mr merge 2 -R test-owner/test-repo --squash --remove-source-branch --yes"]
    );
}

#[test]
fn a_failed_merge_sets_the_cleared_squash_flag_again() {
    let mut session = merge_the_listed_mr(
        &[
            ("TEST_GLAB_SQUASH_OPTION", "default_off"),
            ("TEST_GLAB_MR_SQUASH", "1"),
            ("TEST_GLAB_FAIL_MATCH", "mr merge"),
        ],
        Pick::MergeCommit,
    );
    session
        .wait_for_cli_call_count("glab mr update 2 --squash-before-merge", 2, 15000)
        .expect("the failed merge should set the squash flag again");
    let calls: Vec<String> = calls(&session)
        .into_iter()
        .filter(|call| call.starts_with("glab mr update") || call.starts_with("glab mr merge"))
        .collect();
    assert_eq!(
        calls,
        [
            "glab mr update 2 --squash-before-merge -R test-owner/test-repo",
            "glab mr merge 2 -R test-owner/test-repo --remove-source-branch --yes",
            "glab mr update 2 --squash-before-merge -R test-owner/test-repo",
        ]
    );
}
