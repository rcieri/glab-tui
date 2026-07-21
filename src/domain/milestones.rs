use crate::domain::client::GitlabClient;
use crate::domain::issues::Issue;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct Milestone {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub iid: u64,
    #[serde(default)]
    pub title: String,
    pub description: Option<String>,
    #[serde(default)]
    pub state: String,
    pub start_date: Option<String>,
    pub due_date: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

pub async fn list_milestones(client: &GitlabClient, project_path: &str) -> Result<Vec<Milestone>> {
    client
        .backend
        .list_milestones(project_path, client.page_size)
        .await
}

pub async fn list_milestone_issues(
    client: &GitlabClient,
    project_path: &str,
    milestone_iid: u64,
) -> Result<Vec<Issue>> {
    client
        .backend
        .list_milestone_issues(project_path, milestone_iid, client.page_size)
        .await
}

pub async fn update_milestone_state(
    client: &GitlabClient,
    project_path: &str,
    milestone_iid: u64,
    close: bool,
) -> Result<()> {
    client
        .backend
        .update_milestone_state(project_path, milestone_iid, close)
        .await
}

pub async fn update_milestone(
    client: &GitlabClient,
    project_path: &str,
    milestone_iid: u64,
    title: &str,
    description: &str,
    start_date: Option<&str>,
    due_date: Option<&str>,
) -> Result<()> {
    client
        .backend
        .update_milestone(
            project_path,
            milestone_iid,
            title,
            description,
            start_date,
            due_date,
        )
        .await
}

pub async fn delete_milestone(
    client: &GitlabClient,
    project_path: &str,
    milestone_iid: u64,
) -> Result<()> {
    client
        .backend
        .delete_milestone(project_path, milestone_iid)
        .await
}
