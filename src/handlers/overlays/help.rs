use crate::app::App;
use crate::keybinding::{KeyPress, keybinding_matches};
use crossterm::event::{KeyCode, KeyModifiers};

pub fn handle_help_keybinding(app: &mut App, key_event: &KeyPress) -> bool {
    if app.show_help() {
        return false;
    }

    let is_f1 = key_event.code == KeyCode::F(1);
    let is_help_key = is_f1 || keybinding_matches(&app.config.keybindings.global.help, key_event);

    if !is_help_key {
        return false;
    }

    // When the user is actively typing raw text into a text field, allow F1 to open help,
    // but keep regular character keys (like '?') so they can be typed into the field.
    if app.is_capturing_text() && !is_f1 {
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
            _ if keybinding_matches(
                &app.config.keybindings.global.scroll_half_page_down,
                key_event,
            ) =>
            {
                move_help_selection(app, true, HELP_PAGE_ROWS / 2)
            }
            _ if keybinding_matches(
                &app.config.keybindings.global.scroll_half_page_up,
                key_event,
            ) =>
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
