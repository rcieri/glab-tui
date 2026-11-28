//! A GitLab merge that must not squash clears the MR's own squash flag first,
//! and sets it again when the merge fails: GitLab follows that flag whatever
//! `glab mr merge` is asked, and glab's `--squash-before-merge` flips it
//! rather than setting it.
//!
//! The mock `glab` lists `mrs.json` (!2 "Feature: Add pagination support"),
//! sets the MR's squash flag with `TEST_GLAB_MR_SQUASH`, and takes the
//! project's `squash_option` from `TEST_GLAB_SQUASH_OPTION`, which picks the
//! preselected method: Squash on `default_on`, Merge commit on `default_off`,
//! and Squash for an MR whose squash flag is set. The project named by
//! `TEST_GLAB_ALWAYS_SQUASH_PROJECT` squashes every merge instead.

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
/// every `mr merge` fails with a two-line 500. The toast keeps only the first
/// line of the error, so the squash flag left cleared has to come first.
#[test]
fn a_failed_restore_leads_the_toast_with_the_cleared_squash_flag() {
    let mut session = merge_the_listed_mr(
        &[
            ("TEST_GLAB_SQUASH_OPTION", "default_off"),
            ("TEST_GLAB_MR_SQUASH", "1"),
            ("TEST_GLAB_FAIL_MATCH", "mr update"),
            ("TEST_GLAB_FAIL_NTH", "2"),
            ("TEST_GLAB_FAIL_MATCH_2", "mr merge"),
            (
                "TEST_GLAB_FAIL_DETAIL_2",
                "mock: merge request is not mergeable",
            ),
        ],
        Pick::MergeCommit,
    );
    session
        .wait_for_screen_contains(
            "restoring the squash flag of !2 failed (glab command failed: mock: 404 Not Found), so it stays cleared. The merge failed:",
            15000,
        )
        .expect("the toast should say the squash flag stays cleared, then why the merge failed");
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

/// In group scope alpha!5 and beta!5 are both listed, and only alpha always
/// squashes. Both fast-forward, so Rebase is a method they share.
#[test]
fn a_bulk_rebase_leaves_the_squash_flag_of_an_always_squash_project_alone() {
    let sandbox = Sandbox::new(false).unwrap();
    let conf_dir = sandbox.config_dir.join("glab-tui");
    std::fs::create_dir_all(&conf_dir).unwrap();
    std::fs::write(conf_dir.join("config.toml"), "icons = \"ascii\"\n").unwrap();
    let mut session = TestSession::launch_with_args(
        sandbox,
        40,
        160,
        &["-g", "test-group"],
        &[
            ("TEST_GLAB_MERGE_METHOD", "ff"),
            ("TEST_GLAB_ALWAYS_SQUASH_PROJECT", "test-group/alpha"),
            ("TEST_GLAB_MR_SQUASH", "1"),
        ],
    );
    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app should reach the Issues tab");
    session.send_input(b"l");
    session
        .wait_for_screen_contains("Beta feature", 15000)
        .expect("the MRs tab should list both projects' MRs");
    session.press_keys(b" j ");
    session.send_input(b"m");
    session
        .wait_for_screen_contains("Merge method", 15000)
        .expect("the bulk merge dialog should show its method radio");
    session
        .wait_for_screen_without("loading…", 15000)
        .expect("the dialog should list the methods both projects allow");
    session.press_keys(b"\tl ");
    session
        .wait_for_screen_contains("* Rebase", 5000)
        .expect("Rebase should be picked");
    session.press_keys(b"\t\th");
    session
        .wait_for_screen_contains("Enter merge", 5000)
        .expect("the focus should be back on Submit");
    session.send_input(b"\r");
    session
        .wait_for_cli_call_count("glab mr merge 5", 2, 15000)
        .expect("both MRs should be merged");
    assert_eq!(
        merge_calls(&session),
        [
            "glab mr merge 5 -R test-group/alpha --remove-source-branch --rebase --yes",
            "glab mr update 5 --squash-before-merge -R test-group/beta",
            "glab mr merge 5 -R test-group/beta --remove-source-branch --rebase --yes",
        ]
    );
}
