//! Summaries the Dashboard tab draws from the lists the other tabs fetch.
//! Everything here is a pure function of those lists, so the dashboard never
//! issues a call of its own.

use crate::domain::contributions::Contribution;
use crate::domain::issues::Issue;
use crate::domain::milestones::Milestone;
use crate::domain::mr::MergeRequest;
use crate::domain::mr_state::WorkflowStatus;
use crate::domain::pipelines::Pipeline;
use chrono::{DateTime, Local, NaiveDate, Utc};
use std::collections::BTreeSet;

/// How many of the newest pipelines the health summary looks at.
pub const PIPELINE_HISTORY_LEN: usize = 20;
/// How many refs the pipeline card lists with their latest status.
pub const LATEST_REFS_SHOWN: usize = 3;
/// An open issue due within this many days counts as due soon.
pub const DUE_SOON_DAYS: i64 = 7;
/// An open MR/PR not updated for this many days counts as idle.
pub const MR_IDLE_DAYS: i64 = 7;
/// An open issue not updated for this many days counts as idle.
pub const ISSUE_IDLE_DAYS: i64 = 14;

const DUE_DATE_FORMAT: &str = "%Y-%m-%d";

/// The moment a summary is taken at. Idle ages count from `instant`; due
/// dates are calendar dates, so they count from the local `today`.
#[derive(Debug, Clone, Copy)]
pub struct Now {
    pub instant: DateTime<Utc>,
    pub today: NaiveDate,
}

impl Now {
    pub fn current() -> Self {
        Self {
            instant: Utc::now(),
            today: Local::now().date_naive(),
        }
    }
}

/// Whose work a summary is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Person<'a> {
    Everyone,
    User(&'a str),
}

/// The signed-in user's login, as the MR/PR list reports it with each review
/// state. `None` until a list with review states has loaded.
pub fn signed_in_user(mrs: &[MergeRequest]) -> Option<&str> {
    mrs.iter().find_map(|mr| {
        mr.approval
            .as_ref()?
            .current_user
            .as_deref()
            .filter(|login| !login.is_empty())
    })
}

/// Days from `today` to a `YYYY-MM-DD` date, negative once it has passed.
pub fn days_until(date: Option<&str>, today: NaiveDate) -> Option<i64> {
    let date = NaiveDate::parse_from_str(date?.trim(), DUE_DATE_FORMAT).ok()?;
    Some((date - today).num_days())
}

/// Whole days since an RFC 3339 timestamp. `None` when it does not parse.
pub fn days_since(timestamp: &str, now: DateTime<Utc>) -> Option<i64> {
    let then = DateTime::parse_from_rfc3339(timestamp).ok()?;
    Some((now - then.with_timezone(&Utc)).num_days())
}

fn is_idle(updated_at: &str, idle_days: i64, now: DateTime<Utc>) -> bool {
    days_since(updated_at, now).is_some_and(|days| days >= idle_days)
}

fn is_open_mr(mr: &MergeRequest) -> bool {
    mr.state == "opened"
}

fn is_open_issue(issue: &Issue) -> bool {
    issue.state == "opened"
}

fn is_mr_owner(mr: &MergeRequest, login: &str) -> bool {
    mr.author.username == login || mr.assignees.iter().any(|a| a.username == login)
}

fn is_mr_reviewer(mr: &MergeRequest, login: &str) -> bool {
    mr.reviewers.iter().any(|r| r.username == login)
}

/// Whether `login` authored, is assigned to or reviews the MR/PR.
pub fn mr_involves(mr: &MergeRequest, login: &str) -> bool {
    is_mr_owner(mr, login) || is_mr_reviewer(mr, login)
}

fn is_issue_assignee(issue: &Issue, login: &str) -> bool {
    issue.assignees.iter().any(|a| a.username == login)
}

/// Whether `login` opened or is assigned to the issue.
pub fn issue_involves(issue: &Issue, login: &str) -> bool {
    issue.author.username == login || is_issue_assignee(issue, login)
}

fn is_approved(mr: &MergeRequest) -> bool {
    mr.approval.as_ref().is_some_and(|a| a.approved)
}

fn has_changes_requested(mr: &MergeRequest) -> bool {
    mr.approval.as_ref().is_some_and(|a| a.changes_requested)
}

fn is_blocked(mr: &MergeRequest) -> bool {
    mr.mergeability
        .as_ref()
        .is_some_and(|m| m.conflicts || m.needs_rebase)
}

fn is_ready_to_merge(mr: &MergeRequest) -> bool {
    is_approved(mr) && !mr.draft && !is_blocked(mr)
}

