//! End-to-end coverage for the Todos tab on GitLab.
//!
//! The mock `glab` serves `tests/fixtures/todos.json` for `todo list` (todo 1
//! for issue #8 of `test-owner/test-repo`, todo 2 for issue #7 of
//! `other-group/other-repo`, todo 3 for MR !2 of `test-owner/test-repo`) and
//! `todos_done.json` for `api todos?state=done`.
//! The Issues tab lists `custom_command_issues.json`: issues #7 and #8 of the
//! repository in scope.

use crate::{Sandbox, TestSession};
use std::time::{Duration, Instant};

const OWN_TODO: &str = "Ship worktree eight";
const FOREIGN_TODO: &str = "Foreign seven";
const DONE_TODO: &str = "Already handled todo";
/// `icons.highlight_arrow` of the Nerd Font set, drawn in front of the
/// selected table row.
const HIGHLIGHT_ARROW: &str = "\u{f44a}";
/// `icons.todo_unread` of the Nerd Font set, drawn in the State badge.
const UNREAD_BADGE: &str = "\u{f0e0} NEW ";

/// Only Issues and Todos stay enabled, so `l` from Issues lands on Todos.
const CONFIG: &str = r#"disabled_tabs = ["mrs", "pipelines", "jobs", "runners", "releases"]"#;

fn session_on_todos(extra: &[(&str, &str)]) -> TestSession {
    launch_on_todos(false, extra, "Worktree for eight", OWN_TODO)
}

/// Launches with only Issues and Todos enabled, waits for `issues_ready` and
/// switches to the Todos tab until it lists `todo`.
fn launch_on_todos(
    is_github: bool,
    extra: &[(&str, &str)],
    issues_ready: &str,
    todo: &str,
) -> TestSession {
    let sandbox = Sandbox::new(is_github).unwrap();
    let conf_dir = sandbox.config_dir.join("glab-tui");
    std::fs::create_dir_all(&conf_dir).unwrap();
    std::fs::write(conf_dir.join("config.toml"), CONFIG).unwrap();
    let mut session = TestSession::launch(sandbox, 30, 140, extra);
    session
        .wait_for_screen_contains(issues_ready, 30000)
        .expect("app should reach the Issues tab");
    session.send_input(b"l");
    session
        .wait_for_screen_contains(todo, 15000)
        .expect("Todos tab should list the pending todos");
    session
}

fn wait_for_screen_without(session: &mut TestSession, gone: &str, timeout_ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        session.settle(50);
        let screen = session.emulator.get_text();
        if !screen.contains(gone) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{gone:?} should have left the screen:\n{screen}"
        );
    }
}

/// Feeds the app's output to the emulator until `call` has been logged; an
/// undrained PTY blocks the app mid-redraw.
fn wait_for_cli_call(session: &mut TestSession, call: &str, timeout_ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while !session.get_cli_calls().contains(call) {
        assert!(
            Instant::now() < deadline,
            "expected CLI call {call:?}, got:\n{}",
            session.get_cli_calls()
        );
        session.settle(20);
    }
}

/// Sends keys one at a time: the app reads a burst of bytes as one sequence.
fn press_keys(session: &mut TestSession, keys: &[u8]) {
    for key in keys {
        session.send_input(&[*key]);
        session.settle(150);
    }
}

#[test]
fn test_space_selects_todos_and_d_marks_every_selected_one_done() {
    let mut session = session_on_todos(&[]);

    press_keys(&mut session, b" j d");

    wait_for_cli_call(&mut session, "glab todo done 1", 5000);
    wait_for_cli_call(&mut session, "glab todo done 2", 5000);
    wait_for_screen_without(&mut session, OWN_TODO, 5000);
    wait_for_screen_without(&mut session, FOREIGN_TODO, 5000);
    let calls = session.get_cli_calls();
    assert!(!calls.contains("glab todo done 3"), "{calls}");
    assert!(session.emulator.get_text().contains("Review pagination"));
}

#[test]
fn test_d_marks_the_selection_even_when_the_search_hides_every_row() {
    let mut session = session_on_todos(&[]);

    press_keys(&mut session, b" j ");
    session.send_input(b"/no such todo\r");
    wait_for_screen_without(&mut session, OWN_TODO, 5000);
    press_keys(&mut session, b"d");

    wait_for_cli_call(&mut session, "glab todo done 1", 5000);
    wait_for_cli_call(&mut session, "glab todo done 2", 5000);
    let calls = session.get_cli_calls();
    assert!(!calls.contains("glab todo done 3"), "{calls}");
}

#[test]
fn test_visual_select_and_enter_mark_the_range_read_without_leaving_the_tab() {
    let mut session = session_on_todos(&[]);

    press_keys(&mut session, b"vj\r");

    wait_for_cli_call(&mut session, "glab todo done 1", 5000);
    wait_for_cli_call(&mut session, "glab todo done 2", 5000);
    wait_for_screen_without(&mut session, OWN_TODO, 5000);
    wait_for_screen_without(&mut session, FOREIGN_TODO, 5000);
    let calls = session.get_cli_calls();
    assert!(!calls.contains("glab todo done 3"), "{calls}");
    let screen = session.emulator.get_text();
    assert!(screen.contains("Review pagination"), "{screen}");
    assert!(!screen.contains("Worktree for eight"), "{screen}");
}

