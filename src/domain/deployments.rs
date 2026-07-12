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

// GitHub types
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubEnvironment {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubDeployment {
    pub id: u64,
    pub sha: String,
    pub ref_name: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub description: String,
    pub statuses_url: Option<String>,
    pub repository_url: Option<String>,
    pub environment: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub status: Option<String>,
    // For display purposes
    #[serde(default)]
    pub display_status: String,
}

impl From<GithubEnvironment> for Environment {
    fn from(gh: GithubEnvironment) -> Self {
        Self {
            id: gh.id,
            name: gh.name,
            state: "available".to_string(),
            external_url: gh.html_url,
            last_deployment: None,
        }
    }
}

impl From<GithubDeployment> for Deployment {
    fn from(gh: GithubDeployment) -> Self {
        Self {
            id: gh.id,
            iid: gh.id,
            ref_name: gh.ref_name,
            tag: false,
            sha: gh.sha,
            status: gh.status.unwrap_or_else(|| gh.display_status.clone()),
            created_at: gh.created_at,
            updated_at: gh.updated_at,
            environment: gh.environment.map(|e| EnvironmentInfo {
                name: e,
                external_url: None,
            }),
            deployable: None,
            description: gh.description,
            user: None,
        }
    }
}

// GitLab types
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabEnvironment {
    pub id: u64,
    pub name: String,
    pub state: String,
    #[serde(default)]
    pub external_url: Option<String>,
    #[serde(default)]
    pub last_deployment: Option<GitlabDeployment>,
}

impl From<GitlabEnvironment> for Environment {
    fn from(gl: GitlabEnvironment) -> Self {
        Self {
            id: gl.id,
            name: gl.name,
            state: gl.state,
            external_url: gl.external_url,
            last_deployment: gl.last_deployment.map(Deployment::from),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabDeployment {
    pub id: u64,
    pub iid: u64,
    pub r#ref: String,
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

impl From<GitlabDeployment> for Deployment {
    fn from(gl: GitlabDeployment) -> Self {
        Self {
            id: gl.id,
            iid: gl.iid,
            ref_name: gl.r#ref,
            tag: gl.tag,
            sha: gl.sha,
            status: gl.status,
            created_at: gl.created_at,
            updated_at: gl.updated_at,
            environment: gl.environment,
            deployable: gl.deployable,
            description: gl.description,
            user: gl.user,
        }
    }
}

pub async fn list_environments(
    client: &GitlabClient,
    project_path: &str,
) -> Result<Vec<Environment>> {
    if client.is_github {
        let endpoint = format!(
            "/repos/{}/environments?per_page={}",
            project_path, client.page_size
        );
        let raw = client.execute_github_api(&endpoint, "GET", None).await?;
        // GitHub environments response has a `environments` key
        let resp: GithubEnvironmentsResponse = serde_json::from_str(&raw)?;
        Ok(resp
            .environments
            .into_iter()
            .map(Environment::from)
            .collect())
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!(
            "/projects/{}/environments?per_page={}",
            encoded_path, client.page_size
        );
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_envs: Vec<GitlabEnvironment> = serde_json::from_str(&raw)?;
        Ok(gl_envs.into_iter().map(Environment::from).collect())
    }
}

#[derive(Debug, Deserialize)]
struct GithubEnvironmentsResponse {
    environments: Vec<GithubEnvironment>,
}

pub async fn list_deployments(
    client: &GitlabClient,
    project_path: &str,
    environment: Option<&str>,
) -> Result<Vec<Deployment>> {
    if client.is_github {
        let mut endpoint = format!(
            "/repos/{}/deployments?per_page={}",
            project_path, client.page_size
        );
        if let Some(env) = environment {
            endpoint.push_str(&format!("&environment={}", env));
        }
        let raw = client.execute_github_api(&endpoint, "GET", None).await?;
        let gh_deployments: Vec<GithubDeployment> = serde_json::from_str(&raw)?;
        Ok(gh_deployments.into_iter().map(Deployment::from).collect())
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let mut endpoint = format!(
            "/projects/{}/deployments?per_page={}",
            encoded_path, client.page_size
        );
        if let Some(env) = environment {
            endpoint.push_str(&format!("&environment={}", env));
        }
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_deployments: Vec<GitlabDeployment> = serde_json::from_str(&raw)?;
        Ok(gl_deployments.into_iter().map(Deployment::from).collect())
    }
}