/// The fetch derived the signed-in user's workflow status, which alone knows
/// whether they reviewed. For anyone else an approval stands in for a review.
fn signed_in_workflow(
    mr: &MergeRequest,
    login: &str,
    signed_in: Option<&str>,
) -> Option<WorkflowStatus> {
    if signed_in == Some(login) {
        mr.workflow
    } else {
        None
    }
}

fn is_returned_to(mr: &MergeRequest, login: &str, signed_in: Option<&str>) -> bool {
    if let Some(workflow) = signed_in_workflow(mr, login, signed_in) {
        return workflow == WorkflowStatus::ReturnedToYou;
    }
    mr.author.username == login && has_changes_requested(mr)
}

fn awaits_review_from(mr: &MergeRequest, login: &str, signed_in: Option<&str>) -> bool {
    if let Some(workflow) = signed_in_workflow(mr, login, signed_in) {
        return workflow == WorkflowStatus::ReviewRequested;
    }
    let has_approved = mr
        .approval
        .as_ref()
        .is_some_and(|a| a.approved_by.iter().any(|u| u == login));
    is_mr_reviewer(mr, login) && !has_approved && !is_returned_to(mr, login, signed_in)
}

/// A slice of the open MRs/PRs the dashboard counts and lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MrQueue {
    /// Every open one, or the ones a person authored, is assigned or reviews.
    Open,
    /// Not yet reviewed: everyone's are the non-draft ones neither approved
    /// nor sent back, a person's are the ones they review and have not.
    AwaitingReview,
    /// Changes requested, a person's only on the ones they authored.
    ChangesRequested,
    /// Approved, not a draft, no conflict and no rebase needed.
    ReadyToMerge,
    /// A conflict or a needed rebase.
    Blocked,
    /// Not updated for `MR_IDLE_DAYS`.
    Idle,
    Drafts,
}

impl MrQueue {
    pub const ALL: [MrQueue; 7] = [
        MrQueue::Open,
        MrQueue::AwaitingReview,
        MrQueue::ChangesRequested,
        MrQueue::ReadyToMerge,
        MrQueue::Blocked,
        MrQueue::Idle,
        MrQueue::Drafts,
    ];

    /// `signed_in` is the signed-in user's login, whose review state the
    /// fetch knows better than anyone else's.
    pub fn holds(
        self,
        mr: &MergeRequest,
        person: Person,
        signed_in: Option<&str>,
        now: Now,
    ) -> bool {
        if !is_open_mr(mr) {
            return false;
        }
        let is_idle = || is_idle(&mr.updated_at, MR_IDLE_DAYS, now.instant);
        match person {
            Person::Everyone => match self {
                MrQueue::Open => true,
                MrQueue::AwaitingReview => {
                    !mr.draft && !is_approved(mr) && !has_changes_requested(mr)
                }
                MrQueue::ChangesRequested => has_changes_requested(mr),
                MrQueue::ReadyToMerge => is_ready_to_merge(mr),
                MrQueue::Blocked => is_blocked(mr),
                MrQueue::Idle => is_idle(),
                MrQueue::Drafts => mr.draft,
            },
            Person::User(login) => match self {
                MrQueue::Open => mr_involves(mr, login),
                MrQueue::AwaitingReview => awaits_review_from(mr, login, signed_in),
                MrQueue::ChangesRequested => is_returned_to(mr, login, signed_in),
                MrQueue::ReadyToMerge => is_mr_owner(mr, login) && is_ready_to_merge(mr),
                MrQueue::Blocked => is_mr_owner(mr, login) && is_blocked(mr),
                MrQueue::Idle => mr_involves(mr, login) && is_idle(),
                MrQueue::Drafts => mr.author.username == login && mr.draft,
            },
        }
    }

    pub fn count(
        self,
        mrs: &[MergeRequest],
        person: Person,
        signed_in: Option<&str>,
        now: Now,
    ) -> usize {
        mrs.iter()
            .filter(|mr| self.holds(mr, person, signed_in, now))
            .count()
    }

    /// The MRs/PRs in the queue: idle ones longest idle first, the others
    /// most recently updated first.
    pub fn list<'a>(
        self,
        mrs: &'a [MergeRequest],
        person: Person,
        signed_in: Option<&str>,
        now: Now,
    ) -> Vec<&'a MergeRequest> {
        let mut listed: Vec<&MergeRequest> = mrs
            .iter()
            .filter(|mr| self.holds(mr, person, signed_in, now))
            .collect();
        sort_by_update(&mut listed, |mr| &mr.updated_at, self == MrQueue::Idle);
        listed
    }
}

