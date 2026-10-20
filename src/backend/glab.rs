use super::Backend;
use super::bounded_output::{CapturedStdout, output_bounded};
use super::logged_command::{ChildOutput, LoggedCommand};
use crate::command_log::{Invocation, Program};
use crate::domain::branches::Branch;
use crate::domain::deployments::{Deployment, Environment};
use crate::domain::issues::{Issue, RelatedMrRef};
use crate::domain::job_trace::{JOB_TRACE_MAX_BYTES, JobTrace, TraceCursor, TraceUpdate};
use crate::domain::labels::Label;
use crate::domain::milestones::Milestone;
use crate::domain::mr::{DiscussionNote, MR_DIFF_MAX_BYTES, MergeRequest, ensure_complete_diff};
use crate::domain::notifications::{Notification, NotificationTarget};
use crate::domain::pipelines::{Job, Pipeline};
use crate::domain::releases::Release;
use crate::domain::review::{DraftComment, ReviewEvent};
use crate::domain::runners::{Runner, RunnerType};
use crate::domain::state_query::StateQuery;
use crate::event::Event;
use crate::scope::Scope;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinSet;

/// GitLab rejects a job log `byte_limit` above 500 KiB.
const GITLAB_TRACE_RANGE_MAX_BYTES: usize = 500 * 1024;

/// GitLab caps `per_page` at 100.
const GITLAB_MAX_PER_PAGE: usize = 100;

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

fn parse_glab_issue(raw: &str) -> Result<Issue> {
    Ok(serde_json::from_str(raw)?)
}

fn parse_glab_issues(raw: &str) -> Result<Vec<Issue>> {
    Ok(serde_json::from_str(raw)?)
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
        anyhow::bail!("project path contains unsupported characters: {path}");
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() < 2 || segments.iter().any(|s| s.is_empty()) {
        anyhow::bail!("expected a project path of the form owner/project: {path}");
    }
    Ok(())
}

#[derive(Deserialize)]
struct GiProject {
    path_with_namespace: String,
}

pub fn parse_group_projects(raw: &str) -> anyhow::Result<Vec<String>> {
    let projects: Vec<GiProject> = serde_json::from_str(raw)?;
    Ok(projects
        .into_iter()
        .map(|p| p.path_with_namespace)
        .collect())
}

/// One runner's details from the bulk GraphQL query: its tags, when it was
/// last contacted, and the version and IP of its most recently contacted
/// manager. An absent manager value stays `None`, never `Some("")`.
#[derive(Debug, Default, Deserialize, Clone)]
struct GiRunnerDetails {
    #[serde(default)]
    tag_list: Vec<String>,
    contacted_at: Option<String>,
    #[serde(default)]
    version: Option<String>,
    ip_address: Option<String>,
}

