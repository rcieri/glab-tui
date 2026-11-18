//! The action registry: every key glab-tui handles in a list view, the diff
//! view, a selector or a confirmation dialog is one [`Action`] row here, with
//! the context it acts in, its config name, its default keys, its help label
//! and whether it destroys something. Handlers, the help modal and the
//! custom-command shadow report all read this one table. The review threads
//! and submit-review overlays, the configure popup, the date picker,
//! edit-menu navigation and the help list still match literal keys.
//!
//! `[keybindings.<table>] <field> = "key"` (or a list of keys) replaces an
//! action's default keys in its context: the old keys stop triggering it.

use crate::app::Tab;
use crate::custom_commands::ConfigProblem;
use crate::keybinding::{KeyPress, is_readable_binding, keybinding_matches};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where an action is looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyContext {
    /// Keys every list view handles before the active tab sees them, or that
    /// act on the shared detail pane.
    Global,
    /// Keys every list view handles after the active tab declined them.
    ListView,
    Tab(Tab),
    DiffView,
    Selector,
    /// The confirmation dialog in front of a mutating action.
    Dialog,
    EditMenu,
}

impl KeyContext {
    /// The `[keybindings.<table>]` that remaps this context's actions.
    /// `submit_edit` predates the edit-menu context and stays under `global`
    /// so existing configs keep working.
    pub fn table(self) -> &'static str {
        match self {
            KeyContext::Global | KeyContext::EditMenu => "global",
            KeyContext::ListView => "list",
            KeyContext::Tab(tab) => tab_table(tab),
            KeyContext::DiffView => "diff",
            KeyContext::Selector => "selector",
            KeyContext::Dialog => "dialog",
        }
    }
}

/// The `[keybindings.<table>]` and `[custom_keybindings.<pane>]` name of a tab.
pub fn tab_table(tab: Tab) -> &'static str {
    match tab {
        Tab::Dashboard => "dashboard",
        Tab::Issues => "issues",
        Tab::MergeRequests => "mrs",
        Tab::Pipelines => "pipelines",
        Tab::Jobs => "jobs",
        Tab::Runners => "runners",
        Tab::Releases => "releases",
        Tab::Todos => "todos",
        Tab::Milestones => "milestones",
        Tab::Branches => "branches",
        Tab::Environments => "environments",
        Tab::Terminal => "terminal",
    }
}

/// Which hosts an action does anything on. The key is taken on both, so a
/// custom binding never fires in its place. The help modal hides it where it
/// does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    Both,
    GitLab,
    GitHub,
}

impl Support {
    pub fn includes(self, kind: crate::backend::BackendKind) -> bool {
        match self {
            Support::Both => true,
            Support::GitLab => kind.is_gitlab(),
            Support::GitHub => kind.is_github(),
        }
    }
}

/// One registry row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionSpec {
    pub context: KeyContext,
    /// The key under `[keybindings.<context table>]`.
    pub field: &'static str,
    pub defaults: &'static [&'static str],
    /// Help text. `{mr}` reads `MR` or `PR`, `{merge_request}` reads
    /// `Merge Request` or `Pull Request` and `{pipelines}` reads `Pipelines`
    /// or `Actions`, after the host.
    pub label: &'static str,
    /// The help text on GitHub when it differs beyond the host's terms.
    pub github_label: Option<&'static str>,
    /// Destroys or reverts something on the forge: the key only ever opens a
    /// confirmation (see `handlers::confirm`).
    pub destructive: bool,
    pub support: Support,
}

impl ActionSpec {
    const fn new(
        context: KeyContext,
        field: &'static str,
        defaults: &'static [&'static str],
        label: &'static str,
    ) -> Self {
        Self {
            context,
            field,
            defaults,
            label,
            github_label: None,
            destructive: false,
            support: Support::Both,
        }
    }

    const fn destructive(self) -> Self {
        Self {
            destructive: true,
            ..self
        }
    }

    const fn only(self, support: Support) -> Self {
        Self { support, ..self }
    }

    const fn on_github(self, label: &'static str) -> Self {
        Self {
            github_label: Some(label),
            ..self
        }
    }

    /// The help text for `kind`, with the host's own terms.
    pub fn label_for(&self, kind: crate::backend::BackendKind) -> String {
        let label = match self.github_label {
            Some(label) if kind.is_github() => label,
            _ => self.label,
        };
        label
            .replace("{merge_request}", kind.term("mr"))
            .replace("{mr}", kind.term("mr_short"))
            .replace("{pipelines}", kind.term("pipeline_plural"))
    }
}

macro_rules! registry {
    ( $( $action:ident => $spec:expr ; )+ ) => {
        /// Every key-driven action. Its rows live in [`ACTIONS`], in the
        /// same order.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Action {
            $( $action ),+
        }

        impl Action {
            pub const ALL: &'static [Action] = &[ $( Action::$action ),+ ];
        }

        /// The registry, indexed by `Action as usize`.
        pub const ACTIONS: &[ActionSpec] = &[ $( $spec ),+ ];
    };
}

use KeyContext::{Dialog, DiffView, EditMenu, Global, ListView, Selector};
const ISSUES: KeyContext = KeyContext::Tab(Tab::Issues);
const MRS: KeyContext = KeyContext::Tab(Tab::MergeRequests);
const PIPELINES: KeyContext = KeyContext::Tab(Tab::Pipelines);
const JOBS: KeyContext = KeyContext::Tab(Tab::Jobs);
const RUNNERS: KeyContext = KeyContext::Tab(Tab::Runners);
const RELEASES: KeyContext = KeyContext::Tab(Tab::Releases);
const TODOS: KeyContext = KeyContext::Tab(Tab::Todos);
const MILESTONES: KeyContext = KeyContext::Tab(Tab::Milestones);
const BRANCHES: KeyContext = KeyContext::Tab(Tab::Branches);
const ENVIRONMENTS: KeyContext = KeyContext::Tab(Tab::Environments);
const TERMINAL: KeyContext = KeyContext::Tab(Tab::Terminal);
const DASHBOARD: KeyContext = KeyContext::Tab(Tab::Dashboard);

