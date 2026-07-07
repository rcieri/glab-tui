use super::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Author {
    pub username: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Milestone {
    pub title: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Assignee {
    pub username: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Issue {
    pub iid: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub labels: Vec<String>,
    pub updated_at: String,
    pub created_at: Option<String>,
    pub closed_at: Option<String>,
    pub author: Author,
    pub milestone: Option<Milestone>,
    #[serde(default)]
    pub assignees: Vec<Assignee>,
    pub description: Option<String>,
    #[serde(default)]
    pub due_date: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabIssue {
    pub iid: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub labels: Vec<String>,
    pub updated_at: String,
    pub created_at: Option<String>,
    pub closed_at: Option<String>,
    pub author: Author,
    pub milestone: Option<Milestone>,
    #[serde(default)]
    pub assignees: Vec<Assignee>,
    pub description: Option<String>,
    #[serde(default)]
    pub due_date: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubUser {
    pub login: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubLabel {
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubMilestone {
    pub title: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubIssue {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub labels: Vec<GithubLabel>,
    pub updated_at: String,
    pub created_at: Option<String>,
    pub closed_at: Option<String>,
    pub user: Option<GithubUser>,
    pub milestone: Option<GithubMilestone>,
    #[serde(default)]
    pub assignees: Vec<GithubUser>,
    pub body: Option<String>,
    pub pull_request: Option<serde_json::Value>,
}

impl From<GitlabIssue> for Issue {
    fn from(gl: GitlabIssue) -> Self {
        Self {
            iid: gl.iid,
            title: gl.title,
            state: gl.state,
            labels: gl.labels,
            updated_at: gl.updated_at,
            created_at: gl.created_at,
            closed_at: gl.closed_at,
            author: gl.author,
            milestone: gl.milestone,
            assignees: gl.assignees,
            description: gl.description,
            due_date: gl.due_date,
        }
    }
}

impl From<GithubIssue> for Issue {
    fn from(gh: GithubIssue) -> Self {
        let state = if gh.state == "open" {
            "opened"
        } else {
            "closed"
        }
        .to_string();
        let labels = gh.labels.into_iter().map(|l| l.name).collect();
        let username = gh
            .user
            .map(|u| u.login)
            .unwrap_or_else(|| "unknown".to_string());
        let assignees = gh
            .assignees
            .into_iter()
            .map(|u| Assignee { username: u.login })
            .collect();
        Self {
            iid: gh.number,
            title: gh.title,
            state,
            labels,
            updated_at: gh.updated_at,
            created_at: gh.created_at,
            closed_at: gh.closed_at,
            author: Author { username },
            milestone: gh.milestone.map(|m| Milestone { title: m.title }),
            assignees,
            description: gh.body,
            due_date: None,
        }
    }
}

pub async fn list_issues(
    client: &GitlabClient,
    project_path: &str,
    show_closed: bool,
) -> Result<Vec<Issue>> {
    if client.is_github {
        let state_param = if show_closed { "all" } else { "open" };
        let endpoint = format!(
            "/repos/{}/issues?state={}&per_page=100",
            project_path, state_param
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
        let state_param = if show_closed { "all" } else { "opened" };
        let endpoint = format!(
            "/projects/{}/issues?state={}&per_page=100",
            encoded_path, state_param
        );
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_issues: Vec<GitlabIssue> = serde_json::from_str(&raw)?;
        Ok(gl_issues.into_iter().map(Issue::from).collect())
    }
}

pub async fn get_issue(client: &GitlabClient, project_path: &str, iid: u64) -> Result<Issue> {
    if client.is_github {
        let endpoint = format!("/repos/{}/issues/{}", project_path, iid);
        let raw = client.execute_github_api(&endpoint, "GET", None).await?;
        let gh_issue: GithubIssue = serde_json::from_str(&raw)?;
        Ok(Issue::from(gh_issue))
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!("/projects/{}/issues/{}", encoded_path, iid);
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_issue: GitlabIssue = serde_json::from_str(&raw)?;
        Ok(Issue::from(gl_issue))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_issue_due_date() {
        let json_data = r#"{
            "iid": 42,
            "title": "Test Issue",
            "state": "opened",
            "labels": ["bug"],
            "updated_at": "2026-07-03T00:00:00Z",
            "author": { "username": "testuser" },
            "due_date": "2026-07-15"
        }"#;

        let issue: Issue = serde_json::from_str(json_data).unwrap();
        assert_eq!(issue.iid, 42);
        assert_eq!(issue.due_date, Some("2026-07-15".to_string()));

        let json_no_due_date = r#"{
            "iid": 43,
            "title": "Test Issue No Due Date",
            "state": "opened",
            "labels": [],
            "updated_at": "2026-07-03T00:00:00Z",
            "author": { "username": "testuser" }
        }"#;

        let issue_no_due: Issue = serde_json::from_str(json_no_due_date).unwrap();
        assert_eq!(issue_no_due.iid, 43);
        assert_eq!(issue_no_due.due_date, None);
    }
}
