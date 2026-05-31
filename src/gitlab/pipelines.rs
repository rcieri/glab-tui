use super::client::GitlabClient;
use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Pipeline {
    pub id: u64,
    pub status: String,
    pub r#ref: String,
    pub updated_at: String,
}

pub async fn list_pipelines(client: &GitlabClient, project_path: &str) -> Result<Vec<Pipeline>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!("/projects/{}/pipelines?per_page=20", encoded_path);
    client.fetch_api(&endpoint).await
}
