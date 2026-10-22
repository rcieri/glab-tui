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
use crate::domain::mr::{
    DiscussionNote, MR_DIFF_MAX_BYTES, MergeRequest, NotePosition, ensure_complete_diff,
};
use crate::domain::notifications::{Notification, NotificationDetail};
use crate::domain::pipelines::{CommitSummary, Job, Pipeline};
use crate::domain::releases::Release;
use crate::domain::review::{DraftComment, ReviewEvent};
use crate::domain::runners::Runner;
use crate::domain::state_query::StateQuery;
use crate::event::Event;
use crate::scope::Scope;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

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

#[derive(Deserialize)]
struct GhRepo {
    full_name: String,
}

pub fn parse_group_projects(raw: &str) -> Result<Vec<String>> {
    let repos: Vec<GhRepo> = serde_json::from_str(raw)?;
    Ok(repos.into_iter().map(|r| r.full_name).collect())
}

#[derive(Deserialize)]
struct GhIssueJson {
    number: u64,
    title: String,
    state: String,
    url: String,
    #[serde(default)]
    labels: Vec<serde_json::Value>,
    author: Option<GhIssueLogin>,
    body: Option<String>,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    #[serde(rename = "closedAt")]
    closed_at: Option<String>,
    milestone: Option<GhIssueMilestone>,
    #[serde(default)]
    assignees: Vec<GhIssueLogin>,
    #[serde(rename = "closedByPullRequestsReferences", default)]
    closed_by_pull_requests_references: Option<Vec<GhIssuePrRef>>,
}

#[derive(Deserialize)]
struct GhIssueRepoRef {
    name: String,
    owner: GhIssueLogin,
}

#[derive(Deserialize)]
struct GhIssuePrRef {
    number: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    repository: Option<GhIssueRepoRef>,
}

#[derive(Deserialize)]
struct GhIssueLogin {
    login: String,
}

#[derive(Deserialize)]
struct GhIssueMilestone {
    title: String,
    number: u64,
    #[serde(default)]
    state: Option<String>,
}

fn issue_from_gh_json(issue: GhIssueJson) -> Issue {
    let state = if issue.state == "OPEN" {
        "opened"
    } else {
        "closed"
    }
    .to_string();
    let labels = issue
        .labels
        .iter()
        .filter_map(|value| value.get("name")?.as_str().map(String::from))
        .collect();

    let related_mrs = issue.closed_by_pull_requests_references.map(|refs| {
        if refs.is_empty() {
            crate::domain::issues::RelatedMrsState::Empty
        } else {
            crate::domain::issues::RelatedMrsState::Items(
                refs.into_iter()
                    .map(|pr| {
                        let project_path = pr
                            .repository
                            .map(|r| format!("{}/{}", r.owner.login, r.name));
                        crate::domain::issues::RelatedMrRef {
                            iid: pr.number,
                            title: pr.title.unwrap_or_default(),
                            state: pr.state.unwrap_or_default(),
                            project_path,
                        }
                    })
                    .collect(),
            )
        }
    });

    Issue {
        iid: issue.number,
        title: issue.title,
        state,
        labels,
        updated_at: issue.updated_at,
        created_at: Some(issue.created_at),
        closed_at: issue.closed_at,
        author: crate::domain::issues::Author {
            username: issue.author.map(|author| author.login).unwrap_or_default(),
        },
        milestone: issue
            .milestone
            .map(|milestone| crate::domain::issues::Milestone {
                title: milestone.title,
                iid: milestone.number,
                id: 0,
                state: milestone.state.unwrap_or_default(),
            }),
        assignees: issue
            .assignees
            .into_iter()
            .map(|assignee| crate::domain::issues::Assignee {
                username: assignee.login,
            })
            .collect(),
        description: issue.body,
        due_date: None,
        web_url: issue.url,
        project_path: String::new(),
        related_mrs,
    }
}

fn parse_gh_issue(raw: &str) -> Result<Issue> {
    Ok(issue_from_gh_json(serde_json::from_str(raw)?))
}

fn parse_gh_issues(raw: &str) -> Result<Vec<Issue>> {
    Ok(serde_json::from_str::<Vec<GhIssueJson>>(raw)?
        .into_iter()
        .map(issue_from_gh_json)
        .collect())
}

/// Qualifies a PR head ref as `owner:branch` when the target repository is not
/// the one `origin` points at, which is what GitHub requires for a pull request
/// opened from a fork. Anything it cannot qualify is passed through untouched.
fn qualify_head_ref(local_project: &str, target_project: &str, branch: &str) -> String {
    if branch.contains(':') || local_project == target_project {
        return branch.to_string();
    }
    match local_project.split_once('/') {
        Some((owner, _)) if !owner.is_empty() => format!("{owner}:{branch}"),
        _ => branch.to_string(),
    }
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

/// The `repos/…` path of a GitHub API URL. GitHub Enterprise serves the API
/// under `/api/v3`, so the host part is dropped rather than parsed.
fn github_api_path(url: &str) -> Option<String> {
    url.find("/repos/")
        .map(|start| url[start + 1..].to_string())
}

/// The `gh issue view` / `gh pr view` arguments that read the issue or PR an
/// API path names. Comments, releases, discussions and commits have no
/// native command.
fn native_detail_args(detail_api_path: &str) -> Option<[&str; 7]> {
    let path = detail_api_path.strip_prefix("repos/")?;
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let name = parts.next()?;
    let command = match parts.next()? {
        "issues" => "issue",
        "pulls" => "pr",
        _ => return None,
    };
    let number = parts.next()?;
    if parts.next().is_some() || number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let repo = &path[..owner.len() + 1 + name.len()];
    Some([command, "view", number, "-R", repo, "--json", "body,author"])
}

fn parse_github_notifications(raw: &str) -> Result<Vec<Notification>> {
    #[derive(Deserialize)]
    struct GhNotif {
        id: String,
        repository: GhNotifRepo,
        subject: GhNotifSubject,
        unread: bool,
        updated_at: String,
        #[serde(default)]
        reason: Option<String>,
    }
    #[derive(Deserialize)]
    struct GhNotifRepo {
        full_name: String,
        #[serde(default)]
        html_url: Option<String>,
    }
    #[derive(Deserialize)]
    struct GhNotifSubject {
        title: String,
        r#type: String,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        latest_comment_url: Option<String>,
    }
    let gh_notifs: Vec<GhNotif> = serde_json::from_str(raw)?;
    Ok(gh_notifs
        .into_iter()
        .map(|item| {
            let subject_url = item.subject.url.unwrap_or_default();
            let target_iid = subject_url
                .split('/')
                .next_back()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let repo_url = item
                .repository
                .html_url
                .unwrap_or_else(|| format!("https://github.com/{}", item.repository.full_name));
            let (target_type, web_url) = match item.subject.r#type.as_str() {
                "PullRequest" => (
                    "MergeRequest".to_string(),
                    format!("{repo_url}/pull/{target_iid}"),
                ),
                "Issue" => (
                    "Issue".to_string(),
                    format!("{repo_url}/issues/{target_iid}"),
                ),
                _ => (item.subject.r#type, repo_url),
            };
            let detail_api_path = item
                .subject
                .latest_comment_url
                .as_deref()
                .and_then(github_api_path)
                .or_else(|| github_api_path(&subject_url));
            let state = if item.unread { "unread" } else { "read" };
            Notification {
                id: item.id,
                project_path: item.repository.full_name,
                title: item.subject.title,
                target_type,
                target_iid,
                state: state.to_string(),
                updated_at: item.updated_at,
                author: String::new(),
                reason: item.reason.unwrap_or_default(),
                web_url,
                body: None,
                detail_api_path,
            }
        })
        .collect())
}

/// Reads the comment, issue, PR or release a notification's
/// `detail_api_path` points at. The API names a comment's author `user`,
/// a release's `author`, and `gh issue view` / `gh pr view` use `author`.
fn parse_github_notification_detail(raw: &str) -> Result<NotificationDetail> {
    #[derive(Deserialize)]
    struct GhUser {
        login: String,
    }
    #[derive(Deserialize)]
    struct GhDetail {
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        user: Option<GhUser>,
        #[serde(default)]
        author: Option<GhUser>,
    }
    let detail: GhDetail = serde_json::from_str(raw).context("parsing notification detail")?;
    Ok(NotificationDetail {
        author: detail
            .user
            .or(detail.author)
            .map(|user| user.login)
            .unwrap_or_default(),
        body: detail.body.unwrap_or_default(),
    })
}

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

/// Map GitHub's PR `state` JSON field onto the host-neutral state string.
///
/// GitHub's `gh pr list --state all` returns `"OPEN"`, `"CLOSED"`, or
/// `"MERGED"`. Only `"OPEN"` and `"MERGED"` map onto themselves; everything
/// else collapses to `"closed"`. Without the `"MERGED"` arm, the
/// downstream MRs table — and the State column filter — would render a
/// merged PR as CLOSED, hiding the badge `src/ui/tabs.rs:406` already
/// draws for `m.state == "merged"`.
pub(crate) fn map_gh_pr_state(raw: &str) -> &'static str {
    match raw {
        "OPEN" => "opened",
        "MERGED" => "merged",
        _ => "closed",
    }
}

/// `gh issue list --state` value. Issues are never merged, so a merged query
/// lists every state and leaves the State column filter to the client.
fn gh_issue_state_flag(state: StateQuery) -> &'static str {
    match state {
        StateQuery::Open => "open",
        StateQuery::Closed => "closed",
        StateQuery::Merged | StateQuery::All => "all",
    }
}

/// `gh pr list --state` value. GitHub's `closed` also returns merged PRs
/// (only `merged` returns a single state), so a CLOSED listing still relies
/// on the State column filter to hide them.
fn gh_pr_state_flag(state: StateQuery) -> &'static str {
    match state {
        StateQuery::Open => "open",
        StateQuery::Closed => "closed",
        StateQuery::Merged => "merged",
        StateQuery::All => "all",
    }
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

    /// Percent-encode every `/` in `name` so a branch like `feature/foo`
    /// becomes `feature%2Ffoo` in the URL path. GitHub's `/git/refs/heads/{}`
    /// endpoint treats `/` as a path separator, so a literal slash in the
    /// branch name produces a malformed URL and a 404/422 from the API.
    /// Mirrors `GlabBackend::encode_path` (`src/backend/glab.rs:412`).
    fn encode_branch(name: &str) -> String {
        name.replace('/', "%2F")
    }

    pub(crate) fn merge_args(
        project: &str,
        iid: u64,
        squash: bool,
        delete_branch: bool,
        strategy: Option<&str>,
        auto_merge: bool,
    ) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "pr".into(),
            "merge".into(),
            iid.to_string(),
            "-R".into(),
            project.into(),
        ];
        if squash {
            args.push("--squash".into());
        } else if matches!(strategy, Some("rebase")) {
            args.push("--rebase".into());
        } else {
            // "Strategy: Merge commit" is the default and is always sent
            // explicitly so gh pr merge doesn't silently fall back to whatever
            // the repo's default merge method happens to be.
            args.push("--merge".into());
        }
        if delete_branch {
            args.push("--delete-branch".into());
        }
        if auto_merge {
            args.push("--auto".into());
        }
        args
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
        let stdout = self.run_gh_bounded(args, desc, None).await?;
        Ok(String::from_utf8(stdout.bytes)?)
    }

    /// `run_gh` keeping at most `max_stdout_bytes` of its output.
    async fn run_gh_bounded(
        &self,
        args: &[&str],
        desc: &str,
        max_stdout_bytes: Option<usize>,
    ) -> Result<CapturedStdout> {
        let tx = self.tx.clone();
        let args = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let desc = desc.to_string();
        super::rate_limit::execute_with_retry(|| {
            let tx = tx.clone();
            let args = args.clone();
            let desc = desc.clone();
            async move {
                let args_refs: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
                run_gh_command(tx, &args_refs, &desc, max_stdout_bytes).await
            }
        })
        .await
    }

    /// Builds the diff GitHub refuses to serve (over 20,000 lines) with git,
    /// in the recently used clone whose `origin` is `project`.
    async fn diff_pr_locally(&self, project: &str, pr_number: u64) -> Result<String> {
        let lookup_project = project.to_string();
        let checkout = tokio::task::spawn_blocking(move || {
            crate::utils::cache::find_local_checkout(&lookup_project)
        })
        .await
        .context("looking up a local clone")?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "the diff of PR #{pr_number} is over GitHub's 20,000-line limit and needs a \
                 local clone of {project}; run glab-tui in a clone of {project} once, then retry"
            )
        })?;
        let raw = self
            .run_gh(
                &[
                    "pr",
                    "view",
                    &pr_number.to_string(),
                    "-R",
                    project,
                    "--json",
                    "baseRefOid,headRefOid",
                ],
                "Fetching PR Commit IDs",
            )
            .await?;
        let commits: PrDiffCommits = serde_json::from_str(&raw)
            .with_context(|| format!("parsing the commit ids of PR #{pr_number}"))?;
        diff_pr_in_checkout(self.tx.as_ref(), &checkout, pr_number, &commits).await
    }
}

async fn run_gh_command(
    tx: Option<UnboundedSender<Event>>,
    args: &[&str],
    desc: &str,
    max_stdout_bytes: Option<usize>,
) -> Result<CapturedStdout> {
    let logged = LoggedCommand::start(desc, Invocation::new(Program::Gh, args));

    let output = match output_bounded(Command::new("gh").args(args), max_stdout_bytes).await {
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
        anyhow::bail!("gh command failed: {reason}")
    }
}

/// Marker GitHub puts in the error when a PR's diff exceeds the 20,000 lines
/// it serves (`PullRequest.diff too_large`, HTTP 406).
const PR_DIFF_TOO_LARGE_MARKER: &str = "too_large";

fn is_pr_diff_too_large(err: &anyhow::Error) -> bool {
    format!("{err:#}").contains(PR_DIFF_TOO_LARGE_MARKER)
}

#[derive(Deserialize)]
struct PrDiffCommits {
    #[serde(rename = "baseRefOid")]
    base: String,
    #[serde(rename = "headRefOid")]
    head: String,
}

/// A full SHA-1 or SHA-256 object name. Anything else is refused before it
/// reaches a git command line, where it could be read as an option.
fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `git -C <checkout>` that can never prompt: the TUI owns the terminal.
fn git_in(checkout: &str) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(checkout)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null());
    command
}

async fn has_commit(checkout: &str, oid: &str) -> bool {
    git_in(checkout)
        .args(["cat-file", "-e", &format!("{oid}^{{commit}}")])
        .output()
        .await
        .is_ok_and(|output| output.status.success())
}

