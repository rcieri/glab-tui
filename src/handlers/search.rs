use crate::app::App;
use crate::keybinding::KeyPress;
use crossterm::event::{KeyCode, KeyModifiers};

/// Keys while the job log search query is being typed.
pub fn handle_trace_search_key(app: &mut App, key_event: &KeyPress) {
    match key_event.code {
        KeyCode::Enter | KeyCode::Esc => {
            app.job_trace_searching = false;
        }
        KeyCode::Backspace => {
            app.job_trace_search_query.pop();
        }
        KeyCode::Char(c) => {
            app.job_trace_search_query.push(c);
        }
        _ => {}
    }
    if !app.job_trace_search_query.is_empty() {
        let query = app.job_trace_search_query.to_lowercase();
        let line = app.job_trace.as_ref().and_then(|view| {
            view.trace()
                .text()
                .lines()
                .position(|line| line.to_lowercase().contains(&query))
        });
        if let Some(line) = line {
            app.scroll_job_trace_to_line(line);
        }
    }
}

/// Keys while the search bar is being typed in.
pub fn handle_search_bar_key(app: &mut App, key_event: &KeyPress) {
    match key_event.code {
        KeyCode::Enter | KeyCode::Esc => app.is_typing_search = false,
        KeyCode::Char('u') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            app.clear_search_query();
        }
        KeyCode::Char('w') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            app.pop_search_word();
        }
        KeyCode::Backspace => {
            app.search_query.pop();
            app.update_filter_selection();
        }
        KeyCode::Char(c) => {
            app.search_query.push(c);
            app.update_filter_selection();
        }
        _ => {}
    }
}
