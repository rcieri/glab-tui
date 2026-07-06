use super::client::GitlabClient;
use super::issues::{GithubIssue, Issue};
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

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabMilestone {
    pub id: u64,
    pub iid: u64,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub start_date: Option<String>,
    pub due_date: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubMilestone {
    pub id: u64,
    pub number: u64,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub due_on: Option<String>,
    pub created_at: String,
}

impl From<GitlabMilestone> for Milestone {
    fn from(gl: GitlabMilestone) -> Self {
        Self {
            id: gl.id,
            iid: gl.iid,
            title: gl.title,
            description: gl.description,
            state: gl.state,
            start_date: gl.start_date,
            due_date: gl.due_date,
            created_at: gl.created_at,
        }
    }
}

impl From<GithubMilestone> for Milestone {
    fn from(gh: GithubMilestone) -> Self {
        let state = if gh.state == "open" {
            "active"
        } else {
            "closed"
        }
        .to_string();
        let due_date = gh
            .due_on
            .as_deref()
            .map(|s| s.chars().take(10).collect::<String>());
        Self {
            id: gh.id,
            iid: gh.number,
            title: gh.title,
            description: gh.description,
            state,
            start_date: None,
            due_date,
            created_at: gh.created_at,
        }
    }
}

pub async fn list_milestones(client: &GitlabClient, project_path: &str) -> Result<Vec<Milestone>> {
    if client.is_github {
        let endpoint = format!("/repos/{}/milestones?state=all&per_page=100", project_path);
        let raw = client.execute_github_api(&endpoint, "GET", None).await?;
        let gh_milestones: Vec<GithubMilestone> = serde_json::from_str(&raw)?;
        Ok(gh_milestones.into_iter().map(Milestone::from).collect())
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!("/projects/{}/milestones?per_page=100", encoded_path);
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_milestones: Vec<GitlabMilestone> = serde_json::from_str(&raw)?;
        Ok(gl_milestones.into_iter().map(Milestone::from).collect())
    }
}

pub async fn list_milestone_issues(
    client: &GitlabClient,
    project_path: &str,
    milestone_iid: u64,
) -> Result<Vec<Issue>> {
    if client.is_github {
        let endpoint = format!(
            "/repos/{}/issues?milestone={}&state=all&per_page=100",
            project_path, milestone_iid
        );
        let raw = client.execute_github_api(&endpoint, "GET", None).await?;
        let gh_issues: Vec<GithubIssue> = serde_json::from_str(&raw)?;
        Ok(gh_issues
            .into_iter()
            .filter(|i| i.pull_request.is_none())
            .map(Issue::from)
            .collect())
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!(
            "/projects/{}/milestones/{}/issues?per_page=100",
            encoded_path, milestone_iid
        );
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_issues: Vec<crate::gitlab::issues::GitlabIssue> = serde_json::from_str(&raw)?;
        Ok(gl_issues.into_iter().map(Issue::from).collect())
    }
}

pub async fn update_milestone_state(
    client: &GitlabClient,
    project_path: &str,
    milestone_iid: u64,
    close: bool,
) -> Result<()> {
    if client.is_github {
        let state = if close { "closed" } else { "open" };
        client
            .execute_raw_command(
                "gh",
                &[
                    "api",
                    "-X",
                    "PATCH",
                    &format!("repos/{}/milestones/{}", project_path, milestone_iid),
                    "-F",
                    &format!("state={}", state),
                ],
                "Updating Milestone State",
            )
            .await?;
        Ok(())
    } else {
        let action = if close { "close" } else { "reopen" };
        let encoded_path = project_path.replace("/", "%2F");
        client
            .execute_raw_command(
                "glab",
                &[
                    "milestone",
                    action,
                    &milestone_iid.to_string(),
                    "-R",
                    &encoded_path,
                ],
                "Updating Milestone State",
            )
            .await?;
        Ok(())
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
        let mut iso_due_storage = String::new();
        if let Some(due) = due_date {
            if !due.is_empty() {
                let iso_due = if due.contains('T') {
                    due.to_string()
                } else {
                    format!("{}T23:59:59Z", due)
                };
                iso_due_storage = iso_due;
                args.push("-F".to_string());
                args.push(format!("due_on={}", iso_due_storage));
            }
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        client
            .execute_raw_command("gh", &args_refs, "Updating Milestone")
            .await?;
        Ok(())
    } else {
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
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        client
            .execute_raw_command("glab", &args_refs, "Updating Milestone")
            .await?;
        Ok(())
    }
}

pub async fn delete_milestone(
    client: &GitlabClient,
    project_path: &str,
    milestone_iid: u64,
) -> Result<()> {
    if client.is_github {
        client
            .execute_raw_command(
                "gh",
                &[
                    "api",
                    "-X",
                    "DELETE",
                    &format!("repos/{}/milestones/{}", project_path, milestone_iid),
                ],
                "Deleting Milestone",
            )
            .await?;
        Ok(())
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        client
            .execute_raw_command(
                "glab",
                &[
                    "milestone",
                    "delete",
                    &milestone_iid.to_string(),
                    "-R",
                    &encoded_path,
                    "-y",
                ],
                "Deleting Milestone",
            )
            .await?;
        Ok(())
    }
}