async fn run_git_command(
    tx: Option<&UnboundedSender<Event>>,
    checkout: &str,
    args: &[&str],
    desc: &str,
    max_stdout_bytes: Option<usize>,
) -> Result<CapturedStdout> {
    let git_args: Vec<&str> = ["-C", checkout]
        .into_iter()
        .chain(args.iter().copied())
        .collect();
    let logged = LoggedCommand::start(desc, Invocation::new(Program::Git, &git_args));
    let output = output_bounded(git_in(checkout).args(args), max_stdout_bytes)
        .await
        .with_context(|| format!("Failed to execute: {}", logged.command_line()))?;
    if output.succeeded() {
        logged.finish(tx, "Success".to_string(), output.child_output());
        Ok(output.stdout)
    } else {
        let child_output = output.child_output();
        let reason = logged.failure_reason(&child_output);
        logged.finish(tx, format!("Failed: {reason}"), child_output);
        anyhow::bail!("git command failed: {reason}")
    }
}

/// Diffs a PR with git in `checkout`, fetching its base and head commits from
/// `origin` only when the clone lacks them. Fetching without a destination
/// ref writes nothing but `FETCH_HEAD`, so the clone's branches, index and
/// working tree are left as they were.
async fn diff_pr_in_checkout(
    tx: Option<&UnboundedSender<Event>>,
    checkout: &str,
    pr_number: u64,
    commits: &PrDiffCommits,
) -> Result<String> {
    for oid in [&commits.base, &commits.head] {
        if !is_object_id(oid) {
            anyhow::bail!("unexpected commit id {oid:?} for PR #{pr_number}");
        }
    }
    if !(has_commit(checkout, &commits.base).await && has_commit(checkout, &commits.head).await) {
        let pr_head = format!("pull/{pr_number}/head");
        run_git_command(
            tx,
            checkout,
            &["fetch", "--no-tags", "origin", &commits.base, &pr_head],
            "Fetching PR Commits",
            None,
        )
        .await?;
    }
    // Pinned flags keep user diff config (colour, external drivers, textconv,
    // prefixes, octal-quoted paths) out of the text the diff view parses.
    let range = format!("{}...{}", commits.base, commits.head);
    let diff = run_git_command(
        tx,
        checkout,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            &range,
        ],
        "Diffing PR Locally",
        Some(MR_DIFF_MAX_BYTES),
    )
    .await?;
    ensure_complete_diff(diff.is_truncated, &format!("PR #{pr_number}"))?;
    // Local files can be in any encoding; the diff view only needs text.
    Ok(String::from_utf8_lossy(&diff.bytes).into_owned())
}

/// Build the GraphQL query that finds the PRs which close (or closed) an issue.
/// The template is a regular string so the `\`-newline continuations are real
/// line continuations — a raw string would emit literal backslashes and the
/// query would fail to parse server-side.
fn related_prs_graphql_query(owner: &str, repo: &str, issue_number: u64, first: usize) -> String {
    let owner = owner.replace('\\', "\\\\").replace('"', "\\\"");
    let repo = repo.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "{{ repository(owner:\"{owner}\",name:\"{repo}\") {{ issue(number:{issue_number}) {{ \
         closedByPullRequestsReferences(first:{first}) {{ \
         nodes {{ number title state }} }} }} }} }}",
    )
}

/// Serde shape returned by the `closedByPullRequestsReferences` GraphQL field.
/// Shared between the production parser and its tests so both stay in sync.
mod related_prs_types {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub(super) struct GhResponse {
        pub(super) data: Option<GhData>,
    }
    #[derive(Deserialize)]
    pub(super) struct GhData {
        pub(super) repository: Option<GhRepo>,
    }
    #[derive(Deserialize)]
    pub(super) struct GhRepo {
        pub(super) issue: Option<GhIssue>,
    }
    #[derive(Deserialize)]
    pub(super) struct GhIssue {
        #[serde(default, rename = "closedByPullRequestsReferences")]
        pub(super) closed_by_pull_requests_references: GhConn,
    }
    #[derive(Deserialize, Default)]
    pub(super) struct GhConn {
        #[serde(default)]
        pub(super) nodes: Vec<GhPrRef>,
    }
    #[derive(Deserialize)]
    pub(super) struct GhPrRef {
        pub(super) number: u64,
        pub(super) title: String,
        pub(super) state: String,
    }
}

/// Build a GraphQL query to fetch stack info for a batch of PR numbers.
fn pr_stack_graphql_query(owner: &str, repo: &str, pr_numbers: &[u64]) -> String {
    let owner = owner.replace('\\', "\\\\").replace('"', "\\\"");
    let repo = repo.replace('\\', "\\\\").replace('"', "\\\"");
    let mut fields = String::new();
    for &num in pr_numbers {
        fields.push_str(&format!(
            "pr_{num}: pullRequest(number: {num}) {{ \
             number \
             stack {{ number size }} \
             stackEntry {{ position }} \
             }} "
        ));
    }
    format!("{{ repository(owner:\"{owner}\",name:\"{repo}\") {{ {fields} }} }}")
}

mod pr_stack_types {
    use super::*;

    #[derive(Deserialize)]
    pub(super) struct GhStackBatchResponse {
        pub(super) data: Option<GhStackBatchData>,
    }

    #[derive(Deserialize)]
    pub(super) struct GhStackBatchData {
        pub(super) repository: Option<std::collections::HashMap<String, Option<GhPrStackNode>>>,
    }

    #[derive(Deserialize)]
    pub(super) struct GhPrStackNode {
        pub(super) number: Option<u64>,
        pub(super) stack: Option<GhPrStackObj>,
        #[serde(rename = "stackEntry")]
        pub(super) stack_entry: Option<GhPrStackEntryObj>,
    }

    #[derive(Deserialize)]
    pub(super) struct GhPrStackObj {
        pub(super) number: u64,
        pub(super) size: usize,
    }

    #[derive(Deserialize)]
    pub(super) struct GhPrStackEntryObj {
        pub(super) position: usize,
    }
}

fn parse_pr_stacks_response(
    raw: &str,
) -> std::collections::HashMap<u64, crate::domain::mr::StackInfo> {
    let mut map = std::collections::HashMap::new();
    let Ok(resp) = serde_json::from_str::<pr_stack_types::GhStackBatchResponse>(raw) else {
        return map;
    };
    let Some(data) = resp.data else {
        return map;
    };
    let Some(repo) = data.repository else {
        return map;
    };
    for (key, node_opt) in repo {
        if let Some(node) = node_opt {
            let pr_num = node
                .number
                .or_else(|| key.strip_prefix("pr_").and_then(|s| s.parse::<u64>().ok()));
            if let (Some(pr_num), Some(stk), Some(entry)) = (pr_num, node.stack, node.stack_entry) {
                map.insert(
                    pr_num,
                    crate::domain::mr::StackInfo {
                        number: stk.number,
                        size: stk.size,
                        position: entry.position,
                    },
                );
            }
        }
    }
    map
}

/// Build a GraphQL query to fetch a PR's stack position and all its entries.
fn pr_stack_entries_graphql_query(owner: &str, repo: &str, pr_number: u64, first: usize) -> String {
    let owner = owner.replace('\\', "\\\\").replace('"', "\\\"");
    let repo = repo.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "{{ repository(owner:\"{owner}\",name:\"{repo}\") {{ \
         pullRequest(number:{pr_number}) {{ \
         stackEntry {{ position }} \
         stack {{ \
           number \
           size \
           entries(first:{first}) {{ \
             nodes {{ \
               position \
               pullRequest {{ number title state isDraft }} \
             }} \
           }} \
         }} \
         }} }} }}"
    )
}

mod pr_stack_entries_types {
    use super::*;

    #[derive(Deserialize)]
    pub(super) struct GhStackEntriesResp {
        pub(super) data: Option<GhStackEntriesData>,
    }

    #[derive(Deserialize)]
    pub(super) struct GhStackEntriesData {
        pub(super) repository: Option<GhStackEntriesRepo>,
    }

    #[derive(Deserialize)]
    pub(super) struct GhStackEntriesRepo {
        #[serde(rename = "pullRequest")]
        pub(super) pull_request: Option<GhStackEntriesPr>,
    }

    #[derive(Deserialize)]
    pub(super) struct GhStackEntriesPr {
        #[serde(rename = "stackEntry")]
        pub(super) stack_entry: Option<pr_stack_types::GhPrStackEntryObj>,
        pub(super) stack: Option<GhStackEntriesStackObj>,
    }

    #[derive(Deserialize)]
    pub(super) struct GhStackEntriesStackObj {
        pub(super) number: u64,
        pub(super) size: usize,
        pub(super) entries: Option<GhStackEntriesNodes>,
    }

    #[derive(Deserialize)]
    pub(super) struct GhStackEntriesNodes {
        #[serde(default)]
        pub(super) nodes: Vec<GhStackEntryNode>,
    }

    #[derive(Deserialize)]
    pub(super) struct GhStackEntryNode {
        pub(super) position: usize,
        #[serde(rename = "pullRequest")]
        pub(super) pull_request: Option<GhStackEntryPrDetails>,
    }

    #[derive(Deserialize)]
    pub(super) struct GhStackEntryPrDetails {
        pub(super) number: u64,
        pub(super) title: String,
        pub(super) state: String,
        #[serde(rename = "isDraft", default)]
        pub(super) is_draft: bool,
    }
}

fn parse_pr_stack_entries_response(raw: &str) -> Result<Option<crate::domain::mr::PrStack>> {
    let Ok(resp) = serde_json::from_str::<pr_stack_entries_types::GhStackEntriesResp>(raw) else {
        return Ok(None);
    };
    let Some(data) = resp.data else {
        return Ok(None);
    };
    let Some(repo) = data.repository else {
        return Ok(None);
    };
    let Some(pr) = repo.pull_request else {
        return Ok(None);
    };
    let (Some(stack), Some(own_entry)) = (pr.stack, pr.stack_entry) else {
        return Ok(None);
    };
    let mut entries: Vec<crate::domain::mr::StackEntry> = stack
        .entries
        .map(|e| e.nodes)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|node| {
            let pr_details = node.pull_request?;
            Some(crate::domain::mr::StackEntry {
                position: node.position,
                number: pr_details.number,
                title: pr_details.title,
                state: pr_details.state.to_lowercase(),
                is_draft: pr_details.is_draft,
            })
        })
        .collect();
    entries.sort_by_key(|e| e.position);
    Ok(Some(crate::domain::mr::PrStack {
        info: crate::domain::mr::StackInfo {
            number: stack.number,
            size: stack.size,
            position: own_entry.position,
        },
        entries,
    }))
}

/// How long to wait between polls of a pending asynchronous merge, and how
/// many polls to make before reporting it as still running.
const ASYNC_MERGE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
const ASYNC_MERGE_MAX_POLLS: u32 = 90;

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum AsyncMergeStatus {
    Pending,
    Merged,
    Enqueued,
    Failed,
}

#[derive(Deserialize)]
struct AsyncMergeResult {
    status: AsyncMergeStatus,
    #[serde(default)]
    details: AsyncMergeDetails,
}

#[derive(Deserialize, Default)]
struct AsyncMergeDetails {
    message: Option<String>,
    uuid: Option<String>,
}

/// Where an asynchronous merge stands. A merge queue takes over an enqueued
/// PR, so being enqueued settles the request as much as being merged does.
#[derive(Debug, PartialEq, Eq)]
enum AsyncMergeProgress {
    Settled,
    Pending { uuid: String },
}

fn parse_async_merge_result(raw: &str) -> Result<AsyncMergeProgress> {
    let result: AsyncMergeResult =
        serde_json::from_str(raw).context("parsing GitHub's asynchronous merge result")?;
    match result.status {
        AsyncMergeStatus::Merged | AsyncMergeStatus::Enqueued => Ok(AsyncMergeProgress::Settled),
        AsyncMergeStatus::Failed => {
            let message = result
                .details
                .message
                .unwrap_or_else(|| "GitHub reported the merge as failed".to_string());
            anyhow::bail!("{message}")
        }
        AsyncMergeStatus::Pending => result
            .details
            .uuid
            .map(|uuid| AsyncMergeProgress::Pending { uuid })
            .ok_or_else(|| anyhow::anyhow!("GitHub reported a pending merge without its uuid")),
    }
}

/// How many of an environment's newest deployments a stop searches for the
/// one GitHub still serves.
const STOP_DEPLOYMENT_LOOKBACK: usize = 30;

/// State of a deployment's newest status (GitHub lists them newest first),
/// `None` while it has no status yet.
fn parse_latest_deployment_state(raw: &str) -> Result<Option<String>> {
    #[derive(Deserialize)]
    struct GhDeploymentStatus {
        state: String,
    }
    let statuses: Vec<GhDeploymentStatus> =
        serde_json::from_str(raw).context("parsing GitHub deployment statuses")?;
    Ok(statuses.into_iter().next().map(|status| status.state))
}

/// Build the GraphQL query that finds the issues which a PR closes.
fn related_issues_graphql_query(owner: &str, repo: &str, pr_number: u64, first: usize) -> String {
    let owner = owner.replace('\\', "\\\\").replace('"', "\\\"");
    let repo = repo.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "{{ repository(owner:\"{owner}\",name:\"{repo}\") {{ pullRequest(number:{pr_number}) {{ \
         closingIssuesReferences(first:{first}) {{ \
         nodes {{ number title state }} }} }} }} }}",
    )
}

mod related_issues_types {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub(super) struct GhResponse {
        pub(super) data: Option<GhData>,
    }
    #[derive(Deserialize)]
    pub(super) struct GhData {
        pub(super) repository: Option<GhRepo>,
    }
    #[derive(Deserialize)]
    pub(super) struct GhRepo {
        #[serde(rename = "pullRequest")]
        pub(super) pull_request: Option<GhPr>,
    }
    #[derive(Deserialize)]
    pub(super) struct GhPr {
        #[serde(default, rename = "closingIssuesReferences")]
        pub(super) closing_issues_references: GhConn,
    }
    #[derive(Deserialize, Default)]
    pub(super) struct GhConn {
        #[serde(default)]
        pub(super) nodes: Vec<GhIssueRef>,
    }
    #[derive(Deserialize)]
    pub(super) struct GhIssueRef {
        pub(super) number: u64,
        pub(super) title: String,
        pub(super) state: String,
    }
}

/// The REST review-comments endpoint carries no resolution state; only the
/// GraphQL `reviewThreads` connection exposes `isResolved`. A comment's REST
/// `id` is its GraphQL `databaseId`, and every REST reply names its thread's
/// first comment in `in_reply_to_id`, so that first comment is enough to
/// resolve the whole thread. `gh api graphql --paginate` feeds `$endCursor`
/// back in until `pageInfo.hasNextPage` is false.
fn review_thread_resolution_graphql_query(owner: &str, repo: &str, pr_number: u64) -> String {
    let owner = owner.replace('\\', "\\\\").replace('"', "\\\"");
    let repo = repo.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "query($endCursor: String) {{ repository(owner:\"{owner}\",name:\"{repo}\") {{ \
         pullRequest(number:{pr_number}) {{ reviewThreads(first:100, after:$endCursor) {{ \
         pageInfo {{ hasNextPage endCursor }} \
         nodes {{ isResolved comments(first:1) {{ nodes {{ databaseId }} }} }} }} }} }} }}"
    )
}

