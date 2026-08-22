use super::Backend;
use crate::domain::branches::Branch;
use crate::domain::deployments::{Deployment, Environment};
use crate::domain::issues::Issue;
use crate::domain::labels::Label;
use crate::domain::milestones::Milestone;
use crate::domain::mr::{DiscussionNote, MergeRequest, NotePosition};
use crate::domain::notifications::Notification;
use crate::domain::pipelines::{Job, Pipeline};
use crate::domain::releases::Release;
use crate::domain::runners::Runner;
use crate::event::Event;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinSet;

fn strip_ats(s: &str) -> String {
    if s.is_empty() {
        return s.to_string();
    }
    s.split(',')
        .map(|a| a.trim().trim_start_matches('@').to_string())
        .collect::<Vec<_>>()
        .join(",")
}
fn normalize_labels(s: &str) -> String {
    s.replace(", ", ",")
}

/// `None` for anything that is not a usable login, so an unknown user can
/// never be mistaken for a known one. Never returns `Some("")`.
fn parse_gh_login(raw: &str) -> Option<String> {
    let login = raw.trim();
    if login.is_empty() {
        None
    } else {
        Some(login.to_string())
    }
}

/// Cap the number of concurrent per-run metadata requests when enriching
/// pipelines. The Pipelines tab can list hundreds of runs; without a bound
/// each list refresh would spawn a subprocess per row.
const MAX_GH_METADATA_CONCURRENCY: usize = 8;

fn deserialize_needs<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        Some(serde_json::Value::Array(items)) => Ok(items
            .into_iter()
            .filter_map(|item| item.as_str().map(ToString::to_string))
            .collect()),
        Some(serde_json::Value::String(item)) => Ok(vec![item]),
        _ => Ok(Vec::new()),
    }
}

/// Splits `latestReviews` into (every review's author, approving authors only).
///
/// Two lists because they answer different questions: `you_reviewed` must be
/// true for any review including a rejection, while `approved_by` must not
/// credit someone who only commented.
fn split_review_authors(latest_reviews: &[serde_json::Value]) -> (Vec<String>, Vec<String>) {
    let all_authors: Vec<String> = latest_reviews
        .iter()
        .filter_map(|r| r.get("author")?.get("login")?.as_str().map(String::from))
        .collect();
    let approved_authors: Vec<String> = latest_reviews
        .iter()
        .filter(|r| {
            r.get("state")
                .and_then(|s| s.as_str())
                .map(|s| s.eq_ignore_ascii_case("APPROVED"))
                .unwrap_or(false)
        })
        .filter_map(|r| r.get("author")?.get("login")?.as_str().map(String::from))
        .collect();
    (all_authors, approved_authors)
}

/// Map GitHub's list fields onto the host-neutral state structs.
///
/// `mergeStateStatus == "BLOCKED"` means blocked by branch protection, NOT a
/// merge conflict — conflicts come only from `mergeable == "CONFLICTING"`.
pub fn gh_state_from_fields(
    review_decision: Option<&str>,
    mergeable: Option<&str>,
    merge_state_status: Option<&str>,
    latest_review_authors: Vec<String>,
    current_user: Option<&str>,
    approved_authors: &[String],
) -> (
    Option<crate::domain::mr_state::ApprovalState>,
    Option<crate::domain::mr_state::MergeabilityState>,
) {
    use crate::domain::mr_state::{ApprovalState, MergeabilityState};

    let decision = review_decision.unwrap_or_default();
    let approved = decision.eq_ignore_ascii_case("APPROVED");

    let you_reviewed = current_user
        .map(|me| latest_review_authors.iter().any(|a| a == me))
        .unwrap_or(false);

    // `approved_authors` (unlike `latest_review_authors`, which includes
    // rejections and comments) is the approvals-only list, so this is the
    // one place the workflow cascade's `you_approved` input can be derived
    // correctly. Without it, this stayed hard-coded `false` and the cascade
    // could never reach `ApprovedByYou` on GitHub — see the regression this
    // guards in the tests below.
    let you_approved = current_user
        .map(|me| approved_authors.iter().any(|a| a == me))
        .unwrap_or(false);

    let approval = ApprovalState {
        approved,
        // GitHub exposes no approval counts.
        approvals_left: None,
        approvals_required: None,
        approved_by: if approved {
            latest_review_authors
        } else {
            Vec::new()
        },
        changes_requested: decision.eq_ignore_ascii_case("CHANGES_REQUESTED"),
        you_approved,
        // Needs canApprove, which gh pr list does not provide.
        awaiting_you: false,
        current_user: current_user.map(|s| s.to_string()),
        you_reviewed,
    };

    let merge_raw = mergeable.unwrap_or("UNKNOWN");
    let state_raw = merge_state_status.unwrap_or("UNKNOWN");
    let mergeability = MergeabilityState {
        conflicts: merge_raw.eq_ignore_ascii_case("CONFLICTING"),
        needs_rebase: state_raw.eq_ignore_ascii_case("BEHIND"),
        computing: merge_raw.eq_ignore_ascii_case("UNKNOWN"),
    };

    (Some(approval), Some(mergeability))
}

/// The authenticated GitHub login, resolved at most once per process.
///
/// Process-global rather than a field on `GhBackend`: `GitlabClient::clone`
/// rebuilds the backend from scratch (`create_backend`, see
/// `domain::client`'s `Clone` impl), and every refresh clones the client
/// (`spawn_refresh_active_tab`), so a per-instance cell would start empty on
/// every single refresh and never serve a hit. Matches the `ICONS`/`THEME`
/// precedent of process-global state for values that do not change within a
/// run. Holds `Option` rather than `String` so a *failed* lookup is cached
/// too — `get_or_try_init` leaves the cell uninitialised on error and would
/// retry on every refresh, which is the per-refresh request the design
/// forbids.
static GH_CURRENT_USER: tokio::sync::OnceCell<Option<String>> = tokio::sync::OnceCell::const_new();

pub struct GhBackend {
    tx: Option<UnboundedSender<Event>>,
}

impl GhBackend {
    pub fn new() -> Self {
        Self { tx: None }
    }

    /// `None` if the lookup fails — an unknown user must yield an unknown
    /// workflow status, never a wrong one. The failure itself is cached
    /// alongside a success, so this never re-issues the `gh api user` call
    /// after the first attempt, whatever the outcome — and because the cache
    /// is the process-global `GH_CURRENT_USER`, that holds across cloned
    /// clients too, not just repeated calls on one instance.
    async fn current_user(&self) -> Option<&str> {
        GH_CURRENT_USER
            .get_or_init(|| async {
                let raw = self
                    .run_gh(&["api", "user", "--jq", ".login"], "FETCHING GH USER")
                    .await
                    .ok()?;
                parse_gh_login(&raw)
            })
            .await
            .as_deref()
    }

    async fn run_gh(&self, args: &[&str], desc: &str) -> Result<String> {
        let label = desc.to_uppercase();
        let cmd_str = format!("gh {}", args.join(" "));

        let output = Command::new("gh")
            .args(args)
            .output()
            .await
            .with_context(|| format!("Failed to execute: gh {}", args.join(" ")))?;

        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        if output.status.success() {
            let s = String::from_utf8(output.stdout)?;
            if let Some(ref tx) = self.tx {
                let _ = tx.send(Event::TerminalCommandLogged {
                    timestamp,
                    command: format!("{}: {}", label, cmd_str),
                    status: "Success".to_string(),
                });
            }
            Ok(s)
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if let Some(ref tx) = self.tx {
                let _ = tx.send(Event::TerminalCommandLogged {
                    timestamp,
                    command: format!("{}: {}", label, cmd_str),
                    status: format!("Failed: {}", err_msg),
                });
            }
            anyhow::bail!("gh command failed: {}", err_msg)
        }
    }
}

#[async_trait]
impl Backend for GhBackend {
    fn kind(&self) -> super::BackendKind {
        super::BackendKind::GitHub
    }