/// A slice of the open issues the dashboard counts and lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IssueQueue {
    /// Every open issue, or the ones assigned to a person.
    Open,
    /// Past their due date, a person's among the ones assigned to them.
    Overdue,
    /// Due within `DUE_SOON_DAYS`, a person's among the ones assigned to them.
    DueSoon,
    /// Not updated for `ISSUE_IDLE_DAYS`, a person's among the ones assigned
    /// to them.
    Idle,
    /// Nobody assigned, a person's among the ones they opened.
    Unassigned,
}

impl IssueQueue {
    pub const ALL: [IssueQueue; 5] = [
        IssueQueue::Open,
        IssueQueue::Overdue,
        IssueQueue::DueSoon,
        IssueQueue::Idle,
        IssueQueue::Unassigned,
    ];

    /// GitHub issues carry no due date, so these queues are always empty there.
    pub fn needs_due_dates(self) -> bool {
        matches!(self, IssueQueue::Overdue | IssueQueue::DueSoon)
    }

    pub fn holds(self, issue: &Issue, person: Person, now: Now) -> bool {
        if !is_open_issue(issue) {
            return false;
        }
        let is_theirs = match (self, person) {
            (_, Person::Everyone) => true,
            (IssueQueue::Unassigned, Person::User(login)) => issue.author.username == login,
            (_, Person::User(login)) => is_issue_assignee(issue, login),
        };
        let due_in = || days_until(issue.due_date.as_deref(), now.today);
        is_theirs
            && match self {
                IssueQueue::Open => true,
                IssueQueue::Overdue => due_in().is_some_and(|days| days < 0),
                IssueQueue::DueSoon => {
                    due_in().is_some_and(|days| (0..=DUE_SOON_DAYS).contains(&days))
                }
                IssueQueue::Idle => is_idle(&issue.updated_at, ISSUE_IDLE_DAYS, now.instant),
                IssueQueue::Unassigned => issue.assignees.is_empty(),
            }
    }

    pub fn count(self, issues: &[Issue], person: Person, now: Now) -> usize {
        issues
            .iter()
            .filter(|issue| self.holds(issue, person, now))
            .count()
    }

    /// The issues in the queue: overdue and due-soon ones soonest due first,
    /// idle ones longest idle first, the others most recently updated first.
    pub fn list<'a>(self, issues: &'a [Issue], person: Person, now: Now) -> Vec<&'a Issue> {
        let mut listed: Vec<&Issue> = issues
            .iter()
            .filter(|issue| self.holds(issue, person, now))
            .collect();
        match self {
            IssueQueue::Overdue | IssueQueue::DueSoon => {
                listed.sort_by_key(|issue| days_until(issue.due_date.as_deref(), now.today));
            }
            IssueQueue::Idle => sort_by_update(&mut listed, |issue| &issue.updated_at, true),
            IssueQueue::Open | IssueQueue::Unassigned => {
                sort_by_update(&mut listed, |issue| &issue.updated_at, false);
            }
        }
        listed
    }
}

/// Sort by an RFC 3339 update timestamp, newest first unless
/// `is_oldest_first`. Rows whose timestamp does not parse sort last.
fn sort_by_update<T>(rows: &mut [T], updated_at: impl Fn(&T) -> &str, is_oldest_first: bool) {
    rows.sort_by_cached_key(|row| {
        let timestamp = DateTime::parse_from_rfc3339(updated_at(row)).ok();
        let missing = timestamp.is_none();
        let seconds = timestamp.map_or(0, |t| t.timestamp());
        (missing, if is_oldest_first { seconds } else { -seconds })
    });
}

/// What one person, or everyone, has open.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Workload {
    /// Open MRs/PRs they authored or are assigned (everyone's: every open one).
    pub open_mrs: usize,
    /// Open MRs/PRs waiting for their review.
    pub reviews: usize,
    /// Open issues assigned to them (everyone's: every open one).
    pub open_issues: usize,
    pub overdue_issues: usize,
}

pub fn workload(
    issues: &[Issue],
    mrs: &[MergeRequest],
    person: Person,
    signed_in: Option<&str>,
    now: Now,
) -> Workload {
    let open_mrs = match person {
        Person::Everyone => MrQueue::Open.count(mrs, person, signed_in, now),
        Person::User(login) => mrs
            .iter()
            .filter(|mr| is_open_mr(mr) && is_mr_owner(mr, login))
            .count(),
    };
    Workload {
        open_mrs,
        reviews: MrQueue::AwaitingReview.count(mrs, person, signed_in, now),
        open_issues: IssueQueue::Open.count(issues, person, now),
        overdue_issues: IssueQueue::Overdue.count(issues, person, now),
    }
}