#[test]
fn test_preview_shows_the_todo_comment_as_markdown_with_badges() {
    let mut session = session_on_todos(&[]);
    session
        .wait_for_screen_contains(UNREAD_BADGE, 2000)
        .expect("the State column should show a NEW badge");
    session
        .wait_for_screen_contains(" MR ", 2000)
        .expect("the Type column should show an MR badge");

    press_keys(&mut session, b"p");

    session
        .wait_for_screen_contains("Ping from carol about worktree eight", 5000)
        .expect("the preview should render the mention as Markdown");
    session
        .wait_for_screen_contains("@carol", 2000)
        .expect("the preview should name the author");
}

#[test]
fn test_github_preview_fetches_the_latest_comment() {
    let mut session = launch_on_todos(true, &[], "Issues", "Todo Item 1");

    press_keys(&mut session, b"p");

    wait_for_cli_call(
        &mut session,
        "gh api repos/test-owner/test-repo/issues/comments/77",
        5000,
    );
    session
        .wait_for_screen_contains("Comment from octocat on item one", 5000)
        .expect("the preview should render the fetched comment");
    session
        .wait_for_screen_contains("@octocat", 2000)
        .expect("the preview should name the comment author");
}

#[test]
fn test_todos_list_unread_only_until_read_ones_are_toggled_on() {
    let mut session = session_on_todos(&[]);
    session
        .wait_for_screen_contains("Todos › Unread", 2000)
        .expect("the title should name the unread-only filter");
    assert!(
        !session.get_cli_calls().contains("todos?state=done"),
        "done todos must not be requested by default:\n{}",
        session.get_cli_calls()
    );

    session.send_input(b"r");

    session
        .wait_for_screen_contains(DONE_TODO, 5000)
        .expect("toggling should list the done todos too");
    session
        .wait_for_screen_contains("Todos › All", 2000)
        .expect("the title should name the new filter");
    wait_for_cli_call(&mut session, "glab api todos?state=done", 2000);
}

#[test]
fn test_marking_a_todo_done_drops_it_without_a_refetch() {
    let mut session = session_on_todos(&[]);

    session.send_input(b"d");

    wait_for_screen_without(&mut session, OWN_TODO, 5000);
    assert!(session.emulator.get_text().contains(FOREIGN_TODO));
    let calls = session.get_cli_calls();
    assert!(calls.contains("glab todo done 1"), "{calls}");
    assert_eq!(calls.matches("glab todo list").count(), 1, "{calls}");
}

#[test]
fn test_enter_marks_the_todo_read_and_selects_its_issue_in_the_todo_project() {
    let mut session = session_on_todos(&[]);

    session.send_input(b"\r");

    wait_for_cli_call(&mut session, "glab todo done 1", 5000);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        session.settle(50);
        let screen = session.emulator.get_text();
        let highlighted = screen
            .lines()
            .find(|row| row.contains(HIGHLIGHT_ARROW) && row.contains("Worktree for"));
        if highlighted.is_some_and(|row| row.contains("Worktree for eight")) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Enter should select issue #8 on the Issues tab:\n{screen}"
        );
    }
}

#[test]
fn test_a_todo_outside_the_scope_is_reported_and_stays_unread() {
    let mut session = session_on_todos(&[]);

    session.send_input(b"j");
    std::thread::sleep(Duration::from_millis(150));
    session.send_input(b"\r");

    session
        .wait_for_screen_contains(
            "other-group/other-repo#7 is outside test-owner/test-repo",
            5000,
        )
        .expect("a foreign todo must not land on this project's issue #7");
    // The read mark would be spawned by the same keypress that raised the
    // error, so this window is enough for its CLI call to be logged.
    session.settle(500);
    let calls = session.get_cli_calls();
    assert!(!calls.contains("glab todo done 2"), "{calls}");
    assert!(session.emulator.get_text().contains(FOREIGN_TODO));
}

#[test]
fn test_failed_mark_read_raises_an_error() {
    let mut session = session_on_todos(&[("TEST_GLAB_FAIL_MATCH", "todo done")]);

    session.send_input(b"\r");

    session
        .wait_for_screen_contains("Failed to mark todo as read", 5000)
        .expect("the failed mark-read should surface as an error");
}

#[test]
fn test_unsubscribe_and_mark_all_read_run_their_cli_commands() {
    let mut session = session_on_todos(&[]);

    session.send_input(b"U");
    wait_for_cli_call(
        &mut session,
        "glab issue unsubscribe 8 -R test-owner/test-repo",
        5000,
    );
    assert!(session.emulator.get_text().contains(OWN_TODO));

    session.send_input(b"A");
    session
        .wait_for_screen_contains("Mark All Todos Read", 5000)
        .expect("A should ask before marking everything");
    // The dialog opens on Cancel; move to Submit.
    session.send_input(b"h");
    std::thread::sleep(Duration::from_millis(150));
    session.send_input(b"\r");

    wait_for_cli_call(&mut session, "glab todo done --all", 5000);
    wait_for_screen_without(&mut session, FOREIGN_TODO, 5000);
    wait_for_screen_without(&mut session, OWN_TODO, 5000);
}

#[test]
fn test_github_notification_actions_use_the_threads_api() {
    // The mock `gh` answers every `notifications` call with one unread
    // notification, thread 1.
    let mut session = launch_on_todos(true, &[], "Issues", "Todo Item 1");
    let calls = session.get_cli_calls();
    assert!(calls.contains("gh api notifications\n"), "{calls}");
    assert!(!calls.contains("all=true"), "{calls}");

    session.send_input(b"U");
    wait_for_cli_call(
        &mut session,
        "gh api -X DELETE notifications/threads/1/subscription",
        5000,
    );

    session.send_input(b"d");
    wait_for_cli_call(
        &mut session,
        "gh api -X DELETE notifications/threads/1\n",
        5000,
    );
    wait_for_screen_without(&mut session, "Todo Item 1", 5000);
}
