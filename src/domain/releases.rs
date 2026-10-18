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
    #[serde(default)]
    pub project_path: String,
}

impl Release {
    /// The project this release belongs to, or the repository in scope when
    /// the listing left it empty (caches written before the field existed).
    pub fn project_or<'a>(&'a self, scope: &'a crate::scope::Scope) -> &'a str {
        if self.project_path.is_empty() {
            scope.as_str()
        } else {
            &self.project_path
        }
    }
}

pub async fn list_releases(
    client: &GitlabClient,
    scope: &crate::scope::Scope,
) -> Result<Vec<Release>> {
    client.backend.list_releases(scope, client.page_size).await
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
