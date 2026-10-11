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

impl Notification {
    /// GitLab todos go from `pending` to `done`, GitHub notifications from
    /// `unread` to `read`. Anything else is already read.
    pub fn mark_read(&mut self) {
        let read = match self.state.as_str() {
            "pending" => "done",
            "unread" => "read",
            _ => return,
        };
        self.state = read.to_string();
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(state: &str) -> Notification {
        Notification {
            id: "1".to_string(),
            project_path: "org/repo".to_string(),
            title: "t".to_string(),
            target_type: "Issue".to_string(),
            target_iid: 1,
            state: state.to_string(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn mark_read_moves_each_backend_to_its_read_state() {
        for (before, after) in [("pending", "done"), ("unread", "read"), ("done", "done")] {
            let mut n = todo(before);
            n.mark_read();
            assert_eq!(n.state, after, "from {before}");
        }
    }
}
