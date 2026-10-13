use crate::TestSession;

/// The group-scope placeholder names the switch-repo key the user actually
/// configured, and the tab does not try to list runners it can never show:
/// that fetch failed on every refresh and raised an error toast.
#[test]
fn test_runners_in_group_scope_name_the_configured_switch_key_without_fetching() {
    let config = "[keybindings.global]\nswitch_repo = \"Ctrl+o\"\n";
    let mut session = TestSession::with_config_and_args(
        false,
        30,
        160,
        Some(config),
        &["--group", "test-owner", "--tab", "runners"],
    );

    session
        .wait_for_screen_contains("Runners tab is not available in group scope", 10_000)
        .unwrap();
    session.settle(1_500);

    let screen = session.emulator.get_text();
    assert!(
        screen.contains("Press `Ctrl+o` to switch to a repository scope"),
        "{screen}"
    );
    assert!(!screen.contains("Failed to fetch runners"), "{screen}");
}

/// GitHub has no API to pause, resume or rename a runner. The keys must not
/// pretend otherwise (flipping the row to PAUSED and reverting it with an
/// error toast, or opening a description editor), and the help must not list
/// them.
#[test]
fn test_github_runners_tab_does_not_offer_pause_resume_or_edit() {
    let mut session = TestSession::with_config_and_args(true, 60, 160, None, &["--tab", "runners"]);

    session
        .wait_for_screen_contains("Shared Runner 1", 10_000)
        .unwrap();
    session.send_input(b"p");
    session.settle(500);
    session.send_input(b"r");
    session.settle(500);
    session.send_input(b"e");
    session.settle(1_500);

    let screen = session.emulator.get_text();
    let row = screen
        .lines()
        .find(|line| line.contains("#9876"))
        .unwrap_or_default();
    assert!(row.contains("ONLINE") && row.contains("YES"), "{screen}");
    assert!(!row.contains("PAUSED"), "{screen}");
    assert!(!screen.contains("not supported"), "{screen}");
    assert!(!screen.contains("Edit Runner Description"), "{screen}");

    session.send_input(b"?");
    session
        .wait_for_screen_contains("Open runner in browser", 3_000)
        .unwrap();
    let help = session.emulator.get_text();
    assert!(!help.contains("Pause / Resume runner"), "{help}");
    assert!(!help.contains("Edit runner description text"), "{help}");
}
