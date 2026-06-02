use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;
use super::client::GitlabClient;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub project_path: String,
    pub title: String,
    pub target_type: String, // "Issue" or "MergeRequest"
    pub target_iid: u64,
    pub state: String, // "unread" or "pending"
    pub updated_at: String,
}

pub async fn list_notifications(client: &GitlabClient) -> Result<Vec<Notification>> {
    if client.is_github {
        let output = Command::new("gh")
            .args(["api", "notifications"])
            .output()
            .await
            .context("Failed to run gh api notifications")?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("gh api notifications failed: {}", err);
        }

        let json: Value = serde_json::from_slice(&output.stdout)?;
        let mut list = Vec::new();
        if let Some(arr) = json.as_array() {
            for item in arr {
                let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let project_path = item.get("repository").and_then(|r| r.get("full_name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let subject = item.get("subject");
                let title = subject.and_then(|s| s.get("title")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let ttype_raw = subject.and_then(|s| s.get("type")).and_then(|v| v.as_str()).unwrap_or("Issue");
                let target_type = if ttype_raw == "PullRequest" { "MergeRequest".to_string() } else { "Issue".to_string() };
                
                let url = subject.and_then(|s| s.get("url")).and_then(|v| v.as_str()).unwrap_or("");
                let target_iid = url.split('/').last().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                
                let unread = item.get("unread").and_then(|v| v.as_bool()).unwrap_or(true);
                let state = if unread { "unread".to_string() } else { "read".to_string() };
                let updated_at = item.get("updated_at").and_then(|v| v.as_str()).unwrap_or("").to_string();
                
                list.push(Notification {
                    id,
                    project_path,
                    title,
                    target_type,
                    target_iid,
                    state,
                    updated_at,
                });
            }
        }
        Ok(list)
    } else {
        let output = Command::new("glab")
            .args(["api", "todos"])
            .output()
            .await
            .context("Failed to run glab api todos")?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("glab api todos failed: {}", err);
        }

        let json: Value = serde_json::from_slice(&output.stdout)?;
        let mut list = Vec::new();
        if let Some(arr) = json.as_array() {
            for item in arr {
                let id = item.get("id").map(|v| v.to_string()).unwrap_or_default();
                let project_path = item.get("project").and_then(|p| p.get("path_with_namespace")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                
                let target = item.get("target");
                let title = target.and_then(|t| t.get("title")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let target_iid = target.and_then(|t| t.get("iid")).and_then(|v| v.as_u64()).unwrap_or(0);
                
                let target_type = item.get("target_type").and_then(|v| v.as_str()).unwrap_or("Issue").to_string();
                let state = item.get("state").and_then(|v| v.as_str()).unwrap_or("pending").to_string();
                let updated_at = item.get("updated_at").and_then(|v| v.as_str()).unwrap_or("").to_string();
                
                list.push(Notification {
                    id,
                    project_path,
                    title,
                    target_type,
                    target_iid,
                    state,
                    updated_at,
                });
            }
        }
        Ok(list)
    }
}

pub async fn mark_notification_as_read(client: &GitlabClient, id: &str) -> Result<()> {
    if client.is_github {
        let endpoint = format!("notifications/threads/{}", id);
        let output = Command::new("gh")
            .args(["api", "--method", "PATCH", &endpoint])
            .output()
            .await
            .context("Failed to mark github notification as read")?;
            
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("gh api mark as read failed: {}", err);
        }
        Ok(())
    } else {
        let endpoint = format!("todos/{}/mark_as_done", id);
        let output = Command::new("glab")
            .args(["api", "--method", "POST", &endpoint])
            .output()
            .await
            .context("Failed to mark gitlab todo as done")?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("glab api mark as done failed: {}", err);
        }
        Ok(())
    }
}
