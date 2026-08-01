use super::Backend;
use crate::domain::branches::Branch;
use crate::domain::deployments::{Deployment, Environment};
use crate::domain::issues::Issue;
use crate::domain::milestones::Milestone;
use crate::domain::mr::{DiscussionNote, MergeRequest};
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

/// Number of `--per-page per_request` calls needed to cover a `page_size` item budget.
fn page_count(page_size: usize, per_request: usize) -> usize {
    page_size.div_ceil(per_request.max(1)).max(1)
}

/// GitLab's `mergeRequests(iids: [...])` GraphQL connection caps results at
/// 100 nodes when no `first:` argument is given. `page_size` (src/config.rs)
/// is user-configurable above that, so a single query can silently drop MRs
/// past the 100th — they'd render "—" on both axes with no error. Chunk so
/// no single query can exceed the cap.
const MR_STATE_QUERY_BATCH_SIZE: usize = 100;

/// Split `iids` into batches of at most `batch_size`, preserving order.
fn chunk_iids(iids: &[u64], batch_size: usize) -> Vec<&[u64]> {
    if batch_size == 0 {
        return vec![iids];
    }
    iids.chunks(batch_size).collect()
}

type MrStateMap = HashMap<
    u64,
    (
        Option<crate::domain::mr_state::ApprovalState>,
        Option<crate::domain::mr_state::MergeabilityState>,
    ),
>;

/// The project path originates from `git remote get-url` and is interpolated
/// into a GraphQL query string, so it must be constrained before use.
/// Permits only the characters that legitimately appear in a GitLab path.
pub fn validate_project_path(path: &str) -> anyhow::Result<()> {
    if path.is_empty() {
        anyhow::bail!("empty project path");
    }
    let ok = path
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'));
    if !ok {
        anyhow::bail!("project path contains unsupported characters: {}", path);
    }
    Ok(())
}