    fn program(&self) -> &'static str {
        "gh"
    }

    fn set_tx(&mut self, tx: UnboundedSender<Event>) {
        self.tx = Some(tx);
    }

    // ── Issues ──

    /// `_per_request` is unused in the GitHub backend (as with all `Backend`
    /// methods that take it) — `gh` uses `--limit` for the total item count,
    /// not per-page pagination. Only GitLab backends paginate per-request.
    async fn list_issues(
        &self,
        project: &str,
        show_closed: bool,
        page_size: usize,
        _per_request: usize,
    ) -> Result<Vec<Issue>> {
        let state = if show_closed { "all" } else { "open" };
        let total = page_size * 10;
        let raw = self
            .run_gh(
                &[
                    "issue",
                    "list",
                    "--json",
                    "number,title,state,labels,author,body,createdAt,updatedAt,closedAt,milestone,assignees",
                    "-R",
                    project,
                    "--state",
                    state,
                    "--limit",
                    &total.to_string(),
                ],
                "Fetching Issues",
            )
            .await?;

        #[derive(Deserialize)]
        struct GhIssue {
            number: u64,
            title: String,
            state: String,
            #[serde(default)]
            labels: Vec<serde_json::Value>,
            author: Option<GhLogin>,
            body: Option<String>,
            #[serde(rename = "createdAt")]
            #[allow(dead_code)]
            created_at: String,
            #[serde(rename = "updatedAt")]
            updated_at: String,
            #[serde(rename = "closedAt")]
            closed_at: Option<String>,
            milestone: Option<GhMs>,
            #[serde(default)]
            assignees: Vec<GhLogin>,
        }
        #[derive(Deserialize)]
        struct GhLogin {
            login: String,
        }
        #[derive(Deserialize)]
        struct GhMs {
            title: String,
        }

        let gh_issues: Vec<GhIssue> = serde_json::from_str(&raw)?;
        Ok(gh_issues
            .into_iter()
            .map(|gi| {
                let state = if gi.state == "OPEN" {
                    "opened"
                } else {
                    "closed"
                }
                .to_string();
                let labels: Vec<String> = gi
                    .labels
                    .iter()
                    .filter_map(|v| v.get("name")?.as_str().map(String::from))
                    .collect();
                let author = crate::domain::issues::Author {
                    username: gi.author.map(|a| a.login).unwrap_or_default(),
                };
                let milestone = gi
                    .milestone
                    .map(|m| crate::domain::issues::Milestone { title: m.title });
                let assignees: Vec<crate::domain::issues::Assignee> = gi
                    .assignees
                    .into_iter()
                    .map(|a| crate::domain::issues::Assignee { username: a.login })
                    .collect();
                Issue {
                    iid: gi.number,
                    title: gi.title,
                    state,
                    labels,
                    updated_at: gi.updated_at,
                    created_at: Some(gi.created_at),
                    closed_at: gi.closed_at,
                    author,
                    milestone,
                    assignees,
                    description: gi.body,
                    due_date: None,
                }
            })
            .collect())
    }

    async fn get_issue(&self, project: &str, iid: u64) -> Result<Issue> {
        let raw = self
            .run_gh(
                &[
                    "issue",
                    "view",
                    &iid.to_string(),
                    "--json",
                    "number,title,state,labels,author,body,createdAt,updatedAt,closedAt,milestone,assignees",
                    "-R",
                    project,
                ],
                "Fetching Issue",
            )
            .await?;
        #[derive(Deserialize)]
        struct GhIssue {
            number: u64,
            title: String,
            state: String,
            #[serde(default)]
            labels: Vec<serde_json::Value>,
            author: Option<GhLogin>,
            body: Option<String>,
            #[serde(rename = "createdAt")]
            #[allow(dead_code)]
            created_at: String,
            #[serde(rename = "updatedAt")]
            updated_at: String,
            #[serde(rename = "closedAt")]
            closed_at: Option<String>,
            milestone: Option<GhMs>,
            #[serde(default)]
            assignees: Vec<GhLogin>,
        }
        #[derive(Deserialize)]
        struct GhLogin {
            login: String,
        }
        #[derive(Deserialize)]
        struct GhMs {
            title: String,
        }
        let gi: GhIssue = serde_json::from_str(&raw)?;
        let state = if gi.state == "OPEN" {
            "opened"
        } else {
            "closed"
        }
        .to_string();
        let labels: Vec<String> = gi
            .labels
            .iter()
            .filter_map(|v| v.get("name")?.as_str().map(String::from))
            .collect();
        let author = crate::domain::issues::Author {
            username: gi.author.map(|a| a.login).unwrap_or_default(),
        };
        let milestone = gi
            .milestone
            .map(|m| crate::domain::issues::Milestone { title: m.title });
        let assignees: Vec<crate::domain::issues::Assignee> = gi
            .assignees
            .into_iter()
            .map(|a| crate::domain::issues::Assignee { username: a.login })
            .collect();
        Ok(Issue {
            iid: gi.number,
            title: gi.title,
            state,
            labels,
            updated_at: gi.updated_at,
            created_at: Some(gi.created_at),
            closed_at: gi.closed_at,
            author,
            milestone,
            assignees,
            description: gi.body,
            due_date: None,
        })
    }

    async fn close_issue(&self, project: &str, iid: u64) -> Result<()> {
        self.run_gh(
            &["issue", "close", &iid.to_string(), "-R", project],
            "CLOSING ISSUE",
        )
        .await?;
        Ok(())
    }

    async fn reopen_issue(&self, project: &str, iid: u64) -> Result<()> {
        self.run_gh(
            &["issue", "reopen", &iid.to_string(), "-R", project],
            "REOPENING ISSUE",
        )
        .await?;
        Ok(())
    }

    async fn delete_issue(&self, project: &str, iid: u64) -> Result<()> {
        self.run_gh(
            &["issue", "delete", &iid.to_string(), "-R", project, "--yes"],
            "DELETING ISSUE",
        )
        .await?;
        Ok(())
    }

    async fn create_issue(
        &self,
        project: &str,
        title: &str,
        description: &str,
        labels: &str,
        assignees: &str,
        milestone: &str,
        due_date: &str,
        weight: &str,
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "issue".into(),
            "create".into(),
            "-R".into(),
            project.into(),
            "--title".into(),
            title.into(),
        ];
        if !description.is_empty() {
            args.push("--body".into());
            args.push(description.into());
        }
        if !labels.is_empty() {
            args.push("--label".into());
            args.push(normalize_labels(labels).into());
        }
        if !assignees.is_empty() {
            args.push("--assignee".into());
            args.push(strip_ats(assignees).into());
        }
        if !milestone.is_empty() {
            args.push("--milestone".into());
            args.push(milestone.into());
        }
        if !due_date.is_empty() {
            args.push("--due-date".into());
            args.push(due_date.into());
        }
        if !weight.is_empty() {
            args.push("--weight".into());
            args.push(weight.into());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_gh(&args_refs, "CREATING ISSUE").await?;
        Ok(())
    }

    // ── Issue Field Updates ──

    async fn update_issue_title(&self, project: &str, iid: u64, title: &str) -> Result<()> {
        self.run_gh(
            &[
                "issue",
                "edit",
                &iid.to_string(),
                "--title",
                title,
                "-R",
                project,
            ],
            "UPDATING ISSUE",
        )
        .await?;
        Ok(())
    }

    async fn update_issue_description(
        &self,
        project: &str,
        iid: u64,
        description: &str,
    ) -> Result<()> {
        self.run_gh(
            &[
                "issue",
                "edit",
                &iid.to_string(),
                "--body",
                description,
                "-R",
                project,
            ],
            "UPDATING ISSUE",
        )
        .await?;
        Ok(())
    }

    async fn update_issue_labels(
        &self,
        project: &str,
        iid: u64,
        add_labels: &[String],
        remove_labels: &[String],
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "issue".into(),
            "edit".into(),
            iid.to_string(),
            "-R".into(),
            project.into(),
        ];
        for label in add_labels {
            args.push("--add-label".into());
            args.push(label.clone());
        }
        for label in remove_labels {
            args.push("--remove-label".into());
            args.push(label.clone());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_gh(&args_refs, "UPDATING ISSUE").await?;
        Ok(())
    }

    async fn update_issue_assignees(
        &self,
        project: &str,
        iid: u64,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "issue".into(),
            "edit".into(),
            iid.to_string(),
            "-R".into(),
            project.into(),
        ];
        for a in add {
            args.push("--add-assignee".into());
            args.push(a.clone());
        }
        for a in remove {
            args.push("--remove-assignee".into());
            args.push(a.clone());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_gh(&args_refs, "UPDATING ISSUE").await?;
        Ok(())
    }

    async fn update_issue_milestone(&self, project: &str, iid: u64, milestone: &str) -> Result<()> {
        let args: Vec<String> = if milestone == "--" || milestone.is_empty() {
            vec![
                "issue".to_string(),
                "edit".to_string(),
                iid.to_string(),
                "--remove-milestone".to_string(),
                "-R".to_string(),
                project.to_string(),
            ]
        } else {
            vec![
                "issue".to_string(),
                "edit".to_string(),
                iid.to_string(),
                "--milestone".to_string(),
                milestone.to_string(),
                "-R".to_string(),
                project.to_string(),
            ]
        };
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_gh(&args_refs, "UPDATING ISSUE").await?;
        Ok(())
    }

    async fn update_issue_due_date(&self, project: &str, iid: u64, due_date: &str) -> Result<()> {
        self.run_gh(
            &[
                "issue",
                "edit",
                &iid.to_string(),
                "--due-date",
                due_date,
                "-R",
                project,
            ],
            "UPDATING ISSUE",
        )
        .await?;
        Ok(())
    }

    async fn update_issue_weight(&self, _project: &str, _iid: u64, _weight: &str) -> Result<()> {
        Ok(())
    }

    async fn update_issue_confidential(
        &self,
        project: &str,
        iid: u64,
        confidential: bool,
    ) -> Result<()> {
        Ok(())
    }

    // ── Merge Requests ──

    /// `_per_request` is unused in the GitHub backend — `gh` uses `--limit`
    /// for the total item count, not per-page pagination.
    async fn list_mrs(
        &self,
        project: &str,
        show_closed: bool,
        page_size: usize,
        _per_request: usize,
    ) -> Result<Vec<MergeRequest>> {
        let state = if show_closed { "all" } else { "open" };
        let total = page_size * 10;
        let raw = self
            .run_gh(
                &[
                    "pr",
                    "list",
                    "--json",
                    "number,title,state,labels,author,body,createdAt,updatedAt,headRefName,baseRefName,isDraft,assignees,milestone,reviewDecision,latestReviews,mergeable,mergeStateStatus,reviewRequests",
                    "-R",
                    project,
                    "--state",
                    state,
                    "--limit",
                    &total.to_string(),
                ],
                "Fetching PRs",
            )
            .await?;

        #[derive(Deserialize)]
        struct GhPr {
            number: u64,
            title: String,
            state: String,
            #[serde(default)]
            labels: Vec<serde_json::Value>,
            author: Option<GhLogin>,
            body: Option<String>,
            #[serde(rename = "createdAt")]
            #[allow(dead_code)]
            created_at: String,
            #[serde(rename = "updatedAt")]
            updated_at: String,
            #[serde(rename = "headRefName")]
            head_ref_name: Option<String>,
            #[serde(rename = "baseRefName")]
            base_ref_name: Option<String>,
            #[serde(rename = "isDraft")]
            is_draft: Option<bool>,
            #[serde(default)]
            assignees: Vec<GhLogin>,
            milestone: Option<GhMs>,
            #[serde(rename = "reviewDecision", default)]
            review_decision: Option<String>,
            #[serde(rename = "latestReviews", default)]
            latest_reviews: Vec<serde_json::Value>,
            #[serde(default)]
            mergeable: Option<String>,
            #[serde(rename = "mergeStateStatus", default)]
            merge_state_status: Option<String>,
            #[serde(rename = "reviewRequests", default)]
            review_requests: Vec<serde_json::Value>,
        }
        #[derive(Deserialize)]
        struct GhLogin {
            login: String,
        }
        #[derive(Deserialize)]
        struct GhMs {
            title: String,
        }

        let me = self.current_user().await;
        let gh_prs: Vec<GhPr> = serde_json::from_str(&raw)?;
        Ok(gh_prs
            .into_iter()
            .map(|gp| {
                let state = if gp.state == "OPEN" {
                    "opened"
                } else {
                    "closed"
                }
                .to_string();
                let labels: Vec<String> = gp
                    .labels
                    .iter()
                    .filter_map(|v| v.get("name")?.as_str().map(String::from))
                    .collect();
                let author = crate::domain::mr::Author {
                    username: gp.author.map(|a| a.login).unwrap_or_default(),
                };
                let milestone = gp
                    .milestone
                    .map(|m| crate::domain::mr::Milestone { title: m.title });
                let assignees: Vec<crate::domain::mr::Assignee> = gp
                    .assignees
                    .into_iter()
                    .map(|a| crate::domain::mr::Assignee { username: a.login })
                    .collect();
                let (latest_review_authors, approved_authors) =
                    split_review_authors(&gp.latest_reviews);
                let (approval, mergeability) = gh_state_from_fields(
                    gp.review_decision.as_deref(),
                    gp.mergeable.as_deref(),
                    gp.merge_state_status.as_deref(),
                    latest_review_authors,
                    me,
                    &approved_authors,
                );
                // gh_state_from_fields derives approved_by from the same
                // (unfiltered) list it uses for you_reviewed; restore the
                // approvals-only view here. (you_approved was already
                // derived correctly inside gh_state_from_fields, from this
                // same approved_authors list passed in below.)
                let approval = approval.map(|a| crate::domain::mr_state::ApprovalState {
                    approved_by: approved_authors,
                    ..a
                });
                let reviewers: Vec<crate::domain::mr::Reviewer> = gp
                    .review_requests
                    .iter()
                    .filter_map(|r| {
                        r.get("login").or_else(|| r.get("name"))?.as_str().map(|s| {
                            crate::domain::mr::Reviewer {
                                username: s.to_string(),
                            }
                        })
                    })
                    .collect();
                MergeRequest {
                    iid: gp.number,
                    title: gp.title,
                    state,
                    labels,
                    updated_at: gp.updated_at,
                    author,
                    milestone,
                    assignees,
                    reviewers,
                    target_branch: gp.base_ref_name.unwrap_or_default(),
                    source_branch: gp.head_ref_name.unwrap_or_default(),
                    draft: gp.is_draft.unwrap_or(false),
                    description: gp.body,
                    head_pipeline: None,
                    blocking_discussions_resolved: None,
                    approval,
                    mergeability,
                    workflow: None,
                }
            })
            .collect())
    }

    async fn get_mr(&self, project: &str, iid: u64) -> Result<MergeRequest> {
        let raw = self
            .run_gh(
                &[
                    "pr",
                    "view",
                    &iid.to_string(),
                    "--json",
                    "number,title,state,labels,author,body,createdAt,updatedAt,headRefName,baseRefName,isDraft,assignees,milestone",
                    "-R",
                    project,
                ],
                "Fetching PR",
            )
            .await?;
        #[derive(Deserialize)]
        struct GhPr {
            number: u64,
            title: String,
            state: String,
            #[serde(default)]
            labels: Vec<serde_json::Value>,
            author: Option<GhLogin>,
            body: Option<String>,
            #[serde(rename = "createdAt")]
            #[allow(dead_code)]
            created_at: String,
            #[serde(rename = "updatedAt")]
            updated_at: String,
            #[serde(rename = "headRefName")]
            head_ref_name: Option<String>,
            #[serde(rename = "baseRefName")]
            base_ref_name: Option<String>,
            #[serde(rename = "isDraft")]
            is_draft: Option<bool>,
            #[serde(default)]
            assignees: Vec<GhLogin>,
            milestone: Option<GhMs>,
        }
        #[derive(Deserialize)]
        struct GhLogin {
            login: String,
        }
        #[derive(Deserialize)]
        struct GhMs {
            title: String,
        }
        let gp: GhPr = serde_json::from_str(&raw)?;
        let state = if gp.state == "OPEN" {
            "opened"
        } else {
            "closed"
        }
        .to_string();
        let labels: Vec<String> = gp
            .labels
            .iter()
            .filter_map(|v| v.get("name")?.as_str().map(String::from))
            .collect();
        let author = crate::domain::mr::Author {
            username: gp.author.map(|a| a.login).unwrap_or_default(),
        };
        let milestone = gp
            .milestone
            .map(|m| crate::domain::mr::Milestone { title: m.title });
        let assignees: Vec<crate::domain::mr::Assignee> = gp
            .assignees
            .into_iter()
            .map(|a| crate::domain::mr::Assignee { username: a.login })
            .collect();
        Ok(MergeRequest {
            iid: gp.number,
            title: gp.title,
            state,
            labels,
            updated_at: gp.updated_at,
            author,
            milestone,
            assignees,
            // Deliberately left empty: get_mr has no UI caller, so fetching
            // reviewRequests here would cost a request for no benefit.
            reviewers: vec![],
            target_branch: gp.base_ref_name.unwrap_or_default(),
            source_branch: gp.head_ref_name.unwrap_or_default(),
            draft: gp.is_draft.unwrap_or(false),
            description: gp.body,
            head_pipeline: None,
            blocking_discussions_resolved: None,
            approval: None,
            mergeability: None,
            workflow: None,
        })
    }

    async fn get_mr_diff(&self, project: &str, iid: u64) -> Result<String> {
        self.run_gh(
            &["pr", "diff", &iid.to_string(), "-R", project],
            "Fetching PR Diff",
        )
        .await
    }

    async fn list_mr_notes(
        &self,
        project: &str,
        mr_iid: u64,
        page_size: usize,
    ) -> Result<Vec<DiscussionNote>> {
        let endpoint = format!(
            "/repos/{}/pulls/{}/comments?per_page={}",
            project, mr_iid, page_size
        );
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching MR Notes")
            .await?;

        #[derive(Deserialize)]
        struct GhComment {
            id: u64,
            body: String,
            user: Option<GhLogin>,
            created_at: String,
            path: Option<String>,
            line: Option<u64>,
            #[serde(default = "default_side")]
            side: String,
            start_line: Option<u64>,
            in_reply_to_id: Option<u64>,
        }
        fn default_side() -> String {
            "RIGHT".to_string()
        }
        #[derive(Deserialize)]
        struct GhLogin {
            login: String,
        }

        let gh_comments: Vec<GhComment> = serde_json::from_str(&raw)?;
        Ok(gh_comments
            .into_iter()
            .map(|gc| {
                let username = gc.user.map(|u| u.login).unwrap_or_default();
                let position = if let Some(p) = gc.path {
                    let (new_line, old_line) = if gc.side == "LEFT" {
                        (None, gc.line)
                    } else {
                        (gc.line, None)
                    };
                    Some(NotePosition {
                        new_path: Some(p.clone()),
                        old_path: Some(p),
                        new_line,
                        old_line,
                        start_line: gc.start_line,
                        line_range: None,
                    })
                } else {
                    None
                };
                let disc_id = gc
                    .in_reply_to_id
                    .map(|rid| rid.to_string())
                    .unwrap_or_else(|| gc.id.to_string());
                DiscussionNote {
                    id: gc.id,
                    body: gc.body,
                    author: crate::domain::mr::Author { username },
                    created_at: gc.created_at,
                    system: false,
                    position,
                    discussion_id: Some(disc_id),
                    resolved: Some(false),
                    resolvable: Some(true),
                }
            })
            .collect())
    }

    async fn close_mr(&self, project: &str, iid: u64) -> Result<()> {
        self.run_gh(
            &["pr", "close", &iid.to_string(), "-R", project],
            "CLOSING PR",
        )
        .await?;
        Ok(())
    }

    async fn reopen_mr(&self, project: &str, iid: u64) -> Result<()> {
        self.run_gh(
            &["pr", "reopen", &iid.to_string(), "-R", project],
            "REOPENING PR",
        )
        .await?;
        Ok(())
    }

    async fn delete_mr(&self, _project: &str, _iid: u64) -> Result<()> {
        anyhow::bail!("GitHub does not support deleting pull requests")
    }

    async fn approve_mr(&self, project: &str, iid: u64) -> Result<()> {
        self.run_gh(
            &["pr", "review", &iid.to_string(), "--approve", "-R", project],
            "APPROVING PR",
        )
        .await?;
        Ok(())
    }

    async fn revoke_mr(&self, _project: &str, _iid: u64) -> Result<()> {
        // `gh pr review` has no revoke flag. The only path is the review
        // dismissal API, which needs a review-ID lookup and write permission,
        // so it is deliberately out of scope.
        anyhow::bail!("Revoking approval isn't supported on GitHub")
    }

    async fn rebase_mr(&self, project: &str, iid: u64) -> Result<()> {
        self.run_gh(
            &[
                "pr",
                "update-branch",
                &iid.to_string(),
                "-R",
                project,
                "--rebase",
            ],
            "REBASING PR",
        )
        .await?;
        Ok(())
    }

    async fn list_mr_state(
        &self,
        _project: &str,
        _iids: &[u64],
    ) -> Result<
        HashMap<
            u64,
            (
                Option<crate::domain::mr_state::ApprovalState>,
                Option<crate::domain::mr_state::MergeabilityState>,
            ),
        >,
    > {
        // GitHub state rides on the existing `gh pr list --json` call instead
        // (see list_mrs), so there is nothing extra to fetch here.
        Ok(HashMap::new())
    }

    async fn merge_mr(
        &self,
        project: &str,
        iid: u64,
        squash: bool,
        delete_branch: bool,
        strategy: Option<&str>,
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "pr".into(),
            "merge".into(),
            iid.to_string(),
            "-R".into(),
            project.into(),
        ];
        if squash {
            args.push("--squash".into());
        } else if let Some(s) = strategy {
            match s {
                "rebase" => args.push("--rebase".into()),
                _ => args.push("--merge".into()),
            }
        }
        if delete_branch {
            args.push("--delete-branch".into());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_gh(&args_refs, "MERGING PR").await?;
        Ok(())
    }

    async fn toggle_mr_draft(&self, project: &str, iid: u64, is_draft: bool) -> Result<()> {
        if is_draft {
            self.run_gh(
                &["pr", "ready", &iid.to_string(), "--undo", "-R", project],
                "MARKING PR DRAFT",
            )
            .await?;
        } else {
            self.run_gh(
                &["pr", "ready", &iid.to_string(), "-R", project],
                "MARKING PR READY",
            )
            .await?;
        }
        Ok(())
    }

    async fn create_mr(
        &self,
        project: &str,
        title: &str,
        description: &str,
        source_branch: &str,
        target_branch: &str,
        labels: &str,
        assignees: &str,
        reviewers: &str,
        milestone: &str,
        issue_iid: Option<u64>,
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "pr".into(),
            "create".into(),
            "-R".into(),
            project.into(),
            "--title".into(),
            title.into(),
        ];
        if !source_branch.is_empty() {
            args.push("--head".into());
            args.push(source_branch.into());
        }
        if !target_branch.is_empty() {
            args.push("--base".into());
            args.push(target_branch.into());
        }
        let body = if let Some(iid) = issue_iid {
            if description.is_empty() {
                format!("Closes #{}", iid)
            } else if !description.contains("Closes #") {
                format!("{}\n\nCloses #{}", description, iid)
            } else {
                description.to_string()
            }
        } else {
            description.to_string()
        };
        if !body.is_empty() {
            args.push("--body".into());
            args.push(body.into());
        }
        if !labels.is_empty() {
            args.push("--label".into());
            args.push(normalize_labels(labels).into());
        }
        if !assignees.is_empty() {
            args.push("--assignee".into());
            args.push(strip_ats(assignees).into());
        }
        if !reviewers.is_empty() {
            args.push("--reviewer".into());
            args.push(strip_ats(reviewers).into());
        }
        if !milestone.is_empty() {
            args.push("--milestone".into());
            args.push(milestone.into());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_gh(&args_refs, "CREATING PR").await?;
        Ok(())
    }

    async fn add_mr_comment(
        &self,
        project: &str,
        iid: u64,
        body: &str,
        _file_path: Option<&str>,
        _line: Option<u64>,
        _old_line: Option<u64>,
    ) -> Result<()> {
        self.run_gh(
            &[
                "pr",
                "comment",
                &iid.to_string(),
                "-R",
                project,
                "--body",
                body,
            ],
            "ADDING PR COMMENT",
        )
        .await?;
        Ok(())
    }

    // ── PR Field Updates ──

    async fn update_mr_title(&self, project: &str, iid: u64, title: &str) -> Result<()> {
        self.run_gh(
            &[
                "pr",
                "edit",
                &iid.to_string(),
                "--title",
                title,
                "-R",
                project,
            ],
            "UPDATING PR",
        )
        .await?;
        Ok(())
    }

    async fn update_mr_description(
        &self,
        project: &str,
        iid: u64,
        description: &str,
    ) -> Result<()> {
        self.run_gh(
            &[
                "pr",
                "edit",
                &iid.to_string(),
                "--body",
                description,
                "-R",
                project,
            ],
            "UPDATING PR",
        )
        .await?;
        Ok(())
    }

    async fn update_mr_labels(
        &self,
        project: &str,
        iid: u64,
        add_labels: &[String],
        remove_labels: &[String],
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "pr".into(),
            "edit".into(),
            iid.to_string(),
            "-R".into(),
            project.into(),
        ];
        for label in add_labels {
            args.push("--add-label".into());
            args.push(label.clone());
        }
        for label in remove_labels {
            args.push("--remove-label".into());
            args.push(label.clone());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_gh(&args_refs, "UPDATING PR").await?;
        Ok(())
    }

    async fn update_mr_assignees(
        &self,
        project: &str,
        iid: u64,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "pr".into(),
            "edit".into(),
            iid.to_string(),
            "-R".into(),
            project.into(),
        ];
        for a in add {
            args.push("--add-assignee".into());
            args.push(a.clone());
        }
        for a in remove {
            args.push("--remove-assignee".into());
            args.push(a.clone());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_gh(&args_refs, "UPDATING PR").await?;
        Ok(())
    }

    async fn update_mr_reviewers(
        &self,
        project: &str,
        iid: u64,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "pr".into(),
            "edit".into(),
            iid.to_string(),
            "-R".into(),
            project.into(),
        ];
        for r in add {
            args.push("--add-reviewer".into());
            args.push(r.clone());
        }
        for r in remove {
            args.push("--remove-reviewer".into());
            args.push(r.clone());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_gh(&args_refs, "UPDATING PR").await?;
        Ok(())
    }

    async fn update_mr_milestone(&self, project: &str, iid: u64, milestone: &str) -> Result<()> {
        let args: Vec<String> = if milestone == "--" || milestone.is_empty() {
            vec![
                "pr".to_string(),
                "edit".to_string(),
                iid.to_string(),
                "--remove-milestone".to_string(),
                "-R".to_string(),
                project.to_string(),
            ]
        } else {
            vec![
                "pr".to_string(),
                "edit".to_string(),
                iid.to_string(),
                "--milestone".to_string(),
                milestone.to_string(),
                "-R".to_string(),
                project.to_string(),
            ]
        };
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_gh(&args_refs, "UPDATING PR").await?;
        Ok(())
    }

    async fn update_mr_target_branch(&self, project: &str, iid: u64, branch: &str) -> Result<()> {
        self.run_gh(
            &[
                "pr",
                "edit",
                &iid.to_string(),
                "--base",
                branch,
                "-R",
                project,
            ],
            "UPDATING PR",
        )
        .await?;
        Ok(())
    }

    // ── Pipelines ──

    /// `_per_request` is unused in the GitHub backend — `gh run list` uses
    /// `--limit` for the total item count, not per-page pagination.
    async fn list_pipelines(
        &self,
        project: &str,
        page_size: usize,
        _per_request: usize,
    ) -> Result<Vec<Pipeline>> {
        let total = page_size * 10;
        let raw = self
            .run_gh(
                &[
                    "run",
                    "list",
                    "--json",
                    "databaseId,status,conclusion,headBranch,createdAt,startedAt,updatedAt,workflowName,displayTitle,headSha,event",
                    "-R",
                    project,
                    "--limit",
                    &total.to_string(),
                ],
                "Fetching Actions",
            )
            .await?;

        #[derive(Deserialize)]
        struct GhRun {
            #[serde(rename = "databaseId")]
            database_id: u64,
            status: String,
            conclusion: Option<String>,
            #[serde(rename = "headBranch")]
            head_branch: String,
            #[serde(rename = "updatedAt")]
            updated_at: String,
            #[serde(rename = "createdAt")]
            created_at: Option<String>,
            #[serde(rename = "startedAt")]
            started_at: Option<String>,
            #[serde(rename = "workflowName")]
            workflow_name: Option<String>,
            #[serde(rename = "displayTitle")]
            display_title: Option<String>,
            #[serde(rename = "headSha")]
            head_sha: Option<String>,
            event: Option<String>,
        }

        let runs: Vec<GhRun> = serde_json::from_str(&raw)?;
        let mut pipelines: Vec<Pipeline> = runs
            .into_iter()
            .map(|r| {
                let status = match r.status.as_str() {
                    "completed" | "COMPLETED" => match r.conclusion.as_deref() {
                        Some("success") | Some("SUCCESS") => "success",
                        Some("failure") | Some("FAILURE") => "failed",
                        Some("cancelled") | Some("CANCELLED") | Some("canceled")
                        | Some("CANCELED") => "canceled",
                        Some("skipped") | Some("SKIPPED") => "skipped",
                        _ => "failed",
                    },
                    "in_progress" | "IN_PROGRESS" => "running",
                    "queued" | "QUEUED" | "waiting" | "WAITING" => "pending",
                    _ => "pending",
                }
                .to_string();
                let duration = r
                    .started_at
                    .as_deref()
                    .and_then(|started| chrono_duration(started, &r.updated_at));
                Pipeline {
                    id: r.database_id,
                    status,
                    r#ref: r.head_branch,
                    updated_at: r.updated_at,
                    name: r.workflow_name.unwrap_or_default(),
                    display_title: r.display_title.unwrap_or_default(),
                    event: r.event.as_deref().unwrap_or_default().to_string(),
                    head_sha: r.head_sha.unwrap_or_default(),
                    actor_login: String::new(),
                    duration_seconds: duration,
                    created_at: r.created_at,
                    source: r.event,
                }
            })
            .collect();

        enrich_pipeline_actors(project, &mut pipelines).await;
        Ok(pipelines)
    }

    async fn list_pipeline_jobs(
        &self,
        project: &str,
        pipeline_id: u64,
        _page_size: usize,
    ) -> Result<Vec<Job>> {
        let raw = self
            .run_gh(
                &[
                    "run",
                    "view",
                    &pipeline_id.to_string(),
                    "--json",
                    "jobs",
                    "--jq",
                    ".jobs",
                    "-R",
                    project,
                ],
                "Fetching Jobs",
            )
            .await?;

        #[derive(Deserialize)]
        struct GhJob {
            #[serde(rename = "databaseId")]
            id: u64,
            name: String,
            status: String,
            conclusion: Option<String>,
            #[serde(rename = "runnerName")]
            runner_name: Option<String>,
            #[serde(rename = "startedAt")]
            started_at: Option<String>,
            #[serde(rename = "completedAt")]
            completed_at: Option<String>,
            #[serde(default, deserialize_with = "deserialize_needs")]
            needs: Vec<String>,
        }

        let jobs: Vec<GhJob> = serde_json::from_str(&raw)?;
        let all_jobs: Vec<Job> = jobs
            .into_iter()
            .map(|j| {
                let status = match j.status.as_str() {
                    "completed" | "COMPLETED" => match j.conclusion.as_deref() {
                        Some("success") | Some("SUCCESS") => "success",
                        Some("failure") | Some("FAILURE") => "failed",
                        Some("cancelled") | Some("CANCELLED") | Some("canceled")
                        | Some("CANCELED") => "canceled",
                        Some("skipped") | Some("SKIPPED") => "skipped",
                        _ => "failed",
                    },
                    "in_progress" | "IN_PROGRESS" => "running",
                    "queued" | "QUEUED" | "waiting" | "WAITING" => "pending",
                    _ => "pending",
                }
                .to_string();
                let duration = match (&j.started_at, &j.completed_at) {
                    (Some(start), Some(end)) => chrono_duration(start, end),
                    _ => None,
                };
                Job {
                    id: j.id,
                    status,
                    // GitHub jobs form a dependency graph, not GitLab-style
                    // sequential stages. Keep this empty so stage grouping
                    // cannot imply semantics that do not exist.
                    stage: String::new(),
                    name: j.name,
                    matrix: None,
                    duration_seconds: duration,
                    runner: j.runner_name,
                    needs: j.needs,
                }
            })
            .collect();
        Ok(crate::domain::pipelines::process_pipeline_jobs(all_jobs))
    }

    async fn get_job_trace(&self, project: &str, job_id: u64) -> Result<String> {
        self.run_gh(
            &[
                "run",
                "view",
                "--job",
                &job_id.to_string(),
                "--log",
                "-R",
                project,
            ],
            "Fetching Job Log",
        )
        .await
    }

    async fn retry_pipeline(&self, project: &str, pipeline_id: u64) -> Result<()> {
        self.run_gh(
            &["run", "rerun", &pipeline_id.to_string(), "-R", project],
            "Retrying Action",
        )
        .await?;
        Ok(())
    }

    async fn cancel_pipeline(&self, project: &str, pipeline_id: u64) -> Result<()> {
        self.run_gh(
            &["run", "cancel", &pipeline_id.to_string(), "-R", project],
            "Cancelling Action",
        )
        .await?;
        Ok(())
    }

    async fn retry_job(&self, project: &str, job_id: u64) -> Result<()> {
        self.run_gh(
            &["run", "rerun", "--job", &job_id.to_string(), "-R", project],
            "Retrying Job",
        )
        .await?;
        Ok(())
    }

    async fn cancel_job(&self, project: &str, job_id: u64) -> Result<()> {
        // GitHub cancels at the run level, but for individual jobs we use raw API
        let endpoint = format!("/repos/{}/actions/jobs/{}/cancel", project, job_id);
        self.raw_api(&endpoint, "POST", Some(""), "Cancelling Job")
            .await?;
        Ok(())
    }

    async fn start_job(&self, _project: &str, _job_id: u64) -> Result<()> {
        Err(anyhow::anyhow!(
            "Starting manual jobs is not supported on GitHub"
        ))
    }

    async fn run_pipeline(
        &self,
        project: &str,
        branch: &str,
        _mr: bool,
        variables: &[(String, String)],
        inputs: &[(String, String)],
        workflow_file: &str,
    ) -> Result<()> {
        let mut args: Vec<String> = Vec::new();
        if !workflow_file.is_empty() {
            args.push(workflow_file.into());
        }
        args.push("-R".into());
        args.push(project.into());
        if !branch.is_empty() {
            args.push("-r".into());
            args.push(branch.into());
        }
        for (k, v) in variables {
            args.push("-f".into());
            args.push(format!("{}={}", k, v));
        }
        for (k, v) in inputs {
            args.push("-f".into());
            args.push(format!("{}={}", k, v));
        }
        let mut cmd: Vec<String> = vec!["workflow".into(), "run".into()];
        cmd.extend(args);
        let cmd_refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
        self.run_gh(&cmd_refs, "RUNNING WORKFLOW").await?;
        Ok(())
    }

    async fn download_artifact(
        &self,
        project: &str,
        _ref_name: &str,
        job_name: &str,
    ) -> Result<()> {
        self.run_gh(
            &["run", "download", "-R", project, "-n", job_name],
            "DOWNLOADING ARTIFACT",
        )
        .await?;
        Ok(())
    }

    // ── Runners ──

    async fn list_runners(&self, project: &str, page_size: usize) -> Result<Vec<Runner>> {
        let endpoint = format!("/repos/{}/actions/runners?per_page={}", project, page_size);
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Runners")
            .await?;
        #[derive(Deserialize)]
        struct GhRunners {
            runners: Vec<GhRunner>,
        }
        #[derive(Deserialize)]
        struct GhRunner {
            id: u64,
            name: String,
            status: String,
        }
        let res: GhRunners = serde_json::from_str(&raw)?;
        Ok(res
            .runners
            .into_iter()
            .map(|r| Runner {
                id: r.id,
                description: Some(r.name),
                status: r.status,
                active: true,
            })
            .collect())
    }

    async fn pause_runner(&self, _project: &str, runner_id: u64) -> Result<()> {
        anyhow::bail!(
            "GitHub runner management requires a repository path; trait method lacks project parameter"
        )
    }

    async fn resume_runner(&self, _project: &str, runner_id: u64) -> Result<()> {
        anyhow::bail!(
            "GitHub runner management requires a repository path; trait method lacks project parameter"
        )
    }

    async fn update_runner_description(
        &self,
        _project: &str,
        runner_id: u64,
        description: &str,
    ) -> Result<()> {
        anyhow::bail!(
            "GitHub runner management requires a repository path; trait method lacks project parameter"
        )
    }

    // ── Releases ──

    async fn list_releases(&self, project: &str, page_size: usize) -> Result<Vec<Release>> {
        let raw = self
            .run_gh(
                &[
                    "release",
                    "list",
                    "--json",
                    "name,tagName,publishedAt,isDraft,isPrerelease,createdAt",
                    "-R",
                    project,
                    "--limit",
                    &page_size.to_string(),
                ],
                "Fetching Releases",
            )
            .await?;

        #[derive(Deserialize)]
        struct GhRel {
            name: Option<String>,
            #[serde(rename = "tagName")]
            tag_name: String,
            #[serde(rename = "publishedAt")]
            published_at: Option<String>,
            #[serde(rename = "createdAt")]
            #[allow(dead_code)]
            created_at: Option<String>,
        }

        let rels: Vec<GhRel> = serde_json::from_str(&raw)?;
        Ok(rels
            .into_iter()
            .map(|r| Release {
                name: r.name.unwrap_or_else(|| r.tag_name.clone()),
                tag_name: r.tag_name,
                released_at: r
                    .published_at
                    .as_deref()
                    .map(|s| s.chars().take(10).collect::<String>())
                    .unwrap_or_default(),
                description: None,
                author_name: None,
                commit_id: None,
                commit_title: None,
                assets_link: None,
            })
            .collect())
    }

    async fn create_release(
        &self,
        project: &str,
        tag: &str,
        name: &str,
        description: &str,
    ) -> Result<()> {
        self.run_gh(
            &[
                "release",
                "create",
                tag,
                "-R",
                project,
                "-t",
                name,
                "-n",
                description,
            ],
            "CREATING RELEASE",
        )
        .await?;
        Ok(())
    }

    async fn update_release(
        &self,
        project: &str,
        tag_name: &str,
        name: &str,
        description: &str,
    ) -> Result<()> {
        self.run_gh(
            &[
                "release",
                "edit",
                tag_name,
                "-R",
                project,
                "-t",
                name,
                "-n",
                description,
            ],
            "Updating Release",
        )
        .await?;
        Ok(())
    }

    async fn delete_release(&self, project: &str, tag_name: &str) -> Result<()> {
        self.run_gh(
            &["release", "delete", tag_name, "-R", project, "-y"],
            "Deleting Release",
        )
        .await?;
        Ok(())
    }

    // ── Milestones ──

    async fn list_milestones(&self, project: &str, page_size: usize) -> Result<Vec<Milestone>> {
        let endpoint = format!(
            "/repos/{}/milestones?state=all&per_page={}",
            project, page_size
        );
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Milestones")
            .await?;
        #[derive(Deserialize, Default)]
        struct GhMs {
            #[serde(default)]
            id: u64,
            #[serde(default)]
            number: u64,
            #[serde(default)]
            title: String,
            description: Option<String>,
            #[serde(default)]
            state: String,
            due_on: Option<String>,
            #[serde(default)]
            created_at: String,
        }
        let milestones: Vec<GhMs> = serde_json::from_str(&raw)?;
        Ok(milestones
            .into_iter()
            .map(|m| {
                let state = if m.state == "open" {
                    "active"
                } else {
                    "closed"
                }
                .to_string();
                let due_date = m
                    .due_on
                    .as_deref()
                    .map(|s| s.chars().take(10).collect::<String>());
                Milestone {
                    id: m.id,
                    iid: m.number,
                    title: m.title,
                    description: m.description,
                    state,
                    start_date: None,
                    due_date,
                    created_at: m.created_at,
                }
            })
            .collect())
    }

    async fn list_milestone_issues(
        &self,
        project: &str,
        milestone_iid: u64,
        _milestone_title: &str,
        page_size: usize,
    ) -> Result<Vec<Issue>> {
        let total = page_size * 10;
        let raw = self
            .run_gh(
                &[
                    "issue",
                    "list",
                    "--json",
                    "number,title,state,labels,author,body,createdAt,updatedAt,closedAt,milestone,assignees",
                    "-R",
                    project,
                    "--milestone",
                    &milestone_iid.to_string(),
                    "--state",
                    "all",
                    "--limit",
                    &total.to_string(),
                ],
                "Fetching Milestone Issues",
            )
            .await?;
        #[derive(Deserialize)]
        struct GhIssue {
            number: u64,
            title: String,
            state: String,
            #[serde(default)]
            labels: Vec<serde_json::Value>,
            author: Option<GhLogin>,
            body: Option<String>,
            #[serde(rename = "createdAt")]
            #[allow(dead_code)]
            created_at: String,
            #[serde(rename = "updatedAt")]
            updated_at: String,
            #[serde(rename = "closedAt")]
            closed_at: Option<String>,
            milestone: Option<GhMs>,
            #[serde(default)]
            assignees: Vec<GhLogin>,
        }
        #[derive(Deserialize)]
        struct GhLogin {
            login: String,
        }
        #[derive(Deserialize)]
        struct GhMs {
            title: String,
        }
        let gh_issues: Vec<GhIssue> = serde_json::from_str(&raw)?;
        Ok(gh_issues
            .into_iter()
            .map(|gi| {
                let state = if gi.state == "OPEN" {
                    "opened"
                } else {
                    "closed"
                }
                .to_string();
                let labels: Vec<String> = gi
                    .labels
                    .iter()
                    .filter_map(|v| v.get("name")?.as_str().map(String::from))
                    .collect();
                let author = crate::domain::issues::Author {
                    username: gi.author.map(|a| a.login).unwrap_or_default(),
                };
                let milestone = gi
                    .milestone
                    .map(|m| crate::domain::issues::Milestone { title: m.title });
                let assignees: Vec<crate::domain::issues::Assignee> = gi
                    .assignees
                    .into_iter()
                    .map(|a| crate::domain::issues::Assignee { username: a.login })
                    .collect();
                Issue {
                    iid: gi.number,
                    title: gi.title,
                    state,
                    labels,
                    updated_at: gi.updated_at,
                    created_at: Some(gi.created_at),
                    closed_at: gi.closed_at,
                    author,
                    milestone,
                    assignees,
                    description: gi.body,
                    due_date: None,
                }
            })
            .collect())
    }

    async fn create_milestone(
        &self,
        project: &str,
        title: &str,
        description: &str,
        _start_date: Option<&str>,
        due_date: Option<&str>,
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "api".into(),
            format!("repos/{}/milestones", project),
            "-f".into(),
            format!("title={}", title),
        ];
        if !description.is_empty() {
            args.push("-f".into());
            args.push(format!("description={}", description));
        }
        if let Some(due) = due_date {
            if !due.is_empty() {
                let iso_due = if due.contains('T') {
                    due.to_string()
                } else {
                    format!("{}T00:00:00Z", due)
                };
                args.push("-f".into());
                args.push(format!("due_on={}", iso_due));
            }
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_gh(&args_refs, "CREATING MILESTONE").await?;
        Ok(())
    }

    async fn update_milestone_state(
        &self,
        project: &str,
        milestone_iid: u64,
        close: bool,
    ) -> Result<()> {
        let state = if close { "closed" } else { "open" };
        let desc = if close {
            "CLOSING MILESTONE"
        } else {
            "REOPENING MILESTONE"
        };
        self.run_gh(
            &[
                "api",
                "-X",
                "PATCH",
                &format!("repos/{}/milestones/{}", project, milestone_iid),
                "-f",
                &format!("state={}", state),
            ],
            desc,
        )
        .await?;
        Ok(())
    }

    async fn update_milestone(
        &self,
        project: &str,
        milestone_iid: u64,
        title: &str,
        description: &str,
        _start_date: Option<&str>,
        due_date: Option<&str>,
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "api".into(),
            "-X".into(),
            "PATCH".into(),
            format!("repos/{}/milestones/{}", project, milestone_iid),
            "-f".into(),
            format!("title={}", title),
        ];
        if !description.is_empty() {
            args.push("-f".into());
            args.push(format!("description={}", description));
        }
        if let Some(due) = due_date {
            if !due.is_empty() {
                let iso_due = if due.contains('T') {
                    due.to_string()
                } else {
                    format!("{}T23:59:59Z", due)
                };
                args.push("-f".into());
                args.push(format!("due_on={}", iso_due));
            }
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_gh(&args_refs, "Updating Milestone").await?;
        Ok(())
    }

    async fn delete_milestone(&self, project: &str, milestone_iid: u64) -> Result<()> {
        self.run_gh(
            &[
                "api",
                "-X",
                "DELETE",
                &format!("repos/{}/milestones/{}", project, milestone_iid),
            ],
            "Deleting Milestone",
        )
        .await?;
        Ok(())
    }

    // ── Notifications ──

    async fn list_notifications(&self, show_read: bool) -> Result<Vec<Notification>> {
        let endpoint = if show_read {
            "notifications?all=true"
        } else {
            "notifications"
        };
        let raw = self
            .raw_api(endpoint, "GET", None, "Fetching Todos")
            .await?;
        #[derive(Deserialize)]
        struct GhNotif {
            id: String,
            repository: GhNotifRepo,
            subject: GhNotifSubject,
            unread: bool,
            updated_at: String,
        }
        #[derive(Deserialize)]
        struct GhNotifRepo {
            full_name: String,
        }
        #[derive(Deserialize)]
        struct GhNotifSubject {
            title: String,
            r#type: String,
            url: String,
        }
        let gh_notifs: Vec<GhNotif> = serde_json::from_str(&raw)?;
        Ok(gh_notifs
            .into_iter()
            .map(|item| {
                let target_type = if item.subject.r#type == "PullRequest" {
                    "MergeRequest".to_string()
                } else {
                    item.subject.r#type
                };
                let target_iid = item
                    .subject
                    .url
                    .split('/')
                    .last()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                let state = if item.unread {
                    "unread".to_string()
                } else {
                    "read".to_string()
                };
                Notification {
                    id: item.id,
                    project_path: item.repository.full_name,
                    title: item.subject.title,
                    target_type,
                    target_iid,
                    state,
                    updated_at: item.updated_at,
                }
            })
            .collect())
    }

    async fn mark_notification_as_read(&self, id: &str) -> Result<()> {
        let endpoint = format!("notifications/threads/{}", id);
        self.raw_api(&endpoint, "PATCH", None, "Marking Todo Done")
            .await?;
        Ok(())
    }

    // ── Branches ──

    async fn list_branches(&self, project: &str, page_size: usize) -> Result<Vec<Branch>> {
        let endpoint = format!("/repos/{}/branches?per_page={}", project, page_size);
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Branches")
            .await?;
        #[derive(Deserialize)]
        struct GhBr {
            name: String,
            #[serde(default)]
            protected: bool,
            commit: Option<GhBrCommit>,
        }
        #[derive(Deserialize)]
        struct GhBrCommit {
            sha: String,
        }
        #[derive(Deserialize)]
        struct GhRepo {
            default_branch: String,
        }
        let repo_raw = self
            .raw_api(
                &format!("/repos/{}", project),
                "GET",
                None,
                "Fetching Repo Info",
            )
            .await?;
        let repo: GhRepo = serde_json::from_str(&repo_raw)?;
        let default_branch = repo.default_branch;

        let branches: Vec<Branch> = serde_json::from_str::<Vec<GhBr>>(&raw)?
            .into_iter()
            .map(|b| Branch {
                default: b.name == default_branch,
                protected: b.protected,
                web_url: format!("https://github.com/{}/tree/{}", project, b.name),
                name: b.name,
                can_push: false,
                commit_sha: b.commit.as_ref().map(|c| c.sha.clone()).unwrap_or_default(),
            })
            .collect();
        Ok(branches)
    }

    async fn create_branch(
        &self,
        project: &str,
        branch_name: &str,
        ref_branch: &str,
    ) -> Result<()> {
        let endpoint = format!("/repos/{}/git/refs", project);
        let payload = serde_json::json!({
            "ref": format!("refs/heads/{}", branch_name),
            "sha": ref_branch,
        });
        let json_str = serde_json::to_string(&payload)?;
        self.raw_api(&endpoint, "POST", Some(&json_str), "Creating Branch")
            .await?;
        Ok(())
    }

    async fn delete_branch(&self, project: &str, branch_name: &str) -> Result<()> {
        let endpoint = format!("/repos/{}/git/refs/heads/{}", project, branch_name);
        self.raw_api(&endpoint, "DELETE", None, "Deleting Branch")
            .await?;
        Ok(())
    }

    // ── Environments / Deployments ──

    async fn list_environments(&self, project: &str, page_size: usize) -> Result<Vec<Environment>> {
        let endpoint = format!("/repos/{}/environments?per_page={}", project, page_size);
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Environments")
            .await?;
        #[derive(Deserialize)]
        struct GhEnvResp {
            environments: Vec<GhEnv>,
        }
        #[derive(Deserialize)]
        struct GhEnv {
            id: u64,
            name: String,
            #[serde(default)]
            html_url: Option<String>,
        }
        let resp: GhEnvResp = serde_json::from_str(&raw)?;
        Ok(resp
            .environments
            .into_iter()
            .map(|e| Environment {
                id: e.id,
                name: e.name,
                state: "available".to_string(),
                external_url: e.html_url,
                last_deployment: None,
            })
            .collect())
    }

    async fn list_deployments(
        &self,
        project: &str,
        page_size: usize,
        environment: Option<&str>,
    ) -> Result<Vec<Deployment>> {
        let mut endpoint = format!("/repos/{}/deployments?per_page={}", project, page_size);
        if let Some(env) = environment {
            endpoint.push_str(&format!("&environment={}", env));
        }
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Deployments")
            .await?;
        #[derive(Deserialize)]
        struct GhDeploy {
            id: u64,
            sha: String,
            #[serde(rename = "ref")]
            ref_name: String,
            #[serde(default)]
            description: String,
            environment: Option<String>,
            created_at: String,
            updated_at: String,
            #[serde(default)]
            status: Option<String>,
        }
        let deploys: Vec<GhDeploy> = serde_json::from_str(&raw)?;
        Ok(deploys
            .into_iter()
            .map(|d| Deployment {
                id: d.id,
                iid: d.id,
                ref_name: d.ref_name,
                tag: false,
                sha: d.sha,
                status: d.status.unwrap_or_default(),
                created_at: d.created_at,
                updated_at: d.updated_at,
                environment: d
                    .environment
                    .map(|e| crate::domain::deployments::EnvironmentInfo {
                        name: e,
                        external_url: None,
                    }),
                deployable: None,
                description: d.description,
                user: None,
            })
            .collect())
    }

    // ── Labels / Members / Misc ──

    async fn fetch_labels(&self, project: &str, _per_request: usize) -> Result<Vec<Label>> {
        let raw = self
            .run_gh(
                &[
                    "label",
                    "list",
                    "--json",
                    "name,color",
                    "-R",
                    project,
                    "--limit",
                    "100",
                ],
                "Fetching Labels",
            )
            .await?;
        #[derive(Deserialize)]
        struct GhLabel {
            name: String,
            #[serde(default)]
            color: String,
        }
        let labels: Vec<GhLabel> = serde_json::from_str(&raw)?;
        Ok(labels
            .into_iter()
            .map(|l| Label {
                name: l.name,
                color: if l.color.is_empty() {
                    None
                } else {
                    Some(l.color)
                },
            })
            .collect())
    }

    async fn fetch_members(&self, project: &str) -> Result<Vec<String>> {
        let endpoint = format!("/repos/{}/assignees?per_page=100", project);
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Members")
            .await?;
        #[derive(Deserialize)]
        struct GhAsn {
            login: String,
        }
        let members: Vec<GhAsn> = serde_json::from_str(&raw)?;
        Ok(members
            .into_iter()
            .map(|a| format!("@{}", a.login))
            .collect())
    }

    // ── Browser ──

    async fn open_in_browser(&self, _project: &str, entity: &str, id: &str) -> Result<()> {
        self.run_gh(&[entity, "view", id, "--web"], "OPENING IN BROWSER")
            .await?;
        Ok(())
    }

    async fn open_pipeline_in_browser(&self, _project: &str, id: &str) -> Result<()> {
        self.run_gh(&["run", "view", id, "--web"], "OPENING IN BROWSER")
            .await?;
        Ok(())
    }

    async fn open_workflow_in_browser(&self, project: &str, workflow: &str) -> Result<()> {
        self.run_gh(
            &["workflow", "view", workflow, "-R", project, "--web"],
            "OPENING WORKFLOW IN BROWSER",
        )
        .await?;
        Ok(())
    }

    async fn open_job_in_browser(&self, _project: &str, id: &str) -> Result<()> {
        self.run_gh(&["run", "view", id, "--web"], "OPENING IN BROWSER")
            .await?;
        Ok(())
    }

    async fn open_milestone_in_browser(&self, project: &str, id: &str) -> Result<()> {
        let url = format!("https://github.com/{}/milestone/{}", project, id);
        let label = "OPENING IN BROWSER";
        let cmd_str = format!("git web--browse {}", url);
        let output = tokio::process::Command::new("git")
            .args(["web--browse", &url])
            .output()
            .await;
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        let status = match &output {
            Ok(out) if out.status.success() => "Success".to_string(),
            _ => "Success".to_string(),
        };
        if let Some(ref tx) = self.tx {
            let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                timestamp,
                command: format!("{}: {}", label, cmd_str),
                status,
            });
        }
        Ok(())
    }
    // ── Raw API ──

    async fn raw_api(
        &self,
        endpoint: &str,
        method: &str,
        body: Option<&str>,
        desc: &str,
    ) -> Result<String> {
        let mut cmd_args: Vec<String> = vec!["api".into()];
        if method != "GET" {
            cmd_args.push("-X".into());
            cmd_args.push(method.into());
        }
        cmd_args.push(endpoint.into());
        let cmd_str = format!("gh {}", cmd_args.join(" "));
        let label = desc.to_uppercase();

        let mut cmd = Command::new("gh");
        cmd.arg("api");
        if method != "GET" {
            cmd.arg("-X");
            cmd.arg(method);
        }
        if let Some(b) = body {
            if !b.is_empty() {
                cmd.arg("--input");
                cmd.arg("-");
                cmd.stdin(std::process::Stdio::piped());
            }
        }
        cmd.arg(endpoint);

        let output = if let Some(b) = body {
            if !b.is_empty() {
                let mut child = cmd.spawn().context("Failed to spawn gh api command")?;
                use tokio::io::AsyncWriteExt;
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(b.as_bytes()).await?;
                    stdin.flush().await?;
                }
                child.wait_with_output().await
            } else {
                cmd.output().await
            }
        } else {
            cmd.output().await
        };

        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        match output {
            Ok(out) => {
                if out.status.success() {
                    let s = String::from_utf8(out.stdout)?;
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(Event::TerminalCommandLogged {
                            timestamp,
                            command: format!("{}: {}", label, cmd_str),
                            status: "Success".to_string(),
                        });
                    }
                    Ok(s)
                } else {
                    let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(Event::TerminalCommandLogged {
                            timestamp,
                            command: format!("{}: {}", label, cmd_str),
                            status: format!("Failed: {}", err_msg),
                        });
                    }
                    anyhow::bail!("gh api failed: {}", err_msg)
                }
            }
            Err(e) => {
                let err_msg = format!("{}", e);
                if let Some(ref tx) = self.tx {
                    let _ = tx.send(Event::TerminalCommandLogged {
                        timestamp,
                        command: format!("{}: {}", label, cmd_str),
                        status: format!("Failed: {}", err_msg),
                    });
                }
                Err(e.into())
            }
        }
    }
}

