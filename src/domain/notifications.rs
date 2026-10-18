use crate::backend::BackendKind;
use crate::domain::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub project_path: String,
    pub title: String,
    pub target_type: String,
    pub target_iid: u64,
    pub state: String,
    pub updated_at: String,
}

/// The issue or MR/PR a notification points at. Other targets (commits,
/// releases, epics, discussions) have no view in glab-tui.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationTarget {
    Issue,
    MergeRequest,
}

impl Notification {
    pub fn is_unread(&self) -> bool {
        self.state == "unread" || self.state == "pending"
    }

    pub fn target(&self) -> Option<NotificationTarget> {
        match self.target_type.as_str() {
            "Issue" => Some(NotificationTarget::Issue),
            "MergeRequest" => Some(NotificationTarget::MergeRequest),
            _ => None,
        }
    }
}

/// A notification mutation, carried back to the event loop so the local list
/// follows the server without a refetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TodoChange {
    MarkedRead(String),
    MarkedDone(String),
    MarkedAllRead,
    Unsubscribed(String),
}

impl TodoChange {
    pub fn failure_message(&self, kind: BackendKind, error: &str) -> String {
        let todo = kind.term("todo").to_lowercase();
        match self {
            TodoChange::MarkedRead(_) => format!("Failed to mark {todo} as read: {error}"),
            TodoChange::MarkedDone(_) => format!("Failed to mark {todo} as done: {error}"),
            TodoChange::MarkedAllRead => format!(
                "Failed to mark all {} as read: {error}",
                kind.term("todo_plural").to_lowercase()
            ),
            TodoChange::Unsubscribed(_) => format!("Failed to unsubscribe: {error}"),
        }
    }
}

/// The state a notification reports once read. GitLab todos only know
/// `pending` and `done`.
pub fn read_state(kind: BackendKind) -> &'static str {
    if kind.is_github() { "read" } else { "done" }
}
pub async fn list_notifications(
    client: &GitlabClient,
    show_read: bool,
) -> Result<Vec<Notification>> {
    client.backend.list_notifications(show_read).await
}

pub async fn mark_notification_as_read(client: &GitlabClient, id: &str) -> Result<()> {
    client.backend.mark_notification_as_read(id).await
}

pub async fn mark_notification_as_done(client: &GitlabClient, id: &str) -> Result<()> {
    client.backend.mark_notification_as_done(id).await
}

pub async fn mark_all_notifications_as_read(client: &GitlabClient) -> Result<()> {
    client.backend.mark_all_notifications_as_read().await
}

pub async fn unsubscribe_from_notification(
    client: &GitlabClient,
    notification: &Notification,
) -> Result<()> {
    client
        .backend
        .unsubscribe_from_notification(notification)
        .await
}