/// Parse the bulk MR-state GraphQL response.
///
/// A top-level `errors` array is a hard error: GraphQL is all-or-nothing, so a
/// single unknown field yields no data at all, and that must not be mistaken
/// for "this project has no approval state".
pub fn parse_mr_state_response(json: &str) -> anyhow::Result<MrStateMap> {
    use crate::domain::mr_state::{
        ApprovalState, MergeabilityState, derive_awaiting_you, is_transient_merge_status,
    };

    let root: serde_json::Value = serde_json::from_str(json)?;

    if let Some(errors) = root.get("errors").and_then(|e| e.as_array()) {
        let first = errors
            .first()
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("unknown GraphQL error");
        anyhow::bail!("GraphQL error: {}", first);
    }

    let data = root
        .get("data")
        .ok_or_else(|| anyhow::anyhow!("GraphQL response has no data"))?;

    let current_user = data
        .get("currentUser")
        .and_then(|u| u.get("username"))
        .and_then(|u| u.as_str())
        .unwrap_or_default()
        .to_string();

    let nodes = data
        .get("project")
        .and_then(|p| p.get("mergeRequests"))
        .and_then(|m| m.get("nodes"))
        .and_then(|n| n.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = HashMap::new();
    for node in nodes {
        // GraphQL returns iid as a string. A malformed one drops that row only.
        let Some(iid) = node
            .get("iid")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
        else {
            continue;
        };

        let approved = node
            .get("approved")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let approved_by: Vec<String> = node
            .get("approvedBy")
            .and_then(|a| a.get("nodes"))
            .and_then(|n| n.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|u| u.get("username")?.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let can_approve = node
            .get("userPermissions")
            .and_then(|p| p.get("canApprove"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Per-reviewer reviewState, NOT detailedMergeStatus: the latter is
        // precedence-ordered and masks this.
        let changes_requested = node
            .get("reviewers")
            .and_then(|r| r.get("nodes"))
            .and_then(|n| n.as_array())
            .map(|arr| {
                arr.iter().any(|r| {
                    r.get("mergeRequestInteraction")
                        .and_then(|i| i.get("reviewState"))
                        .and_then(|s| s.as_str())
                        .map(|s| s.eq_ignore_ascii_case("REQUESTED_CHANGES"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);

        let you_approved =
            !current_user.is_empty() && approved_by.iter().any(|u| *u == current_user);

        // Your own review state, matched by username. Someone else having
        // reviewed must not set this.
        let you_reviewed = !current_user.is_empty()
            && (approved_by.iter().any(|u| *u == current_user)
                || node
                    .get("reviewers")
                    .and_then(|r| r.get("nodes"))
                    .and_then(|n| n.as_array())
                    .map(|arr| {
                        arr.iter().any(|r| {
                            let is_me = r
                                .get("username")
                                .and_then(|u| u.as_str())
                                .map(|u| u == current_user)
                                .unwrap_or(false);
                            let reviewed = r
                                .get("mergeRequestInteraction")
                                .and_then(|i| i.get("reviewState"))
                                .and_then(|s| s.as_str())
                                .map(|s| !s.eq_ignore_ascii_case("UNREVIEWED"))
                                .unwrap_or(false);
                            is_me && reviewed
                        })
                    })
                    .unwrap_or(false));

        let approval = ApprovalState {
            approved,
            approvals_left: node
                .get("approvalsLeft")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            approvals_required: node
                .get("approvalsRequired")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            approved_by,
            changes_requested,
            you_approved,
            awaiting_you: derive_awaiting_you(can_approve, you_approved, approved),
            current_user: if current_user.is_empty() {
                None
            } else {
                Some(current_user.clone())
            },
            you_reviewed,
        };

        let raw_status = node
            .get("detailedMergeStatus")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        let mergeability = MergeabilityState {
            conflicts: node
                .get("conflicts")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            needs_rebase: node
                .get("shouldBeRebased")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            computing: is_transient_merge_status(raw_status),
        };

        out.insert(iid, (Some(approval), Some(mergeability)));
    }
    Ok(out)
}

/// Maximum `glab` subprocesses in flight for one paged fetch.
const MAX_CONCURRENT_REQUESTS: usize = 8;

/// Run one `glab` invocation and log it to the terminal pane.
///
/// Free-standing rather than a method so its future is `'static` and can be
/// spawned: `tx` is the only state a command needs from `GlabBackend`.
async fn run_glab_command(
    tx: Option<UnboundedSender<Event>>,
    args: Vec<String>,
    desc: String,
) -> Result<String> {
    let label = desc.to_uppercase();
    let cmd_str = format!("glab {}", args.join(" "));

    let output = Command::new("glab")
        .args(&args)
        .output()
        .await
        .with_context(|| format!("Failed to execute: glab {}", args.join(" ")))?;

    let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
    if output.status.success() {
        let s = String::from_utf8(output.stdout)?;
        if let Some(ref tx) = tx {
            let _ = tx.send(Event::TerminalCommandLogged {
                timestamp,
                command: format!("{}: {}", label, cmd_str),
                status: "Success".to_string(),
            });
        }
        Ok(s)
    } else {
        let err_msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if let Some(ref tx) = tx {
            let _ = tx.send(Event::TerminalCommandLogged {
                timestamp,
                command: format!("{}: {}", label, cmd_str),
                status: format!("Failed: {}", err_msg),
            });
        }
        anyhow::bail!("glab command failed: {}", err_msg)
    }
}

/// Issue every request concurrently and return each response paired with the
/// index of the request that produced it, in ascending index order.
///
/// At most `MAX_CONCURRENT_REQUESTS` subprocesses run at once; beyond that the
/// fetch degrades to waves rather than a subprocess storm. A task that panics
/// or is cancelled yields an `Err` for its index rather than a silently
/// missing response, so no caller can mistake it for a short page.
async fn run_glab_concurrent(
    tx: Option<UnboundedSender<Event>>,
    requests: Vec<Vec<String>>,
    desc: &str,
) -> Vec<(usize, Result<String>)> {
    let total = requests.len();
    let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
    let mut tasks = JoinSet::new();

    for (index, args) in requests.into_iter().enumerate() {
        let tx = tx.clone();
        let desc = desc.to_string();
        let permits = Arc::clone(&permits);
        tasks.spawn(async move {
            // Held until the request finishes, so the permit — not the number
            // of spawned tasks — bounds the subprocesses in flight.
            let _permit = permits.acquire_owned().await;
            (index, run_glab_command(tx, args, desc).await)
        });
    }

    let mut slots: Vec<Option<Result<String>>> = (0..total).map(|_| None).collect();
    let mut join_failure: Option<String> = None;
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok((index, response)) => slots[index] = Some(response),
            // A panicked or cancelled task never reports its index; the empty
            // slot it leaves behind is filled in below.
            Err(e) => join_failure = Some(e.to_string()),
        }
    }

    slots
        .into_iter()
        .enumerate()
        .map(|(index, slot)| {
            let response = slot.unwrap_or_else(|| {
                let cause = join_failure
                    .clone()
                    .unwrap_or_else(|| "task did not complete".to_string());
                Err(anyhow::anyhow!("request {} failed: {}", index + 1, cause))
            });
            (index, response)
        })
        .collect()
}

/// Assemble indexed responses into index order, failing on the first error.
///
/// Mirrors the `?` the sequential page loops used: one failed page fails the
/// whole fetch, because rendering a silently truncated list is worse than an
/// error. Sorting first means the surfaced error is the lowest-indexed
/// failure, not whichever task happened to lose the race.
fn ordered_or_first_error<T>(mut results: Vec<(usize, Result<T>)>) -> Result<Vec<T>> {
    results.sort_by_key(|(index, _)| *index);
    let mut ordered = Vec::with_capacity(results.len());
    for (_, result) in results {
        ordered.push(result?);
    }
    Ok(ordered)
}

/// Merge indexed batch results, tolerating partial failure.
///
/// A failed batch shouldn't poison the batches that did succeed — the caller
/// (src/fetch.rs) already degrades a missing entry to "—" per row. But if every
/// batch failed, surface the error rather than returning a suspiciously-empty
/// `Ok(HashMap::new())`, which the caller cannot distinguish from "no MRs had
/// state". Sorting first keeps both which error surfaces and which duplicate
/// key wins tied to batch order rather than completion order.
fn merge_tolerating_partial_failure(
    mut results: Vec<(usize, Result<MrStateMap>)>,
) -> Result<MrStateMap> {
    results.sort_by_key(|(index, _)| *index);

    let mut merged = HashMap::new();
    let mut last_err = None;
    let mut any_batch_succeeded = false;

    for (_, result) in results {
        match result {
            Ok(state) => {
                any_batch_succeeded = true;
                merged.extend(state);
            }
            Err(e) => last_err = Some(e),
        }
    }

    if !any_batch_succeeded {
        if let Some(e) = last_err {
            return Err(e);
        }
    }

    Ok(merged)
}

pub struct GlabBackend {
    tx: Option<UnboundedSender<Event>>,
}

impl GlabBackend {
    pub fn new() -> Self {
        Self { tx: None }
    }

    fn encode_path(project: &str) -> String {
        project.replace('/', "%2F")
    }

    async fn run_glab(&self, args: &[&str], desc: &str) -> Result<String> {
        run_glab_command(
            self.tx.clone(),
            args.iter().map(|a| (*a).to_string()).collect(),
            desc.to_string(),
        )
        .await
    }

    /// Build the `glab` arguments that fetch approval/mergeability state for
    /// one batch of iids (at most `MR_STATE_QUERY_BATCH_SIZE`, to stay under
    /// GitLab's 100-node connection cap — see `list_mr_state`).
    fn mr_state_batch_args(project: &str, iids: &[u64]) -> Vec<String> {
        // Query the exact iids the list returned, so there is no pagination drift.
        let iid_list = iids
            .iter()
            .map(|i| format!("\"{}\"", i))
            .collect::<Vec<_>>()
            .join(", ");

        // Only fields verified present on the reference instance. GraphQL is
        // all-or-nothing: one unknown field blanks both axes.
        let query = format!(
            "query {{ currentUser {{ username }} \
             project(fullPath: \"{}\") {{ \
             mergeRequests(iids: [{}]) {{ nodes {{ \
             iid approved approvalsLeft approvalsRequired \
             userPermissions {{ canApprove }} \
             approvedBy {{ nodes {{ username }} }} \
             reviewers {{ nodes {{ username mergeRequestInteraction {{ reviewState }} }} }} \
             conflicts shouldBeRebased detailedMergeStatus \
             }} }} }} }}",
            project, iid_list
        );

        vec![
            "api".to_string(),
            "graphql".to_string(),
            "-f".to_string(),
            format!("query={}", query),
        ]
    }
}

#[async_trait]
impl Backend for GlabBackend {
    fn kind(&self) -> super::BackendKind {
        super::BackendKind::GitLab
    }

    fn program(&self) -> &'static str {
        "glab"
    }

    fn set_tx(&mut self, tx: UnboundedSender<Event>) {
        self.tx = Some(tx);
    }

    // ── Issues ──

    async fn list_issues(
        &self,
        project: &str,
        show_closed: bool,
        page_size: usize,
        per_request: usize,
    ) -> Result<Vec<Issue>> {
        let pages = page_count(page_size, per_request);
        // Every page is issued at once, so there is no last-page detection to
        // stop early on: a repo with fewer issues than the budget pays for a
        // few requests that come back empty, which merge harmlessly. That is
        // the price of not serialising the round trips, and at the default
        // `api_per_page = 100` it is still a single request.
        let requests: Vec<Vec<String>> = (1..=pages)
            .map(|page| {
                let mut args: Vec<String> = vec![
                    "issue".to_string(),
                    "list".to_string(),
                    "--output".to_string(),
                    "json".to_string(),
                    "-R".to_string(),
                    project.to_string(),
                ];
                if show_closed {
                    args.push("--all".to_string());
                }
                args.push("--page".to_string());
                args.push(page.to_string());
                args.push("--per-page".to_string());
                args.push(per_request.to_string());
                args
            })
            .collect();
        let responses = ordered_or_first_error(
            run_glab_concurrent(self.tx.clone(), requests, "Fetching Issues").await,
        )?;

        let mut all: Vec<Issue> = Vec::new();
        for raw in responses {
            #[derive(Deserialize)]
            struct GiIssue {
                iid: u64,
                title: String,
                state: String,
                #[serde(default)]
                labels: Vec<String>,
                updated_at: String,
                #[serde(default)]
                created_at: Option<String>,
                #[serde(default)]
                closed_at: Option<String>,
                author: GiAuthor,
                milestone: Option<GiMilestone>,
                #[serde(default)]
                assignees: Vec<GiAssignee>,
                #[serde(default)]
                description: Option<String>,
                #[serde(default)]
                due_date: Option<String>,
            }
            #[derive(Deserialize)]
            struct GiAuthor {
                username: String,
            }
            #[derive(Deserialize)]
            struct GiMilestone {
                title: String,
            }
            #[derive(Deserialize)]
            struct GiAssignee {
                username: String,
            }
            let issues: Vec<GiIssue> = serde_json::from_str(&raw).unwrap_or_default();
            all.extend(issues.into_iter().map(|i| {
                Issue {
                    iid: i.iid,
                    title: i.title,
                    state: i.state,
                    labels: i.labels,
                    updated_at: i.updated_at,
                    created_at: i.created_at,
                    closed_at: i.closed_at,
                    author: crate::domain::issues::Author {
                        username: i.author.username,
                    },
                    milestone: i
                        .milestone
                        .map(|m| crate::domain::issues::Milestone { title: m.title }),
                    assignees: i
                        .assignees
                        .into_iter()
                        .map(|a| crate::domain::issues::Assignee {
                            username: a.username,
                        })
                        .collect(),
                    description: i.description,
                    due_date: i.due_date,
                }
            }));
        }
        Ok(all)
    }

    async fn get_issue(&self, project: &str, iid: u64) -> Result<Issue> {
        let raw = self
            .run_glab(
                &[
                    "issue",
                    "view",
                    &iid.to_string(),
                    "--output",
                    "json",
                    "-R",
                    project,
                ],
                "Fetching Issue",
            )
            .await?;
        #[derive(Deserialize)]
        struct GiIssue {
            iid: u64,
            title: String,
            state: String,
            #[serde(default)]
            labels: Vec<String>,
            updated_at: String,
            #[serde(default)]
            created_at: Option<String>,
            #[serde(default)]
            closed_at: Option<String>,
            author: GiAuthor,
            milestone: Option<GiMilestone>,
            #[serde(default)]
            assignees: Vec<GiAssignee>,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            due_date: Option<String>,
        }
        #[derive(Deserialize)]
        struct GiAuthor {
            username: String,
        }
        #[derive(Deserialize)]
        struct GiMilestone {
            title: String,
        }
        #[derive(Deserialize)]
        struct GiAssignee {
            username: String,
        }
        let i: GiIssue = serde_json::from_str(&raw)?;
        Ok(Issue {
            iid: i.iid,
            title: i.title,
            state: i.state,
            labels: i.labels,
            updated_at: i.updated_at,
            created_at: i.created_at,
            closed_at: i.closed_at,
            author: crate::domain::issues::Author {
                username: i.author.username,
            },
            milestone: i
                .milestone
                .map(|m| crate::domain::issues::Milestone { title: m.title }),
            assignees: i
                .assignees
                .into_iter()
                .map(|a| crate::domain::issues::Assignee {
                    username: a.username,
                })
                .collect(),
            description: i.description,
            due_date: i.due_date,
        })
    }

    async fn close_issue(&self, project: &str, iid: u64) -> Result<()> {
        self.run_glab(
            &["issue", "close", &iid.to_string(), "-R", project],
            "CLOSING ISSUE",
        )
        .await?;
        Ok(())
    }

    async fn reopen_issue(&self, project: &str, iid: u64) -> Result<()> {
        self.run_glab(
            &["issue", "reopen", &iid.to_string(), "-R", project],
            "REOPENING ISSUE",
        )
        .await?;
        Ok(())
    }

    async fn delete_issue(&self, project: &str, iid: u64) -> Result<()> {
        self.run_glab(
            &["issue", "delete", &iid.to_string(), "-R", project, "-y"],
            "DELETING ISSUE",
        )
        .await?;
        Ok(())
    }

    async fn create_issue(
        &self,
        _project: &str,
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
            "-y".into(),
            "--title".into(),
            title.into(),
        ];
        if !description.is_empty() {
            args.push("--description".into());
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
        self.run_glab(&args_refs, "CREATING ISSUE").await?;
        Ok(())
    }

    async fn update_issue_title(&self, project: &str, iid: u64, title: &str) -> Result<()> {
        self.run_glab(
            &[
                "issue",
                "update",
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
        self.run_glab(
            &[
                "issue",
                "update",
                &iid.to_string(),
                "-d",
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
            "update".into(),
            iid.to_string(),
            "-R".into(),
            project.into(),
        ];
        for label in add_labels {
            args.push("--label".into());
            args.push(label.clone());
        }
        for label in remove_labels {
            args.push("--unlabel".into());
            args.push(label.clone());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, "UPDATING ISSUE").await?;
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
            "update".into(),
            iid.to_string(),
            "-R".into(),
            project.into(),
        ];
        for a in add {
            args.push("--assignee".into());
            args.push(a.clone());
        }
        for a in remove {
            args.push("--unassign".into());
            args.push(a.clone());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, "UPDATING ISSUE").await?;
        Ok(())
    }

    async fn update_issue_milestone(&self, project: &str, iid: u64, milestone: &str) -> Result<()> {
        let val = if milestone == "None" || milestone.is_empty() {
            "0"
        } else {
            milestone
        };
        self.run_glab(
            &[
                "issue",
                "update",
                &iid.to_string(),
                "--milestone",
                val,
                "-R",
                project,
            ],
            "UPDATING ISSUE",
        )
        .await?;
        Ok(())
    }

    async fn update_issue_due_date(&self, project: &str, iid: u64, due_date: &str) -> Result<()> {
        self.run_glab(
            &[
                "issue",
                "update",
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

    async fn update_issue_weight(&self, project: &str, iid: u64, weight: &str) -> Result<()> {
        self.run_glab(
            &[
                "issue",
                "update",
                &iid.to_string(),
                "--weight",
                weight,
                "-R",
                project,
            ],
            "UPDATING ISSUE",
        )
        .await?;
        Ok(())
    }

    async fn update_issue_confidential(
        &self,
        project: &str,
        iid: u64,
        confidential: bool,
    ) -> Result<()> {
        let flag = if confidential {
            "--confidential"
        } else {
            "--public"
        };
        self.run_glab(
            &["issue", "update", &iid.to_string(), flag, "-R", project],
            "UPDATING ISSUE",
        )
        .await?;
        Ok(())
    }

    // ── Merge Requests ──

    async fn list_mrs(
        &self,
        project: &str,
        show_closed: bool,
        page_size: usize,
        per_request: usize,
    ) -> Result<Vec<MergeRequest>> {
        let pages = page_count(page_size, per_request);
        // Every page is issued at once, so there is no last-page detection to
        // stop early on: a repo with fewer MRs than the budget pays for a few
        // requests that come back empty, which merge harmlessly. That is the
        // price of not serialising the round trips, and at the default
        // `api_per_page = 100` it is still a single request.
        let requests: Vec<Vec<String>> = (1..=pages)
            .map(|page| {
                let mut args: Vec<String> = vec![
                    "mr".to_string(),
                    "list".to_string(),
                    "--output".to_string(),
                    "json".to_string(),
                    "-R".to_string(),
                    project.to_string(),
                ];
                if show_closed {
                    args.push("--all".to_string());
                }
                args.push("--page".to_string());
                args.push(page.to_string());
                args.push("--per-page".to_string());
                args.push(per_request.to_string());
                args
            })
            .collect();
        // Pages are merged in page order, not completion order: the MR table's
        // row order is this list's order.
        let responses = ordered_or_first_error(
            run_glab_concurrent(self.tx.clone(), requests, "Fetching MRs").await,
        )?;

        let mut all: Vec<MergeRequest> = Vec::new();
        for raw in responses {
            #[derive(Deserialize)]
            struct GiMr {
                iid: u64,
                title: String,
                state: String,
                #[serde(default)]
                labels: Vec<String>,
                updated_at: String,
                author: GiAuthor,
                milestone: Option<GiMilestone>,
                #[serde(default)]
                assignees: Vec<GiAssignee>,
                #[serde(default)]
                reviewers: Vec<GiReviewer>,
                target_branch: String,
                #[serde(default)]
                source_branch: String,
                draft: bool,
                #[serde(default)]
                description: Option<String>,
                #[serde(default)]
                head_pipeline: Option<GiPipeline>,
                #[serde(default)]
                blocking_discussions_resolved: Option<bool>,
            }
            #[derive(Deserialize)]
            struct GiAuthor {
                username: String,
            }
            #[derive(Deserialize)]
            struct GiMilestone {
                title: String,
            }
            #[derive(Deserialize)]
            struct GiAssignee {
                username: String,
            }
            #[derive(Deserialize)]
            struct GiReviewer {
                username: String,
            }
            #[derive(Deserialize)]
            struct GiPipeline {
                id: u64,
                status: String,
                #[serde(rename = "ref")]
                pipe_ref: String,
                updated_at: String,
            }
            let mrs: Vec<GiMr> = serde_json::from_str(&raw).unwrap_or_default();
            all.extend(mrs.into_iter().map(|m| {
                MergeRequest {
                    iid: m.iid,
                    title: m.title,
                    state: m.state,
                    labels: m.labels,
                    updated_at: m.updated_at,
                    author: crate::domain::mr::Author {
                        username: m.author.username,
                    },
                    milestone: m
                        .milestone
                        .map(|ms| crate::domain::mr::Milestone { title: ms.title }),
                    assignees: m
                        .assignees
                        .into_iter()
                        .map(|a| crate::domain::mr::Assignee {
                            username: a.username,
                        })
                        .collect(),
                    reviewers: m
                        .reviewers
                        .into_iter()
                        .map(|r| crate::domain::mr::Reviewer {
                            username: r.username,
                        })
                        .collect(),
                    target_branch: m.target_branch,
                    source_branch: m.source_branch,
                    draft: m.draft,
                    description: m.description,
                    head_pipeline: m.head_pipeline.map(|p| Pipeline {
                        id: p.id,
                        status: p.status,
                        r#ref: p.pipe_ref,
                        updated_at: p.updated_at,
                        name: String::new(),
                        display_title: String::new(),
                        event: String::new(),
                        head_sha: String::new(),
                        actor_login: String::new(),
                    }),
                    blocking_discussions_resolved: m.blocking_discussions_resolved,
                    approval: None,
                    mergeability: None,
                    workflow: None,
                }
            }));
        }
        Ok(all)
    }

    async fn get_mr(&self, project: &str, iid: u64) -> Result<MergeRequest> {
        let raw = self
            .run_glab(
                &[
                    "mr",
                    "view",
                    &iid.to_string(),
                    "--output",
                    "json",
                    "-R",
                    project,
                ],
                "Fetching MR",
            )
            .await?;
        #[derive(Deserialize)]
        struct GiMr {
            iid: u64,
            title: String,
            state: String,
            #[serde(default)]
            labels: Vec<String>,
            updated_at: String,
            author: GiAuthor,
            milestone: Option<GiMilestone>,
            #[serde(default)]
            assignees: Vec<GiAssignee>,
            #[serde(default)]
            reviewers: Vec<GiReviewer>,
            target_branch: String,
            #[serde(default)]
            source_branch: String,
            draft: bool,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            head_pipeline: Option<GiPipeline>,
            #[serde(default)]
            blocking_discussions_resolved: Option<bool>,
        }
        #[derive(Deserialize)]
        struct GiAuthor {
            username: String,
        }
        #[derive(Deserialize)]
        struct GiMilestone {
            title: String,
        }
        #[derive(Deserialize)]
        struct GiAssignee {
            username: String,
        }
        #[derive(Deserialize)]
        struct GiReviewer {
            username: String,
        }
        #[derive(Deserialize)]
        struct GiPipeline {
            id: u64,
            status: String,
            #[serde(rename = "ref")]
            pipe_ref: String,
            updated_at: String,
        }
        let m: GiMr = serde_json::from_str(&raw)?;
        Ok(MergeRequest {
            iid: m.iid,
            title: m.title,
            state: m.state,
            labels: m.labels,
            updated_at: m.updated_at,
            author: crate::domain::mr::Author {
                username: m.author.username,
            },
            milestone: m
                .milestone
                .map(|ms| crate::domain::mr::Milestone { title: ms.title }),
            assignees: m
                .assignees
                .into_iter()
                .map(|a| crate::domain::mr::Assignee {
                    username: a.username,
                })
                .collect(),
            reviewers: m
                .reviewers
                .into_iter()
                .map(|r| crate::domain::mr::Reviewer {
                    username: r.username,
                })
                .collect(),
            target_branch: m.target_branch,
            source_branch: m.source_branch,
            draft: m.draft,
            description: m.description,
            head_pipeline: m.head_pipeline.map(|p| Pipeline {
                id: p.id,
                status: p.status,
                r#ref: p.pipe_ref,
                updated_at: p.updated_at,
                name: String::new(),
                display_title: String::new(),
                event: String::new(),
                head_sha: String::new(),
                actor_login: String::new(),
            }),
            blocking_discussions_resolved: m.blocking_discussions_resolved,
            approval: None,
            mergeability: None,
            workflow: None,
        })
    }

    async fn get_mr_diff(&self, project: &str, iid: u64) -> Result<String> {
        self.run_glab(
            &["mr", "diff", &iid.to_string(), "-R", project],
            "Fetching MR Diff",
        )
        .await
    }

    async fn list_mr_notes(
        &self,
        project: &str,
        mr_iid: u64,
        _page_size: usize,
    ) -> Result<Vec<DiscussionNote>> {
        let raw = self
            .run_glab(
                &[
                    "mr",
                    "note",
                    "list",
                    &mr_iid.to_string(),
                    "--output",
                    "json",
                    "-R",
                    project,
                ],
                "Fetching MR Notes",
            )
            .await?;
        #[derive(Deserialize)]
        struct GiNote {
            id: u64,
            body: String,
            author: GiAuthor,
            created_at: String,
            system: bool,
            #[serde(default)]
            position: Option<GiPosition>,
            #[serde(default)]
            discussion_id: Option<String>,
            #[serde(default)]
            resolved: Option<bool>,
            #[serde(default)]
            resolvable: Option<bool>,
        }
        #[derive(Deserialize)]
        struct GiAuthor {
            username: String,
        }
        #[derive(Deserialize)]
        struct GiPosition {
            #[serde(default)]
            new_path: Option<String>,
            #[serde(default)]
            old_path: Option<String>,
            #[serde(default)]
            new_line: Option<u64>,
            #[serde(default)]
            old_line: Option<u64>,
            #[serde(default)]
            start_line: Option<u64>,
            #[serde(default)]
            line_range: Option<serde_json::Value>,
        }
        let notes: Vec<GiNote> = serde_json::from_str(&raw)?;
        Ok(notes
            .into_iter()
            .map(|n| DiscussionNote {
                id: n.id,
                body: n.body,
                author: crate::domain::mr::Author {
                    username: n.author.username,
                },
                created_at: n.created_at,
                system: n.system,
                position: n.position.map(|p| crate::domain::mr::NotePosition {
                    new_path: p.new_path,
                    old_path: p.old_path,
                    new_line: p.new_line,
                    old_line: p.old_line,
                    start_line: p.start_line,
                    line_range: p.line_range,
                }),
                discussion_id: n.discussion_id,
                resolved: n.resolved,
                resolvable: n.resolvable,
            })
            .collect())
    }

    async fn close_mr(&self, project: &str, iid: u64) -> Result<()> {
        self.run_glab(
            &["mr", "close", &iid.to_string(), "-R", project],
            "CLOSING MR",
        )
        .await?;
        Ok(())
    }

    async fn reopen_mr(&self, project: &str, iid: u64) -> Result<()> {
        self.run_glab(
            &["mr", "reopen", &iid.to_string(), "-R", project],
            "REOPENING MR",
        )
        .await?;
        Ok(())
    }

    async fn delete_mr(&self, project: &str, iid: u64) -> Result<()> {
        self.run_glab(
            &["mr", "delete", &iid.to_string(), "-R", project, "-y"],
            "DELETING MR",
        )
        .await?;
        Ok(())
    }

    async fn approve_mr(&self, project: &str, iid: u64) -> Result<()> {
        self.run_glab(
            &["mr", "approve", &iid.to_string(), "-R", project],
            "APPROVING MR",
        )
        .await?;
        Ok(())
    }

    async fn revoke_mr(&self, project: &str, iid: u64) -> Result<()> {
        self.run_glab(
            &["mr", "revoke", &iid.to_string(), "-R", project],
            "REVOKING MR APPROVAL",
        )
        .await?;
        Ok(())
    }

    async fn rebase_mr(&self, project: &str, iid: u64) -> Result<()> {
        self.run_glab(
            &["mr", "rebase", &iid.to_string(), "-R", project],
            "REBASING MR",
        )
        .await?;
        Ok(())
    }

    async fn list_mr_state(&self, project: &str, iids: &[u64]) -> Result<MrStateMap> {
        if iids.is_empty() {
            return Ok(HashMap::new());
        }
        validate_project_path(project)?;

        let requests: Vec<Vec<String>> = chunk_iids(iids, MR_STATE_QUERY_BATCH_SIZE)
            .into_iter()
            .map(|batch| Self::mr_state_batch_args(project, batch))
            .collect();

        let parsed: Vec<(usize, Result<MrStateMap>)> =
            run_glab_concurrent(self.tx.clone(), requests, "FETCHING MR STATE")
                .await
                .into_iter()
                .map(|(index, raw)| (index, raw.and_then(|r| parse_mr_state_response(&r))))
                .collect();

        // Unlike the paged list fetches, a single failed batch must not fail
        // the whole call — see `merge_tolerating_partial_failure`.
        merge_tolerating_partial_failure(parsed)
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
            "mr".into(),
            "merge".into(),
            iid.to_string(),
            "-R".into(),
            project.into(),
        ];
        if squash {
            args.push("--squash".into());
        }
        if delete_branch {
            args.push("--remove-source-branch".into());
        }
        if let Some(s) = strategy {
            args.push(format!("--{}", s));
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, "MERGING MR").await?;
        Ok(())
    }

    async fn toggle_mr_draft(&self, _project: &str, iid: u64, is_draft: bool) -> Result<()> {
        if is_draft {
            self.run_glab(
                &["mr", "update", &iid.to_string(), "--draft"],
                "DRAFTING MR",
            )
            .await?;
        } else {
            self.run_glab(
                &["mr", "update", &iid.to_string(), "--ready"],
                "MARKING MR READY",
            )
            .await?;
        }
        Ok(())
    }

    async fn create_mr(
        &self,
        _project: &str,
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
            "mr".into(),
            "create".into(),
            "-y".into(),
            "--title".into(),
            title.into(),
        ];
        if !source_branch.is_empty() {
            args.push("--source-branch".into());
            args.push(source_branch.into());
        }
        if !target_branch.is_empty() {
            args.push("--target-branch".into());
            args.push(target_branch.into());
        }
        if !description.is_empty() {
            args.push("-d".into());
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
        if !reviewers.is_empty() {
            args.push("--reviewer".into());
            args.push(strip_ats(reviewers).into());
        }
        if !milestone.is_empty() {
            args.push("--milestone".into());
            args.push(milestone.into());
        }
        if let Some(iid) = issue_iid {
            args.push("--related-issue".into());
            args.push(iid.to_string());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, "CREATING MR").await?;
        Ok(())
    }

    async fn add_mr_comment(
        &self,
        _project: &str,
        iid: u64,
        body: &str,
        file_path: Option<&str>,
        line: Option<u64>,
        _old_line: Option<u64>,
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "mr".into(),
            "note".into(),
            "create".into(),
            iid.to_string(),
            "-m".into(),
            body.into(),
        ];
        if let Some(path) = file_path {
            args.push("--file-path".into());
            args.push(path.into());
        }
        if let Some(l) = line {
            args.push("--line".into());
            args.push(l.to_string());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, "ADDING MR COMMENT").await?;
        Ok(())
    }

    async fn update_mr_title(&self, project: &str, iid: u64, title: &str) -> Result<()> {
        self.run_glab(
            &[
                "mr",
                "update",
                &iid.to_string(),
                "--title",
                title,
                "-R",
                project,
            ],
            "UPDATING MR",
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
        self.run_glab(
            &[
                "mr",
                "update",
                &iid.to_string(),
                "-d",
                description,
                "-R",
                project,
            ],
            "UPDATING MR",
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
            "mr".into(),
            "update".into(),
            iid.to_string(),
            "-R".into(),
            project.into(),
        ];
        for label in add_labels {
            args.push("--label".into());
            args.push(label.clone());
        }
        for label in remove_labels {
            args.push("--unlabel".into());
            args.push(label.clone());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, "UPDATING MR").await?;
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
            "mr".into(),
            "update".into(),
            iid.to_string(),
            "-R".into(),
            project.into(),
        ];
        for a in add {
            args.push("--assignee".into());
            args.push(a.clone());
        }
        for a in remove {
            args.push("--unassign".into());
            args.push(a.clone());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, "UPDATING MR").await?;
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
            "mr".into(),
            "update".into(),
            iid.to_string(),
            "-R".into(),
            project.into(),
        ];
        for r in add {
            args.push("--reviewer".into());
            args.push(r.clone());
        }
        for r in remove {
            args.push("--unreviewer".into());
            args.push(r.clone());
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, "UPDATING MR").await?;
        Ok(())
    }

    async fn update_mr_milestone(&self, project: &str, iid: u64, milestone: &str) -> Result<()> {
        let val = if milestone == "None" || milestone.is_empty() {
            "0"
        } else {
            milestone
        };
        self.run_glab(
            &[
                "mr",
                "update",
                &iid.to_string(),
                "--milestone",
                val,
                "-R",
                project,
            ],
            "UPDATING MR",
        )
        .await?;
        Ok(())
    }

    async fn update_mr_target_branch(&self, project: &str, iid: u64, branch: &str) -> Result<()> {
        self.run_glab(
            &[
                "mr",
                "update",
                &iid.to_string(),
                "--target-branch",
                branch,
                "-R",
                project,
            ],
            "UPDATING MR",
        )
        .await?;
        Ok(())
    }

    // ── Pipelines ──

    async fn list_pipelines(
        &self,
        project: &str,
        page_size: usize,
        per_request: usize,
    ) -> Result<Vec<Pipeline>> {
        let pages = page_count(page_size, per_request);
        // Every page is issued at once, so there is no last-page detection to
        // stop early on: a repo with fewer pipelines than the budget pays for
        // a few requests that come back empty, which merge harmlessly. That is
        // the price of not serialising the round trips, and at the default
        // `api_per_page = 100` it is still a single request.
        let requests: Vec<Vec<String>> = (1..=pages)
            .map(|page| {
                vec![
                    "ci".to_string(),
                    "list".to_string(),
                    "--output".to_string(),
                    "json".to_string(),
                    "-R".to_string(),
                    project.to_string(),
                    "--page".to_string(),
                    page.to_string(),
                    "--per-page".to_string(),
                    per_request.to_string(),
                ]
            })
            .collect();
        let responses = ordered_or_first_error(
            run_glab_concurrent(self.tx.clone(), requests, "Fetching Pipelines").await,
        )?;

        let mut all: Vec<Pipeline> = Vec::new();
        for raw in responses {
            #[derive(Deserialize)]
            struct GiPipe {
                id: u64,
                status: String,
                #[serde(rename = "ref")]
                pipe_ref: String,
                updated_at: String,
            }
            let pipes: Vec<GiPipe> = serde_json::from_str(&raw).unwrap_or_default();
            all.extend(pipes.into_iter().map(|p| Pipeline {
                id: p.id,
                status: p.status,
                r#ref: p.pipe_ref,
                updated_at: p.updated_at,
                name: String::new(),
                display_title: String::new(),
                event: String::new(),
                head_sha: String::new(),
                actor_login: String::new(),
            }));
        }
        Ok(all)
    }

    async fn list_pipeline_jobs(
        &self,
        project: &str,
        pipeline_id: u64,
        page_size: usize,
    ) -> Result<Vec<Job>> {
        let encoded = Self::encode_path(project);
        let endpoint = format!(
            "/projects/{}/pipelines/{}/jobs?per_page={}",
            encoded, pipeline_id, page_size
        );
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Jobs")
            .await?;
        #[derive(Deserialize)]
        struct GiJob {
            id: u64,
            status: String,
            stage: String,
            name: String,
        }
        let jobs: Vec<GiJob> = serde_json::from_str(&raw)?;
        let all_jobs: Vec<Job> = jobs
            .into_iter()
            .map(|j| Job {
                id: j.id,
                status: j.status,
                stage: j.stage,
                name: j.name,
                matrix: None,
            })
            .collect();
        Ok(crate::domain::pipelines::process_pipeline_jobs(all_jobs))
    }

    async fn get_job_trace(&self, project: &str, job_id: u64) -> Result<String> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/jobs/{}/trace", encoded, job_id);
        self.raw_api(&endpoint, "GET", None, "Fetching Job Log")
            .await
    }

    async fn retry_pipeline(&self, project: &str, pipeline_id: u64) -> Result<()> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/pipelines/{}/retry", encoded, pipeline_id);
        self.raw_api(&endpoint, "POST", None, "Retrying Pipeline")
            .await?;
        Ok(())
    }

    async fn cancel_pipeline(&self, project: &str, pipeline_id: u64) -> Result<()> {
        self.run_glab(
            &[
                "ci",
                "cancel",
                "pipeline",
                &pipeline_id.to_string(),
                "-R",
                project,
            ],
            "Cancelling Pipeline",
        )
        .await?;
        Ok(())
    }

    async fn retry_job(&self, project: &str, job_id: u64) -> Result<()> {
        self.run_glab(
            &["ci", "retry", &job_id.to_string(), "-R", project],
            "Retrying Job",
        )
        .await?;
        Ok(())
    }

    async fn cancel_job(&self, project: &str, job_id: u64) -> Result<()> {
        self.run_glab(
            &["ci", "cancel", "job", &job_id.to_string(), "-R", project],
            "Cancelling Job",
        )
        .await?;
        Ok(())
    }

    async fn start_job(&self, project: &str, job_id: u64) -> Result<()> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/jobs/{}/play", encoded, job_id);
        self.raw_api(&endpoint, "POST", None, "Starting Job")
            .await?;
        Ok(())
    }

    async fn run_pipeline(
        &self,
        _project: &str,
        branch: &str,
        mr: bool,
        variables: &[(String, String)],
        inputs: &[(String, String)],
        _workflow_file: &str,
    ) -> Result<()> {
        let mut args: Vec<String> = vec!["ci".into(), "run".into()];
        if !branch.is_empty() {
            args.push("--branch".into());
            args.push(branch.into());
        }
        if mr && variables.is_empty() && inputs.is_empty() {
            args.push("--mr".into());
        }
        for (k, v) in variables {
            args.push("--variables".into());
            args.push(format!("{}:{}", k, v));
        }
        for (k, v) in inputs {
            args.push("--input".into());
            args.push(format!("{}:{}", k, v));
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, "RUNNING PIPELINE").await?;
        Ok(())
    }

    async fn download_artifact(
        &self,
        _project: &str,
        ref_name: &str,
        job_name: &str,
    ) -> Result<()> {
        self.run_glab(
            &["job", "artifact", ref_name, job_name],
            "DOWNLOADING ARTIFACT",
        )
        .await?;
        Ok(())
    }

    // ── Runners ──

    async fn list_runners(&self, project: &str, page_size: usize) -> Result<Vec<Runner>> {
        let raw = self
            .run_glab(
                &[
                    "runner",
                    "list",
                    "--output",
                    "json",
                    "-R",
                    project,
                    "--per-page",
                    &page_size.to_string(),
                ],
                "Fetching Runners",
            )
            .await?;
        #[derive(Deserialize)]
        struct GiRunner {
            id: u64,
            description: Option<String>,
            status: String,
            #[serde(default)]
            active: bool,
        }
        let runners: Vec<GiRunner> = serde_json::from_str(&raw)?;
        Ok(runners
            .into_iter()
            .map(|r| Runner {
                id: r.id,
                description: r.description,
                status: r.status,
                active: r.active,
            })
            .collect())
    }

    async fn pause_runner(&self, _project: &str, runner_id: u64) -> Result<()> {
        let endpoint = format!("runners/{}", runner_id);
        let body = r#"{"paused":true}"#;
        self.raw_api(&endpoint, "PUT", Some(body), "PAUSING RUNNER")
            .await?;
        Ok(())
    }

    async fn resume_runner(&self, _project: &str, runner_id: u64) -> Result<()> {
        let endpoint = format!("runners/{}", runner_id);
        let body = r#"{"paused":false}"#;
        self.raw_api(&endpoint, "PUT", Some(body), "RESUMING RUNNER")
            .await?;
        Ok(())
    }

    async fn update_runner_description(
        &self,
        _project: &str,
        runner_id: u64,
        description: &str,
    ) -> Result<()> {
        let endpoint = format!("runners/{}", runner_id);
        let body = serde_json::json!({ "description": description }).to_string();
        self.raw_api(&endpoint, "PUT", Some(&body), "UPDATING RUNNER DESCRIPTION")
            .await?;
        Ok(())
    }

    // ── Releases ──

    async fn list_releases(&self, project: &str, page_size: usize) -> Result<Vec<Release>> {
        let raw = self
            .run_glab(
                &[
                    "release",
                    "list",
                    "--output",
                    "json",
                    "-R",
                    project,
                    "--per-page",
                    &page_size.to_string(),
                ],
                "Fetching Releases",
            )
            .await?;
        #[derive(Deserialize)]
        struct GiRel {
            #[serde(default)]
            name: Option<String>,
            tag_name: String,
            released_at: String,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            author_name: Option<String>,
            #[serde(default)]
            commit: Option<GiRelCommit>,
            #[serde(default)]
            assets_link: Option<String>,
        }
        #[derive(Deserialize)]
        struct GiRelCommit {
            #[serde(default)]
            id: Option<String>,
            #[serde(default)]
            title: Option<String>,
        }
        let rels: Vec<GiRel> = serde_json::from_str(&raw)?;
        Ok(rels
            .into_iter()
            .map(|r| {
                let name = r.name.unwrap_or_else(|| r.tag_name.clone());
                let (commit_id, commit_title) = match r.commit {
                    Some(c) => (c.id, c.title),
                    None => (None, None),
                };
                Release {
                    name,
                    tag_name: r.tag_name,
                    released_at: r.released_at,
                    description: r.description,
                    author_name: r.author_name,
                    commit_id,
                    commit_title,
                    assets_link: r.assets_link,
                }
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
        self.run_glab(
            &[
                "release",
                "create",
                tag,
                "-R",
                project,
                "-n",
                name,
                "-N",
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
        self.run_glab(
            &[
                "release",
                "update",
                tag_name,
                "-R",
                project,
                "-n",
                name,
                "-N",
                description,
            ],
            "Updating Release",
        )
        .await?;
        Ok(())
    }

    async fn delete_release(&self, project: &str, tag_name: &str) -> Result<()> {
        self.run_glab(
            &["release", "delete", tag_name, "-R", project, "-y"],
            "Deleting Release",
        )
        .await?;
        Ok(())
    }

    // ── Milestones ──

    async fn list_milestones(&self, project: &str, page_size: usize) -> Result<Vec<Milestone>> {
        let raw = self
            .run_glab(
                &[
                    "milestone",
                    "list",
                    "--output",
                    "json",
                    "-R",
                    project,
                    "--per-page",
                    &page_size.to_string(),
                ],
                "Fetching Milestones",
            )
            .await?;
        #[derive(Deserialize, Default)]
        struct GiMs {
            #[serde(default)]
            id: u64,
            #[serde(default)]
            iid: u64,
            #[serde(default)]
            title: String,
            pub description: Option<String>,
            #[serde(default)]
            state: String,
            start_date: Option<String>,
            due_date: Option<String>,
            #[serde(default)]
            created_at: String,
        }
        let milestones: Vec<GiMs> = serde_json::from_str(&raw)?;
        Ok(milestones
            .into_iter()
            .map(|m| Milestone {
                id: m.id,
                iid: m.iid,
                title: m.title,
                description: m.description,
                state: m.state,
                start_date: m.start_date,
                due_date: m.due_date,
                created_at: m.created_at,
            })
            .collect())
    }

    async fn list_milestone_issues(
        &self,
        project: &str,
        milestone_iid: u64,
        page_size: usize,
    ) -> Result<Vec<Issue>> {
        let raw = self
            .run_glab(
                &[
                    "issue",
                    "list",
                    "--milestone",
                    &milestone_iid.to_string(),
                    "--all",
                    "--output",
                    "json",
                    "-R",
                    project,
                    "--per-page",
                    &page_size.to_string(),
                ],
                "Fetching Milestone Issues",
            )
            .await?;
        #[derive(Deserialize)]
        struct GiIssue {
            iid: u64,
            title: String,
            state: String,
            #[serde(default)]
            labels: Vec<String>,
            updated_at: String,
            #[serde(default)]
            created_at: Option<String>,
            #[serde(default)]
            closed_at: Option<String>,
            author: GiAuthor,
            milestone: Option<GiMilestone>,
            #[serde(default)]
            assignees: Vec<GiAssignee>,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            due_date: Option<String>,
        }
        #[derive(Deserialize)]
        struct GiAuthor {
            username: String,
        }
        #[derive(Deserialize)]
        struct GiMilestone {
            title: String,
        }
        #[derive(Deserialize)]
        struct GiAssignee {
            username: String,
        }
        let issues: Vec<GiIssue> = serde_json::from_str(&raw)?;
        Ok(issues
            .into_iter()
            .map(|i| Issue {
                iid: i.iid,
                title: i.title,
                state: i.state,
                labels: i.labels,
                updated_at: i.updated_at,
                created_at: i.created_at,
                closed_at: i.closed_at,
                author: crate::domain::issues::Author {
                    username: i.author.username,
                },
                milestone: i
                    .milestone
                    .map(|m| crate::domain::issues::Milestone { title: m.title }),
                assignees: i
                    .assignees
                    .into_iter()
                    .map(|a| crate::domain::issues::Assignee {
                        username: a.username,
                    })
                    .collect(),
                description: i.description,
                due_date: i.due_date,
            })
            .collect())
    }

    async fn create_milestone(
        &self,
        project: &str,
        title: &str,
        description: &str,
        start_date: Option<&str>,
        due_date: Option<&str>,
    ) -> Result<()> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("projects/{}/milestones", encoded);
        let mut body_val = serde_json::json!({
            "title": title,
        });
        if !description.is_empty() {
            body_val["description"] = serde_json::Value::String(description.to_string());
        }
        if let Some(sd) = start_date {
            if !sd.is_empty() {
                body_val["start_date"] = serde_json::Value::String(sd.to_string());
            }
        }
        if let Some(dd) = due_date {
            if !dd.is_empty() {
                body_val["due_date"] = serde_json::Value::String(dd.to_string());
            }
        }
        let body = body_val.to_string();
        self.raw_api(&endpoint, "POST", Some(&body), "CREATING MILESTONE")
            .await?;
        Ok(())
    }

    async fn update_milestone_state(
        &self,
        project: &str,
        milestone_iid: u64,
        close: bool,
    ) -> Result<()> {
        let action = if close { "close" } else { "reopen" };
        let desc = if close {
            "CLOSING MILESTONE"
        } else {
            "REOPENING MILESTONE"
        };
        self.run_glab(
            &[
                "milestone",
                action,
                &milestone_iid.to_string(),
                "-R",
                project,
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
        start_date: Option<&str>,
        due_date: Option<&str>,
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "milestone".into(),
            "update".into(),
            milestone_iid.to_string(),
            "-R".into(),
            project.to_string(),
            "--title".into(),
            title.into(),
            "--description".into(),
            description.into(),
        ];
        if let Some(start) = start_date {
            if !start.is_empty() {
                args.push("--start-date".into());
                args.push(start.into());
            }
        }
        if let Some(due) = due_date {
            if !due.is_empty() {
                args.push("--due-date".into());
                args.push(due.into());
            }
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, "Updating Milestone").await?;
        Ok(())
    }

    async fn delete_milestone(&self, project: &str, milestone_iid: u64) -> Result<()> {
        self.run_glab(
            &[
                "milestone",
                "delete",
                &milestone_iid.to_string(),
                "-R",
                project,
                "-y",
            ],
            "Deleting Milestone",
        )
        .await?;
        Ok(())
    }

    // ── Notifications ──

    async fn list_notifications(&self, show_read: bool) -> Result<Vec<Notification>> {
        // glab todo list does active todos; for "done" we use glab api
        let raw = self
            .run_glab(&["todo", "list", "--output=json"], "Fetching Todos")
            .await?;
        #[derive(Deserialize)]
        struct GiTodo {
            id: serde_json::Value,
            project: GiTodoProject,
            target: GiTodoTarget,
            target_type: String,
            state: String,
            updated_at: String,
        }
        #[derive(Deserialize)]
        struct GiTodoProject {
            path_with_namespace: String,
        }
        #[derive(Deserialize)]
        struct GiTodoTarget {
            title: String,
            iid: u64,
        }
        let todos: Vec<GiTodo> = serde_json::from_str(&raw)?;
        let mut list: Vec<Notification> = todos
            .into_iter()
            .map(|item| {
                let id = match item.id {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s,
                    _ => String::new(),
                };
                Notification {
                    id,
                    project_path: item.project.path_with_namespace,
                    title: item.target.title,
                    target_type: item.target_type,
                    target_iid: item.target.iid,
                    state: item.state,
                    updated_at: item.updated_at,
                }
            })
            .collect();
        if show_read {
            let endpoint = "todos?state=done";
            let raw = self
                .raw_api(endpoint, "GET", None, "Fetching Done Todos")
                .await?;
            let done_todos: Vec<GiTodo> = serde_json::from_str(&raw).unwrap_or_default();
            list.extend(done_todos.into_iter().map(|item| {
                let id = match item.id {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s,
                    _ => String::new(),
                };
                Notification {
                    id,
                    project_path: item.project.path_with_namespace,
                    title: item.target.title,
                    target_type: item.target_type,
                    target_iid: item.target.iid,
                    state: item.state,
                    updated_at: item.updated_at,
                }
            }));
        }
        Ok(list)
    }

    async fn mark_notification_as_read(&self, id: &str) -> Result<()> {
        self.run_glab(&["todo", "done", id], "Marking Todo Done")
            .await?;
        Ok(())
    }

    // ── Branches ──

    async fn list_branches(&self, project: &str, page_size: usize) -> Result<Vec<Branch>> {
        let encoded = Self::encode_path(project);
        let endpoint = format!(
            "/projects/{}/repository/branches?per_page={}",
            encoded, page_size
        );
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Branches")
            .await?;
        #[derive(Deserialize)]
        struct GiBr {
            name: String,
            #[serde(default)]
            default: Option<bool>,
            #[serde(default)]
            protected: Option<bool>,
            #[serde(default)]
            web_url: Option<String>,
            #[serde(default)]
            can_push: Option<bool>,
            commit: Option<GiBrCommit>,
        }
        #[derive(Deserialize)]
        struct GiBrCommit {
            id: String,
        }
        let gl_branches: Vec<GiBr> = serde_json::from_str(&raw)?;
        Ok(gl_branches
            .into_iter()
            .map(|b| Branch {
                name: b.name,
                default: b.default.unwrap_or(false),
                protected: b.protected.unwrap_or(false),
                web_url: b.web_url.unwrap_or_default(),
                can_push: b.can_push.unwrap_or(false),
                commit_sha: b.commit.as_ref().map(|c| c.id.clone()).unwrap_or_default(),
            })
            .collect())
    }

    async fn create_branch(
        &self,
        project: &str,
        branch_name: &str,
        ref_branch: &str,
    ) -> Result<()> {
        let encoded = Self::encode_path(project);
        let endpoint = format!(
            "/projects/{}/repository/branches?branch={}&ref={}",
            encoded, branch_name, ref_branch
        );
        self.raw_api(&endpoint, "POST", None, "Creating Branch")
            .await?;
        Ok(())
    }

    async fn delete_branch(&self, project: &str, branch_name: &str) -> Result<()> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/repository/branches/{}", encoded, branch_name);
        self.raw_api(&endpoint, "DELETE", None, "Deleting Branch")
            .await?;
        Ok(())
    }

    // ── Environments / Deployments ──

    async fn list_environments(&self, project: &str, page_size: usize) -> Result<Vec<Environment>> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/environments?per_page={}", encoded, page_size);
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Environments")
            .await?;
        #[derive(Deserialize)]
        struct GiEnv {
            id: u64,
            name: String,
            state: String,
            #[serde(default)]
            external_url: Option<String>,
            #[serde(default)]
            last_deployment: Option<GiDeploy>,
        }
        #[derive(Deserialize)]
        struct GiDeploy {
            id: u64,
            iid: u64,
            #[serde(rename = "ref")]
            ref_name: String,
            tag: bool,
            sha: String,
            status: String,
            created_at: String,
            updated_at: String,
            #[serde(default)]
            environment: Option<crate::domain::deployments::EnvironmentInfo>,
            #[serde(default)]
            deployable: Option<crate::domain::deployments::Deployable>,
            #[serde(default)]
            description: String,
            #[serde(default)]
            user: Option<crate::domain::deployments::DeploymentUser>,
        }
        let envs: Vec<GiEnv> = serde_json::from_str(&raw)?;
        Ok(envs
            .into_iter()
            .map(|e| Environment {
                id: e.id,
                name: e.name,
                state: e.state,
                external_url: e.external_url,
                last_deployment: e.last_deployment.map(|d| Deployment {
                    id: d.id,
                    iid: d.iid,
                    ref_name: d.ref_name,
                    tag: d.tag,
                    sha: d.sha,
                    status: d.status,
                    created_at: d.created_at,
                    updated_at: d.updated_at,
                    environment: d.environment,
                    deployable: d.deployable,
                    description: d.description,
                    user: d.user,
                }),
            })
            .collect())
    }

    async fn list_deployments(
        &self,
        project: &str,
        page_size: usize,
        environment: Option<&str>,
    ) -> Result<Vec<Deployment>> {
        let encoded = Self::encode_path(project);
        let mut endpoint = format!("/projects/{}/deployments?per_page={}", encoded, page_size);
        if let Some(env) = environment {
            endpoint.push_str(&format!("&environment={}", env));
        }
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Deployments")
            .await?;
        #[derive(Deserialize)]
        struct GiDeploy {
            id: u64,
            iid: u64,
            #[serde(rename = "ref")]
            ref_name: String,
            tag: bool,
            sha: String,
            status: String,
            created_at: String,
            updated_at: String,
            #[serde(default)]
            environment: Option<crate::domain::deployments::EnvironmentInfo>,
            #[serde(default)]
            deployable: Option<crate::domain::deployments::Deployable>,
            #[serde(default)]
            description: String,
            #[serde(default)]
            user: Option<crate::domain::deployments::DeploymentUser>,
        }
        let deploys: Vec<GiDeploy> = serde_json::from_str(&raw)?;
        Ok(deploys
            .into_iter()
            .map(|d| Deployment {
                id: d.id,
                iid: d.iid,
                ref_name: d.ref_name,
                tag: d.tag,
                sha: d.sha,
                status: d.status,
                created_at: d.created_at,
                updated_at: d.updated_at,
                environment: d.environment,
                deployable: d.deployable,
                description: d.description,
                user: d.user,
            })
            .collect())
    }

    // ── Labels / Members / Misc ──

    async fn fetch_labels(&self, project: &str, per_request: usize) -> Result<Vec<String>> {
        let raw = self
            .run_glab(
                &[
                    "label",
                    "list",
                    "--output",
                    "json",
                    "-R",
                    project,
                    "--per-page",
                    &per_request.to_string(),
                ],
                "Fetching Labels",
            )
            .await?;
        #[derive(Deserialize)]
        struct GiLabel {
            name: String,
        }
        let labels: Vec<GiLabel> = serde_json::from_str(&raw)?;
        Ok(labels.into_iter().map(|l| l.name).collect())
    }

    async fn fetch_members(&self, project: &str) -> Result<Vec<String>> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/members/all?per_page=100", encoded);
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Members")
            .await?;
        #[derive(Deserialize)]
        struct GiMember {
            username: String,
        }
        let members: Vec<GiMember> = serde_json::from_str(&raw)?;
        Ok(members
            .into_iter()
            .map(|m| format!("@{}", m.username))
            .collect())
    }

    async fn open_in_browser(&self, _project: &str, entity: &str, id: &str) -> Result<()> {
        self.run_glab(&[entity, "view", id, "-w"], "OPENING IN BROWSER")
            .await?;
        Ok(())
    }

    async fn open_pipeline_in_browser(&self, _project: &str, id: &str) -> Result<()> {
        self.run_glab(&["ci", "view", id, "-w"], "OPENING IN BROWSER")
            .await?;
        Ok(())
    }

    async fn open_job_in_browser(&self, _project: &str, id: &str) -> Result<()> {
        self.run_glab(&["job", "view", id, "-w"], "OPENING IN BROWSER")
            .await?;
        Ok(())
    }

    async fn open_milestone_in_browser(&self, project: &str, id: &str) -> Result<()> {
        self.run_glab(
            &["milestone", "view", id, "-w", "-R", project],
            "OPENING IN BROWSER",
        )
        .await?;
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
        let cmd_str = format!("glab {}", cmd_args.join(" "));
        let label = desc.to_uppercase();

        let mut cmd = Command::new("glab");
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
                let mut child = cmd.spawn().context("Failed to spawn glab api command")?;
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
                    anyhow::bail!("glab api failed: {}", err_msg)
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── iid batching (GraphQL 100-node connection cap) ──

    #[test]
    fn chunk_iids_splits_250_into_100_100_50() {
        let iids: Vec<u64> = (1..=250).collect();
        let batches = chunk_iids(&iids, 100);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), 100);
        assert_eq!(batches[1].len(), 100);
        assert_eq!(batches[2].len(), 50);
        // Order and identity are preserved across the split.
        assert_eq!(batches[0][0], 1);
        assert_eq!(batches[2][49], 250);
    }

    #[test]
    fn chunk_iids_keeps_exactly_100_as_a_single_batch() {
        let iids: Vec<u64> = (1..=100).collect();
        let batches = chunk_iids(&iids, 100);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 100);
    }

    // ── assembling concurrently-fetched pages (fail-fast) ──

    #[test]
    fn ordered_or_first_error_assembles_out_of_order_completions_by_index() {
        // JoinSet yields completions in arbitrary order, but the MR table's
        // row order is page order, so assembly must re-sort.
        let results: Vec<(usize, Result<&str>)> =
            vec![(2, Ok("page 3")), (0, Ok("page 1")), (1, Ok("page 2"))];

        let ordered = ordered_or_first_error(results).expect("every page succeeded");

        assert_eq!(ordered, vec!["page 1", "page 2", "page 3"]);
    }

    #[test]
    fn ordered_or_first_error_fails_when_one_page_fails() {
        // Rendering a subset of MRs as though it were complete is worse than
        // an error, so a single failure fails the whole fetch.
        let results: Vec<(usize, Result<&str>)> = vec![
            (0, Ok("page 1")),
            (1, Err(anyhow::anyhow!("page 2 was truncated"))),
            (2, Ok("page 3")),
        ];

        let err = ordered_or_first_error(results).expect_err("a failed page fails the fetch");

        assert!(err.to_string().contains("page 2 was truncated"));
    }

    #[test]
    fn ordered_or_first_error_reports_the_lowest_indexed_failure() {
        // Which error surfaces must not depend on which request lost the race.
        let results: Vec<(usize, Result<&str>)> = vec![
            (2, Err(anyhow::anyhow!("page 3 was truncated"))),
            (1, Err(anyhow::anyhow!("page 2 was truncated"))),
            (0, Ok("page 1")),
        ];

        let err = ordered_or_first_error(results).expect_err("a failed page fails the fetch");

        assert!(err.to_string().contains("page 2 was truncated"));
    }

    #[test]
    fn ordered_or_first_error_merges_empty_and_short_pages_in_index_order() {
        // Requesting all pages unconditionally means trailing pages can come
        // back empty; they must merge without erroring or duplicating.
        let results: Vec<(usize, Result<Vec<u64>>)> =
            vec![(2, Ok(vec![])), (1, Ok(vec![4])), (0, Ok(vec![1, 2, 3]))];

        let merged: Vec<u64> = ordered_or_first_error(results)
            .expect("empty pages are successful responses")
            .into_iter()
            .flatten()
            .collect();

        assert_eq!(merged, vec![1, 2, 3, 4]);
    }

    // ── merging concurrently-fetched MR-state batches (tolerate partial) ──

    fn mr_state_for(iid: u64) -> MrStateMap {
        let mut state: MrStateMap = HashMap::new();
        state.insert(iid, (None, None));
        state
    }

    #[test]
    fn merge_tolerating_partial_failure_keeps_the_batches_that_succeeded() {
        // src/fetch.rs degrades a missing entry to "—" per row, so one bad
        // batch must not discard the state the others returned.
        let results: Vec<(usize, Result<MrStateMap>)> = vec![
            (1, Err(anyhow::anyhow!("batch 2 was truncated"))),
            (0, Ok(mr_state_for(7))),
            (2, Ok(mr_state_for(9))),
        ];

        let merged =
            merge_tolerating_partial_failure(results).expect("successful batches still count");

        assert_eq!(merged.len(), 2);
        assert!(merged.contains_key(&7));
        assert!(merged.contains_key(&9));
    }

    #[test]
    fn merge_tolerating_partial_failure_errors_when_every_batch_failed() {
        // An empty map here is indistinguishable from "no MRs had state".
        let results: Vec<(usize, Result<MrStateMap>)> = vec![
            (1, Err(anyhow::anyhow!("batch 2 was truncated"))),
            (0, Err(anyhow::anyhow!("batch 1 was truncated"))),
        ];

        let err = merge_tolerating_partial_failure(results)
            .expect_err("a total failure must not look like an empty result");

        // The last batch in batch order, not whichever finished last.
        assert!(err.to_string().contains("batch 2 was truncated"));
    }

    #[test]
    fn merge_tolerating_partial_failure_treats_an_empty_batch_as_a_success() {
        // An empty response is a successful batch, not a failure, so it must
        // not push the merge onto the "every batch failed" path.
        let results: Vec<(usize, Result<MrStateMap>)> = vec![(0, Ok(HashMap::new()))];

        let merged = merge_tolerating_partial_failure(results)
            .expect("an empty batch is a success, not a failure");

        assert!(merged.is_empty());
    }

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
    fn test_page_count_arithmetic() {
        // page_count() is what list_issues, list_mrs, and list_pipelines call to decide
        // how many `--per-page per_request` requests to issue for a given total item
        // budget. Exercising it directly (rather than re-deriving the formula inline)
        // means a regression to the hard-coded `div_ceil(100)` this replaced would fail
        // this test.
        assert_eq!(page_count(100, 20), 5);
        assert_eq!(page_count(100, 100), 1);
        assert_eq!(page_count(250, 100), 3);
        // value already clamped to 1..=100 by Config::api_per_page_clamped(),
        // but the max(1) guard in page_count() ensures division safety.
        assert_eq!(page_count(100, 1), 100);
    }

    // ── project path validation (GraphQL injection guard) ──

    #[test]
    fn accepts_normal_project_paths() {
        assert!(validate_project_path("excalibur/mvp").is_ok());
        assert!(validate_project_path("dev/cbr/salesforce/salesforce").is_ok());
        assert!(validate_project_path("group/sub-group/pro_ject.42").is_ok());
    }

    #[test]
    fn rejects_graphql_significant_characters() {
        // The path comes from `git remote get-url`, so it is untrusted input
        // interpolated into a query string.
        for bad in ["a\"b", "a{b", "a}b", "a\\b", "a\nb", "a b", ""] {
            assert!(
                validate_project_path(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    // ── GraphQL response parsing ──

    /// Captured from the live instance: dev/cbr/salesforce/salesforce.
    const GRAPHQL_OK: &str = r#"{
      "data": {
        "currentUser": { "username": "chandler.anderson" },
        "project": { "mergeRequests": { "nodes": [
          {
            "iid": "5281",
            "approved": true,
            "approvalsLeft": 0,
            "approvalsRequired": 0,
            "userPermissions": { "canApprove": true },
            "approvedBy": { "nodes": [ { "username": "julien.carmignani" } ] },
            "reviewers": { "nodes": [ { "username": "julien.carmignani",
                "mergeRequestInteraction": { "reviewState": "APPROVED" } } ] },
            "conflicts": false,
            "shouldBeRebased": false,
            "detailedMergeStatus": "UNCHECKED"
          },
          {
            "iid": "5279",
            "approved": false,
            "approvalsLeft": 1,
            "approvalsRequired": 1,
            "userPermissions": { "canApprove": false },
            "approvedBy": { "nodes": [] },
            "reviewers": { "nodes": [ { "username": "julien.carmignani",
                "mergeRequestInteraction": { "reviewState": "REQUESTED_CHANGES" } } ] },
            "conflicts": false,
            "shouldBeRebased": false,
            "detailedMergeStatus": "REQUESTED_CHANGES"
          },
          {
            "iid": "1448",
            "approved": true,
            "approvalsLeft": 0,
            "approvalsRequired": 1,
            "userPermissions": { "canApprove": false },
            "approvedBy": { "nodes": [ { "username": "ozgur.gurkan" },
                                       { "username": "chandler.anderson" } ] },
            "reviewers": { "nodes": [] },
            "conflicts": true,
            "shouldBeRebased": false,
            "detailedMergeStatus": "CONFLICT"
          },
          {
            "iid": "402",
            "approved": true,
            "approvalsLeft": 0,
            "approvalsRequired": 0,
            "userPermissions": { "canApprove": false },
            "approvedBy": { "nodes": [ { "username": "chandler.anderson" } ] },
            "reviewers": { "nodes": [] },
            "conflicts": false,
            "shouldBeRebased": true,
            "detailedMergeStatus": "NEED_REBASE"
          }
        ] } }
      }
    }"#;

    #[test]
    fn parses_approval_counts_and_approvers() {
        let map = parse_mr_state_response(GRAPHQL_OK).unwrap();
        let (approval, _) = map.get(&1448).unwrap();
        let a = approval.as_ref().unwrap();
        assert!(a.approved);
        assert_eq!(a.approvals_required, Some(1));
        assert_eq!(a.approved_by.len(), 2);
    }

    #[test]
    fn derives_you_approved_from_current_user() {
        let map = parse_mr_state_response(GRAPHQL_OK).unwrap();
        // chandler.anderson is in !1448's approvedBy.
        assert!(map.get(&1448).unwrap().0.as_ref().unwrap().you_approved);
        // ...and not in !5281's.
        assert!(!map.get(&5281).unwrap().0.as_ref().unwrap().you_approved);
    }

    #[test]
    fn awaiting_you_is_false_for_satisfied_mr_you_can_still_approve() {
        // !5281: canApprove true, you_approved false, but approved true.
        let map = parse_mr_state_response(GRAPHQL_OK).unwrap();
        assert!(!map.get(&5281).unwrap().0.as_ref().unwrap().awaiting_you);
    }

    #[test]
    fn changes_requested_comes_from_reviewer_review_state() {
        let map = parse_mr_state_response(GRAPHQL_OK).unwrap();
        assert!(
            map.get(&5279)
                .unwrap()
                .0
                .as_ref()
                .unwrap()
                .changes_requested
        );
        assert!(
            !map.get(&1448)
                .unwrap()
                .0
                .as_ref()
                .unwrap()
                .changes_requested
        );
    }

    #[test]
    fn conflicts_are_read_from_the_boolean_not_the_merge_status() {
        // !1448 is approved AND conflicted — the two axes must not interfere.
        let map = parse_mr_state_response(GRAPHQL_OK).unwrap();
        let (approval, merge) = map.get(&1448).unwrap();
        assert!(merge.as_ref().unwrap().conflicts);
        assert!(approval.as_ref().unwrap().approved);
    }

    #[test]
    fn needs_rebase_is_read_from_the_boolean_not_the_merge_status() {
        // !402 is approved AND needs rebase — detailedMergeStatus: NEED_REBASE
        // must not suppress the approval, mirroring the conflicts case above.
        let map = parse_mr_state_response(GRAPHQL_OK).unwrap();
        let (approval, merge) = map.get(&402).unwrap();
        assert!(merge.as_ref().unwrap().needs_rebase);
        assert!(approval.as_ref().unwrap().approved);
    }

    #[test]
    fn transient_merge_status_sets_computing() {
        // !5281 reports UNCHECKED.
        let map = parse_mr_state_response(GRAPHQL_OK).unwrap();
        assert!(map.get(&5281).unwrap().1.as_ref().unwrap().computing);
        assert!(!map.get(&1448).unwrap().1.as_ref().unwrap().computing);
    }

    #[test]
    fn graphql_errors_response_is_an_error_not_empty_data() {
        // One unknown field fails the whole query; must not look like "no state".
        let body = r#"{"errors":[{"message":"Field 'x' doesn't exist on type 'MergeRequest'"}]}"#;
        assert!(parse_mr_state_response(body).is_err());
    }

    #[test]
    fn unparseable_iid_skips_that_row_only() {
        let body = GRAPHQL_OK.replace("\"iid\": \"5279\"", "\"iid\": \"not-a-number\"");
        let map = parse_mr_state_response(&body).unwrap();
        assert!(!map.contains_key(&5279));
        assert!(map.contains_key(&1448), "other rows must survive");
    }

    #[test]
    fn parses_current_user_onto_every_state() {
        let map = parse_mr_state_response(GRAPHQL_OK).unwrap();
        let (approval, _) = map.get(&1448).unwrap();
        assert_eq!(
            approval.as_ref().unwrap().current_user.as_deref(),
            Some("chandler.anderson")
        );
    }

    #[test]
    fn you_reviewed_is_true_when_your_review_state_is_present() {
        // !1448's approvedBy contains chandler.anderson, so you reviewed it.
        let map = parse_mr_state_response(GRAPHQL_OK).unwrap();
        assert!(map.get(&1448).unwrap().0.as_ref().unwrap().you_reviewed);
    }

    #[test]
    fn you_reviewed_is_false_when_you_are_an_unreviewed_reviewer() {
        // A reviewer entry for the current user with reviewState UNREVIEWED.
        let body = r#"{
          "data": {
            "currentUser": { "username": "chandler.anderson" },
            "project": { "mergeRequests": { "nodes": [
              {
                "iid": "9001",
                "approved": false,
                "approvalsLeft": 1,
                "approvalsRequired": 1,
                "userPermissions": { "canApprove": true },
                "approvedBy": { "nodes": [] },
                "reviewers": { "nodes": [
                  { "username": "chandler.anderson",
                    "mergeRequestInteraction": { "reviewState": "UNREVIEWED" } }
                ] },
                "conflicts": false,
                "shouldBeRebased": false,
                "detailedMergeStatus": "NOT_APPROVED"
              }
            ] } }
          }
        }"#;
        let map = parse_mr_state_response(body).unwrap();
        assert!(!map.get(&9001).unwrap().0.as_ref().unwrap().you_reviewed);
    }

    #[test]
    fn you_reviewed_ignores_other_peoples_review_state() {
        // Regression guard for restoring `username` in the reviewers
        // subselection: someone ELSE having reviewed must not set your flag.
        let body = r#"{
          "data": {
            "currentUser": { "username": "chandler.anderson" },
            "project": { "mergeRequests": { "nodes": [
              {
                "iid": "9002",
                "approved": false,
                "approvalsLeft": 1,
                "approvalsRequired": 1,
                "userPermissions": { "canApprove": true },
                "approvedBy": { "nodes": [] },
                "reviewers": { "nodes": [
                  { "username": "someone.else",
                    "mergeRequestInteraction": { "reviewState": "REQUESTED_CHANGES" } }
                ] },
                "conflicts": false,
                "shouldBeRebased": false,
                "detailedMergeStatus": "NOT_APPROVED"
              }
            ] } }
          }
        }"#;
        let map = parse_mr_state_response(body).unwrap();
        assert!(!map.get(&9002).unwrap().0.as_ref().unwrap().you_reviewed);
    }

    #[test]
    fn absent_current_user_is_none_not_an_empty_string() {
        // An empty string would make every downstream username comparison
        // silently false, rendering a confident "not yours" instead of the
        // honest "unknown" the cascade expects.
        let body = r#"{
          "data": {
            "currentUser": {},
            "project": { "mergeRequests": { "nodes": [
              {
                "iid": "9003",
                "approved": true,
                "approvalsLeft": 0,
                "approvalsRequired": 1,
                "userPermissions": { "canApprove": false },
                "approvedBy": { "nodes": [ { "username": "someone.else" } ] },
                "reviewers": { "nodes": [
                  { "username": "someone.else",
                    "mergeRequestInteraction": { "reviewState": "APPROVED" } }
                ] },
                "conflicts": false,
                "shouldBeRebased": false,
                "detailedMergeStatus": "MERGEABLE"
              }
            ] } }
          }
        }"#;
        let map = parse_mr_state_response(body).unwrap();
        let a = map.get(&9003).unwrap().0.as_ref().unwrap();
        assert_eq!(a.current_user, None, "must be None, never Some(\"\")");
        assert!(
            !a.you_approved,
            "cannot have approved when the user is unknown"
        );
        assert!(
            !a.you_reviewed,
            "cannot have reviewed when the user is unknown"
        );
    }
}
