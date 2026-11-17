use crate::domain::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, Default)]
pub struct Branch {
    pub name: String,
    pub default: bool,
    pub protected: bool,
    pub web_url: String,
    #[serde(default)]
    pub can_push: bool,
    #[serde(default)]
    pub commit_sha: String,
    #[serde(default)]
    pub commit_message: Option<String>,
    #[serde(default)]
    pub author_name: Option<String>,
    #[serde(default)]
    pub author_email: Option<String>,
    #[serde(default)]
    pub commit_date: Option<String>,
    #[serde(default)]
    pub ahead_behind: Option<(usize, usize)>,
}

pub fn enrich_branches(branches: &mut [Branch]) {
    if crate::git_helpers::repo_root().is_none() {
        return;
    }
    let base_sha = branches
        .iter()
        .find(|b| b.default)
        .map(|b| b.commit_sha.clone())
        .filter(|s| !s.is_empty());

    for branch in branches.iter_mut() {
        if branch.author_name.is_none()
            || branch.commit_message.is_none()
            || branch.commit_date.is_none()
        {
            let rev = if !branch.commit_sha.is_empty() {
                &branch.commit_sha
            } else {
                &branch.name
            };
            if let Some(info) = crate::git_helpers::get_commit_info(rev) {
                if branch.commit_message.is_none() && !info.message.is_empty() {
                    branch.commit_message = Some(info.message);
                }
                if branch.author_name.is_none() && !info.author_name.is_empty() {
                    branch.author_name = Some(info.author_name);
                }
                if branch.author_email.is_none() && !info.author_email.is_empty() {
                    branch.author_email = Some(info.author_email);
                }
                if branch.commit_date.is_none() && !info.date.is_empty() {
                    branch.commit_date = Some(info.date);
                }
            }
        }
        if branch.ahead_behind.is_none() {
            branch.ahead_behind = if branch.default {
                Some((0, 0))
            } else {
                base_sha
                    .as_deref()
                    .and_then(|base| crate::git_helpers::get_ahead_behind(base, &branch.commit_sha))
            };
        }
    }
}

pub async fn list_branches(
    client: &GitlabClient,
    scope: &crate::scope::Scope,
) -> Result<Vec<Branch>> {
    let branches = client
        .backend
        .list_branches(scope, client.page_size)
        .await?;
    let branches = tokio::task::spawn_blocking(move || {
        let mut branches = branches;
        enrich_branches(&mut branches);
        branches
    })
    .await?;
    Ok(branches)
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
