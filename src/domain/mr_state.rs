#![allow(dead_code)]
// Can be removed once later tasks (Task 7+) consume these functions.

use serde::{Deserialize, Serialize};

/// One merge request's workflow status, in GitLab's vocabulary.
///
/// GitLab assigns each MR exactly one status. The first three are "Active"
/// (they count toward your review total); the rest are inactive.
///
/// `Inactive` collapses GitLab's "Waiting for assignee" and "Waiting for
/// approvals": their documented definitions overlap with no stated
/// precedence, so splitting them would mean inventing semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowStatus {
    // Active
    ReturnedToYou,
    ReviewRequested,
    YourMergeRequest,
    // Inactive
    ApprovedByYou,
    ApprovedByOthers,
    Inactive,
    /// Known: this MR has no relationship to you. Distinct from `None`.
    NotYours,
}

/// Everything the cascade needs, gathered in one place so the rules stay pure.
pub struct WorkflowInputs<'a> {
    /// `None` when the current user could not be determined — the whole
    /// cascade is then unanswerable.
    pub current_user: Option<&'a str>,
    pub author: &'a str,
    pub assignees: &'a [String],
    pub reviewers: &'a [String],
    pub changes_requested: bool,
    pub approved: bool,
    pub you_approved: bool,
    /// You submitted any review — approved OR requested changes.
    pub you_reviewed: bool,
}

impl WorkflowStatus {
    /// The three statuses GitLab counts toward your review total.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            WorkflowStatus::ReturnedToYou
                | WorkflowStatus::ReviewRequested
                | WorkflowStatus::YourMergeRequest
        )
    }
}

/// First-match cascade. Returns `None` only when the answer is unknowable —
/// never as a stand-in for "not yours", which is `NotYours`.
pub fn workflow_status(i: &WorkflowInputs) -> Option<WorkflowStatus> {
    let me = i.current_user?;
    if me.is_empty() {
        return None;
    }

    let is_author = i.author == me;
    let is_assignee = i.assignees.iter().any(|a| a == me);
    let is_reviewer = i.reviewers.iter().any(|r| r == me);
    let involves_me = is_author || is_assignee || is_reviewer;

    if is_author && i.changes_requested {
        return Some(WorkflowStatus::ReturnedToYou);
    }
    if is_reviewer && !i.you_reviewed {
        return Some(WorkflowStatus::ReviewRequested);
    }
    if is_author || is_assignee {
        return Some(WorkflowStatus::YourMergeRequest);
    }
    if i.you_approved {
        return Some(WorkflowStatus::ApprovedByYou);
    }
    // The `involves_me` guard is load-bearing: without it this fires on every
    // approved MR in the project, not just the ones you are part of.
    if involves_me && i.approved {
        return Some(WorkflowStatus::ApprovedByOthers);
    }
    if involves_me {
        return Some(WorkflowStatus::Inactive);
    }
    Some(WorkflowStatus::NotYours)
}

/// Sort ordinal: GitLab's Active order first, then inactive, unknown last.
///
/// Returned to the table as a decimal string by the caller, so the
/// comparator's `u64` fast path orders by ordinal rather than alphabetically
/// by label. Grouping uses the same value, because group-by is sort.
pub fn workflow_sort_key(s: Option<WorkflowStatus>) -> u8 {
    match s {
        Some(WorkflowStatus::ReturnedToYou) => 0,
        Some(WorkflowStatus::ReviewRequested) => 1,
        Some(WorkflowStatus::YourMergeRequest) => 2,
        Some(WorkflowStatus::ApprovedByYou) => 3,
        Some(WorkflowStatus::ApprovedByOthers) => 4,
        Some(WorkflowStatus::Inactive) => 5,
        Some(WorkflowStatus::NotYours) => 6,
        None => 7,
    }
}

