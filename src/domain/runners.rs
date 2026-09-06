use crate::domain::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Runner {
    pub id: u64,
    pub description: Option<String>,
    pub status: String,
    pub active: bool,
}

pub async fn list_runners(
    client: &GitlabClient,
    scope: &crate::scope::Scope,
) -> Result<Vec<Runner>> {
    client.backend.list_runners(scope, client.page_size).await
}
