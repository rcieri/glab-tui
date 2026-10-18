use crate::{Sandbox, TestSession};

/// In group scope two projects can each have issue/MR #5. Confirming an
/// action on the second row must reach that row's project, not the first
/// project listed with the same iid.
fn confirm_on_second_row(tab_keys: &[u8], first_title: &str, close_key: &[u8]) -> TestSession {
    let mut session = TestSession::launch_with_args(
        Sandbox::new(false).unwrap(),
        30,
        140,
        &["-g", "test-group"],
        &[],
    );
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");
    session.send_input(tab_keys);
    session
        .wait_for_screen_contains(first_title, 5000)
        .expect("both rows are listed");

    session.send_input(b"j");
    session.settle(200);
    session.send_input(close_key);
    session
        .wait_for_screen_contains("Close", 5000)
        .expect("the confirm dialog opens");
    // Destructive dialogs open on Cancel; move to Submit and confirm.
    session.send_input(b"h");
    session.send_input(b"\r");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !session.get_cli_calls().contains(" close 5 -R ") {
        assert!(
            std::time::Instant::now() < deadline,
            "no close call; calls:\n{}",
            session.get_cli_calls()
        );
        session.settle(20);
    }
    session
}

#[test]
fn closing_an_issue_in_group_scope_targets_the_selected_rows_project() {
    let session = confirm_on_second_row(b"", "Alpha crash", b"c");
    let calls = session.get_cli_calls();
    assert!(
        calls.contains("glab issue close 5 -R test-group/beta"),
        "close goes to the selected row's project; calls:\n{calls}"
    );
    assert!(
        !calls.contains("glab issue close 5 -R test-group/alpha"),
        "the other project's issue #5 is untouched; calls:\n{calls}"
    );
}

#[test]
fn closing_an_mr_in_group_scope_targets_the_selected_rows_project() {
    let session = confirm_on_second_row(b"l", "Alpha feature", b"c");
    let calls = session.get_cli_calls();
    assert!(
        calls.contains("glab mr close 5 -R test-group/beta"),
        "close goes to the selected row's project; calls:\n{calls}"
    );
    assert!(
        !calls.contains("glab mr close 5 -R test-group/alpha"),
        "the other project's MR !5 is untouched; calls:\n{calls}"
    );
}

#[test]
fn own_jobs_key_in_group_scope_fetches_from_the_rows_project() {
    let mut session = TestSession::launch_with_args(
        Sandbox::new(false).unwrap(),
        30,
        140,
        &["-g", "test-group"],
        &[],
    );
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");
    session.send_input(b"ll");
    session
        .wait_for_screen_contains("7002", 5000)
        .expect("the group pipeline is listed");
    // Highlighting a pipeline already fetches its jobs for the preview, so
    // only the calls made after Alt+j count.
    session.settle(1000);
    let row_project_jobs = "projects/test-group%2Fbeta/pipelines/7002/jobs";
    let before = session.get_cli_calls().matches(row_project_jobs).count();

    session.send_input(b"\x1bj"); // Alt+j
    session.settle(1500);

    let calls = session.get_cli_calls();
    let after = calls.matches(row_project_jobs).count();
    assert!(
        after > before,
        "Alt+j fetches jobs from the pipeline's project (before {before}, after {after}); calls:\n{calls}"
    );
    assert!(
        !calls.contains("projects/test-group/pipelines/7002/jobs"),
        "the group path is not a project; calls:\n{calls}"
    );
}