registry! {
    GlobalForceQuit => ActionSpec::new(Global, "force_quit", &["Ctrl+c"], "Quit program, even from an overlay");
    GlobalQuit => ActionSpec::new(Global, "quit", &["q"], "Quit program");
    GlobalHelp => ActionSpec::new(Global, "help", &["?", "F1"], "Show this help modal");
    GlobalSearch => ActionSpec::new(Global, "search", &["/"], "Open fuzzy search / filter bar");
    GlobalRefresh => ActionSpec::new(Global, "refresh", &["Ctrl+r", "F5"], "Refresh active tab data");
    GlobalSwitchRepo => ActionSpec::new(Global, "switch_repo", &["Ctrl+s"], "Switch repository");
    GlobalConfigure => ActionSpec::new(Global, "configure", &["Tab", ","], "Toggle columns config popup (filter / group / sort)");
    GlobalJumpToId => ActionSpec::new(Global, "jump_to_id", &["g"], "Jump to issue/MR by ID; fetch if not cached");
    GlobalSortNext => ActionSpec::new(Global, "sort_next", &[">"], "Sort list by next column");
    GlobalSortPrev => ActionSpec::new(Global, "sort_prev", &["<"], "Sort list by previous column");
    GlobalSortReverse => ActionSpec::new(Global, "sort_reverse", &["="], "Reverse sort order");
    GlobalNextTab => ActionSpec::new(Global, "next_tab", &["l", "Right"], "Next tab");
    GlobalPrevTab => ActionSpec::new(Global, "prev_tab", &["h", "Left"], "Previous tab");
    GlobalScrollDown => ActionSpec::new(Global, "scroll_down", &["J"], "Scroll description / trace / notes down");
    GlobalScrollUp => ActionSpec::new(Global, "scroll_up", &["K"], "Scroll description / trace / notes up");
    GlobalScrollPageDown => ActionSpec::new(Global, "scroll_page_down", &["PageDown"], "Scroll description pane down by one page");
    GlobalScrollPageUp => ActionSpec::new(Global, "scroll_page_up", &["PageUp"], "Scroll description pane up by one page");
    GlobalScrollHalfPageDown => ActionSpec::new(Global, "scroll_half_page_down", &["Ctrl+d"], "Scroll description pane (or the list when it is hidden) down by half a page");
    GlobalScrollHalfPageUp => ActionSpec::new(Global, "scroll_half_page_up", &["Ctrl+u"], "Scroll description pane (or the list when it is hidden) up by half a page");
    GlobalScrollToEnd => ActionSpec::new(Global, "scroll_to_end", &["End"], "Jump to the last line of the description pane");
    GlobalScrollTop => ActionSpec::new(Global, "scroll_top", &["Home"], "Jump to the top of the description pane");
    GlobalLeaveScope => ActionSpec::new(Global, "leave_scope", &["Esc"], "Return from a project narrowed with G to its group");
    EditMenuSubmit => ActionSpec::new(EditMenu, "submit_edit", &["Ctrl+x"], "Submit active edit form");

    ListNextRow => ActionSpec::new(ListView, "next_row", &["j", "Down"], "Select the next row (scroll a zoomed pane)");
    ListPreviousRow => ActionSpec::new(ListView, "previous_row", &["k", "Up"], "Select the previous row (scroll a zoomed pane)");
    ListFirstRow => ActionSpec::new(ListView, "first_row", &["Home"], "Select the first row");
    ListLastRow => ActionSpec::new(ListView, "last_row", &["End"], "Select the last row");
    ListOpen => ActionSpec::new(ListView, "open", &["Enter"], "Open details, zoom, then edit; descend into a pipeline");
    ListBack => ActionSpec::new(ListView, "back", &["Esc", "Backspace"], "Go back: clear the selection, close details, leave a pipeline, clear the filter");
    ListSearch => ActionSpec::new(ListView, "search", &["f"], "Open fuzzy search / filter bar (unless the tab uses the key)");
    ListCheckUpdates => ActionSpec::new(ListView, "check_updates", &["u"], "Check for updates");

    IssuesCreateIssue => ActionSpec::new(ISSUES, "create_issue", &["n"], "Create new Issue");
    IssuesEditEntity => ActionSpec::new(ISSUES, "edit_entity", &["e"], "Open parameter edit menu (bulk edit with a selection)");
    IssuesCloseEntity => ActionSpec::new(ISSUES, "close_entity", &["c"], "Close selected Issue").destructive();
    IssuesReopenEntity => ActionSpec::new(ISSUES, "reopen_entity", &["r"], "Reopen selected Issue").destructive();
    IssuesDeleteEntity => ActionSpec::new(ISSUES, "delete_entity", &["d"], "Delete selected Issue").destructive();
    IssuesSelectIssue => ActionSpec::new(ISSUES, "select_issue", &["Space"], "Toggle issue selection (bulk edit with e)");
    IssuesCreateMr => ActionSpec::new(ISSUES, "create_mr", &["m"], "Create {merge_request} from selected Issue");
    IssuesOpenInBrowser => ActionSpec::new(ISSUES, "open_in_browser", &["o"], "Open selected Issue in browser");
    IssuesCopyReference => ActionSpec::new(ISSUES, "copy_reference", &["y"], "Copy selected Issue as Markdown link");
    IssuesSelectionToggle => ActionSpec::new(ISSUES, "selection_toggle", &["v"], "Toggle select mode (paint selection while navigating)");
    IssuesSelectAll => ActionSpec::new(ISSUES, "select_all", &["Ctrl+a"], "Select all filtered issues (in select mode)");
    IssuesDrillIntoScope => ActionSpec::new(ISSUES, "drill_into_scope", &["G"], "Narrow a group scope to the selected issue's project");
    IssuesJumpRelatedMrs => ActionSpec::new(ISSUES, "jump_related_mrs", &["M"], "Jump to related {merge_request}s");

    MrsCreateMr => ActionSpec::new(MRS, "create_mr", &["n"], "Create new {merge_request}");
    MrsApproveMr => ActionSpec::new(MRS, "approve_mr", &["a"], "Approve selected {mr}");
    MrsRevokeMr => ActionSpec::new(MRS, "revoke_mr", &["A"], "Revoke your approval").destructive().only(Support::GitLab);
    MrsRebaseMr => ActionSpec::new(MRS, "rebase_mr", &["R"], "Rebase source branch onto target");
    MrsMergeMr => ActionSpec::new(MRS, "merge_mr", &["m"], "Merge selected {mr} (configure squash/delete)").on_github("Merge selected {mr} (a stacked PR merges with the open PRs below it)");
    MrsToggleDraft => ActionSpec::new(MRS, "toggle_draft", &["s"], "Toggle Draft / Ready status");
    MrsViewDiff => ActionSpec::new(MRS, "view_diff", &["D"], "View {merge_request} diff changes");
    MrsViewRelatedPipelines => ActionSpec::new(MRS, "view_related_pipelines", &["P"], "View related {pipelines} for selected {mr}");
    MrsEditEntity => ActionSpec::new(MRS, "edit_entity", &["e"], "Open parameter edit menu (bulk edit with a selection)");
    MrsCloseEntity => ActionSpec::new(MRS, "close_entity", &["c"], "Close selected {mr}").destructive();
    MrsReopenEntity => ActionSpec::new(MRS, "reopen_entity", &["r"], "Reopen selected {mr}").destructive();
    MrsDeleteEntity => ActionSpec::new(MRS, "delete_entity", &["d"], "Delete selected {mr}").destructive().only(Support::GitLab);
    MrsSelectMr => ActionSpec::new(MRS, "select_mr", &["Space"], "Toggle {mr} selection (bulk edit/merge with e/m)");
    MrsOpenInBrowser => ActionSpec::new(MRS, "open_in_browser", &["o"], "Open selected {mr} in browser");
    MrsSelectionToggle => ActionSpec::new(MRS, "selection_toggle", &["v"], "Toggle select mode (paint selection while navigating)");
    MrsSelectAll => ActionSpec::new(MRS, "select_all", &["Ctrl+a"], "Select all filtered {mr}s (in select mode)");
    MrsDrillIntoScope => ActionSpec::new(MRS, "drill_into_scope", &["G"], "Narrow a group scope to the selected {mr}'s project");
    MrsCopyReference => ActionSpec::new(MRS, "copy_reference", &["y"], "Copy selected {mr} as Markdown link");
    MrsViewStack => ActionSpec::new(MRS, "view_stack", &["Y"], "Browse the stack of the selected stacked PR").only(Support::GitHub);
    MrsJumpLinkedIssues => ActionSpec::new(MRS, "jump_linked_issues", &["I"], "Jump to linked Issues");

    PipelinesTriggerPipeline => ActionSpec::new(PIPELINES, "trigger_pipeline", &["p"], "Trigger a new pipeline from the current branch");
    PipelinesRunNew => ActionSpec::new(PIPELINES, "run_new", &["n"], "Create pipeline with interactive form");
    PipelinesEnterJobs => ActionSpec::new(PIPELINES, "enter_jobs", &["Alt+j"], "View the selected pipeline's own jobs");
    PipelinesRetry => ActionSpec::new(PIPELINES, "retry", &["r"], "Retry selected pipeline(s)");
    PipelinesRetryFailed => ActionSpec::new(PIPELINES, "retry_failed", &["R"], "Retry only the failed jobs of the selected pipeline(s)");
    PipelinesCancel => ActionSpec::new(PIPELINES, "cancel", &["d"], "Cancel pipeline execution").destructive();
    PipelinesOpenWorkflow => ActionSpec::new(PIPELINES, "open_workflow", &["W"], "Open pipeline workflow in browser").only(Support::GitHub);
    PipelinesOpenInBrowser => ActionSpec::new(PIPELINES, "open_in_browser", &["o"], "Open pipeline in browser");
    PipelinesCopySha => ActionSpec::new(PIPELINES, "copy_sha", &["C"], "Copy selected pipeline commit SHA");
    PipelinesCopyUrl => ActionSpec::new(PIPELINES, "copy_url", &["y"], "Copy selected pipeline URL");
    PipelinesCopyId => ActionSpec::new(PIPELINES, "copy_id", &["Y"], "Copy selected pipeline ID (#123)");
    PipelinesCycleStatusFilter => ActionSpec::new(PIPELINES, "cycle_status_filter", &["s"], "Filter by status: All / Running / Failed / Success");
    PipelinesTogglePreview => ActionSpec::new(PIPELINES, "toggle_preview", &["i"], "Show / hide the pipeline inspector (commit, failed jobs)");
    PipelinesSelectPipeline => ActionSpec::new(PIPELINES, "select_pipeline", &["Space"], "Check / uncheck pipeline for bulk retry");
    PipelinesSelectionToggle => ActionSpec::new(PIPELINES, "selection_toggle", &["v"], "Toggle select mode (paint selection while navigating)");
    PipelinesSelectAll => ActionSpec::new(PIPELINES, "select_all", &["Ctrl+a"], "Select all filtered pipelines (in select mode)");

    JobsEnterPipeline => ActionSpec::new(JOBS, "enter_pipeline", &["p"], "Switch to pipeline selector");
    JobsSelectJob => ActionSpec::new(JOBS, "select_job", &["Space"], "Check / uncheck job for bulk retry/cancel");
    JobsRetry => ActionSpec::new(JOBS, "retry", &["r"], "Retry selected job(s)");
    JobsStartJob => ActionSpec::new(JOBS, "start_job", &["S"], "Start manual job").only(Support::GitLab);
    JobsSelectStage => ActionSpec::new(JOBS, "select_stage", &["s"], "Select all jobs in stage");
    JobsCancel => ActionSpec::new(JOBS, "cancel", &["c"], "Cancel selected job(s)").destructive();
    JobsDownloadArtifact => ActionSpec::new(JOBS, "download_artifact", &["d"], "Download job artifact");
    JobsOpenInBrowser => ActionSpec::new(JOBS, "open_in_browser", &["o"], "Open selected job in browser");
    JobsViewTraceEditor => ActionSpec::new(JOBS, "view_trace_editor", &["e"], "Open job trace in external $EDITOR");
    JobsViewTrace => ActionSpec::new(JOBS, "view_trace", &["Enter"], "View job trace (toggle zoom)");
    JobsToggleTraceWrap => ActionSpec::new(JOBS, "toggle_trace_wrap", &["w"], "Toggle trace word wrap / clipped view");
    JobsTraceSearch => ActionSpec::new(JOBS, "trace_search", &["/"], "Search within job trace");
    JobsClearTraceSearch => ActionSpec::new(JOBS, "clear_trace_search", &["Esc"], "Clear the job trace search");
    JobsToggleTraceFollow => ActionSpec::new(JOBS, "toggle_trace_follow", &["f"], "Toggle trace auto-follow mode").only(Support::GitLab);
    JobsJumpToFailure => ActionSpec::new(JOBS, "jump_to_failure", &["E"], "Jump to the next failure line in the job trace");
    JobsCopyUrl => ActionSpec::new(JOBS, "copy_url", &["y"], "Copy job web URL");
    JobsCopyId => ActionSpec::new(JOBS, "copy_id", &["Y"], "Copy job ID");
    JobsCopySha => ActionSpec::new(JOBS, "copy_sha", &["Alt+y"], "Copy pipeline commit SHA");
    JobsSelectionToggle => ActionSpec::new(JOBS, "selection_toggle", &["v"], "Toggle select mode (paint selection while navigating)");
    JobsSelectAll => ActionSpec::new(JOBS, "select_all", &["Ctrl+a"], "Select all filtered jobs (in select mode)");

    RunnersPause => ActionSpec::new(RUNNERS, "pause", &["p"], "Pause runner").only(Support::GitLab);
    RunnersResume => ActionSpec::new(RUNNERS, "resume", &["r"], "Resume runner").only(Support::GitLab);
    RunnersEditDescription => ActionSpec::new(RUNNERS, "edit_description", &["e"], "Edit runner description text").only(Support::GitLab);
    RunnersOpenInBrowser => ActionSpec::new(RUNNERS, "open_in_browser", &["o"], "Open runner in browser");
    RunnersCycleStatus => ActionSpec::new(RUNNERS, "cycle_status", &["s"], "Cycle the status filter: Online, Paused, Offline, All").on_github("Cycle the status filter: Online, Offline, All");
    RunnersCopyUrl => ActionSpec::new(RUNNERS, "copy_url", &["y"], "Copy runner URL");
    RunnersCopyId => ActionSpec::new(RUNNERS, "copy_id", &["Y"], "Copy runner ID");

    ReleasesCreateRelease => ActionSpec::new(RELEASES, "create_release", &["n"], "Create new release");
    ReleasesEditRelease => ActionSpec::new(RELEASES, "edit_release", &["e"], "Edit selected release");
    ReleasesDeleteRelease => ActionSpec::new(RELEASES, "delete_release", &["d"], "Delete selected release").destructive();
    ReleasesOpenInBrowser => ActionSpec::new(RELEASES, "open_in_browser", &["o"], "Open release in browser");

    MilestonesCreateMilestone => ActionSpec::new(MILESTONES, "create_milestone", &["n"], "Create new milestone");
    MilestonesEditMilestone => ActionSpec::new(MILESTONES, "edit_milestone", &["e"], "Edit selected milestone");
    MilestonesCloseMilestone => ActionSpec::new(MILESTONES, "close_milestone", &["c"], "Close selected milestone").destructive();
    MilestonesReopenMilestone => ActionSpec::new(MILESTONES, "reopen_milestone", &["r"], "Reopen selected milestone").destructive();
    MilestonesDeleteMilestone => ActionSpec::new(MILESTONES, "delete_milestone", &["d"], "Delete selected milestone").destructive();
    MilestonesOpenInBrowser => ActionSpec::new(MILESTONES, "open_in_browser", &["o"], "Open milestone in browser");

    TodosMarkAsRead => ActionSpec::new(TODOS, "mark_as_read", &["Enter"], "Open todo target & mark read (selection: mark all selected read)");
    TodosOpenInBrowser => ActionSpec::new(TODOS, "open_in_browser", &["o"], "Open todo in browser");
    TodosMarkAsDone => ActionSpec::new(TODOS, "mark_as_done", &["d"], "Mark todo (or all selected) done");
    TodosMarkAllAsRead => ActionSpec::new(TODOS, "mark_all_as_read", &["A"], "Mark all todos read").destructive();
    TodosUnsubscribe => ActionSpec::new(TODOS, "unsubscribe", &["U"], "Unsubscribe from todo target");
    TodosToggleShowRead => ActionSpec::new(TODOS, "toggle_show_read", &["r"], "Toggle showing read todos");
    TodosSelectTodo => ActionSpec::new(TODOS, "select_todo", &["Space"], "Toggle selection of todo");
    TodosSelectionToggle => ActionSpec::new(TODOS, "selection_toggle", &["v"], "Toggle select mode (paint selection while navigating)");
    TodosSelectAll => ActionSpec::new(TODOS, "select_all", &["Ctrl+a"], "Select all filtered todos");
    TodosCopyUrl => ActionSpec::new(TODOS, "copy_url", &["y"], "Copy todo target URL");
    TodosCopyReference => ActionSpec::new(TODOS, "copy_reference", &["Y"], "Copy todo target reference (#123 / !456)");
    TodosTogglePreview => ActionSpec::new(TODOS, "toggle_preview", &["p"], "Show / hide the todo preview");

    BranchesCreateBranch => ActionSpec::new(BRANCHES, "create_branch", &["n"], "Create new branch");
    BranchesDeleteBranch => ActionSpec::new(BRANCHES, "delete_branch", &["d"], "Delete selected branch(es)").destructive();
    BranchesOpenInBrowser => ActionSpec::new(BRANCHES, "open_in_browser", &["o"], "Open branch in browser");
    BranchesCopyBranch => ActionSpec::new(BRANCHES, "copy_branch", &["y"], "Copy selected branch name");
    BranchesCheckoutBranch => ActionSpec::new(BRANCHES, "checkout_branch", &["C"], "Checkout selected branch");
    BranchesCreateMr => ActionSpec::new(BRANCHES, "create_mr", &["m"], "Create {merge_request} from selected branch");
    BranchesSelectBranch => ActionSpec::new(BRANCHES, "select_branch", &["Space"], "Toggle branch selection");
    BranchesSelectionToggle => ActionSpec::new(BRANCHES, "selection_toggle", &["v"], "Toggle visual select mode");
    BranchesSelectAll => ActionSpec::new(BRANCHES, "select_all", &["Ctrl+a"], "Select all visible branches");

    EnvironmentsViewDeployments => ActionSpec::new(ENVIRONMENTS, "view_deployments", &["Enter"], "List and focus the environment's deployments");
    EnvironmentsOpenInBrowser => ActionSpec::new(ENVIRONMENTS, "open_in_browser", &["o"], "Open environment in browser");
    EnvironmentsFocusDeployments => ActionSpec::new(ENVIRONMENTS, "focus_deployments", &["Tab"], "Move focus between environments and deployments");
    EnvironmentsRollbackDeployment => ActionSpec::new(ENVIRONMENTS, "rollback_deployment", &["R"], "Roll back to the selected deployment").destructive();
    EnvironmentsStopEnvironment => ActionSpec::new(ENVIRONMENTS, "stop_environment", &["d"], "Stop environment").destructive();
    EnvironmentsCopyUrl => ActionSpec::new(ENVIRONMENTS, "copy_url", &["y"], "Copy environment URL");
    EnvironmentsViewDeploymentLog => ActionSpec::new(ENVIRONMENTS, "view_deployment_log", &["e"], "Open the selected deployment's job log in $EDITOR").only(Support::GitLab);

    TerminalToggleWrap => ActionSpec::new(TERMINAL, "toggle_wrap", &["w"], "Toggle terminal line wrapping");
    TerminalClearLog => ActionSpec::new(TERMINAL, "clear_log", &["C"], "Clear the terminal log");
    TerminalCopyCommand => ActionSpec::new(TERMINAL, "copy_command", &["y"], "Copy the highlighted command line");
    TerminalCopyOutput => ActionSpec::new(TERMINAL, "copy_output", &["Y"], "Copy the highlighted command's output or error");
    TerminalInspectCommand => ActionSpec::new(TERMINAL, "inspect_command", &["Enter", "P"], "Inspect the highlighted command (full command, timing, stdout, stderr)");
    TerminalRerunCommand => ActionSpec::new(TERMINAL, "rerun_command", &["r"], "Re-run the highlighted glab, gh or git call");
    TerminalCycleStatusFilter => ActionSpec::new(TERMINAL, "cycle_status_filter", &["s"], "Cycle the status filter: All, Errors, Success");

    DashboardCycleWindow => ActionSpec::new(DASHBOARD, "cycle_window", &["w"], "Contributors: open work, then activity in the last day, week, month");

    DiffBack => ActionSpec::new(DiffView, "back", &["Esc"], "Leave search or selection, focus the file tree, then exit the diff view");
    DiffClose => ActionSpec::new(DiffView, "close", &["q"], "Cancel the selection, or exit the diff view");
    DiffToggleFocus => ActionSpec::new(DiffView, "toggle_focus", &["Tab"], "Toggle focus (files / diff)");
    DiffFocusFiles => ActionSpec::new(DiffView, "focus_files", &["h", "Left"], "Collapse a directory, or focus the file tree");
    DiffFocusDiff => ActionSpec::new(DiffView, "focus_diff", &["l", "Right"], "Expand a directory, or open the file");
    DiffOpen => ActionSpec::new(DiffView, "open", &["Enter", "Space"], "Expand file tree entry / toggle zoom");
    DiffNextLine => ActionSpec::new(DiffView, "next_line", &["j", "Down"], "Move down (files or diff lines)");
    DiffPreviousLine => ActionSpec::new(DiffView, "previous_line", &["k", "Up"], "Move up (files or diff lines)");
    DiffPageDown => ActionSpec::new(DiffView, "page_down", &["J"], "Move down 10 lines / files");
    DiffPageUp => ActionSpec::new(DiffView, "page_up", &["K"], "Move up 10 lines / files");
    DiffPreviousHunk => ActionSpec::new(DiffView, "previous_hunk", &["["], "Previous hunk");
    DiffNextHunk => ActionSpec::new(DiffView, "next_hunk", &["]"], "Next hunk");
    DiffCollapseAll => ActionSpec::new(DiffView, "collapse_all", &["z"], "Collapse all files");
    DiffExpandAll => ActionSpec::new(DiffView, "expand_all", &["Z"], "Expand all files");
    DiffToggleReviewed => ActionSpec::new(DiffView, "toggle_reviewed", &["m"], "Mark / unmark file (or directory) as reviewed");
    DiffToggleHideReviewed => ActionSpec::new(DiffView, "toggle_hide_reviewed", &["M"], "Hide / show reviewed files in the tree");
    DiffToggleSideBySide => ActionSpec::new(DiffView, "toggle_side_by_side", &["d"], "Toggle unified / side-by-side layout (pinned for the session)");
    DiffLayoutSideBySide => ActionSpec::new(DiffView, "layout_side_by_side", &["1"], "Pin the side-by-side layout for the session");
    DiffLayoutUnified => ActionSpec::new(DiffView, "layout_unified", &["2"], "Pin the unified layout for the session");
    DiffLayoutAuto => ActionSpec::new(DiffView, "layout_auto", &["0"], "Pick the layout from the diff pane width again (unified when narrow)");
    DiffSearch => ActionSpec::new(DiffView, "search", &["/", "f"], "Search within diff");
    DiffSearchNext => ActionSpec::new(DiffView, "search_next", &["Ctrl+n"], "Next search match");
    DiffSearchPrevious => ActionSpec::new(DiffView, "search_previous", &["Ctrl+p"], "Previous search match");
    DiffToggleSelection => ActionSpec::new(DiffView, "toggle_selection", &["v", "V"], "Start / stop line selection for comment");
    DiffComment => ActionSpec::new(DiffView, "comment", &["c"], "Add comment on line / selection");
    DiffCommentInEditor => ActionSpec::new(DiffView, "comment_in_editor", &["C"], "Add comment via external $EDITOR");
    DiffSuggest => ActionSpec::new(DiffView, "suggest", &["e"], "Add code suggestion via $EDITOR");
    DiffCommentActions => ActionSpec::new(DiffView, "comment_actions", &["a"], "Interact with comments on the current line (reply, resolve, edit, delete)");
    DiffReviewThreads => ActionSpec::new(DiffView, "review_threads", &["T"], "Review threads overview (general, outdated, unresolved)");
    DiffSubmitReview => ActionSpec::new(DiffView, "submit_review", &["r"], "Submit review (comment / approve / request changes / keep as draft)");

    SelectorNext => ActionSpec::new(Selector, "next", &["j", "Down"], "Next option");
    SelectorPrevious => ActionSpec::new(Selector, "previous", &["k", "Up"], "Previous option");
    SelectorToggle => ActionSpec::new(Selector, "toggle", &["Space"], "Toggle item (multi-select)");
    SelectorConfirm => ActionSpec::new(Selector, "confirm", &["Enter"], "Confirm selection");
    SelectorFilter => ActionSpec::new(Selector, "filter", &["f", "/", "i"], "Type to fuzzy filter the options");
    SelectorClose => ActionSpec::new(Selector, "close", &["Esc"], "Clear the filter, then close the selector");

    DialogNextField => ActionSpec::new(Dialog, "next_field", &["j", "Down"], "Next option / button row");
    DialogPreviousField => ActionSpec::new(Dialog, "previous_field", &["k", "Up"], "Previous option / button row");
    DialogNextButton => ActionSpec::new(Dialog, "next_button", &["l", "Right"], "Next button");
    DialogPreviousButton => ActionSpec::new(Dialog, "previous_button", &["h", "Left"], "Previous button");
    DialogNext => ActionSpec::new(Dialog, "next", &["Tab"], "Next control");
    DialogPrevious => ActionSpec::new(Dialog, "previous", &["Shift+Tab"], "Previous control");
    DialogToggleOption => ActionSpec::new(Dialog, "toggle_option", &["Space"], "Toggle the focused option");
    DialogActivate => ActionSpec::new(Dialog, "activate", &["Enter"], "Activate the focused button");
    DialogSubmit => ActionSpec::new(Dialog, "submit", &["y", "Y"], "Submit while the Submit button is focused");
    DialogCancel => ActionSpec::new(Dialog, "cancel", &["Esc"], "Cancel");
}

