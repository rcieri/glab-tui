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

pub async fn update_release(
    client: &GitlabClient,
    project_path: &str,
    tag_name: &str,
    name: &str,
    description: &str,
) -> Result<()> {
    if client.is_github {
        let gh_repo = project_path;
        let out = tokio::process::Command::new("gh")
            .args([
                "release",
                "edit",
                tag_name,
                "-R",
                gh_repo,
                "-t",
                name,
                "-n",
                description,
            ])
            .output()
            .await?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            anyhow::bail!("GitHub API error: {}", err);
        }
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let out = tokio::process::Command::new("glab")
            .args([
                "release",
                "update",
                tag_name,
                "-R",
                &encoded_path,
                "-n",
                name,
                "-N",
                description,
            ])
            .output()
            .await?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            anyhow::bail!("GitLab API error: {}", err);
        }
    }
}

pub async fn delete_release(
    client: &GitlabClient,
    project_path: &str,
    tag_name: &str,
) -> Result<()> {
    if client.is_github {
        let gh_repo = project_path;
        let out = tokio::process::Command::new("gh")
            .args(["release", "delete", tag_name, "-R", gh_repo, "-y"])
            .output()
            .await?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            anyhow::bail!("GitHub API error: {}", err);
        }
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let out = tokio::process::Command::new("glab")
            .args(["release", "delete", tag_name, "-R", &encoded_path, "-y"])
            .output()
            .await?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            anyhow::bail!("GitLab API error: {}", err);
        }
    }
}
