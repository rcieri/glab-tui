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
pub struct Reviewer {
    pub username: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MergeRequest {
    pub iid: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub labels: Vec<String>,
    pub updated_at: String,
    pub author: Author,
    pub milestone: Option<Milestone>,
    #[serde(default)]
    pub assignees: Vec<Assignee>,
    #[serde(default)]
    pub reviewers: Vec<Reviewer>,
    pub target_branch: String,
    #[serde(default)]
    pub source_branch: String,
    pub draft: bool,
    pub description: Option<String>,
    #[serde(default)]
    pub head_pipeline: Option<crate::domain::pipelines::Pipeline>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct NotePosition {
    #[serde(default)]
    pub new_path: Option<String>,
    #[serde(default)]
    pub old_path: Option<String>,
    #[serde(default)]
    pub new_line: Option<u64>,
    #[serde(default)]
    pub old_line: Option<u64>,
    #[serde(default)]
    pub start_line: Option<u64>,
    #[serde(default)]
    pub line_range: Option<serde_json::Value>,
}

impl NotePosition {
    pub fn get_line_range(&self) -> (Option<u64>, Option<u64>, Option<u64>, Option<u64>) {
        let mut start_new = self.new_line;
        let mut end_new = self.new_line;
        let mut start_old = self.old_line;
        let mut end_old = self.old_line;

        if let Some(ref lr) = self.line_range {
            if let Some(start_obj) = lr.get("start") {
                if let Some(nl) = start_obj.get("new_line").and_then(|v| v.as_u64()) {
                    start_new = Some(nl);
                }
                if let Some(ol) = start_obj.get("old_line").and_then(|v| v.as_u64()) {
                    start_old = Some(ol);
                }
            }
            if let Some(end_obj) = lr.get("end") {
                if let Some(nl) = end_obj.get("new_line").and_then(|v| v.as_u64()) {
                    end_new = Some(nl);
                }
                if let Some(ol) = end_obj.get("old_line").and_then(|v| v.as_u64()) {
                    end_old = Some(ol);
                }
            }
        }

        if let Some(sl) = self.start_line {
            if self.new_line.is_some() {
                start_new = Some(sl);
            } else if self.old_line.is_some() {
                start_old = Some(sl);
            }
        }

        (start_new, end_new, start_old, end_old)
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DiscussionNote {
    pub id: u64,
    pub body: String,
    pub author: Author,
    pub created_at: String,
    pub system: bool,
    #[serde(default)]
    pub position: Option<NotePosition>,
    #[serde(default)]
    pub discussion_id: Option<String>,
    #[serde(default)]
    pub resolved: Option<bool>,
    #[serde(default)]
    pub resolvable: Option<bool>,
}

pub async fn list_mrs(
    client: &GitlabClient,
    project_path: &str,
    show_closed: bool,
) -> Result<Vec<MergeRequest>> {
    client
        .backend
        .list_mrs(project_path, show_closed, client.page_size)
        .await
}

#[allow(dead_code)]
pub async fn get_mr(client: &GitlabClient, project_path: &str, iid: u64) -> Result<MergeRequest> {
    client.backend.get_mr(project_path, iid).await
}

pub async fn list_mr_notes(
    client: &GitlabClient,
    project_path: &str,
    mr_iid: u64,
) -> Result<Vec<DiscussionNote>> {
    client
        .backend
        .list_mr_notes(project_path, mr_iid, client.page_size)
        .await
}
