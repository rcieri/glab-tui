//! The REST mutations the pinned API descriptions flag, driven through the
//! app: cancelling one GitHub job, and creating a branch on either host. The
//! gh mock fails the job cancel as GitHub does (#781), and the mocks remember
//! a created branch (`created_branch` next to the call log), so each test also
//! checks what the user then sees.

use crate::TestSession;
use std::time::{Duration, Instant};

const ASCII_ICONS: &str = "icons = \"ascii\"\n";
const JOB_STATUS_CANCELED: &str = "CANCEL";
const JOB_STATUS_SUCCESS: &str = "SUCCESS";
/// The call that lists the run's jobs, before and after the cancel.
const JOBS_FETCH: &str = "run view 12345 --json jobs";

/// The request bodies `gh api --input -` received, one per line.
fn request_bodies(session: &TestSession) -> String {
    std::fs::read_to_string(format!("{}.stdin", session.sandbox.log_path.display()))
        .unwrap_or_default()
}

fn assert_job_shows_success(session: &TestSession, what: &str) {
    let screen = session.emulator.get_text();
    let row = screen.lines().find(|row| row.contains("#54321"));
    assert!(
        row.is_some_and(
            |row| row.contains(JOB_STATUS_SUCCESS) && !row.contains(JOB_STATUS_CANCELED)
        ),
        "{what}:\n{screen}"
    );
}

/// Waits for a branch list row naming `name`. The terminal log line of the
/// create call names the branch too, as part of the `api` command.
fn wait_for_branch_row(session: &mut TestSession, name: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let screen = session.emulator.get_text();
        if screen
            .lines()
            .any(|row| row.contains(name) && !row.contains(" api "))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the refetched branch list shows no row for {name}:\n{screen}"
        );
        session.settle(100);
    }
}

fn session_on_branches(is_github: bool) -> TestSession {
    let mut session = TestSession::with_config_and_args(
        is_github,
        24,
        140,
        Some(ASCII_ICONS),
        &["--tab", "branches"],
    );
    session
        .wait_for_screen_contains("feature/pagination", 30000)
        .expect("the branches are listed");
    session
}

/// Creates `name` from the highlighted branch through the Create Branch form.
fn create_branch(session: &mut TestSession, name: &str) {
    session.press_keys(b"n");
    session
        .wait_for_screen_contains("Create Branch", 5000)
        .expect("n opens the create branch form");
    session.press_keys(b"\r");
    session.press_keys(name.as_bytes());
    session.press_keys(b"\x18"); // Ctrl+X submits the form
}

#[test]
fn github_job_cancel_posts_to_an_endpoint_github_lacks_and_the_row_reverts() {
    let mut session = TestSession::with_config(true, 40, 140, Some(ASCII_ICONS));
    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app starts");
    session.press_keys(b"ll");
    session
        .wait_for_screen_contains("12345", 15000)
        .expect("the run is listed");
    session.send_input(b"\x1bj"); // Alt+j: the run's own jobs
    session
        .wait_for_screen_contains("rspec", 15000)
        .expect("the run's job is listed");
    session.press_keys(b"c");
    session
        .wait_for_screen_contains("Cancel", 5000)
        .expect("cancel asks for confirmation");
    let fetches_before_cancel = session.get_cli_calls().matches(JOBS_FETCH).count();
    session.press_keys(b"h\r");
    session
        .wait_for_cli_call(
            "gh api -X POST /repos/test-owner/test-repo/actions/jobs/54321/cancel\n",
            5000,
        )
        .expect("confirming posts to the job's own cancel endpoint");
    session
        .wait_for_screen_contains("Failed to cancel job #54321", 5000)
        .expect("the failed cancel is reported");
    // The list is refetched a second after the cancel, so the row shown with
    // the error is the one the app restored.
    assert_eq!(
        session.get_cli_calls().matches(JOBS_FETCH).count(),
        fetches_before_cancel,
        "the job list was refetched before the restored row could be checked"
    );
    assert_job_shows_success(&session, "the failed cancel restores the row");
    let deadline = Instant::now() + Duration::from_secs(10);
    while session.get_cli_calls().matches(JOBS_FETCH).count() <= fetches_before_cancel {
        assert!(Instant::now() < deadline, "the job list is not refetched");
        session.settle(100);
    }
    assert_job_shows_success(&session, "the refetched job list shows the job unchanged");
}

#[test]
fn github_branch_create_resolves_the_base_then_creates_the_ref() {
    let mut session = session_on_branches(true);
    create_branch(&mut session, "feature/created");
    session
        .wait_for_cli_call(
            "gh api /repos/test-owner/test-repo/git/refs/heads/main\n",
            5000,
        )
        .expect("the base branch is resolved to its commit");
    session
        .wait_for_cli_call(
            "gh api -X POST --input - /repos/test-owner/test-repo/git/refs\n",
            5000,
        )
        .expect("the new ref is created");
    wait_for_branch_row(&mut session, "feature/created");
    let bodies = request_bodies(&session);
    assert!(
        bodies.contains(r#""ref":"refs/heads/feature/created""#)
            && bodies.contains(r#""sha":"a1b2c3d4e5f67890123456789012345678901234""#),
        "the ref is created at the base branch's commit:\n{bodies}"
    );
}

#[test]
fn gitlab_branch_create_sends_branch_and_ref_as_query_parameters() {
    let mut session = session_on_branches(false);
    create_branch(&mut session, "feature-created");
    session
        .wait_for_cli_call(
            "glab api -X POST /projects/test-owner%2Ftest-repo/repository/branches?branch=feature-created&ref=main\n",
            5000,
        )
        .expect("the branch is created from the highlighted one");
    wait_for_branch_row(&mut session, "feature-created");
}
