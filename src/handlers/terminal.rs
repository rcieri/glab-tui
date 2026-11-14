//! Terminal tab actions: filtering the log, inspecting an entry, copying it
//! and running its CLI call again.

use tokio::sync::mpsc::UnboundedSender;

use crate::app::App;
use crate::command_log::Invocation;
use crate::effect::Effects;
use crate::event::Event;
use crate::keybinding::KeyPress;
use crate::keymap::Action;

/// Keys of the Terminal tab's log view. Returns false for a key it does not
/// handle.
pub fn handle_tab_key(app: &mut App, key_event: &KeyPress) -> bool {
    if app
        .config
        .keybindings
        .matches(Action::TerminalToggleWrap, key_event)
    {
        app.terminal_wrap = !app.terminal_wrap;
        app.terminal_scroll = 0;
    } else if app
        .config
        .keybindings
        .matches(Action::TerminalClearLog, key_event)
    {
        app.clear_terminal_log();
    } else if app
        .config
        .keybindings
        .matches(Action::TerminalCycleStatusFilter, key_event)
    {
        app.cycle_terminal_status_filter();
    } else if app
        .config
        .keybindings
        .matches(Action::TerminalInspectCommand, key_event)
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
pub fn handle_inspector_key(app: &mut App, key_event: &KeyPress) -> bool {
    if app.selector().is_some() || app.text_input().is_some() {
        return false;
    }
    let Some(mut inspector) = app.terminal_inspector else {
        return false;
    };
    if [
        Action::ListBack,
        Action::GlobalQuit,
        Action::TerminalInspectCommand,
    ]
    .into_iter()
    .any(|action| app.config.keybindings.matches(action, key_event))
    {
        app.terminal_inspector = None;
        return true;
    }
    if handle_entry_action(app, key_event) {
        return true;
    }

    let page = inspector.page_height.max(1);
    let half_page = (page / 2).max(1);
    let scroll = inspector.scroll;
    inspector.scroll = match key_event.code {
        _ if app
            .config
            .keybindings
            .matches(Action::GlobalScrollHalfPageDown, key_event) =>
        {
            scroll.saturating_add(half_page)
        }
        _ if app
            .config
            .keybindings
            .matches(Action::GlobalScrollHalfPageUp, key_event) =>
        {
            scroll.saturating_sub(half_page)
        }
        _ if app
            .config
            .keybindings
            .matches(Action::GlobalScrollPageDown, key_event) =>
        {
            scroll.saturating_add(page)
        }
        _ if app
            .config
            .keybindings
            .matches(Action::GlobalScrollPageUp, key_event) =>
        {
            scroll.saturating_sub(page)
        }
        _ if app
            .config
            .keybindings
            .matches(Action::GlobalScrollDown, key_event) =>
        {
            scroll.saturating_add(1)
        }
        _ if app
            .config
            .keybindings
            .matches(Action::GlobalScrollUp, key_event) =>
        {
            scroll.saturating_sub(1)
        }
        _ if app
            .config
            .keybindings
            .matches(Action::GlobalScrollTop, key_event) =>
        {
            0
        }
        // The render clamps it to the last line.
        _ if app
            .config
            .keybindings
            .matches(Action::GlobalScrollToEnd, key_event) =>
        {
            usize::MAX
        }
        _ if app
            .config
            .keybindings
            .matches(Action::ListNextRow, key_event) =>
        {
            scroll.saturating_add(1)
        }
        _ if app
            .config
            .keybindings
            .matches(Action::ListPreviousRow, key_event) =>
        {
            scroll.saturating_sub(1)
        }
        _ if app
            .config
            .keybindings
            .matches(Action::ListFirstRow, key_event) =>
        {
            0
        }
        _ if app
            .config
            .keybindings
            .matches(Action::ListLastRow, key_event) =>
        {
            usize::MAX
        }
        _ => scroll,
    };
    app.terminal_inspector = Some(inspector);
    true
}

/// Copy and re-run, which act on the inspected entry while the inspector is
/// open and on the highlighted one otherwise.
fn handle_entry_action(app: &mut App, key_event: &KeyPress) -> bool {
    let (result, failure) = if app
        .config
        .keybindings
        .matches(Action::TerminalCopyCommand, key_event)
    {
        (app.copy_terminal_command(), "Failed to copy command")
    } else if app
        .config
        .keybindings
        .matches(Action::TerminalCopyOutput, key_event)
    {
        (app.copy_terminal_output(), "Failed to copy output")
    } else if app
        .config
        .keybindings
        .matches(Action::TerminalRerunCommand, key_event)
    {
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
pub fn rerun(app: &mut App, invocation: Invocation, tx: UnboundedSender<Event>, fx: &mut Effects) {
    let row = app.start_rerun(&invocation);
    fx.spawn(async move {
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
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn inspector_leaves_keys_to_a_selector_opened_over_it() {
        let mut app = App::default();
        app.start_command("Creating branch: x from main".to_string());
        app.open_terminal_inspector().unwrap();
        app.open_selector(crate::app::Selector::draft_actions(0));

        for code in [KeyCode::Char('j'), KeyCode::Enter, KeyCode::Esc] {
            let key = KeyEvent::new(code, KeyModifiers::NONE).into();
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
        app.config.keybindings.set(Action::GlobalScrollDown, &["z"]);
        app.config
            .keybindings
            .set(Action::GlobalScrollToEnd, &["x"]);
        let scroll_after = |app: &mut App, key: char| {
            handle_inspector_key(
                app,
                &KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE).into(),
            );
            app.terminal_inspector.map(|inspector| inspector.scroll)
        };

        assert_eq!(scroll_after(&mut app, 'z'), Some(1));
        assert_eq!(scroll_after(&mut app, 'x'), Some(usize::MAX));
    }

    #[test]
    fn inspector_closes_and_scrolls_with_remapped_list_keys() {
        let mut app = App::default();
        app.start_command("Creating branch: x from main".to_string());
        app.open_terminal_inspector().unwrap();
        app.config.keybindings.set(Action::ListBack, &["x"]);
        app.config.keybindings.set(Action::ListNextRow, &["n"]);
        let press = |app: &mut App, code: KeyCode| {
            handle_inspector_key(app, &KeyEvent::new(code, KeyModifiers::NONE).into());
            app.terminal_inspector.map(|inspector| inspector.scroll)
        };

        assert_eq!(press(&mut app, KeyCode::Char('j')), Some(0));
        assert_eq!(press(&mut app, KeyCode::Char('n')), Some(1));
        assert_eq!(press(&mut app, KeyCode::Esc), Some(1));
        assert_eq!(press(&mut app, KeyCode::Char('x')), None);
    }
}