/// Maps the first comment of every review thread to the thread's resolution
/// state, across every page `gh api graphql --paginate` printed (one JSON
/// object per page, back to back).
fn parse_review_thread_resolution(raw: &str) -> Result<HashMap<u64, bool>> {
    use review_threads_types::*;
    let mut resolution = HashMap::new();
    for page in serde_json::Deserializer::from_str(raw).into_iter::<GhResponse>() {
        let page = page.context("parsing review threads")?;
        if let Some(error) = page.errors.first() {
            anyhow::bail!("GraphQL error: {}", error.message);
        }
        let threads = page
            .data
            .and_then(|d| d.repository)
            .and_then(|r| r.pull_request)
            .map(|pr| pr.review_threads.nodes)
            .unwrap_or_default();
        resolution.extend(threads.into_iter().filter_map(|thread| {
            let first = thread.comments.nodes.first()?;
            Some((first.database_id, thread.is_resolved))
        }));
    }
    Ok(resolution)
}

#[derive(Deserialize)]
struct GhReviewComment {
    id: u64,
    body: String,
    user: Option<GhReviewCommentUser>,
    created_at: String,
    path: Option<String>,
    line: Option<u64>,
    #[serde(default = "default_review_comment_side")]
    side: String,
    start_line: Option<u64>,
    in_reply_to_id: Option<u64>,
}

#[derive(Deserialize)]
struct GhReviewCommentUser {
    login: String,
}

fn default_review_comment_side() -> String {
    "RIGHT".to_string()
}

/// Builds the notes of a PR from its REST review comments, reading every
/// array `gh api --paginate` printed: it may print one array per page instead
/// of a single merged one. A comment takes the resolution state of its
/// thread, which `resolution` keys by the thread's first comment.
fn parse_pr_review_comments(
    raw: &str,
    resolution: &HashMap<u64, bool>,
) -> Result<Vec<DiscussionNote>> {
    let mut notes = Vec::new();
    for page in serde_json::Deserializer::from_str(raw).into_iter::<Vec<GhReviewComment>>() {
        let page = page.context("parsing PR review comments")?;
        notes.extend(
            page.into_iter()
                .map(|comment| review_comment_note(comment, resolution)),
        );
    }
    Ok(notes)
}

fn review_comment_note(
    comment: GhReviewComment,
    resolution: &HashMap<u64, bool>,
) -> DiscussionNote {
    let position = comment.path.map(|path| {
        let (new_line, old_line) = if comment.side == "LEFT" {
            (None, comment.line)
        } else {
            (comment.line, None)
        };
        NotePosition {
            new_path: Some(path.clone()),
            old_path: Some(path),
            new_line,
            old_line,
            start_line: comment.start_line,
            line_range: None,
        }
    });
    let thread_id = comment.in_reply_to_id.unwrap_or(comment.id);
    let resolved = resolution.get(&thread_id).copied();
    DiscussionNote {
        id: comment.id,
        body: comment.body,
        author: crate::domain::mr::Author {
            username: comment.user.map(|u| u.login).unwrap_or_default(),
        },
        created_at: comment.created_at,
        system: false,
        position,
        discussion_id: Some(thread_id.to_string()),
        resolved,
        // A comment GraphQL does not list under any thread has no known
        // state; it is shown as a plain note rather than as open.
        resolvable: Some(resolved.is_some()),
    }
}

mod review_threads_types {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub(super) struct GhResponse {
        pub(super) data: Option<GhData>,
        #[serde(default)]
        pub(super) errors: Vec<GhError>,
    }
    #[derive(Deserialize)]
    pub(super) struct GhError {
        pub(super) message: String,
    }
    #[derive(Deserialize)]
    pub(super) struct GhData {
        pub(super) repository: Option<GhRepo>,
    }
    #[derive(Deserialize)]
    pub(super) struct GhRepo {
        #[serde(rename = "pullRequest")]
        pub(super) pull_request: Option<GhPr>,
    }
    #[derive(Deserialize)]
    pub(super) struct GhPr {
        #[serde(rename = "reviewThreads")]
        pub(super) review_threads: GhConn<GhThread>,
    }
    #[derive(Deserialize)]
    pub(super) struct GhConn<T> {
        pub(super) nodes: Vec<T>,
    }
    #[derive(Deserialize)]
    pub(super) struct GhThread {
        #[serde(rename = "isResolved")]
        pub(super) is_resolved: bool,
        pub(super) comments: GhConn<GhComment>,
    }
    #[derive(Deserialize)]
    pub(super) struct GhComment {
        #[serde(rename = "databaseId")]
        pub(super) database_id: u64,
    }
}

fn github_review_event(event: ReviewEvent) -> &'static str {
    match event {
        ReviewEvent::Comment => "COMMENT",
        ReviewEvent::Approve => "APPROVE",
        ReviewEvent::RequestChanges => "REQUEST_CHANGES",
    }
}

/// One `comments[]` entry of the create-review endpoint. An unchanged line
/// carries both line numbers; it is anchored on the new side, because GitHub
/// reads `line` against the side it is given.
fn github_review_comment(comment: &DraftComment) -> serde_json::Value {
    let (side, start, end) = match (comment.line_num, comment.old_line_num) {
        (Some(line), _) => ("RIGHT", line, comment.end_line_num),
        (None, Some(line)) => ("LEFT", line, comment.end_old_line_num),
        // `GitlabClient::submit_review` rejects comments without a line first.
        (None, None) => ("RIGHT", 1, None),
    };
    let mut value = serde_json::json!({
        "path": comment.file_path,
        "line": start,
        "side": side,
        "body": comment.body,
    });
    if let Some(end) = end.filter(|&end| end != start) {
        value["start_line"] = serde_json::json!(start.min(end));
        value["start_side"] = serde_json::json!(side);
        value["line"] = serde_json::json!(start.max(end));
    }
    value
}

