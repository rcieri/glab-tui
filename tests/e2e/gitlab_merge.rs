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
    /// Pick Rebase and squash, right of the preselected Squash, then Submit.
    RebaseAndSquash,
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
    let picked = match pick {
        Pick::Preselected => None,
        Pick::MergeCommit => Some((b"\th ", "* Merge commit")),
        Pick::RebaseAndSquash => Some((b"\tl ", "* Rebase and squash")),
    };
    if let Some((keys, shown)) = picked {
        session.press_keys(keys);
        session
            .wait_for_screen_contains(shown, 5000)
            .expect("the method should be picked");
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

/// Every squash-flag update and merge call, in the order they were made.
fn merge_calls(session: &TestSession) -> Vec<String> {
    session
        .get_cli_calls()
        .lines()
        .filter(|call| call.starts_with("glab mr update") || call.starts_with("glab mr merge"))
        .map(str::to_string)
        .collect()
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
    assert_eq!(
        merge_calls(&session),
        [
            "glab mr update 2 --squash-before-merge -R test-owner/test-repo",
            "glab mr merge 2 -R test-owner/test-repo --remove-source-branch --yes",
        ]
    );
}

#[test]
fn a_squash_merge_leaves_the_mr_squash_flag_alone() {
    let session = merge_the_listed_mr(&[("TEST_GLAB_MR_SQUASH", "1")], Pick::Preselected);
    assert_eq!(
        merge_calls(&session),
        ["glab mr merge 2 -R test-owner/test-repo --squash --remove-source-branch --yes"]
    );
}

#[test]
fn a_merge_commit_of_an_mr_without_the_squash_flag_sends_no_update() {
    let session = merge_the_listed_mr(
        &[("TEST_GLAB_SQUASH_OPTION", "default_off")],
        Pick::Preselected,
    );
    assert_eq!(
        merge_calls(&session),
        ["glab mr merge 2 -R test-owner/test-repo --remove-source-branch --yes"]
    );
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
    assert_eq!(
        merge_calls(&session),
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
    assert_eq!(
        merge_calls(&session),
        [
            "glab mr update 2 --squash-before-merge -R test-owner/test-repo",
            "glab mr merge 2 -R test-owner/test-repo --remove-source-branch --yes",
            "glab mr update 2 --squash-before-merge -R test-owner/test-repo",
        ]
    );
}

/// The second `mr update` is the restore, so only it fails with 404, while
/// every `mr merge` fails with 500.
#[test]
fn a_failed_restore_still_reports_why_the_merge_failed() {
    let mut session = merge_the_listed_mr(
        &[
            ("TEST_GLAB_SQUASH_OPTION", "default_off"),
            ("TEST_GLAB_MR_SQUASH", "1"),
            ("TEST_GLAB_FAIL_MATCH", "mr update"),
            ("TEST_GLAB_FAIL_NTH", "2"),
            ("TEST_GLAB_FAIL_MATCH_2", "mr merge"),
        ],
        Pick::MergeCommit,
    );
    session
        .wait_for_screen_contains(
            "mock: 500 Internal Server Error (restoring the squash flag of !2 also failed: glab command failed: mock: 404 Not Found)",
            15000,
        )
        .expect("the toast should give the merge's reason, then the restore's");
    assert_eq!(
        merge_calls(&session),
        [
            "glab mr update 2 --squash-before-merge -R test-owner/test-repo",
            "glab mr merge 2 -R test-owner/test-repo --remove-source-branch --yes",
            "glab mr update 2 --squash-before-merge -R test-owner/test-repo",
        ]
    );
}

#[test]
fn a_rebase_on_a_project_that_always_squashes_leaves_the_squash_flag_alone() {
    let session = merge_the_listed_mr(
        &[
            ("TEST_GLAB_MERGE_METHOD", "ff"),
            ("TEST_GLAB_SQUASH_OPTION", "always"),
            ("TEST_GLAB_MR_SQUASH", "1"),
        ],
        Pick::RebaseAndSquash,
    );
    assert_eq!(
        merge_calls(&session),
        ["glab mr merge 2 -R test-owner/test-repo --remove-source-branch --rebase --yes"]
    );
}