/// Compute duration in seconds between two ISO 8601 timestamps.
fn chrono_duration(start: &str, end: &str) -> Option<u64> {
    let s = chrono::DateTime::parse_from_rfc3339(start).ok()?;
    let e = chrono::DateTime::parse_from_rfc3339(end).ok()?;
    let diff = e.signed_duration_since(s);
    if diff < chrono::Duration::zero() {
        return None;
    }
    Some(diff.num_seconds() as u64)
}

/// Fetch the actor login for a single GitHub Actions run.
///
/// `gh run list` does not expose the actor, so this hits
/// `GET repos/{}/actions/runs/{id}` and reads `actor.login`. Returns `None` on
/// any failure rather than propagating — an unknown actor must never block the
/// pipelines list.
async fn fetch_actor_login(project: &str, run_id: u64) -> Option<String> {
    let raw = Command::new("gh")
        .args([
            "api",
            "-H",
            "Accept: application/vnd.github+json",
            &format!("repos/{project}/actions/runs/{run_id}"),
        ])
        .output()
        .await
        .ok()?;
    if !raw.status.success() {
        return None;
    }
    let body: serde_json::Value = serde_json::from_slice(&raw.stdout).ok()?;
    body.get("actor")
        .and_then(|a| a.get("login"))
        .and_then(|l| l.as_str())
        .map(|s| s.to_string())
}