/// Parse the runner-details GraphQL response into runner id -> details.
///
/// GraphQL is all-or-nothing: a top-level `errors` array or a missing
/// `data.project` shape yields `None`, and every runner keeps unknown
/// details rather than a half-parsed set. A malformed runner id or a
/// manager without a contact time drops only that runner's extras.
fn parse_runner_details_response(json: &str) -> Option<HashMap<u64, GiRunnerDetails>> {
    let root: serde_json::Value = serde_json::from_str(json).ok()?;
    if root.get("errors").and_then(|e| e.as_array()).is_some() {
        return None;
    }
    let nodes = root
        .get("data")?
        .get("project")?
        .get("runners")?
        .get("nodes")?
        .as_array()?
        .clone();

    let mut details = HashMap::new();
    for node in nodes {
        let Some(id) = node
            .get("id")
            .and_then(|v| v.as_str())
            .and_then(parse_runner_gid)
        else {
            continue;
        };
        let tag_list = node
            .get("tagList")
            .and_then(|t| t.as_array())
            .map(|tags| {
                tags.iter()
                    .filter_map(|tag| tag.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let contacted_at = node
            .get("contactedAt")
            .and_then(|c| c.as_str())
            .filter(|c| !c.is_empty())
            .map(String::from);

        // The most recently contacted manager names the runner's version and
        // IP; GitLab deprecated the runner's own fields in 17.0.
        let newest_manager = node
            .get("managers")
            .and_then(|m| m.get("nodes"))
            .and_then(|n| n.as_array())
            .and_then(|managers| {
                managers
                    .iter()
                    .filter_map(|m| {
                        m.get("contactedAt")
                            .and_then(|c| c.as_str())
                            .map(|contacted_at| (contacted_at, m))
                    })
                    .max_by(|(left, _), (right, _)| left.cmp(right))
                    .map(|(_, m)| m)
            });
        let version = newest_manager
            .and_then(|m| m.get("version"))
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .map(String::from);
        let ip_address = newest_manager
            .and_then(|m| m.get("ipAddress"))
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .map(String::from);

        details.insert(
            id,
            GiRunnerDetails {
                tag_list,
                contacted_at,
                version,
                ip_address,
            },
        );
    }
    Some(details)
}

/// The numeric id of a GraphQL runner global id
/// (`gid://gitlab/Ci::Runner/9876` -> `9876`).
fn parse_runner_gid(gid: &str) -> Option<u64> {
    gid.rsplit('/').next()?.parse().ok()
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
        anyhow::bail!("GraphQL error: {first}");
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

        let you_approved = !current_user.is_empty() && approved_by.contains(&current_user);

        // Your own review state, matched by username. Someone else having
        // reviewed must not set this.
        let you_reviewed = !current_user.is_empty()
            && (approved_by.contains(&current_user)
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

/// Run one `glab` invocation and log it to the terminal pane.
///
/// Free-standing rather than a method so its future is `'static` and can be
/// spawned: `tx` is the only state a command needs from `GlabBackend`.
async fn run_glab_command(
    tx: Option<UnboundedSender<Event>>,
    args: Vec<String>,
    desc: String,
) -> Result<String> {
    let stdout = run_glab_bounded(tx, args, desc, None).await?;
    Ok(String::from_utf8(stdout.bytes)?)
}

/// `run_glab_command` keeping at most `max_stdout_bytes` of its output.
async fn run_glab_bounded(
    tx: Option<UnboundedSender<Event>>,
    args: Vec<String>,
    desc: String,
    max_stdout_bytes: Option<usize>,
) -> Result<CapturedStdout> {
    super::rate_limit::execute_with_retry(|| {
        let tx = tx.clone();
        let args = args.clone();
        let desc = desc.clone();
        async move { run_glab_command_inner(tx, args, desc, max_stdout_bytes).await }
    })
    .await
}

async fn run_glab_command_inner(
    tx: Option<UnboundedSender<Event>>,
    args: Vec<String>,
    desc: String,
    max_stdout_bytes: Option<usize>,
) -> Result<CapturedStdout> {
    let logged = LoggedCommand::start(&desc, Invocation::new(Program::Glab, &args));

    let output = match output_bounded(Command::new("glab").args(&args), max_stdout_bytes).await {
        Ok(output) => output,
        Err(e) => {
            logged.finish(
                tx.as_ref(),
                format!("Failed: {e}"),
                ChildOutput::NOT_STARTED,
            );
            return Err(e.into());
        }
    };

    if output.succeeded() {
        logged.finish(tx.as_ref(), "Success".to_string(), output.child_output());
        Ok(output.stdout)
    } else {
        let child_output = output.child_output();
        let reason = logged.failure_reason(&child_output);
        logged.finish(tx.as_ref(), format!("Failed: {reason}"), child_output);
        anyhow::bail!("glab command failed: {reason}")
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
    let mut tasks = JoinSet::new();

    for (index, args) in requests.into_iter().enumerate() {
        let tx = tx.clone();
        let desc = desc.to_string();
        tasks.spawn(async move { (index, run_glab_command(tx, args, desc).await) });
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

/// First element of the JSON array a `glab … --output json` list printed.
/// `glab` is written in Go, where an empty nil slice marshals to `null`.
fn first_listed<T: serde::de::DeserializeOwned>(raw: &str) -> Result<Option<T>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let items: Option<Vec<T>> = serde_json::from_str(raw)?;
    Ok(items.and_then(|items| items.into_iter().next()))
}

pub struct GlabBackend {
    tx: Option<UnboundedSender<Event>>,
}

/// Projects whose GitLab answered a ranged job log read with the whole log:
/// it predates `byte_offset`/`byte_limit` and ignores them. Process-wide
/// because every `GitlabClient` clone builds a fresh backend.
static PROJECTS_IGNORING_TRACE_RANGES: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(Mutex::default);

fn ignores_trace_ranges(project: &str) -> bool {
    PROJECTS_IGNORING_TRACE_RANGES
        .lock()
        .is_ok_and(|projects| projects.contains(project))
}

fn remember_ignored_trace_ranges(project: &str) {
    if let Ok(mut projects) = PROJECTS_IGNORING_TRACE_RANGES.lock() {
        projects.insert(project.to_string());
    }
}

/// How one reply to a ranged job log read relates to the part already read.
#[derive(Debug, PartialEq)]
enum RangedReply<'a> {
    /// The requested range, filling `byte_limit`: the log may hold more.
    Chunk(&'a [u8]),
    /// The requested range, ending before `byte_limit`.
    Last(&'a [u8]),
    /// The whole log, which the server sent ignoring the range: what follows
    /// the read part.
    WholeLog(&'a [u8]),
    /// More than `byte_limit`, which a server honouring it never sends.
    Oversized,
    /// Lines up with the read part neither way: the log was rewritten.
    Misaligned,
}

/// Classifies `reply` to a read of at most `limit` bytes from
/// `read_to.resume_offset()`.
fn classify_ranged_reply<'a>(
    read_to: &TraceCursor,
    reply: &'a CapturedStdout,
    limit: usize,
) -> RangedReply<'a> {
    if reply.is_truncated {
        return RangedReply::Oversized;
    }
    if let Some(chunk) = read_to.new_bytes_in_resumed_read(&reply.bytes) {
        return if reply.bytes.len() < limit {
            RangedReply::Last(chunk)
        } else {
            RangedReply::Chunk(chunk)
        };
    }
    match read_to.new_bytes_in_full_read(&reply.bytes) {
        Some(rest) => RangedReply::WholeLog(rest),
        None => RangedReply::Misaligned,
    }
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

    /// Fill tags, version, last contact and IP from one GraphQL query over the
    /// project's runners, the same bulk read that fills the Approval and
    /// Mergeable columns. A runner the query does not return (a group or
    /// project runner the token cannot read details of, which needs
    /// Maintainer on its owning group or project) still lists, with those
    /// fields unknown. GitLab stopped reporting a runner's own `ip_address`
    /// and deprecated its `version` in 17.0 in favour of the runner managers,
    /// so both come from the most recently contacted manager.
    async fn fill_runner_details(&self, project: &str, runners: &mut [Runner]) {
        if runners.is_empty() || validate_project_path(project).is_err() {
            return;
        }
        let first = runners.len().min(RUNNER_DETAILS_QUERY_MAX).to_string();
        let query = format!(
            "query {{ project(fullPath: \"{project}\") {{ runners(first: {first}) {{ nodes {{ \
             id tagList contactedAt managers {{ nodes {{ version ipAddress contactedAt }} }} \
             }} }} }} }}"
        );
        let raw = self
            .run_glab(
                &["api", "graphql", "-f", &format!("query={query}")],
                "Fetching Runner Details",
            )
            .await;
        let Ok(raw) = raw else {
            return;
        };
        let Some(details) = parse_runner_details_response(&raw) else {
            return;
        };
        for runner in runners.iter_mut() {
            let Some(details) = details.get(&runner.id) else {
                continue;
            };
            runner.tags = details.tag_list.clone();
            runner.contacted_at = details.contacted_at.clone();
            runner.version = details.version.clone();
            runner.ip_address = details.ip_address.clone();
        }
    }

    /// Newest pipeline built for `git_ref`. `glab ci list` orders by id,
    /// newest first, so the first row is the one wanted.
    async fn find_latest_pipeline_id(&self, project: &str, git_ref: &str) -> Result<Option<u64>> {
        #[derive(Deserialize)]
        struct ListedPipeline {
            id: u64,
        }
        let raw = self
            .run_glab(
                &[
                    "ci",
                    "list",
                    "--output",
                    "json",
                    "-R",
                    project,
                    "--ref",
                    git_ref,
                    "--per-page",
                    "1",
                ],
                "Finding Branch Pipeline",
            )
            .await?;
        let pipeline: Option<ListedPipeline> =
            first_listed(&raw).context("parsing `glab ci list` output")?;
        Ok(pipeline.map(|p| p.id))
    }

    async fn find_open_mr_iid(&self, project: &str, source_branch: &str) -> Result<Option<u64>> {
        #[derive(Deserialize)]
        struct ListedMr {
            iid: u64,
        }
        let raw = self
            .run_glab(
                &[
                    "mr",
                    "list",
                    "--output",
                    "json",
                    "-R",
                    project,
                    "--source-branch",
                    source_branch,
                    "--per-page",
                    "1",
                ],
                "Finding Branch MR",
            )
            .await?;
        let mr: Option<ListedMr> = first_listed(&raw).context("parsing `glab mr list` output")?;
        Ok(mr.map(|m| m.iid))
    }

    async fn run_glab_capped(
        &self,
        args: &[&str],
        desc: &str,
        max_stdout_bytes: usize,
    ) -> Result<CapturedStdout> {
        run_glab_bounded(
            self.tx.clone(),
            args.iter().map(|a| (*a).to_string()).collect(),
            desc.to_string(),
            Some(max_stdout_bytes),
        )
        .await
    }

    fn job_trace_endpoint(project: &str, job_id: u64) -> String {
        format!(
            "/projects/{}/jobs/{}/trace",
            Self::encode_path(project),
            job_id
        )
    }

    async fn read_job_trace(&self, endpoint: &str) -> Result<CapturedStdout> {
        self.run_glab_capped(&["api", endpoint], "Fetching Job Log", JOB_TRACE_MAX_BYTES)
            .await
    }

    /// Reads what the log gained since `cursor` with `byte_offset` and
    /// `byte_limit`, one chunk of at most `GITLAB_TRACE_RANGE_MAX_BYTES` per
    /// request. `None` when the replies do not line up with `cursor` and a full
    /// read has to settle it: the log was rewritten, or the server ignores the
    /// range and its whole log did not fit in one reply.
    async fn read_job_trace_ranges(
        &self,
        project: &str,
        endpoint: &str,
        cursor: &TraceCursor,
    ) -> Result<Option<TraceUpdate>> {
        let mut read_to = cursor.clone();
        let mut new_bytes = Vec::new();
        loop {
            let remaining = read_to.remaining_bytes();
            if remaining == 0 {
                return Ok(Some(cursor.append_update(new_bytes, true)));
            }
            let limit = (read_to.overlap_len() + remaining).min(GITLAB_TRACE_RANGE_MAX_BYTES);
            let ranged = format!(
                "{endpoint}?byte_offset={}&byte_limit={limit}",
                read_to.resume_offset()
            );
            let reply = self
                .run_glab_capped(&["api", &ranged], "Following Job Log", limit)
                .await?;
            match classify_ranged_reply(&read_to, &reply, limit) {
                RangedReply::Chunk(chunk) => {
                    new_bytes.extend_from_slice(chunk);
                    read_to.advance(chunk);
                }
                RangedReply::Last(chunk) => {
                    new_bytes.extend_from_slice(chunk);
                    return Ok(Some(cursor.append_update(new_bytes, false)));
                }
                RangedReply::WholeLog(rest) => {
                    remember_ignored_trace_ranges(project);
                    new_bytes.extend_from_slice(rest);
                    return Ok(Some(cursor.append_update(new_bytes, false)));
                }
                RangedReply::Oversized => {
                    remember_ignored_trace_ranges(project);
                    return Ok(None);
                }
                RangedReply::Misaligned => return Ok(None),
            }
        }
    }

    /// Build the `glab` arguments that fetch approval/mergeability state for
    /// one batch of iids (at most `MR_STATE_QUERY_BATCH_SIZE`, to stay under
    /// GitLab's 100-node connection cap — see `list_mr_state`).
    fn mr_state_batch_args(project: &str, iids: &[u64]) -> Vec<String> {
        // Query the exact iids the list returned, so there is no pagination drift.
        let iid_list = iids
            .iter()
            .map(|i| format!("\"{i}\""))
            .collect::<Vec<_>>()
            .join(", ");

        // Only fields verified present on the reference instance. GraphQL is
        // all-or-nothing: one unknown field blanks both axes.
        let query = format!(
            "query {{ currentUser {{ username }} \
             project(fullPath: \"{project}\") {{ \
             mergeRequests(iids: [{iid_list}]) {{ nodes {{ \
             iid approved approvalsLeft approvalsRequired \
             userPermissions {{ canApprove }} \
             approvedBy {{ nodes {{ username }} }} \
             reviewers {{ nodes {{ username mergeRequestInteraction {{ reviewState }} }} }} \
             conflicts shouldBeRebased detailedMergeStatus \
             }} }} }} }}"
        );

        vec![
            "api".to_string(),
            "graphql".to_string(),
            "-f".to_string(),
            format!("query={}", query),
        ]
    }

    fn merge_args(
        project: &str,
        iid: u64,
        squash: bool,
        delete_branch: bool,
        strategy: Option<&str>,
        auto_merge: bool,
        sha: Option<&str>,
    ) -> Vec<String> {
        let mut args = vec![
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
            match s {
                "rebase" => args.push("--rebase".into()),
                // "merge" and "squash" are the default behaviors — squash
                // is set via the `squash` flag above, and plain merge is
                // implicit when no flag is passed.
                "merge" | "squash" => {}
                other => debug_assert!(false, "unknown merge strategy: {other}"),
            }
        }
        if auto_merge {
            args.push("--auto-merge=true".into());
        }
        // When auto_merge is false we intentionally omit the flag and let
        // `glab` decide: it defaults to --auto-merge when a pipeline is
        // running, and merges immediately when the pipeline is absent/done.
        // Passing --auto-merge=false forces an immediate merge attempt which
        // causes glab to exhaust its internal retry loop ("All attempts fail")
        // whenever a pipeline is still in progress (#372).
        if let Some(sha) = sha {
            // GitLab 19.2+ and any repo with the group "Require a commit SHA
            // when merging" setting reject merges without --sha
            // (`400 SHA must be provided when merging`). Pass it through
            // verbatim so callers don't have to know the flag name (#470).
            args.push(format!("--sha={sha}"));
        }
        args.push("--yes".into());
        args
    }

    /// Ref and job names come from pipeline data anyone who can push a branch
    /// or edit `.gitlab-ci.yml` controls, so they follow `--` and a name like
    /// `--help` or `-R` stays a positional argument instead of a glab flag.
    fn download_artifact_args(project: &str, ref_name: &str, job_name: &str) -> Vec<String> {
        let mut args: Vec<String> = vec!["job".into(), "artifact".into()];
        if !project.is_empty() {
            args.push("-R".into());
            args.push(project.into());
        }
        args.push("--".into());
        args.push(ref_name.into());
        args.push(job_name.into());
        args
    }

    async fn mr_diff_refs(&self, project: &str, iid: u64) -> Result<GiDiffRefs> {
        #[derive(Deserialize)]
        struct GiMr {
            diff_refs: Option<GiDiffRefs>,
        }
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
                "FETCHING MR DIFF REFS",
            )
            .await?;
        serde_json::from_str::<GiMr>(&raw)
            .context("Failed to parse glab mr view output")?
            .diff_refs
            .with_context(|| format!("!{iid} has no diff refs to anchor comments on"))
    }
}

/// Serde shape returned by the GitLab `/closed_by` endpoint.
/// Shared between the production parser and its tests.
#[derive(Deserialize)]
struct GiClosedBy {
    iid: u64,
    title: String,
    state: String,
}

/// The commits a merge request's diff is computed between; GitLab needs all
/// three in every diff note position.
#[derive(Deserialize)]
struct GiDiffRefs {
    base_sha: String,
    start_sha: String,
    head_sha: String,
}

/// Body of `POST .../draft_notes` for one review comment.
fn gitlab_draft_note_payload(comment: &DraftComment, diff_refs: &GiDiffRefs) -> serde_json::Value {
    let mut position = serde_json::json!({
        "position_type": "text",
        "new_path": comment.file_path,
        "base_sha": diff_refs.base_sha,
        "start_sha": diff_refs.start_sha,
        "head_sha": diff_refs.head_sha,
    });
    if let Some(line) = comment.line_num {
        position["new_line"] = serde_json::json!(line);
    }
    if let Some(old_line) = comment.old_line_num {
        position["old_line"] = serde_json::json!(old_line);
        position["old_path"] = serde_json::json!(comment.file_path);
    }
    let range = match (comment.line_num, comment.end_line_num) {
        (Some(start), Some(end)) => Some(("new_line", start, end)),
        _ => match (comment.old_line_num, comment.end_old_line_num) {
            (Some(start), Some(end)) if comment.end_line_num.is_none() => {
                Some(("old_line", start, end))
            }
            _ => None,
        },
    };
    if let Some((line_type, start, end)) = range.filter(|&(_, start, end)| start != end) {
        position["line_range"] = serde_json::json!({
            "start": { "line_code": "", "type": line_type, line_type: start.min(end) },
            "end": { "line_code": "", "type": line_type, line_type: start.max(end) },
        });
    }
    serde_json::json!({ "note": comment.body, "position": position })
}

/// One element of `glab mr note list --output json`: a discussion with its
/// notes. GitLab does not repeat the discussion id on its notes.
#[derive(Deserialize)]
struct GiDiscussion {
    id: String,
    notes: Vec<GiNote>,
}

#[derive(Deserialize)]
struct GiNote {
    id: u64,
    body: String,
    author: GiNoteAuthor,
    created_at: String,
    system: bool,
    #[serde(default)]
    position: Option<GiNotePosition>,
    #[serde(default)]
    resolved: Option<bool>,
    #[serde(default)]
    resolvable: Option<bool>,
}

#[derive(Deserialize)]
struct GiNoteAuthor {
    username: String,
}

#[derive(Deserialize)]
struct GiNotePosition {
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

/// Flattens the discussions `glab mr note list` prints into notes, each
/// tagged with the id of the discussion it belongs to.
fn parse_mr_discussions(raw: &str) -> Result<Vec<DiscussionNote>> {
    let discussions: Vec<GiDiscussion> =
        serde_json::from_str(raw).context("parsing MR discussions")?;
    Ok(discussions
        .into_iter()
        .flat_map(|discussion| {
            let discussion_id = discussion.id;
            discussion
                .notes
                .into_iter()
                .map(move |note| DiscussionNote {
                    id: note.id,
                    body: note.body,
                    author: crate::domain::mr::Author {
                        username: note.author.username,
                    },
                    created_at: note.created_at,
                    system: note.system,
                    position: note.position.map(|p| crate::domain::mr::NotePosition {
                        new_path: p.new_path,
                        old_path: p.old_path,
                        new_line: p.new_line,
                        old_line: p.old_line,
                        start_line: p.start_line,
                        line_range: p.line_range,
                    }),
                    discussion_id: Some(discussion_id.clone()),
                    resolved: note.resolved,
                    resolvable: note.resolvable,
                })
        })
        .collect())
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
        scope: &Scope,
        state: StateQuery,
        page_size: usize,
        per_request: usize,
    ) -> Result<Vec<Issue>> {
        // Only open-only listings are narrowed here. Closed and merged rows
        // come with every state and the State column filter hides the rest.
        let show_closed = state != StateQuery::Open;
        match scope {
            Scope::Repository(project) => {
                let pages = page_count(page_size, per_request);
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
                    let mut issues = parse_glab_issues(&raw).unwrap_or_default();
                    for issue in &mut issues {
                        if issue.project_path.is_empty() {
                            issue.project_path = project.to_string();
                        }
                    }
                    all.extend(issues);
                }
                Ok(all)
            }
            Scope::Group(group) => {
                let pages = page_count(page_size, per_request);
                let encoded_group = group.replace('/', "%2F");
                let state = if show_closed { "all" } else { "opened" };
                let requests: Vec<Vec<String>> = (1..=pages)
                    .map(|page| {
                        vec![
                            "api".to_string(),
                            format!(
                                "groups/{encoded_group}/issues?state={state}&per_page={per_request}&page={page}"
                            ),
                        ]
                    })
                    .collect();
                let responses = ordered_or_first_error(
                    run_glab_concurrent(self.tx.clone(), requests, "FETCHING GROUP ISSUES").await,
                )?;

                let mut all: Vec<Issue> = Vec::new();
                for raw in responses {
                    let mut issues: Vec<Issue> = serde_json::from_str(&raw).unwrap_or_default();
                    for issue in &mut issues {
                        if issue.project_path.is_empty() {
                            issue.project_path =
                                crate::git_helpers::parse_project_path_from_web_url(&issue.web_url)
                                    .unwrap_or_else(|| group.clone());
                        }
                    }
                    all.extend(issues);
                }
                Ok(all)
            }
        }
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
        parse_glab_issue(&raw)
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

    async fn list_issue_related_mrs(
        &self,
        project: &str,
        issue_iid: u64,
        page_size: usize,
    ) -> Result<Vec<RelatedMrRef>> {
        let encoded = Self::encode_path(project);
        let endpoint =
            format!("/projects/{encoded}/issues/{issue_iid}/closed_by?per_page={page_size}");
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Related MRs")
            .await?;
        let items: Vec<GiClosedBy> = serde_json::from_str(&raw)?;
        Ok(items
            .into_iter()
            .map(|m| RelatedMrRef {
                iid: m.iid,
                title: m.title,
                state: m.state,
                project_path: Some(project.to_string()),
            })
            .collect())
    }

    async fn create_issue(&self, project: &str, issue: &super::NewIssue<'_>) -> Result<()> {
        let super::NewIssue {
            title,
            description,
            labels,
            assignees,
            milestone,
            due_date,
            weight,
        } = *issue;
        let mut args: Vec<String> = vec!["issue".into(), "create".into(), "-y".into()];
        if !project.is_empty() {
            args.push("-R".into());
            args.push(project.into());
        }
        args.extend(["--title".into(), title.into()]);
        if !description.is_empty() {
            args.push("--description".into());
            args.push(description.into());
        }
        if !labels.is_empty() {
            args.push("--label".into());
            args.push(normalize_labels(labels));
        }
        if !assignees.is_empty() {
            args.push("--assignee".into());
            args.push(strip_ats(assignees));
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

    async fn update_issue(
        &self,
        project: &str,
        iid: u64,
        update: &super::IssueUpdate,
    ) -> Result<()> {
        if update.is_empty() {
            return Ok(());
        }
        let iid_str = iid.to_string();
        let mut args: Vec<String> = vec!["issue".into(), "update".into(), iid_str];
        if !project.is_empty() {
            args.extend(["-R".into(), project.into()]);
        }
        if let Some(ref title) = update.title {
            args.extend(["--title".into(), title.clone()]);
        }
        if let Some(ref desc) = update.description {
            args.extend(["-d".into(), desc.clone()]);
        }
        for label in &update.add_labels {
            args.extend(["--label".into(), label.clone()]);
        }
        for label in &update.remove_labels {
            args.extend(["--unlabel".into(), label.clone()]);
        }
        for a in &update.add_assignees {
            args.extend(["--assignee".into(), a.clone()]);
        }
        for a in &update.remove_assignees {
            args.extend(["--unassign".into(), a.clone()]);
        }
        if let Some(ref milestone) = update.milestone {
            let val = if milestone == "--" || milestone.is_empty() {
                "0"
            } else {
                milestone.as_str()
            };
            args.extend(["--milestone".into(), val.to_string()]);
        }
        if let Some(ref due_date) = update.due_date {
            if !due_date.is_empty() && due_date != "YYYY-MM-DD" {
                args.extend(["--due-date".into(), due_date.clone()]);
            }
        }
        if let Some(ref weight) = update.weight {
            if !weight.is_empty() {
                args.extend(["--weight".into(), weight.clone()]);
            }
        }
        if let Some(confidential) = update.confidential {
            if confidential {
                args.push("--confidential".into());
            } else {
                args.push("--public".into());
            }
        }
        let args_refs: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
        self.run_glab(&args_refs, "UPDATING ISSUE").await?;
        Ok(())
    }

    // ── Merge Requests ──

    async fn list_mrs(
        &self,
        scope: &Scope,
        state: StateQuery,
        page_size: usize,
        per_request: usize,
    ) -> Result<Vec<MergeRequest>> {
        // Only open-only listings are narrowed here. Closed and merged rows
        // come with every state and the State column filter hides the rest.
        let show_closed = state != StateQuery::Open;
        match scope {
            Scope::Repository(project) => {
                let pages = page_count(page_size, per_request);
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
                        #[serde(default)]
                        sha: Option<String>,
                        draft: bool,
                        description: Option<String>,
                        #[serde(default)]
                        head_pipeline: Option<GiPipeline>,
                        #[serde(default)]
                        blocking_discussions_resolved: Option<bool>,
                        #[serde(default)]
                        web_url: Option<String>,
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
                        #[serde(default)]
                        web_url: Option<String>,
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
                            sha: m.sha,
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
                                duration_seconds: None,
                                created_at: None,
                                source: None,
                                project_path: String::new(),
                                web_url: p.web_url,
                                commit: None,
                            }),
                            blocking_discussions_resolved: m.blocking_discussions_resolved,
                            approval: None,
                            mergeability: None,
                            workflow: None,
                            project_path: String::new(),
                            web_url: m.web_url,
                            related_issues: None,
                            stack: None,
                            stack_entries: None,
                        }
                    }));
                }
                for mr in &mut all {
                    if mr.project_path.is_empty() {
                        mr.project_path = project.to_string();
                    }
                }
                Ok(all)
            }
            Scope::Group(group) => {
                let pages = page_count(page_size, per_request);
                let encoded_group = group.replace('/', "%2F");
                let state = if show_closed { "all" } else { "opened" };
                let requests: Vec<Vec<String>> = (1..=pages)
                    .map(|page| {
                        vec![
                            "api".to_string(),
                            format!(
                                "groups/{}/merge_requests?state={}&per_page={}&page={}",
                                encoded_group, state, per_request, page
                            ),
                        ]
                    })
                    .collect();
                let responses = ordered_or_first_error(
                    run_glab_concurrent(self.tx.clone(), requests, "FETCHING GROUP MERGE REQUESTS")
                        .await,
                )?;

                let mut all: Vec<MergeRequest> = Vec::new();
                for raw in responses {
                    let mut mrs: Vec<MergeRequest> = serde_json::from_str(&raw).unwrap_or_default();
                    for mr in &mut mrs {
                        if mr.project_path.is_empty() {
                            mr.project_path = mr
                                .web_url
                                .as_deref()
                                .and_then(crate::git_helpers::parse_project_path_from_web_url)
                                .unwrap_or_else(|| group.clone());
                        }
                    }
                    all.extend(mrs);
                }
                Ok(all)
            }
        }
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
            #[serde(default)]
            sha: Option<String>,
            draft: bool,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            head_pipeline: Option<GiPipeline>,
            #[serde(default)]
            blocking_discussions_resolved: Option<bool>,
            #[serde(default)]
            web_url: Option<String>,
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
            #[serde(default)]
            web_url: Option<String>,
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
            sha: m.sha,
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
                duration_seconds: None,
                created_at: None,
                source: None,
                project_path: String::new(),
                web_url: p.web_url,
                commit: None,
            }),
            blocking_discussions_resolved: m.blocking_discussions_resolved,
            approval: None,
            mergeability: None,
            workflow: None,
            project_path: String::new(),
            web_url: m.web_url,
            related_issues: None,
            stack: None,
            stack_entries: None,
        })
    }

    async fn get_mr_diff(&self, project: &str, iid: u64) -> Result<String> {
        let diff = self
            .run_glab_capped(
                &["mr", "diff", &iid.to_string(), "-R", project],
                "Fetching MR Diff",
                MR_DIFF_MAX_BYTES,
            )
            .await?;
        ensure_complete_diff(diff.is_truncated, &format!("MR !{iid}"))?;
        Ok(String::from_utf8(diff.bytes)?)
    }

    async fn list_mr_notes(&self, project: &str, mr_iid: u64) -> Result<Vec<DiscussionNote>> {
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
        parse_mr_discussions(&raw)
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

    async fn list_mr_related_issues(
        &self,
        project: &str,
        mr_iid: u64,
        page_size: usize,
    ) -> Result<Vec<crate::domain::mr::RelatedIssueRef>> {
        let encoded = Self::encode_path(project);
        let endpoint = format!(
            "/projects/{encoded}/merge_requests/{mr_iid}/closes_issues?per_page={page_size}"
        );
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Related Issues")
            .await?;
        #[derive(Deserialize)]
        struct GiClosesIssue {
            iid: u64,
            title: String,
            state: String,
        }
        let items: Vec<GiClosesIssue> = serde_json::from_str(&raw)?;
        Ok(items
            .into_iter()
            .map(|i| crate::domain::mr::RelatedIssueRef {
                iid: i.iid,
                title: i.title,
                state: i.state,
                project_path: Some(project.to_string()),
            })
            .collect())
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
        options: &super::MergeOptions<'_>,
    ) -> Result<()> {
        let super::MergeOptions {
            squash,
            delete_branch,
            strategy,
            auto_merge,
            sha,
        } = *options;
        let args = Self::merge_args(
            project,
            iid,
            squash,
            delete_branch,
            strategy,
            auto_merge,
            sha,
        );
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, "MERGING MR").await?;
        Ok(())
    }

    async fn toggle_mr_draft(&self, project: &str, iid: u64, is_draft: bool) -> Result<()> {
        let flag = if is_draft { "--draft" } else { "--ready" };
        let label = if is_draft {
            "DRAFTING MR"
        } else {
            "MARKING MR READY"
        };
        let mut args: Vec<String> =
            vec!["mr".into(), "update".into(), iid.to_string(), flag.into()];
        if !project.is_empty() {
            args.extend(["-R".into(), project.into()]);
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, label).await?;
        Ok(())
    }

    async fn create_mr(&self, project: &str, mr: &super::NewMr<'_>) -> Result<()> {
        let super::NewMr {
            title,
            description,
            source_branch,
            target_branch,
            labels,
            assignees,
            reviewers,
            milestone,
            issue_iid,
        } = *mr;
        let mut args: Vec<String> = vec!["mr".into(), "create".into(), "-y".into()];
        if !project.is_empty() {
            args.push("-R".into());
            args.push(project.into());
        }
        args.extend(["--title".into(), title.into()]);
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
            args.push(normalize_labels(labels));
        }
        if !assignees.is_empty() {
            args.push("--assignee".into());
            args.push(strip_ats(assignees));
        }
        if !reviewers.is_empty() {
            args.push("--reviewer".into());
            args.push(strip_ats(reviewers));
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

    async fn submit_review(
        &self,
        project: &str,
        iid: u64,
        event: ReviewEvent,
        body: &str,
        comments: &[DraftComment],
    ) -> Result<()> {
        if !comments.is_empty() {
            let diff_refs = self.mr_diff_refs(project, iid).await?;
            let endpoint = format!(
                "/projects/{}/merge_requests/{}/draft_notes",
                Self::encode_path(project),
                iid
            );
            for (created, comment) in comments.iter().enumerate() {
                let payload = gitlab_draft_note_payload(comment, &diff_refs).to_string();
                self.raw_api(&endpoint, "POST", Some(&payload), "CREATING DRAFT NOTE")
                    .await
                    .with_context(|| {
                        format!(
                            "draft note on {} failed; {} earlier draft note(s) of this review stay pending on !{}",
                            comment.file_path, created, iid
                        )
                    })?;
            }
            self.raw_api(
                &format!("{endpoint}/bulk_publish"),
                "POST",
                None,
                "PUBLISHING DRAFT NOTES",
            )
            .await?;
        }
        if event == ReviewEvent::Approve {
            self.approve_mr(project, iid).await?;
        }
        if !body.trim().is_empty() {
            self.run_glab(
                &[
                    "mr",
                    "note",
                    "create",
                    &iid.to_string(),
                    "-R",
                    project,
                    "-m",
                    body,
                ],
                "ADDING REVIEW SUMMARY",
            )
            .await?;
        }
        Ok(())
    }

    async fn reply_to_thread(
        &self,
        project: &str,
        iid: u64,
        thread_id: &str,
        body: &str,
    ) -> Result<()> {
        self.run_glab(
            &[
                "mr",
                "note",
                "create",
                &iid.to_string(),
                "-R",
                project,
                "--reply",
                thread_id,
                "-m",
                body,
            ],
            "REPLYING TO THREAD",
        )
        .await?;
        Ok(())
    }

    async fn set_thread_resolved(
        &self,
        project: &str,
        iid: u64,
        thread_id: &str,
        resolved: bool,
    ) -> Result<()> {
        let (action, desc) = if resolved {
            ("resolve", "RESOLVING THREAD")
        } else {
            ("reopen", "REOPENING THREAD")
        };
        self.run_glab(
            &[
                "mr",
                "note",
                action,
                thread_id,
                &iid.to_string(),
                "-R",
                project,
            ],
            desc,
        )
        .await?;
        Ok(())
    }

    async fn update_mr(&self, project: &str, iid: u64, update: &super::MrUpdate) -> Result<()> {
        if update.is_empty() {
            return Ok(());
        }
        let iid_str = iid.to_string();
        let mut args: Vec<String> = vec!["mr".into(), "update".into(), iid_str];
        if !project.is_empty() {
            args.extend(["-R".into(), project.into()]);
        }
        if let Some(ref title) = update.title {
            args.extend(["--title".into(), title.clone()]);
        }
        if let Some(ref desc) = update.description {
            args.extend(["-d".into(), desc.clone()]);
        }
        for label in &update.add_labels {
            args.extend(["--label".into(), label.clone()]);
        }
        for label in &update.remove_labels {
            args.extend(["--unlabel".into(), label.clone()]);
        }
        for a in &update.add_assignees {
            args.extend(["--assignee".into(), a.clone()]);
        }
        for a in &update.remove_assignees {
            args.extend(["--unassign".into(), a.clone()]);
        }
        for r in &update.add_reviewers {
            args.extend(["--reviewer".into(), r.clone()]);
        }
        for r in &update.remove_reviewers {
            args.extend(["--unreviewer".into(), r.clone()]);
        }
        if let Some(ref milestone) = update.milestone {
            let val = if milestone == "--" || milestone.is_empty() {
                "0"
            } else {
                milestone.as_str()
            };
            args.extend(["--milestone".into(), val.to_string()]);
        }
        if let Some(ref target_branch) = update.target_branch {
            args.extend(["--target-branch".into(), target_branch.clone()]);
        }
        if let Some(draft) = update.draft {
            if draft {
                args.push("--draft".into());
            } else {
                args.push("--ready".into());
            }
        }
        let args_refs: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
        self.run_glab(&args_refs, "UPDATING MR").await?;
        Ok(())
    }

    // ── Pipelines ──

    async fn list_pipelines(
        &self,
        scope: &Scope,
        page_size: usize,
        per_request: usize,
    ) -> Result<Vec<Pipeline>> {
        match scope {
            Scope::Repository(project) => {
                let pages = page_count(page_size, per_request);
                let encoded = Self::encode_path(project);

                let requests: Vec<Vec<String>> = (1..=pages)
                    .map(|page| {
                        vec![
                            "api".to_string(),
                            format!(
                                "/projects/{encoded}/pipelines?per_page={per_request}&page={page}"
                            ),
                        ]
                    })
                    .collect();
                let responses = ordered_or_first_error(
                    run_glab_concurrent(self.tx.clone(), requests, "Fetching Pipelines").await,
                )?;

                #[derive(Deserialize)]
                struct GiUser {
                    username: Option<String>,
                    name: Option<String>,
                }
                #[derive(Deserialize)]
                struct GiPipe {
                    id: u64,
                    status: String,
                    #[serde(rename = "ref")]
                    pipe_ref: String,
                    updated_at: String,
                    created_at: Option<String>,
                    duration: Option<f64>,
                    source: Option<String>,
                    sha: Option<String>,
                    user: Option<GiUser>,
                    #[serde(default)]
                    web_url: Option<String>,
                }

                let mut all: Vec<Pipeline> = Vec::new();
                for raw in responses {
                    let pipes: Vec<GiPipe> = serde_json::from_str(&raw)?;
                    all.extend(pipes.into_iter().map(|p| {
                        Pipeline {
                            id: p.id,
                            status: p.status,
                            r#ref: p.pipe_ref,
                            updated_at: p.updated_at,
                            name: String::new(),
                            display_title: String::new(),
                            event: p.source.clone().unwrap_or_default(),
                            head_sha: p.sha.unwrap_or_default(),
                            actor_login: p
                                .user
                                .and_then(|u| u.username.or(u.name))
                                .unwrap_or_default(),
                            duration_seconds: p.duration.map(|d| d.max(0.0) as u64),
                            created_at: p.created_at,
                            source: p.source,
                            project_path: project.to_string(),
                            web_url: p.web_url,
                            commit: None,
                        }
                    }));
                }
                Ok(all)
            }
            Scope::Group(group) => {
                let pages = page_count(page_size, per_request);
                let encoded_group = group.replace('/', "%2F");
                let requests: Vec<Vec<String>> = (1..=pages)
                    .map(|page| {
                        vec![
                            "api".to_string(),
                            format!(
                                "groups/{}/pipelines?per_page={}&page={}",
                                encoded_group, per_request, page
                            ),
                        ]
                    })
                    .collect();
                let responses = ordered_or_first_error(
                    run_glab_concurrent(self.tx.clone(), requests, "FETCHING GROUP PIPELINES")
                        .await,
                )?;

                let mut all: Vec<Pipeline> = Vec::new();
                for raw in responses {
                    let mut pipelines: Vec<Pipeline> =
                        serde_json::from_str(&raw).unwrap_or_default();
                    for pipe in &mut pipelines {
                        if pipe.project_path.is_empty() {
                            pipe.project_path = pipe
                                .web_url
                                .as_deref()
                                .and_then(crate::git_helpers::parse_project_path_from_web_url)
                                .unwrap_or_else(|| group.clone());
                        }
                    }
                    all.extend(pipelines);
                }
                Ok(all)
            }
        }
    }

    async fn find_branch_pipeline_id(&self, project: &str, branch: &str) -> Result<Option<u64>> {
        let (branch_pipeline, mr_iid) = tokio::try_join!(
            self.find_latest_pipeline_id(project, branch),
            self.find_open_mr_iid(project, branch),
        )?;
        let mr_pipeline = match mr_iid {
            Some(iid) => {
                let mr_head_ref = format!("refs/merge-requests/{iid}/head");
                self.find_latest_pipeline_id(project, &mr_head_ref).await?
            }
            None => None,
        };
        Ok(branch_pipeline.max(mr_pipeline))
    }

    async fn list_pipeline_jobs(
        &self,
        project: &str,
        pipeline_id: u64,
        page_size: usize,
    ) -> Result<Vec<Job>> {
        let encoded = Self::encode_path(project);
        let endpoint =
            format!("/projects/{encoded}/pipelines/{pipeline_id}/jobs?per_page={page_size}");
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Jobs")
            .await?;
        #[derive(Deserialize)]
        struct GiJob {
            id: u64,
            status: String,
            stage: String,
            name: String,
            #[serde(default)]
            duration: Option<f64>,
            #[serde(default)]
            queued_duration: Option<f64>,
            #[serde(default)]
            runner: Option<GiRunner>,
            #[serde(default)]
            web_url: Option<String>,
            #[serde(default)]
            failure_reason: Option<String>,
            #[serde(default)]
            artifacts: Option<Vec<GiArtifact>>,
            #[serde(default)]
            commit: Option<crate::domain::pipelines::CommitSummary>,
        }
        #[derive(Deserialize)]
        struct GiRunner {
            #[serde(default)]
            description: Option<String>,
        }
        #[derive(Deserialize)]
        struct GiArtifact {
            file_type: String,
        }
        let jobs: Vec<GiJob> = serde_json::from_str(&raw)?;
        let all_jobs: Vec<Job> = jobs
            .into_iter()
            .map(|j| Job {
                failure_reason: j
                    .failure_reason
                    .filter(|_| j.status == "failed")
                    .map(|reason| reason.replace('_', " ")),
                id: j.id,
                status: j.status,
                stage: j.stage,
                name: j.name,
                api_name: String::new(),
                matrix: None,
                duration_seconds: j.duration.map(|seconds| seconds.max(0.0) as u64),
                runner: j.runner.and_then(|runner| runner.description),
                needs: Vec::new(),
                web_url: j.web_url,
                queued_seconds: j.queued_duration.map(|seconds| seconds.max(0.0) as u64),
                artifact_file_types: j
                    .artifacts
                    .into_iter()
                    .flatten()
                    .map(|artifact| artifact.file_type)
                    // GitLab lists every finished job's own log as a `trace` artifact.
                    .filter(|file_type| file_type != "trace")
                    .collect(),
                commit: j.commit,
            })
            .collect();
        Ok(crate::domain::pipelines::process_pipeline_jobs(all_jobs))
    }

    async fn list_pipeline_bridges(
        &self,
        project: &str,
        pipeline_id: u64,
        page_size: usize,
    ) -> Result<Vec<crate::domain::pipelines::Bridge>> {
        let encoded = Self::encode_path(project);
        let endpoint =
            format!("/projects/{encoded}/pipelines/{pipeline_id}/bridges?per_page={page_size}");
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Bridges")
            .await?;
        Ok(serde_json::from_str(&raw)?)
    }

    async fn get_job_trace(&self, project: &str, job_id: u64) -> Result<JobTrace> {
        let log = self
            .read_job_trace(&Self::job_trace_endpoint(project, job_id))
            .await?;
        Ok(JobTrace::from_read(&log.bytes, log.is_truncated))
    }

    async fn get_job_trace_since(
        &self,
        project: &str,
        job_id: u64,
        cursor: &TraceCursor,
    ) -> Result<TraceUpdate> {
        let endpoint = Self::job_trace_endpoint(project, job_id);
        if !ignores_trace_ranges(project) {
            if let Some(update) = self
                .read_job_trace_ranges(project, &endpoint, cursor)
                .await?
            {
                return Ok(update);
            }
        }
        let log = self.read_job_trace(&endpoint).await?;
        Ok(match cursor.new_bytes_in_full_read(&log.bytes) {
            Some(new_bytes) => cursor.append_update(new_bytes.to_vec(), log.is_truncated),
            None => cursor.restart_update(JobTrace::from_read(&log.bytes, log.is_truncated)),
        })
    }

    async fn retry_pipeline(&self, project: &str, pipeline_id: u64) -> Result<()> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{encoded}/pipelines/{pipeline_id}/retry");
        self.raw_api(&endpoint, "POST", None, "Retrying Pipeline")
            .await?;
        Ok(())
    }

    /// Retries each failed job on its own: GitLab's pipeline retry endpoint
    /// also reruns canceled jobs. Without `include_retried` the listing holds
    /// only the latest attempt of each job, so a retried job leaves it and
    /// shifts the pages after it: every page is read before the first retry.
    async fn retry_failed_jobs(&self, project: &str, pipeline_id: u64) -> Result<()> {
        #[derive(Deserialize)]
        struct FailedJob {
            id: u64,
        }
        let encoded = Self::encode_path(project);
        let mut failed: Vec<FailedJob> = Vec::new();
        for page in 1.. {
            let endpoint = format!(
                "/projects/{encoded}/pipelines/{pipeline_id}/jobs?scope[]=failed&per_page={GITLAB_MAX_PER_PAGE}&page={page}"
            );
            let raw = self
                .raw_api(&endpoint, "GET", None, "Fetching Failed Jobs")
                .await?;
            let batch: Vec<FailedJob> =
                serde_json::from_str(&raw).context("parsing the failed jobs of a pipeline")?;
            let is_last_page = batch.len() < GITLAB_MAX_PER_PAGE;
            failed.extend(batch);
            if is_last_page {
                break;
            }
        }
        if failed.is_empty() {
            anyhow::bail!("pipeline #{pipeline_id} has no failed jobs to retry");
        }
        let mut errors = Vec::new();
        for (i, job) in failed.iter().enumerate() {
            if i > 0 {
                crate::backend::rate_limit::pace_bulk_operation().await;
            }
            if let Err(error) = self.retry_job(project, job.id).await {
                errors.push(format!("job #{}: {error}", job.id));
            }
        }
        if !errors.is_empty() {
            anyhow::bail!(
                "retried {} of {} failed jobs: {}",
                failed.len() - errors.len(),
                failed.len(),
                errors.join("; ")
            );
        }
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
        let endpoint = format!("/projects/{encoded}/jobs/{job_id}/play");
        self.raw_api(&endpoint, "POST", None, "Starting Job")
            .await?;
        Ok(())
    }

    async fn run_pipeline(
        &self,
        project: &str,
        branch: &str,
        mr: bool,
        variables: &[(String, String)],
        inputs: &[(String, String)],
        _workflow_file: &str,
    ) -> Result<()> {
        let mut args: Vec<String> = vec!["ci".into(), "run".into()];
        if !project.is_empty() {
            args.push("-R".into());
            args.push(project.into());
        }
        if !branch.is_empty() {
            args.push("--branch".into());
            args.push(branch.into());
        }
        if mr && variables.is_empty() && inputs.is_empty() {
            args.push("--mr".into());
        }
        for (k, v) in variables {
            args.push("--variables".into());
            args.push(format!("{k}:{v}"));
        }
        for (k, v) in inputs {
            args.push("--input".into());
            args.push(format!("{k}:{v}"));
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, "RUNNING PIPELINE").await?;
        Ok(())
    }

    async fn download_artifact(&self, project: &str, ref_name: &str, job_name: &str) -> Result<()> {
        let args = Self::download_artifact_args(project, ref_name, job_name);
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, "DOWNLOADING ARTIFACT").await?;
        Ok(())
    }

    // ── Runners ──

    async fn list_runners(&self, scope: &Scope, page_size: usize) -> Result<Vec<Runner>> {
        let project = match scope {
            Scope::Repository(p) => p,
            Scope::Group(_) => return Err(anyhow::anyhow!("Group-level runners not supported")),
        };
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
            active: Option<bool>,
            paused: Option<bool>,
            runner_type: Option<RunnerType>,
        }
        let listed: Vec<GiRunner> = serde_json::from_str(&raw)?;
        let host = tokio::task::spawn_blocking(gitlab_host_or_default)
            .await
            .unwrap_or_else(|_| DEFAULT_GITLAB_HOST.to_string());
        let mut runners: Vec<Runner> = listed
            .into_iter()
            .map(|r| Runner {
                id: r.id,
                description: r.description,
                status: r.status,
                active: r.paused.map(|paused| !paused).or(r.active).unwrap_or(true),
                runner_type: r.runner_type,
                web_url: Some(gitlab_runner_url(&host, scope, r.id)),
                ..Runner::default()
            })
            .collect();
        self.fill_runner_details(project, &mut runners).await;
        Ok(runners)
    }

    async fn pause_runner(&self, scope: &Scope, runner_id: u64) -> Result<()> {
        let runner_str = runner_id.to_string();
        let mut args = vec!["runner", "update", &runner_str, "--pause"];
        let proj;
        if let Scope::Repository(p) = scope {
            if !p.is_empty() {
                proj = p.as_str();
                args.push("-R");
                args.push(proj);
            }
        }
        self.run_glab(&args, "PAUSING RUNNER").await?;
        Ok(())
    }

    async fn resume_runner(&self, scope: &Scope, runner_id: u64) -> Result<()> {
        let runner_str = runner_id.to_string();
        let mut args = vec!["runner", "update", &runner_str, "--unpause"];
        let proj;
        if let Scope::Repository(p) = scope {
            if !p.is_empty() {
                proj = p.as_str();
                args.push("-R");
                args.push(proj);
            }
        }
        self.run_glab(&args, "RESUMING RUNNER").await?;
        Ok(())
    }

    async fn update_runner_description(
        &self,
        _scope: &Scope,
        runner_id: u64,
        description: &str,
    ) -> Result<()> {
        let endpoint = format!("runners/{runner_id}");
        let body = serde_json::json!({ "description": description }).to_string();
        self.raw_api(&endpoint, "PUT", Some(&body), "UPDATING RUNNER DESCRIPTION")
            .await?;
        Ok(())
    }

    // ── Releases ──

    async fn list_releases(&self, scope: &Scope, page_size: usize) -> Result<Vec<Release>> {
        let project = match scope {
            Scope::Repository(p) => p,
            Scope::Group(_) => return Err(anyhow::anyhow!("Group-level releases not supported")),
        };
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
                    project_path: project.clone(),
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
                "create",
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

    async fn list_milestones(&self, scope: &Scope, page_size: usize) -> Result<Vec<Milestone>> {
        let raw = match scope {
            Scope::Repository(project) => {
                self.run_glab(
                    &[
                        "milestone",
                        "list",
                        "--output",
                        "json",
                        "--project",
                        project,
                        "--per-page",
                        &page_size.to_string(),
                    ],
                    "Fetching Milestones",
                )
                .await?
            }
            Scope::Group(group) => {
                let encoded_group = group.replace('/', "%2F");
                self.run_glab(
                    &[
                        "api",
                        &format!("groups/{encoded_group}/milestones?per_page={page_size}"),
                    ],
                    "Fetching Group Milestones",
                )
                .await?
            }
        };
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
        let project_fallback = match scope {
            Scope::Repository(r) => r.clone(),
            Scope::Group(g) => g.clone(),
        };
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
                project_path: project_fallback.clone(),
            })
            .collect())
    }

    async fn list_milestone_issues(
        &self,
        project: &str,
        _milestone_iid: u64,
        milestone_title: &str,
        page_size: usize,
    ) -> Result<Vec<Issue>> {
        // `glab issue list --milestone` filters by milestone **title**, not
        // iid/id — passing the iid silently returns an empty list, which made
        // milestone progress render as 0%. The title is resolved at the call
        // site and passed through the domain layer.
        let raw = self
            .run_glab(
                &[
                    "issue",
                    "list",
                    "--milestone",
                    milestone_title,
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
        parse_glab_issues(&raw)
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
        let endpoint = format!("projects/{encoded}/milestones");
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
        ];
        if !description.is_empty() {
            args.push("--description".into());
            args.push(description.into());
        }
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

    async fn mark_notification_as_done(&self, id: &str) -> Result<()> {
        self.mark_notification_as_read(id).await
    }

    async fn mark_all_notifications_as_read(&self) -> Result<()> {
        self.run_glab(&["todo", "done", "--all"], "Marking All Todos Done")
            .await?;
        Ok(())
    }

    async fn unsubscribe_from_notification(&self, notification: &Notification) -> Result<()> {
        let entity = match notification.target() {
            Some(NotificationTarget::Issue) => "issue",
            Some(NotificationTarget::MergeRequest) => "mr",
            None => anyhow::bail!(
                "unsubscribing from a {} todo is not supported on GitLab",
                notification.target_type
            ),
        };
        let iid = notification.target_iid.to_string();
        self.run_glab(
            &[
                entity,
                "unsubscribe",
                &iid,
                "-R",
                &notification.project_path,
            ],
            "Unsubscribing",
        )
        .await?;
        Ok(())
    }

    async fn list_branches(&self, scope: &Scope, page_size: usize) -> Result<Vec<Branch>> {
        let project = match scope {
            Scope::Repository(p) => p,
            Scope::Group(_) => return Err(anyhow::anyhow!("Group-level branches not supported")),
        };
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{encoded}/repository/branches?per_page={page_size}");
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
            "/projects/{encoded}/repository/branches?branch={branch_name}&ref={ref_branch}"
        );
        self.raw_api(&endpoint, "POST", None, "Creating Branch")
            .await?;
        Ok(())
    }

    async fn delete_branch(&self, project: &str, branch_name: &str) -> Result<()> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{encoded}/repository/branches/{branch_name}");
        self.raw_api(&endpoint, "DELETE", None, "Deleting Branch")
            .await?;
        Ok(())
    }

    // ── Environments / Deployments ──

    async fn list_environments(&self, scope: &Scope, page_size: usize) -> Result<Vec<Environment>> {
        let project = match scope {
            Scope::Repository(p) => p,
            Scope::Group(_) => {
                return Err(anyhow::anyhow!("Group-level environments not supported"));
            }
        };
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{encoded}/environments?per_page={page_size}");
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
        scope: &Scope,
        page_size: usize,
        environment: Option<&str>,
    ) -> Result<Vec<Deployment>> {
        let project = match scope {
            Scope::Repository(p) => p,
            Scope::Group(_) => {
                return Err(anyhow::anyhow!("Group-level deployments not supported"));
            }
        };
        let encoded = Self::encode_path(project);
        let mut endpoint =
            format!("/projects/{encoded}/deployments?per_page={page_size}&order_by=id&sort=desc");
        if let Some(env) = environment {
            endpoint.push_str(&format!("&environment={env}"));
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

    async fn stop_environment(&self, project: &str, environment: &Environment) -> Result<()> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{encoded}/environments/{}/stop", environment.id);
        self.raw_api(&endpoint, "POST", None, "Stopping Environment")
            .await
            .map(|_| ())
    }

    async fn rollback_deployment(
        &self,
        project: &str,
        _environment: &str,
        deployment: &Deployment,
    ) -> Result<()> {
        let job_id = deployment.job_id().ok_or_else(|| {
            anyhow::anyhow!("deployment #{} has no deploy job to re-run", deployment.iid)
        })?;
        self.run_glab(
            &["ci", "retry", &job_id.to_string(), "-R", project],
            "Rolling Back Deployment",
        )
        .await
        .map(|_| ())
    }

    // ── Labels / Members / Misc ──

    async fn fetch_labels(&self, scope: &Scope, per_request: usize) -> Result<Vec<Label>> {
        let raw = match scope {
            Scope::Repository(project) => {
                self.run_glab(
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
                .await?
            }
            Scope::Group(group) => {
                let encoded = Self::encode_path(group);
                self.run_glab(
                    &[
                        "api",
                        &format!("groups/{encoded}/labels?per_page={per_request}"),
                    ],
                    "Fetching Group Labels",
                )
                .await?
            }
        };
        #[derive(Deserialize)]
        struct GiLabel {
            name: String,
            color: Option<String>,
        }
        let labels: Vec<GiLabel> = serde_json::from_str(&raw)?;
        Ok(labels
            .into_iter()
            .map(|l| Label {
                name: l.name,
                color: l.color.map(|c| c.trim_start_matches('#').to_string()),
            })
            .collect())
    }

    async fn fetch_members(&self, scope: &Scope) -> Result<Vec<String>> {
        let endpoint = match scope {
            Scope::Repository(project) => {
                let encoded = Self::encode_path(project);
                format!("/projects/{encoded}/members/all?per_page=100")
            }
            Scope::Group(group) => {
                let encoded = Self::encode_path(group);
                format!("/groups/{encoded}/members/all?per_page=100")
            }
        };
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

    async fn list_group_projects(
        &self,
        group: &str,
        page_size: usize,
        per_request: usize,
    ) -> Result<Vec<String>> {
        let encoded = Self::encode_path(group);
        let per = per_request.clamp(1, 100);
        let pages = page_size.div_ceil(per);
        let mut out = Vec::new();
        for page in 1..=pages {
            let endpoint = format!(
                "groups/{encoded}/projects?include_subgroups=true&per_page={per}&page={page}&archived=false"
            );
            let raw = self
                .raw_api(&endpoint, "GET", None, "Fetching Group Projects")
                .await?;
            let batch = parse_group_projects(&raw)?;
            let n = batch.len();
            out.extend(batch);
            if out.len() >= page_size || n < per {
                break;
            }
        }
        out.truncate(page_size);
        Ok(out)
    }

    async fn open_in_browser(&self, project: &str, entity: &str, id: &str) -> Result<()> {
        if !project.is_empty() {
            self.run_glab(
                &[entity, "view", id, "-R", project, "-w"],
                "OPENING IN BROWSER",
            )
            .await?;
        } else {
            self.run_glab(&[entity, "view", id, "-w"], "OPENING IN BROWSER")
                .await?;
        }
        Ok(())
    }

    async fn open_pipeline_in_browser(&self, project: &str, id: &str) -> Result<()> {
        if !project.is_empty() {
            self.run_glab(
                &["ci", "view", "-p", id, "-R", project, "-w"],
                "OPENING IN BROWSER",
            )
            .await?;
        } else {
            self.run_glab(&["ci", "view", "-p", id, "-w"], "OPENING IN BROWSER")
                .await?;
        }
        Ok(())
    }

    async fn open_workflow_in_browser(&self, project: &str, workflow: &str) -> Result<()> {
        self.run_glab(
            &["ci", "view", workflow, "-R", project, "-w"],
            "OPENING PIPELINE IN BROWSER",
        )
        .await?;
        Ok(())
    }

    async fn open_job_in_browser(&self, project: &str, id: &str) -> Result<()> {
        if !project.is_empty() {
            self.run_glab(
                &["job", "view", id, "-R", project, "-w"],
                "OPENING IN BROWSER",
            )
            .await?;
        } else {
            self.run_glab(&["job", "view", id, "-w"], "OPENING IN BROWSER")
                .await?;
        }
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

    async fn open_runner_in_browser(&self, scope: &Scope, runner_id: u64) -> Result<()> {
        let host = gitlab_host_or_default();
        let url = gitlab_runner_url(&host, scope, runner_id);
        open_in_web_browser(&url, self.tx.clone(), "OPENING RUNNER IN BROWSER").await
    }

    async fn open_branch_in_browser(&self, project: &str, branch: &str) -> Result<()> {
        if project.is_empty() {
            anyhow::bail!("project path required to open branch in browser");
        }
        let host = gitlab_host_or_default();
        let url = format!(
            "https://{}/{}/-/tree/{}",
            host,
            project,
            url_encode_branch(branch),
        );
        open_in_web_browser(&url, self.tx.clone(), "OPENING BRANCH IN BROWSER").await
    }

    async fn open_environment_in_browser(&self, project: &str, name: &str) -> Result<()> {
        if project.is_empty() {
            anyhow::bail!("project path required to open environment in browser");
        }
        let host = gitlab_host_or_default();
        let url = format!(
            "https://{}/{}/-/environments/{}",
            host,
            project,
            url_encode_branch(name),
        );
        open_in_web_browser(&url, self.tx.clone(), "OPENING ENVIRONMENT IN BROWSER").await
    }
    // ── Raw API ──

    async fn raw_api(
        &self,
        endpoint: &str,
        method: &str,
        body: Option<&str>,
        desc: &str,
    ) -> Result<String> {
        let tx = self.tx.clone();
        let endpoint = endpoint.to_string();
        let method = method.to_string();
        let body = body.map(ToString::to_string);
        let desc = desc.to_string();
        super::rate_limit::execute_with_retry(|| {
            let tx = tx.clone();
            let endpoint = endpoint.clone();
            let method = method.clone();
            let body = body.clone();
            let desc = desc.clone();
            async move { run_glab_raw_api(tx, &endpoint, &method, body.as_deref(), &desc).await }
        })
        .await
    }
}

async fn run_glab_raw_api(
    tx: Option<UnboundedSender<Event>>,
    endpoint: &str,
    method: &str,
    body: Option<&str>,
    desc: &str,
) -> Result<String> {
    let input = body.filter(|b| !b.is_empty());
    let mut args = vec!["api"];
    if method != "GET" {
        args.extend(["-X", method]);
    }
    if input.is_some() {
        args.extend(["--input", "-"]);
    }
    args.push(endpoint);
    let logged = LoggedCommand::start(
        desc,
        Invocation::new(Program::Glab, &args).with_stdin(input),
    );

    let output =
        super::subprocess::output(Command::new("glab").args(&args), input.map(str::as_bytes)).await;

    match output {
        Ok(out) => {
            if out.status.success() {
                logged.finish(tx.as_ref(), "Success".to_string(), (&out).into());
                Ok(String::from_utf8(out.stdout)?)
            } else {
                let child_output = ChildOutput::from(&out);
                let reason = logged.failure_reason(&child_output);
                logged.finish(tx.as_ref(), format!("Failed: {reason}"), child_output);
                anyhow::bail!("glab api failed: {reason}")
            }
        }
        Err(e) => {
            logged.finish(
                tx.as_ref(),
                format!("Failed: {e}"),
                ChildOutput::NOT_STARTED,
            );
            Err(e)
        }
    }
}

/// The GitLab instance assumed when `git remote get-url origin` cannot name
/// one: no remote is configured, or its URL has no host.
const DEFAULT_GITLAB_HOST: &str = "gitlab.com";

/// At most this many runners per GraphQL details query, matching the single
/// `glab runner list --per-page <page_size>` call it follows.
const RUNNER_DETAILS_QUERY_MAX: usize = 100;

/// Resolve the GitLab host (e.g. `gitlab.com`, `gitlab.example.com`) by
/// reading `git remote get-url origin` and stripping the path, falling back
/// to `DEFAULT_GITLAB_HOST` when there is no remote to read. Used to
/// construct browser URLs for entities whose `glab` subcommand doesn't
/// ship a `-w` flag (runners, branches, environments).
fn gitlab_host_or_default() -> String {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok();
    let host = output
        .and_then(|out| out.status.success().then_some(out))
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|url| crate::git_helpers::parse_remote_host(&url));
    host.unwrap_or_else(|| DEFAULT_GITLAB_HOST.to_string())
}

/// Percent-encode a branch or environment name for use in a GitLab URL
/// path segment. We keep `/` as `%2F` (so `feature/foo` becomes
/// `feature%2Ffoo`) and pass through everything else verbatim — GitLab
/// URLs do their own escaping of `%` and other reserved characters at
/// the edge, so we avoid double-encoding.
fn url_encode_branch(name: &str) -> String {
    name.replace('/', "%2F")
}

/// The runner's settings page on `host`, under the project or group that
/// `scope` names.
fn gitlab_runner_url(host: &str, scope: &Scope, runner_id: u64) -> String {
    match scope {
        Scope::Repository(project) => format!("https://{host}/{project}/-/runners/{runner_id}"),
        Scope::Group(group) => format!("https://{host}/groups/{group}/-/runners/{runner_id}"),
    }
}

/// Open `url` in the user's default browser via `git web--browse`, which
/// honours `$BROWSER` and falls back to xdg-open / open / wslview. Logs
/// the command to the Terminal tab so it shows up next to the `glab`
/// and `gh` commands. Returns Ok even if the browser helper exits
/// non-zero — opening a URL should never block the user's flow.
async fn open_in_web_browser(
    url: &str,
    tx: Option<tokio::sync::mpsc::UnboundedSender<crate::event::Event>>,
    label: &str,
) -> Result<()> {
    let logged = LoggedCommand::start(label, Invocation::new(Program::Git, &["web--browse", url]));
    let output = tokio::process::Command::new("git")
        .args(["web--browse", url])
        .output()
        .await;
    let (status, child_output) = match &output {
        Ok(out) if out.status.success() => ("Success".to_string(), ChildOutput::from(out)),
        Ok(out) => {
            let child_output = ChildOutput::from(out);
            let reason = logged.failure_reason(&child_output);
            (format!("Failed: {reason}"), child_output)
        }
        Err(error) => (format!("Failed: {error}"), ChildOutput::NOT_STARTED),
    };
    logged.finish(tx.as_ref(), status, child_output);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff_refs() -> GiDiffRefs {
        GiDiffRefs {
            base_sha: "base".to_string(),
            start_sha: "start".to_string(),
            head_sha: "head".to_string(),
        }
    }

    fn draft(
        line: Option<u32>,
        old: Option<u32>,
        end: Option<u32>,
        end_old: Option<u32>,
    ) -> DraftComment {
        DraftComment {
            file_path: "src/lib.rs".to_string(),
            line_num: line,
            old_line_num: old,
            end_line_num: end,
            end_old_line_num: end_old,
            body: "note".to_string(),
        }
    }

    #[test]
    fn draft_note_on_unchanged_line_carries_both_lines() {
        let payload =
            gitlab_draft_note_payload(&draft(Some(12), Some(10), None, None), &diff_refs());
        let position = &payload["position"];
        assert_eq!(
            (position["new_line"].as_u64(), position["old_line"].as_u64()),
            (Some(12), Some(10))
        );
        assert_eq!(position["head_sha"], "head");
        assert!(position.get("line_range").is_none());
    }

    #[test]
    fn draft_note_range_on_old_side_uses_old_line_range() {
        let payload = gitlab_draft_note_payload(&draft(None, Some(9), None, Some(5)), &diff_refs());
        let range = &payload["position"]["line_range"];
        assert_eq!(range["start"]["type"], "old_line");
        assert_eq!(
            (
                range["start"]["old_line"].as_u64(),
                range["end"]["old_line"].as_u64()
            ),
            (Some(5), Some(9))
        );
    }

    #[test]
    fn url_encode_branch_escapes_slash() {
        assert_eq!(url_encode_branch("main"), "main");
        assert_eq!(url_encode_branch("feature/foo"), "feature%2Ffoo");
        assert_eq!(url_encode_branch("a/b/c"), "a%2Fb%2Fc");
    }

    #[test]
    fn gitlab_host_without_origin_remote_falls_back_to_gitlab_com() {
        // No upstream remote configured in CI/sandbox; assert graceful
        // fallback rather than a panic.
        std::env::set_current_dir("/").ok();
        assert_eq!(gitlab_host_or_default(), "gitlab.com");
    }

    #[test]
    fn gitlab_issue_json_produces_a_copyable_reference() {
        let raw = r#"{
            "iid": 42,
            "title": "Fix parser",
            "state": "opened",
            "labels": ["bug"],
            "updated_at": "2026-08-29T11:00:00Z",
            "created_at": "2026-08-29T10:00:00Z",
            "closed_at": null,
            "author": {"username": "octocat"},
            "milestone": null,
            "assignees": [],
            "description": "Details",
            "due_date": null,
            "web_url": "https://gitlab.com/acme/project/-/issues/42"
        }"#;

        let issue = parse_glab_issue(raw).unwrap();

        assert_eq!(
            issue.markdown_reference(),
            "[#42: Fix parser](https://gitlab.com/acme/project/-/issues/42)"
        );
    }

    #[test]
    fn gitlab_issue_list_json_keeps_each_copyable_url() {
        let raw = r#"[{
            "iid": 42,
            "title": "Fix parser",
            "state": "opened",
            "labels": [],
            "updated_at": "2026-08-29T11:00:00Z",
            "created_at": "2026-08-29T10:00:00Z",
            "closed_at": null,
            "author": {"username": "octocat"},
            "milestone": null,
            "assignees": [],
            "description": null,
            "due_date": null,
            "web_url": "https://gitlab.com/acme/project/-/issues/42"
        }]"#;

        let issues = parse_glab_issues(raw).unwrap();

        assert_eq!(issues.len(), 1);
        assert_eq!(
            issues[0].markdown_reference(),
            "[#42: Fix parser](https://gitlab.com/acme/project/-/issues/42)"
        );
    }

    #[test]
    fn parse_closed_by_mr_refs() {
        let raw = r#"[
            { "iid": 1471, "title": "wire up webhooks", "state": "opened" },
            { "iid": 1502, "title": "fix closing flow",   "state": "merged" }
        ]"#;
        let parsed: Vec<GiClosedBy> = serde_json::from_str(raw).unwrap();
        let refs: Vec<RelatedMrRef> = parsed
            .into_iter()
            .map(|m| RelatedMrRef {
                iid: m.iid,
                title: m.title,
                state: m.state,
                project_path: None,
            })
            .collect();
        assert_eq!(
            refs,
            vec![
                RelatedMrRef {
                    iid: 1471,
                    title: "wire up webhooks".into(),
                    state: "opened".into(),
                    project_path: None,
                },
                RelatedMrRef {
                    iid: 1502,
                    title: "fix closing flow".into(),
                    state: "merged".into(),
                    project_path: None,
                },
            ]
        );
    }

    #[test]
    fn parse_empty_closed_by_returns_empty_vec() {
        let raw = "[]";
        let parsed: Vec<GiClosedBy> = serde_json::from_str(raw).unwrap();
        let refs: Vec<RelatedMrRef> = parsed
            .into_iter()
            .map(|m| RelatedMrRef {
                iid: m.iid,
                title: m.title,
                state: m.state,
                project_path: None,
            })
            .collect();
        assert!(refs.is_empty());
    }

    #[test]
    fn merge_args_skip_confirmation_prompt() {
        let args = GlabBackend::merge_args("group/project", 42, true, true, None, false, None);

        assert_eq!(
            args,
            vec![
                "mr",
                "merge",
                "42",
                "-R",
                "group/project",
                "--squash",
                "--remove-source-branch",
                // No --auto-merge flag when auto_merge=false; glab decides based on
                // pipeline state, avoiding "All attempts fail" for in-progress pipelines.
                "--yes",
            ]
        );
        // Explicitly verify --auto-merge=false is never injected.
        assert!(!args.contains(&"--auto-merge=false".to_string()));
    }

    #[test]
    fn merge_args_auto_merge_true_adds_flag() {
        let args = GlabBackend::merge_args("group/project", 42, false, false, None, true, None);
        assert!(args.contains(&"--auto-merge=true".to_string()));
        assert!(!args.contains(&"--auto-merge=false".to_string()));
    }

    #[test]
    fn merge_args_auto_merge_false_omits_flag() {
        // When the user does not check "Auto-merge", we must not pass
        // --auto-merge=false because that forces glab to attempt an immediate
        // merge even when a pipeline is still running, causing it to exhaust
        // its internal retry loop with "All attempts fail" (#372).
        let args = GlabBackend::merge_args("group/project", 42, false, false, None, false, None);
        assert!(!args.iter().any(|a| a.starts_with("--auto-merge")));
    }

    #[test]
    fn merge_args_rebase_strategy_adds_rebase_flag() {
        let args = GlabBackend::merge_args(
            "group/project",
            42,
            false,
            false,
            Some("rebase"),
            false,
            None,
        );
        assert!(args.contains(&"--rebase".to_string()));
        assert!(!args.contains(&"--squash".to_string()));
    }

    #[test]
    fn merge_args_merge_strategy_is_default() {
        let args = GlabBackend::merge_args(
            "group/project",
            42,
            false,
            false,
            Some("merge"),
            false,
            None,
        );
        assert!(!args.contains(&"--rebase".to_string()));
        assert!(!args.contains(&"--squash".to_string()));
    }

    #[test]
    fn merge_args_passes_sha_when_provided() {
        // GitLab 19.2+ and any group with the "Require a commit SHA when
        // merging" setting reject merges without --sha
        // (`400 SHA must be provided when merging`). The flag must be present
        // and carry the exact SHA the caller supplied (#470).
        let args = GlabBackend::merge_args(
            "group/project",
            42,
            false,
            false,
            None,
            false,
            Some("b968d3bc753974a475d5475b7ca46639f9ec3cba"),
        );
        assert!(args.contains(&"--sha=b968d3bc753974a475d5475b7ca46639f9ec3cba".to_string()));
    }

    #[test]
    fn merge_args_omits_sha_flag_when_none() {
        // Cached MRs that pre-date this field deserialize without a SHA.
        // We must not invent or pass an empty --sha; instead omit the flag
        // and let glab behave as it did pre-19.2.
        let args = GlabBackend::merge_args("group/project", 42, false, false, None, false, None);
        assert!(!args.iter().any(|a| a.starts_with("--sha")));
    }

    #[test]
    fn download_artifact_args_keep_dash_prefixed_ref_and_job_names_positional() {
        let args = GlabBackend::download_artifact_args("group/project", "-R", "--help");
        assert_eq!(
            args,
            vec![
                "job",
                "artifact",
                "-R",
                "group/project",
                "--",
                "-R",
                "--help"
            ]
        );
    }

    #[test]
    fn download_artifact_args_without_project_still_end_options_before_names() {
        let args = GlabBackend::download_artifact_args("", "--repo=evil/project", "-p");
        assert_eq!(
            args,
            vec!["job", "artifact", "--", "--repo=evil/project", "-p"]
        );
    }

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
        for bad in [
            "a\"b",
            "a{b",
            "a}b",
            "a\\b",
            "a\nb",
            "a b",
            "",
            "singlegroup",
            "group/",
            "/project",
            "group//project",
            "a/b/c/",
        ] {
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
    fn runner_details_come_from_the_newest_manager() {
        let body = r#"{"data":{"project":{"runners":{"nodes":[
            {"id":"gid://gitlab/Ci::Runner/9876","tagList":["docker","linux"],
             "contactedAt":"2026-01-02T03:04:05.000Z",
             "managers":{"nodes":[
                {"version":"17.3.1","ipAddress":"10.0.0.8","contactedAt":"2025-12-31T00:00:00.000Z"},
                {"version":"17.4.0","ipAddress":"10.0.0.7","contactedAt":"2026-01-02T03:04:05.000Z"}]}},
            {"id":"gid://gitlab/Ci::Runner/9877","tagList":["macos"],
             "contactedAt":null,
             "managers":{"nodes":[{"version":"","ipAddress":"","contactedAt":null}]}}
        ]}}}}"#;
        let details = parse_runner_details_response(body).unwrap();

        let first = details.get(&9876).unwrap();
        assert_eq!(first.tag_list, ["docker", "linux"]);
        assert_eq!(
            first.contacted_at.as_deref(),
            Some("2026-01-02T03:04:05.000Z")
        );
        assert_eq!(first.version.as_deref(), Some("17.4.0"));
        assert_eq!(first.ip_address.as_deref(), Some("10.0.0.7"));

        let second = details.get(&9877).unwrap();
        assert_eq!(second.tag_list, ["macos"]);
        assert_eq!(second.contacted_at, None);
        assert_eq!(second.version, None);
        assert_eq!(second.ip_address, None);
    }

    #[test]
    fn runner_details_graphql_errors_and_a_malformed_gid_yield_none_or_skip() {
        let errors = r#"{"errors":[{"message":"Field 'managers' doesn't exist"}]}"#;
        assert!(parse_runner_details_response(errors).is_none());

        let malformed = r#"{"data":{"project":{"runners":{"nodes":[
            {"id":"not-a-gid","tagList":[]},
            {"id":"gid://gitlab/Ci::Runner/9876","tagList":["docker"]}
        ]}}}}"#;
        let details = parse_runner_details_response(malformed).unwrap();
        assert!(!details.contains_key(&0));
        assert_eq!(details.get(&9876).unwrap().tag_list, ["docker"]);
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

    #[test]
    fn test_parse_group_projects() {
        let json = r#"[
            {"path_with_namespace": "mygroup/proj1"},
            {"path_with_namespace": "mygroup/subgroup/proj2"}
        ]"#;
        let paths = parse_group_projects(json).unwrap();
        assert_eq!(paths, vec!["mygroup/proj1", "mygroup/subgroup/proj2"]);

        // Empty array
        let empty_paths = parse_group_projects("[]").unwrap();
        assert!(empty_paths.is_empty());

        // Malformed / missing field
        assert!(parse_group_projects(r#"[{"id": 1}]"#).is_err());
    }

    /// `len` bytes of numbered lines, so no two stretches of it are equal.
    fn log_of(len: usize) -> Vec<u8> {
        (0..)
            .flat_map(|line: usize| format!("{line:07}\n").into_bytes())
            .take(len)
            .collect()
    }

    fn reply(bytes: &[u8], is_truncated: bool) -> CapturedStdout {
        CapturedStdout {
            bytes: bytes.to_vec(),
            is_truncated,
        }
    }

    /// A 5000-byte log, read up to byte 3000, and where a resumed read starts.
    fn log_read_to_3000() -> (Vec<u8>, TraceCursor, usize) {
        let log = log_of(5000);
        let cursor = JobTrace::from_read(&log[..3000], false).cursor().clone();
        let resume = cursor.resume_offset();
        (log, cursor, resume)
    }

    #[test]
    fn honoured_reply_filling_the_limit_is_a_chunk_with_more_to_follow() {
        let (log, cursor, resume) = log_read_to_3000();
        let reply = reply(&log[resume..resume + 1500], false);

        assert_eq!(
            classify_ranged_reply(&cursor, &reply, 1500),
            RangedReply::Chunk(&log[3000..resume + 1500])
        );
    }

    #[test]
    fn honoured_reply_short_of_the_limit_is_the_last_chunk() {
        let (log, cursor, resume) = log_read_to_3000();
        let reply = reply(&log[resume..], false);

        assert_eq!(
            classify_ranged_reply(&cursor, &reply, GITLAB_TRACE_RANGE_MAX_BYTES),
            RangedReply::Last(&log[3000..])
        );
    }

    #[test]
    fn honoured_reply_holding_only_the_overlap_is_an_empty_last_chunk() {
        let (log, cursor, resume) = log_read_to_3000();
        let reply = reply(&log[resume..3000], false);

        assert_eq!(
            classify_ranged_reply(&cursor, &reply, GITLAB_TRACE_RANGE_MAX_BYTES),
            RangedReply::Last(&[])
        );
    }

    #[test]
    fn whole_log_sent_for_a_ranged_read_yields_what_follows_the_read_part() {
        let (log, cursor, _) = log_read_to_3000();
        let reply = reply(&log, false);

        assert_eq!(
            classify_ranged_reply(&cursor, &reply, GITLAB_TRACE_RANGE_MAX_BYTES),
            RangedReply::WholeLog(&log[3000..])
        );
    }

    #[test]
    fn reply_over_the_limit_is_oversized_even_when_it_lines_up() {
        let (log, cursor, resume) = log_read_to_3000();
        let reply = reply(&log[resume..resume + 1500], true);

        assert_eq!(
            classify_ranged_reply(&cursor, &reply, 1500),
            RangedReply::Oversized
        );
    }

    #[test]
    fn reply_matching_neither_read_is_misaligned() {
        let (_, cursor, _) = log_read_to_3000();
        let reply = reply(&[b'z'; 5000], false);

        assert_eq!(
            classify_ranged_reply(&cursor, &reply, GITLAB_TRACE_RANGE_MAX_BYTES),
            RangedReply::Misaligned
        );
    }

    #[test]
    fn empty_reply_after_the_log_shrank_is_misaligned() {
        let (_, cursor, _) = log_read_to_3000();
        let reply = reply(b"", false);

        assert_eq!(
            classify_ranged_reply(&cursor, &reply, GITLAB_TRACE_RANGE_MAX_BYTES),
            RangedReply::Misaligned
        );
    }

    #[test]
    fn mr_discussions_flatten_into_notes_tagged_with_their_discussion() {
        let raw = r#"[
            {"id":"6a9c1750","individual_note":false,"notes":[
                {"id":1,"type":"DiffNote","body":"Rename this","author":{"id":7,"username":"alice"},
                 "created_at":"2026-07-06T09:00:00.000Z","system":false,
                 "position":{"position_type":"text","new_path":"src/a.rs","old_path":"src/a.rs","new_line":12},
                 "resolvable":true,"resolved":true},
                {"id":2,"type":"DiffNote","body":"Done","author":{"id":8,"username":"bob"},
                 "created_at":"2026-07-06T10:00:00.000Z","system":false,
                 "position":{"position_type":"text","new_path":"src/a.rs","old_path":"src/a.rs","new_line":12},
                 "resolvable":true,"resolved":true}
            ]},
            {"id":"f00dfeed","individual_note":true,"notes":[
                {"id":3,"type":null,"body":"added 1 commit","author":{"id":8,"username":"bob"},
                 "created_at":"2026-07-06T11:00:00.000Z","system":true,"position":null,
                 "resolvable":false,"resolved":false}
            ]}
        ]"#;
        let notes = parse_mr_discussions(raw).unwrap();
        let summary: Vec<_> = notes
            .iter()
            .map(|n| {
                (
                    n.id,
                    n.discussion_id.as_deref(),
                    n.author.username.as_str(),
                    n.system,
                    n.position.as_ref().and_then(|p| p.new_line),
                    n.resolved,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (1, Some("6a9c1750"), "alice", false, Some(12), Some(true)),
                (2, Some("6a9c1750"), "bob", false, Some(12), Some(true)),
                (3, Some("f00dfeed"), "bob", true, None, Some(false)),
            ]
        );
    }
}
