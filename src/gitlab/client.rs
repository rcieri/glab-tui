use anyhow::{Context, Result};
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct GitlabClient;

impl GitlabClient {
    pub async fn new() -> Result<Self> {
        Ok(Self)
    }

    pub async fn fetch_api<T: serde::de::DeserializeOwned>(&self, endpoint: &str) -> Result<T> {
        let output = Command::new("glab")
            .args(["api", endpoint])
            .output()
            .await
            .context("Failed to execute glab api command")?;

        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("glab api failed: {}", err_msg);
        }

        let data: T = serde_json::from_slice(&output.stdout)?;
        Ok(data)
    }
}

pub async fn get_project_context() -> Result<String> {
    // Execute `git remote get-url origin`
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .context("Failed to execute git command")?;

    if !output.status.success() {
        return Ok("unknown/unknown".to_string());
    }

    let url = String::from_utf8(output.stdout)?.trim().to_string();
    
    // Parse url to extract namespace/repo
    let path = if url.starts_with("git@") {
        url.split(':').nth(1).unwrap_or("unknown/unknown")
    } else if url.starts_with("http") {
        let parts: Vec<&str> = url.split('/').collect();
        if parts.len() >= 2 {
            let p = format!("{}/{}", parts[parts.len()-2], parts[parts.len()-1]);
            return Ok(p.trim_end_matches(".git").to_string());
        }
        "unknown/unknown"
    } else {
        "unknown/unknown"
    };

    Ok(path.trim_end_matches(".git").to_string())
}


