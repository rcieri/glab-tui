use crate::config::Icons;
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

/// The (glyph, word) pair for each of the six real statuses — the single
/// source both `workflow_cell` and `workflow_icon` build on, so a
/// same-variant icon swap in one can never silently diverge from the other.
/// `NotYours` returns `None`: it has no glyph or word of its own (both
/// callers render it as blank, not through this match), which is why it is
/// excluded here rather than given an empty pair.
fn workflow_glyph_and_word(s: WorkflowStatus) -> Option<(WorkflowGlyph, &'static str)> {
    Some(match s {
        WorkflowStatus::ReturnedToYou => (|icons| &icons.workflow_returned, "Returned"),
        WorkflowStatus::ReviewRequested => (|icons| &icons.workflow_review, "Review req"),
        WorkflowStatus::YourMergeRequest => (|icons| &icons.workflow_yours, "Yours"),
        WorkflowStatus::ApprovedByYou => (|icons| &icons.workflow_approved, "Approved"),
        WorkflowStatus::ApprovedByOthers => (|icons| &icons.workflow_approved_others, "By others"),
        WorkflowStatus::Inactive => (|icons| &icons.workflow_inactive, "Inactive"),
        WorkflowStatus::NotYours => return None,
    })
}

/// Picks a status's glyph out of the icon set, so the word can be read
/// without one.
type WorkflowGlyph = fn(&Icons) -> &String;

/// Abbreviated cell text. Full wording lives in the Preview pane, because
/// "Returned to you" is 16 chars and the column clamps to 10 below 90 columns.
pub fn workflow_cell(s: Option<WorkflowStatus>, icons: &Icons) -> String {
    match s {
        // Known "not yours" renders blank so the 24-of-33 common case stays quiet.
        Some(WorkflowStatus::NotYours) => String::new(),
        // Unknown is visibly distinct from blank.
        None => "—".to_string(),
        Some(status) => match workflow_glyph_and_word(status) {
            Some((glyph, word)) => format!("{} {word}", glyph(icons)),
            None => String::new(),
        },
    }
}

/// Just the glyph `workflow_cell` prefixes its text with, for the Preview
/// pane — which spells out the full label separately instead of clamping
/// icon and text together like the table column does. Reads from the same
/// `workflow_glyph_and_word` match as `workflow_cell` so the two can never
/// disagree on which glyph a status gets.
pub fn workflow_icon(s: Option<WorkflowStatus>, icons: &Icons) -> String {
    s.and_then(workflow_glyph_and_word)
        .map(|(glyph, _)| glyph(icons).clone())
        .unwrap_or_default()
}

/// GitLab's full wording, for the Preview pane and filter values.
/// `None` for both `NotYours` and unknown — neither gets a Preview line.
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

/// The abbreviated word shown in the Workflow table cell (e.g. "Returned",
/// "Review req"). Used as the column-filter picker value so it matches
/// exactly what the user sees. `None` for statuses that render blank.
pub fn workflow_cell_word(s: Option<WorkflowStatus>) -> Option<&'static str> {
    match s {
        Some(status) => workflow_glyph_and_word(status).map(|(_, word)| word),
        None => None,
    }
}

/// Approval readiness for one merge request. Host-neutral.
///
/// `None` at the call site means *unknown* (fetch failed or unsupported),
/// never "unapproved" — see `approval_cell`.
///
/// Container-level `#[serde(default)]`: deserializing a cache written before
/// a field existed on this struct (e.g. `you_reviewed`, added later) must not
/// fail. Without this, `load_cache`'s `serde_json::from_str` returns `Err`
/// for a *missing field*, which `load_cache` swallows into
/// `ProjectCache::default()` — silently discarding the entire cache, not
/// just this field: issues, pipelines, runners, releases, todos, milestones,
/// branches, and environments all vanish with it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
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
    /// The authenticated user, carried alongside the flags derived from them
    /// so the workflow cascade has one input struct. `None` when unknown.
    pub current_user: Option<String>,
    /// You submitted any review on this MR — any reviewer state other than
    /// `UNREVIEWED` (e.g. approved, requested changes, or reviewed).
    /// Distinct from `you_approved`: you can review without approving.
    pub you_reviewed: bool,
}

