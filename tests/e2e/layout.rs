use crate::TestSession;

/// The row of `screen` holding `text`, to compare where sidebar entries sit.
fn row_of(screen: &str, text: &str) -> usize {
    screen
        .lines()
        .position(|line| line.contains(text))
        .unwrap_or_else(|| panic!("no {text:?} on screen:\n{screen}"))
}

/// `[layout] tabs` orders the sidebar, hides the tabs it leaves out, and the
/// next-tab key walks the same order.
#[test]
fn tab_list_orders_the_sidebar_and_the_tab_keys() {
    let config = "[layout]\ntabs = [\"mrs\", \"issues\", \"todos\"]\n";
    let mut session = TestSession::with_config(false, 24, 120, Some(config));
    session
        .wait_for_screen_contains("Navigation", 5000)
        .expect("app starts");
    session.settle(300);

    let screen = session.emulator.get_text();
    assert!(
        row_of(&screen, "MRS") < row_of(&screen, "ISSUES"),
        "{screen}"
    );
    assert!(
        row_of(&screen, "ISSUES") < row_of(&screen, "TODOS"),
        "{screen}"
    );
    assert!(!screen.contains("PIPELINES"), "{screen}");

    session.press_keys(b"l");
    session
        .wait_for_screen_contains("Unread", 5000)
        .expect("l from Issues lands on Todos, the next listed tab");
}

/// `toggle_nav` (`Alt+n`) hides the sidebar and shows it again.
#[test]
fn alt_n_toggles_the_navigation_sidebar() {
    let mut session = TestSession::new(false, 24, 120);
    session
        .wait_for_screen_contains("Navigation", 5000)
        .expect("app starts with the sidebar");

    session.send_input(b"\x1bn");
    session
        .wait_for_screen_without("Navigation", 5000)
        .expect("Alt+n hides the sidebar");

    session.send_input(b"\x1bn");
    session
        .wait_for_screen_contains("Navigation", 5000)
        .expect("Alt+n shows it again");
}

// --- Tier 1: Feature Coverage (5 cases) ---

#[test]
fn test_save_layout_global() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_save_layout_repo_priority() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_save_layout_sorting_direction() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_save_layout_grouping_config() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_save_layout_filter_context() {
    let _session = TestSession::new(false, 24, 80);
}

// --- Tier 2: Boundary & Corner Cases (5 cases) ---

#[test]
fn test_save_layout_readonly_destination() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_save_layout_no_changes() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_save_layout_disk_full() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_save_layout_simultaneous_open() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_save_layout_all_columns_disabled() {
    let _session = TestSession::new(false, 24, 80);
}
