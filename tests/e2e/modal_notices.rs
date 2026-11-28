use crate::TestSession;

const FAILURE: &str = "Failed to load options: glab command failed: mock: 404 Not Found";

fn rows_with<'a>(screen: &'a str, text: &str) -> Vec<&'a str> {
    screen.lines().filter(|row| row.contains(text)).collect()
}

/// The column of the open selector's left border, read from its title row.
fn selector_left_column(screen: &str) -> usize {
    screen
        .lines()
        .find_map(|line| {
            let byte = line.find("┌ Select Labels")?;
            Some(line[..byte].chars().count())
        })
        .expect("the selector's title row is drawn")
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
    let column = selector_left_column(&screen);
    assert_eq!(
        rows[0].chars().nth(column),
        Some('│'),
        "the error sits inside the selector frame:\n{screen}"
    );

    session.send_input(b"\x1b");
    session
        .wait_for_screen_without("Select Labels", 5000)
        .expect("Esc closes the selector");
    let screen = session.emulator.get_text();
    let rows = rows_with(&screen, FAILURE);
    assert_eq!(rows.len(), 1, "the error moves to the toast:\n{screen}");
    assert_ne!(
        rows[0].chars().nth(column),
        Some('│'),
        "no longer framed by the selector"
    );
}

/// The selector keeps reporting its failure for as long as it is open, past
/// the toast lifetime, and the error does not come back as a toast once the
/// user has read it in the selector and closed it.
#[test]
fn test_selector_failure_stays_while_the_selector_is_open() {
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
        .wait_for_screen_contains(FAILURE, 5000)
        .expect("the failed label fetch is reported");

    session.settle(5500);
    let screen = session.emulator.get_text();
    let rows = rows_with(&screen, FAILURE);
    assert_eq!(rows.len(), 1, "still reported after 5 seconds:\n{screen}");
    assert_eq!(
        rows[0].chars().nth(selector_left_column(&screen)),
        Some('│'),
        "inside the selector:\n{screen}"
    );

    session.send_input(b"\x1b");
    session
        .wait_for_screen_without("Select Labels", 5000)
        .expect("Esc closes the selector");
    let screen = session.emulator.get_text();
    assert!(
        !screen.contains(FAILURE),
        "an error already read in the selector is not toasted again:\n{screen}"
    );
}
