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
    assert!(!session.get_cli_calls().contains("runner list"));
}

/// GitHub has no API to pause, resume or rename a runner. The keys must not
/// pretend otherwise: flipping the row to PAUSED and then reverting it with an
/// error toast is the behaviour this guards against.
#[test]
fn test_github_runner_pause_key_is_not_offered() {
    let mut session = TestSession::with_config_and_args(true, 30, 160, None, &["--tab", "runners"]);

    session
        .wait_for_screen_contains("Shared Runner 1", 10_000)
        .unwrap();
    session.send_input(b"p");
    session.settle(1_500);

    let screen = session.emulator.get_text();
    assert!(screen.contains("ONLINE"), "{screen}");
    assert!(screen.contains("YES"), "{screen}");
    assert!(!screen.contains("PAUSED"), "{screen}");
    assert!(!screen.contains("not supported"), "{screen}");
}
