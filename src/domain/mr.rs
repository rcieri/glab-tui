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
    /// From `blocking_discussions_resolved` in the GitLab list response.
    /// `None` on GitHub, which has no equivalent list field.
    /// Deliberately NOT inside `MergeabilityState`: it comes free from REST,
    /// so it must survive a GraphQL outage.
    #[serde(default)]
    pub blocking_discussions_resolved: Option<bool>,
    /// Populated after the list fetch. `None` means unknown, never "unapproved".
    #[serde(default)]
    pub approval: Option<crate::domain::mr_state::ApprovalState>,
    /// Populated after the list fetch. `None` means unknown, never "clean".
    #[serde(default)]
    pub mergeability: Option<crate::domain::mr_state::MergeabilityState>,
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
        .list_mrs(
            project_path,
            show_closed,
            client.page_size,
            client.api_per_page,
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real `glab mr list --output json` response.
    const GLAB_MR_JSON: &str = r#"{
        "iid": 1471,
        "title": "wire up webhooks",
        "state": "opened",
        "updated_at": "2026-07-29T15:11:38.322Z",
        "author": { "username": "chandler.anderson" },
        "milestone": null,
        "target_branch": "main",
        "draft": false,
        "description": null,
        "blocking_discussions_resolved": false
    }"#;

    #[test]
    fn deserializes_blocking_discussions_resolved_from_glab_list() {
        let mr: MergeRequest = serde_json::from_str(GLAB_MR_JSON).unwrap();
        assert_eq!(mr.blocking_discussions_resolved, Some(false));
    }

    #[test]
    fn state_axes_default_to_none_when_absent() {
        let mr: MergeRequest = serde_json::from_str(GLAB_MR_JSON).unwrap();
        assert!(mr.approval.is_none());
        assert!(mr.mergeability.is_none());
    }

    #[test]
    fn missing_discussions_field_is_none_not_false() {
        // GitHub's mapping never sets it; unknown must not read as a problem.
        let json = GLAB_MR_JSON.replace(",\n        \"blocking_discussions_resolved\": false", "");
        let mr: MergeRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(mr.blocking_discussions_resolved, None);
    }
}