/// Merge readiness for one merge request. Independent of `ApprovalState`:
/// an MR can be approved *and* conflicted at the same time.
///
/// Container-level `#[serde(default)]` for the same reason as
/// `ApprovalState`: a future field added here must not turn a missing-field
/// error into a silently discarded cache.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
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
        Some(req) if req > 0 => format!("{given}/{req}"),
        _ => given.to_string(),
    }
}

/// Repeat the pending icon once per approval still needed (capped at 5).
/// Falls back to a single icon when `approvals_left` is unknown.
fn pending_icons(s: &ApprovalState, icon: &str) -> String {
    let n = s.approvals_left.unwrap_or(1).clamp(1, 5) as usize;
    std::iter::repeat_n(icon, n).collect::<Vec<_>>().join(" ")
}

/// First-match-wins cascade. See the precedence flowchart in the design spec.
pub fn approval_tone(state: Option<&ApprovalState>, is_github: bool) -> ApprovalTone {
    let Some(s) = state else {
        return ApprovalTone::Unknown;
    };
    if s.changes_requested {
        return ApprovalTone::ChangesRequested;
    }
    // GitHub exposes no canApprove, so it never awaits you.
    if !is_github && s.awaiting_you {
        return ApprovalTone::AwaitingYou;
    }
    if is_attributably_approved(s) {
        ApprovalTone::Approved
    } else {
        ApprovalTone::Pending
    }
}

pub fn approval_cell(
    state: Option<&ApprovalState>,
    is_github: bool,
    icons: &Icons,
) -> (String, ApprovalTone) {
    let tone = approval_tone(state, is_github);
    let Some(s) = state else {
        return ("—".to_string(), tone);
    };
    // GitHub exposes no counts, so it renders words only.
    let text = match (tone, is_github) {
        (ApprovalTone::ChangesRequested, true) => format!("{} CHANGES", icons.approval_changes),
        (ApprovalTone::ChangesRequested, false) => format!("{} CHG", icons.approval_changes),
        (ApprovalTone::Approved, true) => format!("{} APPROVED", icons.approval_approved),
        (ApprovalTone::Approved, false) => {
            format!("{} {}", icons.approval_approved, format_counts(s))
        }
        (ApprovalTone::Pending, true) => format!("{} REVIEW REQ", icons.approval_pending),
        (ApprovalTone::Pending, false) => format!(
            "{} {}",
            pending_icons(s, &icons.approval_pending),
            format_counts(s)
        ),
        (ApprovalTone::AwaitingYou, _) => {
            format!("{} AWAITING", pending_icons(s, &icons.approval_pending))
        }
        (ApprovalTone::Unknown, _) => "—".to_string(),
    };
    (text, tone)
}

/// First-match-wins cascade. Conflict outranks rebase because it is the more
/// blocking state and the only one the user cannot fix from the TUI. Known
/// state outranks `computing`.
pub fn mergeable_tone(state: Option<&MergeabilityState>) -> MergeTone {
    match state {
        None => MergeTone::Unknown,
        Some(s) if s.conflicts => MergeTone::Conflict,
        Some(s) if s.needs_rebase => MergeTone::Rebase,
        Some(s) if s.computing => MergeTone::Computing,
        Some(_) => MergeTone::Clean,
    }
}

pub fn mergeable_cell(state: Option<&MergeabilityState>, icons: &Icons) -> (String, MergeTone) {
    let tone = mergeable_tone(state);
    let text = match tone {
        MergeTone::Unknown => "—".to_string(),
        MergeTone::Conflict => format!("{} CONFLICT", icons.merge_conflict),
        MergeTone::Rebase => format!("{} REBASE", icons.merge_rebase),
        MergeTone::Computing => icons.merge_checking.clone(),
        MergeTone::Clean => format!("{} CLEAN", icons.merge_clean),
    };
    (text, tone)
}