/// The span of the activity feed the Contributors card counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActivityWindow {
    Day,
    Week,
    Month,
}

impl ActivityWindow {
    pub fn days(self) -> i64 {
        match self {
            ActivityWindow::Day => 1,
            ActivityWindow::Week => 7,
            ActivityWindow::Month => crate::domain::contributions::HISTORY_DAYS,
        }
    }

    pub fn starts_at(self, now: Now) -> DateTime<Utc> {
        now.instant - chrono::Duration::days(self.days())
    }
}

/// What one person, or everyone, did in an activity window.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ActivityCounts {
    pub commits: usize,
    pub comments: usize,
    pub threads: usize,
    pub reviews: usize,
}

/// Whether a contribution was made after `since`. One whose time does not
/// parse is kept rather than silently dropped.
pub fn is_made_after(contribution: &Contribution, since: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(&contribution.created_at)
        .map_or(true, |at| at.with_timezone(&Utc) > since)
}

pub fn activity_counts(
    contributions: &[Contribution],
    person: Person,
    since: DateTime<Utc>,
) -> ActivityCounts {
    use crate::domain::contributions::ContributionKind;
    let mut counts = ActivityCounts::default();
    let made = contributions.iter().filter(|contribution| {
        Activity::Contribution(contribution).involves(person) && is_made_after(contribution, since)
    });
    for contribution in made {
        match contribution.kind {
            ContributionKind::Commit => counts.commits += 1,
            ContributionKind::Comment => counts.comments += 1,
            ContributionKind::ThreadComment => counts.threads += 1,
            ContributionKind::Review => counts.reviews += 1,
        }
    }
    counts
}

/// Everyone taking part in an open issue or MR/PR, or in the activity feed,
/// other than the signed-in user, by login.
pub fn contributor_logins<'a>(
    issues: &'a [Issue],
    mrs: &'a [MergeRequest],
    contributions: &'a [Contribution],
    signed_in: Option<&str>,
) -> Vec<&'a str> {
    let mut logins: BTreeSet<&str> = BTreeSet::new();
    for mr in mrs.iter().filter(|mr| is_open_mr(mr)) {
        logins.insert(&mr.author.username);
        logins.extend(mr.assignees.iter().map(|a| a.username.as_str()));
        logins.extend(mr.reviewers.iter().map(|r| r.username.as_str()));
    }
    for issue in issues.iter().filter(|issue| is_open_issue(issue)) {
        logins.insert(&issue.author.username);
        logins.extend(issue.assignees.iter().map(|a| a.username.as_str()));
    }
    logins.extend(contributions.iter().map(|c| c.author.as_str()));
    logins
        .into_iter()
        .filter(|login| !login.is_empty() && Some(*login) != signed_in)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineOutcome {
    Succeeded,
    Failed,
    InProgress,
    /// Canceled, skipped or waiting on a manual action: no verdict on the code.
    Inconclusive,
}

