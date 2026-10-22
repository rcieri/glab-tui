use crate::backend::BackendKind;
use crate::domain::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub project_path: String,
    pub title: String,
    pub target_type: String,
    pub target_iid: u64,
    pub state: String,
    pub updated_at: String,
    /// On GitLab, whoever caused the todo. GitHub lists no author, so it
    /// stays empty there until the detail fetch fills it with the author of
    /// the latest comment, or of the target when there is none.
    #[serde(default)]
    pub author: String,
    /// Why it arrived (GitLab `action_name`, GitHub `reason`), e.g.
    /// `mentioned` or `review_requested`.
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub web_url: String,
    /// The comment that triggered it, or else the target's description.
    /// `None` until known: GitHub lists notifications without it, so it
    /// comes from `detail_api_path` on demand.
    #[serde(default)]
    pub body: Option<String>,
    /// GitHub API path of the latest comment, or of the subject when it has
    /// none. GitLab todos carry their body in the list and have none.
    #[serde(default)]
    pub detail_api_path: Option<String>,
}

/// The issue or MR/PR a notification points at. Other targets (commits,
/// releases, epics, discussions) have no view in glab-tui.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationTarget {
    Issue,
    MergeRequest,
}

/// What the detail fetch adds to a GitHub notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationDetail {
    pub author: String,
    pub body: String,
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

    /// The target as written in a comment: `!456` for a GitLab MR, `#123`
    /// for issues and GitHub PRs. Other targets have none: GitHub puts a
    /// release's database id or `0` in `target_iid`.
    pub fn reference(&self, kind: BackendKind) -> Option<String> {
        let marker = match self.target()? {
            NotificationTarget::MergeRequest if kind.is_gitlab() => '!',
            _ => '#',
        };
        Some(format!("{marker}{}", self.target_iid))
    }

    /// Short name of the target kind for the Type column: `MR` or `PR`,
    /// `Issue`, or the raw type for targets glab-tui has no view for.
    pub fn type_label(&self, kind: BackendKind) -> &str {
        match self.target() {
            Some(NotificationTarget::MergeRequest) if kind.is_github() => "PR",
            Some(NotificationTarget::MergeRequest) => "MR",
            Some(NotificationTarget::Issue) => "Issue",
            None => &self.target_type,
        }
    }

    pub fn apply_detail(&mut self, detail: NotificationDetail) {
        if !detail.author.is_empty() {
            self.author = detail.author;
        }
        self.body = Some(detail.body);
    }
}

/// A notification mutation, carried back to the event loop so the local list
/// follows the server without a refetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TodoChange {
    MarkedRead(Vec<String>),
    MarkedDone(Vec<String>),
    MarkedAllRead,
    Unsubscribed(String),
}

impl TodoChange {
    pub fn failure_message(&self, kind: BackendKind, error: &str) -> String {
        let todos = |ids: &[String]| match ids.len() {
            1 => kind.term("todo").to_lowercase(),
            count => format!("{count} {}", kind.term("todo_plural").to_lowercase()),
        };
        match self {
            TodoChange::MarkedRead(ids) => {
                format!("Failed to mark {} as read: {error}", todos(ids))
            }
            TodoChange::MarkedDone(ids) => {
                format!("Failed to mark {} as done: {error}", todos(ids))
            }
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

pub async fn get_notification_detail(
    client: &GitlabClient,
    detail_api_path: &str,
) -> Result<NotificationDetail> {
    client
        .backend
        .get_notification_detail(detail_api_path)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(target_type: &str) -> Notification {
        Notification {
            target_type: target_type.to_string(),
            target_iid: 42,
            ..Default::default()
        }
    }

    #[test]
    fn only_gitlab_merge_requests_are_referenced_with_a_bang() {
        let mr = notification("MergeRequest");
        assert_eq!(mr.reference(BackendKind::GitLab).as_deref(), Some("!42"));
        assert_eq!(mr.reference(BackendKind::GitHub).as_deref(), Some("#42"));
        assert_eq!(
            notification("Issue")
                .reference(BackendKind::GitLab)
                .as_deref(),
            Some("#42")
        );
    }

    #[test]
    fn a_target_without_a_view_has_no_reference() {
        for target in ["Commit", "Release", "CheckSuite"] {
            assert_eq!(notification(target).reference(BackendKind::GitHub), None);
        }
    }

    #[test]
    fn type_label_names_merge_requests_after_the_host() {
        let mr = notification("MergeRequest");
        assert_eq!(mr.type_label(BackendKind::GitLab), "MR");
        assert_eq!(mr.type_label(BackendKind::GitHub), "PR");
        assert_eq!(
            notification("Issue").type_label(BackendKind::GitHub),
            "Issue"
        );
        assert_eq!(
            notification("Release").type_label(BackendKind::GitHub),
            "Release"
        );
    }

    #[test]
    fn a_batch_failure_names_how_many_todos_it_covered() {
        let one = TodoChange::MarkedRead(vec!["1".to_string()]);
        let two = TodoChange::MarkedDone(vec!["1".to_string(), "2".to_string()]);
        assert_eq!(
            one.failure_message(BackendKind::GitLab, "boom"),
            "Failed to mark todo as read: boom"
        );
        assert_eq!(
            two.failure_message(BackendKind::GitLab, "boom"),
            "Failed to mark 2 todos as done: boom"
        );
    }
}