/// Abbreviated cell text. Full wording lives in the Details pane, because
/// "Returned to you" is 16 chars and the column clamps to 10 below 90 columns.
pub fn workflow_cell(s: Option<WorkflowStatus>) -> String {
    let icons = crate::config::ICONS.read().unwrap();
    match s {
        Some(WorkflowStatus::ReturnedToYou) => format!("{} Returned", icons.workflow_returned),
        Some(WorkflowStatus::ReviewRequested) => {
            format!("{} Review req", icons.workflow_review)
        }
        Some(WorkflowStatus::YourMergeRequest) => format!("{} Yours", icons.workflow_yours),
        Some(WorkflowStatus::ApprovedByYou) => format!("{} Approved", icons.workflow_approved),
        Some(WorkflowStatus::ApprovedByOthers) => {
            format!("{} by others", icons.workflow_approved_others)
        }
        Some(WorkflowStatus::Inactive) => format!("{} Inactive", icons.workflow_inactive),
        // Known "not yours" renders blank so the 24-of-33 common case stays quiet.
        Some(WorkflowStatus::NotYours) => String::new(),
        // Unknown is visibly distinct from blank.
        None => "—".to_string(),
    }
}

/// GitLab's full wording, for the Details pane and filter values.
/// `None` for both `NotYours` and unknown — neither gets a Details line.
pub fn workflow_label(s: Option<WorkflowStatus>) -> Option<&'static str> {
    match s {
        Some(WorkflowStatus::ReturnedToYou) => Some("Returned to you"),
        Some(WorkflowStatus::ReviewRequested) => Some("Review requested"),
        Some(WorkflowStatus::YourMergeRequest) => Some("Your merge requests"),
        Some(WorkflowStatus::ApprovedByYou) => Some("Approved by you"),
        Some(WorkflowStatus::ApprovedByOthers) => Some("Approved by others"),
        Some(WorkflowStatus::Inactive) => Some("Inactive"),
        Some(WorkflowStatus::NotYours) | None => None,
    }
}

/// Approval readiness for one merge request. Host-neutral.
///
/// `None` at the call site means *unknown* (fetch failed or unsupported),
/// never "unapproved" — see `approval_cell`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApprovalState {
    pub approved: bool,
    /// `None` on GitHub, which exposes no approval counts.
    pub approvals_left: Option<u32>,
    /// `None` on GitHub or where no approval rule is configured.
    pub approvals_required: Option<u32>,
    pub approved_by: Vec<String>,
    pub changes_requested: bool,
    pub you_approved: bool,
    pub awaiting_you: bool,
}

