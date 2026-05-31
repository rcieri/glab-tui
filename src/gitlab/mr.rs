use super::client::GitlabClient;
use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Author {
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct MergeRequest {
    pub iid: u64,
    pub title: String,
    pub state: String,
    pub labels: Vec<String>,
    pub updated_at: String,
    pub author: Author,
}

pub async fn list_mrs(client: &GitlabClient, project_path: &str) -> Result<Vec<MergeRequest>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!("/projects/{}/merge_requests?state=opened", encoded_path);
    client.fetch_api(&endpoint).await
}