impl PipelineOutcome {
    /// Backends normalise pipeline status to GitLab's vocabulary.
    pub fn of(status: &str) -> Self {
        match status {
            "success" => Self::Succeeded,
            "failed" => Self::Failed,
            "running"
            | "pending"
            | "created"
            | "waiting_for_resource"
            | "preparing"
            | "scheduled" => Self::InProgress,
            _ => Self::Inconclusive,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PipelineHealth<'a> {
    /// The newest pipelines, newest first, at most `PIPELINE_HISTORY_LEN`.
    pub recent: &'a [Pipeline],
    /// Succeeded out of succeeded plus failed in `recent`. `None` while none
    /// of them has finished either way.
    pub success_rate_percent: Option<u8>,
    pub in_progress: usize,
    pub failed: usize,
    /// The newest pipeline of each ref, in the order the refs last ran.
    pub latest_per_ref: Vec<&'a Pipeline>,
}

/// `pipelines` must be newest first, the order both backends list them in.
pub fn assess_pipelines(pipelines: &[Pipeline]) -> PipelineHealth<'_> {
    let recent = &pipelines[..pipelines.len().min(PIPELINE_HISTORY_LEN)];
    let outcomes = || recent.iter().map(|p| PipelineOutcome::of(&p.status));
    let succeeded = outcomes()
        .filter(|o| *o == PipelineOutcome::Succeeded)
        .count();
    let failed = outcomes().filter(|o| *o == PipelineOutcome::Failed).count();
    let in_progress = outcomes()
        .filter(|o| *o == PipelineOutcome::InProgress)
        .count();
    let finished = succeeded + failed;
    let success_rate_percent = (finished > 0).then(|| {
        u8::try_from(succeeded * 100 / finished).expect("a share of a whole is at most 100")
    });

    let mut latest_per_ref: Vec<&Pipeline> = Vec::with_capacity(LATEST_REFS_SHOWN);
    for pipeline in pipelines {
        if latest_per_ref.len() == LATEST_REFS_SHOWN {
            break;
        }
        if !latest_per_ref.iter().any(|p| p.r#ref == pipeline.r#ref) {
            latest_per_ref.push(pipeline);
        }
    }

    PipelineHealth {
        recent,
        success_rate_percent,
        in_progress,
        failed,
        latest_per_ref,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MilestoneProgress<'a> {
    pub milestone: &'a Milestone,
    /// `(closed, total)` issues, `None` while no issue of it is loaded.
    pub issues: Option<(usize, usize)>,
    pub due_in_days: Option<i64>,
}

/// Open milestones, soonest due first and undated ones last, at most `limit`.
/// `progress` maps a milestone iid to its `(closed, total)` issues.
pub fn upcoming_milestones<'a>(
    milestones: &'a [Milestone],
    progress: impl Fn(u64) -> Option<(usize, usize)>,
    today: NaiveDate,
    limit: usize,
) -> Vec<MilestoneProgress<'a>> {
    let mut open: Vec<MilestoneProgress> = milestones
        .iter()
        .filter(|m| m.state != "closed")
        .map(|milestone| MilestoneProgress {
            milestone,
            issues: progress(milestone.iid).filter(|&(_, total)| total > 0),
            due_in_days: days_until(milestone.due_date.as_deref(), today),
        })
        .collect();
    open.sort_by(|a, b| {
        let due = |m: &MilestoneProgress| m.due_in_days.unwrap_or(i64::MAX);
        due(a)
            .cmp(&due(b))
            .then_with(|| a.milestone.title.cmp(&b.milestone.title))
    });
    open.truncate(limit);
    open
}