/// Global actions that still act while the diff view is open, ahead of the
/// diff view's own actions.
pub const DIFF_VIEW_GLOBAL_ACTIONS: &[Action] = &[
    Action::GlobalForceQuit,
    Action::GlobalHelp,
    Action::GlobalRefresh,
    Action::GlobalSwitchRepo,
    Action::GlobalScrollHalfPageDown,
    Action::GlobalScrollHalfPageUp,
];

impl Action {
    pub fn spec(self) -> &'static ActionSpec {
        &ACTIONS[self as usize]
    }

    pub fn is_destructive(self) -> bool {
        self.spec().destructive
    }

    pub fn context(self) -> KeyContext {
        self.spec().context
    }

    /// `keybindings.<table>.<field>`, the config key that remaps it.
    pub fn config_key(self) -> String {
        let spec = self.spec();
        format!("keybindings.{}.{}", spec.context.table(), spec.field)
    }

    /// Actions of `context`, in registry order.
    pub fn in_context(context: KeyContext) -> impl Iterator<Item = Action> {
        Action::ALL
            .iter()
            .copied()
            .filter(move |action| action.context() == context)
    }
}

/// One config value: a single key or a list of keys, every one of which
/// triggers the action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum BindingValue {
    One(String),
    Many(Vec<String>),
}

