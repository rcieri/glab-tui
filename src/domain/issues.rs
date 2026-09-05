use crate::domain::client::GitlabClient;
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
    #[serde(default)]
    pub web_url: String,
    #[serde(default)]
    pub project_path: String,
}

impl Issue {
    pub fn markdown_reference(&self) -> String {
        let title = self
            .title
            .replace('\\', "\\\\")
            .replace('[', "\\[")
            .replace(']', "\\]");
        format!("[#{}: {}]({})", self.iid, title, self.web_url)
    }
}

pub async fn list_issues(
    client: &GitlabClient,
    scope: &crate::scope::Scope,
    show_closed: bool,
) -> Result<Vec<Issue>> {
    client
        .backend
        .list_issues(scope, show_closed, client.page_size, client.api_per_page)
        .await
}

#[allow(dead_code)]
pub async fn get_issue(client: &GitlabClient, project_path: &str, iid: u64) -> Result<Issue> {
    client.backend.get_issue(project_path, iid).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_reference_is_a_markdown_link_with_id_and_title() {
        let issue: Issue = serde_json::from_str(
            r#"{
                "iid": 42,
                "title": "Fix parser",
                "state": "opened",
                "labels": [],
                "updated_at": "2026-07-03T00:00:00Z",
                "author": { "username": "testuser" },
                "web_url": "https://github.com/acme/project/issues/42"
            }"#,
        )
        .unwrap();

        assert_eq!(
            issue.markdown_reference(),
            "[#42: Fix parser](https://github.com/acme/project/issues/42)"
        );
    }

    #[test]
    fn issue_reference_escapes_markdown_link_text() {
        let issue: Issue = serde_json::from_str(
            r#"{
                "iid": 42,
                "title": "Fix [parser] \\ paths",
                "state": "opened",
                "labels": [],
                "updated_at": "2026-07-03T00:00:00Z",
                "author": { "username": "testuser" },
                "web_url": "https://github.com/acme/project/issues/42"
            }"#,
        )
        .unwrap();

        assert_eq!(
            issue.markdown_reference(),
            r"[#42: Fix \[parser\] \\ paths](https://github.com/acme/project/issues/42)"
        );
    }

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
