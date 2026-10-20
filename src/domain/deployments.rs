use crate::domain::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

const SHORT_SHA_LEN: usize = 8;

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

impl Deployment {
    /// The CI job that ran this deployment, the one a rollback re-runs on
    /// GitLab. GitHub deployments carry none.
    pub fn job_id(&self) -> Option<u64> {
        self.deployable.as_ref().map(|job| job.id)
    }

    /// The SHA abbreviated the way GitLab and GitHub show it.
    pub fn short_sha(&self) -> &str {
        self.sha.get(..SHORT_SHA_LEN).unwrap_or(&self.sha)
    }
}

pub async fn list_environments(
    client: &GitlabClient,
    scope: &crate::scope::Scope,
) -> Result<Vec<Environment>> {
    client
        .backend
        .list_environments(scope, client.page_size)
        .await
}

pub async fn list_deployments(
    client: &GitlabClient,
    scope: &crate::scope::Scope,
    environment: Option<&str>,
) -> Result<Vec<Deployment>> {
    client
        .backend
        .list_deployments(scope, client.page_size, environment)
        .await
}

pub async fn stop_environment(
    client: &GitlabClient,
    project: &str,
    environment: &Environment,
) -> Result<()> {
    client.backend.stop_environment(project, environment).await
}

pub async fn rollback_deployment(
    client: &GitlabClient,
    project: &str,
    environment: &str,
    deployment: &Deployment,
) -> Result<()> {
    client
        .backend
        .rollback_deployment(project, environment, deployment)
        .await
}
