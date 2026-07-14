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

pub async fn list_notifications(
    client: &GitlabClient,
    show_read: bool,
) -> Result<Vec<Notification>> {
    client.backend.list_notifications(show_read).await
}

pub async fn mark_notification_as_read(client: &GitlabClient, id: &str) -> Result<()> {
    client.backend.mark_notification_as_read(id).await
}
