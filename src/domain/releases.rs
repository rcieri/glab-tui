use crate::domain::client::GitlabClient;
use crate::domain::issues::GithubUser;
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

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabReleaseCommit {
    pub id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabRelease {
    pub name: Option<String>,
    pub tag_name: String,
    pub released_at: String,
    pub description: Option<String>,
    pub author_name: Option<String>,
    pub commit: Option<GitlabReleaseCommit>,
    pub assets_link: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubAsset {
    pub browser_download_url: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubRelease {
    pub name: Option<String>,
    pub tag_name: String,
    pub published_at: Option<String>,
    pub body: Option<String>,
    pub author: Option<GithubUser>,
    #[serde(default)]
    pub assets: Vec<GithubAsset>,
}

impl From<GitlabRelease> for Release {
    fn from(gl: GitlabRelease) -> Self {
        let name = gl.name.unwrap_or_else(|| gl.tag_name.clone());
        let (commit_id, commit_title) = match gl.commit {
            Some(c) => (c.id, c.title),
            None => (None, None),
        };
        Self {
            name,
            tag_name: gl.tag_name,
            released_at: gl.released_at,
            description: gl.description,
            author_name: gl.author_name,
            commit_id,
            commit_title,
            assets_link: gl.assets_link,
        }
    }
}

impl From<GithubRelease> for Release {
    fn from(gh: GithubRelease) -> Self {
        let name = gh.name.unwrap_or_else(|| gh.tag_name.clone());
        let released_at = gh
            .published_at
            .as_deref()
            .map(|s| s.chars().take(10).collect::<String>())
            .unwrap_or_default();
        let author_name = gh.author.map(|u| u.login);
        let assets_link = gh
            .assets
            .first()
            .and_then(|a| a.browser_download_url.clone());
        Self {
            name,
            tag_name: gh.tag_name,
            released_at,
            description: gh.body,
            author_name,
            commit_id: None,
            commit_title: None,
            assets_link,
        }
    }
}

pub async fn list_releases(client: &GitlabClient, project_path: &str) -> Result<Vec<Release>> {
    if client.is_github {
        let endpoint = format!(
            "/repos/{}/releases?per_page={}",
            project_path, client.page_size
        );
        let raw = client.execute_github_api(&endpoint, "GET", None).await?;
        let gh_rels: Vec<GithubRelease> = serde_json::from_str(&raw)?;
        Ok(gh_rels.into_iter().map(Release::from).collect())
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!(
            "/projects/{}/releases?per_page={}",
            encoded_path, client.page_size
        );
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_rels: Vec<GitlabRelease> = serde_json::from_str(&raw)?;
        Ok(gl_rels.into_iter().map(Release::from).collect())
    }
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
        client
            .execute_raw_command(
                "gh",
                &[
                    "release",
                    "edit",
                    tag_name,
                    "-R",
                    gh_repo,
                    "-t",
                    name,
                    "-n",
                    description,
                ],
                "Editing Release",
            )
            .await?;
        Ok(())
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        client
            .execute_raw_command(
                "glab",
                &[
                    "release",
                    "update",
                    tag_name,
                    "-R",
                    &encoded_path,
                    "-n",
                    name,
                    "-N",
                    description,
                ],
                "Updating Release",
            )
            .await?;
        Ok(())
    }
}

pub async fn delete_release(
    client: &GitlabClient,
    project_path: &str,
    tag_name: &str,
) -> Result<()> {
    if client.is_github {
        let gh_repo = project_path;
        client
            .execute_raw_command(
                "gh",
                &["release", "delete", tag_name, "-R", gh_repo, "-y"],
                "Deleting Release",
            )
            .await?;
        Ok(())
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        client
            .execute_raw_command(
                "glab",
                &["release", "delete", tag_name, "-R", &encoded_path, "-y"],
                "Deleting Release",
            )
            .await?;
        Ok(())
    }
}
