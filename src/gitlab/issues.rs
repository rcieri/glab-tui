use super::client::GitlabClient;
use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Author {
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct Issue {
    pub iid: u64,
    pub title: String,
    pub state: String,
    pub labels: Vec<String>,
    pub updated_at: String,
    pub author: Author,
}

pub async fn list_issues(client: &GitlabClient, project_path: &str) -> Result<Vec<Issue>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!("/projects/{}/issues?state=opened", encoded_path);
    client.fetch_api(&endpoint).await
}
