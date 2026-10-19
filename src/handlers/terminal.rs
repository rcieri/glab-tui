//! Terminal tab actions: filtering the log, inspecting an entry, copying it
//! and running its CLI call again.

use crossterm::event::{KeyCode, KeyEvent};
use tokio::sync::mpsc::UnboundedSender;

use crate::app::App;
use crate::command_log::Invocation;
use crate::event::Event;
use crate::keybinding::keybinding_matches;

/// Keys of the Terminal tab's log view. Returns false for a key it does not
/// handle.
pub fn handle_tab_key(app: &mut App, key_event: &KeyEvent) -> bool {
    let terminal = &app.config.keybindings.terminal;
    if keybinding_matches(&terminal.toggle_wrap, key_event) {
        app.terminal_wrap = !app.terminal_wrap;
        app.terminal_scroll = 0;
    } else if keybinding_matches(&terminal.clear_log, key_event) {
        app.clear_terminal_log();
    } else if keybinding_matches(&terminal.cycle_status_filter, key_event) {
        app.cycle_terminal_status_filter();
    } else if key_event.code == KeyCode::Char('P')
        || keybinding_matches(&terminal.inspect_command, key_event)
    {
        if let Err(error) = app.open_terminal_inspector() {
            app.raise_error_toast(format!("Cannot inspect command: {error}"));
        }
    } else {
        return handle_entry_action(app, key_event);
    }
    true
}

/// Keys of the command inspector. The re-run confirmation it opens is drawn
/// on top and takes its keys first. A selector or text input opened over it,
/// such as the repo switcher, reads its keys later in the main loop, so the
/// inspector leaves them alone.
pub fn handle_inspector_key(app: &mut App, key_event: &KeyEvent) -> bool {
    if app.selector.is_some() || app.text_input.is_some() {
        return false;
    }
    let Some(mut inspector) = app.terminal_inspector else {
        return false;
    };
    let terminal = &app.config.keybindings.terminal;
    if matches!(
        key_event.code,
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('P')
    ) || keybinding_matches(&terminal.inspect_command, key_event)
    {
        app.terminal_inspector = None;
        return true;
    }
    if handle_entry_action(app, key_event) {
        return true;
    }

    let global = &app.config.keybindings.global;
    let page = inspector.page_height.max(1);
    let half_page = (page / 2).max(1);
    let scroll = inspector.scroll;
    inspector.scroll = match key_event.code {
        _ if keybinding_matches(&global.scroll_half_page_down, key_event) => {
            scroll.saturating_add(half_page)
        }
        _ if keybinding_matches(&global.scroll_half_page_up, key_event) => {
            scroll.saturating_sub(half_page)
        }
        _ if keybinding_matches(&global.scroll_page_down, key_event) => scroll.saturating_add(page),
        _ if keybinding_matches(&global.scroll_page_up, key_event) => scroll.saturating_sub(page),
        _ if keybinding_matches(&global.scroll_down, key_event) => scroll.saturating_add(1),
        _ if keybinding_matches(&global.scroll_up, key_event) => scroll.saturating_sub(1),
        _ if keybinding_matches(&global.scroll_top, key_event) => 0,
        // The render clamps it to the last line.
        _ if keybinding_matches(&global.scroll_to_end, key_event) => usize::MAX,
        KeyCode::Char('j') | KeyCode::Down => scroll.saturating_add(1),
        KeyCode::Char('k') | KeyCode::Up => scroll.saturating_sub(1),
        KeyCode::Char('g') | KeyCode::Home => 0,
        KeyCode::Char('G') | KeyCode::End => usize::MAX,
        _ => scroll,
    };
    app.terminal_inspector = Some(inspector);
    true
}

/// Copy and re-run, which act on the inspected entry while the inspector is
/// open and on the highlighted one otherwise.
fn handle_entry_action(app: &mut App, key_event: &KeyEvent) -> bool {
    let terminal = &app.config.keybindings.terminal;
    let (result, failure) = if keybinding_matches(&terminal.copy_command, key_event) {
        (app.copy_terminal_command(), "Failed to copy command")
    } else if keybinding_matches(&terminal.copy_output, key_event) {
        (app.copy_terminal_output(), "Failed to copy output")
    } else if keybinding_matches(&terminal.rerun_command, key_event) {
        (app.confirm_terminal_rerun(), "Cannot re-run command")
    } else {
        return false;
    };
    if let Err(error) = result {
        app.raise_error_toast(format!("{failure}: {error}"));
    }
    true
}

/// Runs a confirmed re-run in the background. Its entry is logged as
/// running now and settled by `Event::CommandRerun`.
pub fn rerun(app: &mut App, invocation: Invocation, tx: UnboundedSender<Event>) {
    let row = app.start_rerun(&invocation);
    tokio::spawn(async move {
        let (status, execution) = crate::backend::logged_command::rerun(invocation).await;
        let _ = tx.send(Event::CommandRerun {
            row,
            status,
            execution: Box::new(execution),
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn inspector_leaves_keys_to_a_selector_opened_over_it() {
        let mut app = App::default();
        app.start_command("Creating branch: x from main".to_string());
        app.open_terminal_inspector().unwrap();
        app.selector = Some(crate::app::Selector::draft_actions(0));

        for code in [KeyCode::Char('j'), KeyCode::Enter, KeyCode::Esc] {
            let key = KeyEvent::new(code, KeyModifiers::NONE);
            assert!(!handle_inspector_key(&mut app, &key), "{code:?}");
        }
        assert_eq!(
            app.terminal_inspector.map(|inspector| inspector.scroll),
            Some(0)
        );
    }

    #[test]
    fn inspector_scrolls_with_remapped_global_scroll_keys() {
        let mut app = App::default();
        app.start_command("Creating branch: x from main".to_string());
        app.open_terminal_inspector().unwrap();
        app.config.keybindings.global.scroll_down = "z".to_string();
        app.config.keybindings.global.scroll_to_end = "x".to_string();
        let scroll_after = |app: &mut App, key: char| {
            handle_inspector_key(app, &KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
            app.terminal_inspector.map(|inspector| inspector.scroll)
        };

        assert_eq!(scroll_after(&mut app, 'z'), Some(1));
        assert_eq!(scroll_after(&mut app, 'x'), Some(usize::MAX));
    }
}
