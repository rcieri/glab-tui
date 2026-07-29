#![allow(dead_code)]
// Can be removed once later tasks (Task 7+) consume these functions.

use serde::{Deserialize, Serialize};

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

pub fn mergeable_sort_key(state: Option<&MergeabilityState>) -> u8 {
    match mergeable_cell(state).1 {
        MergeTone::Conflict => 0,
        MergeTone::Rebase => 1,
        MergeTone::Computing => 2,
        MergeTone::Clean => 3,
        MergeTone::Unknown => 4,
    }
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
}
