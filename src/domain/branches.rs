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

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubBranch {
    pub name: String,
    #[serde(default)]
    pub protected: bool,
    pub commit: Option<GithubBranchCommit>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubBranchCommit {
    pub sha: String,
}

impl From<GithubBranch> for Branch {
    fn from(gh: GithubBranch) -> Self {
        Self {
            name: gh.name.clone(),
            default: false,
            protected: gh.protected,
            web_url: String::new(),
            can_push: false,
            commit_sha: gh
                .commit
                .as_ref()
                .map(|c| c.sha.clone())
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabBranch {
    pub name: String,
    pub default: Option<bool>,
    pub protected: Option<bool>,
    pub web_url: Option<String>,
    pub can_push: Option<bool>,
    pub commit: Option<GitlabBranchCommit>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabBranchCommit {
    pub id: String,
}

impl From<GitlabBranch> for Branch {
    fn from(gl: GitlabBranch) -> Self {
        Self {
            name: gl.name,
            default: gl.default.unwrap_or(false),
            protected: gl.protected.unwrap_or(false),
            web_url: gl.web_url.unwrap_or_default(),
            can_push: gl.can_push.unwrap_or(false),
            commit_sha: gl.commit.as_ref().map(|c| c.id.clone()).unwrap_or_default(),
        }
    }
}

pub async fn list_branches(client: &GitlabClient, project_path: &str) -> Result<Vec<Branch>> {
    if client.is_github {
        let endpoint = format!(
            "/repos/{}/branches?per_page={}",
            project_path, client.page_size
        );
        let raw = client.execute_github_api(&endpoint, "GET", None).await?;
        let gh_branches: Vec<GithubBranch> = serde_json::from_str(&raw)?;
        let mut branches: Vec<Branch> = gh_branches.into_iter().map(Branch::from).collect();
        // Mark default branch (first one is typically default on GitHub)
        if let Some(first) = branches.first_mut() {
            first.default = true;
        }
        Ok(branches)
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!(
            "/projects/{}/repository/branches?per_page={}",
            encoded_path, client.page_size
        );
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_branches: Vec<GitlabBranch> = serde_json::from_str(&raw)?;
        Ok(gl_branches.into_iter().map(Branch::from).collect())
    }
}

pub async fn create_branch(
    client: &GitlabClient,
    project_path: &str,
    branch_name: &str,
    ref_branch: &str,
) -> Result<()> {
    if client.is_github {
        let endpoint = format!("/repos/{}/git/refs", project_path);
        let payload = serde_json::json!({
            "ref": format!("refs/heads/{}", branch_name),
            "sha": ref_branch,
        });
        let json_str = serde_json::to_string(&payload)?;
        let temp_path = std::env::temp_dir().join("glab-tui-create-branch.json");
        let _ = std::fs::write(&temp_path, &json_str);
        let path_str = temp_path.to_string_lossy().to_string();
        client
            .execute_github_api(&endpoint, "POST", Some(&path_str))
            .await?;
        let _ = std::fs::remove_file(&temp_path);
        Ok(())
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!(
            "/projects/{}/repository/branches?branch={}&ref={}",
            encoded_path, branch_name, ref_branch
        );
        client.execute_gitlab_api(&endpoint, "POST", None).await?;
        Ok(())
    }
}

pub async fn delete_branch(
    client: &GitlabClient,
    project_path: &str,
    branch_name: &str,
) -> Result<()> {
    if client.is_github {
        let endpoint = format!("/repos/{}/git/refs/heads/{}", project_path, branch_name);
        client.execute_github_api(&endpoint, "DELETE", None).await?;
        Ok(())
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!(
            "/projects/{}/repository/branches/{}",
            encoded_path, branch_name
        );
        client.execute_gitlab_api(&endpoint, "DELETE", None).await?;
        Ok(())
    }
}
