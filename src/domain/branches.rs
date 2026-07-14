use crate::domain::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Branch {
    pub name: String,
    pub default: bool,
    pub protected: bool,
    pub web_url: String,
    #[serde(default)]
    pub can_push: bool,
    #[serde(default)]
    pub commit_sha: String,
}

pub async fn list_branches(client: &GitlabClient, project_path: &str) -> Result<Vec<Branch>> {
    client
        .backend
        .list_branches(project_path, client.page_size)
        .await
}

pub async fn create_branch(
    client: &GitlabClient,
    project_path: &str,
    branch_name: &str,
    ref_branch: &str,
) -> Result<()> {
    client
        .backend
        .create_branch(project_path, branch_name, ref_branch)
        .await
}

pub async fn delete_branch(
    client: &GitlabClient,
    project_path: &str,
    branch_name: &str,
) -> Result<()> {
    client
        .backend
        .delete_branch(project_path, branch_name)
        .await
}
