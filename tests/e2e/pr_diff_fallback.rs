//! End-to-end coverage for building a GitHub PR diff from a local clone when
//! `gh pr diff` fails with GitHub's 20,000-line `too_large` error.
//!
//! The mock `gh` serves `tests/fixtures/gh_prs.json` for `pr list`,
//! `gh_pr_comments.json` for the review comments, fails `pr diff` as
//! `MOCK_GH_PR_DIFF_ERROR` says, and answers the commit-id lookup with
//! `MOCK_GH_BASE_OID` / `MOCK_GH_HEAD_OID`. The sandbox repository is the
//! local clone: glab-tui registers it on startup, and its `origin` names the
//! PR's repository.

use crate::{Sandbox, TestSession};
use std::path::Path;

const PR_TITLE: &str = "Rewrite the monorepo build";
const PR_FILES: [&str; 4] = ["app.rs", "new.rs", "old.md", "build.sh"];
/// Marker the diff pane draws in front of the cursor row.
const CURSOR_MARKER: &str = "❯";

/// Runs git in `dir` with an identity and no signing, so the developer's
/// global config cannot break commit creation.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=glab-tui",
            "-c",
            "user.email=glab-tui@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn write(repo: &Path, path: &str, content: &str) {
    let file = repo.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, content).unwrap();
}

/// Commits the PR's base on `main` and its head on `rewrite-build`, then
/// returns to `main` with a staged and an unstaged local edit, the way a
/// clone in daily use looks. Returns the base and head commit ids.
fn commit_pr(repo: &Path) -> (String, String) {
    git(repo, &["checkout", "-q", "-b", "main"]);
    write(repo, "build.sh", "#!/bin/sh\nmake all\n");
    write(repo, "tools/old.md", "# Old build\n");
    write(repo, "src/app.rs", "fn main() {}\n");
    write(repo, "notes.txt", "local notes\n");
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "base"]);
    let base = git(repo, &["rev-parse", "HEAD"]).trim().to_string();

    git(repo, &["checkout", "-q", "-b", "rewrite-build"]);
    write(
        repo,
        "build.sh",
        "#!/bin/sh\ncargo build --release\ncargo test\n",
    );
    std::fs::remove_file(repo.join("tools/old.md")).unwrap();
    write(repo, "src/app.rs", "fn main() {\n    run();\n}\n");
    write(repo, "src/new.rs", "pub fn run() {}\n");
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", "head"]);
    let head = git(repo, &["rev-parse", "HEAD"]).trim().to_string();

    git(repo, &["checkout", "-q", "main"]);
    write(repo, "notes.txt", "staged local notes\n");
    git(repo, &["add", "notes.txt"]);
    write(repo, "src/app.rs", "fn main() { /* unstaged */ }\n");
    (base, head)
}

/// Everything the fallback promises not to touch in the clone.
fn clone_state(repo: &Path) -> [String; 5] {
    [
        git(repo, &["symbolic-ref", "HEAD"]),
        git(repo, &["for-each-ref"]),
        git(repo, &["status", "--porcelain=v1"]),
        git(repo, &["diff", "--cached"]),
        git(repo, &["diff"]),
    ]
}

/// Sends keys one at a time (the app reads a burst of bytes as one sequence)
/// and keeps draining the app's output in between: a PTY whose buffer fills
/// up blocks the app's redraw, and with it the event loop.
fn press_keys(session: &mut TestSession, keys: &[u8]) {
    for key in keys {
        session.send_input(&[*key]);
        session.settle(150);
    }
}

/// Launch, switch to the PRs tab and press `D` on the fixture PR.
fn open_pr_diff(session: &mut TestSession) {
    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app should reach the Issues tab");
    session.send_input(b"l");
    session
        .wait_for_screen_contains(PR_TITLE, 15000)
        .expect("PRs tab should list the fixture PR");
    session.send_input(b"D");
}