#[derive(Debug, Clone, Copy)]
pub enum Activity<'a> {
    Issue(&'a Issue),
    MergeRequest(&'a MergeRequest),
    /// A commit, comment, review thread comment or review from the feed.
    Contribution(&'a Contribution),
}

impl Activity<'_> {
    pub fn updated_at(&self) -> &str {
        match self {
            Activity::Issue(issue) => &issue.updated_at,
            Activity::MergeRequest(mr) => &mr.updated_at,
            Activity::Contribution(contribution) => &contribution.created_at,
        }
    }

    fn involves(&self, person: Person) -> bool {
        match (self, person) {
            (_, Person::Everyone) => true,
            (Activity::Issue(issue), Person::User(login)) => issue_involves(issue, login),
            (Activity::MergeRequest(mr), Person::User(login)) => mr_involves(mr, login),
            (Activity::Contribution(contribution), Person::User(login)) => {
                contribution.author == login
            }
        }
    }
}

/// The issues and MRs/PRs `person` takes part in, in any state, and their
/// commits, comments and reviews made after `since`, most recently updated
/// first.
pub fn recent_activity<'a>(
    issues: &'a [Issue],
    mrs: &'a [MergeRequest],
    contributions: &'a [Contribution],
    person: Person,
    since: DateTime<Utc>,
) -> Vec<Activity<'a>> {
    let mut activity: Vec<Activity> = issues
        .iter()
        .map(Activity::Issue)
        .chain(mrs.iter().map(Activity::MergeRequest))
        .chain(
            contributions
                .iter()
                .filter(|c| is_made_after(c, since))
                .map(Activity::Contribution),
        )
        .filter(|item| item.involves(person))
        .collect();
    sort_by_update(&mut activity, Activity::updated_at, false);
    activity
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ADA: Person = Person::User("ada");

    fn now() -> Now {
        Now {
            instant: DateTime::parse_from_rfc3339("2026-10-29T12:00:00Z")
                .expect("valid timestamp")
                .with_timezone(&Utc),
            today: NaiveDate::from_ymd_opt(2026, 10, 29).expect("valid date"),
        }
    }

    fn with(mut base: serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
        base.as_object_mut()
            .expect("an object")
            .extend(extra.as_object().expect("an object").clone());
        base
    }

    fn issue(iid: u64, extra: serde_json::Value) -> Issue {
        serde_json::from_value(with(
            json!({
                "iid": iid,
                "title": format!("issue {iid}"),
                "state": "opened",
                "updated_at": "2026-10-28T00:00:00Z",
                "created_at": null,
                "closed_at": null,
                "author": { "username": "bob" },
                "milestone": null,
                "description": null,
            }),
            extra,
        ))
        .expect("a valid issue")
    }

    fn mr(iid: u64, extra: serde_json::Value) -> MergeRequest {
        serde_json::from_value(with(
            json!({
                "iid": iid,
                "title": format!("mr {iid}"),
                "state": "opened",
                "updated_at": "2026-10-28T00:00:00Z",
                "author": { "username": "bob" },
                "milestone": null,
                "target_branch": "main",
                "draft": false,
                "description": null,
            }),
            extra,
        ))
        .expect("a valid merge request")
    }

    fn pipeline(id: u64, status: &str, r#ref: &str) -> Pipeline {
        serde_json::from_value(json!({
            "id": id,
            "status": status,
            "ref": r#ref,
            "updated_at": "2026-10-01T00:00:00Z",
        }))
        .expect("a valid pipeline")
    }

    fn milestone(iid: u64, title: &str, state: &str, due_date: Option<&str>) -> Milestone {
        serde_json::from_value(json!({
            "id": iid,
            "iid": iid,
            "title": title,
            "state": state,
            "due_date": due_date,
            "created_at": "2026-01-01T00:00:00Z",
        }))
        .expect("a valid milestone")
    }

    fn iids<T>(rows: &[&T], iid: impl Fn(&T) -> u64) -> Vec<u64> {
        rows.iter().map(|row| iid(row)).collect()
    }

    fn mr_queue(queue: MrQueue, mrs: &[MergeRequest], person: Person) -> Vec<u64> {
        iids(&queue.list(mrs, person, None, now()), |mr| mr.iid)
    }

    #[test]
    fn a_persons_review_queue_holds_what_they_review_and_have_not_approved() {
        let mrs = [
            mr(1, json!({ "reviewers": [{ "username": "ada" }] })),
            mr(
                2,
                json!({
                    "reviewers": [{ "username": "ada" }],
                    "approval": { "approved_by": ["ada"] },
                }),
            ),
            mr(3, json!({ "reviewers": [{ "username": "cy" }] })),
            mr(
                4,
                json!({ "state": "merged", "reviewers": [{ "username": "ada" }] }),
            ),
        ];

        assert_eq!(mr_queue(MrQueue::AwaitingReview, &mrs, ADA), vec![1]);
        assert_eq!(
            mr_queue(MrQueue::AwaitingReview, &mrs, Person::Everyone),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn the_signed_in_users_queues_follow_the_fetched_workflow_status() {
        let mut requested = mr(1, json!({}));
        requested.workflow = Some(WorkflowStatus::ReviewRequested);
        let mut returned = mr(2, json!({}));
        returned.workflow = Some(WorkflowStatus::ReturnedToYou);
        let mrs = [requested, returned];

        let list = |queue: MrQueue| iids(&queue.list(&mrs, ADA, Some("ada"), now()), |mr| mr.iid);
        assert_eq!(list(MrQueue::AwaitingReview), vec![1]);
        assert_eq!(list(MrQueue::ChangesRequested), vec![2]);
    }

    #[test]
    fn a_persons_merge_queues_hold_only_what_they_own() {
        let mrs = [
            mr(
                1,
                json!({ "author": { "username": "ada" }, "approval": { "approved": true } }),
            ),
            mr(
                2,
                json!({
                    "assignees": [{ "username": "ada" }],
                    "approval": { "approved": true },
                    "mergeability": { "needs_rebase": true },
                }),
            ),
            mr(
                3,
                json!({
                    "reviewers": [{ "username": "ada" }],
                    "approval": { "approved": true },
                }),
            ),
            mr(
                4,
                json!({
                    "author": { "username": "ada" },
                    "draft": true,
                    "approval": { "approved": true, "changes_requested": true },
                }),
            ),
        ];

        assert_eq!(mr_queue(MrQueue::ReadyToMerge, &mrs, ADA), vec![1]);
        assert_eq!(mr_queue(MrQueue::Blocked, &mrs, ADA), vec![2]);
        assert_eq!(mr_queue(MrQueue::ChangesRequested, &mrs, ADA), vec![4]);
        assert_eq!(mr_queue(MrQueue::Drafts, &mrs, ADA), vec![4]);
        assert_eq!(mr_queue(MrQueue::Open, &mrs, ADA), vec![1, 2, 3, 4]);
        assert_eq!(
            mr_queue(MrQueue::ReadyToMerge, &mrs, Person::Everyone),
            vec![1, 3]
        );
    }

    #[test]
    fn idle_merge_requests_start_at_the_threshold_and_list_longest_idle_first() {
        let mrs = [
            mr(1, json!({ "updated_at": "2026-10-23T00:00:00Z" })),
            mr(2, json!({ "updated_at": "2026-10-22T12:00:00Z" })),
            mr(3, json!({ "updated_at": "2026-09-01T00:00:00Z" })),
        ];

        assert_eq!(mr_queue(MrQueue::Idle, &mrs, Person::Everyone), vec![3, 2]);
    }

    #[test]
    fn a_persons_issue_queues_follow_their_assignments_and_due_dates() {
        let assigned = json!([{ "username": "ada" }]);
        let issues = [
            issue(
                1,
                json!({ "assignees": assigned, "due_date": "2026-10-28" }),
            ),
            issue(
                2,
                json!({ "assignees": assigned, "due_date": "2026-10-29" }),
            ),
            issue(
                3,
                json!({ "assignees": assigned, "due_date": "2026-11-05" }),
            ),
            issue(
                4,
                json!({ "assignees": assigned, "due_date": "2026-11-06" }),
            ),
            issue(
                5,
                json!({ "assignees": assigned, "updated_at": "2026-10-15T12:00:00Z" }),
            ),
            issue(6, json!({ "author": { "username": "ada" } })),
            issue(7, json!({ "due_date": "2026-10-01" })),
            issue(8, json!({ "state": "closed", "assignees": assigned })),
        ];
        let list = |queue: IssueQueue, person: Person| {
            iids(&queue.list(&issues, person, now()), |issue| issue.iid)
        };

        assert_eq!(list(IssueQueue::Overdue, ADA), vec![1]);
        assert_eq!(list(IssueQueue::DueSoon, ADA), vec![2, 3]);
        assert_eq!(list(IssueQueue::Idle, ADA), vec![5]);
        assert_eq!(list(IssueQueue::Unassigned, ADA), vec![6]);
        assert_eq!(IssueQueue::Open.count(&issues, ADA, now()), 5);
        assert_eq!(list(IssueQueue::Overdue, Person::Everyone), vec![7, 1]);
        assert_eq!(list(IssueQueue::Unassigned, Person::Everyone), vec![6, 7]);
    }

    #[test]
    fn contributors_are_everyone_in_open_work_or_the_feed_but_the_signed_in_user() {
        let issues = [
            issue(
                1,
                json!({ "assignees": [{ "username": "cy" }], "due_date": "2026-10-01" }),
            ),
            issue(2, json!({ "assignees": [{ "username": "cy" }] })),
        ];
        let mrs = [
            mr(
                1,
                json!({ "author": { "username": "ada" }, "reviewers": [{ "username": "dee" }] }),
            ),
            mr(
                2,
                json!({ "state": "merged", "author": { "username": "eve" } }),
            ),
        ];
        let feed = [contribution("fay", "2026-10-02T00:00:00Z")];

        assert_eq!(
            contributor_logins(&issues, &mrs, &feed, Some("ada")),
            vec!["bob", "cy", "dee", "fay"]
        );
        assert_eq!(
            workload(&issues, &mrs, Person::User("cy"), Some("ada"), now()),
            Workload {
                open_issues: 2,
                overdue_issues: 1,
                ..Workload::default()
            }
        );
    }

    #[test]
    fn activity_counts_split_by_kind_and_keep_only_the_window() {
        use crate::domain::contributions::ContributionKind;
        let made = |kind, created_at: &str| Contribution {
            kind,
            ..contribution("ada", created_at)
        };
        let feed = [
            made(ContributionKind::Commit, "2026-10-29T11:00:00Z"),
            made(ContributionKind::Commit, "2026-10-28T13:00:00Z"),
            made(ContributionKind::Comment, "2026-10-25T00:00:00Z"),
            made(ContributionKind::ThreadComment, "2026-10-23T00:00:00Z"),
            made(ContributionKind::Review, "2026-10-10T00:00:00Z"),
            contribution("bob", "2026-10-29T11:00:00Z"),
        ];
        let counts = |window: ActivityWindow| activity_counts(&feed, ADA, window.starts_at(now()));

        assert_eq!(
            counts(ActivityWindow::Day),
            ActivityCounts {
                commits: 2,
                ..ActivityCounts::default()
            }
        );
        assert_eq!(
            counts(ActivityWindow::Week),
            ActivityCounts {
                commits: 2,
                comments: 1,
                threads: 1,
                reviews: 0,
            }
        );
        assert_eq!(counts(ActivityWindow::Month).reviews, 1);
        assert_eq!(
            activity_counts(
                &feed,
                Person::Everyone,
                ActivityWindow::Day.starts_at(now())
            )
            .commits,
            3
        );
    }

    #[test]
    fn recent_activity_of_a_person_spans_every_state_newest_first() {
        let issues = [
            issue(
                1,
                json!({ "author": { "username": "ada" }, "updated_at": "2026-10-03T10:00:00Z" }),
            ),
            issue(2, json!({ "updated_at": "2026-10-05T00:00:00Z" })),
        ];
        let mrs = [
            mr(
                7,
                json!({
                    "state": "merged",
                    "reviewers": [{ "username": "ada" }],
                    "updated_at": "2026-10-03T11:00:00.000+02:00",
                }),
            ),
            mr(
                8,
                json!({ "assignees": [{ "username": "ada" }], "updated_at": "2026-10-04T00:00:00Z" }),
            ),
        ];

        let feed = [
            contribution("ada", "2026-10-04T12:00:00Z"),
            contribution("bob", "2026-10-06T00:00:00Z"),
        ];

        let since = ActivityWindow::Month.starts_at(now());
        let shown: Vec<String> = recent_activity(&issues, &mrs, &feed, ADA, since)
            .iter()
            .map(|item| match item {
                Activity::Issue(issue) => format!("#{}", issue.iid),
                Activity::MergeRequest(mr) => format!("!{}", mr.iid),
                Activity::Contribution(contribution) => contribution.summary.clone(),
            })
            .collect();
        assert_eq!(shown, vec!["commit by ada", "!8", "#1", "!7"]);
    }

    fn contribution(author: &str, created_at: &str) -> Contribution {
        Contribution {
            author: author.to_string(),
            kind: crate::domain::contributions::ContributionKind::Commit,
            created_at: created_at.to_string(),
            summary: format!("commit by {author}"),
            target: None,
            location: None,
        }
    }

    #[test]
    fn the_signed_in_user_comes_from_the_first_known_review_state() {
        let mrs = [
            mr(1, json!({})),
            mr(2, json!({ "approval": { "current_user": "" } })),
            mr(3, json!({ "approval": { "current_user": "ada" } })),
        ];

        assert_eq!(signed_in_user(&mrs), Some("ada"));
        assert_eq!(signed_in_user(&mrs[..2]), None);
    }

    #[test]
    fn pipeline_health_rates_only_finished_runs_of_the_recent_window() {
        let mut pipelines = vec![
            pipeline(30, "running", "main"),
            pipeline(29, "failed", "feature"),
            pipeline(28, "success", "main"),
            pipeline(27, "canceled", "fix"),
            pipeline(26, "success", "release"),
        ];
        pipelines.extend((0..PIPELINE_HISTORY_LEN as u64).map(|id| pipeline(id, "failed", "old")));

        let health = assess_pipelines(&pipelines);

        assert_eq!(health.recent.len(), PIPELINE_HISTORY_LEN);
        assert_eq!(health.in_progress, 1);
        assert_eq!(health.failed, PIPELINE_HISTORY_LEN - 5 + 1);
        assert_eq!(health.success_rate_percent, Some(2 * 100 / 17));
        assert_eq!(
            health
                .latest_per_ref
                .iter()
                .map(|p| (p.id, p.r#ref.as_str()))
                .collect::<Vec<_>>(),
            vec![(30, "main"), (29, "feature"), (27, "fix")]
        );
    }

    #[test]
    fn pipeline_health_has_no_rate_until_a_run_finishes() {
        let pipelines = [
            pipeline(2, "running", "main"),
            pipeline(1, "manual", "main"),
        ];

        assert_eq!(assess_pipelines(&pipelines).success_rate_percent, None);
    }

    #[test]
    fn upcoming_milestones_lists_open_ones_soonest_due_first_and_undated_last() {
        let milestones = [
            milestone(1, "undated", "active", None),
            milestone(2, "later", "active", Some("2026-12-01")),
            milestone(3, "done", "closed", Some("2026-10-30")),
            milestone(4, "overdue", "open", Some("2026-10-20")),
            milestone(5, "soon", "active", Some("2026-11-01")),
        ];
        let progress = |iid| match iid {
            4 => Some((1, 4)),
            5 => Some((0, 0)),
            _ => None,
        };

        let upcoming = upcoming_milestones(&milestones, progress, now().today, 3);

        assert_eq!(
            upcoming
                .iter()
                .map(|m| (m.milestone.title.as_str(), m.issues, m.due_in_days))
                .collect::<Vec<_>>(),
            vec![
                ("overdue", Some((1, 4)), Some(-9)),
                ("soon", None, Some(3)),
                ("later", None, Some(33)),
            ]
        );
    }
}
