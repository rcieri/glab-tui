//! Commits, comments, review threads and reviews people made in a project,
//! read from its activity feed (GitLab project events, GitHub repository
//! events). Opening, merging and closing issues and MRs/PRs are left out:
//! the issue and MR/PR lists already carry them.

use crate::domain::client::GitlabClient;
use crate::scope::Scope;
use serde::Deserialize;

pub async fn list_contributions(
    client: &GitlabClient,
    scope: &Scope,
) -> anyhow::Result<Vec<Contribution>> {
    client.backend.list_contributions(scope).await
}

/// How many of the newest feed events one fetch reads.
pub const EVENTS_PER_FETCH: usize = 100;

const SHORT_SHA_CHARS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContributionKind {
    Commit,
    Comment,
    /// A comment in a discussion thread or on a diff line.
    ThreadComment,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Issue,
    MergeRequest,
}

/// The issue or MR/PR a contribution was made on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributionTarget {
    pub kind: TargetKind,
    pub iid: u64,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contribution {
    pub author: String,
    pub kind: ContributionKind,
    /// RFC 3339.
    pub created_at: String,
    /// The commit title, the comment's first line, or the review verdict.
    pub summary: String,
    pub target: Option<ContributionTarget>,
    /// The branch and short SHA of a commit, or the file a diff comment is on.
    pub location: Option<String>,
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn short_sha(sha: &str) -> &str {
    sha.get(..SHORT_SHA_CHARS).unwrap_or(sha)
}

fn branch_name(r#ref: &str) -> &str {
    r#ref.strip_prefix("refs/heads/").unwrap_or(r#ref)
}

#[derive(Deserialize)]
struct GlEvent {
    action_name: String,
    #[serde(default)]
    author_username: String,
    created_at: String,
    #[serde(default)]
    target_type: Option<String>,
    #[serde(default)]
    target_iid: Option<u64>,
    #[serde(default)]
    target_title: Option<String>,
    #[serde(default)]
    push_data: Option<GlPushData>,
    #[serde(default)]
    note: Option<GlNote>,
}

#[derive(Deserialize)]
struct GlPushData {
    #[serde(default)]
    commit_count: u64,
    #[serde(default)]
    r#ref: Option<String>,
    #[serde(default)]
    commit_to: Option<String>,
    #[serde(default)]
    commit_title: Option<String>,
}

#[derive(Deserialize)]
struct GlNote {
    #[serde(default)]
    body: String,
    #[serde(default)]
    noteable_type: Option<String>,
    #[serde(default)]
    noteable_iid: Option<u64>,
    #[serde(default)]
    position: Option<GlNotePosition>,
}

#[derive(Deserialize)]
struct GlNotePosition {
    #[serde(default)]
    new_path: Option<String>,
}

fn gitlab_target_kind(noteable_type: &str) -> Option<TargetKind> {
    match noteable_type {
        "Issue" => Some(TargetKind::Issue),
        "MergeRequest" => Some(TargetKind::MergeRequest),
        _ => None,
    }
}

fn gitlab_contribution(event: GlEvent) -> Option<Contribution> {
    let base = |kind, summary, target, location| Contribution {
        author: event.author_username.clone(),
        kind,
        created_at: event.created_at.clone(),
        summary,
        target,
        location,
    };
    if let Some(push) = &event.push_data {
        if push.commit_count == 0 {
            return None;
        }
        let branch = push.r#ref.as_deref().unwrap_or_default();
        let sha = push.commit_to.as_deref().map(short_sha).unwrap_or_default();
        let mut summary = push.commit_title.clone().unwrap_or_default();
        if push.commit_count > 1 {
            summary = format!("{summary} (+{} more)", push.commit_count - 1);
        }
        return Some(base(
            ContributionKind::Commit,
            summary,
            None,
            Some(format!("{branch} {sha}").trim().to_string()),
        ));
    }
    if let Some(note) = &event.note {
        let target = note
            .noteable_type
            .as_deref()
            .and_then(gitlab_target_kind)
            .zip(note.noteable_iid)
            .map(|(kind, iid)| ContributionTarget {
                kind,
                iid,
                title: event.target_title.clone().unwrap_or_default(),
            });
        let kind = match event.target_type.as_deref() {
            Some("DiffNote" | "DiscussionNote") => ContributionKind::ThreadComment,
            _ => ContributionKind::Comment,
        };
        let path = note.position.as_ref().and_then(|p| p.new_path.clone());
        return Some(base(kind, first_line(&note.body), target, path));
    }
    if event.action_name == "approved" {
        let target = event.target_iid.map(|iid| ContributionTarget {
            kind: TargetKind::MergeRequest,
            iid,
            title: event.target_title.clone().unwrap_or_default(),
        });
        return Some(base(
            ContributionKind::Review,
            "approved".to_string(),
            target,
            None,
        ));
    }
    None
}

/// Contributions in a GitLab `/projects/:id/events` reply, newest first.
pub fn parse_gitlab_events(raw: &str) -> anyhow::Result<Vec<Contribution>> {
    let events: Vec<GlEvent> = serde_json::from_str(raw)?;
    Ok(events.into_iter().filter_map(gitlab_contribution).collect())
}

#[derive(Deserialize)]
struct GhEvent {
    r#type: String,
    actor: GhActor,
    created_at: String,
    #[serde(default)]
    payload: serde_json::Value,
}

#[derive(Deserialize)]
struct GhActor {
    login: String,
}

#[derive(Deserialize)]
struct GhPushPayload {
    #[serde(default)]
    r#ref: String,
    #[serde(default)]
    head: String,
    #[serde(default)]
    commits: Vec<GhPushCommit>,
}

#[derive(Deserialize)]
struct GhPushCommit {
    #[serde(default)]
    sha: String,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct GhIssueCommentPayload {
    #[serde(default)]
    action: String,
    comment: GhComment,
    issue: GhIssue,
}

#[derive(Deserialize)]
struct GhIssue {
    number: u64,
    #[serde(default)]
    title: String,
    /// Present when the issue is a pull request.
    #[serde(default)]
    pull_request: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct GhComment {
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Deserialize)]
struct GhPullRequest {
    number: u64,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize)]
struct GhReviewCommentPayload {
    comment: GhComment,
    pull_request: GhPullRequest,
}

#[derive(Deserialize)]
struct GhReviewPayload {
    review: GhReview,
    pull_request: GhPullRequest,
}

#[derive(Deserialize)]
struct GhReview {
    #[serde(default)]
    state: String,
}

fn pull_request_target(pr: GhPullRequest) -> ContributionTarget {
    ContributionTarget {
        kind: TargetKind::MergeRequest,
        iid: pr.number,
        title: pr.title,
    }
}

/// One feed event as contributions: a push yields one per commit it lists,
/// or one for the push itself when the feed leaves the commits out.
fn github_contributions(event: GhEvent) -> Vec<Contribution> {
    let author = event.actor.login;
    let created_at = event.created_at;
    let base = |kind, summary, target, location| Contribution {
        author: author.clone(),
        kind,
        created_at: created_at.clone(),
        summary,
        target,
        location,
    };
    match event.r#type.as_str() {
        "PushEvent" => {
            let Ok(push) = serde_json::from_value::<GhPushPayload>(event.payload) else {
                return Vec::new();
            };
            let branch = branch_name(&push.r#ref);
            if push.commits.is_empty() {
                return vec![base(
                    ContributionKind::Commit,
                    format!("pushed to {branch}"),
                    None,
                    Some(
                        format!("{branch} {}", short_sha(&push.head))
                            .trim()
                            .to_string(),
                    ),
                )];
            }
            push.commits
                .iter()
                .rev()
                .map(|commit| {
                    base(
                        ContributionKind::Commit,
                        first_line(&commit.message),
                        None,
                        Some(format!("{branch} {}", short_sha(&commit.sha))),
                    )
                })
                .collect()
        }
        "IssueCommentEvent" => {
            let Ok(payload) = serde_json::from_value::<GhIssueCommentPayload>(event.payload) else {
                return Vec::new();
            };
            if payload.action != "created" {
                return Vec::new();
            }
            let kind = if payload.issue.pull_request.is_some() {
                TargetKind::MergeRequest
            } else {
                TargetKind::Issue
            };
            vec![base(
                ContributionKind::Comment,
                first_line(payload.comment.body.as_deref().unwrap_or_default()),
                Some(ContributionTarget {
                    kind,
                    iid: payload.issue.number,
                    title: payload.issue.title,
                }),
                None,
            )]
        }
        "PullRequestReviewCommentEvent" => {
            let Ok(payload) = serde_json::from_value::<GhReviewCommentPayload>(event.payload)
            else {
                return Vec::new();
            };
            vec![base(
                ContributionKind::ThreadComment,
                first_line(payload.comment.body.as_deref().unwrap_or_default()),
                Some(pull_request_target(payload.pull_request)),
                payload.comment.path,
            )]
        }
        "PullRequestReviewEvent" => {
            let Ok(payload) = serde_json::from_value::<GhReviewPayload>(event.payload) else {
                return Vec::new();
            };
            let verdict = match payload.review.state.to_lowercase().as_str() {
                "approved" => "approved",
                "changes_requested" => "requested changes",
                _ => "reviewed",
            };
            vec![base(
                ContributionKind::Review,
                verdict.to_string(),
                Some(pull_request_target(payload.pull_request)),
                None,
            )]
        }
        _ => Vec::new(),
    }
}

/// Contributions in a GitHub `/repos/{owner}/{repo}/events` reply, newest
/// first.
pub fn parse_github_events(raw: &str) -> anyhow::Result<Vec<Contribution>> {
    let events: Vec<GhEvent> = serde_json::from_str(raw)?;
    Ok(events.into_iter().flat_map(github_contributions).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitlab_pushes_comments_threads_and_approvals_become_contributions() {
        let raw = r#"[
            {"action_name": "pushed to", "author_username": "ada", "created_at": "2026-10-05T00:00:00Z",
             "push_data": {"commit_count": 3, "ref": "fix/parser", "commit_to": "0123456789abcdef",
                           "commit_title": "Fix parser"}},
            {"action_name": "deleted", "author_username": "ada", "created_at": "2026-10-05T00:00:00Z",
             "push_data": {"commit_count": 0, "ref": "old"}},
            {"action_name": "commented on", "author_username": "bob", "created_at": "2026-10-04T00:00:00Z",
             "target_type": "DiffNote", "target_title": "Fix parser",
             "note": {"body": "\n  Why not a match?\nmore", "noteable_type": "MergeRequest",
                      "noteable_iid": 7, "position": {"new_path": "src/parser.rs"}}},
            {"action_name": "commented on", "author_username": "bob", "created_at": "2026-10-03T00:00:00Z",
             "target_type": "Note", "target_title": "Crash",
             "note": {"body": "Reproduced", "noteable_type": "Issue", "noteable_iid": 3}},
            {"action_name": "approved", "author_username": "cy", "created_at": "2026-10-02T00:00:00Z",
             "target_type": "MergeRequest", "target_iid": 7, "target_title": "Fix parser"},
            {"action_name": "opened", "author_username": "cy", "created_at": "2026-10-01T00:00:00Z",
             "target_type": "Issue", "target_iid": 4}
        ]"#;

        let contributions = parse_gitlab_events(raw).expect("valid events");

        let target = |kind, iid, title: &str| {
            Some(ContributionTarget {
                kind,
                iid,
                title: title.to_string(),
            })
        };
        assert_eq!(
            contributions,
            vec![
                Contribution {
                    author: "ada".to_string(),
                    kind: ContributionKind::Commit,
                    created_at: "2026-10-05T00:00:00Z".to_string(),
                    summary: "Fix parser (+2 more)".to_string(),
                    target: None,
                    location: Some("fix/parser 01234567".to_string()),
                },
                Contribution {
                    author: "bob".to_string(),
                    kind: ContributionKind::ThreadComment,
                    created_at: "2026-10-04T00:00:00Z".to_string(),
                    summary: "Why not a match?".to_string(),
                    target: target(TargetKind::MergeRequest, 7, "Fix parser"),
                    location: Some("src/parser.rs".to_string()),
                },
                Contribution {
                    author: "bob".to_string(),
                    kind: ContributionKind::Comment,
                    created_at: "2026-10-03T00:00:00Z".to_string(),
                    summary: "Reproduced".to_string(),
                    target: target(TargetKind::Issue, 3, "Crash"),
                    location: None,
                },
                Contribution {
                    author: "cy".to_string(),
                    kind: ContributionKind::Review,
                    created_at: "2026-10-02T00:00:00Z".to_string(),
                    summary: "approved".to_string(),
                    target: target(TargetKind::MergeRequest, 7, "Fix parser"),
                    location: None,
                },
            ]
        );
    }

    #[test]
    fn github_events_become_contributions_and_a_push_without_commits_stays_one() {
        let raw = r#"[
            {"type": "PushEvent", "actor": {"login": "ada"}, "created_at": "2026-10-06T00:00:00Z",
             "payload": {"ref": "refs/heads/main", "head": "fedcba9876543210",
                         "commits": [{"sha": "1111111111", "message": "First\n\nbody"},
                                     {"sha": "2222222222", "message": "Second"}]}},
            {"type": "PushEvent", "actor": {"login": "ada"}, "created_at": "2026-10-05T00:00:00Z",
             "payload": {"ref": "refs/heads/dev", "head": "abcdef0123456789"}},
            {"type": "IssueCommentEvent", "actor": {"login": "bob"}, "created_at": "2026-10-04T00:00:00Z",
             "payload": {"action": "created", "comment": {"body": "LGTM"},
                         "issue": {"number": 9, "title": "Add cache", "pull_request": {}}}},
            {"type": "IssueCommentEvent", "actor": {"login": "bob"}, "created_at": "2026-10-04T00:00:00Z",
             "payload": {"action": "deleted", "comment": {"body": "gone"},
                         "issue": {"number": 9, "title": "Add cache"}}},
            {"type": "PullRequestReviewCommentEvent", "actor": {"login": "cy"},
             "created_at": "2026-10-03T00:00:00Z",
             "payload": {"comment": {"body": "nit", "path": "src/cache.rs"},
                         "pull_request": {"number": 9, "title": "Add cache"}}},
            {"type": "PullRequestReviewEvent", "actor": {"login": "cy"}, "created_at": "2026-10-03T00:00:00Z",
             "payload": {"review": {"state": "changes_requested"},
                         "pull_request": {"number": 9, "title": "Add cache"}}},
            {"type": "WatchEvent", "actor": {"login": "dee"}, "created_at": "2026-10-01T00:00:00Z",
             "payload": {}}
        ]"#;

        let shown: Vec<String> = parse_github_events(raw)
            .expect("valid events")
            .into_iter()
            .map(|c| {
                format!(
                    "{} {:?} {} | {} | {}",
                    c.author,
                    c.kind,
                    c.summary,
                    c.location.unwrap_or_default(),
                    c.target.map_or(String::new(), |t| format!("#{}", t.iid)),
                )
            })
            .collect();

        assert_eq!(
            shown,
            vec![
                "ada Commit Second | main 22222222 | ",
                "ada Commit First | main 11111111 | ",
                "ada Commit pushed to dev | dev abcdef01 | ",
                "bob Comment LGTM |  | #9",
                "cy ThreadComment nit | src/cache.rs | #9",
                "cy Review requested changes |  | #9",
            ]
        );
    }
}
