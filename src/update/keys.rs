use crate::app::App;
use crate::effect::Effects;
use crate::event::Event;
use crate::handlers::overlays::{
    self, handle_date_picker, handle_help_keybinding, handle_help_overlay, handle_review_threads,
    handle_submit_dialog, handle_switch_repo,
};
use crate::handlers::{self, refresh::handle_refresh};
use crate::keybinding::KeyPress;
use crate::keymap::Action;
use tokio::sync::mpsc::UnboundedSender;

/// Routes a resolved keypress: overlays first, then the global keys, then
/// the active tab.
pub(super) fn handle_key(
    app: &mut App,
    key_event: &KeyPress,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    if app
        .config
        .keybindings
        .matches(Action::GlobalForceQuit, key_event)
    {
        if app
            .review_submit
            .as_ref()
            .is_some_and(|o| o.is_submitting())
        {
            app.show_notice(format!(
                "A review is being posted: press {} again once it finishes",
                app.config.keybindings.display(Action::GlobalForceQuit)
            ));
            return;
        }
        fx.quit();
        return;
    }

    if app
        .config
        .keybindings
        .matches(Action::GlobalQuit, key_event)
        && app.text_input.is_none()
        && app.edit_menu.is_none()
        && app.selector.is_none()
        && !app.show_help
        && app.review_threads.is_none()
        && app.review_submit.is_none()
        && app.terminal_inspector.is_none()
        && !app.focus_column_checklist
        && !app.is_typing_search
        && !app.job_trace_searching
        && app.diff_view.is_none()
    {
        fx.quit();
        return;
    }

    if app.review_submit.is_some() {
        handlers::review_submit::handle_key(app, key_event, tx, fx);
        return;
    }

    if handle_help_overlay(app, key_event)
        || handle_help_keybinding(app, key_event)
        || handle_submit_dialog(app, key_event, tx, fx)
        || handle_switch_repo(app, key_event)
        || handle_refresh(app, key_event, tx, fx)
        || handle_date_picker(app, key_event, tx, fx)
        || handle_review_threads(app, key_event)
        || handlers::terminal::handle_inspector_key(app, key_event)
    {
        return;
    }
    if app.text_input.is_some() {
        overlays::text_input::handle_key(app, key_event, tx, fx);
        return;
    }
    if app.selector.is_some() {
        overlays::selector::handle_key(app, key_event, tx, fx);
        return;
    }
    if app.edit_menu.is_some() {
        overlays::edit_menu::handle_key(app, key_event, tx, fx);
        return;
    }
    if app.diff_view.is_some() {
        handlers::diff_view::handle_key(app, key_event, tx, fx);
        return;
    }
    if app.focus_column_checklist {
        overlays::configure::handle_key(app, key_event, tx, fx);
        return;
    }
    if app.job_trace_searching {
        handlers::search::handle_trace_search_key(app, key_event);
        return;
    }
    if app.active_tab == crate::app::Tab::Jobs
        && app.job_trace.is_some()
        && app.config.keybindings.matches(Action::ListBack, key_event)
        && !app.job_trace_search_query.is_empty()
    {
        app.job_trace_search_query.clear();
        return;
    }

    if app.active_tab == crate::app::Tab::Jobs
        && app.job_trace.is_some()
        && app
            .config
            .keybindings
            .matches(Action::JobsTraceSearch, key_event)
    {
        app.job_trace_search_query.clear();
        app.job_trace_searching = true;
        return;
    }

    if app.is_typing_search {
        handlers::search::handle_search_bar_key(app, key_event);
        return;
    }
    if app
        .config
        .keybindings
        .matches(Action::GlobalSearch, key_event)
        && !app.is_typing_search
        && app.text_input.is_none()
        && app.edit_menu.is_none()
        && app.selector.is_none()
        && !app.focus_column_checklist
    {
        app.is_typing_search = true;
        return;
    }

    if app
        .config
        .keybindings
        .matches(Action::GlobalJumpToId, key_event)
        && !app.is_typing_search
        && app.text_input.is_none()
        && app.edit_menu.is_none()
        && app.selector.is_none()
        && !app.focus_column_checklist
    {
        let mut items = Vec::new();
        for issue in &app.issues.items {
            items.push(format!("#{}: {}", issue.iid, issue.title));
        }
        for mr in &app.mrs.items {
            items.push(format!("!{}: {}", mr.iid, mr.title));
        }

        app.selector = Some(crate::app::Selector {
            title: " Jump to Issue/MR (#123 / !123) ".to_string(),
            all_items: items,
            selected_items: std::collections::HashSet::new(),
            cursor_idx: 0,
            search_query: String::new(),
            is_filtering: false,
            is_loading: false,
            entity_iid: 0,
            entity_type: "jump_to_id".to_string(),
            field_type: "jump_to_id".to_string(),
            multi_select: false,
            state: {
                let mut s = ratatui::widgets::ListState::default();
                s.select(Some(0));
                s
            },
            request: None,
        });
        return;
    }

    if app.text_input.is_none()
        && app.edit_menu.is_none()
        && app.selector.is_none()
        && !app.focus_column_checklist
    {
        if app
            .config
            .keybindings
            .matches(Action::GlobalSortNext, key_event)
        {
            app.cycle_sort_column(true);
            return;
        }
        if app
            .config
            .keybindings
            .matches(Action::GlobalSortPrev, key_event)
        {
            app.cycle_sort_column(false);
            return;
        }
        if app
            .config
            .keybindings
            .matches(Action::GlobalSortReverse, key_event)
        {
            app.reverse_sort_order();
            return;
        }
    }

    if app
        .config
        .keybindings
        .matches(Action::GlobalConfigure, key_event)
        && !app.focus_column_checklist
        && app.text_input.is_none()
        && app.edit_menu.is_none()
        && app.selector.is_none()
        && !app.is_deployments_focus_key(key_event)
    {
        app.focus_column_checklist = true;
        app.column_checklist_idx = 0;
        return;
    }

    if app.config.keybindings.matches(Action::ListBack, key_event)
        && !app.focus_column_checklist
        && app.text_input.is_none()
        && app.edit_menu.is_none()
        && app.selector.is_none()
        && app.submit_dialog.is_none()
        && app.prev_scope.is_some()
    {
        if let Some(prev) = app.prev_scope.take() {
            app.scope = prev;
            app.reset_on_scope_change();
        }
    }

    let old_scope = app.scope.clone();
    handlers::tabs::handle_active_tab_key(app, key_event, tx, fx);
    handlers::scope::sync_after_scope_change(app, &old_scope, tx, fx);
}
