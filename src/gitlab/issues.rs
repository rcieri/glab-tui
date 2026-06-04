use super::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Author {
    pub username: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Milestone {
    pub title: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Assignee {
    pub username: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Issue {
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
    pub description: Option<String>,
}

pub async fn list_issues(client: &GitlabClient, project_path: &str) -> Result<Vec<Issue>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!(
        "/projects/{}/issues?state=opened&per_page=100",
        encoded_path
    );
    client.fetch_api(&endpoint).await
}

pub async fn get_issue(client: &GitlabClient, project_path: &str, iid: u64) -> Result<Issue> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!("/projects/{}/issues/{}", encoded_path, iid);
    client.fetch_api(&endpoint).await
}
