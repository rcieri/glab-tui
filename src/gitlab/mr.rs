use super::client::GitlabClient;
use super::issues::{GithubLabel, GithubMilestone, GithubUser};
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
    pub head_pipeline: Option<super::pipelines::PipelineItem>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabMergeRequest {
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
    pub head_pipeline: Option<super::pipelines::GitlabPipeline>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubBranchInfo {
    pub r#ref: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubPullRequest {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub labels: Vec<GithubLabel>,
    pub updated_at: String,
    pub user: Option<GithubUser>,
    pub milestone: Option<GithubMilestone>,
    #[serde(default)]
    pub assignees: Vec<GithubUser>,
    #[serde(default)]
    pub requested_reviewers: Vec<GithubUser>,
    pub base: Option<GithubBranchInfo>,
    pub head: Option<GithubBranchInfo>,
    pub draft: Option<bool>,
    pub body: Option<String>,
}

impl From<GitlabMergeRequest> for MergeRequest {
    fn from(gl: GitlabMergeRequest) -> Self {
        Self {
            iid: gl.iid,
            title: gl.title,
            state: gl.state,
            labels: gl.labels,
            updated_at: gl.updated_at,
            author: gl.author,
            milestone: gl.milestone,
            assignees: gl.assignees,
            reviewers: gl.reviewers,
            target_branch: gl.target_branch,
            source_branch: gl.source_branch,
            draft: gl.draft,
            description: gl.description,
            head_pipeline: gl
                .head_pipeline
                .map(super::pipelines::PipelineItem::from_gitlab),
        }
    }
}

impl From<GithubPullRequest> for MergeRequest {
    fn from(gh: GithubPullRequest) -> Self {
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
        let reviewers = gh
            .requested_reviewers
            .into_iter()
            .map(|u| Reviewer { username: u.login })
            .collect();
        let target_branch = gh
            .base
            .map(|b| b.r#ref)
            .unwrap_or_else(|| "main".to_string());
        let source_branch = gh.head.map(|h| h.r#ref).unwrap_or_default();
        let draft = gh.draft.unwrap_or(false);
        Self {
            iid: gh.number,
            title: gh.title,
            state,
            labels,
            updated_at: gh.updated_at,
            author: Author { username },
            milestone: gh.milestone.map(|m| Milestone { title: m.title }),
            assignees,
            reviewers,
            target_branch,
            source_branch,
            draft,
            description: gh.body,
            head_pipeline: None,
        }
    }
}

pub async fn list_mrs(
    client: &GitlabClient,
    project_path: &str,
    show_closed: bool,
) -> Result<Vec<MergeRequest>> {
    let page_size = client.page_size.min(100);
    let pages_to_fetch = ((client.page_size + 99) / 100).max(1);
    let mut all_mrs = Vec::new();

    if client.is_github {
        let state_param = if show_closed { "all" } else { "open" };
        for page in 1..=pages_to_fetch {
            let endpoint = format!(
                "/repos/{}/pulls?state={}&per_page={}&page={}",
                project_path, state_param, page_size, page
            );
            let raw = match client.execute_github_api(&endpoint, "GET", None).await {
                Ok(r) => r,
                Err(e) => {
                    if page == 1 {
                        return Err(e);
                    } else {
                        break;
                    }
                }
            };
            let gh_prs: Vec<GithubPullRequest> = serde_json::from_str(&raw)?;
            let len = gh_prs.len();
            all_mrs.extend(gh_prs.into_iter().map(MergeRequest::from));
            if len < page_size {
                break;
            }
        }
        Ok(all_mrs)
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let state_param = if show_closed { "all" } else { "opened" };
        for page in 1..=pages_to_fetch {
            let endpoint = format!(
                "/projects/{}/merge_requests?state={}&per_page={}&page={}",
                encoded_path, state_param, page_size, page
            );
            let raw = match client.execute_gitlab_api(&endpoint, "GET", None).await {
                Ok(r) => r,
                Err(e) => {
                    if page == 1 {
                        return Err(e);
                    } else {
                        break;
                    }
                }
            };
            let gl_mrs: Vec<GitlabMergeRequest> = serde_json::from_str(&raw)?;
            let len = gl_mrs.len();
            all_mrs.extend(gl_mrs.into_iter().map(MergeRequest::from));
            if len < page_size {
                break;
            }
        }
        Ok(all_mrs)
    }
}

pub async fn get_mr(client: &GitlabClient, project_path: &str, iid: u64) -> Result<MergeRequest> {
    if client.is_github {
        let endpoint = format!("/repos/{}/pulls/{}", project_path, iid);
        let raw = client.execute_github_api(&endpoint, "GET", None).await?;
        let gh_pr: GithubPullRequest = serde_json::from_str(&raw)?;
        Ok(MergeRequest::from(gh_pr))
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!("/projects/{}/merge_requests/{}", encoded_path, iid);
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_mr: GitlabMergeRequest = serde_json::from_str(&raw)?;
        Ok(MergeRequest::from(gl_mr))
    }
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

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabDiscussionNote {
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

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubPullComment {
    pub id: u64,
    pub body: String,
    pub user: Option<GithubUser>,
    pub created_at: String,
    pub path: Option<String>,
    pub line: Option<u64>,
    #[serde(default = "default_side")]
    pub side: String,
    pub start_line: Option<u64>,
    pub in_reply_to_id: Option<u64>,
}

fn default_side() -> String {
    "RIGHT".to_string()
}

impl From<GitlabDiscussionNote> for DiscussionNote {
    fn from(gl: GitlabDiscussionNote) -> Self {
        Self {
            id: gl.id,
            body: gl.body,
            author: gl.author,
            created_at: gl.created_at,
            system: gl.system,
            position: gl.position,
            discussion_id: gl.discussion_id,
            resolved: gl.resolved,
            resolvable: gl.resolvable,
        }
    }
}

impl From<GithubPullComment> for DiscussionNote {
    fn from(gh: GithubPullComment) -> Self {
        let username = gh
            .user
            .map(|u| u.login)
            .unwrap_or_else(|| "unknown".to_string());

        let position = if let Some(p) = gh.path {
            let (new_line, old_line) = if gh.side == "LEFT" {
                (None, gh.line)
            } else {
                (gh.line, None)
            };
            Some(NotePosition {
                new_path: Some(p.clone()),
                old_path: Some(p),
                new_line,
                old_line,
                start_line: gh.start_line,
                line_range: None,
            })
        } else {
            None
        };

        let disc_id = gh
            .in_reply_to_id
            .map(|rid| rid.to_string())
            .unwrap_or_else(|| gh.id.to_string());

        Self {
            id: gh.id,
            body: gh.body,
            author: Author { username },
            created_at: gh.created_at,
            system: false,
            position,
            discussion_id: Some(disc_id),
            resolved: Some(false),
            resolvable: Some(true),
        }
    }
}

pub async fn list_mr_notes(
    client: &GitlabClient,
    project_path: &str,
    mr_iid: u64,
) -> Result<Vec<DiscussionNote>> {
    if client.is_github {
        let endpoint = format!(
            "/repos/{}/pulls/{}/comments?per_page={}",
            project_path, mr_iid, client.page_size
        );
        let raw = client.execute_github_api(&endpoint, "GET", None).await?;
        let gh_comments: Vec<GithubPullComment> = serde_json::from_str(&raw)?;
        Ok(gh_comments.into_iter().map(DiscussionNote::from).collect())
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!(
            "/projects/{}/merge_requests/{}/notes?per_page={}",
            encoded_path, mr_iid, client.page_size
        );
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_notes: Vec<GitlabDiscussionNote> = serde_json::from_str(&raw)?;
        Ok(gl_notes.into_iter().map(DiscussionNote::from).collect())
    }
}
