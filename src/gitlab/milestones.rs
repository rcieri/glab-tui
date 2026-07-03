use super::client::GitlabClient;
use super::issues::Issue;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Milestone {
    pub id: u64,
    pub iid: u64,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub start_date: Option<String>,
    pub due_date: Option<String>,
    pub created_at: String,
}

pub async fn list_milestones(client: &GitlabClient, project_path: &str) -> Result<Vec<Milestone>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!("/projects/{}/milestones?per_page=100", encoded_path);
    client.fetch_api(&endpoint).await
}

pub async fn list_milestone_issues(
    client: &GitlabClient,
    project_path: &str,
    milestone_iid: u64,
) -> Result<Vec<Issue>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!(
        "/projects/{}/milestones/{}/issues?per_page=100",
        encoded_path, milestone_iid
    );
    client.fetch_api(&endpoint).await
}

pub async fn update_milestone_state(
    client: &GitlabClient,
    project_path: &str,
    milestone_iid: u64,
    close: bool,
) -> Result<()> {
    if client.is_github {
        let state = if close { "closed" } else { "open" };
        let out = tokio::process::Command::new("gh")
            .args([
                "api",
                "-X",
                "PATCH",
                &format!("repos/{}/milestones/{}", project_path, milestone_iid),
                "-F",
                &format!("state={}", state),
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
        let action = if close { "close" } else { "reopen" };
        let encoded_path = project_path.replace("/", "%2F");
        let out = tokio::process::Command::new("glab")
            .args([
                "milestone",
                action,
                &milestone_iid.to_string(),
                "-R",
                &encoded_path,
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

pub async fn update_milestone(
    client: &GitlabClient,
    project_path: &str,
    milestone_iid: u64,
    title: &str,
    description: &str,
    start_date: Option<&str>,
    due_date: Option<&str>,
) -> Result<()> {
    if client.is_github {
        // gh api PATCH /repos/{owner}/{repo}/milestones/{milestone_number}
        let mut args = vec![
            "api".to_string(),
            "-X".to_string(),
            "PATCH".to_string(),
            format!("repos/{}/milestones/{}", project_path, milestone_iid),
            "-F".to_string(),
            format!("title={}", title),
            "-F".to_string(),
            format!("description={}", description),
        ];
        if let Some(due) = due_date {
            if !due.is_empty() {
                // GitHub expects ISO 8601, e.g. YYYY-MM-DDTHH:MM:SSZ
                let iso_due = if due.contains('T') {
                    due.to_string()
                } else {
                    format!("{}T23:59:59Z", due)
                };
                args.push("-F".to_string());
                args.push(format!("due_on={}", iso_due));
            }
        }
        let out = tokio::process::Command::new("gh")
            .args(&args)
            .output()
            .await?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            anyhow::bail!("GitHub API error: {}", err);
        }
    } else {
        // glab milestone update <id> --title <title> --description <description> --start-date <start-date> --due-date <due-date>
        let encoded_path = project_path.replace("/", "%2F");
        let mut args = vec![
            "milestone".to_string(),
            "update".to_string(),
            milestone_iid.to_string(),
            "-R".to_string(),
            encoded_path,
            "--title".to_string(),
            title.to_string(),
            "--description".to_string(),
            description.to_string(),
        ];
        if let Some(start) = start_date {
            if !start.is_empty() {
                args.push("--start-date".to_string());
                args.push(start.to_string());
            }
        }
        if let Some(due) = due_date {
            if !due.is_empty() {
                args.push("--due-date".to_string());
                args.push(due.to_string());
            }
        }
        let out = tokio::process::Command::new("glab")
            .args(&args)
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

pub async fn delete_milestone(
    client: &GitlabClient,
    project_path: &str,
    milestone_iid: u64,
) -> Result<()> {
    if client.is_github {
        let out = tokio::process::Command::new("gh")
            .args([
                "api",
                "-X",
                "DELETE",
                &format!("repos/{}/milestones/{}", project_path, milestone_iid),
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
                "milestone",
                "delete",
                &milestone_iid.to_string(),
                "-R",
                &encoded_path,
                "-y",
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