type BindingTables = BTreeMap<String, BTreeMap<String, BindingValue>>;

/// `[keybindings]` as written, so an entry the registry does not know is
/// reported instead of failing the whole config.
type RawTables = BTreeMap<String, toml::Value>;

/// The keys bound to every action: the registry defaults with the config's
/// `[keybindings.<table>]` entries in their place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RawTables", into = "BindingTables")]
pub struct Keymap {
    bindings: Vec<Vec<String>>,
    problems: Vec<ConfigProblem>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            bindings: ACTIONS
                .iter()
                .map(|spec| spec.defaults.iter().map(|key| key.to_string()).collect())
                .collect(),
            problems: Vec::new(),
        }
    }
}

impl From<RawTables> for Keymap {
    fn from(tables: RawTables) -> Self {
        let mut keymap = Keymap::default();
        for (table_name, value) in tables {
            let table_entry = format!("keybindings.{table_name}");
            let is_known_table = ACTIONS
                .iter()
                .any(|spec| spec.context.table() == table_name);
            let table = match value {
                toml::Value::Table(table) if is_known_table => table,
                _ if is_known_table => {
                    keymap.report(table_entry, "expected a table of actions, ignored");
                    continue;
                }
                _ => {
                    keymap.report(
                        table_entry,
                        "not a keybinding table glab-tui knows, ignored",
                    );
                    continue;
                }
            };
            for (field, value) in table {
                let entry = format!("{table_entry}.{field}");
                let Some(action) = Action::ALL.iter().copied().find(|action| {
                    action.spec().context.table() == table_name && action.spec().field == field
                }) else {
                    keymap.report(entry, "not an action glab-tui knows, ignored");
                    continue;
                };
                let keys = match value.try_into::<BindingValue>() {
                    Ok(BindingValue::One(key)) => vec![key],
                    Ok(BindingValue::Many(keys)) => keys,
                    Err(_) => {
                        keymap.report(entry, "expected a key or a list of keys, kept the default");
                        continue;
                    }
                };
                if let Some(unreadable) = keys
                    .iter()
                    .find(|key| !key.is_empty() && !is_readable_binding(key))
                {
                    keymap.report(
                        entry,
                        &format!("{unreadable:?} is not a key glab-tui can read, kept the default"),
                    );
                    continue;
                }
                keymap.bindings[action as usize] = keys;
            }
        }
        keymap
    }
}

