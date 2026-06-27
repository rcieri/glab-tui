use super::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Release {
    pub name: String,
    pub tag_name: String,
    pub released_at: String,
    pub description: Option<String>,
    pub author_name: Option<String>,
    pub commit_id: Option<String>,
    pub commit_title: Option<String>,
    pub assets_link: Option<String>,
}

pub async fn list_releases(client: &GitlabClient, project_path: &str) -> Result<Vec<Release>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!("/projects/{}/releases?per_page=100", encoded_path);
    client.fetch_api(&endpoint).await
}
