use crate::app::App;
use crate::keybinding::KeyPress;
use crate::keymap::Action;
use crossterm::event::{KeyCode, KeyModifiers};

pub fn handle_help_keybinding(app: &mut App, key_event: &KeyPress) -> bool {
    if app.show_help() {
        return false;
    }

    let is_help_key = app
        .config
        .keybindings
        .matches(Action::GlobalHelp, key_event);

    if !is_help_key {
        return false;
    }

    let reaches_over_text = matches!(key_event.code, KeyCode::F(_));
    if app.is_capturing_text() && !reaches_over_text {
        return false;
    }

    app.open_help();
    app.help_search_query.clear();
    reset_help_selection(app);
    true
}

/// Rows a PageUp/PageDown in the help list moves by.
const HELP_PAGE_ROWS: usize = 10;

fn reset_help_selection(app: &mut App) {
    app.help_selected = 0;
    app.help_table_state = ratatui::widgets::TableState::default();
}

/// Moves the help selection; the renderer clamps it to the listed entries.
pub fn move_help_selection(app: &mut App, down: bool, rows: usize) {
    app.help_selected = if down {
        app.help_selected.saturating_add(rows)
    } else {
        app.help_selected.saturating_sub(rows)
    };
}

pub fn handle_help_overlay(app: &mut App, key_event: &KeyPress) -> bool {
    if app.show_help() {
        match key_event.code {
            KeyCode::Esc | KeyCode::Enter => {
                app.close_help();
                app.help_search_query.clear();
            }
            KeyCode::Down => move_help_selection(app, true, 1),
            KeyCode::Up => move_help_selection(app, false, 1),
            KeyCode::Char('n') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                move_help_selection(app, true, 1)
            }
            KeyCode::Char('p') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                move_help_selection(app, false, 1)
            }
            KeyCode::PageDown => move_help_selection(app, true, HELP_PAGE_ROWS),
            KeyCode::PageUp => move_help_selection(app, false, HELP_PAGE_ROWS),
            KeyCode::Home => app.help_selected = 0,
            KeyCode::End => app.help_selected = usize::MAX,
            KeyCode::Backspace => {
                app.help_search_query.pop();
                reset_help_selection(app);
            }
            _ if app
                .config
                .keybindings
                .matches(Action::GlobalScrollHalfPageDown, key_event) =>
            {
                move_help_selection(app, true, HELP_PAGE_ROWS / 2)
            }
            _ if app
                .config
                .keybindings
                .matches(Action::GlobalScrollHalfPageUp, key_event) =>
            {
                move_help_selection(app, false, HELP_PAGE_ROWS / 2)
            }
            KeyCode::Char(c) => {
                app.help_search_query.push(c);
                reset_help_selection(app);
            }
            _ => {}
        }
        return true;
    }
    false
}