impl From<Keymap> for BindingTables {
    fn from(keymap: Keymap) -> Self {
        let mut tables = BindingTables::new();
        for (spec, keys) in ACTIONS.iter().zip(keymap.bindings) {
            let value = match <[String; 1]>::try_from(keys) {
                Ok([key]) => BindingValue::One(key),
                Err(keys) => BindingValue::Many(keys),
            };
            tables
                .entry(spec.context.table().to_string())
                .or_default()
                .insert(spec.field.to_string(), value);
        }
        tables
    }
}

impl Keymap {
    pub fn bindings(&self, action: Action) -> &[String] {
        &self.bindings[action as usize]
    }

    /// The `[keybindings]` entries that bind nothing.
    pub fn problems(&self) -> &[ConfigProblem] {
        &self.problems
    }

    fn report(&mut self, entry: String, problem: &str) {
        self.problems.push(ConfigProblem {
            binding: entry,
            problem: problem.to_string(),
        });
    }

    /// Replaces every key of `action`, without the checks a config entry
    /// goes through.
    #[cfg(test)]
    pub(crate) fn set(&mut self, action: Action, keys: &[&str]) {
        self.bindings[action as usize] = keys.iter().map(|key| key.to_string()).collect();
    }

    pub fn matches(&self, action: Action, press: &KeyPress) -> bool {
        self.bindings(action)
            .iter()
            .any(|binding| keybinding_matches(binding, press))
    }

