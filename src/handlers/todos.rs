//! Todos tab actions: GitLab todos and GitHub notifications.

use crate::app::{App, ConfirmAction, SubmitDialog, Tab};
use crate::domain::client::GitlabClient;
use crate::domain::notifications::{self, Notification, NotificationTarget, TodoChange};
use crate::event::Event;
use tokio::sync::mpsc::UnboundedSender;

/// Opens the highlighted todo's issue or MR/PR and marks the todo read. A
/// todo with nothing to open here stays unread. Bound to both
/// `keybindings.todos.mark_as_read` and `Enter`.
pub fn open_selected(app: &mut App, tx: &UnboundedSender<Event>) {
    let Some(todo) = app.selected_todo().cloned() else {
        return;
    };
    let Some(client) = backend_client(app) else {
        return;
    };
    if !jump_to_target(app, &client, &todo, tx) || !todo.is_unread() {
        return;
    }
    let id = todo.id.clone();
    spawn_todo_change(tx, TodoChange::MarkedRead(id.clone()), async move {
        notifications::mark_notification_as_read(&client, &id).await
    });
}

pub fn mark_selected_done(app: &mut App, tx: &UnboundedSender<Event>) {
    let Some(id) = app.selected_todo().map(|todo| todo.id.clone()) else {
        return;
    };
    let Some(client) = backend_client(app) else {
        return;
    };
    spawn_todo_change(tx, TodoChange::MarkedDone(id.clone()), async move {
        notifications::mark_notification_as_done(&client, &id).await
    });
}

pub fn unsubscribe_selected(app: &mut App, tx: &UnboundedSender<Event>) {
    let Some(todo) = app.selected_todo().cloned() else {
        return;
    };
    let Some(client) = backend_client(app) else {
        return;
    };
    spawn_todo_change(tx, TodoChange::Unsubscribed(todo.id.clone()), async move {
        notifications::unsubscribe_from_notification(&client, &todo).await
    });
}

pub fn open_selected_in_browser(app: &mut App, tx: &UnboundedSender<Event>) {
    let Some(todo) = app.selected_todo() else {
        return;
    };
    let entity = match todo.target() {
        Some(NotificationTarget::MergeRequest) if app.is_github() => "pr",
        Some(NotificationTarget::MergeRequest) => "mr",
        _ => "issue",
    };
    let project_path = if todo.project_path.is_empty() {
        app.scope.as_str().to_string()
    } else {
        todo.project_path.clone()
    };
    let target_iid = todo.target_iid.to_string();
    let Some(client) = backend_client(app) else {
        return;
    };
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = client
            .open_in_browser(&project_path, entity, &target_iid)
            .await;
        let _ = tx.send(Event::CommandCompleted(
            Tab::Todos,
            result.map_err(|e| e.to_string()),
        ));
    });
}

/// Asks before marking everything read: neither GitLab nor GitHub can undo
/// it in bulk.
pub fn confirm_mark_all_read(app: &mut App) {
    app.submit_dialog = Some(SubmitDialog::build(ConfirmAction::MarkAllTodosRead, app));
}

pub fn mark_all_read(app: &mut App, tx: &UnboundedSender<Event>) {
    let Some(client) = backend_client(app) else {
        return;
    };
    spawn_todo_change(tx, TodoChange::MarkedAllRead, async move {
        notifications::mark_all_notifications_as_read(&client).await
    });
}

/// Switches the tab between unread-only and everything, and refetches it.
pub fn toggle_show_read(app: &mut App, tx: &UnboundedSender<Event>) {
    app.toggle_show_read_todos();
    let Some(client) = app.gitlab_client.clone() else {
        return;
    };
    app.start_loading_tab(Tab::Todos);
    crate::fetch::spawn_refresh_active_tab(
        &client,
        &app.scope,
        app.requested_state_queries(),
        Tab::Todos,
        app.scoped_sender(tx.clone()),
    );
}

fn backend_client(app: &mut App) -> Option<GitlabClient> {
    let client = app.gitlab_client.clone();
    if client.is_none() {
        app.show_error("No backend client available".to_string());
    }
    client
}

/// Runs a todo mutation in the background and reports it as `TodoChanged`,
/// which updates the list on success and raises the error otherwise.
fn spawn_todo_change(
    tx: &UnboundedSender<Event>,
    change: TodoChange,
    task: impl Future<Output = anyhow::Result<()>> + Send + 'static,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = task.await.map_err(|e| format!("{e:#}"));
        let _ = tx.send(Event::TodoChanged(change, result));
    });
}

/// Selects the todo's issue or MR/PR in its own project, fetching it when the
/// list does not hold it. Targets outside the scope cannot be listed here.
/// Returns `false`, after raising the reason, when there is nothing to open.
fn jump_to_target(
    app: &mut App,
    client: &GitlabClient,
    todo: &Notification,
    tx: &UnboundedSender<Event>,
) -> bool {
    let Some(target) = todo.target() else {
        app.show_error(format!(
            "{} {}s have no view in glab-tui.",
            todo.target_type,
            app.kind().term("todo").to_lowercase(),
        ));
        return false;
    };
    let project = todo.project_path.as_str();
    let iid = todo.target_iid;
    if !app.scope.contains_project(project) {
        let marker = match target {
            NotificationTarget::MergeRequest if app.kind().is_gitlab() => "!",
            _ => "#",
        };
        let open_in_browser = &app.config.keybindings.todos.open_in_browser;
        app.show_error(format!(
            "{project}{marker}{iid} is outside {}. Press {open_in_browser} to open it in the browser.",
            app.scope
        ));
        return false;
    }
    let is_loaded = match target {
        NotificationTarget::Issue => app.focus_issue(Some(project), iid),
        NotificationTarget::MergeRequest => app.focus_mr(Some(project), iid),
    };
    if is_loaded {
        return true;
    }
    match target {
        NotificationTarget::Issue => {
            crate::fetch::spawn_fetch_issue(client, project, iid, app.scoped_sender(tx.clone()));
        }
        NotificationTarget::MergeRequest => {
            crate::fetch::spawn_fetch_mr(client, project, iid, app.scoped_sender(tx.clone()));
        }
    }
    true
}
