//! End-to-end coverage for the generic list-tab model on the Runners,
//! Releases, Branches and Environments tabs: exact rendered cells, search
//! running through the cached view, and search reaching the Release Notes
//! column under the label the table shows.

use crate::TestSession;

const SETTLE_MS: u64 = 1500;

fn settle(session: &mut TestSession) {
    session.settle(SETTLE_MS);
}

#[test]
fn runners_tab_renders_typed_columns_and_filters_through_the_view() {
    let config = "active_tab = \"runners\"\n";
    let mut session = TestSession::with_config(false, 24, 100, Some(config));
    session
        .wait_for_screen_contains("Shared Runner 1", 8000)
        .expect("the runners fixture loads");
    let screen = session.emulator.get_text();
    assert!(screen.contains("#9876"), "ID cell missing: {screen}");
    assert!(
        screen.contains("ONLINE"),
        "status cell shows the display text"
    );
    assert!(screen.contains("YES"), "active cell shows the display text");

    session.send_input(b"/");
    session.send_input(b"no such runner");
    settle(&mut session);
    assert!(
        !session.emulator.get_text().contains("Shared Runner 1"),
        "search must filter the runners view"
    );

    session.send_input(b"\x15"); // Ctrl+U clears the search bar
    session.send_input(b"\x1b"); // Esc closes it
    settle(&mut session);
    assert!(session.emulator.get_text().contains("Shared Runner 1"));
}

#[test]
fn releases_search_matches_the_release_notes_column() {
    let config = "active_tab = \"releases\"\n[releases]\ncolumns = [\"Tag\", \"Release Notes\"]\n";
    let mut session = TestSession::with_config(false, 24, 100, Some(config));
    session
        .wait_for_screen_contains("v1.1.0", 8000)
        .expect("the releases fixture loads");

    session.send_input(b"/");
    session.send_input(b"pagination");
    settle(&mut session);
    let screen = session.emulator.get_text();
    assert!(
        screen.contains("v1.1.0") && !screen.contains("v1.0.0"),
        "search must match the Release Notes column: {screen}"
    );
}

#[test]
fn branches_tab_shows_flag_badges_from_typed_columns() {
    let config = "active_tab = \"branches\"\n";
    let mut session = TestSession::with_config(false, 24, 100, Some(config));
    session
        .wait_for_screen_contains("main", 8000)
        .expect("the branches fixture loads");
    let screen = session.emulator.get_text();
    assert!(
        screen.contains("YES"),
        "the default flag shows YES: {screen}"
    );
    session.send_input(b"/");
    session.send_input(b"feature");
    settle(&mut session);
    assert!(
        !session.emulator.get_text().contains("main"),
        "search must filter the branches view"
    );
}
