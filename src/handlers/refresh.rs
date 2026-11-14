use crate::app::App;
use crate::effect::Effects;
use crate::event::Event;
use crate::fetch::{spawn_fetch_repo_attributes, spawn_refresh_shown_tab};
use crate::keybinding::KeyPress;
use crate::keymap::Action;
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

pub fn handle_refresh(
    app: &mut App,
    key_event: &KeyPress,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) -> bool {
    let is_refresh = app
        .config
        .keybindings
        .matches(Action::GlobalRefresh, key_event);

    if is_refresh
        && app.text_input.is_none()
        && app.date_picker.is_none()
        && app.edit_menu.is_none()
        && app.selector.is_none()
        && !app.is_typing_search
        && !app.job_trace_searching
        && !app.diff_view.as_ref().is_some_and(|d| d.search_active)
    {
        app.last_list_refresh = Instant::now();
        app.last_attr_refresh = Instant::now();
        if let Some(client) = app.gitlab_client.clone() {
            let tx = app.scoped_sender(tx.clone());
            if !app.loading_tabs.contains(&app.active_tab) {
                if app.active_tab == crate::app::Tab::Milestones {
                    app.milestone_prefetch = None;
                }
                app.start_loading_tab(app.active_tab);
                spawn_refresh_shown_tab(
                    fx,
                    &client,
                    app,
                    crate::dashboard::DashboardRefresh::Full,
                    tx.clone(),
                );
            }
            if let Some(parent_id) = app.current_parent_id() {
                crate::fetch::spawn_refresh_child_level(
                    fx,
                    &client,
                    app.scope.as_str(),
                    parent_id,
                    tx.clone(),
                );
            }
            spawn_fetch_repo_attributes(fx, &client.muted(), &app.scope, tx);
        }
        return true;
    }
    false
}