/// Sort ordinal: most-blocking first, unknown last. The caller (`App::mr_sort_value`)
/// stringifies this via `.to_string()` before handing it to the table's sort
/// comparator, whose `u64` fast path then orders rows by state rather than
/// alphabetically by label.
pub fn approval_sort_key(state: Option<&ApprovalState>) -> u8 {
    match approval_tone(state, false) {
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
    match mergeable_tone(state) {
        MergeTone::Conflict => 0,
        MergeTone::Rebase => 1,
        MergeTone::Computing => 2,
        MergeTone::Clean => 3,
        MergeTone::Unknown => 4,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadsTone {
    Blocking,
    Clean,
}

/// The Preview pane's single `Threads:` line, merging two signals that
/// arrive at different times: whether unresolved threads block the merge
/// (`resolved`, always known once the MR list loads) and how many are
/// unresolved (`count`, known only after this MR's diff has been fetched).
/// `None` when `resolved` itself is unknown — GitLab always has it; GitHub
/// never does (see the call site in `ui/tabs.rs`, which explains why no
/// GitHub-side count is substituted instead).
///
/// A count of zero cannot *sharpen* a blocking claim, only contradict it —
/// `blocking` with a zero count is a stale-count artifact (the list flag
/// hadn't caught up with freshly-fetched notes), not evidence of zero open
/// threads while still blocking. So `(false, Some(0))` collapses into the
/// same wording as `(false, None)` rather than rendering the self-contradictory
/// "0 open, blocking merge".
pub fn threads_line_text(
    resolved: Option<bool>,
    count: Option<usize>,
    icons: &Icons,
) -> Option<(String, ThreadsTone)> {
    let resolved = resolved?;
    Some(match (resolved, count) {
        (false, None) | (false, Some(0)) => (
            format!("{} Blocking merge", icons.flag_unresolved),
            ThreadsTone::Blocking,
        ),
        (false, Some(n)) => (
            format!("{} {} open, blocking merge", icons.flag_unresolved, n),
            ThreadsTone::Blocking,
        ),
        (true, None) => (
            format!("{} Not blocking", icons.merge_clean),
            ThreadsTone::Clean,
        ),
        (true, Some(0)) => (
            format!("{} All resolved", icons.merge_clean),
            ThreadsTone::Clean,
        ),
        (true, Some(n)) => (
            format!("{} {} open, not blocking", icons.merge_clean, n),
            ThreadsTone::Clean,
        ),
    })
}

/// Independent flag glyphs appended to the Status cell's base word.
///
/// Each flag occupies its own slot, so `DRAFT` and the flag never displace one
/// another. Flags append in a fixed order so the cell is stable across
/// refreshes; later flags must be added to the end, not interleaved.
pub fn status_flags(blocking_discussions_resolved: Option<bool>, icons: &Icons) -> String {
    let mut out = String::new();
    // Only Some(false) is a problem. Some(true) and None render nothing.
    if blocking_discussions_resolved == Some(false) {
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
    let mut v = vec![if draft { "DRAFT" } else { "READY" }.to_string()];
    if blocking_discussions_resolved == Some(false) {
        v.push("UNRESOLVED".to_string());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── cache backward-compatibility ──

    #[test]
    fn approval_state_missing_new_fields_deserializes_with_defaults() {
        // Regression guard: without a container-level `#[serde(default)]`,
        // deserializing a cache written before `you_reviewed` (or
        // `current_user`) existed on this struct fails with "missing field",
        // and `load_cache` silently swallows that into
        // `ProjectCache::default()` — discarding not just this field but the
        // entire cache (issues, pipelines, runners, releases, todos,
        // milestones, branches, environments all vanish with it).
        let json = r#"{
            "approved": true,
            "approvals_left": null,
            "approvals_required": null,
            "approved_by": ["someone"],
            "changes_requested": false,
            "you_approved": false,
            "awaiting_you": false
        }"#;

        let state: ApprovalState =
            serde_json::from_str(json).expect("must deserialize despite missing fields");

        assert!(!state.you_reviewed);
        assert_eq!(state.current_user, None);
        // Fields that were present must still round-trip correctly, so this
        // isn't just falling back to a blanket default.
        assert!(state.approved);
        assert_eq!(state.approved_by, vec!["someone".to_string()]);
    }

    #[test]
    fn mergeability_state_missing_fields_deserializes_with_defaults() {
        let json = r#"{"conflicts": true}"#;

        let state: MergeabilityState =
            serde_json::from_str(json).expect("must deserialize despite missing fields");

        assert!(state.conflicts);
        assert!(!state.needs_rebase);
        assert!(!state.computing);
    }

    fn approved_by(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    // Build expected strings from the same icon set the cells are rendered with,
    // rather than duplicating glyph literals here.
    fn expect_chg() -> String {
        let icons = icons();
        format!("{} CHG", icons.approval_changes)
    }

    fn expect_approved(counts: &str) -> String {
        let icons = icons();
        format!("{} {}", icons.approval_approved, counts)
    }

    fn expect_github_approved() -> String {
        let icons = icons();
        format!("{} APPROVED", icons.approval_approved)
    }

    /// Builds the expected awaiting-you string: n icons + " AWAITING".
    fn expect_awaiting_you(approvals_left: u32) -> String {
        let icons = icons();
        let n = approvals_left.clamp(1, 5) as usize;
        let dots = std::iter::repeat_n(icons.approval_pending.as_str(), n)
            .collect::<Vec<_>>()
            .join(" ");
        format!("{dots} AWAITING")
    }

    /// Builds the expected pending string: n icons + " " + counts.
    fn expect_pending(approvals_left: u32, counts: &str) -> String {
        let icons = icons();
        let n = approvals_left.clamp(1, 5) as usize;
        let dots = std::iter::repeat_n(icons.approval_pending.as_str(), n)
            .collect::<Vec<_>>()
            .join(" ");
        format!("{dots} {counts}")
    }

    fn expect_github_pending() -> String {
        let icons = icons();
        format!("{} REVIEW REQ", icons.approval_pending)
    }

    fn expect_conflict() -> String {
        let icons = icons();
        format!("{} CONFLICT", icons.merge_conflict)
    }

    fn expect_rebase() -> String {
        let icons = icons();
        format!("{} REBASE", icons.merge_rebase)
    }

    fn expect_computing() -> String {
        icons().merge_checking
    }

    fn expect_clean() -> String {
        let icons = icons();
        format!("{} CLEAN", icons.merge_clean)
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
            ..Default::default()
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
        let (text, tone) = approval_cell(None, false, &icons());
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
            ..Default::default()
        };
        let (text, tone) = approval_cell(Some(&s), false, &icons());
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
            ..Default::default()
        };
        let (text, _) = approval_cell(Some(&s), false, &icons());
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
            ..Default::default()
        };
        let (text, _) = approval_cell(Some(&s), false, &icons());
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
            ..Default::default()
        };
        let (text, tone) = approval_cell(Some(&s), false, &icons());
        assert_ne!(tone, ApprovalTone::Approved);
        assert_eq!(text, expect_pending(0, "0"));
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
            ..Default::default()
        };
        let (text, tone) = approval_cell(Some(&s), false, &icons());
        assert_eq!(text, expect_awaiting_you(1));
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
            ..Default::default()
        };
        let (text, tone) = approval_cell(Some(&s), false, &icons());
        assert_eq!(text, expect_pending(1, "1/2"));
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
            ..Default::default()
        };
        let (text, _) = approval_cell(Some(&s), true, &icons());
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
            ..Default::default()
        };
        let (text, _) = approval_cell(Some(&s), true, &icons());
        assert_eq!(text, expect_github_pending());
    }

    // ── mergeability cell rendering ──

    #[test]
    fn mergeable_cell_unknown_renders_dash() {
        let (text, tone) = mergeable_cell(None, &icons());
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
        let (text, tone) = mergeable_cell(Some(&s), &icons());
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
        let (text, tone) = mergeable_cell(Some(&s), &icons());
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
        let (text, tone) = mergeable_cell(Some(&s), &icons());
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
        let (text, tone) = mergeable_cell(Some(&s), &icons());
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
            ..Default::default()
        };
        let approved = ApprovalState {
            approved: true,
            approvals_left: None,
            approvals_required: None,
            approved_by: approved_by(&["a"]),
            changes_requested: false,
            you_approved: false,
            awaiting_you: false,
            ..Default::default()
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
        assert!(!status_flags(Some(false), &icons()).is_empty());
    }

    #[test]
    fn resolved_discussions_produce_no_flag() {
        assert_eq!(status_flags(Some(true), &icons()), "");
    }

    #[test]
    fn unknown_discussions_produce_no_flag() {
        // An unknown must not look like a problem.
        assert_eq!(status_flags(None, &icons()), "");
    }

    #[test]
    fn status_filter_values_include_base_word_only_when_resolved() {
        assert_eq!(
            status_filter_values(true, Some(true)),
            vec!["DRAFT".to_string()]
        );
        assert_eq!(status_filter_values(false, None), vec!["READY".to_string()]);
    }

    #[test]
    fn status_filter_values_include_both_facts_when_unresolved() {
        // !1471 is draft AND unresolved; a filter on either must match it,
        // which is what preserves fidelity in the shared column.
        let v = status_filter_values(true, Some(false));
        assert!(v.contains(&"DRAFT".to_string()));
        assert!(v.contains(&"UNRESOLVED".to_string()));
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
        assert_eq!(workflow_cell(Some(WorkflowStatus::NotYours), &icons()), "");
        assert_eq!(workflow_cell(None, &icons()), "—");
        assert_ne!(
            workflow_cell(Some(WorkflowStatus::NotYours), &icons()),
            workflow_cell(None, &icons()),
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
    fn icon_is_nonempty_for_every_active_or_inactive_status_and_empty_for_not_yours_or_unknown() {
        use WorkflowStatus::*;
        for status in [
            ReturnedToYou,
            ReviewRequested,
            YourMergeRequest,
            ApprovedByYou,
            ApprovedByOthers,
            Inactive,
        ] {
            assert!(
                !workflow_icon(Some(status), &icons()).is_empty(),
                "{status:?} must have a glyph"
            );
        }
        assert_eq!(workflow_icon(Some(NotYours), &icons()), "");
        assert_eq!(workflow_icon(None, &icons()), "");
    }

    #[test]
    fn approved_by_others_has_its_own_icon() {
        let by_others = workflow_cell(Some(WorkflowStatus::ApprovedByOthers), &icons());
        let inactive = workflow_cell(Some(WorkflowStatus::Inactive), &icons());
        let mine = workflow_cell(Some(WorkflowStatus::ApprovedByYou), &icons());
        assert_ne!(
            by_others, inactive,
            "ApprovedByOthers must not share Inactive's icon"
        );
        assert_ne!(
            by_others, mine,
            "ApprovedByOthers must not share ApprovedByYou's icon"
        );
    }

    // ── threads line (Details pane matrix) ──

    fn icons() -> Icons {
        Icons::nerd()
    }

    #[test]
    fn threads_line_unknown_flag_is_omitted() {
        assert_eq!(threads_line_text(None, None, &icons()), None);
        assert_eq!(threads_line_text(None, Some(3), &icons()), None);
    }

    #[test]
    fn threads_line_blocking_with_unknown_count() {
        let (text, tone) = threads_line_text(Some(false), None, &icons()).unwrap();
        assert_eq!(text, format!("{} Blocking merge", icons().flag_unresolved));
        assert_eq!(tone, ThreadsTone::Blocking);
    }

    #[test]
    fn threads_line_blocking_with_known_count() {
        let (text, tone) = threads_line_text(Some(false), Some(3), &icons()).unwrap();
        assert_eq!(
            text,
            format!("{} 3 open, blocking merge", icons().flag_unresolved)
        );
        assert_eq!(tone, ThreadsTone::Blocking);
    }

    #[test]
    fn threads_line_blocking_with_zero_count_collapses_to_unknown_wording() {
        // The bug this guards: a stale `blocking` flag alongside a
        // freshly-fetched zero count must not render the self-contradictory
        // "0 open, blocking merge" — a zero count can only contradict a
        // blocking claim, never sharpen it, so this must render identically
        // to the unknown-count row.
        let with_zero = threads_line_text(Some(false), Some(0), &icons()).unwrap();
        let unknown = threads_line_text(Some(false), None, &icons()).unwrap();
        assert_eq!(with_zero, unknown);
    }

    #[test]
    fn threads_line_clean_with_unknown_count() {
        let (text, tone) = threads_line_text(Some(true), None, &icons()).unwrap();
        assert_eq!(text, format!("{} Not blocking", icons().merge_clean));
        assert_eq!(tone, ThreadsTone::Clean);
    }

    #[test]
    fn threads_line_clean_with_zero_count() {
        let (text, tone) = threads_line_text(Some(true), Some(0), &icons()).unwrap();
        assert_eq!(text, format!("{} All resolved", icons().merge_clean));
        assert_eq!(tone, ThreadsTone::Clean);
    }

    #[test]
    fn threads_line_clean_with_positive_count() {
        // The row that justifies keeping both signals separate: threads are
        // open but not required to be resolved, so they do not block.
        let (text, tone) = threads_line_text(Some(true), Some(3), &icons()).unwrap();
        assert_eq!(
            text,
            format!("{} 3 open, not blocking", icons().merge_clean)
        );
        assert_eq!(tone, ThreadsTone::Clean);
    }
}
