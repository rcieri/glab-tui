use crate::domain::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Environment {
    pub id: u64,
    pub name: String,
    pub state: String,
    #[serde(default)]
    pub external_url: Option<String>,
    #[serde(default)]
    pub last_deployment: Option<Deployment>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Deployment {
    pub id: u64,
    pub iid: u64,
    pub ref_name: String,
    pub tag: bool,
    pub sha: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub environment: Option<EnvironmentInfo>,
    #[serde(default)]
    pub deployable: Option<Deployable>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub user: Option<DeploymentUser>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct EnvironmentInfo {
    pub name: String,
    #[serde(default)]
    pub external_url: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Deployable {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DeploymentUser {
    pub username: String,
}

pub async fn list_environments(
    client: &GitlabClient,
    project_path: &str,
) -> Result<Vec<Environment>> {
    client
        .backend
        .list_environments(project_path, client.page_size)
        .await
}

pub async fn list_deployments(
    client: &GitlabClient,
    project_path: &str,
    environment: Option<&str>,
) -> Result<Vec<Deployment>> {
    client
        .backend
        .list_deployments(project_path, client.page_size, environment)
        .await
}
