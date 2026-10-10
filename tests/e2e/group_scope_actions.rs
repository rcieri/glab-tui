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
    session.settle(1000);
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
