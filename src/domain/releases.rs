use crate::domain::client::GitlabClient;
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
    client
        .backend
        .list_releases(project_path, client.page_size)
        .await
}

pub async fn update_release(
    client: &GitlabClient,
    project_path: &str,
    tag_name: &str,
    name: &str,
    description: &str,
) -> Result<()> {
    client
        .backend
        .update_release(project_path, tag_name, name, description)
        .await
}

pub async fn delete_release(
    client: &GitlabClient,
    project_path: &str,
    tag_name: &str,
) -> Result<()> {
    client.backend.delete_release(project_path, tag_name).await
}
