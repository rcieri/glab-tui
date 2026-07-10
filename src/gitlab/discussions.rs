use super::client::GitlabClient;
use super::mr::{Author, DiscussionNote, GitlabDiscussionNote};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Discussion {
    pub id: String,
    pub notes: Vec<DiscussionNote>,
    pub individual_note: bool,
    pub resolvable: bool,
    pub resolved: bool,
    pub resolved_by: Option<Author>,
    pub resolved_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubIssueComment {
    pub id: u64,
    pub body: String,
    pub user: Option<GithubIssueUser>,
    pub created_at: String,
    #[serde(default)]
    pub in_reply_to_id: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubIssueUser {
    pub login: String,
}

impl From<GithubIssueComment> for DiscussionNote {
    fn from(gh: GithubIssueComment) -> Self {
        let username = gh
            .user
            .map(|u| u.login)
            .unwrap_or_else(|| "unknown".to_string());

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
            position: None,
            discussion_id: Some(disc_id),
            resolved: Some(false),
            resolvable: Some(true),
        }
    }
}

/// GitHub issue comments are flat; we group them by in_reply_to_id into discussions.
/// For GitHub, each "discussion" is a single note unless it has replies.
fn group_github_comments(notes: Vec<DiscussionNote>) -> Vec<Discussion> {
    let mut discussions: Vec<Discussion> = Vec::new();
    let mut replies: Vec<DiscussionNote> = Vec::new();
    for note in notes {
        let is_reply = note
            .discussion_id
            .as_ref()
            .is_some_and(|id| id != &note.id.to_string());
        if is_reply {
            replies.push(note);
        } else {
            discussions.push(Discussion {
                id: note.id.to_string(),
                notes: vec![note],
                individual_note: true,
                resolvable: true,
                resolved: false,
                resolved_by: None,
                resolved_at: None,
            });
        }
    }
    for reply in replies {
        if let Some(ref parent_id) = reply.discussion_id {
            if let Some(disc) = discussions.iter_mut().find(|d| d.id == *parent_id) {
                disc.notes.push(reply);
            } else {
                discussions.push(Discussion {
                    id: reply.id.to_string(),
                    notes: vec![reply],
                    individual_note: true,
                    resolvable: true,
                    resolved: false,
                    resolved_by: None,
                    resolved_at: None,
                });
            }
        }
    }
    discussions
}

pub async fn list_issue_discussions(
    client: &GitlabClient,
    project_path: &str,
    issue_iid: u64,
) -> Result<Vec<Discussion>> {
    if client.is_github {
        let endpoint = format!(
            "/repos/{}/issues/{}/comments?per_page={}",
            project_path, issue_iid, client.page_size
        );
        let raw = client.execute_github_api(&endpoint, "GET", None).await?;
        let gh_comments: Vec<GithubIssueComment> = serde_json::from_str(&raw)?;
        let notes: Vec<DiscussionNote> =
            gh_comments.into_iter().map(DiscussionNote::from).collect();
        Ok(group_github_comments(notes))
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!(
            "/projects/{}/issues/{}/discussions?per_page={}",
            encoded_path, issue_iid, client.page_size
        );
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_discussions: Vec<GitlabDiscussion> = serde_json::from_str(&raw)?;
        Ok(gl_discussions.into_iter().map(Discussion::from).collect())
    }
}

pub async fn list_mr_discussions(
    client: &GitlabClient,
    project_path: &str,
    mr_iid: u64,
) -> Result<Vec<Discussion>> {
    if client.is_github {
        let endpoint = format!(
            "/repos/{}/pulls/{}/comments?per_page={}",
            project_path, mr_iid, client.page_size
        );
        let raw = client.execute_github_api(&endpoint, "GET", None).await?;
        let gh_comments: Vec<super::mr::GithubPullComment> = serde_json::from_str(&raw)?;
        let notes: Vec<DiscussionNote> =
            gh_comments.into_iter().map(DiscussionNote::from).collect();
        Ok(group_github_comments(notes))
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!(
            "/projects/{}/merge_requests/{}/discussions?per_page={}",
            encoded_path, mr_iid, client.page_size
        );
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_discussions: Vec<GitlabDiscussion> = serde_json::from_str(&raw)?;
        Ok(gl_discussions.into_iter().map(Discussion::from).collect())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabDiscussion {
    pub id: Value,
    pub notes: Vec<GitlabDiscussionNote>,
    pub individual_note: bool,
    #[serde(default)]
    pub resolvable: bool,
    #[serde(default)]
    pub resolved: bool,
    pub resolved_by: Option<Author>,
    pub resolved_at: Option<String>,
}

impl From<GitlabDiscussion> for Discussion {
    fn from(gd: GitlabDiscussion) -> Self {
        Self {
            id: format!("{}", gd.id),
            notes: gd.notes.into_iter().map(DiscussionNote::from).collect(),
            individual_note: gd.individual_note,
            resolvable: gd.resolvable,
            resolved: gd.resolved,
            resolved_by: gd.resolved_by,
            resolved_at: gd.resolved_at,
        }
    }
}

/// Create a new top-level note (comment) on an issue.
/// For GitHub, this posts to `/repos/:owner/:repo/issues/:number/comments`.
/// For GitLab, this posts to `/projects/:id/issues/:iid/notes`.
pub async fn add_issue_note(
    client: &GitlabClient,
    project_path: &str,
    issue_iid: u64,
    body: &str,
) -> anyhow::Result<()> {
    let encoded_path = project_path.replace("/", "%2F");
    let (endpoint, method) = if client.is_github {
        (
            format!("/repos/{}/issues/{}/comments", project_path, issue_iid),
            "POST",
        )
    } else {
        (
            format!("/projects/{}/issues/{}/notes", encoded_path, issue_iid),
            "POST",
        )
    };

    let payload = serde_json::json!({ "body": body });
    let json_str = serde_json::to_string(&payload)?;

    if client.is_github {
        client
            .execute_github_api(&endpoint, method, Some(&json_str))
            .await?;
    } else {
        client
            .execute_gitlab_api(&endpoint, method, Some(&json_str))
            .await?;
    }
    Ok(())
}

/// Create a new top-level note (comment) on a merge request.
/// For GitHub, this posts to `/repos/:owner/:repo/issues/:number/comments` (PRs use the issues endpoint on GH).
/// For GitLab, this posts to `/projects/:id/merge_requests/:iid/notes`.
pub async fn add_mr_note(
    client: &GitlabClient,
    project_path: &str,
    mr_iid: u64,
    body: &str,
) -> anyhow::Result<()> {
    let encoded_path = project_path.replace("/", "%2F");
    let (endpoint, method) = if client.is_github {
        (
            format!("/repos/{}/issues/{}/comments", project_path, mr_iid),
            "POST",
        )
    } else {
        (
            format!("/projects/{}/merge_requests/{}/notes", encoded_path, mr_iid),
            "POST",
        )
    };

    let payload = serde_json::json!({ "body": body });
    let json_str = serde_json::to_string(&payload)?;

    if client.is_github {
        client
            .execute_github_api(&endpoint, method, Some(&json_str))
            .await?;
    } else {
        client
            .execute_gitlab_api(&endpoint, method, Some(&json_str))
            .await?;
    }
    Ok(())
}

pub async fn add_discussion_reply(
    client: &GitlabClient,
    project_path: &str,
    mr_iid: Option<u64>,
    issue_iid: Option<u64>,
    discussion_id: &str,
    body: &str,
) -> anyhow::Result<()> {
    let encoded_path = project_path.replace("/", "%2F");
    let (endpoint, method) = if client.is_github {
        // GitHub doesn't have discussion threads; use issue/PR comment endpoint
        if let Some(iid) = issue_iid {
            (
                format!("/repos/{}/issues/{}/comments", project_path, iid),
                "POST",
            )
        } else if let Some(iid) = mr_iid {
            (
                format!("/repos/{}/pulls/{}/comments", project_path, iid),
                "POST",
            )
        } else {
            anyhow::bail!("No issue or MR iid provided");
        }
    } else if let Some(iid) = issue_iid {
        (
            format!(
                "/projects/{}/issues/{}/discussions/{}",
                encoded_path, iid, discussion_id
            ),
            "POST",
        )
    } else if let Some(iid) = mr_iid {
        (
            format!(
                "/projects/{}/merge_requests/{}/discussions/{}",
                encoded_path, iid, discussion_id
            ),
            "POST",
        )
    } else {
        anyhow::bail!("No issue or MR iid provided");
    };

    let payload = serde_json::json!({ "body": body });
    let json_str = serde_json::to_string(&payload)?;

    if client.is_github {
        client
            .execute_github_api(&endpoint, method, Some(&json_str))
            .await?;
    } else {
        client
            .execute_gitlab_api(&endpoint, method, Some(&json_str))
            .await?;
    }
    Ok(())
}

pub async fn toggle_discussion_resolution(
    client: &GitlabClient,
    project_path: &str,
    mr_iid: Option<u64>,
    issue_iid: Option<u64>,
    discussion_id: &str,
    resolved: bool,
) -> anyhow::Result<()> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = if let Some(iid) = issue_iid {
        format!(
            "/projects/{}/issues/{}/discussions/{}",
            encoded_path, iid, discussion_id
        )
    } else if let Some(iid) = mr_iid {
        format!(
            "/projects/{}/merge_requests/{}/discussions/{}",
            encoded_path, iid, discussion_id
        )
    } else {
        anyhow::bail!("No issue or MR iid provided");
    };

    // GitLab uses PUT with query param; GitHub doesn't support resolve
    if client.is_github {
        anyhow::bail!("Resolving discussions is not supported on GitHub via API");
    }
    let resolve_endpoint = format!("{}?resolved={}", endpoint, resolved);
    let payload = serde_json::json!({ "resolved": resolved });
    let json_str = serde_json::to_string(&payload)?;

    client
        .execute_gitlab_api(&resolve_endpoint, "PUT", Some(&json_str))
        .await?;
    Ok(())
}
