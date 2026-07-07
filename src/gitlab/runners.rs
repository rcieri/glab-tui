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

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabRunner {
    pub id: u64,
    pub description: Option<String>,
    pub status: String,
    pub active: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubRunner {
    pub id: u64,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubRunners {
    pub runners: Vec<GithubRunner>,
}

impl From<GitlabRunner> for Runner {
    fn from(gl: GitlabRunner) -> Self {
        Self {
            id: gl.id,
            description: gl.description,
            status: gl.status,
            active: gl.active,
        }
    }
}

impl From<GithubRunner> for Runner {
    fn from(gh: GithubRunner) -> Self {
        Self {
            id: gh.id,
            description: Some(gh.name),
            status: gh.status,
            active: true,
        }
    }
}

pub async fn list_runners(client: &GitlabClient, project_path: &str) -> Result<Vec<Runner>> {
    if client.is_github {
        let endpoint = format!(
            "/repos/{}/actions/runners?per_page={}",
            project_path, client.page_size
        );
        let raw = client.execute_github_api(&endpoint, "GET", None).await?;
        let res: GithubRunners = serde_json::from_str(&raw)?;
        Ok(res.runners.into_iter().map(Runner::from).collect())
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!(
            "/projects/{}/runners?per_page={}",
            encoded_path, client.page_size
        );
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_runners: Vec<GitlabRunner> = serde_json::from_str(&raw)?;
        Ok(gl_runners.into_iter().map(Runner::from).collect())
    }
}
