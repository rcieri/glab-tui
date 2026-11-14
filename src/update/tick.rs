use crate::app::{App, Tab};
use crate::effect::Effects;
use crate::event::Event;
use crate::fetch;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

const JOB_TRACE_POLL_INTERVAL: Duration = Duration::from_secs(5);
const LIST_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const ATTRIBUTE_REFRESH_INTERVAL: Duration = Duration::from_secs(300);

/// Work that waits for time to pass: key sequence timeouts, debounced
/// preview fetches, the followed job log and the periodic list refresh.
pub(super) fn tick(app: &mut App, tx: &UnboundedSender<Event>, fx: &mut Effects) {
    app.expire_typed_keys();
    app.tick();
    dispatch_debounced_fetches(app, tx, fx);
    poll_followed_job_trace(app, tx, fx);
    refresh_periodically(app, tx, fx);
}

fn dispatch_debounced_fetches(app: &mut App, tx: &UnboundedSender<Event>, fx: &mut Effects) {
    let Some(client) = app.gitlab_client.clone() else {
        app.pending_related_mrs_iid = None;
        app.pending_related_mrs_since = None;
        app.pending_mr_related_issues_iid = None;
        app.pending_mr_related_issues_since = None;
        app.pr_stack_candidate = None;
        app.todo_detail_candidate = None;
        return;
    };
    fetch::dispatch_pending_related_mrs_fetch(fx, &client, app, tx);
    fetch::dispatch_pending_mr_related_issues_fetch(fx, &client, app, tx);
    fetch::dispatch_state_query_refetch(fx, &client, app, tx);
    fetch::dispatch_inspected_pr_stack_fetch(fx, &client, app, tx);
    fetch::dispatch_inspected_todo_detail_fetch(fx, &client, app, tx);
}

fn poll_followed_job_trace(app: &mut App, tx: &UnboundedSender<Event>, fx: &mut Effects) {
    if app.active_tab != Tab::Jobs
        || !app.job_trace_follow
        || app.job_trace_loading
        || app.job_trace_poll_in_flight
        || app.job_trace_last_refresh.elapsed() < JOB_TRACE_POLL_INTERVAL
    {
        return;
    }
    let cursor = app
        .job_trace
        .as_ref()
        .filter(|view| !view.trace().is_truncated())
        .map(|view| view.trace().cursor().clone());
    let (Some(cursor), Some(job_id), Some(client)) =
        (cursor, app.trace_job_id(), app.gitlab_client.clone())
    else {
        return;
    };
    let project = app.project_path_for_jobs();
    let tx = app.scoped_sender(tx.clone());
    app.job_trace_poll_in_flight = true;
    app.job_trace_last_refresh = Instant::now();
    fx.spawn(async move {
        let result =
            crate::domain::pipelines::get_job_trace_since(&client, &project, job_id, &cursor).await;
        let _ = tx.send(Event::JobTraceFollowed(
            job_id,
            result.map_err(|e| e.to_string()),
        ));
    });
}

/// Refreshes a high-churn tab's list every minute, and the labels and
/// members every five, while no form or picker is open. Switching tabs
/// restarts both timers.
fn refresh_periodically(app: &mut App, tx: &UnboundedSender<Event>, fx: &mut Effects) {
    if app.active_tab != app.refresh_timer_tab {
        app.refresh_timer_tab = app.active_tab;
        app.last_list_refresh = Instant::now();
        app.last_attr_refresh = Instant::now();
        return;
    }
    let is_form_open =
        app.text_input.is_some() || app.edit_menu.is_some() || app.selector.is_some();
    if app.last_list_refresh.elapsed() >= LIST_REFRESH_INTERVAL && app.active_tab.is_high_churn() {
        if !is_form_open && !app.loading_tabs.contains(&app.active_tab) {
            if let Some(client) = app.gitlab_client.clone() {
                app.start_loading_tab(app.active_tab);
                let tx = app.scoped_sender(tx.clone());
                fetch::spawn_refresh_shown_tab(
                    fx,
                    &client.muted(),
                    app,
                    crate::dashboard::DashboardRefresh::Periodic,
                    tx,
                );
            }
        }
        app.last_list_refresh = Instant::now();
    }
    if app.last_attr_refresh.elapsed() >= ATTRIBUTE_REFRESH_INTERVAL && !is_form_open {
        if let Some(client) = app.gitlab_client.clone() {
            fetch::spawn_fetch_repo_attributes(
                fx,
                &client.muted(),
                &app.scope,
                app.scoped_sender(tx.clone()),
            );
        }
        app.last_attr_refresh = Instant::now();
    }
}
