use crate::app::App;
use crate::keybinding::{KeyPress, keybinding_matches};
use crossterm::event::KeyCode;

/// Keys for the review threads overlay. Selectors and text inputs opened from
/// it (comment actions, replies) sit on top and take their keys first.
pub fn handle_review_threads(app: &mut App, key_event: &KeyPress) -> bool {
    if app.selector.is_some() || app.text_input.is_some() {
        return false;
    }
    let Some(mut overview) = app.review_threads.take() else {
        return false;
    };

    let mut keep_open = true;
    match key_event.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('T') => keep_open = false,
        _ if keybinding_matches(&app.config.keybindings.global.scroll_top, key_event) => {
            overview.first()
        }
        _ if keybinding_matches(&app.config.keybindings.global.scroll_to_end, key_event) => {
            overview.last()
        }
        // Ahead of the plain `u` arm, which ignores modifiers.
        _ if keybinding_matches(
            &app.config.keybindings.global.scroll_half_page_down,
            key_event,
        ) =>
        {
            move_review_threads_half_page(&mut overview, true)
        }
        _ if keybinding_matches(
            &app.config.keybindings.global.scroll_half_page_up,
            key_event,
        ) =>
        {
            move_review_threads_half_page(&mut overview, false)
        }
        KeyCode::Char('j') | KeyCode::Down => overview.next(),
        KeyCode::Char('k') | KeyCode::Up => overview.previous(),
        KeyCode::Char('g') | KeyCode::Home => overview.first(),
        KeyCode::Char('G') | KeyCode::End => overview.last(),
        KeyCode::Char('J') => overview.preview_scroll = overview.preview_scroll.saturating_add(1),
        KeyCode::Char('K') => overview.preview_scroll = overview.preview_scroll.saturating_sub(1),
        KeyCode::Char('u') => overview.toggle_unresolved_only(),
        KeyCode::Enter => keep_open = !jump_to_review_thread(app, &overview),
        KeyCode::Char('a') => open_review_thread_actions(app, &overview),
        _ => {}
    }
    if keep_open {
        app.review_threads = Some(overview);
    }
    true
}

fn move_review_threads_half_page(overview: &mut crate::app::ReviewThreadsOverview, down: bool) {
    let rows = overview
        .list_rect
        .map_or(1, |rect| (rect.height / 2).max(1) as usize);
    let count = overview.visible_threads().count();
    overview.select(crate::utils::ui::step_clamped(
        overview.cursor_idx,
        count,
        down,
        rows,
    ));
}

/// Moves the diff cursor onto the selected thread's anchor. Returns whether
/// the diff now shows it.
fn jump_to_review_thread(app: &mut App, overview: &crate::app::ReviewThreadsOverview) -> bool {
    let Some(thread) = overview.selected_thread() else {
        return false;
    };
    let position = match (&thread.anchor, thread.root().position.as_ref()) {
        (crate::domain::review_threads::ThreadAnchor::General, _) | (_, None) => {
            app.show_error("General comments are not attached to a line in the diff.".to_string());
            return false;
        }
        (_, Some(position)) => position,
    };
    let Some(diff_view) = app.diff_view.as_mut() else {
        return false;
    };
    let was_hiding_reviewed = diff_view.hide_reviewed;
    if diff_view.jump_to_anchor(position) {
        if was_hiding_reviewed && !diff_view.hide_reviewed {
            app.hide_reviewed_files = false;
            app.show_notice("Showing reviewed files to reach the thread");
        }
        return true;
    }
    let file_path = position
        .new_path
        .as_deref()
        .or(position.old_path.as_deref())
        .unwrap_or_default();
    app.show_error(format!("{file_path} is no longer part of this diff."));
    false
}

fn open_review_thread_actions(app: &mut App, overview: &crate::app::ReviewThreadsOverview) {
    let Some(thread) = overview.selected_thread() else {
        return;
    };
    let Some(mr_iid) = app.diff_view.as_ref().map(|d| d.mr_iid) else {
        return;
    };
    app.selector = Some(if thread.notes.len() == 1 {
        crate::app::Selector::comment_actions(thread.root(), mr_iid, app.is_github())
    } else {
        crate::app::Selector::comment_choice(
            thread
                .notes
                .iter()
                .map(crate::app::Selector::comment_choice_item)
                .collect(),
            mr_iid,
        )
    });
}