#[test]
fn too_large_pr_diff_is_built_from_the_local_clone() {
    let sandbox = Sandbox::new(true).unwrap();
    let repo = sandbox.repo_dir.clone();
    let (base, head) = commit_pr(&repo);
    let before = clone_state(&repo);
    let mut session = TestSession::launch(
        sandbox,
        40,
        140,
        &[
            ("MOCK_GH_PR_DIFF_ERROR", "too_large"),
            ("MOCK_GH_BASE_OID", &base),
            ("MOCK_GH_HEAD_OID", &head),
            // Any fetch from the fake github.com origin must fail fast: the
            // clone already holds both commits, so none should happen.
            ("GIT_SSH_COMMAND", "false"),
        ],
    );

    open_pr_diff(&mut session);
    session
        .wait_for_screen_contains("Pull Request Diff #1", 15000)
        .expect("D should open the diff built from the local clone");
    for file in PR_FILES {
        session
            .wait_for_screen_contains(file, 2000)
            .unwrap_or_else(|e| panic!("the file tree should list {file}: {e}"));
    }
    session
        .wait_for_screen_contains("Existing note on the run call", 5000)
        .expect("existing review comments should load beside the fallback diff");

    session.send_input(b"\t");
    move_cursor_to(&mut session, "run();");
    press_keys(&mut session, b"c");
    session
        .wait_for_screen_contains("Add Comment to src/app.rs", 5000)
        .expect("c should open the comment input on the cursor line");
    press_keys(&mut session, b"Call site\r");
    session
        .wait_for_screen_contains("REVIEW MODE: ON (1 pending)", 5000)
        .expect("the inline comment should be kept as a draft");
    press_keys(&mut session, b"r");
    session
        .wait_for_screen_contains("Submit Pull Request Review", 5000)
        .expect("r should offer the review outcomes");
    press_keys(&mut session, b"jj\r");
    session
        .wait_for_screen_contains("Submit Review (Comment)", 5000)
        .expect("choosing Comment should ask for the review summary");
    press_keys(&mut session, b"Done\r");

    let payload = wait_for_review_payload(&mut session);
    assert_eq!(
        payload,
        r#"{"body":"Done","comments":[{"body":"Call site","line":2,"path":"src/app.rs","side":"RIGHT"}],"event":"COMMENT"}"#
    );
    assert_eq!(clone_state(&repo), before);
    let calls = session.get_cli_calls();
    assert!(
        calls.contains("gh pr view 1 -R test-owner/test-repo --json baseRefOid,headRefOid"),
        "the fallback should look up the PR's commits: {calls}"
    );
}

/// Presses `j` until the diff cursor sits on the row containing `code`.
fn move_cursor_to(session: &mut TestSession, code: &str) {
    for _ in 0..20 {
        session.settle(150);
        let screen = session.emulator.get_text();
        if screen
            .lines()
            .any(|row| row.contains(CURSOR_MARKER) && row.contains(code))
        {
            return;
        }
        session.send_input(b"j");
    }
    panic!(
        "the cursor never reached {code:?}:\n{}",
        session.emulator.get_text()
    );
}

/// The review payload the mock `gh` logged when the review was submitted.
fn wait_for_review_payload(session: &mut TestSession) -> String {
    const PREFIX: &str = "review payload: ";
    for _ in 0..200 {
        if let Some(line) = session
            .get_cli_calls()
            .lines()
            .find_map(|line| line.strip_prefix(PREFIX))
        {
            return line.to_string();
        }
        session.settle(50);
    }
    panic!(
        "no review was submitted: {}\nscreen:\n{}",
        session.get_cli_calls(),
        session.emulator.get_text()
    );
}

#[test]
fn too_large_pr_diff_without_a_local_clone_says_a_clone_is_needed() {
    let sandbox = Sandbox::new(true).unwrap();
    let repo = sandbox.repo_dir.clone();
    let mut session =
        TestSession::launch(sandbox, 40, 140, &[("MOCK_GH_PR_DIFF_ERROR", "too_large")]);
    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app should reach the Issues tab");
    // The only recently used repository stops being a clone of the PR's
    // repository, so no local clone is known.
    git(
        &repo,
        &[
            "remote",
            "set-url",
            "origin",
            "git@github.com:someone-else/other-repo.git",
        ],
    );

    open_pr_diff(&mut session);
    session
        .wait_for_screen_contains("over GitHub's 20,000-line limit", 15000)
        .expect("the toast should name the size limit");
    session
        .wait_for_screen_contains("local clone", 2000)
        .expect("the toast should say a local clone is needed");
    let screen = session.emulator.get_text();
    assert!(
        !screen.contains("HTTP 406"),
        "the raw 406 should not reach the toast:\n{screen}"
    );
    assert!(
        !session.get_cli_calls().contains("baseRefOid"),
        "without a clone there is nothing to diff in"
    );
}

#[test]
fn other_pr_diff_errors_skip_the_local_fallback() {
    let mut session =
        TestSession::with_envs(true, 40, 140, &[("MOCK_GH_PR_DIFF_ERROR", "not_found")]);

    open_pr_diff(&mut session);
    session
        .wait_for_screen_contains("Could not resolve to a PullRequest", 15000)
        .expect("the toast should show gh's own error, as before");
    assert!(
        !session.get_cli_calls().contains("baseRefOid"),
        "only the too_large error may start the local fallback"
    );
}

#[test]
fn pressing_d_again_while_the_diff_loads_fetches_it_once() {
    let mut session = TestSession::with_envs(true, 40, 140, &[]);
    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app should reach the Issues tab");
    session.send_input(b"l");
    session
        .wait_for_screen_contains(PR_TITLE, 15000)
        .expect("PRs tab should list the fixture PR");

    // One write: the second `D` reaches the app before the first fetch ends.
    session.send_input(b"DD");
    session
        .wait_for_screen_contains("Pull Request Diff #1", 15000)
        .expect("D should open the diff");
    session.settle(1000);

    let diff_calls = session
        .get_cli_calls()
        .lines()
        .filter(|line| line.starts_with("gh pr diff "))
        .count();
    assert_eq!(diff_calls, 1);
}
