use super::client::GitlabClient;
use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Author {
    pub username: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Milestone {
    pub title: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Assignee {
    pub username: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Reviewer {
    pub username: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MergeRequest {
    pub iid: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub labels: Vec<String>,
    pub updated_at: String,
    pub author: Author,
    pub milestone: Option<Milestone>,
    #[serde(default)]
    pub assignees: Vec<Assignee>,
    #[serde(default)]
    pub reviewers: Vec<Reviewer>,
    pub target_branch: String,
    pub draft: bool,
    pub description: Option<String>,
}

pub async fn list_mrs(client: &GitlabClient, project_path: &str) -> Result<Vec<MergeRequest>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!("/projects/{}/merge_requests?state=opened", encoded_path);
    client.fetch_api(&endpoint).await
}

pub async fn get_mr(client: &GitlabClient, project_path: &str, iid: u64) -> Result<MergeRequest> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!("/projects/{}/merge_requests/{}", encoded_path, iid);
    client.fetch_api(&endpoint).await
}

