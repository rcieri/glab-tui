use crate::app::App;
use crate::domain::client::GitlabClient;
use crate::effect::Effects;
use crate::event::Event;
use crate::fetch::{spawn_fetch_repo_attributes, spawn_refresh_all_tabs};
use crate::scope::Scope;
use tokio::sync::mpsc::UnboundedSender;

/// After a key dispatch changed `app.scope`, reload its cache into `app` and
/// start building the GitLab/GitHub client for the new scope. No-op if the
/// scope didn't change.
pub(crate) fn sync_after_scope_change(
    app: &mut App,
    old_scope: &Scope,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    if app.scope == *old_scope {
        return;
    }

    let cache = crate::utils::cache::load_cache(app.scope.as_str());
    app.project_cache = cache.clone();
    app.restore_cached_issues(cache.issues, cache.issues_state_query);
    app.restore_cached_mrs(cache.mrs, cache.mrs_state_query);
    crate::fetch::derive_workflow(&mut app.mrs.items);
    app.pipelines.items = cache.pipelines;
    app.runners.set_items(cache.runners);
    app.releases.set_items(cache.releases);
    app.todos.items = cache.todos;
    app.milestones.items = cache.milestones;
    app.pipeline_jobs = cache.pipeline_jobs;
    app.branches.set_items(cache.branches);
    app.environments.set_items(cache.environments);
    app.milestone_issues_cache = cache.milestone_issues;
    app.rebuild_milestone_progress_cache();
    app.cached_labels = cache.labels;
    app.cached_members = cache.members;

    app.start_loading_tab(app.active_tab);
    spawn_client_for_scope(app, tx, fx);
}

/// Builds the client for `app.scope` off the event loop: backend detection
/// shells out to `git remote` and `gh`/`glab auth status`, which can take
/// seconds on a slow network. Until `Event::ClientReady` arrives the app has
/// no client, so nothing runs against the previous scope's backend. The
/// result travels on the scoped sender, so a build that a later switch
/// superseded is dropped by generation, even when both builds are for the
/// same scope.
pub(crate) fn spawn_client_for_scope(app: &mut App, tx: &UnboundedSender<Event>, fx: &mut Effects) {
    app.gitlab_client = None;
    let config = app.config.clone();
    let scope = app.scope.clone();
    let tx = app.scoped_sender(tx.clone());
    fx.spawn(async move {
        let result = GitlabClient::new(&config)
            .await
            .map_err(|e| format!("Failed to initialize backend client: {e}"));
        let _ = tx.send(Event::ClientReady { scope, result });
    });
}

/// Installs a client built by `spawn_client_for_scope` and starts the refresh
/// for its scope. A client for a scope the user already left is dropped.
pub(crate) fn apply_client_ready(
    app: &mut App,
    scope: Scope,
    result: Result<GitlabClient, String>,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    if app.scope != scope {
        return;
    }
    // Without a client nothing could fetch the tabs the user left while it
    // was being built, so their loading marks would never clear.
    let active_tab = app.active_tab;
    app.loading_tabs.retain(|tab| *tab == active_tab);
    let mut client = match result {
        Ok(client) => client,
        Err(message) => {
            app.loading_tabs.remove(&active_tab);
            app.show_error(message);
            return;
        }
    };
    client.page_size = app.config.page_size;
    client.api_per_page = app.config.api_per_page_clamped();
    client.tx = Some(tx.clone());
    client.backend.set_tx(tx.clone());
    app.gitlab_client = Some(client.clone());
    // Remember the group implied by a repository scope so the Switch view
    // accumulates every group worked in (see startup).
    if let Scope::Repository(r) = &app.scope {
        if let Some((group, _)) = r.rsplit_once('/') {
            crate::utils::cache::add_recent_group(group);
        }
    }

    let tx = app.scoped_sender(tx.clone());
    crate::fetch::spawn_refresh_shown_tab(
        fx,
        &client,
        app,
        crate::dashboard::DashboardRefresh::Full,
        tx.clone(),
    );
    if app.config.prefetch_tabs {
        spawn_refresh_all_tabs(fx, &client, app, tx.clone());
    }
    spawn_fetch_repo_attributes(fx, &client.muted(), &app.scope, tx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_client_build_clears_loading_marks_of_tabs_left_while_it_ran() {
        let mut app = App::default();
        app.scope = Scope::Group("grp".to_string());
        app.loading_tabs.insert(crate::app::Tab::Issues);
        app.active_tab = crate::app::Tab::MergeRequests;
        let scope = app.scope.clone();
        let tx = app.tx.clone();

        apply_client_ready(
            &mut app,
            scope,
            Err("auth probe failed".to_string()),
            &tx,
            &mut Effects::default(),
        );

        assert!(app.loading_tabs.is_empty());
    }
}