fn github_review_payload(
    event: ReviewEvent,
    body: &str,
    comments: &[DraftComment],
) -> serde_json::Value {
    serde_json::json!({
        "body": body,
        "event": github_review_event(event),
        "comments": comments.iter().map(github_review_comment).collect::<Vec<_>>(),
    })
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

    /// `gh issue list` and `gh pr list` narrow a repository listing by state.
    /// Organization listings go through the search API and keep listing every
    /// state.
    fn server_state_query(&self, scope: &Scope, filtered: StateQuery) -> StateQuery {
        match scope {
            Scope::Repository(_) => filtered,
            Scope::Group(_) => StateQuery::All,
        }
    }

    // ── Issues ──

    /// `_per_request` is unused in the GitHub backend (as with all `Backend`
    /// methods that take it) — `gh` uses `--limit` for the total item count,
    /// not per-page pagination. Only GitLab backends paginate per-request.
    async fn list_issues(
        &self,
        scope: &Scope,
        state: StateQuery,
        page_size: usize,
        _per_request: usize,
    ) -> Result<Vec<Issue>> {
        match scope {
            Scope::Repository(project) => {
                let state = gh_issue_state_flag(state);
                let total = page_size * 10;
                let raw = self
                    .run_gh(
                        &[
                            "issue",
                            "list",
                            "--json",
                            "number,title,state,labels,author,body,createdAt,updatedAt,closedAt,milestone,assignees,url,closedByPullRequestsReferences",
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

                let mut issues = parse_gh_issues(&raw)?;
                for issue in &mut issues {
                    if issue.project_path.is_empty() {
                        issue.project_path = project.to_string();
                    }
                }
                Ok(issues)
            }
            Scope::Group(org) => {
                let per_page = (page_size * 10).clamp(1, 100);
                let q = if state != StateQuery::Open {
                    format!("owner:{org}+is:issue")
                } else {
                    format!("owner:{org}+is:issue+state:open")
                };
                let endpoint = format!("search/issues?q={q}&per_page={per_page}");
                let raw = self
                    .run_gh(&["api", &endpoint], "Fetching Org Issues")
                    .await?;

                #[derive(Deserialize)]
                struct GhSearchIssueUser {
                    login: String,
                }
                #[derive(Deserialize)]
                struct GhSearchIssueMs {
                    title: String,
                    number: u64,
                    #[serde(default)]
                    state: Option<String>,
                }
                #[derive(Deserialize)]
                struct GhSearchIssueLabel {
                    name: String,
                }
                #[derive(Deserialize)]
                struct GhSearchIssueItem {
                    number: u64,
                    title: String,
                    state: String,
                    #[serde(default)]
                    labels: Vec<GhSearchIssueLabel>,
                    user: Option<GhSearchIssueUser>,
                    body: Option<String>,
                    created_at: Option<String>,
                    updated_at: String,
                    closed_at: Option<String>,
                    milestone: Option<GhSearchIssueMs>,
                    #[serde(default)]
                    assignees: Vec<GhSearchIssueUser>,
                    html_url: Option<String>,
                    repository_url: Option<String>,
                    #[serde(default)]
                    pull_request: Option<serde_json::Value>,
                }
                #[derive(Deserialize)]
                struct GhSearchResponse {
                    items: Vec<GhSearchIssueItem>,
                }

                let resp: GhSearchResponse = serde_json::from_str(&raw)?;
                let issues: Vec<Issue> = resp
                    .items
                    .into_iter()
                    .filter(|item| item.pull_request.is_none())
                    .map(|item| {
                        let project_path = item
                            .repository_url
                            .as_deref()
                            .and_then(|url| url.strip_prefix("https://api.github.com/repos/"))
                            .map(String::from)
                            .or_else(|| {
                                item.html_url.as_deref().and_then(|url| {
                                    let parts: Vec<&str> = url.split('/').collect();
                                    if parts.len() >= 5 {
                                        Some(format!("{}/{}", parts[3], parts[4]))
                                    } else {
                                        None
                                    }
                                })
                            })
                            .unwrap_or_default();

                        Issue {
                            iid: item.number,
                            title: item.title,
                            state: item.state.to_lowercase(),
                            labels: item.labels.into_iter().map(|l| l.name).collect(),
                            updated_at: item.updated_at,
                            created_at: item.created_at,
                            closed_at: item.closed_at,
                            author: crate::domain::issues::Author {
                                username: item.user.map(|u| u.login).unwrap_or_default(),
                            },
                            milestone: item.milestone.map(|m| crate::domain::issues::Milestone {
                                title: m.title,
                                iid: m.number,
                                id: 0,
                                state: m.state.unwrap_or_default(),
                            }),
                            assignees: item
                                .assignees
                                .into_iter()
                                .map(|a| crate::domain::issues::Assignee { username: a.login })
                                .collect(),
                            description: item.body,
                            due_date: None,
                            web_url: item.html_url.unwrap_or_default(),
                            project_path,
                            related_mrs: None,
                        }
                    })
                    .collect();
                Ok(issues)
            }
        }
    }

    async fn get_issue(&self, project: &str, iid: u64) -> Result<Issue> {
        let raw = self
            .run_gh(
                &[
                    "issue",
                    "view",
                    &iid.to_string(),
                    "--json",
                    "number,title,state,labels,author,body,createdAt,updatedAt,closedAt,milestone,assignees,url,closedByPullRequestsReferences",
                    "-R",
                    project,
                ],
                "Fetching Issue",
            )
            .await?;
        parse_gh_issue(&raw)
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

    async fn list_issue_related_mrs(
        &self,
        project: &str,
        issue_iid: u64,
        page_size: usize,
    ) -> Result<Vec<RelatedMrRef>> {
        let owner = project.split('/').next().unwrap_or(project);
        let repo = project.split('/').nth(1).unwrap_or(project);
        // `gh issue view --json closedByPullRequestsReferences` only exposes
        // `id`/`number`/`repository`/`url` per PR — no title or state — because
        // the CLI surfaces the raw GraphQL nodes. Pull the same relationship
        // via a single GraphQL call so we get `title` and `state` in one round
        // trip, capped to the requested page size.
        let first = page_size.clamp(1, 100);
        let query = related_prs_graphql_query(owner, repo, issue_iid, first);
        let raw = self
            .run_gh(
                &["api", "graphql", "-f", &format!("query={query}")],
                "Fetching Related PRs",
            )
            .await?;
        use related_prs_types::*;
        let resp: GhResponse = serde_json::from_str(&raw)?;
        let refs = resp
            .data
            .and_then(|d| d.repository)
            .and_then(|r| r.issue)
            .map(|i| i.closed_by_pull_requests_references.nodes)
            .unwrap_or_default();
        Ok(refs
            .into_iter()
            .map(|p| RelatedMrRef {
                iid: p.number,
                title: p.title,
                state: p.state.to_lowercase(),
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
        self.run_gh(&args_refs, "CREATING ISSUE").await?;
        Ok(())
    }

    // ── Issue Field Updates ──

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
        let mut args: Vec<String> = vec!["issue".into(), "edit".into(), iid_str];
        if !project.is_empty() {
            args.extend(["-R".into(), project.into()]);
        }
        if let Some(ref title) = update.title {
            args.extend(["--title".into(), title.clone()]);
        }
        if let Some(ref desc) = update.description {
            args.extend(["--body".into(), desc.clone()]);
        }
        for label in &update.add_labels {
            args.extend(["--add-label".into(), label.clone()]);
        }
        for label in &update.remove_labels {
            args.extend(["--remove-label".into(), label.clone()]);
        }
        for a in &update.add_assignees {
            args.extend(["--add-assignee".into(), a.clone()]);
        }
        for a in &update.remove_assignees {
            args.extend(["--remove-assignee".into(), a.clone()]);
        }
        if let Some(ref milestone) = update.milestone {
            if milestone == "--" || milestone.is_empty() {
                args.push("--remove-milestone".into());
            } else {
                args.extend(["--milestone".into(), milestone.clone()]);
            }
        }
        if update.due_date.is_some()
            && update.title.is_none()
            && update.description.is_none()
            && update.add_labels.is_empty()
            && update.remove_labels.is_empty()
            && update.add_assignees.is_empty()
            && update.remove_assignees.is_empty()
            && update.milestone.is_none()
        {
            anyhow::bail!("GitHub issues do not support due dates");
        }
        let args_refs: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
        self.run_gh(&args_refs, "UPDATING ISSUE").await?;
        Ok(())
    }

    // ── Merge Requests ──

    /// `_per_request` is unused in the GitHub backend — `gh` uses `--limit`
    /// for the total item count, not per-page pagination.
    async fn list_mrs(
        &self,
        scope: &Scope,
        state: StateQuery,
        page_size: usize,
        _per_request: usize,
    ) -> Result<Vec<MergeRequest>> {
        match scope {
            Scope::Repository(project) => {
                let state = gh_pr_state_flag(state);
                let total = page_size * 10;
                let raw = self
                    .run_gh(
                        &[
                            "pr",
                            "list",
                            "--json",
                            "number,title,state,labels,author,body,createdAt,updatedAt,headRefName,baseRefName,isDraft,assignees,milestone,reviewDecision,latestReviews,mergeable,mergeStateStatus,reviewRequests,url,closingIssuesReferences",
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
                    #[serde(default)]
                    url: Option<String>,
                    #[serde(rename = "closingIssuesReferences", default)]
                    closing_issues_references: Option<Vec<GhPrIssueRef>>,
                }
                #[derive(Deserialize)]
                struct GhPrIssueRef {
                    number: u64,
                    #[serde(default)]
                    title: Option<String>,
                    #[serde(default)]
                    state: Option<String>,
                    #[serde(default)]
                    repository: Option<GhIssueRepoRef>,
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
                        let state = map_gh_pr_state(&gp.state).to_string();
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
                        let related_issues = gp.closing_issues_references.map(|refs| {
                            if refs.is_empty() {
                                crate::domain::mr::RelatedIssuesState::Empty
                            } else {
                                crate::domain::mr::RelatedIssuesState::Items(
                                    refs.into_iter()
                                        .map(|r| {
                                            let project_path = r.repository.map(|repo| {
                                                format!("{}/{}", repo.owner.login, repo.name)
                                            });
                                            crate::domain::mr::RelatedIssueRef {
                                                iid: r.number,
                                                title: r.title.unwrap_or_default(),
                                                state: r.state.unwrap_or_default(),
                                                project_path,
                                            }
                                        })
                                        .collect(),
                                )
                            }
                        });
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
                            // GitHub has no equivalent of GitLab's diff head SHA —
                            // `gh pr merge` decides locally and the `--sha`
                            // plumbing simply ignores this at the backend.
                            sha: None,
                            draft: gp.is_draft.unwrap_or(false),
                            description: gp.body,
                            head_pipeline: None,
                            blocking_discussions_resolved: None,
                            approval,
                            mergeability,
                            workflow: None,
                            project_path: project.to_string(),
                            web_url: gp.url,
                            related_issues,
                            stack: None,
                            stack_entries: None,
                        }
                    })
                    .collect())
            }
            Scope::Group(org) => {
                let per_page = (page_size * 10).clamp(1, 100);
                let q = if state != StateQuery::Open {
                    format!("owner:{org}+is:pr")
                } else {
                    format!("owner:{org}+is:pr+state:open")
                };
                let endpoint = format!("search/issues?q={q}&per_page={per_page}");
                let raw = self.run_gh(&["api", &endpoint], "Fetching Org PRs").await?;

                #[derive(Deserialize)]
                struct GhSearchPrUser {
                    login: String,
                }
                #[derive(Deserialize)]
                struct GhSearchPrMs {
                    title: String,
                }
                #[derive(Deserialize)]
                struct GhSearchPrLabel {
                    name: String,
                }
                #[derive(Deserialize)]
                struct GhSearchPrItem {
                    number: u64,
                    title: String,
                    state: String,
                    #[serde(default)]
                    labels: Vec<GhSearchPrLabel>,
                    user: Option<GhSearchPrUser>,
                    body: Option<String>,
                    updated_at: String,
                    milestone: Option<GhSearchPrMs>,
                    #[serde(default)]
                    assignees: Vec<GhSearchPrUser>,
                    #[serde(default)]
                    draft: bool,
                    html_url: Option<String>,
                    repository_url: Option<String>,
                }
                #[derive(Deserialize)]
                struct GhSearchResponse {
                    items: Vec<GhSearchPrItem>,
                }

                let resp: GhSearchResponse = serde_json::from_str(&raw)?;
                let mrs: Vec<MergeRequest> = resp
                    .items
                    .into_iter()
                    .map(|item| {
                        let project_path = item
                            .repository_url
                            .as_deref()
                            .and_then(|url| url.strip_prefix("https://api.github.com/repos/"))
                            .map(String::from)
                            .or_else(|| {
                                item.html_url.as_deref().and_then(|url| {
                                    let parts: Vec<&str> = url.split('/').collect();
                                    if parts.len() >= 5 {
                                        Some(format!("{}/{}", parts[3], parts[4]))
                                    } else {
                                        None
                                    }
                                })
                            })
                            .unwrap_or_default();

                        let state_str = item.state.to_lowercase();
                        let norm_state = if state_str == "open" {
                            "opened".to_string()
                        } else {
                            state_str
                        };

                        MergeRequest {
                            iid: item.number,
                            title: item.title,
                            state: norm_state,
                            labels: item.labels.into_iter().map(|l| l.name).collect(),
                            updated_at: item.updated_at,
                            author: crate::domain::mr::Author {
                                username: item.user.map(|u| u.login).unwrap_or_default(),
                            },
                            milestone: item
                                .milestone
                                .map(|m| crate::domain::mr::Milestone { title: m.title }),
                            assignees: item
                                .assignees
                                .into_iter()
                                .map(|a| crate::domain::mr::Assignee { username: a.login })
                                .collect(),
                            reviewers: vec![],
                            target_branch: String::new(),
                            source_branch: String::new(),
                            // GitHub has no equivalent of GitLab's diff head SHA.
                            sha: None,
                            draft: item.draft,
                            description: item.body,
                            head_pipeline: None,
                            blocking_discussions_resolved: None,
                            approval: None,
                            mergeability: None,
                            workflow: None,
                            project_path,
                            web_url: item.html_url.clone(),
                            related_issues: None,
                            stack: None,
                            stack_entries: None,
                        }
                    })
                    .collect();
                Ok(mrs)
            }
        }
    }

    async fn get_mr(&self, project: &str, iid: u64) -> Result<MergeRequest> {
        let raw = self
            .run_gh(
                &[
                    "pr",
                    "view",
                    &iid.to_string(),
                    "--json",
                    "number,title,state,labels,author,body,createdAt,updatedAt,headRefName,baseRefName,isDraft,assignees,milestone,url,closingIssuesReferences",
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
            #[serde(default)]
            url: Option<String>,
            #[serde(rename = "closingIssuesReferences", default)]
            closing_issues_references: Option<Vec<GhPrIssueRef>>,
        }
        #[derive(Deserialize)]
        struct GhPrIssueRef {
            number: u64,
            #[serde(default)]
            title: Option<String>,
            #[serde(default)]
            state: Option<String>,
            #[serde(default)]
            repository: Option<GhIssueRepoRef>,
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
        let state = map_gh_pr_state(&gp.state).to_string();
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
        let related_issues = gp.closing_issues_references.map(|refs| {
            if refs.is_empty() {
                crate::domain::mr::RelatedIssuesState::Empty
            } else {
                crate::domain::mr::RelatedIssuesState::Items(
                    refs.into_iter()
                        .map(|r| {
                            let project_path = r
                                .repository
                                .map(|repo| format!("{}/{}", repo.owner.login, repo.name));
                            crate::domain::mr::RelatedIssueRef {
                                iid: r.number,
                                title: r.title.unwrap_or_default(),
                                state: r.state.unwrap_or_default(),
                                project_path,
                            }
                        })
                        .collect(),
                )
            }
        });
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
            // GitHub has no equivalent of GitLab's diff head SHA.
            sha: None,
            draft: gp.is_draft.unwrap_or(false),
            description: gp.body,
            head_pipeline: None,
            blocking_discussions_resolved: None,
            approval: None,
            mergeability: None,
            workflow: None,
            project_path: String::new(),
            web_url: gp.url,
            related_issues,
            stack: None,
            stack_entries: None,
        })
    }

    async fn get_pr_stack(
        &self,
        project: &str,
        pr_number: u64,
    ) -> Result<Option<crate::domain::mr::PrStack>> {
        let owner = project.split('/').next().unwrap_or(project);
        let repo = project.split('/').nth(1).unwrap_or(project);
        let query = pr_stack_entries_graphql_query(owner, repo, pr_number, 50);
        let raw = self
            .run_gh(
                &["api", "graphql", "-f", &format!("query={query}")],
                "Fetching PR Stack",
            )
            .await?;
        parse_pr_stack_entries_response(&raw)
    }

    async fn list_pr_stack_summaries(
        &self,
        project: &str,
        pr_numbers: &[u64],
    ) -> Result<HashMap<u64, crate::domain::mr::StackInfo>> {
        if pr_numbers.is_empty() {
            return Ok(HashMap::new());
        }
        let owner = project.split('/').next().unwrap_or(project);
        let repo = project.split('/').nth(1).unwrap_or(project);
        let query = pr_stack_graphql_query(owner, repo, pr_numbers);
        let raw = self
            .run_gh(
                &["api", "graphql", "-f", &format!("query={query}")],
                "Fetching PR Stacks",
            )
            .await?;
        Ok(parse_pr_stacks_response(&raw))
    }

    /// `gh pr merge` cannot merge a stacked PR: GitHub refuses stacks on the
    /// GraphQL mutation it calls, and the asynchronous merge endpoint is the
    /// only one that accepts them.
    async fn merge_pr_stack(
        &self,
        project: &str,
        pr_number: u64,
        method: crate::domain::mr::MergeMethod,
    ) -> Result<()> {
        let endpoint = format!("repos/{project}/pulls/{pr_number}/merge-async");
        let body = serde_json::json!({ "merge_method": method.as_api_str() }).to_string();
        let raw = self
            .raw_api(&endpoint, "PUT", Some(&body), "Merging PR Stack")
            .await?;
        let mut progress = parse_async_merge_result(&raw)?;
        let mut polls = 0;
        while let AsyncMergeProgress::Pending { uuid } = progress {
            if polls == ASYNC_MERGE_MAX_POLLS {
                anyhow::bail!(
                    "GitHub is still merging PR #{pr_number} after {} seconds; check it on GitHub",
                    ASYNC_MERGE_POLL_INTERVAL.as_secs() * u64::from(ASYNC_MERGE_MAX_POLLS)
                );
            }
            polls += 1;
            tokio::time::sleep(ASYNC_MERGE_POLL_INTERVAL).await;
            let raw = self
                .raw_api(
                    &format!("{endpoint}/{uuid}"),
                    "GET",
                    None,
                    "Checking PR Stack Merge",
                )
                .await?;
            progress = parse_async_merge_result(&raw)?;
        }
        Ok(())
    }

    async fn get_mr_diff(&self, project: &str, iid: u64) -> Result<String> {
        match self
            .run_gh_bounded(
                &["pr", "diff", &iid.to_string(), "-R", project],
                "Fetching PR Diff",
                Some(MR_DIFF_MAX_BYTES),
            )
            .await
        {
            Err(err) if is_pr_diff_too_large(&err) => self.diff_pr_locally(project, iid).await,
            Err(err) => Err(err),
            Ok(diff) => {
                ensure_complete_diff(diff.is_truncated, &format!("PR #{iid}"))?;
                Ok(String::from_utf8(diff.bytes)?)
            }
        }
    }

    async fn list_mr_notes(&self, project: &str, mr_iid: u64) -> Result<Vec<DiscussionNote>> {
        let endpoint = format!("/repos/{project}/pulls/{mr_iid}/comments?per_page=100");
        let owner = project.split('/').next().unwrap_or(project);
        let repo = project.split('/').nth(1).unwrap_or(project);
        let query_arg = format!(
            "query={}",
            review_thread_resolution_graphql_query(owner, repo, mr_iid)
        );
        let comments_args = ["api", "--paginate", endpoint.as_str()];
        let graphql_args = ["api", "graphql", "--paginate", "-f", query_arg.as_str()];
        let (raw_comments, raw_threads) = tokio::try_join!(
            self.run_gh(&comments_args, "Fetching MR Notes"),
            self.run_gh(&graphql_args, "Fetching Review Threads"),
        )?;
        let resolution = parse_review_thread_resolution(&raw_threads)?;
        parse_pr_review_comments(&raw_comments, &resolution)
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

    async fn list_mr_related_issues(
        &self,
        project: &str,
        mr_iid: u64,
        page_size: usize,
    ) -> Result<Vec<crate::domain::mr::RelatedIssueRef>> {
        let owner = project.split('/').next().unwrap_or(project);
        let repo = project.split('/').nth(1).unwrap_or(project);
        let first = page_size.clamp(1, 100);
        let query = related_issues_graphql_query(owner, repo, mr_iid, first);
        let raw = self
            .run_gh(
                &["api", "graphql", "-f", &format!("query={query}")],
                "Fetching Related Issues",
            )
            .await?;
        use related_issues_types::*;
        let resp: GhResponse = serde_json::from_str(&raw)?;
        let refs = resp
            .data
            .and_then(|d| d.repository)
            .and_then(|r| r.pull_request)
            .map(|pr| pr.closing_issues_references.nodes)
            .unwrap_or_default();
        Ok(refs
            .into_iter()
            .map(|i| crate::domain::mr::RelatedIssueRef {
                iid: i.number,
                title: i.title,
                state: i.state.to_lowercase(),
                project_path: Some(project.to_string()),
            })
            .collect())
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
        options: &super::MergeOptions<'_>,
    ) -> Result<()> {
        // GitHub's `gh pr merge` has no `--sha` equivalent — the CLI works
        // off the PR's HEAD ref rather than a specific commit SHA. Ignore it.
        let super::MergeOptions {
            squash,
            delete_branch,
            strategy,
            auto_merge,
            sha: _,
        } = *options;
        let args = Self::merge_args(project, iid, squash, delete_branch, strategy, auto_merge);
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
        let mut args: Vec<String> = vec![
            "pr".into(),
            "create".into(),
            "-R".into(),
            project.into(),
            "--title".into(),
            title.into(),
        ];
        if !source_branch.is_empty() {
            let local_project =
                crate::git_helpers::remote_project_path("origin").unwrap_or_default();
            args.push("--head".into());
            args.push(qualify_head_ref(&local_project, project, source_branch));
        }
        if !target_branch.is_empty() {
            args.push("--base".into());
            args.push(target_branch.into());
        }
        let body = if let Some(iid) = issue_iid {
            if description.is_empty() {
                format!("Closes #{iid}")
            } else if !description.contains("Closes #") {
                format!("{description}\n\nCloses #{iid}")
            } else {
                description.to_string()
            }
        } else {
            description.to_string()
        };
        if !body.is_empty() {
            args.push("--body".into());
            args.push(body);
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
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_gh(&args_refs, "CREATING PR").await?;
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
        let payload = github_review_payload(event, body, comments).to_string();
        self.raw_api(
            &format!("/repos/{project}/pulls/{iid}/reviews"),
            "POST",
            Some(&payload),
            "SUBMITTING REVIEW",
        )
        .await?;
        Ok(())
    }

    async fn reply_to_thread(
        &self,
        project: &str,
        iid: u64,
        thread_id: &str,
        body: &str,
    ) -> Result<()> {
        let root_comment_id: u64 = thread_id.parse().with_context(|| {
            format!(
                "GitHub thread ids are the numeric id of the thread's first comment, got '{thread_id}'"
            )
        })?;
        let payload = serde_json::json!({ "body": body, "in_reply_to": root_comment_id });
        self.raw_api(
            &format!("/repos/{project}/pulls/{iid}/comments"),
            "POST",
            Some(&payload.to_string()),
            "REPLYING TO THREAD",
        )
        .await?;
        Ok(())
    }

    async fn set_thread_resolved(
        &self,
        _project: &str,
        _iid: u64,
        _thread_id: &str,
        _resolved: bool,
    ) -> Result<()> {
        // Only the GraphQL `resolveReviewThread` mutation resolves a thread,
        // and it takes the thread's node id, which the REST comment listing
        // this backend reads does not carry.
        anyhow::bail!("Resolving review threads isn't supported on GitHub")
    }

    // ── PR Field Updates ──

    async fn update_mr(&self, project: &str, iid: u64, update: &super::MrUpdate) -> Result<()> {
        if update.is_empty() {
            return Ok(());
        }
        let iid_str = iid.to_string();
        let mut args: Vec<String> = vec!["pr".into(), "edit".into(), iid_str];
        if !project.is_empty() {
            args.extend(["-R".into(), project.into()]);
        }
        if let Some(ref title) = update.title {
            args.extend(["--title".into(), title.clone()]);
        }
        if let Some(ref desc) = update.description {
            args.extend(["--body".into(), desc.clone()]);
        }
        for label in &update.add_labels {
            args.extend(["--add-label".into(), label.clone()]);
        }
        for label in &update.remove_labels {
            args.extend(["--remove-label".into(), label.clone()]);
        }
        for a in &update.add_assignees {
            args.extend(["--add-assignee".into(), a.clone()]);
        }
        for a in &update.remove_assignees {
            args.extend(["--remove-assignee".into(), a.clone()]);
        }
        for r in &update.add_reviewers {
            args.extend(["--add-reviewer".into(), r.clone()]);
        }
        for r in &update.remove_reviewers {
            args.extend(["--remove-reviewer".into(), r.clone()]);
        }
        if let Some(ref milestone) = update.milestone {
            if milestone == "--" || milestone.is_empty() {
                args.push("--remove-milestone".into());
            } else {
                args.extend(["--milestone".into(), milestone.clone()]);
            }
        }
        if let Some(ref target_branch) = update.target_branch {
            args.extend(["--base".into(), target_branch.clone()]);
        }
        if let Some(draft) = update.draft {
            if draft {
                args.push("--draft".into());
            } else {
                args.push("--ready-for-review".into());
            }
        }
        let args_refs: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
        self.run_gh(&args_refs, "UPDATING PR").await?;
        Ok(())
    }

    // ── Pipelines ──

    /// `_per_request` is unused in the GitHub backend — `gh api repos/{owner}/{repo}/actions/runs`
    /// uses `per_page` for pagination.
    async fn list_pipelines(
        &self,
        scope: &Scope,
        page_size: usize,
        _per_request: usize,
    ) -> Result<Vec<Pipeline>> {
        match scope {
            Scope::Repository(project) => {
                let mut runs = Vec::new();
                let limit = page_size.max(1);
                let per_page = limit.min(100);
                let mut page = 1;
                while runs.len() < limit {
                    let endpoint =
                        format!("repos/{project}/actions/runs?per_page={per_page}&page={page}");
                    let Ok(raw) = self.run_gh(&["api", &endpoint], "Fetching Actions").await else {
                        break;
                    };
                    let fetched = parse_github_actions_runs(&raw)?;
                    if fetched.is_empty() {
                        break;
                    }
                    let count = fetched.len();
                    for mut run in fetched {
                        if run.project_path.is_empty() {
                            run.project_path = project.to_string();
                        }
                        runs.push(run);
                    }
                    if count < per_page {
                        break;
                    }
                    page += 1;
                }
                runs.truncate(limit);
                Ok(runs)
            }
            Scope::Group(org) => {
                let repos_raw = match self
                    .run_gh(
                        &[
                            "api",
                            &format!("orgs/{org}/repos?per_page=100"),
                            "--jq",
                            ".[].full_name",
                        ],
                        "Fetching Org Repos",
                    )
                    .await
                {
                    Ok(raw) => raw,
                    Err(_) => {
                        self.run_gh(
                            &[
                                "api",
                                &format!("users/{org}/repos?per_page=100"),
                                "--jq",
                                ".[].full_name",
                            ],
                            "Fetching User Repos",
                        )
                        .await?
                    }
                };
                let repos: Vec<String> = repos_raw
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();

                let mut all_pipelines: Vec<Pipeline> = Vec::new();
                for repo in repos {
                    let endpoint = format!("repos/{repo}/actions/runs?per_page=10");
                    if let Ok(raw) = self.run_gh(&["api", &endpoint], "Fetching Actions").await {
                        if let Ok(mut runs) = parse_github_actions_runs(&raw) {
                            for run in runs.iter_mut() {
                                run.project_path = repo.clone();
                            }
                            all_pipelines.extend(runs);
                        }
                    }
                }
                all_pipelines.sort_by(|a, b| b.created_at().cmp(&a.created_at()));
                all_pipelines.truncate(page_size * 10);
                Ok(all_pipelines)
            }
        }
    }

    /// `gh run list` lists newest first. Pull-request runs carry the head
    /// branch too, so one query covers them.
    async fn find_branch_pipeline_id(&self, project: &str, branch: &str) -> Result<Option<u64>> {
        #[derive(Deserialize)]
        struct ListedRun {
            #[serde(rename = "databaseId")]
            id: u64,
        }
        let raw = self
            .run_gh(
                &[
                    "run",
                    "list",
                    "-R",
                    project,
                    "--branch",
                    branch,
                    "--limit",
                    "1",
                    "--json",
                    "databaseId",
                ],
                "Finding Branch Run",
            )
            .await?;
        let runs: Vec<ListedRun> =
            serde_json::from_str(raw.trim()).context("parsing `gh run list` output")?;
        Ok(runs.first().map(|run| run.id))
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
            #[serde(default)]
            steps: Vec<GhStep>,
            #[serde(default)]
            url: Option<String>,
        }

        #[derive(Deserialize)]
        struct GhStep {
            name: String,
            conclusion: Option<String>,
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
                let failure_reason = (status == "failed")
                    .then(|| {
                        github_job_failure_reason(
                            j.conclusion.as_deref(),
                            j.steps
                                .iter()
                                .map(|step| (step.name.as_str(), step.conclusion.as_deref())),
                        )
                    })
                    .flatten();
                Job {
                    id: j.id,
                    status,
                    // GitHub jobs form a dependency graph, not GitLab-style
                    // sequential stages. Keep this empty so stage grouping
                    // cannot imply semantics that do not exist.
                    stage: String::new(),
                    name: j.name,
                    api_name: String::new(),
                    matrix: None,
                    duration_seconds: duration,
                    runner: j.runner_name,
                    needs: j.needs,
                    web_url: j.url,
                    queued_seconds: None,
                    failure_reason,
                    artifact_file_types: Vec::new(),
                    commit: None,
                }
            })
            .collect();
        Ok(crate::domain::pipelines::process_pipeline_jobs(all_jobs))
    }

    async fn get_job_trace(&self, project: &str, job_id: u64) -> Result<JobTrace> {
        let log = self
            .run_gh_bounded(
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
                Some(JOB_TRACE_MAX_BYTES),
            )
            .await?;
        Ok(JobTrace::from_read(&log.bytes, log.is_truncated))
    }

    /// `gh run view --log` only serves the log of a completed job, and a
    /// re-run gets a new job id, so a log read once never gains anything.
    async fn get_job_trace_since(
        &self,
        _project: &str,
        _job_id: u64,
        cursor: &TraceCursor,
    ) -> Result<TraceUpdate> {
        Ok(cursor.append_update(Vec::new(), false))
    }

    async fn retry_pipeline(&self, project: &str, pipeline_id: u64) -> Result<()> {
        self.run_gh(
            &["run", "rerun", &pipeline_id.to_string(), "-R", project],
            "Retrying Action",
        )
        .await?;
        Ok(())
    }

    async fn retry_failed_jobs(&self, project: &str, pipeline_id: u64) -> Result<()> {
        self.run_gh(
            &[
                "run",
                "rerun",
                &pipeline_id.to_string(),
                "--failed",
                "-R",
                project,
            ],
            "Retrying Failed Jobs",
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
        let endpoint = format!("/repos/{project}/actions/jobs/{job_id}/cancel");
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
            args.push(format!("{k}={v}"));
        }
        for (k, v) in inputs {
            args.push("-f".into());
            args.push(format!("{k}={v}"));
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

    async fn list_runners(&self, scope: &Scope, page_size: usize) -> Result<Vec<Runner>> {
        let project = match scope {
            Scope::Repository(p) => p,
            Scope::Group(_) => {
                return Err(anyhow::anyhow!("Org-level runners not supported on GitHub"));
            }
        };
        let endpoint = format!("/repos/{project}/actions/runners?per_page={page_size}");
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
            #[serde(default)]
            labels: Vec<GhRunnerLabel>,
        }
        #[derive(Deserialize)]
        struct GhRunnerLabel {
            name: String,
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
                tags: r.labels.into_iter().map(|label| label.name).collect(),
                web_url: Some(github_runner_url(project, r.id)),
                ..Runner::default()
            })
            .collect())
    }

    async fn pause_runner(&self, _scope: &Scope, _runner_id: u64) -> Result<()> {
        anyhow::bail!("Runner management (pause/resume/edit) is not supported for GitHub runners")
    }

    async fn resume_runner(&self, _scope: &Scope, _runner_id: u64) -> Result<()> {
        anyhow::bail!("Runner management (pause/resume/edit) is not supported for GitHub runners")
    }

    async fn update_runner_description(
        &self,
        _scope: &Scope,
        _runner_id: u64,
        _description: &str,
    ) -> Result<()> {
        anyhow::bail!("Runner management (pause/resume/edit) is not supported for GitHub runners")
    }

    // ── Releases ──

    async fn list_releases(&self, scope: &Scope, page_size: usize) -> Result<Vec<Release>> {
        let project = match scope {
            Scope::Repository(p) => p,
            Scope::Group(_) => {
                return Err(anyhow::anyhow!(
                    "Org-level releases not supported on GitHub"
                ));
            }
        };
        let raw = self
            .run_gh(
                &[
                    "api",
                    &format!("repos/{project}/releases?per_page={page_size}"),
                ],
                "Fetching Releases",
            )
            .await?;

        #[derive(Deserialize)]
        struct GhRel {
            name: Option<String>,
            tag_name: String,
            published_at: Option<String>,
            created_at: Option<String>,
            body: Option<String>,
            author: Option<GhAuthor>,
        }
        #[derive(Deserialize)]
        struct GhAuthor {
            login: String,
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
                    .unwrap_or_else(|| {
                        r.created_at
                            .as_deref()
                            .map(|s| s.chars().take(10).collect::<String>())
                            .unwrap_or_default()
                    }),
                description: r.body,
                author_name: r.author.map(|a| a.login),
                commit_id: None,
                commit_title: None,
                assets_link: None,
                project_path: project.clone(),
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

    async fn list_milestones(&self, scope: &Scope, page_size: usize) -> Result<Vec<Milestone>> {
        let project = match scope {
            Scope::Repository(p) => p,
            Scope::Group(_) => {
                return Err(anyhow::anyhow!(
                    "Org-level milestones not supported on GitHub"
                ));
            }
        };
        let endpoint = format!("/repos/{project}/milestones?state=all&per_page={page_size}");
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
                    project_path: project.to_string(),
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
                    "number,title,state,labels,author,body,createdAt,updatedAt,closedAt,milestone,assignees,url",
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
        parse_gh_issues(&raw)
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
            args.push(format!("description={description}"));
        }
        if let Some(due) = due_date {
            if !due.is_empty() {
                let iso_due = if due.contains('T') {
                    due.to_string()
                } else {
                    format!("{due}T00:00:00Z")
                };
                args.push("-f".into());
                args.push(format!("due_on={iso_due}"));
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
                &format!("repos/{project}/milestones/{milestone_iid}"),
                "-f",
                &format!("state={state}"),
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
            args.push(format!("description={description}"));
        }
        if let Some(due) = due_date {
            if !due.is_empty() {
                let iso_due = if due.contains('T') {
                    due.to_string()
                } else {
                    format!("{due}T00:00:00Z")
                };
                args.push("-f".into());
                args.push(format!("due_on={iso_due}"));
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
                &format!("repos/{project}/milestones/{milestone_iid}"),
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
        parse_github_notifications(&raw)
    }

    async fn get_notification_detail(&self, detail_api_path: &str) -> Result<NotificationDetail> {
        let raw = match native_detail_args(detail_api_path) {
            Some(args) => self.run_gh(&args, "Fetching Notification").await?,
            None => {
                self.raw_api(detail_api_path, "GET", None, "Fetching Notification")
                    .await?
            }
        };
        parse_github_notification_detail(&raw)
    }

    async fn mark_notification_as_read(&self, id: &str) -> Result<()> {
        let endpoint = format!("notifications/threads/{id}");
        self.raw_api(&endpoint, "PATCH", None, "Marking Notification Read")
            .await?;
        Ok(())
    }

    async fn mark_notification_as_done(&self, id: &str) -> Result<()> {
        let endpoint = format!("notifications/threads/{id}");
        self.raw_api(&endpoint, "DELETE", None, "Marking Notification Done")
            .await?;
        Ok(())
    }

    async fn mark_all_notifications_as_read(&self) -> Result<()> {
        self.raw_api(
            "notifications",
            "PUT",
            None,
            "Marking All Notifications Read",
        )
        .await?;
        Ok(())
    }

    async fn unsubscribe_from_notification(&self, notification: &Notification) -> Result<()> {
        let endpoint = format!("notifications/threads/{}/subscription", notification.id);
        self.raw_api(&endpoint, "DELETE", None, "Unsubscribing")
            .await?;
        Ok(())
    }

    // ── Branches ──

    async fn list_branches(&self, scope: &Scope, page_size: usize) -> Result<Vec<Branch>> {
        let project = match scope {
            Scope::Repository(p) => p,
            Scope::Group(_) => {
                return Err(anyhow::anyhow!(
                    "Org-level branches not supported on GitHub"
                ));
            }
        };
        let endpoint = format!("/repos/{project}/branches?per_page={page_size}");
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
                &format!("/repos/{project}"),
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
        // GitHub API expects a commit SHA, not a branch name. Resolve if needed.
        let sha = if ref_branch.len() == 40 && ref_branch.chars().all(|c| c.is_ascii_hexdigit()) {
            // Already looks like a SHA
            ref_branch.to_string()
        } else {
            // Try to resolve branch name to SHA via GitHub API
            let encoded = Self::encode_branch(ref_branch);
            let endpoint = format!("/repos/{project}/git/refs/heads/{encoded}");
            let resp = self
                .raw_api(&endpoint, "GET", None, "Resolving Branch SHA")
                .await?;
            let parsed: serde_json::Value = serde_json::from_str(&resp)?;
            // GitHub returns {"ref":"refs/heads/main","node_id":...,"object":{"sha":"abc123..."},...}
            // or {"sha":"abc123...","url":"...","node_id":...} for newer API responses
            parsed
                .get("object")
                .and_then(|o| o.get("sha"))
                .and_then(|s| s.as_str())
                .or_else(|| parsed.get("sha").and_then(|s| s.as_str()))
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow::anyhow!("Could not resolve branch '{ref_branch}' to SHA"))?
        };

        let endpoint = format!("/repos/{project}/git/refs");
        let payload = serde_json::json!({
            "ref": format!("refs/heads/{}", branch_name),
            "sha": sha,
        });
        let json_str = serde_json::to_string(&payload)?;
        self.raw_api(&endpoint, "POST", Some(&json_str), "Creating Branch")
            .await?;
        Ok(())
    }

    async fn delete_branch(&self, project: &str, branch_name: &str) -> Result<()> {
        let encoded = Self::encode_branch(branch_name);
        let endpoint = format!("/repos/{project}/git/refs/heads/{encoded}");
        self.raw_api(&endpoint, "DELETE", None, "Deleting Branch")
            .await?;
        Ok(())
    }

    // ── Environments / Deployments ──

    async fn list_environments(&self, scope: &Scope, page_size: usize) -> Result<Vec<Environment>> {
        let project = match scope {
            Scope::Repository(p) => p,
            Scope::Group(_) => {
                return Err(anyhow::anyhow!(
                    "Org-level environments not supported on GitHub"
                ));
            }
        };
        let endpoint = format!("/repos/{project}/environments?per_page={page_size}");
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
        scope: &Scope,
        page_size: usize,
        environment: Option<&str>,
    ) -> Result<Vec<Deployment>> {
        let project = match scope {
            Scope::Repository(p) => p,
            Scope::Group(_) => {
                return Err(anyhow::anyhow!(
                    "Org-level deployments not supported on GitHub"
                ));
            }
        };
        let mut endpoint = format!("/repos/{project}/deployments?per_page={page_size}");
        if let Some(env) = environment {
            endpoint.push_str(&format!("&environment={env}"));
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
            #[serde(default)]
            creator: Option<GhCreator>,
        }
        #[derive(Deserialize)]
        struct GhCreator {
            login: String,
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
                user: d
                    .creator
                    .map(|c| crate::domain::deployments::DeploymentUser { username: c.login }),
            })
            .collect())
    }

    async fn stop_environment(&self, project: &str, environment: &Environment) -> Result<()> {
        let name = &environment.name;
        let deployments = self
            .list_deployments(
                &Scope::Repository(project.to_string()),
                STOP_DEPLOYMENT_LOOKBACK,
                Some(name),
            )
            .await?;
        // GitHub serves an environment from its newest deployment whose
        // latest status is `success`: a newer failed, pending or status-less
        // one (such as a rollback no workflow picked up) is not live.
        for deployment in &deployments {
            let statuses = self
                .raw_api(
                    &format!(
                        "/repos/{project}/deployments/{}/statuses?per_page=1",
                        deployment.id
                    ),
                    "GET",
                    None,
                    "Fetching Deployment Status",
                )
                .await?;
            if parse_latest_deployment_state(&statuses)?.as_deref() != Some("success") {
                continue;
            }
            let endpoint = format!("/repos/{project}/deployments/{}/statuses", deployment.id);
            let body = serde_json::json!({ "state": "inactive" }).to_string();
            return self
                .raw_api(&endpoint, "POST", Some(&body), "Stopping Environment")
                .await
                .map(|_| ());
        }
        Err(anyhow::anyhow!(
            "environment {name} has no active deployment to mark inactive"
        ))
    }

    async fn rollback_deployment(
        &self,
        project: &str,
        environment: &str,
        deployment: &Deployment,
    ) -> Result<()> {
        // By default GitHub verifies every status context on the commit and
        // refuses the deployment when one is not green. This SHA was already
        // deployed once, so the check is skipped.
        let body = serde_json::json!({
            "ref": deployment.sha,
            "environment": environment,
            "auto_merge": false,
            "required_contexts": [],
            "description": format!("Rollback to deployment #{}", deployment.id),
        })
        .to_string();
        self.raw_api(
            &format!("/repos/{project}/deployments"),
            "POST",
            Some(&body),
            "Rolling Back Deployment",
        )
        .await
        .map(|_| ())
    }

    // ── Labels / Members / Misc ──

    async fn fetch_labels(&self, scope: &Scope, _per_request: usize) -> Result<Vec<Label>> {
        let repo_arg = match scope {
            Scope::Repository(project) => project.as_str(),
            Scope::Group(_org) => return Ok(vec![]),
        };
        let raw = self
            .run_gh(
                &[
                    "label",
                    "list",
                    "--json",
                    "name,color",
                    "-R",
                    repo_arg,
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

    async fn fetch_members(&self, scope: &Scope) -> Result<Vec<String>> {
        let endpoint = match scope {
            Scope::Repository(project) => format!("/repos/{project}/assignees?per_page=100"),
            Scope::Group(org) => format!("/orgs/{org}/members?per_page=100"),
        };
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

    async fn list_group_projects(
        &self,
        group: &str,
        page_size: usize,
        per_request: usize,
    ) -> Result<Vec<String>> {
        let per = per_request.clamp(1, 100);
        let pages = page_size.div_ceil(per);
        let mut out = Vec::new();
        let mut is_user = false;

        for page in 1..=pages {
            let endpoint = if is_user {
                format!("/users/{group}/repos?per_page={per}&page={page}")
            } else {
                format!("/orgs/{group}/repos?per_page={per}&page={page}")
            };
            let desc = if is_user {
                "Fetching User Repos"
            } else {
                "Fetching Org Repos"
            };
            let raw = match self.raw_api(&endpoint, "GET", None, desc).await {
                Ok(raw) => raw,
                Err(e)
                    if !is_user
                        && (e.to_string().contains("404")
                            || e.to_string().to_lowercase().contains("not found")) =>
                {
                    is_user = true;
                    let user_endpoint = format!("/users/{group}/repos?per_page={per}&page={page}");
                    self.raw_api(&user_endpoint, "GET", None, "Fetching User Repos")
                        .await?
                }
                Err(e) => return Err(e),
            };
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

    // ── Browser ──

    async fn open_in_browser(&self, project: &str, entity: &str, id: &str) -> Result<()> {
        if !project.is_empty() {
            self.run_gh(
                &[entity, "view", id, "-R", project, "--web"],
                "OPENING IN BROWSER",
            )
            .await?;
        } else {
            self.run_gh(&[entity, "view", id, "--web"], "OPENING IN BROWSER")
                .await?;
        }
        Ok(())
    }

    async fn open_pipeline_in_browser(&self, project: &str, id: &str) -> Result<()> {
        if !project.is_empty() {
            self.run_gh(
                &["run", "view", id, "-R", project, "--web"],
                "OPENING IN BROWSER",
            )
            .await?;
        } else {
            self.run_gh(&["run", "view", id, "--web"], "OPENING IN BROWSER")
                .await?;
        }
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

    async fn open_job_in_browser(&self, project: &str, id: &str) -> Result<()> {
        if !project.is_empty() {
            self.run_gh(
                &["run", "view", id, "-R", project, "--web"],
                "OPENING IN BROWSER",
            )
            .await?;
        } else {
            self.run_gh(&["run", "view", id, "--web"], "OPENING IN BROWSER")
                .await?;
        }
        Ok(())
    }

    async fn open_milestone_in_browser(&self, project: &str, id: &str) -> Result<()> {
        let url = format!("https://github.com/{project}/milestone/{id}");
        open_github_url(&url, self.tx.clone(), "OPENING IN BROWSER").await
    }

    async fn open_runner_in_browser(&self, scope: &Scope, runner_id: u64) -> Result<()> {
        let url = github_runner_url(scope.as_str(), runner_id);
        open_github_url(&url, self.tx.clone(), "OPENING RUNNER IN BROWSER").await
    }

    async fn open_branch_in_browser(&self, project: &str, branch: &str) -> Result<()> {
        if project.is_empty() {
            anyhow::bail!("project path required to open branch in browser");
        }
        let url = format!(
            "https://github.com/{}/tree/{}",
            project,
            branch.replace('/', "%2F"),
        );
        open_github_url(&url, self.tx.clone(), "OPENING BRANCH IN BROWSER").await
    }

    async fn open_environment_in_browser(&self, project: &str, _name: &str) -> Result<()> {
        if project.is_empty() {
            anyhow::bail!("project path required to open environment in browser");
        }
        let url = format!("https://github.com/{project}/settings/environments");
        open_github_url(&url, self.tx.clone(), "OPENING ENVIRONMENT IN BROWSER").await
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
            async move { run_gh_raw_api(tx, &endpoint, &method, body.as_deref(), &desc).await }
        })
        .await
    }
}

async fn run_gh_raw_api(
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
    let logged = LoggedCommand::start(desc, Invocation::new(Program::Gh, &args).with_stdin(input));

    let output =
        super::subprocess::output(Command::new("gh").args(&args), input.map(str::as_bytes)).await;

    match output {
        Ok(out) => {
            if out.status.success() {
                logged.finish(tx.as_ref(), "Success".to_string(), (&out).into());
                Ok(String::from_utf8(out.stdout)?)
            } else {
                let child_output = ChildOutput::from(&out);
                let reason = logged.failure_reason(&child_output);
                logged.finish(tx.as_ref(), format!("Failed: {reason}"), child_output);
                anyhow::bail!("gh api failed: {reason}")
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

/// The self-hosted runner's settings page in `repository` (`owner/repo`).
fn github_runner_url(repository: &str, runner_id: u64) -> String {
    format!("https://github.com/{repository}/settings/actions/runners/{runner_id}")
}

/// Open `url` in the user's default browser via `git web--browse`. Logs the
/// command to the Terminal tab so it shows up next to the `gh` commands.
/// Always returns Ok so a failed browser launch does not break the UI flow;
/// the underlying `git web--browse` exit code is captured into the terminal
/// log instead.
async fn open_github_url(
    url: &str,
    tx: Option<tokio::sync::mpsc::UnboundedSender<crate::event::Event>>,
    label: &str,
) -> anyhow::Result<()> {
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

/// Why a failed GitHub job failed: its timeout, else the first failed step.
/// A job that fails before any step runs (a bad runner label) has neither.
fn github_job_failure_reason<'a>(
    conclusion: Option<&str>,
    mut steps: impl Iterator<Item = (&'a str, Option<&'a str>)>,
) -> Option<String> {
    if conclusion.is_some_and(|c| c.eq_ignore_ascii_case("timed_out")) {
        return Some("timed out".to_string());
    }
    steps
        .find(|(_, conclusion)| conclusion.is_some_and(|c| c.eq_ignore_ascii_case("failure")))
        .map(|(name, _)| format!("step \"{name}\" failed"))
}

/// Parse the JSON response from `gh api repos/{owner}/{repo}/actions/runs`.
pub fn parse_github_actions_runs(raw: &str) -> Result<Vec<Pipeline>> {
    #[derive(Deserialize)]
    struct GhRunsResponse {
        #[serde(default)]
        workflow_runs: Vec<GhRun>,
    }

    #[derive(Deserialize)]
    struct GhRun {
        id: u64,
        status: String,
        conclusion: Option<String>,
        #[serde(default)]
        head_branch: Option<String>,
        #[serde(default)]
        updated_at: String,
        #[serde(default)]
        created_at: Option<String>,
        #[serde(default)]
        run_started_at: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        display_title: Option<String>,
        #[serde(default)]
        head_sha: Option<String>,
        #[serde(default)]
        event: Option<String>,
        #[serde(default)]
        actor: Option<GhActor>,
        #[serde(default)]
        triggering_actor: Option<GhActor>,
        #[serde(default)]
        html_url: Option<String>,
        #[serde(default)]
        repository: Option<GhRepo>,
        #[serde(default)]
        head_commit: Option<GhHeadCommit>,
    }

    #[derive(Deserialize)]
    struct GhHeadCommit {
        #[serde(default)]
        message: String,
        #[serde(default)]
        author: Option<GhCommitAuthor>,
    }

    #[derive(Deserialize)]
    struct GhCommitAuthor {
        #[serde(default)]
        name: String,
    }

    #[derive(Deserialize)]
    struct GhRepo {
        #[serde(default)]
        full_name: String,
    }

    #[derive(Deserialize)]
    struct GhActor {
        #[serde(default)]
        login: Option<String>,
    }

    let res: GhRunsResponse = serde_json::from_str(raw)?;
    let pipelines: Vec<Pipeline> =
        res.workflow_runs
            .into_iter()
            .map(|r| {
                let status = match r.status.as_str() {
                    "completed" | "COMPLETED" => match r.conclusion.as_deref() {
                        Some("success") | Some("SUCCESS") => "success",
                        Some("failure") | Some("FAILURE") | Some("timed_out")
                        | Some("TIMED_OUT") => "failed",
                        Some("cancelled") | Some("CANCELLED") | Some("canceled")
                        | Some("CANCELED") => "canceled",
                        Some("skipped") | Some("SKIPPED") => "skipped",
                        _ => "failed",
                    },
                    "in_progress" | "IN_PROGRESS" => "running",
                    "queued" | "QUEUED" | "waiting" | "WAITING" | "requested" | "REQUESTED"
                    | "pending" | "PENDING" => "pending",
                    _ => "pending",
                }
                .to_string();
                let duration = r
                    .run_started_at
                    .as_deref()
                    .and_then(|started| chrono_duration(started, &r.updated_at));
                let actor_login = r
                    .triggering_actor
                    .and_then(|a| a.login)
                    .or_else(|| r.actor.and_then(|a| a.login))
                    .unwrap_or_default();
                let project_path = r.repository.map(|repo| repo.full_name).unwrap_or_default();
                let commit = r.head_commit.map(|commit| CommitSummary {
                    title: commit
                        .message
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_string(),
                    author_name: commit.author.map(|author| author.name).unwrap_or_default(),
                });
                Pipeline {
                    id: r.id,
                    status,
                    r#ref: r.head_branch.unwrap_or_default(),
                    updated_at: r.updated_at,
                    name: r.name.unwrap_or_default(),
                    display_title: r.display_title.unwrap_or_default(),
                    event: r.event.as_deref().unwrap_or_default().to_string(),
                    head_sha: r.head_sha.unwrap_or_default(),
                    actor_login,
                    duration_seconds: duration,
                    created_at: r.created_at,
                    source: r.event,
                    project_path,
                    web_url: r.html_url,
                    commit,
                }
            })
            .collect();

    Ok(pipelines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_github_notification_links_its_page_and_its_latest_comment() {
        let raw = r#"[
            {"id": "1", "unread": true, "reason": "mention", "updated_at": "2026-07-06T12:00:00Z",
             "repository": {"full_name": "acme/api", "html_url": "https://ghe.example/acme/api"},
             "subject": {"title": "Add retries", "type": "PullRequest",
                         "url": "https://ghe.example/api/v3/repos/acme/api/pulls/4",
                         "latest_comment_url": "https://ghe.example/api/v3/repos/acme/api/issues/comments/99"}},
            {"id": "2", "unread": false, "updated_at": "2026-07-05T12:00:00Z",
             "repository": {"full_name": "acme/api"},
             "subject": {"title": "Crash", "type": "Issue",
                         "url": "https://api.github.com/repos/acme/api/issues/9",
                         "latest_comment_url": null}},
            {"id": "3", "unread": true, "updated_at": "2026-07-04T12:00:00Z",
             "repository": {"full_name": "acme/api"},
             "subject": {"title": "CI", "type": "CheckSuite", "url": null, "latest_comment_url": null}}
        ]"#;

        let notifications = parse_github_notifications(raw).unwrap();

        let summary: Vec<_> = notifications
            .iter()
            .map(|n| {
                (
                    n.target_type.as_str(),
                    n.state.as_str(),
                    n.target_iid,
                    n.web_url.as_str(),
                    n.detail_api_path.as_deref(),
                    n.body.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "MergeRequest",
                    "unread",
                    4,
                    "https://ghe.example/acme/api/pull/4",
                    Some("repos/acme/api/issues/comments/99"),
                    None,
                ),
                (
                    "Issue",
                    "read",
                    9,
                    "https://github.com/acme/api/issues/9",
                    Some("repos/acme/api/issues/9"),
                    None,
                ),
                (
                    "CheckSuite",
                    "unread",
                    0,
                    "https://github.com/acme/api",
                    None,
                    None
                ),
            ]
        );
        assert_eq!(notifications[0].reason, "mention");
    }

    #[test]
    fn a_notification_detail_reads_the_comment_or_release_author() {
        let comment = r#"{"body": "Looks **good**", "user": {"login": "alice"}}"#;
        let release = r#"{"body": null, "author": {"login": "bob"}}"#;
        assert_eq!(
            parse_github_notification_detail(comment).unwrap(),
            NotificationDetail {
                author: "alice".to_string(),
                body: "Looks **good**".to_string(),
            }
        );
        assert_eq!(
            parse_github_notification_detail(release).unwrap(),
            NotificationDetail {
                author: "bob".to_string(),
                body: String::new(),
            }
        );
    }

    #[test]
    fn only_an_issue_or_pr_subject_is_read_with_a_native_command() {
        assert_eq!(
            native_detail_args("repos/acme/api/issues/9"),
            Some([
                "issue",
                "view",
                "9",
                "-R",
                "acme/api",
                "--json",
                "body,author"
            ])
        );
        assert_eq!(
            native_detail_args("repos/acme/api/pulls/4"),
            Some(["pr", "view", "4", "-R", "acme/api", "--json", "body,author"])
        );
        for raw_only in [
            "repos/acme/api/issues/comments/99",
            "repos/acme/api/pulls/comments/7",
            "repos/acme/api/releases/123",
            "repos/acme/api/commits/abc123",
            "repos/acme/api/issues/",
        ] {
            assert_eq!(native_detail_args(raw_only), None, "{raw_only}");
        }
    }

    #[test]
    fn a_pending_async_merge_is_polled_by_its_uuid() {
        let raw = r#"{"status":"pending","details":{"message":"Merge request accepted","uuid":"5f1c","merge_method":"squash","merge_action":"default","expected_head_sha":"abc"}}"#;
        assert_eq!(
            parse_async_merge_result(raw).unwrap(),
            AsyncMergeProgress::Pending {
                uuid: "5f1c".to_string()
            }
        );
    }

    #[test]
    fn merged_and_enqueued_async_merges_are_settled() {
        for raw in [
            r#"{"status":"merged","details":{"message":"Pull request merged","sha":"abc"}}"#,
            r#"{"status":"enqueued","details":{"message":"Added to the merge queue"}}"#,
        ] {
            assert_eq!(
                parse_async_merge_result(raw).unwrap(),
                AsyncMergeProgress::Settled
            );
        }
    }

    #[test]
    fn a_failed_async_merge_reports_githubs_message() {
        let raw = r#"{"status":"failed","details":{"message":"Required status check \"ci\" is failing"}}"#;
        assert_eq!(
            parse_async_merge_result(raw).unwrap_err().to_string(),
            "Required status check \"ci\" is failing"
        );
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
    fn review_comment_on_unchanged_line_uses_the_new_side() {
        let c = github_review_comment(&draft(Some(12), Some(10), None, None));
        assert_eq!(
            (c["side"].as_str(), c["line"].as_u64()),
            (Some("RIGHT"), Some(12))
        );
        assert!(c.get("start_line").is_none());
    }

    #[test]
    fn review_comment_range_on_old_side_spans_low_to_high() {
        let c = github_review_comment(&draft(None, Some(9), None, Some(5)));
        assert_eq!(c["side"], "LEFT");
        assert_eq!(c["start_side"], "LEFT");
        assert_eq!(
            (c["start_line"].as_u64(), c["line"].as_u64()),
            (Some(5), Some(9))
        );
    }

    #[test]
    fn review_payload_holds_every_comment_in_one_review() {
        let comments = [
            draft(Some(1), None, None, None),
            draft(None, Some(2), None, None),
        ];
        let payload = github_review_payload(ReviewEvent::RequestChanges, "summary", &comments);
        assert_eq!(payload["event"], "REQUEST_CHANGES");
        assert_eq!(payload["comments"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn encode_branch_passes_simple_names_through() {
        // A branch name with no slashes must round-trip unchanged so common
        // cases like "main" or "release-1.2" keep working without surprises.
        assert_eq!(GhBackend::encode_branch("main"), "main");
        assert_eq!(GhBackend::encode_branch("release-1.2"), "release-1.2");
        assert_eq!(GhBackend::encode_branch(""), "");
    }

    #[test]
    fn encode_branch_percent_encodes_every_slash() {
        // REGRESSION GUARD for #448: a branch like "feature/foo" must be
        // percent-encoded to "feature%2Ffoo" before it goes into the URL
        // path of `/git/refs/heads/{}`, otherwise GitHub treats the slash
        // as a path separator and returns 404/422.
        assert_eq!(GhBackend::encode_branch("feature/foo"), "feature%2Ffoo");
        // Nested slashes — every slash must be encoded, not just the first.
        assert_eq!(GhBackend::encode_branch("a/b/c"), "a%2Fb%2Fc");
    }

    #[test]
    fn parse_closed_by_pull_requests_references() {
        // The implementation queries `gh api graphql` rather than `gh issue view`,
        // because the latter only exposes the bare GraphQL nodes (no title/state).
        // Verify the parser strips the `data.repository.issue.…nodes` envelope
        // and lowercases the PR state to match the GitLab shape consumed by
        // the rest of the app.
        let raw = r#"{
            "data": {
                "repository": {
                    "issue": {
                        "closedByPullRequestsReferences": {
                            "nodes": [
                                { "number": 408, "title": "fix: guard bracket slice", "state": "MERGED" },
                                { "number": 407, "title": "fix(merge): stop --auto-merge=false", "state": "OPEN" }
                            ]
                        }
                    }
                }
            }
        }"#;
        use super::related_prs_types::*;
        let resp: GhResponse = serde_json::from_str(raw).unwrap();
        let refs: Vec<RelatedMrRef> = resp
            .data
            .and_then(|d| d.repository)
            .and_then(|r| r.issue)
            .map(|i| i.closed_by_pull_requests_references.nodes)
            .unwrap_or_default()
            .into_iter()
            .map(|p| RelatedMrRef {
                iid: p.number,
                title: p.title,
                state: p.state.to_lowercase(),
                project_path: None,
            })
            .collect();
        assert_eq!(
            refs,
            vec![
                RelatedMrRef {
                    iid: 408,
                    title: "fix: guard bracket slice".into(),
                    state: "merged".into(),
                    project_path: None,
                },
                RelatedMrRef {
                    iid: 407,
                    title: "fix(merge): stop --auto-merge=false".into(),
                    state: "open".into(),
                    project_path: None,
                },
            ]
        );
    }

    #[test]
    fn parse_issue_with_no_closing_prs_returns_empty() {
        let raw = r#"{ "data": { "repository": { "issue": { "closedByPullRequestsReferences": { "nodes": [] } } } } }"#;
        use super::related_prs_types::*;
        let resp: GhResponse = serde_json::from_str(raw).unwrap();
        let refs: Vec<RelatedMrRef> = resp
            .data
            .and_then(|d| d.repository)
            .and_then(|r| r.issue)
            .map(|i| i.closed_by_pull_requests_references.nodes)
            .unwrap_or_default()
            .into_iter()
            .map(|p| RelatedMrRef {
                iid: p.number,
                title: p.title,
                state: p.state.to_lowercase(),
                project_path: None,
            })
            .collect();
        assert!(refs.is_empty());
    }

    #[test]
    fn related_prs_query_has_no_literal_backslashes() {
        // Regression: the template once used a raw string with `\`-newline
        // continuations, which emitted literal backslashes into the GraphQL
        // query and made every GitHub fetch fail server-side.
        let query = related_prs_graphql_query("rcieri", "glab-tui", 409, 100);
        assert!(
            !query.contains('\\'),
            "query must not contain backslashes: {query}"
        );
        assert!(query.starts_with("{ repository(owner:\"rcieri\",name:\"glab-tui\") {"));
        assert!(query.contains("issue(number:409)"));
        assert!(query.contains("closedByPullRequestsReferences(first:100)"));
        assert!(query.contains("nodes { number title state }"));
        assert_eq!(
            query.matches('{').count(),
            query.matches('}').count(),
            "query braces must balance: {query}"
        );
    }

    #[test]
    fn related_prs_query_escapes_quotes_in_owner_and_repo() {
        let query = related_prs_graphql_query("org\"inj", "repo\"inj", 1, 10);
        assert!(
            !query.contains("org\"inj"),
            "unescaped double-quote must not appear: {query}"
        );
        assert!(query.contains("org\\\"inj"));
        assert!(query.contains("repo\\\"inj"));
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
    fn head_ref_stays_bare_when_target_is_the_local_project() {
        assert_eq!(
            qualify_head_ref("seflue/glab-tui", "seflue/glab-tui", "my-branch"),
            "my-branch"
        );
    }

    #[test]
    fn head_ref_gains_the_fork_owner_when_target_differs() {
        assert_eq!(
            qualify_head_ref("seflue/glab-tui", "rcieri/glab-tui", "my-branch"),
            "seflue:my-branch"
        );
    }

    #[test]
    fn head_ref_stays_bare_without_a_usable_local_project() {
        assert_eq!(
            qualify_head_ref("", "rcieri/glab-tui", "my-branch"),
            "my-branch"
        );
        assert_eq!(
            qualify_head_ref("no-slash", "rcieri/glab-tui", "my-branch"),
            "my-branch"
        );
    }

    #[test]
    fn head_ref_is_not_qualified_twice() {
        assert_eq!(
            qualify_head_ref("seflue/glab-tui", "rcieri/glab-tui", "seflue:my-branch"),
            "seflue:my-branch"
        );
    }

    #[test]
    fn test_normalize_labels() {
        assert_eq!(normalize_labels(""), "");
        assert_eq!(normalize_labels("bug, feature"), "bug,feature");
        assert_eq!(normalize_labels("bug,feature"), "bug,feature");
        assert_eq!(normalize_labels("bug"), "bug");
    }

    #[test]
    fn github_issue_json_produces_a_copyable_reference() {
        let raw = r#"{
            "number": 42,
            "title": "Fix parser",
            "state": "OPEN",
            "labels": [{"name": "bug"}],
            "author": {"login": "octocat"},
            "body": "Details",
            "createdAt": "2026-08-29T10:00:00Z",
            "updatedAt": "2026-08-29T11:00:00Z",
            "closedAt": null,
            "milestone": null,
            "assignees": [],
            "url": "https://github.com/acme/project/issues/42"
        }"#;

        let issue = parse_gh_issue(raw).unwrap();

        assert_eq!(
            issue.markdown_reference(),
            "[#42: Fix parser](https://github.com/acme/project/issues/42)"
        );
    }

    #[test]
    fn github_issue_list_json_keeps_each_copyable_url() {
        let raw = r#"[{
            "number": 42,
            "title": "Fix parser",
            "state": "OPEN",
            "labels": [],
            "author": {"login": "octocat"},
            "body": null,
            "createdAt": "2026-08-29T10:00:00Z",
            "updatedAt": "2026-08-29T11:00:00Z",
            "closedAt": null,
            "milestone": null,
            "assignees": [],
            "url": "https://github.com/acme/project/issues/42"
        }]"#;

        let issues = parse_gh_issues(raw).unwrap();

        assert_eq!(issues.len(), 1);
        assert_eq!(
            issues[0].markdown_reference(),
            "[#42: Fix parser](https://github.com/acme/project/issues/42)"
        );
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
    fn map_gh_pr_state_pins_three_way_classification() {
        // REGRESSION GUARD for #435. GitHub's `gh pr list --state all` and
        // `gh pr view --json state,...` return one of "OPEN", "CLOSED", or
        // "MERGED". The MRs table renders a purple MERGED badge for
        // `m.state == "merged"` (`src/ui/tabs.rs:406`), and the State column
        // filter exposes a "MERGED" value (`src/app.rs:4078`) — but those
        // both read the `state` string the backend hands them. If the
        // backend collapses "MERGED" to "closed", the merged PR silently
        // shows up as CLOSED with no error.
        assert_eq!(map_gh_pr_state("OPEN"), "opened");
        assert_eq!(map_gh_pr_state("MERGED"), "merged");
        assert_eq!(map_gh_pr_state("CLOSED"), "closed");
    }

    #[test]
    fn map_gh_pr_state_unknown_values_collapse_to_closed() {
        // Forward-compat: GitHub has historically used uppercase ("OPEN") but
        // if a future API response includes an unrecognised variant, fall
        // through to "closed" rather than panic or surface a wrong badge.
        assert_eq!(map_gh_pr_state(""), "closed");
        assert_eq!(map_gh_pr_state("open"), "closed"); // case-sensitive on purpose
        assert_eq!(map_gh_pr_state("DRAFT"), "closed");
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

    #[test]
    fn test_parse_github_actions_runs_with_actors() {
        let json = r#"{
            "total_count": 1,
            "workflow_runs": [
                {
                    "id": 12345,
                    "name": "CI",
                    "display_title": "Fix bug",
                    "head_branch": "main",
                    "head_sha": "abcdef123456",
                    "status": "completed",
                    "conclusion": "success",
                    "event": "push",
                    "created_at": "2026-08-27T10:00:00Z",
                    "updated_at": "2026-08-27T10:02:00Z",
                    "run_started_at": "2026-08-27T10:00:00Z",
                    "actor": { "login": "committer" },
                    "triggering_actor": { "login": "triggerer" }
                }
            ]
        }"#;

        let pipelines = parse_github_actions_runs(json).unwrap();
        assert_eq!(pipelines.len(), 1);
        let p = &pipelines[0];
        assert_eq!(p.id, 12345);
        assert_eq!(p.status, "success");
        assert_eq!(p.name, "CI");
        assert_eq!(p.display_title, "Fix bug");
        assert_eq!(p.r#ref, "main");
        assert_eq!(p.head_sha, "abcdef123456");
        assert_eq!(p.event, "push");
        assert_eq!(p.actor_login, "triggerer");
        assert_eq!(p.duration_seconds, Some(120));
    }

    #[test]
    fn run_head_commit_becomes_the_pipeline_commit_headline_and_author() {
        let json = r#"{"workflow_runs": [{
            "id": 7, "status": "completed", "conclusion": "failure", "updated_at": "",
            "head_commit": {
                "message": "Fix flaky login test\n\nThe retry hid a race.",
                "author": { "name": "Ada Lovelace", "email": "ada@example.com" }
            }
        }]}"#;

        let pipelines = parse_github_actions_runs(json).unwrap();

        assert_eq!(
            pipelines[0].commit,
            Some(CommitSummary {
                title: "Fix flaky login test".to_string(),
                author_name: "Ada Lovelace".to_string(),
            })
        );
    }

    #[test]
    fn failed_job_reason_names_its_timeout_or_its_first_failed_step() {
        let steps = [
            ("Checkout", Some("success")),
            ("Run tests", Some("failure")),
            ("Upload", Some("failure")),
        ];
        let reason = |conclusion| github_job_failure_reason(conclusion, steps.iter().copied());

        assert_eq!(
            reason(Some("failure")).as_deref(),
            Some("step \"Run tests\" failed")
        );
        assert_eq!(reason(Some("timed_out")).as_deref(), Some("timed out"));
        assert_eq!(
            github_job_failure_reason(Some("failure"), std::iter::empty()),
            None
        );
    }

    #[test]
    fn test_parse_org_repos() {
        let json = r#"[
            {"full_name": "myorg/repo1"},
            {"full_name": "myorg/repo2"}
        ]"#;
        let names = parse_group_projects(json).unwrap();
        assert_eq!(names, vec!["myorg/repo1", "myorg/repo2"]);

        // Empty array
        let empty_names = parse_group_projects("[]").unwrap();
        assert!(empty_names.is_empty());

        // Malformed / missing field
        assert!(parse_group_projects(r#"[{"id": 1}]"#).is_err());
    }

    #[test]
    fn test_pr_stack_graphql_query() {
        let q = pr_stack_graphql_query("rcieri", "glab-tui", &[101, 102]);
        assert!(q.contains("pr_101: pullRequest(number: 101)"));
        assert!(q.contains("pr_102: pullRequest(number: 102)"));
        assert!(q.contains("stack { number size }"));
        assert!(q.contains("stackEntry { position }"));
    }

    #[test]
    fn test_parse_pr_stacks_response() {
        let json = r#"{
            "data": {
                "repository": {
                    "pr_101": {
                        "number": 101,
                        "stack": { "number": 7, "size": 3 },
                        "stackEntry": { "position": 1 }
                    },
                    "pr_102": {
                        "number": 102,
                        "stack": { "number": 7, "size": 3 },
                        "stackEntry": { "position": 2 }
                    },
                    "pr_103": {
                        "number": 103,
                        "stack": null,
                        "stackEntry": null
                    }
                }
            }
        }"#;
        let stacks = parse_pr_stacks_response(json);
        assert_eq!(stacks.len(), 2);
        let s101 = stacks.get(&101).unwrap();
        assert_eq!(s101.number, 7);
        assert_eq!(s101.size, 3);
        assert_eq!(s101.position, 1);

        let s102 = stacks.get(&102).unwrap();
        assert_eq!(s102.number, 7);
        assert_eq!(s102.size, 3);
        assert_eq!(s102.position, 2);

        assert!(!stacks.contains_key(&103));
    }

    #[test]
    fn review_thread_resolution_reads_every_page_gh_paginate_prints() {
        let raw = r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{
            "pageInfo":{"hasNextPage":true,"endCursor":"c1"},"nodes":[
            {"isResolved":true,"comments":{"nodes":[{"databaseId":11}]}},
            {"isResolved":false,"comments":{"nodes":[{"databaseId":21}]}}
        ]}}}}}
        {"data":{"repository":{"pullRequest":{"reviewThreads":{
            "pageInfo":{"hasNextPage":false,"endCursor":"c2"},"nodes":[
            {"isResolved":false,"comments":{"nodes":[{"databaseId":31}]}},
            {"isResolved":true,"comments":{"nodes":[]}}
        ]}}}}}"#;
        let map = parse_review_thread_resolution(raw).unwrap();
        assert_eq!(map, HashMap::from([(11, true), (21, false), (31, false)]));
    }

    #[test]
    fn review_thread_resolution_graphql_error_on_a_later_page_is_an_error() {
        let raw = r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{"nodes":[]}}}}}
            {"data":null,"errors":[{"message":"API rate limit exceeded"}]}"#;
        let err = parse_review_thread_resolution(raw).unwrap_err();
        assert_eq!(err.to_string(), "GraphQL error: API rate limit exceeded");
    }

    fn review_comment_json(id: u64, in_reply_to_id: Option<u64>) -> String {
        let in_reply_to_id = in_reply_to_id.map_or("null".to_string(), |id| id.to_string());
        format!(
            r#"{{"id":{id},"body":"comment {id}","user":{{"login":"octo"}},
            "created_at":"2026-07-06T09:00:00Z","path":"src/lib.rs","line":3,
            "side":"RIGHT","start_line":null,"in_reply_to_id":{in_reply_to_id}}}"#
        )
    }

    #[test]
    fn review_comments_on_every_page_take_their_threads_resolution() {
        let raw = format!(
            "[{},{}]\n[{},{},{}]",
            review_comment_json(11, None),
            review_comment_json(21, None),
            review_comment_json(12, Some(11)),
            review_comment_json(31, None),
            review_comment_json(41, None),
        );
        let resolution = HashMap::from([(11, true), (21, false), (31, false)]);
        let notes = parse_pr_review_comments(&raw, &resolution).unwrap();
        let states: Vec<_> = notes
            .iter()
            .map(|n| (n.id, n.discussion_id.as_deref(), n.resolvable, n.resolved))
            .collect();
        assert_eq!(
            states,
            vec![
                (11, Some("11"), Some(true), Some(true)),
                (21, Some("21"), Some(true), Some(false)),
                (12, Some("11"), Some(true), Some(true)),
                (31, Some("31"), Some(true), Some(false)),
                (41, Some("41"), Some(false), None),
            ]
        );
    }

    #[test]
    fn review_thread_resolution_graphql_errors_are_errors() {
        let raw = r#"{"data":null,"errors":[{"message":"Could not resolve to a Repository"}]}"#;
        let err = parse_review_thread_resolution(raw).unwrap_err();
        assert!(err.to_string().contains("Could not resolve"));
    }

    #[test]
    fn stack_query_response_carries_the_prs_position_and_its_entries_in_order() {
        let json = r#"{
            "data": {
                "repository": {
                    "pullRequest": {
                        "stackEntry": { "position": 2 },
                        "stack": {
                            "number": 7,
                            "size": 3,
                            "entries": {
                                "nodes": [
                                    {
                                        "position": 2,
                                        "pullRequest": {
                                            "number": 102,
                                            "title": "Middle PR",
                                            "state": "OPEN",
                                            "isDraft": false
                                        }
                                    },
                                    {
                                        "position": 1,
                                        "pullRequest": {
                                            "number": 101,
                                            "title": "Base PR",
                                            "state": "MERGED",
                                            "isDraft": false
                                        }
                                    },
                                    {
                                        "position": 3,
                                        "pullRequest": {
                                            "number": 103,
                                            "title": "Top PR",
                                            "state": "OPEN",
                                            "isDraft": true
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        }"#;
        let stack = parse_pr_stack_entries_response(json).unwrap().unwrap();
        let entry =
            |position, number, title: &str, state: &str, is_draft| crate::domain::mr::StackEntry {
                position,
                number,
                title: title.to_string(),
                state: state.to_string(),
                is_draft,
            };
        assert_eq!(
            stack,
            crate::domain::mr::PrStack {
                info: crate::domain::mr::StackInfo {
                    number: 7,
                    size: 3,
                    position: 2,
                },
                entries: vec![
                    entry(1, 101, "Base PR", "merged", false),
                    entry(2, 102, "Middle PR", "open", false),
                    entry(3, 103, "Top PR", "open", true),
                ],
            }
        );
    }

    #[test]
    fn stack_query_response_without_a_stack_entry_means_not_stacked() {
        let json = r#"{"data":{"repository":{"pullRequest":{"stackEntry":null,"stack":null}}}}"#;
        assert_eq!(parse_pr_stack_entries_response(json).unwrap(), None);
    }

    #[test]
    fn gh_merge_args_strategy_and_auto_merge() {
        // auto + squash + delete
        let args = GhBackend::merge_args("owner/repo", 42, true, true, None, true);
        assert!(args.contains(&"42".to_string()), "iid must be present");
        assert!(
            args.contains(&"owner/repo".to_string()),
            "project must be present"
        );
        assert!(args.contains(&"--auto".to_string()));
        assert!(args.contains(&"--squash".to_string()));
        assert!(args.contains(&"--delete-branch".to_string()));
        // squash takes priority — no --merge alongside it
        assert!(!args.contains(&"--merge".to_string()));

        // rebase, no auto-merge
        let args_rebase =
            GhBackend::merge_args("owner/repo", 42, false, false, Some("rebase"), false);
        assert!(!args_rebase.contains(&"--auto".to_string()));
        assert!(args_rebase.contains(&"--rebase".to_string()));
        assert!(!args_rebase.contains(&"--merge".to_string()));

        // strategy=None (merge commit) → explicit --merge, not relying on repo default
        let args_merge_commit = GhBackend::merge_args("owner/repo", 42, false, false, None, false);
        assert!(
            args_merge_commit.contains(&"--merge".to_string()),
            "merge commit strategy must send --merge explicitly"
        );
        assert!(!args_merge_commit.contains(&"--squash".to_string()));
        assert!(!args_merge_commit.contains(&"--rebase".to_string()));
        assert!(!args_merge_commit.contains(&"--auto".to_string()));
    }

    #[test]
    fn only_githubs_too_large_diff_error_triggers_the_local_fallback() {
        let too_large = anyhow::anyhow!(
            "gh command failed: could not find pull request diff: HTTP 406: Sorry, the diff \
             exceeded the maximum number of lines (20000) \
             (https://api.github.com/repos/o/r/pulls/1)\nPullRequest.diff too_large"
        );
        let not_found = anyhow::anyhow!(
            "gh command failed: GraphQL: Could not resolve to a PullRequest with the number of 9."
        );
        assert!(is_pr_diff_too_large(&too_large));
        assert!(!is_pr_diff_too_large(&not_found));
    }

    /// Runs git in `dir` with an identity and no signing, so the developer's
    /// global config cannot break commit creation.
    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=glab-tui",
                "-c",
                "user.email=glab-tui@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    /// Everything the fallback promises not to touch in a clone.
    fn clone_state(clone: &std::path::Path) -> [String; 4] {
        [
            git(clone, &["symbolic-ref", "HEAD"]),
            git(clone, &["for-each-ref"]),
            git(clone, &["status", "--porcelain=v1"]),
            git(clone, &["diff", "--cached"]),
        ]
    }

    #[tokio::test]
    async fn local_pr_diff_fetches_missing_commits_without_touching_the_clone() {
        let root = tempfile::tempdir().unwrap();
        let origin = root.path().join("origin.git");
        let author = root.path().join("author");
        let clone = root.path().join("clone");
        std::fs::create_dir_all(&author).unwrap();
        git(
            root.path(),
            &[
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                origin.to_str().unwrap(),
            ],
        );
        git(&author, &["init", "-q", "-b", "main"]);
        git(
            &author,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        std::fs::write(author.join("kept.txt"), "unchanged\n").unwrap();
        std::fs::write(author.join("edited.txt"), "before\n").unwrap();
        git(&author, &["add", "."]);
        git(&author, &["commit", "-q", "-m", "base"]);
        git(&author, &["push", "-q", "origin", "main"]);
        let base = git(&author, &["rev-parse", "HEAD"]).trim().to_string();
        std::fs::write(author.join("edited.txt"), "after\n").unwrap();
        std::fs::write(author.join("added.txt"), "new\n").unwrap();
        git(&author, &["add", "."]);
        git(&author, &["commit", "-q", "-m", "head"]);
        let head = git(&author, &["rev-parse", "HEAD"]).trim().to_string();
        git(&author, &["push", "-q", "origin", "HEAD:refs/pull/7/head"]);

        git(
            root.path(),
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );
        std::fs::write(clone.join("kept.txt"), "staged local edit\n").unwrap();
        git(&clone, &["add", "kept.txt"]);
        std::fs::write(clone.join("edited.txt"), "unstaged local edit\n").unwrap();
        let before = clone_state(&clone);

        let commits = PrDiffCommits {
            base,
            head: head.clone(),
        };
        let diff = diff_pr_in_checkout(None, clone.to_str().unwrap(), 7, &commits)
            .await
            .unwrap();

        let files: Vec<&str> = diff
            .lines()
            .filter(|line| line.starts_with("diff --git "))
            .collect();
        assert_eq!(
            files,
            [
                "diff --git a/added.txt b/added.txt",
                "diff --git a/edited.txt b/edited.txt",
            ]
        );
        assert_eq!(clone_state(&clone), before);
        git(&clone, &["cat-file", "-e", &format!("{head}^{{commit}}")]);
    }
}
