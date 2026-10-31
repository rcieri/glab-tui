use crate::TestSession;

const FAILURE: &str = "Failed to load options: glab command failed: mock: 404 Not Found";

fn rows_with<'a>(screen: &'a str, text: &str) -> Vec<&'a str> {
    screen.lines().filter(|row| row.contains(text)).collect()
}

/// A selector whose option fetch fails reports it inside the selector, where
/// the user is looking, instead of in a toast under it. Once the selector is
/// closed, the same error falls back to the toast.
#[test]
fn test_failed_selector_fetch_is_reported_inside_the_selector() {
    let mut session =
        TestSession::with_envs(false, 30, 140, &[("TEST_GLAB_FAIL_MATCH", "label list")]);
    session
        .wait_for_screen_contains("Worktree for seven", 15000)
        .expect("issues listed");
    session.press_keys(b"n");
    session
        .wait_for_screen_contains("Create Issue", 5000)
        .expect("n opens the create form");
    session.press_keys(b"jjj\r");
    session
        .wait_for_screen_contains("Select Labels", 5000)
        .expect("Enter on Labels opens its selector");
    session
        .wait_for_screen_contains(FAILURE, 5000)
        .expect("the failed label fetch is reported");

    let screen = session.emulator.get_text();
    let rows = rows_with(&screen, FAILURE);
    assert_eq!(rows.len(), 1, "drawn once, not also as a toast:\n{screen}");
    assert_eq!(
        rows[0].matches('║').count(),
        2,
        "the error sits inside the selector frame:\n{screen}"
    );

    session.send_input(b"\x1b");
    session.settle(300);
    let screen = session.emulator.get_text();
    assert!(!screen.contains("Select Labels"), "Esc closes the selector");
    let rows = rows_with(&screen, FAILURE);
    assert_eq!(rows.len(), 1, "the error moves to the toast:\n{screen}");
    assert!(!rows[0].contains('║'), "no longer framed by the selector");
}
