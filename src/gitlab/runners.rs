use super::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Runner {
    pub id: u64,
    pub description: Option<String>,
    pub status: String,
    pub active: bool,
}

pub async fn list_runners(client: &GitlabClient, project_path: &str) -> Result<Vec<Runner>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!("/projects/{}/runners?per_page=100", encoded_path);
    client.fetch_api(&endpoint).await
}