/// Merge readiness for one merge request. Independent of `ApprovalState`:
/// an MR can be approved *and* conflicted at the same time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MergeabilityState {
    pub conflicts: bool,
    pub needs_rebase: bool,
    /// Server has not settled the merge status yet. Transient, resolves on refresh.
    pub computing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalTone {
    Unknown,
    ChangesRequested,
    AwaitingYou,
    Approved,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeTone {
    Unknown,
    Conflict,
    Rebase,
    Computing,
    Clean,
}

/// GitLab merge statuses that mean "not settled yet".
const TRANSIENT_MERGE_STATUSES: [&str; 4] =
    ["CHECKING", "UNCHECKED", "PREPARING", "APPROVALS_SYNCING"];

/// REST returns these lowercase, GraphQL uppercase, so compare case-insensitively.
pub fn is_transient_merge_status(raw: &str) -> bool {
    let upper = raw.to_uppercase();
    TRANSIENT_MERGE_STATUSES.contains(&upper.as_str())
}

/// Your approval is still needed only if you *can* approve, have not already,
/// and the MR is not already satisfied. The final term stops the UI nagging
/// about MRs that need nothing.
pub fn derive_awaiting_you(can_approve: bool, you_approved: bool, approved: bool) -> bool {
    can_approve && !you_approved && !approved
}

/// True only when we can both confirm approval *and* attribute it.
fn is_attributably_approved(s: &ApprovalState) -> bool {
    s.approved && !s.approved_by.is_empty()
}

/// `given/required`, dropping the denominator when nothing is required.
fn format_counts(s: &ApprovalState) -> String {
    let given = s.approved_by.len();
    match s.approvals_required {
        Some(req) if req > 0 => format!("{}/{}", given, req),
        _ => given.to_string(),
    }
}

/// First-match-wins cascade. See the precedence flowchart in the design spec.
pub fn approval_cell(state: Option<&ApprovalState>, is_github: bool) -> (String, ApprovalTone) {
    let icons = crate::config::ICONS.read().unwrap();
    let Some(s) = state else {
        return ("—".to_string(), ApprovalTone::Unknown);
    };

    if s.changes_requested {
        let text = if is_github {
            format!("{} changes", icons.approval_changes)
        } else {
            format!("{} chg", icons.approval_changes)
        };
        return (text, ApprovalTone::ChangesRequested);
    }

    // GitHub exposes no counts and no canApprove, so it renders words only.
    if is_github {
        if is_attributably_approved(s) {
            return (
                format!("{} approved", icons.approval_approved),
                ApprovalTone::Approved,
            );
        }
        return ("review req".to_string(), ApprovalTone::Pending);
    }

    if s.awaiting_you {
        return (
            format!("{} {}", icons.approval_pending, format_counts(s)),
            ApprovalTone::AwaitingYou,
        );
    }
    if is_attributably_approved(s) {
        return (
            format!("{} {}", icons.approval_approved, format_counts(s)),
            ApprovalTone::Approved,
        );
    }
    (format_counts(s), ApprovalTone::Pending)
}

/// First-match-wins cascade. Conflict outranks rebase because it is the more
/// blocking state and the only one the user cannot fix from the TUI. Known
/// state outranks `computing`.
pub fn mergeable_cell(state: Option<&MergeabilityState>) -> (String, MergeTone) {
    let icons = crate::config::ICONS.read().unwrap();
    let Some(s) = state else {
        return ("—".to_string(), MergeTone::Unknown);
    };
    if s.conflicts {
        return (
            format!("{} conflict", icons.merge_conflict),
            MergeTone::Conflict,
        );
    }
    if s.needs_rebase {
        return (format!("{} rebase", icons.merge_rebase), MergeTone::Rebase);
    }
    if s.computing {
        return (icons.merge_checking.clone(), MergeTone::Computing);
    }
    (icons.merge_clean.clone(), MergeTone::Clean)
}

/// Sort ordinal: most-blocking first, unknown last. The caller (`App::mr_sort_value`)
/// stringifies this via `.to_string()` before handing it to the table's sort
/// comparator, whose `u64` fast path then orders rows by state rather than
/// alphabetically by label.
pub fn approval_sort_key(state: Option<&ApprovalState>) -> u8 {
    match approval_cell(state, false).1 {
        ApprovalTone::ChangesRequested => 0,
        ApprovalTone::Pending => 1,
        ApprovalTone::AwaitingYou => 2,
        ApprovalTone::Approved => 3,
        ApprovalTone::Unknown => 4,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseGate {
    Allowed,
    /// Rebase cannot resolve a conflict — that needs local work.
    ResolveLocally,
    NotNeeded,
}

/// Whether `R` should act, refuse, or no-op, so the action matches what the
/// Mergeable column shows. Kept pure and separate from the handler so the
/// decision is unit-testable.
pub fn rebase_gate(state: Option<&MergeabilityState>) -> RebaseGate {
    match state {
        Some(s) if s.conflicts => RebaseGate::ResolveLocally,
        Some(s) if s.needs_rebase => RebaseGate::Allowed,
        _ => RebaseGate::NotNeeded,
    }
}

pub fn mergeable_sort_key(state: Option<&MergeabilityState>) -> u8 {
    match mergeable_cell(state).1 {
        MergeTone::Conflict => 0,
        MergeTone::Rebase => 1,
        MergeTone::Computing => 2,
        MergeTone::Clean => 3,
        MergeTone::Unknown => 4,
    }
}

/// Independent flag glyphs appended to the Status cell's base word.
///
/// Each flag occupies its own slot, so `DRAFT` and the flag never displace one
/// another. Flags append in a fixed order so the cell is stable across
/// refreshes; later flags must be added to the end, not interleaved.
pub fn status_flags(blocking_discussions_resolved: Option<bool>) -> String {
    let mut out = String::new();
    // Only Some(false) is a problem. Some(true) and None render nothing.
    if blocking_discussions_resolved == Some(false) {
        let icons = crate::config::ICONS.read().unwrap();
        out.push(' ');
        out.push_str(&icons.flag_unresolved);
    }
    out
}

/// Filter values for the Status column. Returns more than one value when an MR
/// carries a flag, so the flag is filterable without the base word losing its
/// own filter value.
pub fn status_filter_values(
    draft: bool,
    blocking_discussions_resolved: Option<bool>,
) -> Vec<String> {
    let mut v = vec![if draft { "Draft" } else { "Ready" }.to_string()];
    if blocking_discussions_resolved == Some(false) {
        v.push("Unresolved discussions".to_string());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approved_by(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    // Build expected strings from the same `ICONS` source the render code reads,
    // rather than duplicating glyph literals here.
    fn expect_chg() -> String {
        let icons = crate::config::ICONS.read().unwrap();
        format!("{} chg", icons.approval_changes)
    }

    fn expect_approved(counts: &str) -> String {
        let icons = crate::config::ICONS.read().unwrap();
        format!("{} {}", icons.approval_approved, counts)
    }

    fn expect_github_approved() -> String {
        let icons = crate::config::ICONS.read().unwrap();
        format!("{} approved", icons.approval_approved)
    }

    fn expect_awaiting(counts: &str) -> String {
        let icons = crate::config::ICONS.read().unwrap();
        format!("{} {}", icons.approval_pending, counts)
    }

    fn expect_conflict() -> String {
        let icons = crate::config::ICONS.read().unwrap();
        format!("{} conflict", icons.merge_conflict)
    }

    fn expect_rebase() -> String {
        let icons = crate::config::ICONS.read().unwrap();
        format!("{} rebase", icons.merge_rebase)
    }

    fn expect_computing() -> String {
        crate::config::ICONS.read().unwrap().merge_checking.clone()
    }

    fn expect_clean() -> String {
        crate::config::ICONS.read().unwrap().merge_clean.clone()
    }

    // ── awaiting_you truth table ──

    #[test]
    fn awaiting_you_is_false_when_already_approved_by_you() {
        // !5281: you can still approve an already-satisfied MR; must not nag.
        let s = ApprovalState {
            approved: true,
            approvals_left: Some(0),
            approvals_required: Some(0),
            approved_by: approved_by(&["julien.carmignani"]),
            changes_requested: false,
            you_approved: false,
            awaiting_you: derive_awaiting_you(true, false, true),
        };
        assert!(!s.awaiting_you);
    }

    #[test]
    fn awaiting_you_is_true_when_you_can_approve_and_mr_unsatisfied() {
        assert!(derive_awaiting_you(true, false, false));
    }

    #[test]
    fn awaiting_you_is_false_when_you_cannot_approve() {
        assert!(!derive_awaiting_you(false, false, false));
    }

    #[test]
    fn awaiting_you_is_false_when_you_already_approved() {
        assert!(!derive_awaiting_you(true, true, false));
    }

    // ── approval cell rendering ──

    #[test]
    fn approval_cell_unknown_renders_dash() {
        let (text, tone) = approval_cell(None, false);
        assert_eq!(text, "—");
        assert_eq!(tone, ApprovalTone::Unknown);
    }

    #[test]
    fn approval_cell_changes_requested_wins_over_approved() {
        let s = ApprovalState {
            approved: true,
            approvals_left: Some(0),
            approvals_required: Some(1),
            approved_by: approved_by(&["a"]),
            changes_requested: true,
            you_approved: false,
            awaiting_you: false,
        };
        let (text, tone) = approval_cell(Some(&s), false);
        assert_eq!(text, expect_chg());
        assert_eq!(tone, ApprovalTone::ChangesRequested);
    }

    #[test]
    fn approval_cell_approved_shows_given_over_required() {
        // !1448: two approvals, one required.
        let s = ApprovalState {
            approved: true,
            approvals_left: Some(0),
            approvals_required: Some(1),
            approved_by: approved_by(&["ozgur.gurkan", "chandler.anderson"]),
            changes_requested: false,
            you_approved: true,
            awaiting_you: false,
        };
        let (text, _) = approval_cell(Some(&s), false);
        assert_eq!(text, expect_approved("2/1"));
    }

    #[test]
    fn approval_cell_drops_denominator_when_none_required() {
        // !5281: req=0 must render "✓ 1", never "✓ 1/0".
        let s = ApprovalState {
            approved: true,
            approvals_left: Some(0),
            approvals_required: Some(0),
            approved_by: approved_by(&["julien.carmignani"]),
            changes_requested: false,
            you_approved: false,
            awaiting_you: false,
        };
        let (text, _) = approval_cell(Some(&s), false);
        assert_eq!(text, expect_approved("1"));
    }

    #[test]
    fn approval_cell_not_approved_when_approver_list_empty() {
        // Defensive: never claim an approval we cannot attribute.
        let s = ApprovalState {
            approved: true,
            approvals_left: Some(0),
            approvals_required: Some(0),
            approved_by: vec![],
            changes_requested: false,
            you_approved: false,
            awaiting_you: false,
        };
        let (text, tone) = approval_cell(Some(&s), false);
        assert_ne!(tone, ApprovalTone::Approved);
        assert_eq!(text, "0");
    }

    #[test]
    fn approval_cell_awaiting_you_shows_marker() {
        // !5277: 0 of 1, waiting on you.
        let s = ApprovalState {
            approved: false,
            approvals_left: Some(1),
            approvals_required: Some(1),
            approved_by: vec![],
            changes_requested: false,
            you_approved: false,
            awaiting_you: true,
        };
        let (text, tone) = approval_cell(Some(&s), false);
        assert_eq!(text, expect_awaiting("0/1"));
        assert_eq!(tone, ApprovalTone::AwaitingYou);
    }

    #[test]
    fn approval_cell_pending_has_no_marker() {
        let s = ApprovalState {
            approved: false,
            approvals_left: Some(1),
            approvals_required: Some(2),
            approved_by: approved_by(&["a"]),
            changes_requested: false,
            you_approved: false,
            awaiting_you: false,
        };
        let (text, tone) = approval_cell(Some(&s), false);
        assert_eq!(text, "1/2");
        assert_eq!(tone, ApprovalTone::Pending);
    }

    #[test]
    fn approval_cell_github_uses_words_not_counts() {
        let s = ApprovalState {
            approved: true,
            approvals_left: None,
            approvals_required: None,
            approved_by: approved_by(&["octocat"]),
            changes_requested: false,
            you_approved: false,
            awaiting_you: false,
        };
        let (text, _) = approval_cell(Some(&s), true);
        assert_eq!(text, expect_github_approved());
    }

    #[test]
    fn approval_cell_github_pending_says_review_req() {
        let s = ApprovalState {
            approved: false,
            approvals_left: None,
            approvals_required: None,
            approved_by: vec![],
            changes_requested: false,
            you_approved: false,
            awaiting_you: false,
        };
        let (text, _) = approval_cell(Some(&s), true);
        assert_eq!(text, "review req");
    }

    // ── mergeability cell rendering ──

    #[test]
    fn mergeable_cell_unknown_renders_dash() {
        let (text, tone) = mergeable_cell(None);
        assert_eq!(text, "—");
        assert_eq!(tone, MergeTone::Unknown);
    }

    #[test]
    fn mergeable_cell_conflict_wins_over_rebase_and_computing() {
        let s = MergeabilityState {
            conflicts: true,
            needs_rebase: true,
            computing: true,
        };
        let (text, tone) = mergeable_cell(Some(&s));
        assert_eq!(text, expect_conflict());
        assert_eq!(tone, MergeTone::Conflict);
    }

    #[test]
    fn mergeable_cell_rebase_wins_over_computing() {
        // !402, !8
        let s = MergeabilityState {
            conflicts: false,
            needs_rebase: true,
            computing: true,
        };
        let (text, tone) = mergeable_cell(Some(&s));
        assert_eq!(text, expect_rebase());
        assert_eq!(tone, MergeTone::Rebase);
    }

    #[test]
    fn mergeable_cell_computing_renders_ellipsis() {
        let s = MergeabilityState {
            conflicts: false,
            needs_rebase: false,
            computing: true,
        };
        let (text, tone) = mergeable_cell(Some(&s));
        assert_eq!(text, expect_computing());
        assert_eq!(tone, MergeTone::Computing);
    }

    #[test]
    fn mergeable_cell_clean_renders_check() {
        let s = MergeabilityState {
            conflicts: false,
            needs_rebase: false,
            computing: false,
        };
        let (text, tone) = mergeable_cell(Some(&s));
        assert_eq!(text, expect_clean());
        assert_eq!(tone, MergeTone::Clean);
    }

    // ── transient detection ──

    #[test]
    fn transient_statuses_are_recognised() {
        for raw in ["CHECKING", "UNCHECKED", "PREPARING", "APPROVALS_SYNCING"] {
            assert!(is_transient_merge_status(raw), "{raw} should be transient");
        }
    }

    #[test]
    fn settled_statuses_are_not_transient() {
        for raw in ["CONFLICT", "NEED_REBASE", "MERGEABLE", "NOT_APPROVED"] {
            assert!(
                !is_transient_merge_status(raw),
                "{raw} should not be transient"
            );
        }
    }

    #[test]
    fn transient_detection_is_case_insensitive() {
        // REST returns lowercase, GraphQL returns uppercase.
        assert!(is_transient_merge_status("approvals_syncing"));
    }

    // ── sort keys ──

    #[test]
    fn approval_sort_orders_changes_first_unknown_last() {
        let changes = ApprovalState {
            approved: false,
            approvals_left: None,
            approvals_required: None,
            approved_by: vec![],
            changes_requested: true,
            you_approved: false,
            awaiting_you: false,
        };
        let approved = ApprovalState {
            approved: true,
            approvals_left: None,
            approvals_required: None,
            approved_by: approved_by(&["a"]),
            changes_requested: false,
            you_approved: false,
            awaiting_you: false,
        };
        assert!(approval_sort_key(Some(&changes)) < approval_sort_key(Some(&approved)));
        assert!(approval_sort_key(Some(&approved)) < approval_sort_key(None));
    }

    #[test]
    fn mergeable_sort_orders_conflict_first_unknown_last() {
        let conflict = MergeabilityState {
            conflicts: true,
            needs_rebase: false,
            computing: false,
        };
        let clean = MergeabilityState {
            conflicts: false,
            needs_rebase: false,
            computing: false,
        };
        assert!(mergeable_sort_key(Some(&conflict)) < mergeable_sort_key(Some(&clean)));
        assert!(mergeable_sort_key(Some(&clean)) < mergeable_sort_key(None));
    }

    // ── status flag strip ──

    #[test]
    fn unresolved_discussions_produces_a_flag() {
        assert!(!status_flags(Some(false)).is_empty());
    }

    #[test]
    fn resolved_discussions_produce_no_flag() {
        assert_eq!(status_flags(Some(true)), "");
    }

    #[test]
    fn unknown_discussions_produce_no_flag() {
        // An unknown must not look like a problem.
        assert_eq!(status_flags(None), "");
    }

    #[test]
    fn status_filter_values_include_base_word_only_when_resolved() {
        assert_eq!(
            status_filter_values(true, Some(true)),
            vec!["Draft".to_string()]
        );
        assert_eq!(status_filter_values(false, None), vec!["Ready".to_string()]);
    }

    #[test]
    fn status_filter_values_include_both_facts_when_unresolved() {
        // !1471 is draft AND unresolved; a filter on either must match it,
        // which is what preserves fidelity in the shared column.
        let v = status_filter_values(true, Some(false));
        assert!(v.contains(&"Draft".to_string()));
        assert!(v.contains(&"Unresolved discussions".to_string()));
    }

    // ── rebase gate ──

    #[test]
    fn rebase_allowed_when_behind_target() {
        let s = MergeabilityState {
            conflicts: false,
            needs_rebase: true,
            computing: false,
        };
        assert_eq!(rebase_gate(Some(&s)), RebaseGate::Allowed);
    }

    #[test]
    fn rebase_refused_on_conflicts() {
        // Rebase cannot resolve a conflict; that needs local work.
        let s = MergeabilityState {
            conflicts: true,
            needs_rebase: true,
            computing: false,
        };
        assert_eq!(rebase_gate(Some(&s)), RebaseGate::ResolveLocally);
    }

    #[test]
    fn rebase_not_needed_when_clean() {
        let s = MergeabilityState {
            conflicts: false,
            needs_rebase: false,
            computing: false,
        };
        assert_eq!(rebase_gate(Some(&s)), RebaseGate::NotNeeded);
    }

    #[test]
    fn rebase_not_needed_while_computing_or_unknown() {
        let s = MergeabilityState {
            conflicts: false,
            needs_rebase: false,
            computing: true,
        };
        assert_eq!(rebase_gate(Some(&s)), RebaseGate::NotNeeded);
        assert_eq!(rebase_gate(None), RebaseGate::NotNeeded);
    }

    // ── workflow status cascade ──

    fn inputs<'a>(
        current_user: Option<&'a str>,
        author: &'a str,
        assignees: &'a [String],
        reviewers: &'a [String],
    ) -> WorkflowInputs<'a> {
        WorkflowInputs {
            current_user,
            author,
            assignees,
            reviewers,
            changes_requested: false,
            approved: false,
            you_approved: false,
            you_reviewed: false,
        }
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unknown_current_user_yields_none_not_a_status() {
        // None means "could not determine", and must never be mistaken for
        // "not yours" — that would hide an MR waiting on the user.
        let a = names(&[]);
        let r = names(&[]);
        assert_eq!(workflow_status(&inputs(None, "someone", &a, &r)), None);
    }

    #[test]
    fn no_relation_is_not_yours_not_none() {
        let a = names(&[]);
        let r = names(&[]);
        assert_eq!(
            workflow_status(&inputs(Some("me"), "someone", &a, &r)),
            Some(WorkflowStatus::NotYours)
        );
    }

    #[test]
    fn returned_to_you_requires_authorship() {
        let a = names(&[]);
        let r = names(&[]);
        let mut i = inputs(Some("me"), "me", &a, &r);
        i.changes_requested = true;
        assert_eq!(workflow_status(&i), Some(WorkflowStatus::ReturnedToYou));
    }

    #[test]
    fn changes_requested_on_someone_elses_mr_is_not_returned_to_you() {
        let a = names(&[]);
        let r = names(&["me"]);
        let mut i = inputs(Some("me"), "someone", &a, &r);
        i.changes_requested = true;
        assert_ne!(workflow_status(&i), Some(WorkflowStatus::ReturnedToYou));
    }

    #[test]
    fn review_requested_when_you_are_an_unreviewed_reviewer() {
        let a = names(&[]);
        let r = names(&["me", "other"]);
        let i = inputs(Some("me"), "someone", &a, &r);
        assert_eq!(workflow_status(&i), Some(WorkflowStatus::ReviewRequested));
    }

    #[test]
    fn review_requested_needs_your_own_review_state_not_anyones() {
        // The regression guard for restoring `username` in the GraphQL
        // reviewers subselection: another reviewer being unreviewed must not
        // make this "Review requested" for you.
        let a = names(&[]);
        let r = names(&["me", "other"]);
        let mut i = inputs(Some("me"), "someone", &a, &r);
        i.you_reviewed = true;
        assert_ne!(workflow_status(&i), Some(WorkflowStatus::ReviewRequested));
    }

    #[test]
    fn your_merge_request_covers_author_and_assignee() {
        let none = names(&[]);
        let r = names(&[]);
        assert_eq!(
            workflow_status(&inputs(Some("me"), "me", &none, &r)),
            Some(WorkflowStatus::YourMergeRequest)
        );
        let a = names(&["me"]);
        assert_eq!(
            workflow_status(&inputs(Some("me"), "someone", &a, &r)),
            Some(WorkflowStatus::YourMergeRequest)
        );
    }

    #[test]
    fn author_outranks_approved_by_you() {
        // First-match: an MR you authored is "yours", even if you approved it.
        let a = names(&[]);
        let r = names(&[]);
        let mut i = inputs(Some("me"), "me", &a, &r);
        i.you_approved = true;
        assert_eq!(workflow_status(&i), Some(WorkflowStatus::YourMergeRequest));
    }

    #[test]
    fn approved_by_you_when_only_a_reviewer() {
        let a = names(&[]);
        let r = names(&["me"]);
        let mut i = inputs(Some("me"), "someone", &a, &r);
        i.you_reviewed = true;
        i.you_approved = true;
        assert_eq!(workflow_status(&i), Some(WorkflowStatus::ApprovedByYou));
    }

    #[test]
    fn approved_by_others_requires_involvement() {
        // THE load-bearing guard. Without it this fires on every approved MR
        // in the project — 24 of 33 on the reference instance — turning a
        // workflow column into a second approval column.
        let a = names(&[]);
        let r = names(&[]);
        let mut i = inputs(Some("me"), "someone", &a, &r);
        i.approved = true;
        assert_eq!(workflow_status(&i), Some(WorkflowStatus::NotYours));

        let r2 = names(&["me"]);
        let mut j = inputs(Some("me"), "someone", &a, &r2);
        j.approved = true;
        j.you_reviewed = true;
        assert_eq!(workflow_status(&j), Some(WorkflowStatus::ApprovedByOthers));
    }

    #[test]
    fn involved_but_nothing_else_matches_is_inactive() {
        let a = names(&[]);
        let r = names(&["me"]);
        let mut i = inputs(Some("me"), "someone", &a, &r);
        i.you_reviewed = true; // reviewed, didn't approve, no changes requested
        assert_eq!(workflow_status(&i), Some(WorkflowStatus::Inactive));
    }

    #[test]
    fn sort_key_follows_gitlab_active_order_then_inactive_then_unknown() {
        use WorkflowStatus::*;
        let ordered = [
            Some(ReturnedToYou),
            Some(ReviewRequested),
            Some(YourMergeRequest),
            Some(ApprovedByYou),
            Some(ApprovedByOthers),
            Some(Inactive),
            Some(NotYours),
            None,
        ];
        let keys: Vec<u8> = ordered.iter().map(|s| workflow_sort_key(*s)).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted, "sort keys must already be in cascade order");
        assert_eq!(keys.len(), 8);
    }

    #[test]
    fn not_yours_renders_blank_and_unknown_renders_dash() {
        assert_eq!(workflow_cell(Some(WorkflowStatus::NotYours)), "");
        assert_eq!(workflow_cell(None), "—");
        assert_ne!(
            workflow_cell(Some(WorkflowStatus::NotYours)),
            workflow_cell(None),
            "blank and unknown must be distinguishable"
        );
    }

    #[test]
    fn labels_use_gitlab_wording() {
        assert_eq!(
            workflow_label(Some(WorkflowStatus::ReturnedToYou)),
            Some("Returned to you")
        );
        assert_eq!(
            workflow_label(Some(WorkflowStatus::YourMergeRequest)),
            Some("Your merge requests")
        );
        assert_eq!(workflow_label(Some(WorkflowStatus::NotYours)), None);
        assert_eq!(workflow_label(None), None);
    }

    #[test]
    fn approved_by_others_has_its_own_icon() {
        let by_others = workflow_cell(Some(WorkflowStatus::ApprovedByOthers));
        let inactive = workflow_cell(Some(WorkflowStatus::Inactive));
        let mine = workflow_cell(Some(WorkflowStatus::ApprovedByYou));
        assert_ne!(
            by_others, inactive,
            "ApprovedByOthers must not share Inactive's icon"
        );
        assert_ne!(
            by_others, mine,
            "ApprovedByOthers must not share ApprovedByYou's icon"
        );
    }
}
