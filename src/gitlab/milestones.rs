use super::client::GitlabClient;
use super::issues::Issue;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Milestone {
    pub id: u64,
    pub iid: u64,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub start_date: Option<String>,
    pub due_date: Option<String>,
    pub created_at: String,
}

pub async fn list_milestones(client: &GitlabClient, project_path: &str) -> Result<Vec<Milestone>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!("/projects/{}/milestones?per_page=100", encoded_path);
    client.fetch_api(&endpoint).await
}

pub async fn list_milestone_issues(
    client: &GitlabClient,
    project_path: &str,
    milestone_iid: u64,
) -> Result<Vec<Issue>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!(
        "/projects/{}/milestones/{}/issues?per_page=100",
        encoded_path, milestone_iid
    );
    client.fetch_api(&endpoint).await
}