/// Populate `actor_login` for each pipeline.
///
/// `gh run list` does not expose the actor login, so the bulk listing leaves
/// every row blank. Each run needs its own fetch, but a Pipelines tab can
/// easily hold hundreds of runs, so a `Semaphore` caps the in-flight
/// subprocesses at `MAX_GH_METADATA_CONCURRENCY`. Failures are non-fatal: a
/// missing actor just stays empty, which the column renders as `—`.
async fn enrich_pipeline_actors(project: &str, pipelines: &mut [Pipeline]) {
    let permits = Arc::new(Semaphore::new(MAX_GH_METADATA_CONCURRENCY));
    let mut tasks: JoinSet<(u64, Option<String>)> = JoinSet::new();

    for p in pipelines.iter() {
        if !p.actor_login.is_empty() {
            continue;
        }
        let id = p.id;
        let project = project.to_string();
        let permits = Arc::clone(&permits);
        tasks.spawn(async move {
            let _permit = permits.acquire_owned().await.ok();
            let login = fetch_actor_login(&project, id).await;
            (id, login)
        });
    }

    while let Some(joined) = tasks.join_next().await {
        let Ok((id, login)) = joined else { continue };
        let Some(login) = login else { continue };
        if let Some(p) = pipelines.iter_mut().find(|p| p.id == id) {
            p.actor_login = login;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ats() {
        assert_eq!(strip_ats(""), "");
        assert_eq!(strip_ats("@user1"), "user1");
        assert_eq!(strip_ats("@user1, @user2"), "user1,user2");
        assert_eq!(strip_ats("user1, @user2, @user3"), "user1,user2,user3");
        assert_eq!(strip_ats("user1"), "user1");
    }

    #[test]
    fn test_normalize_labels() {
        assert_eq!(normalize_labels(""), "");
        assert_eq!(normalize_labels("bug, feature"), "bug,feature");
        assert_eq!(normalize_labels("bug,feature"), "bug,feature");
        assert_eq!(normalize_labels("bug"), "bug");
    }

    #[test]
    fn chrono_duration_rejects_invalid_and_negative_ranges() {
        assert_eq!(
            chrono_duration("2026-01-01T00:00:00Z", "2026-01-01T00:02:05Z"),
            Some(125)
        );
        assert_eq!(
            chrono_duration("2026-01-01T00:02:05Z", "2026-01-01T00:00:00Z"),
            None
        );
        assert_eq!(chrono_duration("invalid", "invalid"), None);
    }

    #[test]
    fn deserialize_needs_accepts_missing_null_string_and_array_shapes() {
        #[derive(Deserialize)]
        struct Fixture {
            #[serde(default, deserialize_with = "deserialize_needs")]
            needs: Vec<String>,
        }

        assert!(
            serde_json::from_str::<Fixture>(r#"{}"#)
                .unwrap()
                .needs
                .is_empty()
        );
        assert!(
            serde_json::from_str::<Fixture>(r#"{"needs":null}"#)
                .unwrap()
                .needs
                .is_empty()
        );
        assert_eq!(
            serde_json::from_str::<Fixture>(r#"{"needs":"build"}"#)
                .unwrap()
                .needs,
            vec!["build"]
        );
        assert_eq!(
            serde_json::from_str::<Fixture>(r#"{"needs":["build","test"]}"#)
                .unwrap()
                .needs,
            vec!["build", "test"]
        );
    }

    #[test]
    fn parse_gh_login_reads_a_normal_login() {
        assert_eq!(parse_gh_login("octocat"), Some("octocat".to_string()));
    }

    #[test]
    fn parse_gh_login_trims_the_trailing_newline_jq_emits() {
        // `gh api user --jq .login` emits the login followed by a newline.
        assert_eq!(parse_gh_login("octocat\n"), Some("octocat".to_string()));
    }

    #[test]
    fn parse_gh_login_empty_string_is_none() {
        assert_eq!(parse_gh_login(""), None);
    }

    #[test]
    fn parse_gh_login_whitespace_only_is_none_not_some_empty_string() {
        assert_eq!(parse_gh_login("   \n"), None);
    }

    #[test]
    fn split_review_authors_separates_approvals_from_all_reviews() {
        // One of each: an approval, a rejection, and a non-blocking comment.
        // `you_reviewed` must see all three; `approved_by` must see only the
        // approver.
        let reviews = vec![
            serde_json::json!({"author": {"login": "approver"}, "state": "APPROVED"}),
            serde_json::json!({"author": {"login": "rejecter"}, "state": "CHANGES_REQUESTED"}),
            serde_json::json!({"author": {"login": "commenter"}, "state": "COMMENTED"}),
        ];

        let (all_authors, approved_authors) = split_review_authors(&reviews);

        assert_eq!(
            all_authors,
            vec![
                "approver".to_string(),
                "rejecter".to_string(),
                "commenter".to_string(),
            ]
        );
        assert_eq!(approved_authors, vec!["approver".to_string()]);
    }

    #[tokio::test]
    async fn current_user_cache_is_shared_across_cloned_backend_instances() {
        // Regression guard for the bug this cache exists to avoid: an earlier
        // version cached in a `OnceCell` field *on* `GhBackend`. But
        // `GitlabClient::clone` rebuilds the backend from scratch
        // (`create_backend`), and every refresh clones the client
        // (`spawn_refresh_active_tab`), so a per-instance cell would start
        // empty on every refresh and never see a second call — zero cache
        // hits, ever. A test that only builds one `OnceCell` locally (as the
        // previous version of this test did) cannot see that bug: it passes
        // identically whether the cache is shared, per-instance, or absent.
        //
        // This test instead exercises the actual static, `GH_CURRENT_USER`,
        // through what stand in for two independently-cloned backends —
        // there is nothing left to construct per-instance, which is exactly
        // the fix: the cache no longer lives on the struct at all.
        use std::sync::atomic::{AtomicUsize, Ordering};

        static CALLS: AtomicUsize = AtomicUsize::new(0);
        async fn init() -> Option<String> {
            CALLS.fetch_add(1, Ordering::SeqCst);
            Some("cached-user".to_string())
        }

        let _first_backend = GhBackend::new();
        let _second_backend = GhBackend::new();

        let a = GH_CURRENT_USER.get_or_init(init).await.clone();
        let b = GH_CURRENT_USER.get_or_init(init).await.clone();

        assert_eq!(a, Some("cached-user".to_string()));
        assert_eq!(a, b);
        assert_eq!(
            CALLS.load(Ordering::SeqCst),
            1,
            "the initializer must run once total, not once per backend instance"
        );
    }

    #[test]
    fn blocked_merge_state_is_not_a_conflict() {
        // REGRESSION GUARD. Every open PR sampled on ratatui/ratatui was
        // mergeable=MERGEABLE + mergeStateStatus=BLOCKED, which means blocked
        // by branch protection. Mapping BLOCKED to "conflict" would show false
        // conflicts on most GitHub PRs.
        let (_, merge) = gh_state_from_fields(
            Some("REVIEW_REQUIRED"),
            Some("MERGEABLE"),
            Some("BLOCKED"),
            vec![],
            None,
            &[],
        );
        let m = merge.unwrap();
        assert!(!m.conflicts);
        assert!(!m.needs_rebase);
        assert!(!m.computing);
    }

    #[test]
    fn conflicting_maps_to_conflicts() {
        let (_, merge) =
            gh_state_from_fields(None, Some("CONFLICTING"), Some("DIRTY"), vec![], None, &[]);
        assert!(merge.unwrap().conflicts);
    }

    #[test]
    fn behind_maps_to_needs_rebase() {
        let (_, merge) =
            gh_state_from_fields(None, Some("MERGEABLE"), Some("BEHIND"), vec![], None, &[]);
        assert!(merge.unwrap().needs_rebase);
    }

    #[test]
    fn unknown_mergeable_is_computing_not_failure() {
        // GitHub computes mergeability asynchronously.
        let (_, merge) =
            gh_state_from_fields(None, Some("UNKNOWN"), Some("UNKNOWN"), vec![], None, &[]);
        let m = merge.unwrap();
        assert!(m.computing);
        assert!(!m.conflicts);
    }

    #[test]
    fn absent_mergeable_is_computing() {
        let (_, merge) = gh_state_from_fields(None, None, None, vec![], None, &[]);
        assert!(merge.unwrap().computing);
    }

    #[test]
    fn review_decision_approved_maps_to_approved_with_authors() {
        let (approval, _) = gh_state_from_fields(
            Some("APPROVED"),
            Some("MERGEABLE"),
            Some("CLEAN"),
            vec!["octocat".to_string()],
            None,
            &[],
        );
        let a = approval.unwrap();
        assert!(a.approved);
        assert_eq!(a.approved_by, vec!["octocat".to_string()]);
        assert!(!a.changes_requested);
    }

    #[test]
    fn review_decision_changes_requested_maps_through() {
        let (approval, _) = gh_state_from_fields(
            Some("CHANGES_REQUESTED"),
            Some("MERGEABLE"),
            Some("BLOCKED"),
            vec![],
            None,
            &[],
        );
        assert!(approval.unwrap().changes_requested);
    }

    #[test]
    fn github_never_sets_counts_or_awaiting_you() {
        // canApprove has no gh equivalent, so the ● marker is unreachable.
        let (approval, _) = gh_state_from_fields(
            Some("APPROVED"),
            Some("MERGEABLE"),
            Some("CLEAN"),
            vec!["octocat".to_string()],
            None,
            &[],
        );
        let a = approval.unwrap();
        assert_eq!(a.approvals_required, None);
        assert_eq!(a.approvals_left, None);
        assert!(!a.awaiting_you);
    }

    #[test]
    fn null_review_decision_is_pending_not_unknown() {
        // A PR with no review yet returns null; that is "pending", and the
        // axis is still known.
        let (approval, _) =
            gh_state_from_fields(None, Some("MERGEABLE"), Some("CLEAN"), vec![], None, &[]);
        let a = approval.unwrap();
        assert!(!a.approved);
        assert!(!a.changes_requested);
    }

    #[test]
    fn github_carries_the_current_user_through() {
        let (approval, _) = gh_state_from_fields(
            Some("APPROVED"),
            Some("MERGEABLE"),
            Some("CLEAN"),
            vec!["octocat".to_string()],
            Some("octocat"),
            &[],
        );
        let a = approval.unwrap();
        assert_eq!(a.current_user.as_deref(), Some("octocat"));
    }

    #[test]
    fn github_you_reviewed_is_true_when_you_are_in_latest_reviews() {
        let (approval, _) = gh_state_from_fields(
            Some("APPROVED"),
            Some("MERGEABLE"),
            Some("CLEAN"),
            vec!["octocat".to_string()],
            Some("octocat"),
            &[],
        );
        assert!(approval.unwrap().you_reviewed);
    }

    #[test]
    fn github_you_reviewed_is_false_when_someone_else_reviewed() {
        let (approval, _) = gh_state_from_fields(
            Some("APPROVED"),
            Some("MERGEABLE"),
            Some("CLEAN"),
            vec!["someone.else".to_string()],
            Some("octocat"),
            &[],
        );
        assert!(!approval.unwrap().you_reviewed);
    }

    #[test]
    fn github_unknown_current_user_leaves_it_none() {
        let (approval, _) = gh_state_from_fields(
            Some("APPROVED"),
            Some("MERGEABLE"),
            Some("CLEAN"),
            vec![],
            None,
            &[],
        );
        assert_eq!(approval.unwrap().current_user, None);
    }

    // ── you_approved (CRITICAL/IMPORTANT finding: was hard-coded `false`,
    // making `ApprovedByYou` unreachable on GitHub and rendering a PR you
    // personally approved as blank "not yours" once you dropped out of
    // reviewRequests) ──

    #[test]
    fn github_you_approved_is_true_when_you_are_in_approved_authors() {
        let (approval, _) = gh_state_from_fields(
            Some("APPROVED"),
            Some("MERGEABLE"),
            Some("CLEAN"),
            vec!["octocat".to_string()],
            Some("octocat"),
            &["octocat".to_string()],
        );
        assert!(approval.unwrap().you_approved);
    }

    #[test]
    fn github_you_approved_is_false_when_only_someone_else_approved() {
        let (approval, _) = gh_state_from_fields(
            Some("APPROVED"),
            Some("MERGEABLE"),
            Some("CLEAN"),
            vec!["someone.else".to_string()],
            Some("octocat"),
            &["someone.else".to_string()],
        );
        assert!(!approval.unwrap().you_approved);
    }
}