    /// The first action of `context`, in registry order, that `press`
    /// triggers.
    pub fn resolve(&self, context: KeyContext, press: &KeyPress) -> Option<Action> {
        Action::in_context(context).find(|action| self.matches(*action, press))
    }

    /// The keys of `action` as the help modal shows them, `""` when unbound.
    pub fn display(&self, action: Action) -> String {
        self.bindings(action)
            .iter()
            .map(|key| key.trim())
            .filter(|key| !key.is_empty())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    /// Every bound key of every action.
    pub fn all_bindings(&self) -> impl Iterator<Item = &str> {
        self.bindings.iter().flatten().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybinding::binding_key_event;

    fn press(binding: &str) -> KeyPress {
        KeyPress::Key(binding_key_event(binding).expect("bindable key"))
    }

    fn keymap(toml: &str) -> Keymap {
        toml::from_str(toml).expect("keybinding tables parse")
    }

    #[test]
    fn every_row_sits_at_its_action_index() {
        assert_eq!(Action::ALL.len(), ACTIONS.len());
        for (index, action) in Action::ALL.iter().enumerate() {
            assert_eq!(*action as usize, index, "{action:?}");
        }
    }

    #[test]
    fn config_keys_are_unique() {
        let mut keys: Vec<String> = Action::ALL.iter().map(|a| a.config_key()).collect();
        keys.sort();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "two actions share a config key");
    }

    #[test]
    fn every_default_key_is_bindable() {
        for action in Action::ALL {
            for key in action.spec().defaults {
                assert!(
                    binding_key_event(key).is_some(),
                    "{} default {key:?} cannot be pressed",
                    action.config_key()
                );
            }
        }
    }

    #[test]
    fn no_two_actions_of_one_context_share_a_default_key() {
        for action in Action::ALL {
            for key in action.spec().defaults {
                let event = press(key);
                let first = Keymap::default().resolve(action.context(), &event);
                assert_eq!(
                    first,
                    Some(*action),
                    "{} default {key:?} is taken by {first:?}",
                    action.config_key()
                );
            }
        }
    }

    #[test]
    fn remapping_an_action_drops_its_default_keys() {
        let keymap = keymap("[pipelines]\nretry = \"x\"\n");
        let pipelines = KeyContext::Tab(Tab::Pipelines);

        assert_eq!(
            keymap.resolve(pipelines, &press("x")),
            Some(Action::PipelinesRetry)
        );
        assert_eq!(keymap.resolve(pipelines, &press("r")), None);
    }

    #[test]
    fn a_list_value_binds_every_key_and_replaces_all_defaults() {
        let keymap = keymap("[list]\nnext_row = [\"n\", \"PageDown\"]\n");

        assert!(keymap.matches(Action::ListNextRow, &press("n")));
        assert!(keymap.matches(Action::ListNextRow, &press("PageDown")));
        assert!(!keymap.matches(Action::ListNextRow, &press("j")));
        assert!(!keymap.matches(Action::ListNextRow, &press("Down")));
        assert_eq!(keymap.display(Action::ListNextRow), "n / PageDown");
    }

    #[test]
    fn an_empty_binding_unbinds_the_action() {
        let keymap = keymap("[global]\nhelp = \"\"\n[selector]\nfilter = []\n");

        assert!(!keymap.matches(Action::GlobalHelp, &press("?")));
        assert!(!keymap.matches(Action::GlobalHelp, &press("F1")));
        assert_eq!(keymap.display(Action::GlobalHelp), "");
        assert_eq!(keymap.resolve(KeyContext::Selector, &press("f")), None);
    }

    fn problem(binding: &str, problem: &str) -> ConfigProblem {
        ConfigProblem {
            binding: binding.to_string(),
            problem: problem.to_string(),
        }
    }

    #[test]
    fn unknown_entries_of_any_type_are_reported_and_bind_nothing() {
        let keymap = keymap(
            "quit = \"x\"\n[issues]\ncreate_issue = \"N\"\nnot_an_action = 3\n[nope]\nx = \"y\"\n",
        );

        assert_eq!(keymap.bindings(Action::IssuesCreateIssue), ["N"]);
        assert_eq!(keymap.bindings(Action::IssuesEditEntity), ["e"]);
        assert_eq!(
            keymap.problems(),
            [
                problem(
                    "keybindings.issues.not_an_action",
                    "not an action glab-tui knows, ignored"
                ),
                problem(
                    "keybindings.nope",
                    "not a keybinding table glab-tui knows, ignored"
                ),
                problem(
                    "keybindings.quit",
                    "not a keybinding table glab-tui knows, ignored"
                ),
            ]
        );
    }

    #[test]
    fn a_value_that_is_not_a_key_keeps_the_default_and_is_reported() {
        let keymap = keymap("list = \"x\"\n[global]\nhelp = 3\nquit = [\"q\", 1]\n");

        assert_eq!(keymap.bindings(Action::GlobalHelp), ["?", "F1"]);
        assert_eq!(
            keymap.bindings(Action::GlobalQuit),
            Keymap::default().bindings(Action::GlobalQuit)
        );
        assert_eq!(
            keymap.problems(),
            [
                problem(
                    "keybindings.global.help",
                    "expected a key or a list of keys, kept the default"
                ),
                problem(
                    "keybindings.global.quit",
                    "expected a key or a list of keys, kept the default"
                ),
                problem("keybindings.list", "expected a table of actions, ignored"),
            ]
        );
    }

    #[test]
    fn an_unreadable_key_keeps_the_default_and_is_reported() {
        let keymap = keymap(
            "[dialog]\ncancel = \"esc\"\n[global]\nhelp = \"\"\n[list]\nnext_row = [\"j\", \"down\"]\n",
        );

        assert_eq!(keymap.bindings(Action::DialogCancel), ["Esc"]);
        assert_eq!(keymap.bindings(Action::ListNextRow), ["j", "Down"]);
        assert_eq!(keymap.bindings(Action::GlobalHelp), [""]);
        assert_eq!(
            keymap.problems(),
            [
                problem(
                    "keybindings.dialog.cancel",
                    "\"esc\" is not a key glab-tui can read, kept the default"
                ),
                problem(
                    "keybindings.list.next_row",
                    "\"down\" is not a key glab-tui can read, kept the default"
                ),
            ]
        );
    }

    #[test]
    fn keymap_round_trips_through_its_tables() {
        let mut original = Keymap::default();
        original.set(Action::DiffSearchPrevious, &["Ctrl+b"]);
        original.set(Action::GlobalHelp, &[]);

        let text = toml::to_string(&original).expect("keymap serializes");
        assert_eq!(keymap(&text), original);
    }

    #[test]
    fn labels_use_the_hosts_terms() {
        use crate::backend::BackendKind;
        let spec = Action::IssuesCreateMr.spec();
        assert_eq!(
            spec.label_for(BackendKind::GitHub),
            "Create Pull Request from selected Issue"
        );
        assert_eq!(
            Action::MrsCloseEntity.spec().label_for(BackendKind::GitLab),
            "Close selected MR"
        );
        assert_eq!(
            Action::MrsViewRelatedPipelines
                .spec()
                .label_for(BackendKind::GitHub),
            "View related Actions for selected PR"
        );
        let cycle = Action::RunnersCycleStatus.spec();
        assert_eq!(
            cycle.label_for(BackendKind::GitLab),
            "Cycle the status filter: Online, Paused, Offline, All"
        );
        assert_eq!(
            cycle.label_for(BackendKind::GitHub),
            "Cycle the status filter: Online, Offline, All"
        );
    }
}
