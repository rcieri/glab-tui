use crate::AppTerminal;
use crate::app::{App, Tab};
use crate::event::{Event, ScopedSender};
use crate::fetch::spawn_refresh_active_tab;
use crate::git_helpers::{get_default_branch, slugify};
use crate::keybinding::keybinding_matches;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::ListState;
use tokio::sync::mpsc::UnboundedSender;

/// Fetches the MR/PR diff and opens the diff view when it is built. A second
/// request while one is in flight is dropped: it would repeat every CLI call
/// and build the view twice.
fn spawn_open_diff(
    app: &mut App,
    tx: &UnboundedSender<Event>,
    mr_iid: u64,
    project_context: String,
) {
    if app.diff_loading {
        return;
    }
    let Some(client) = app.gitlab_client.clone() else {
        app.show_error("No backend client available to fetch diff".to_string());
        return;
    };
    app.diff_loading = true;
    let tx = app.scoped_sender(tx.clone());
    tokio::spawn(async move {
        let event = match crate::fetch::fetch_diff_view(&client, &project_context, mr_iid).await {
            Ok((diff_view, comments)) => Event::DiffFetched {
                diff_view,
                comments,
            },
            Err(err) => Event::DiffFetchFailed(format!("Failed to fetch diff: {err}")),
        };
        let _ = tx.send(event);
    });
}

fn show_selected_all_notice(app: &mut App, added: usize) {
    if added == 0 {
        return;
    }
    let plural = if added == 1 { "" } else { "s" };
    app.show_notice(format!("Selected all {added} item{plural}"));
}

/// A row whose status was set before the backend confirmed the mutation:
/// its id and the status it showed before, when the row was on screen.
pub(crate) type OptimisticRow = (u64, Option<String>);

/// The table an optimistic status flip was applied to.
#[derive(Clone, Copy)]
pub(crate) enum OptimisticTable {
    Pipelines,
    Jobs,
}

impl OptimisticTable {
    fn tab(self) -> Tab {
        match self {
            OptimisticTable::Pipelines => Tab::Pipelines,
            OptimisticTable::Jobs => Tab::Jobs,
        }
    }

    fn revert(self, id: u64, status: String) -> Event {
        match self {
            OptimisticTable::Pipelines => Event::PipelineStatusRevert {
                pipeline_id: id,
                status,
            },
            OptimisticTable::Jobs => Event::JobStatusRevert { job_id: id, status },
        }
    }
}

pub(crate) fn flip_pipeline_statuses(
    app: &mut App,
    ids: &[u64],
    status: &str,
) -> Vec<OptimisticRow> {
    ids.iter()
        .map(|&id| {
            let previous = app
                .pipelines
                .items
                .iter_mut()
                .find(|pipeline| pipeline.id() == id && pipeline.status != status)
                .map(|pipeline| std::mem::replace(&mut pipeline.status, status.to_string()));
            (id, previous)
        })
        .collect()
}

pub(crate) fn flip_job_statuses(app: &mut App, ids: &[u64], status: &str) -> Vec<OptimisticRow> {
    ids.iter()
        .map(|&id| {
            let previous = app
                .jobs
                .items
                .iter_mut()
                .find(|job| job.id() == id && job.status != status)
                .map(|job| std::mem::replace(&mut job.status, status.to_string()));
            (id, previous)
        })
        .collect()
}

/// Runs `mutate` for every row, pacing bulk runs. A failed row gets its prior
/// status back, and all failures surface as a single error toast.
pub(crate) async fn run_optimistic_mutation<F, Fut>(
    table: OptimisticTable,
    action: &str,
    rows: Vec<OptimisticRow>,
    tx: &ScopedSender,
    mutate: F,
) where
    F: Fn(u64) -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let mut failures = Vec::new();
    for (i, (id, previous)) in rows.into_iter().enumerate() {
        if i > 0 {
            crate::backend::rate_limit::pace_bulk_operation().await;
        }
        if let Err(e) = mutate(id).await {
            if let Some(status) = previous {
                let _ = tx.send(table.revert(id, status));
            }
            failures.push(format!("#{id}: {e}"));
        }
    }
    if let Some(message) = mutation_failure_message(action, &failures) {
        let _ = tx.send(Event::CommandCompleted(table.tab(), Err(message)));
    }
}

/// Cancels `rows`' jobs, then refreshes the pipeline's job list.
pub(crate) fn spawn_cancel_jobs(
    client: crate::domain::client::GitlabClient,
    tx: ScopedSender,
    project_path: String,
    pipeline_id: u64,
    rows: Vec<OptimisticRow>,
) {
    tokio::spawn(async move {
        run_optimistic_mutation(OptimisticTable::Jobs, "cancel job", rows, &tx, |id| {
            client.cancel_job(&project_path, id)
        })
        .await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if let Ok(jobs) =
            crate::domain::pipelines::list_pipeline_jobs(&client, &project_path, pipeline_id).await
        {
            let _ = tx.send(Event::PipelineJobs(pipeline_id, jobs));
        }
    });
}

fn mutation_failure_message(action: &str, failures: &[String]) -> Option<String> {
    let (first, rest) = failures.split_first()?;
    Some(match rest.len() {
        0 => format!("Failed to {action} {first}"),
        more => format!("Failed to {action} {first} (+{more} more failed)"),
    })
}

/// Returns whether a fetch started: there is none without a client.
pub(crate) fn spawn_job_trace_fetch(
    app: &mut App,
    tx: &UnboundedSender<Event>,
    job_id: u64,
) -> bool {
    let Some(client) = app.gitlab_client.clone() else {
        return false;
    };
    let project_context = app.project_path_for_jobs();
    let tx = app.scoped_sender(tx.clone());
    app.job_trace_loading = true;
    tokio::spawn(async move {
        let res = crate::domain::pipelines::get_job_trace(&client, &project_context, job_id).await;
        let _ = tx.send(Event::JobTraceFetched(
            job_id,
            res.map_err(|e| e.to_string()),
        ));
    });
    true
}

pub fn open_job_trace_in_editor(app: &mut App, job_id: u64, terminal: &mut AppTerminal) {
    let Some(view) = &app.job_trace else {
        return;
    };
    let trace = view.trace().text();
    let prefix = format!("job_{job_id}_trace_");
    if let Err(error) = crate::editor::view_in_editor(trace, &prefix, terminal) {
        app.show_error(format!("Failed to open job trace in editor: {error}"));
    }
}

/// Lists `environment`'s deployments into the detail pane.
fn spawn_fetch_deployments(app: &App, tx: &UnboundedSender<Event>, environment: String) {
    let Some(client) = app.gitlab_client.clone() else {
        return;
    };
    let _ = tx.send(Event::CommandStarted(format!(
        "Fetching deployments for {environment}"
    )));
    let scope = app.scope.clone();
    let tx = app.scoped_sender(tx.clone());
    tokio::spawn(async move {
        match crate::domain::deployments::list_deployments(&client, &scope, Some(&environment))
            .await
        {
            Ok(deployments) => {
                let _ = tx.send(Event::DeploymentsFetched {
                    environment,
                    deployments,
                });
            }
            Err(e) => {
                let _ = tx.send(Event::CommandCompleted(
                    crate::app::Tab::Environments,
                    Err(format!("Failed to fetch deployments: {e}")),
                ));
            }
        }
    });
}

/// Navigation keys while the deployments pane has focus. Returns whether
/// the key was taken.
fn handle_focused_deployments_key(app: &mut App, key_event: &KeyEvent) -> bool {
    let Some(shown) = app.focused_deployments_mut() else {
        return false;
    };
    match key_event.code {
        KeyCode::Down | KeyCode::Char('j') => shown.table.next_item(),
        KeyCode::Up | KeyCode::Char('k') => shown.table.previous_item(),
        KeyCode::Home => shown.table.first_item(),
        KeyCode::End => shown.table.last_item(),
        KeyCode::Esc | KeyCode::Backspace => shown.is_focused = false,
        _ => return false,
    }
    app.detail_scroll = 0;
    true
}

const NO_DEPLOYMENT_SELECTED: &str =
    "No deployment selected: list the environment's deployments first";

fn handle_environments_key(
    app: &mut App,
    key_event: &KeyEvent,
    tx: &UnboundedSender<Event>,
) -> bool {
    if handle_focused_deployments_key(app, key_event) {
        return true;
    }
    let Some(environment) = app.environments.cursor_row().cloned() else {
        return false;
    };
    let keys = &app.config.keybindings.environments;
    if keybinding_matches(&keys.open_in_browser, key_event) {
        let project_path = app.scope.as_str().to_string();
        let client = app.gitlab_client.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let Some(client) = client else {
                return;
            };
            let result = client
                .open_environment_in_browser(&project_path, &environment.name)
                .await;
            let _ = tx.send(Event::CommandCompleted(
                crate::app::Tab::Environments,
                result.map_err(|e| e.to_string()),
            ));
        });
    } else if keybinding_matches(&keys.view_deployments, key_event) {
        app.detail_visible = true;
        if let Some(shown) = app.environment_deployments.as_mut() {
            shown.is_focused = !shown.table.items.is_empty();
        }
        spawn_fetch_deployments(app, tx, environment.name);
    } else if app.is_deployments_focus_key(key_event) {
        if let Some(shown) = app.environment_deployments.as_mut() {
            shown.is_focused = !shown.is_focused;
        }
    } else if keybinding_matches(&keys.stop_environment, key_event) {
        app.submit_dialog = Some(crate::app::SubmitDialog::build(
            crate::app::ConfirmAction::StopEnvironment(Box::new(environment)),
            app,
        ));
    } else if keybinding_matches(&keys.rollback_deployment, key_event) {
        match app.selected_deployment().cloned() {
            Some(deployment) if !app.is_github() && deployment.job_id().is_none() => {
                let iid = deployment.iid;
                app.show_error(format!("Deployment #{iid} has no deploy job to re-run"));
            }
            Some(deployment) => {
                app.submit_dialog = Some(crate::app::SubmitDialog::build(
                    crate::app::ConfirmAction::RollbackDeployment {
                        environment: environment.name,
                        deployment: Box::new(deployment),
                    },
                    app,
                ));
            }
            None => app.show_error(NO_DEPLOYMENT_SELECTED.to_string()),
        }
    } else if keybinding_matches(&keys.copy_url, key_event) {
        if let Err(error) = app.copy_selected_environment_url() {
            app.show_error(format!("Failed to copy environment URL: {error}"));
        }
    } else if keybinding_matches(&keys.view_deployment_log, key_event) {
        spawn_deployment_log_fetch(app, tx);
    } else {
        return false;
    }
    true
}

/// Fetches the log of the selected deployment's job for the editor.
fn spawn_deployment_log_fetch(app: &mut App, tx: &UnboundedSender<Event>) {
    let Some(deployment) = app.selected_deployment() else {
        app.show_error(NO_DEPLOYMENT_SELECTED.to_string());
        return;
    };
    let deployment_id = deployment.id;
    let Some(job_id) = deployment.job_id() else {
        let iid = deployment.iid;
        app.show_error(format!("Deployment #{iid} has no job log"));
        return;
    };
    let Some(client) = app.gitlab_client.clone() else {
        return;
    };
    let project = app.scope.as_str().to_string();
    if app.pending_deployment_log.replace(deployment_id) == Some(deployment_id) {
        return;
    }
    let tx = app.scoped_sender(tx.clone());
    tokio::spawn(async move {
        let result = crate::domain::pipelines::get_job_trace(&client, &project, job_id).await;
        let _ = tx.send(Event::DeploymentLogFetched {
            deployment_id,
            result: result.map_err(|e| e.to_string()),
        });
    });
}
/// Record a request to fetch related MRs/PRs for the currently selected issue.
///
/// This is intentionally *cheap*: it just stores the iid in
/// `App::pending_related_mrs_iid` and a timestamp in
/// `App::pending_related_mrs_since`. The actual `spawn_fetch_related_mrs` is
/// deferred to `dispatch_pending_related_mrs_fetch`, which only fires once
/// the debounce window has elapsed — so holding `j`/`k` through the issue
/// list queues one request per scroll-stop, not one per issue scrolled past.
///
/// The `tx` parameter is accepted (not consumed) so existing keypress handler
/// call sites keep passing their sender without churn; the dispatcher is what
/// ultimately drives `spawn_fetch_related_mrs`.
pub fn maybe_fetch_related_mrs(app: &mut App, _tx: &UnboundedSender<Event>) {
    let Some(iid) = app
        .issues
        .state
        .selected()
        .and_then(|idx| app.filtered_issues().get(idx).map(|i| i.iid))
    else {
        return;
    };
    if app
        .issues
        .items
        .iter()
        .any(|i| i.iid == iid && i.related_mrs.is_some())
    {
        if app.pending_related_mrs_iid == Some(iid) {
            app.pending_related_mrs_iid = None;
            app.pending_related_mrs_since = None;
        }
        return;
    }
    app.pending_related_mrs_iid = Some(iid);
    app.pending_related_mrs_since = Some(std::time::Instant::now());
}

/// Record a request to fetch linked issues for the currently selected MR/PR.
pub fn maybe_fetch_mr_related_issues(app: &mut App, _tx: &UnboundedSender<Event>) {
    let Some(iid) = app
        .mrs
        .state
        .selected()
        .and_then(|idx| app.filtered_mrs().get(idx).map(|m| m.iid))
    else {
        return;
    };
    if app
        .mrs
        .items
        .iter()
        .any(|m| m.iid == iid && m.related_issues.is_some())
    {
        if app.pending_mr_related_issues_iid == Some(iid) {
            app.pending_mr_related_issues_iid = None;
            app.pending_mr_related_issues_since = None;
        }
        return;
    }
    app.pending_mr_related_issues_iid = Some(iid);
    app.pending_mr_related_issues_since = Some(std::time::Instant::now());
}

/// Run `action` on the PR once the session knows its stack. When it does not
/// yet, the stack is fetched first and the action runs once it lands (see
/// `Event::PrStackFetched`).
fn with_pr_stack(
    app: &mut App,
    mr: &crate::domain::mr::MergeRequest,
    action: crate::app::StackAction,
    tx: &UnboundedSender<Event>,
) {
    use crate::domain::mr::StackLookup;
    let pr = app.mr_ref(mr);
    let is_known = matches!(
        app.pr_stack(&pr.0, pr.1),
        Some(StackLookup::NotStacked | StackLookup::Full(_))
    );
    // Stacks exist only in GitHub repositories; anywhere else the answer is
    // already known to be "not stacked".
    let can_be_stacked = app.is_github() && app.scope.is_repository();
    if !is_known && can_be_stacked {
        if let Some(client) = app.gitlab_client.clone() {
            crate::fetch::request_pr_stack(&client, app, pr.clone(), tx);
            app.show_notice(format!("Loading the stack of PR #{}…", pr.1));
            app.pending_stack_action = Some((pr, action));
            return;
        }
    }
    run_stack_action(app, &pr.0, pr.1, action);
}

/// Run a PR action whose stack the session already knows.
pub fn run_stack_action(
    app: &mut App,
    project: &str,
    pr_number: u64,
    action: crate::app::StackAction,
) {
    match action {
        crate::app::StackAction::Browse => open_stack_selector(app, project, pr_number),
        crate::app::StackAction::Merge => open_merge_dialog(app, project, pr_number),
    }
}

/// A stacked PR merges with every open PR below it through GitHub's stack
/// merge; any other MR/PR gets the plain merge dialog.
fn open_merge_dialog(app: &mut App, project: &str, pr_number: u64) {
    let action = match app.pr_stack(project, pr_number) {
        Some(crate::domain::mr::StackLookup::Full(_)) => {
            crate::app::ConfirmAction::MergeStack(pr_number)
        }
        _ => crate::app::ConfirmAction::MergeMr(pr_number),
    };
    app.submit_dialog = Some(crate::app::SubmitDialog::build_with_project(
        action,
        project.to_string(),
        app,
    ));
}

/// Open the selector over a PR's stack the session already knows, so the
/// user can jump to another PR in it, or say why there is nothing to open.
fn open_stack_selector(app: &mut App, project: &str, pr_number: u64) {
    use crate::domain::mr::StackLookup;
    let Some(StackLookup::Full(stack)) = app.pr_stack(project, pr_number) else {
        app.show_error("This PR is not part of a stack.".to_string());
        return;
    };
    if stack.entries.is_empty() {
        app.show_error("This PR has no stack entries to navigate.".to_string());
        return;
    }
    let current_marker = format!(
        " {} (current)",
        crate::config::ICONS.read().unwrap().nav_prev
    );
    let selector = crate::app::Selector {
        title: format!(
            " Stack #{} — {} PRs ",
            stack.info.number,
            stack.entries.len(),
        ),
        all_items: stack
            .entries
            .iter()
            .map(|e| {
                format!(
                    "#{}. #{}: {} ({}){}{}",
                    e.position,
                    e.number,
                    e.title,
                    e.state.to_uppercase(),
                    if e.is_draft { " [draft]" } else { "" },
                    if e.number == pr_number {
                        current_marker.as_str()
                    } else {
                        ""
                    },
                )
            })
            .collect(),
        selected_items: std::collections::HashSet::new(),
        cursor_idx: stack
            .entries
            .iter()
            .position(|e| e.number == pr_number)
            .unwrap_or(0),
        search_query: String::new(),
        is_filtering: false,
        is_loading: false,
        entity_iid: pr_number,
        entity_type: "mr".to_string(),
        field_type: "stack_entries".to_string(),
        multi_select: false,
        state: {
            let mut s = ListState::default();
            s.select(Some(0));
            s
        },
    };
    app.selector = Some(selector);
}

/// Scrolls the detail pane by a full or half page for the
/// `scroll_page_down`/`scroll_page_up`/`scroll_half_page_down`/
/// `scroll_half_page_up` bindings, against the usable height of
/// `detail_rect` (its height minus the two border rows), or of the trace pane
/// below the job header while the Jobs tab shows a log. `pending` is the
/// character captured on a previous keystroke, if any (see
/// `matches_with_pending`); pass `None` when there is no pending sequence.
/// Returns true if a page scroll action matched and was applied.
fn apply_page_scroll(app: &mut App, pending: Option<char>, key_event: &KeyEvent) -> bool {
    if let Some(rect) = app.detail_rect.filter(|_| app.detail_visible) {
        let trace_rows = app
            .job_trace
            .as_ref()
            .filter(|_| app.active_tab == crate::app::Tab::Jobs)
            .map(|view| view.viewport_height())
            .filter(|&rows| rows > 0);
        let full_page = trace_rows
            .map_or_else(
                || rect.height.saturating_sub(2),
                |rows| u16::try_from(rows).unwrap_or(u16::MAX),
            )
            .max(1);
        let half_page = full_page / 2;
        if crate::keybinding::matches_with_pending(
            &app.config.keybindings.global.scroll_page_down,
            pending,
            key_event,
        ) {
            app.detail_scroll = app.detail_scroll.saturating_add(full_page);
            return true;
        } else if crate::keybinding::matches_with_pending(
            &app.config.keybindings.global.scroll_page_up,
            pending,
            key_event,
        ) {
            app.detail_scroll = app.detail_scroll.saturating_sub(full_page);
            return true;
        } else if crate::keybinding::matches_with_pending(
            &app.config.keybindings.global.scroll_half_page_down,
            pending,
            key_event,
        ) {
            app.detail_scroll = app.detail_scroll.saturating_add(half_page);
            return true;
        } else if crate::keybinding::matches_with_pending(
            &app.config.keybindings.global.scroll_half_page_up,
            pending,
            key_event,
        ) {
            app.detail_scroll = app.detail_scroll.saturating_sub(half_page);
            return true;
        }
    }
    false
}

/// Moves the list selection by half a page for the half-page bindings while
/// the detail pane is hidden. A visible pane takes these keys first (see
/// `apply_page_scroll`). Returns true if a binding matched.
fn apply_list_half_page(app: &mut App, key_event: &KeyEvent, tx: &UnboundedSender<Event>) -> bool {
    if app.detail_visible || app.details_zoomed {
        return false;
    }
    let down = if keybinding_matches(
        &app.config.keybindings.global.scroll_half_page_down,
        key_event,
    ) {
        true
    } else if keybinding_matches(
        &app.config.keybindings.global.scroll_half_page_up,
        key_event,
    ) {
        false
    } else {
        return false;
    };
    let Some(rect) = app.content_rect else {
        return false;
    };
    // The table loses four rows to its borders, header and header gap.
    let rows = (rect.height.saturating_sub(4) / 2).max(1) as usize;
    if let Some(list) = app.list_navigation_mut(app.active_tab) {
        list.move_by(down, rows);
    } else {
        match app.active_tab {
            crate::app::Tab::Issues => app.issues.move_by(app.filtered_issues().len(), down, rows),
            crate::app::Tab::MergeRequests => app.mrs.move_by(app.filtered_mrs().len(), down, rows),
            crate::app::Tab::Pipelines => {
                app.pipelines
                    .move_by(app.filtered_pipelines().len(), down, rows)
            }
            crate::app::Tab::Jobs => {
                let len = app.filtered_jobs().len();
                app.jobs.move_by(len, down, rows);
                app.job_trace = None;
                app.job_trace_follow = false;
            }
            crate::app::Tab::Todos => app.todos.move_by(app.filtered_todos().len(), down, rows),
            crate::app::Tab::Milestones => {
                app.milestones
                    .move_by(app.filtered_milestones().len(), down, rows)
            }
            // The terminal log counts its scroll up from the newest line.
            crate::app::Tab::Terminal => {
                app.terminal_scroll = if down {
                    app.terminal_scroll.saturating_sub(rows)
                } else {
                    app.terminal_scroll.saturating_add(rows)
                };
            }
            _ => {}
        }
    }
    app.detail_scroll = 0;
    if app.select_mode {
        app.update_visual_selection();
    }
    if app.active_tab == crate::app::Tab::Issues {
        maybe_fetch_related_mrs(app, tx);
    }
    if app.active_tab == crate::app::Tab::MergeRequests {
        maybe_fetch_mr_related_issues(app, tx);
    }
    true
}

pub async fn handle_active_tab_key(
    app: &mut App,
    key_event: &KeyEvent,
    terminal: &mut AppTerminal,
    tx: UnboundedSender<Event>,
    pending: Option<char>,
) -> bool {
    if pending.is_some() {
        // Resolving the second key of a pending sequence (e.g. the second
        // `g` of `gg`). A key that doesn't complete a known sequence lapses
        // instead of falling through to normal dispatch — vim discards `g`
        // + an unbound key the same way (glt-0009 plan, Entscheidung 5).
        if app.detail_visible
            && crate::keybinding::matches_with_pending(
                &app.config.keybindings.global.scroll_top,
                pending,
                key_event,
            )
        {
            app.detail_scroll = 0;
            return true;
        } else if apply_page_scroll(app, pending, key_event) {
            return true;
        }
        return false;
    }

    let mut handled = true;
    match app.active_tab {
        crate::app::Tab::Issues => match key_event.code {
            _ if keybinding_matches(&app.config.keybindings.issues.create_issue, key_event) => {
                let is_github = app.is_github();
                let project = if app.scope.is_group() {
                    let issues = app.filtered_issues();
                    app.issues
                        .state
                        .selected()
                        .and_then(|idx| issues.get(idx))
                        .map(|i| {
                            if !i.project_path.is_empty() {
                                i.project_path.clone()
                            } else {
                                crate::git_helpers::parse_project_path_from_web_url(&i.web_url)
                                    .unwrap_or_default()
                            }
                        })
                        .filter(|p| !p.is_empty())
                        .unwrap_or_default()
                } else {
                    app.scope.as_str().to_string()
                };
                let fields = crate::entity_editor::issue_fields(
                    crate::entity_editor::IssueFieldValues {
                        confidential: "No".to_string(),
                        weight: "0".to_string(),
                        ..Default::default()
                    },
                    is_github,
                    if app.scope.is_group() {
                        Some(project.clone())
                    } else {
                        None
                    },
                );
                app.open_edit_menu(crate::app::EditMenu {
                    title: "Create Issue".to_string(),
                    entity_project: project,
                    fields,
                    initial_fields: std::collections::HashMap::new(),
                    selected_idx: 0,
                    entity_iid: 0,
                    entity_kind: crate::app::EditEntityKind::CreateIssue,
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                    workflow_inputs: vec![],
                    cursor_pos: 0,
                    editing: false,
                    desc_scroll: 0,
                });
            }
            _ if keybinding_matches(&app.config.keybindings.issues.edit_entity, key_event) => {
                if app.selected_issues.len() > 1 {
                    let count = app.selected_issues.len();
                    app.open_edit_menu(crate::app::EditMenu {
                        title: format!("Bulk Edit {count} Issues"),
                        entity_project: app.scope.as_str().to_string(),
                        fields: vec![
                            crate::app::Field::multi_select("Assignees", String::new()),
                            crate::app::Field::multi_select("Milestone", String::new()),
                            crate::app::Field::multi_select("Labels", String::new()),
                        ],
                        // Bulk-edit forms start empty; `open_edit_menu` will snapshot all
                        // blank values as the baseline. The `IssueUpdate::is_empty()` guard
                        // in the dispatcher handles the true no-op case (nothing filled in).
                        initial_fields: std::collections::HashMap::new(),
                        selected_idx: 0,
                        entity_iid: 0,
                        entity_kind: crate::app::EditEntityKind::BulkEditIssues,
                        state: {
                            let mut s = ListState::default();
                            s.select(Some(0));
                            s
                        },
                        workflow_inputs: vec![],
                        cursor_pos: 0,
                        editing: false,
                        desc_scroll: 0,
                    });
                } else if let Some(selected_idx) = app.issues.state.selected() {
                    let filtered = app.filtered_issues();
                    if let Some(issue) = filtered.get(selected_idx) {
                        let is_github = app.is_github();
                        let mut doc = crate::entity_editor::build_issue_document(
                            issue,
                            is_github,
                            app.fetching_related_mrs.contains(&issue.iid),
                        );
                        doc.fields.push(crate::app::Field::text(
                            "Description",
                            issue.description.clone().unwrap_or_default(),
                        ));
                        app.open_edit_menu(crate::app::EditMenu {
                            title: format!("Edit Issue #{}", issue.iid),
                            entity_project: issue.project_path.clone(),
                            fields: doc.fields,
                            initial_fields: std::collections::HashMap::new(),
                            selected_idx: 0,
                            entity_iid: issue.iid,
                            entity_kind: crate::app::EditEntityKind::EditIssue,
                            state: {
                                let mut s = ListState::default();
                                s.select(Some(0));
                                s
                            },
                            workflow_inputs: vec![],
                            cursor_pos: 0,
                            editing: false,
                            desc_scroll: 0,
                        });
                    }
                }
            }
            _ if (key_event.code == KeyCode::Char('M')
                || keybinding_matches(
                    &app.config.keybindings.issues.jump_related_mrs,
                    key_event,
                )) =>
            {
                let Some(issue_iid) = app
                    .issues
                    .state
                    .selected()
                    .and_then(|idx| app.filtered_issues().get(idx).map(|i| i.iid))
                else {
                    return false;
                };
                use crate::domain::issues::RelatedMrsState;
                let state = app
                    .issues
                    .items
                    .iter()
                    .find(|i| i.iid == issue_iid)
                    .and_then(|i| i.related_mrs.as_ref());
                match state {
                    None if app.fetching_related_mrs.contains(&issue_iid) => {
                        app.show_error("Related Merge Requests still loading…".to_string());
                    }
                    None => {
                        app.show_error(
                            "No related Merge Requests cached yet — open the issue once and retry."
                                .to_string(),
                        );
                    }
                    Some(RelatedMrsState::Empty) => {
                        app.show_error(
                            "This issue has no related Merge Requests / Pull Requests.".to_string(),
                        );
                    }
                    Some(RelatedMrsState::Failed(msg)) => {
                        app.show_error(format!("Failed to fetch related Merge Requests: {msg}"));
                    }
                    Some(RelatedMrsState::Items(items)) => {
                        let is_github = app.is_github();
                        if items.len() == 1 {
                            let client = app.gitlab_client.clone();
                            jump_to_mr_tab(app, items[0].iid, client, tx.clone());
                        } else {
                            app.selector = Some(crate::app::Selector {
                                title: format!(
                                    " Related {} for Issue #{} ",
                                    if is_github {
                                        "Pull Requests"
                                    } else {
                                        "Merge Requests"
                                    },
                                    issue_iid
                                ),
                                all_items: items
                                    .iter()
                                    .map(|r| {
                                        format!(
                                            "!{} [{}] {}",
                                            r.iid,
                                            match r.state.as_str() {
                                                "opened" | "open" => "OPEN",
                                                "closed" | "close" => "CLOSED",
                                                "merged" => "MERGED",
                                                other => other,
                                            },
                                            r.title
                                        )
                                    })
                                    .collect(),
                                selected_items: std::collections::HashSet::new(),
                                cursor_idx: 0,
                                search_query: String::new(),
                                is_filtering: false,
                                is_loading: false,
                                entity_iid: issue_iid,
                                entity_type: String::new(),
                                field_type: "related_mrs".to_string(),
                                multi_select: false,
                                state: {
                                    let mut s = ListState::default();
                                    s.select(Some(0));
                                    s
                                },
                            });
                        }
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.close_entity, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    let filtered = app.filtered_issues();
                    if let Some(issue) = filtered.get(selected_idx) {
                        let issue_iid = issue.iid;
                        let project_path = app.scope.project_of(&issue.project_path).to_string();
                        app.submit_dialog = Some(crate::app::SubmitDialog::build_with_project(
                            crate::app::ConfirmAction::CloseIssue(issue_iid),
                            project_path,
                            app,
                        ));
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.delete_entity, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    let filtered = app.filtered_issues();
                    if let Some(issue) = filtered.get(selected_idx) {
                        let issue_iid = issue.iid;
                        let project_path = app.scope.project_of(&issue.project_path).to_string();
                        app.submit_dialog = Some(crate::app::SubmitDialog::build_with_project(
                            crate::app::ConfirmAction::DeleteIssue(issue_iid),
                            project_path,
                            app,
                        ));
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.copy_reference, key_event) => {
                if let Err(error) = app.copy_selected_issue_reference() {
                    app.show_error(format!("Failed to copy issue reference: {error}"));
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.open_in_browser, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    if let Some(issue) = app.filtered_issues().get(selected_idx) {
                        let Some(client) = app.gitlab_client.clone() else {
                            return false;
                        };
                        let project_path = if !issue.project_path.is_empty() {
                            issue.project_path.clone()
                        } else {
                            app.scope.as_str().to_string()
                        };
                        let iid_str = issue.iid.to_string();
                        let tx2 = tx.clone();
                        tokio::spawn(async move {
                            let result = client
                                .open_in_browser(&project_path, "issue", &iid_str)
                                .await;
                            let _ = tx2.send(Event::CommandCompleted(
                                crate::app::Tab::Issues,
                                result.map_err(|e| e.to_string()),
                            ));
                        });
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.reopen_entity, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    let filtered = app.filtered_issues();
                    if let Some(issue) = filtered.get(selected_idx) {
                        let issue_iid = issue.iid;
                        let project_path = app.scope.project_of(&issue.project_path).to_string();
                        app.submit_dialog = Some(crate::app::SubmitDialog::build_with_project(
                            crate::app::ConfirmAction::ReopenIssue(issue_iid),
                            project_path,
                            app,
                        ));
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.select_issue, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    if let Some(i) = app.filtered_issues().get(selected_idx) {
                        let key = (i.project_path.clone(), i.iid);
                        if app.selected_issues.contains(&key) {
                            app.selected_issues.remove(&key);
                        } else {
                            app.selected_issues.insert(key);
                        }
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.drill_into_scope, key_event) => {
                if app.scope.is_group() {
                    if let Some(idx) = app.issues.state.selected() {
                        let filtered = app.filtered_issues();
                        if let Some(issue) = filtered.get(idx) {
                            if !issue.project_path.is_empty() {
                                app.drill_into(issue.project_path.clone());
                            }
                        }
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.selection_toggle, key_event) => {
                app.toggle_select_mode();
            }
            _ if keybinding_matches(&app.config.keybindings.issues.select_all, key_event) => {
                let added = app.select_all_filtered();
                show_selected_all_notice(app, added);
            }
            _ if keybinding_matches(&app.config.keybindings.issues.create_mr, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    let filtered = app.filtered_issues();
                    if let Some(issue) = filtered.get(selected_idx) {
                        let is_github = app.is_github();
                        let pr_suffix = if is_github {
                            "Pull Request"
                        } else {
                            "Merge Request"
                        };

                        let title_val = issue.title.clone();
                        let source_branch_val = format!("{}-{}", issue.iid, slugify(&issue.title));
                        let create_from_val = format!("#{} - {}", issue.iid, issue.title);
                        let labels_val = if issue.labels.is_empty() {
                            String::new()
                        } else {
                            issue.labels.join(", ")
                        };
                        let assignees_val = if issue.assignees.is_empty() {
                            String::new()
                        } else {
                            issue
                                .assignees
                                .iter()
                                .map(|a| format!("@{}", a.username))
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        let milestone_val = issue
                            .milestone
                            .as_ref()
                            .map(|m| m.title.clone())
                            .unwrap_or_default();
                        let target_branch_val =
                            get_default_branch().unwrap_or_else(|| "main".to_string());
                        let project_val = if !issue.project_path.is_empty() {
                            issue.project_path.clone()
                        } else {
                            app.scope.as_str().to_string()
                        };
                        let mut fields = crate::entity_editor::mr_fields(
                            crate::entity_editor::MrFieldValues {
                                title: title_val,
                                labels: labels_val,
                                assignees: assignees_val,
                                milestone: milestone_val,
                                target_branch: target_branch_val,
                                draft_status: "Draft".to_string(),
                                ..Default::default()
                            },
                            is_github,
                            if app.scope.is_group() {
                                Some(project_val.clone())
                            } else {
                                None
                            },
                        );
                        // Pre-fill the "Create from Issue" and "Source Branch" rows since we launched
                        // the form directly from this issue.
                        if let Some(f) = fields.iter_mut().find(|f| f.label == "Create from Issue")
                        {
                            f.value = create_from_val;
                        }
                        if let Some(f) = fields.iter_mut().find(|f| f.label == "Source Branch") {
                            f.value = source_branch_val;
                        }
                        app.open_edit_menu(crate::app::EditMenu {
                            title: format!("Create {} from #{}", pr_suffix, issue.iid),
                            entity_project: project_val,
                            fields,
                            initial_fields: std::collections::HashMap::new(),
                            selected_idx: 0,
                            entity_iid: issue.iid,
                            entity_kind: crate::app::EditEntityKind::CreateMr,
                            state: {
                                let mut s = ListState::default();
                                s.select(Some(0));
                                s
                            },
                            workflow_inputs: vec![],
                            cursor_pos: 0,
                            editing: false,
                            desc_scroll: 0,
                        });
                    }
                }
            }
            _ => handled = false,
        },
        crate::app::Tab::MergeRequests => {
            if keybinding_matches(&app.config.keybindings.mrs.create_mr, key_event) {
                let is_github = app.is_github();
                let pr_suffix = if is_github {
                    "Pull Request"
                } else {
                    "Merge Request"
                };
                let target_branch_val = get_default_branch().unwrap_or_else(|| "main".to_string());
                let project = if app.scope.is_group() {
                    let mrs = app.filtered_mrs();
                    app.mrs
                        .state
                        .selected()
                        .and_then(|idx| mrs.get(idx))
                        .map(|m| {
                            if !m.project_path.is_empty() {
                                m.project_path.clone()
                            } else {
                                m.web_url
                                    .as_deref()
                                    .and_then(crate::git_helpers::parse_project_path_from_web_url)
                                    .unwrap_or_default()
                            }
                        })
                        .filter(|p| !p.is_empty())
                        .unwrap_or_default()
                } else {
                    app.scope.as_str().to_string()
                };
                let fields = crate::entity_editor::mr_fields(
                    crate::entity_editor::MrFieldValues {
                        target_branch: target_branch_val,
                        draft_status: "Draft".to_string(),
                        ..Default::default()
                    },
                    is_github,
                    if app.scope.is_group() {
                        Some(project.clone())
                    } else {
                        None
                    },
                );
                app.open_edit_menu(crate::app::EditMenu {
                    title: format!("Create {pr_suffix}"),
                    entity_project: project,
                    fields,
                    initial_fields: std::collections::HashMap::new(),
                    selected_idx: 0,
                    entity_iid: 0,
                    entity_kind: crate::app::EditEntityKind::CreateMr,
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                    workflow_inputs: vec![],
                    cursor_pos: 0,
                    editing: false,
                    desc_scroll: 0,
                });
            } else if keybinding_matches(&app.config.keybindings.mrs.select_mr, key_event) {
                if let Some(selected_idx) = app.mrs.state.selected() {
                    if let Some(m) = app.filtered_mrs().get(selected_idx) {
                        let key = (m.project_path.clone(), m.iid);
                        if app.selected_mrs.contains(&key) {
                            app.selected_mrs.remove(&key);
                        } else {
                            app.selected_mrs.insert(key);
                        }
                    }
                }
            } else if keybinding_matches(&app.config.keybindings.mrs.drill_into_scope, key_event) {
                if app.scope.is_group() {
                    if let Some(idx) = app.mrs.state.selected() {
                        let filtered = app.filtered_mrs();
                        if let Some(mr) = filtered.get(idx) {
                            if !mr.project_path.is_empty() {
                                app.drill_into(mr.project_path.clone());
                            }
                        }
                    }
                }
            } else if keybinding_matches(&app.config.keybindings.mrs.selection_toggle, key_event) {
                app.toggle_select_mode();
            } else if keybinding_matches(&app.config.keybindings.mrs.select_all, key_event) {
                let added = app.select_all_filtered();
                show_selected_all_notice(app, added);
            } else if keybinding_matches(&app.config.keybindings.mrs.edit_entity, key_event) {
                if app.selected_mrs.len() > 1 {
                    let count = app.selected_mrs.len();
                    let pr_suffix = if app.is_github() { "PR" } else { "MR" };
                    app.open_edit_menu(crate::app::EditMenu {
                        title: format!("Bulk Edit {count} {pr_suffix}s"),
                        entity_project: app.scope.as_str().to_string(),
                        fields: vec![
                            crate::app::Field::multi_select("Assignees", String::new()),
                            crate::app::Field::multi_select("Milestone", String::new()),
                            crate::app::Field::multi_select("Labels", String::new()),
                        ],
                        // Bulk-edit forms start empty; `open_edit_menu` will snapshot all
                        // blank values as the baseline. The `MrUpdate::is_empty()` guard
                        // in the dispatcher handles the true no-op case (nothing filled in).
                        initial_fields: std::collections::HashMap::new(),
                        selected_idx: 0,
                        entity_iid: 0,
                        entity_kind: crate::app::EditEntityKind::BulkEditMrs,
                        state: {
                            let mut s = ListState::default();
                            s.select(Some(0));
                            s
                        },
                        workflow_inputs: vec![],
                        cursor_pos: 0,
                        editing: false,
                        desc_scroll: 0,
                    });
                } else if let Some(selected_idx) = app.mrs.state.selected() {
                    let filtered = app.filtered_mrs();
                    if let Some(mr) = filtered.get(selected_idx) {
                        let is_github = app.is_github();
                        let pr_suffix = if is_github { "PR" } else { "MR" };
                        let unresolved = if app.diff_view.as_ref().map(|d| d.mr_iid) == Some(mr.iid)
                        {
                            Some(app.unresolved_threads_count())
                        } else {
                            None
                        };
                        let is_fetching_issues = app.fetching_mr_related_issues.contains(&mr.iid);
                        let mut doc = crate::entity_editor::build_mr_document(
                            mr,
                            is_github,
                            unresolved,
                            is_fetching_issues,
                        );
                        doc.fields.push(crate::app::Field::text(
                            "Description",
                            mr.description.clone().unwrap_or_default(),
                        ));
                        app.open_edit_menu(crate::app::EditMenu {
                            title: format!("Edit {} #{}", pr_suffix, mr.iid),
                            entity_project: mr.project_path.clone(),
                            fields: doc.fields,
                            initial_fields: std::collections::HashMap::new(),
                            selected_idx: 0,
                            entity_iid: mr.iid,
                            entity_kind: crate::app::EditEntityKind::EditMr,
                            state: {
                                let mut s = ListState::default();
                                s.select(Some(0));
                                s
                            },
                            workflow_inputs: vec![],
                            cursor_pos: 0,
                            editing: false,
                            desc_scroll: 0,
                        });
                    }
                }
            } else if app.selected_mrs.len() > 1
                && keybinding_matches(&app.config.keybindings.mrs.merge_mr, key_event)
            {
                let items: Vec<(String, u64)> = app.selected_mrs.iter().cloned().collect();
                app.submit_dialog = Some(crate::app::SubmitDialog::build(
                    crate::app::ConfirmAction::BulkMergeMrs(items),
                    app,
                ));
            } else if let Some(selected_idx) = app.mrs.state.selected() {
                let mr_opt = app.filtered_mrs().get(selected_idx).cloned().cloned();
                if let Some(mr) = mr_opt {
                    let mr_iid = mr.iid;
                    let mr_title = mr.title.clone();
                    match key_event.code {
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.approve_mr,
                            key_event,
                        ) =>
                        {
                            if let Some(client) = app.gitlab_client.clone() {
                                let project_path = app.project_path_for_mr(mr_iid);
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let result = client.approve_mr(&project_path, mr_iid).await;
                                    let _ = tx2.send(Event::CommandCompleted(
                                        crate::app::Tab::MergeRequests,
                                        result.map_err(|e| e.to_string()),
                                    ));
                                });
                            }
                        }
                        _ if (key_event.code == KeyCode::Char('A')
                            || keybinding_matches(
                                &app.config.keybindings.mrs.revoke_mr,
                                key_event,
                            )) =>
                        {
                            let is_github = app
                                .gitlab_client
                                .as_ref()
                                .map(|c| c.is_github)
                                .unwrap_or(false);
                            if is_github {
                                app.error_message =
                                    Some("Revoking approval isn't supported on GitHub".to_string());
                                app.error_message_at = Some(std::time::Instant::now());
                            } else {
                                app.submit_dialog =
                                    Some(crate::app::SubmitDialog::build_with_project(
                                        crate::app::ConfirmAction::RevokeMr(mr_iid),
                                        app.scope.project_of(&mr.project_path).to_string(),
                                        app,
                                    ));
                            }
                        }
                        _ if (key_event.code == KeyCode::Char('R')
                            || keybinding_matches(
                                &app.config.keybindings.mrs.rebase_mr,
                                key_event,
                            )) =>
                        {
                            use crate::domain::mr_state::{RebaseGate, rebase_gate};
                            match rebase_gate(mr.mergeability.as_ref()) {
                                RebaseGate::Allowed => {
                                    app.submit_dialog =
                                        Some(crate::app::SubmitDialog::build_with_project(
                                            crate::app::ConfirmAction::RebaseMr(mr_iid),
                                            app.scope.project_of(&mr.project_path).to_string(),
                                            app,
                                        ));
                                }
                                RebaseGate::ResolveLocally => {
                                    app.show_error(
                                        "Resolve conflicts locally; rebase can't fix them"
                                            .to_string(),
                                    );
                                }
                                RebaseGate::NotNeeded => {
                                    app.show_error("This MR doesn't need a rebase".to_string());
                                }
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.merge_mr,
                            key_event,
                        ) =>
                        {
                            with_pr_stack(app, &mr, crate::app::StackAction::Merge, &tx);
                        }
                        _ if (key_event.code == KeyCode::Char('D')
                            || keybinding_matches(
                                &app.config.keybindings.mrs.view_diff,
                                key_event,
                            )) =>
                        {
                            let project_context = if !mr.project_path.is_empty() {
                                mr.project_path.clone()
                            } else {
                                app.scope.as_str().to_string()
                            };
                            spawn_open_diff(app, &tx, mr_iid, project_context);
                        }
                        _ if (key_event.code == KeyCode::Char('P')
                            || keybinding_matches(
                                &app.config.keybindings.mrs.view_related_pipelines,
                                key_event,
                            )) =>
                        {
                            let pipe_id = mr.head_pipeline.as_ref().map(|p| p.id()).or_else(|| {
                                app.pipelines
                                    .items
                                    .iter()
                                    .find(|p| p.ref_branch() == mr.source_branch)
                                    .map(|p| p.id())
                            });
                            app.active_tab = crate::app::Tab::Pipelines;
                            app.pending_pipeline_select = pipe_id;
                            if let Some(client) = &app.gitlab_client {
                                crate::fetch::spawn_refresh_active_tab(
                                    client,
                                    &app.scope,
                                    app.requested_state_queries(),
                                    crate::app::Tab::Pipelines,
                                    app.scoped_sender(tx.clone()),
                                );
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.view_stack,
                            key_event,
                        ) =>
                        {
                            with_pr_stack(app, &mr, crate::app::StackAction::Browse, &tx);
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.open_in_browser,
                            key_event,
                        ) =>
                        {
                            let is_github = app.is_github();
                            let entity = if is_github { "pr" } else { "mr" };
                            let Some(client) = app.gitlab_client.clone() else {
                                return false;
                            };
                            let project_path = if !mr.project_path.is_empty() {
                                mr.project_path.clone()
                            } else {
                                app.scope.as_str().to_string()
                            };
                            let tx2 = tx.clone();
                            let iid_str = mr_iid.to_string();
                            tokio::spawn(async move {
                                let result = client
                                    .open_in_browser(&project_path, entity, &iid_str)
                                    .await;
                                let _ = tx2.send(Event::CommandCompleted(
                                    crate::app::Tab::MergeRequests,
                                    result.map_err(|e| e.to_string()),
                                ));
                            });
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.toggle_draft,
                            key_event,
                        ) =>
                        {
                            let is_draft = app
                                .mrs
                                .items
                                .iter()
                                .find(|m| m.iid == mr_iid)
                                .map(|m| m.draft)
                                .unwrap_or_else(|| {
                                    mr_title.starts_with("Draft:") || mr_title.starts_with("WIP:")
                                });
                            if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == mr_iid) {
                                item.draft = !is_draft;
                            }
                            if let Some(client) = app.gitlab_client.clone() {
                                let project_path = app.project_path_for_mr(mr_iid);
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let result = client
                                        .toggle_mr_draft(&project_path, mr_iid, !is_draft)
                                        .await;
                                    let _ = tx2.send(Event::CommandCompleted(
                                        crate::app::Tab::MergeRequests,
                                        result.map_err(|e| e.to_string()),
                                    ));
                                });
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.close_entity,
                            key_event,
                        ) =>
                        {
                            app.submit_dialog = Some(crate::app::SubmitDialog::build_with_project(
                                crate::app::ConfirmAction::CloseMr(mr_iid),
                                app.scope.project_of(&mr.project_path).to_string(),
                                app,
                            ));
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.delete_entity,
                            key_event,
                        ) =>
                        {
                            if !app
                                .gitlab_client
                                .as_ref()
                                .map(|c| c.is_github)
                                .unwrap_or(false)
                            {
                                app.submit_dialog =
                                    Some(crate::app::SubmitDialog::build_with_project(
                                        crate::app::ConfirmAction::DeleteMr(mr_iid),
                                        app.scope.project_of(&mr.project_path).to_string(),
                                        app,
                                    ));
                            } else {
                                app.show_error(
                                    "GitHub does not support deleting pull requests".to_string(),
                                );
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.reopen_entity,
                            key_event,
                        ) =>
                        {
                            app.submit_dialog = Some(crate::app::SubmitDialog::build_with_project(
                                crate::app::ConfirmAction::ReopenMr(mr_iid),
                                app.scope.project_of(&mr.project_path).to_string(),
                                app,
                            ));
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.copy_reference,
                            key_event,
                        ) =>
                        {
                            if let Err(error) = app.copy_selected_mr_reference() {
                                let label = if app.is_github() { "PR" } else { "MR" };
                                app.show_error(format!(
                                    "Failed to copy {label} reference: {error}"
                                ));
                            }
                        }
                        _ if (key_event.code == KeyCode::Char('I')
                            || keybinding_matches(
                                &app.config.keybindings.mrs.jump_linked_issues,
                                key_event,
                            )) =>
                        {
                            use crate::domain::mr::RelatedIssuesState;
                            let state = app
                                .mrs
                                .items
                                .iter()
                                .find(|m| m.iid == mr_iid)
                                .and_then(|m| m.related_issues.as_ref());
                            match state {
                                None if app.fetching_mr_related_issues.contains(&mr_iid) => {
                                    app.show_error("Linked Issues still loading…".to_string());
                                }
                                None => {
                                    app.show_error(
                                        "No linked Issues cached yet — open the Merge Request once and retry."
                                            .to_string(),
                                    );
                                }
                                Some(RelatedIssuesState::Empty) => {
                                    app.show_error(
                                        "This Merge Request / Pull Request has no linked issues."
                                            .to_string(),
                                    );
                                }
                                Some(RelatedIssuesState::Failed(msg)) => {
                                    app.show_error(format!("Failed to fetch linked issues: {msg}"));
                                }
                                Some(RelatedIssuesState::Items(items)) => {
                                    if items.len() == 1 {
                                        let client = app.gitlab_client.clone();
                                        jump_to_issue_tab(app, items[0].iid, client, tx.clone());
                                    } else {
                                        let is_github = app.is_github();
                                        let pr_term = if is_github { "PR" } else { "MR" };
                                        app.selector = Some(crate::app::Selector {
                                            title: format!(
                                                " Linked Issues for {pr_term} #{mr_iid} "
                                            ),
                                            all_items: items
                                                .iter()
                                                .map(|r| {
                                                    format!(
                                                        "#{} [{}] {}",
                                                        r.iid,
                                                        match r.state.as_str() {
                                                            "opened" | "open" => "OPEN",
                                                            "closed" | "close" => "CLOSED",
                                                            other => other,
                                                        },
                                                        r.title
                                                    )
                                                })
                                                .collect(),
                                            selected_items: std::collections::HashSet::new(),
                                            cursor_idx: 0,
                                            search_query: String::new(),
                                            is_filtering: false,
                                            is_loading: false,
                                            entity_iid: mr_iid,
                                            entity_type: String::new(),
                                            field_type: "linked_issues".to_string(),
                                            multi_select: false,
                                            state: {
                                                let mut s = ListState::default();
                                                s.select(Some(0));
                                                s
                                            },
                                        });
                                    }
                                }
                            }
                        }
                        _ => handled = false,
                    }
                } else {
                    handled = false;
                }
            } else {
                handled = false;
            }
        }
        crate::app::Tab::Pipelines => {
            if keybinding_matches(&app.config.keybindings.pipelines.enter_jobs, key_event) {
                let pipe_id = app
                    .pipelines
                    .state
                    .selected()
                    .and_then(|idx| app.filtered_pipelines().get(idx).map(|p| p.id()));
                if let Some(pipeline_id) = pipe_id {
                    let project_context = app.project_path_for_pipeline(pipeline_id);
                    spawn_open_pipeline(
                        app,
                        &tx,
                        pipeline_id,
                        project_context,
                        PipelineOpening::OwnJobs,
                    );
                }
            } else if keybinding_matches(&app.config.keybindings.pipelines.run_new, key_event) {
                let current_branch = crate::git_helpers::get_current_branch()
                    .await
                    .unwrap_or_else(|| "main".to_string());

                let is_github = app.is_github();
                let mut fields = vec![];
                let default_project = if app.scope.is_group() {
                    let pipelines = app.filtered_pipelines();
                    let proj = app
                        .pipelines
                        .state
                        .selected()
                        .and_then(|idx| pipelines.get(idx))
                        .map(|p| p.project_path.clone())
                        .filter(|p| !p.is_empty())
                        .unwrap_or_default();
                    fields.push(crate::app::Field::ref_field("Project", proj.clone()));
                    proj
                } else {
                    app.scope.as_str().to_string()
                };
                fields.push(crate::app::Field::text(
                    "Branch / Ref",
                    current_branch.clone(),
                ));
                if is_github {
                    fields.push(crate::app::Field::text("Workflow File", String::new()));
                } else {
                    fields.push(crate::app::Field::toggle(
                        "Merge Request Pipeline",
                        "No".to_string(),
                    ));
                }
                fields.push(crate::app::Field::text("Inputs", String::new()));
                fields.push(crate::app::Field::text("Variables", String::new()));

                app.open_edit_menu(crate::app::EditMenu {
                    title: "Run Pipeline".to_string(),
                    entity_project: default_project,
                    fields,
                    initial_fields: std::collections::HashMap::new(),
                    selected_idx: 0,
                    entity_iid: 0,
                    entity_kind: crate::app::EditEntityKind::CreatePipeline,
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                    workflow_inputs: vec![],
                    cursor_pos: 0,
                    editing: false,
                    desc_scroll: 0,
                });
            } else if keybinding_matches(
                &app.config.keybindings.pipelines.selection_toggle,
                key_event,
            ) {
                app.toggle_select_mode();
            } else if keybinding_matches(&app.config.keybindings.pipelines.select_all, key_event) {
                let added = app.select_all_filtered();
                show_selected_all_notice(app, added);
            } else if keybinding_matches(
                &app.config.keybindings.pipelines.trigger_pipeline,
                key_event,
            ) {
                if let Some(client) = app.gitlab_client.clone() {
                    let project_path = app.scope.as_str().to_string();
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        let branch = crate::git_helpers::get_current_branch()
                            .await
                            .unwrap_or_else(|| "main".to_string());
                        let result = client
                            .run_pipeline(&project_path, &branch, false, &[], &[], "")
                            .await;
                        let _ = tx2.send(Event::CommandCompleted(
                            crate::app::Tab::Pipelines,
                            result.map_err(|e| e.to_string()),
                        ));
                    });
                }
            } else if let Some(selected_idx) = app.pipelines.state.selected() {
                if let Some(item) = app.filtered_pipelines().get(selected_idx) {
                    let pipe_id = item.id();
                    match key_event.code {
                        _ if (key_event.code == KeyCode::Char(' ')
                            || keybinding_matches(
                                &app.config.keybindings.pipelines.select_pipeline,
                                key_event,
                            )) =>
                        {
                            if app.selected_pipelines.contains(&pipe_id) {
                                app.selected_pipelines.remove(&pipe_id);
                            } else {
                                app.selected_pipelines.insert(pipe_id);
                            }
                        }
                        _ if (key_event.code == KeyCode::Char('r')
                            || keybinding_matches(
                                &app.config.keybindings.pipelines.retry,
                                key_event,
                            )) =>
                        {
                            if let Some(client) = &app.gitlab_client {
                                let client_clone = client.clone();
                                let scope = app.scope.clone();
                                let tx = app.scoped_sender(tx.clone());
                                let active_tab = app.active_tab;
                                let states = app.requested_state_queries();
                                let pipe_ids: Vec<u64> = if app.selected_pipelines.is_empty() {
                                    vec![pipe_id]
                                } else {
                                    app.selected_pipelines.drain().collect()
                                };
                                let projects: std::collections::HashMap<u64, String> = pipe_ids
                                    .iter()
                                    .map(|&id| (id, app.project_path_for_pipeline(id)))
                                    .collect();
                                let rows = flip_pipeline_statuses(app, &pipe_ids, "running");
                                tokio::spawn(async move {
                                    run_optimistic_mutation(
                                        OptimisticTable::Pipelines,
                                        "retry pipeline",
                                        rows,
                                        &tx,
                                        |id| client_clone.retry_pipeline(&projects[&id], id),
                                    )
                                    .await;
                                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                    spawn_refresh_active_tab(
                                        &client_clone,
                                        &scope,
                                        states,
                                        active_tab,
                                        tx,
                                    );
                                });
                            }
                        }
                        _ if ((key_event.code == KeyCode::Char('d')
                            && key_event.modifiers.is_empty())
                            || keybinding_matches(
                                &app.config.keybindings.pipelines.cancel,
                                key_event,
                            )) =>
                        {
                            let project_path = app.project_path_for_pipeline(pipe_id);
                            app.submit_dialog = Some(crate::app::SubmitDialog::build_with_project(
                                crate::app::ConfirmAction::CancelPipeline(pipe_id),
                                project_path,
                                app,
                            ));
                        }
                        _ if (key_event.code == KeyCode::Char('W')
                            || keybinding_matches(
                                &app.config.keybindings.pipelines.open_workflow,
                                key_event,
                            )) =>
                        {
                            if !app.is_github() {
                                app.show_error(
                                    "Workflow browser is only available for GitHub Actions"
                                        .to_string(),
                                );
                                return false;
                            }
                            let workflow = app
                                .pipelines
                                .items
                                .iter()
                                .find(|pipeline| pipeline.id() == pipe_id)
                                .map(|pipeline| pipeline.name().to_string())
                                .filter(|name| !name.is_empty());
                            let Some(workflow) = workflow else {
                                app.error_message =
                                    Some("Selected pipeline has no workflow name".to_string());
                                return false;
                            };
                            let Some(client) = app.gitlab_client.clone() else {
                                return false;
                            };
                            let project_context = app.project_path_for_pipeline(pipe_id);
                            let tx2 = tx.clone();
                            tokio::spawn(async move {
                                let result = client
                                    .backend
                                    .open_workflow_in_browser(&project_context, &workflow)
                                    .await;
                                let _ = tx2.send(Event::CommandCompleted(
                                    crate::app::Tab::Pipelines,
                                    result.map_err(|e| e.to_string()),
                                ));
                            });
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.pipelines.open_in_browser,
                            key_event,
                        ) =>
                        {
                            let Some(client) = app.gitlab_client.clone() else {
                                return false;
                            };
                            let project_path = if !item.project_path.is_empty() {
                                item.project_path.clone()
                            } else {
                                app.scope.as_str().to_string()
                            };
                            let pid_str = pipe_id.to_string();
                            let tx2 = tx.clone();
                            tokio::spawn(async move {
                                let result = client
                                    .open_pipeline_in_browser(&project_path, &pid_str)
                                    .await;
                                let _ = tx2.send(Event::CommandCompleted(
                                    crate::app::Tab::Pipelines,
                                    result.map_err(|e| e.to_string()),
                                ));
                            });
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.pipelines.copy_sha,
                            key_event,
                        ) =>
                        {
                            if let Err(error) = app.copy_selected_pipeline_sha() {
                                app.show_error(format!("Failed to copy commit SHA: {error}"));
                            }
                        }
                        _ => handled = false,
                    }
                } else {
                    handled = false;
                }
            } else {
                handled = false;
            }
        }
        crate::app::Tab::Jobs => {
            if keybinding_matches(&app.config.keybindings.jobs.enter_pipeline, key_event) {
                let pipelines: Vec<String> = app
                    .pipelines
                    .items
                    .iter()
                    .map(|p| format!("#{} — {} ({})", p.id(), p.ref_branch(), p.status()))
                    .collect();
                let mut pre_selected = std::collections::HashSet::new();
                if let Some(active_id) = app.active_pipeline_id {
                    if let Some(i) = app.pipelines.items.iter().position(|p| p.id() == active_id) {
                        if let Some(p) = pipelines.get(i) {
                            pre_selected.insert(p.clone());
                        }
                    }
                }
                let start_idx = pre_selected
                    .iter()
                    .next()
                    .and_then(|sel| pipelines.iter().position(|p| p == sel))
                    .unwrap_or(0);
                app.selector = Some(crate::app::Selector {
                    title: " Select Pipeline ".to_string(),
                    all_items: pipelines,
                    selected_items: pre_selected,
                    cursor_idx: start_idx,
                    search_query: String::new(),
                    is_filtering: false,
                    is_loading: false,
                    entity_iid: 0,
                    entity_type: String::new(),
                    field_type: "pipeline_select".to_string(),
                    multi_select: false,
                    state: {
                        let mut s = ratatui::widgets::ListState::default();
                        s.select(Some(start_idx));
                        s
                    },
                });
            } else if keybinding_matches(&app.config.keybindings.jobs.selection_toggle, key_event) {
                app.toggle_select_mode();
            } else if keybinding_matches(&app.config.keybindings.jobs.select_all, key_event) {
                let added = app.select_all_filtered();
                show_selected_all_notice(app, added);
            } else if let Some(idx) = app.jobs.state.selected() {
                let job_info = app
                    .filtered_jobs()
                    .get(idx)
                    .map(|j| (j.id(), j.name().to_string()));
                if let Some((job_id, job_name)) = job_info {
                    match key_event.code {
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.select_job,
                            key_event,
                        ) =>
                        {
                            if app.selected_jobs.contains(&job_id) {
                                app.selected_jobs.remove(&job_id);
                            } else {
                                app.selected_jobs.insert(job_id);
                            }
                        }
                        _ if keybinding_matches(&app.config.keybindings.jobs.retry, key_event) => {
                            if let Some(client) = &app.gitlab_client {
                                let client_clone = client.clone();
                                let pipe_id = app.active_pipeline_id.unwrap_or(0);
                                let project_context = app.project_path_for_pipeline(pipe_id);
                                let tx = app.scoped_sender(tx.clone());
                                let job_ids: Vec<u64> = if app.selected_jobs.is_empty() {
                                    vec![job_id]
                                } else {
                                    app.status_message = None;
                                    app.selected_jobs.drain().collect()
                                };
                                let rows = flip_job_statuses(app, &job_ids, "running");
                                tokio::spawn(async move {
                                    run_optimistic_mutation(
                                        OptimisticTable::Jobs,
                                        "retry job",
                                        rows,
                                        &tx,
                                        |id| client_clone.retry_job(&project_context, id),
                                    )
                                    .await;
                                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                    if let Ok(jobs) = crate::domain::pipelines::list_pipeline_jobs(
                                        &client_clone,
                                        &project_context,
                                        pipe_id,
                                    )
                                    .await
                                    {
                                        let _ = tx.send(Event::PipelineJobs(pipe_id, jobs));
                                    }
                                });
                            }
                        }
                        _ if key_event.code == KeyCode::Char('S')
                            || keybinding_matches(
                                &app.config.keybindings.jobs.start_job,
                                key_event,
                            ) =>
                        {
                            // GitHub cannot start a manual job. The key is still
                            // taken there, so it never reaches a custom binding.
                            if !app.is_github()
                                && let Some(client) = &app.gitlab_client
                            {
                                let client_clone = client.clone();
                                let pipe_id = app.active_pipeline_id.unwrap_or(0);
                                let project_context = app.project_path_for_pipeline(pipe_id);
                                let tx = app.scoped_sender(tx.clone());

                                let previous = app
                                    .jobs
                                    .items
                                    .iter_mut()
                                    .find(|job| job.id() == job_id && job.status == "manual")
                                    .map(|job| {
                                        std::mem::replace(&mut job.status, "running".to_string())
                                    });
                                tokio::spawn(async move {
                                    run_optimistic_mutation(
                                        OptimisticTable::Jobs,
                                        "start job",
                                        vec![(job_id, previous)],
                                        &tx,
                                        |id| client_clone.start_job(&project_context, id),
                                    )
                                    .await;
                                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                    if let Ok(jobs) = crate::domain::pipelines::list_pipeline_jobs(
                                        &client_clone,
                                        &project_context,
                                        pipe_id,
                                    )
                                    .await
                                    {
                                        let _ = tx.send(Event::PipelineJobs(pipe_id, jobs));
                                    }
                                });
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.select_stage,
                            key_event,
                        ) =>
                        {
                            let highlighted_stage = app
                                .filtered_jobs()
                                .get(idx)
                                .map(|job| job.stage().to_string());
                            if let Some(stage_name) = highlighted_stage {
                                let stage_ids: Vec<u64> = app
                                    .filtered_jobs()
                                    .iter()
                                    .filter(|job| job.stage() == stage_name)
                                    .map(|job| job.id())
                                    .collect();
                                app.selected_jobs.extend(stage_ids);
                                app.show_notice(format!(
                                    "Selected all jobs in stage '{stage_name}'"
                                ));
                            }
                        }
                        _ if keybinding_matches(&app.config.keybindings.jobs.cancel, key_event) => {
                            let pipe_id = app.active_pipeline_id.unwrap_or(0);
                            if !app.selected_jobs.is_empty() {
                                let mut job_ids: Vec<u64> =
                                    app.selected_jobs.iter().copied().collect();
                                job_ids.sort_unstable();
                                let project_path = app.project_path_for_pipeline(pipe_id);
                                app.submit_dialog =
                                    Some(crate::app::SubmitDialog::build_with_project(
                                        crate::app::ConfirmAction::CancelJobs {
                                            pipeline_id: pipe_id,
                                            job_ids,
                                        },
                                        project_path,
                                        app,
                                    ));
                            } else if let Some(client) = app.gitlab_client.clone() {
                                let project_path = app.project_path_for_pipeline(pipe_id);
                                let tx = app.scoped_sender(tx.clone());
                                let rows = flip_job_statuses(app, &[job_id], "canceled");
                                spawn_cancel_jobs(client, tx, project_path, pipe_id, rows);
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.download_artifact,
                            key_event,
                        ) =>
                        {
                            if let Some(client) = app.gitlab_client.clone() {
                                let ref_name = app
                                    .active_pipeline_id
                                    .and_then(|pipe_id| {
                                        app.pipelines
                                            .items
                                            .iter()
                                            .find(|p| p.id() == pipe_id)
                                            .map(|p| p.ref_branch().to_string())
                                    })
                                    .unwrap_or_else(|| "master".to_string());
                                let active_pipe_path = app
                                    .active_pipeline_id
                                    .and_then(|p_id| {
                                        app.pipelines
                                            .items
                                            .iter()
                                            .find(|p| p.id() == p_id)
                                            .map(|p| p.project_path.clone())
                                    })
                                    .filter(|p| !p.is_empty());
                                let project_path = active_pipe_path
                                    .unwrap_or_else(|| app.scope.as_str().to_string());
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let result = client
                                        .download_artifact(&project_path, &ref_name, &job_name)
                                        .await;
                                    let _ = tx2.send(Event::CommandCompleted(
                                        crate::app::Tab::Jobs,
                                        result.map_err(|e| e.to_string()),
                                    ));
                                });
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.open_in_browser,
                            key_event,
                        ) =>
                        {
                            let Some(client) = app.gitlab_client.clone() else {
                                return false;
                            };
                            let active_pipe_path = app
                                .active_pipeline_id
                                .and_then(|p_id| {
                                    app.pipelines
                                        .items
                                        .iter()
                                        .find(|p| p.id() == p_id)
                                        .map(|p| p.project_path.clone())
                                })
                                .filter(|p| !p.is_empty());
                            let project_path =
                                active_pipe_path.unwrap_or_else(|| app.scope.as_str().to_string());
                            let jid_str = job_id.to_string();
                            let tx2 = tx.clone();
                            tokio::spawn(async move {
                                let result =
                                    client.open_job_in_browser(&project_path, &jid_str).await;
                                let _ = tx2.send(Event::CommandCompleted(
                                    crate::app::Tab::Jobs,
                                    result.map_err(|e| e.to_string()),
                                ));
                            });
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.view_trace_editor,
                            key_event,
                        ) =>
                        {
                            if app.job_trace.is_some() {
                                open_job_trace_in_editor(app, job_id, terminal);
                            } else if spawn_job_trace_fetch(app, &tx, job_id) {
                                app.pending_job_trace_editor = Some(job_id);
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.view_trace,
                            key_event,
                        ) =>
                        {
                            if app.job_trace.is_some() {
                                app.details_zoomed = !app.details_zoomed;
                            } else {
                                spawn_job_trace_fetch(app, &tx, job_id);
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.toggle_trace_wrap,
                            key_event,
                        ) =>
                        {
                            app.job_trace_wrap = !app.job_trace_wrap;
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.toggle_trace_follow,
                            key_event,
                        ) =>
                        {
                            app.job_trace_follow = !app.job_trace_follow;
                            if app.job_trace_follow {
                                app.detail_scroll_to_bottom = true;
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.copy_sha,
                            key_event,
                        ) =>
                        {
                            if let Err(error) = app.copy_selected_job_sha() {
                                app.show_error(format!("Failed to copy commit SHA: {error}"));
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.copy_url,
                            key_event,
                        ) =>
                        {
                            if let Err(error) = app.copy_selected_job_url() {
                                app.show_error(format!("Failed to copy job URL: {error}"));
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.copy_id,
                            key_event,
                        ) =>
                        {
                            if let Err(error) = app.copy_selected_job_id() {
                                app.show_error(format!("Failed to copy job ID: {error}"));
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.jump_to_failure,
                            key_event,
                        ) =>
                        {
                            if let Some(view) = &app.job_trace {
                                let after = view.failure_search_start();
                                app.jump_to_trace_failure(Some(after));
                            } else if spawn_job_trace_fetch(app, &tx, job_id) {
                                app.pending_job_trace_failure = Some(job_id);
                            }
                        }
                        _ => handled = false,
                    }
                } else {
                    handled = false;
                }
            } else {
                handled = false;
            }
        }
        crate::app::Tab::Runners => {
            let can_manage_runners = app.kind().supports_runner_management();
            if let Some(item) = app.runners.cursor_row() {
                let runner_id = item.id;
                match key_event.code {
                    // GitHub cannot pause, resume or rename a runner. The keys are still
                    // taken there, so a rebinding never reaches a list-view or custom binding.
                    _ if !can_manage_runners
                        && [
                            &app.config.keybindings.runners.pause,
                            &app.config.keybindings.runners.resume,
                            &app.config.keybindings.runners.edit_description,
                        ]
                        .into_iter()
                        .any(|binding| keybinding_matches(binding, key_event)) => {}
                    _ if keybinding_matches(&app.config.keybindings.runners.pause, key_event) => {
                        let prev = item.status.clone();
                        let prev_active = item.active;
                        app.runners.update(&runner_id, |runner| {
                            runner.status = "paused".to_string();
                            runner.active = false;
                        });
                        if let Some(client) = app.gitlab_client.clone() {
                            let scope = app.scope.clone();
                            let tx = tx.clone();
                            let scoped_tx = app.scoped_sender(tx.clone());
                            tokio::spawn(async move {
                                let result = client.pause_runner(&scope, runner_id).await;
                                if result.is_err() {
                                    let _ = scoped_tx.send(Event::RunnerStateRevert {
                                        runner_id,
                                        status: prev,
                                        active: prev_active,
                                    });
                                }
                                let _ = tx.send(Event::CommandCompleted(
                                    crate::app::Tab::Runners,
                                    result.map_err(|e| e.to_string()),
                                ));
                            });
                        }
                    }
                    _ if keybinding_matches(&app.config.keybindings.runners.resume, key_event) => {
                        let prev = item.status.clone();
                        let prev_active = item.active;
                        app.runners.update(&runner_id, |runner| {
                            runner.status = "online".to_string();
                            runner.active = true;
                        });
                        if let Some(client) = app.gitlab_client.clone() {
                            let scope = app.scope.clone();
                            let tx = tx.clone();
                            let scoped_tx = app.scoped_sender(tx.clone());
                            tokio::spawn(async move {
                                let result = client.resume_runner(&scope, runner_id).await;
                                if result.is_err() {
                                    let _ = scoped_tx.send(Event::RunnerStateRevert {
                                        runner_id,
                                        status: prev,
                                        active: prev_active,
                                    });
                                }
                                let _ = tx.send(Event::CommandCompleted(
                                    crate::app::Tab::Runners,
                                    result.map_err(|e| e.to_string()),
                                ));
                            });
                        }
                    }
                    _ if keybinding_matches(
                        &app.config.keybindings.runners.edit_description,
                        key_event,
                    ) =>
                    {
                        let current_desc = item.description.clone().unwrap_or_default();
                        app.text_input = Some(crate::app::TextInput::new(
                            " Edit Runner Description ",
                            current_desc,
                            crate::app::TextInputAction::EditField {
                                entity_iid: runner_id,
                                entity_type: "runner".to_string(),
                                field_type: "runner_description".to_string(),
                            },
                        ));
                    }
                    _ if keybinding_matches(
                        &app.config.keybindings.runners.open_in_browser,
                        key_event,
                    ) =>
                    {
                        let runner_id_local = runner_id;
                        let client = app.gitlab_client.clone();
                        let scope = app.scope.clone();
                        let tx2 = tx.clone();
                        tokio::spawn(async move {
                            let Some(client) = client else {
                                return;
                            };
                            let result =
                                client.open_runner_in_browser(&scope, runner_id_local).await;
                            let _ = tx2.send(Event::CommandCompleted(
                                crate::app::Tab::Runners,
                                result.map_err(|e| e.to_string()),
                            ));
                        });
                    }
                    _ => handled = false,
                }
            } else {
                handled = false;
            }
        }
        crate::app::Tab::Releases => match key_event.code {
            _ if keybinding_matches(&app.config.keybindings.releases.create_release, key_event) => {
                let mut fields = vec![crate::app::Field::section("Details")];
                if app.scope.is_group() {
                    fields.push(crate::app::Field::ref_field("Project", String::new()));
                }
                fields.extend(vec![
                    crate::app::Field::ref_field("Tag", String::new()),
                    crate::app::Field::text("Release Name", String::new()),
                    crate::app::Field::section("Release Notes"),
                    crate::app::Field::text("Release Notes", String::new()),
                ]);
                app.open_edit_menu(crate::app::EditMenu {
                    title: "Create Release".to_string(),
                    entity_project: if app.scope.is_group() {
                        String::new()
                    } else {
                        app.scope.as_str().to_string()
                    },
                    fields,
                    initial_fields: std::collections::HashMap::new(),
                    selected_idx: 0,
                    entity_iid: 0,
                    entity_kind: crate::app::EditEntityKind::CreateRelease,
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                    workflow_inputs: vec![],
                    cursor_pos: 0,
                    editing: false,
                    desc_scroll: 0,
                });
            }
            _ if keybinding_matches(&app.config.keybindings.releases.edit_release, key_event) => {
                if let Some(release) = app.releases.cursor_row().cloned() {
                    crate::entity_editor::open_release_edit_menu(app, &release);
                }
            }
            _ if keybinding_matches(&app.config.keybindings.releases.delete_release, key_event) => {
                if let Some(release) = app.releases.cursor_row() {
                    app.submit_dialog = Some(crate::app::SubmitDialog::build_with_project(
                        crate::app::ConfirmAction::DeleteRelease(release.tag_name.clone()),
                        release.project_or(&app.scope).to_string(),
                        app,
                    ));
                }
            }
            _ if keybinding_matches(
                &app.config.keybindings.releases.open_in_browser,
                key_event,
            ) =>
            {
                if let Some(release) = app.releases.cursor_row() {
                    let Some(client) = app.gitlab_client.clone() else {
                        return false;
                    };
                    let project_path = release.project_or(&app.scope).to_string();
                    let tag_name = release.tag_name.clone();
                    let tx2 = tx.clone();

                    tokio::spawn(async move {
                        let result = client
                            .open_in_browser(&project_path, "release", tag_name.as_str())
                            .await;
                        let _ = tx2.send(Event::CommandCompleted(
                            crate::app::Tab::Releases,
                            result.map_err(|e| e.to_string()),
                        ));
                    });
                }
            }
            _ => handled = false,
        },
        crate::app::Tab::Todos => match key_event.code {
            _ if keybinding_matches(&app.config.keybindings.todos.toggle_show_read, key_event) => {
                crate::handlers::todos::toggle_show_read(app, &tx);
            }
            _ if keybinding_matches(&app.config.keybindings.todos.mark_all_as_read, key_event) => {
                crate::handlers::todos::confirm_mark_all_read(app);
            }
            _ if app.selected_todo().is_none() => handled = false,
            _ if keybinding_matches(&app.config.keybindings.todos.mark_as_read, key_event) => {
                crate::handlers::todos::open_selected(app, &tx);
            }
            _ if keybinding_matches(&app.config.keybindings.todos.mark_as_done, key_event) => {
                crate::handlers::todos::mark_selected_done(app, &tx);
            }
            _ if keybinding_matches(&app.config.keybindings.todos.unsubscribe, key_event) => {
                crate::handlers::todos::unsubscribe_selected(app, &tx);
            }
            _ if keybinding_matches(&app.config.keybindings.todos.open_in_browser, key_event) => {
                crate::handlers::todos::open_selected_in_browser(app, &tx);
            }
            _ => handled = false,
        },
        crate::app::Tab::Milestones => match key_event.code {
            _ if keybinding_matches(
                &app.config.keybindings.milestones.create_milestone,
                key_event,
            ) =>
            {
                let is_github = app.is_github();
                let project = if app.scope.is_group() {
                    let milestones = app.filtered_milestones();
                    app.milestones
                        .state
                        .selected()
                        .and_then(|idx| milestones.get(idx))
                        .map(|m| m.project_path.clone())
                        .filter(|p| !p.is_empty())
                        .unwrap_or_default()
                } else {
                    app.scope.as_str().to_string()
                };
                let fields = crate::entity_editor::milestone_fields(
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    is_github,
                    if app.scope.is_group() {
                        Some(project.clone())
                    } else {
                        None
                    },
                );
                app.open_edit_menu(crate::app::EditMenu {
                    title: "Create Milestone".to_string(),
                    entity_project: project,
                    fields,
                    initial_fields: std::collections::HashMap::new(),
                    selected_idx: 0,
                    entity_iid: 0,
                    entity_kind: crate::app::EditEntityKind::CreateMilestone,
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                    workflow_inputs: vec![],
                    cursor_pos: 0,
                    editing: false,
                    desc_scroll: 0,
                });
            }
            _ if keybinding_matches(
                &app.config.keybindings.milestones.edit_milestone,
                key_event,
            ) =>
            {
                if let Some(selected_idx) = app.milestones.state.selected() {
                    let is_github = app.is_github();
                    let milestone_opt: Option<crate::domain::milestones::Milestone> = app
                        .filtered_milestones()
                        .get(selected_idx)
                        .map(|m| (*m).clone());
                    if let Some(m) = milestone_opt {
                        let issues: Option<Vec<crate::domain::issues::Issue>> = app
                            .selected_milestone_issues
                            .clone()
                            .or_else(|| app.milestone_issues_cache.get(&m.iid).cloned());
                        let issues_ref: Option<&[crate::domain::issues::Issue]> = issues.as_deref();
                        let progress = app.milestone_progress_cache.get(&m.iid).copied();
                        let mut doc = crate::entity_editor::build_milestone_document(
                            &m, issues_ref, progress, is_github,
                        );
                        doc.fields.push(crate::app::Field::text(
                            "Description",
                            m.description.clone().unwrap_or_default(),
                        ));
                        app.open_edit_menu(crate::app::EditMenu {
                            title: format!("Edit Milestone %{}", m.iid),
                            entity_project: m.project_path.clone(),
                            fields: doc.fields,
                            initial_fields: std::collections::HashMap::new(),
                            selected_idx: 0,
                            entity_iid: m.iid,
                            entity_kind: crate::app::EditEntityKind::EditMilestone,
                            state: {
                                let mut s = ratatui::widgets::ListState::default();
                                s.select(Some(0));
                                s
                            },
                            workflow_inputs: vec![],
                            cursor_pos: 0,
                            editing: false,
                            desc_scroll: 0,
                        });
                    }
                }
            }
            _ if keybinding_matches(
                &app.config.keybindings.milestones.close_milestone,
                key_event,
            ) =>
            {
                if let Some(selected_idx) = app.milestones.state.selected() {
                    let filtered = app.filtered_milestones();
                    if let Some(milestone) = filtered.get(selected_idx) {
                        app.submit_dialog = Some(crate::app::SubmitDialog::build_with_project(
                            crate::app::ConfirmAction::CloseMilestone(milestone.iid),
                            app.scope.project_of(&milestone.project_path).to_string(),
                            app,
                        ));
                    }
                }
            }
            _ if keybinding_matches(
                &app.config.keybindings.milestones.reopen_milestone,
                key_event,
            ) =>
            {
                if let Some(selected_idx) = app.milestones.state.selected() {
                    let filtered = app.filtered_milestones();
                    if let Some(milestone) = filtered.get(selected_idx) {
                        app.submit_dialog = Some(crate::app::SubmitDialog::build_with_project(
                            crate::app::ConfirmAction::ReopenMilestone(milestone.iid),
                            app.scope.project_of(&milestone.project_path).to_string(),
                            app,
                        ));
                    }
                }
            }
            _ if keybinding_matches(
                &app.config.keybindings.milestones.delete_milestone,
                key_event,
            ) =>
            {
                if let Some(selected_idx) = app.milestones.state.selected() {
                    let filtered = app.filtered_milestones();
                    if let Some(milestone) = filtered.get(selected_idx) {
                        app.submit_dialog = Some(crate::app::SubmitDialog::build_with_project(
                            crate::app::ConfirmAction::DeleteMilestone(milestone.iid),
                            app.scope.project_of(&milestone.project_path).to_string(),
                            app,
                        ));
                    }
                }
            }
            _ if keybinding_matches(
                &app.config.keybindings.milestones.open_in_browser,
                key_event,
            ) =>
            {
                if let Some(selected_idx) = app.milestones.state.selected() {
                    let filtered = app.filtered_milestones();
                    if let Some(milestone) = filtered.get(selected_idx) {
                        let Some(client) = app.gitlab_client.clone() else {
                            return false;
                        };
                        let project_path = app.scope.as_str().to_string();
                        let mid_str = milestone.iid.to_string();
                        let tx2 = tx.clone();

                        tokio::spawn(async move {
                            let result = client
                                .open_milestone_in_browser(&project_path, &mid_str)
                                .await;
                            let _ = tx2.send(Event::CommandCompleted(
                                crate::app::Tab::Milestones,
                                result.map_err(|e| e.to_string()),
                            ));
                        });
                    }
                }
            }
            _ => handled = false,
        },
        crate::app::Tab::Branches => {
            if let Some(branch) = app.branches.cursor_row() {
                let branch_name = branch.name.clone();
                if keybinding_matches(&app.config.keybindings.branches.create_branch, key_event) {
                    let create_from = branch_name.clone();
                    let fields = crate::entity_editor::branch_fields(
                        String::new(),
                        create_from,
                        if app.scope.is_group() {
                            Some(String::new())
                        } else {
                            None
                        },
                    );
                    app.open_edit_menu(crate::app::EditMenu {
                        title: "Create Branch".to_string(),
                        entity_project: if app.scope.is_group() {
                            String::new()
                        } else {
                            app.scope.as_str().to_string()
                        },
                        fields,
                        initial_fields: std::collections::HashMap::new(),
                        selected_idx: 0,
                        entity_iid: 0,
                        entity_kind: crate::app::EditEntityKind::CreateBranch,
                        state: {
                            let mut s = ListState::default();
                            s.select(Some(0));
                            s
                        },
                        workflow_inputs: vec![],
                        cursor_pos: 0,
                        editing: false,
                        desc_scroll: 0,
                    });
                } else if keybinding_matches(
                    &app.config.keybindings.branches.delete_branch,
                    key_event,
                ) {
                    app.submit_dialog = Some(crate::app::SubmitDialog::build(
                        crate::app::ConfirmAction::DeleteBranch(branch_name.clone()),
                        app,
                    ));
                } else if keybinding_matches(
                    &app.config.keybindings.branches.open_in_browser,
                    key_event,
                ) {
                    let branch_local = branch_name.clone();
                    let client = app.gitlab_client.clone();
                    let project_path = app.scope.as_str().to_string();
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        let Some(client) = client else {
                            return;
                        };
                        let result = client
                            .open_branch_in_browser(&project_path, &branch_local)
                            .await;
                        let _ = tx2.send(Event::CommandCompleted(
                            crate::app::Tab::Branches,
                            result.map_err(|e| e.to_string()),
                        ));
                    });
                } else if keybinding_matches(
                    &app.config.keybindings.branches.copy_branch,
                    key_event,
                ) {
                    if let Err(error) = app.copy_selected_branch_name() {
                        app.show_error(format!("Failed to copy branch name: {error}"));
                    }
                } else {
                    handled = false;
                }
            } else {
                handled = false;
            }
        }
        crate::app::Tab::Environments => {
            handled = handle_environments_key(app, key_event, &tx);
        }
        crate::app::Tab::Terminal => {
            handled = crate::handlers::terminal::handle_tab_key(app, key_event);
        }
    }

    if !handled {
        let mut detail_scrolled = true;
        if app.detail_visible
            && (keybinding_matches(&app.config.keybindings.global.scroll_down, key_event)
                || key_event.code == KeyCode::Char('J'))
        {
            app.detail_scroll = app.detail_scroll.saturating_add(1);
        } else if app.detail_visible
            && (keybinding_matches(&app.config.keybindings.global.scroll_up, key_event)
                || key_event.code == KeyCode::Char('K'))
        {
            app.detail_scroll = app.detail_scroll.saturating_sub(1);
        } else if app.detail_visible
            && keybinding_matches(&app.config.keybindings.global.scroll_to_end, key_event)
        {
            // Only the flag — the last line's index is the render pass's
            // `max`, which no handler knows. `settle_detail_scroll` resolves
            // it in the same frame.
            app.detail_scroll_to_bottom = true;
        } else if app.detail_visible
            && crate::keybinding::matches_with_pending(
                &app.config.keybindings.global.scroll_top,
                pending,
                key_event,
            )
        {
            // `scroll_top` jumps to the first line directly: 0 is a known
            // index, no flag round-trip needed.
            app.detail_scroll = 0;
        } else if !apply_page_scroll(app, None, key_event)
            && !apply_list_half_page(app, key_event, &tx)
        {
            detail_scrolled = false;
        }

        match key_event.code {
            KeyCode::Char('?') | KeyCode::F(1) => {
                app.show_help = true;
            }
            KeyCode::Char('u') if key_event.modifiers.is_empty() => {
                app.error_message = Some("Checking for updates...".to_string());
                let tx = tx.clone();
                tokio::spawn(async move {
                    match crate::utils::update::perform_self_update().await {
                        Ok(true) => {
                            let _ = tx.send(Event::Notice(
                                "Update complete! Please restart glab-tui.".to_string(),
                            ));
                        }
                        Ok(false) => {
                            let _ = tx.send(Event::Notice("Already up to date.".to_string()));
                        }
                        Err(e) => {
                            let _ = tx.send(Event::CommandCompleted(
                                crate::app::Tab::Todos,
                                Err(format!("Update failed: {e}")),
                            ));
                        }
                    }
                });
            }
            KeyCode::Char('q') => {
                if app.details_zoomed {
                    app.details_zoomed = false;
                } else if app.detail_visible {
                    app.detail_visible = false;
                } else {
                    app.quit();
                }
            }
            KeyCode::Esc | KeyCode::Backspace => {
                if app.clear_selections() {
                    app.status_message = None;
                } else if app.job_trace_loading {
                    app.job_trace_loading = false;
                    app.pending_job_trace_editor = None;
                    app.pending_job_trace_failure = None;
                } else if app.details_zoomed {
                    app.details_zoomed = false;
                    app.job_trace = None;
                } else if app.detail_visible {
                    app.detail_visible = false;
                } else if app.active_tab == crate::app::Tab::Jobs {
                    if app.job_trace.is_some() {
                        app.job_trace = None;
                    } else {
                        app.active_tab = crate::app::Tab::Pipelines;
                    }
                } else if app.active_tab == crate::app::Tab::Pipelines && app.nav_depth() > 0 {
                    // Inside a descent Esc means the level you can see, not a
                    // job list left over from an earlier visit.
                    app.ascend();
                } else if app.active_tab == crate::app::Tab::Pipelines && !app.jobs.items.is_empty()
                {
                    if app.job_trace.is_some() {
                        app.job_trace = None;
                    } else {
                        app.jobs.items.clear();
                        app.jobs.state.select(None);
                        app.selected_jobs.clear();
                    }
                } else if !app.search_query.is_empty() {
                    // Last-resort Esc action: drop the active filter. The
                    // previous Esc press already exited the search input
                    // box (handled by the is_typing_search match arm above),
                    // so this press clears the query that was keeping the
                    // table filtered.
                    app.clear_search_query();
                }
            }
            KeyCode::Char('f') if key_event.modifiers.is_empty() => {
                app.is_typing_search = true;
            }
            KeyCode::Enter => match app.active_tab {
                crate::app::Tab::Todos => crate::handlers::todos::open_selected(app, &tx),
                crate::app::Tab::Pipelines => {
                    if let Some(idx) = app.pipelines.state.selected() {
                        let pipe_info = app.filtered_pipelines().get(idx).map(|p| {
                            // A group-scoped list mixes projects, so the row's
                            // own path wins over the scope it was listed under.
                            (p.id(), app.scope.project_of(&p.project_path).to_string())
                        });
                        if let Some((pipeline_id, project_context)) = pipe_info {
                            spawn_open_pipeline(
                                app,
                                &tx,
                                pipeline_id,
                                project_context,
                                PipelineOpening::Descend,
                            );
                        }
                    }
                }
                crate::app::Tab::Jobs => {
                    if app.job_trace.is_some() {
                        app.details_zoomed = !app.details_zoomed;
                    } else if let Some(idx) = app.jobs.state.selected() {
                        let job_id = app.filtered_jobs().get(idx).map(|j| j.id());
                        if let Some(job_id) = job_id {
                            spawn_job_trace_fetch(app, &tx, job_id);
                        }
                    }
                }
                _ => {
                    if !app.detail_visible {
                        app.detail_visible = true;
                        app.details_zoomed = false;
                    } else if !app.details_zoomed {
                        app.details_zoomed = true;
                    } else {
                        // Already zoomed in! Second Enter enters edit mode
                        match app.active_tab {
                            crate::app::Tab::Issues => {
                                if let Some(selected_idx) = app.issues.state.selected() {
                                    let filtered = app.filtered_issues();
                                    if let Some(issue) = filtered.get(selected_idx) {
                                        let is_github = app.is_github();
                                        let mut doc = crate::entity_editor::build_issue_document(
                                            issue,
                                            is_github,
                                            app.fetching_related_mrs.contains(&issue.iid),
                                        );
                                        doc.fields.push(crate::app::Field::text(
                                            "Description",
                                            issue.description.clone().unwrap_or_default(),
                                        ));
                                        app.open_edit_menu(crate::app::EditMenu {
                                            title: format!("Edit Issue #{}", issue.iid),
                                            entity_project: issue.project_path.clone(),
                                            fields: doc.fields,
                                            initial_fields: std::collections::HashMap::new(),
                                            selected_idx: 0,
                                            entity_iid: issue.iid,
                                            entity_kind: crate::app::EditEntityKind::EditIssue,
                                            state: {
                                                let mut s = ListState::default();
                                                s.select(Some(0));
                                                s
                                            },
                                            workflow_inputs: vec![],
                                            cursor_pos: 0,
                                            editing: false,
                                            desc_scroll: 0,
                                        });
                                    }
                                }
                            }
                            crate::app::Tab::MergeRequests => {
                                if let Some(selected_idx) = app.mrs.state.selected() {
                                    let filtered = app.filtered_mrs();
                                    if let Some(mr) = filtered.get(selected_idx) {
                                        let is_github = app.is_github();
                                        let pr_suffix = if is_github { "PR" } else { "MR" };
                                        let unresolved = if app.diff_view.as_ref().map(|d| d.mr_iid)
                                            == Some(mr.iid)
                                        {
                                            Some(app.unresolved_threads_count())
                                        } else {
                                            None
                                        };
                                        let is_fetching_issues =
                                            app.fetching_mr_related_issues.contains(&mr.iid);
                                        let mut doc = crate::entity_editor::build_mr_document(
                                            mr,
                                            is_github,
                                            unresolved,
                                            is_fetching_issues,
                                        );
                                        doc.fields.push(crate::app::Field::text(
                                            "Description",
                                            mr.description.clone().unwrap_or_default(),
                                        ));
                                        app.open_edit_menu(crate::app::EditMenu {
                                            title: format!("Edit {} #{}", pr_suffix, mr.iid),
                                            entity_project: mr.project_path.clone(),
                                            fields: doc.fields,
                                            initial_fields: std::collections::HashMap::new(),
                                            selected_idx: 0,
                                            entity_iid: mr.iid,
                                            entity_kind: crate::app::EditEntityKind::EditMr,
                                            state: {
                                                let mut s = ListState::default();
                                                s.select(Some(0));
                                                s
                                            },
                                            workflow_inputs: vec![],
                                            cursor_pos: 0,
                                            editing: false,
                                            desc_scroll: 0,
                                        });
                                    }
                                }
                            }
                            crate::app::Tab::Milestones => {
                                if let Some(selected_idx) = app.milestones.state.selected() {
                                    let filtered = app.filtered_milestones();
                                    if let Some(m) = filtered.get(selected_idx) {
                                        let is_github = app.is_github();
                                        let issues = app
                                            .selected_milestone_issues
                                            .as_deref()
                                            .or_else(|| {
                                                app.milestone_issues_cache
                                                    .get(&m.iid)
                                                    .map(|v| v.as_slice())
                                            });
                                        let progress =
                                            app.milestone_progress_cache.get(&m.iid).copied();
                                        let mut doc =
                                            crate::entity_editor::build_milestone_document(
                                                m, issues, progress, is_github,
                                            );
                                        doc.fields.push(crate::app::Field::text(
                                            "Description",
                                            m.description.clone().unwrap_or_default(),
                                        ));
                                        app.open_edit_menu(crate::app::EditMenu {
                                            title: format!("Edit Milestone %{}", m.iid),
                                            entity_project: m.project_path.clone(),
                                            fields: doc.fields,
                                            initial_fields: std::collections::HashMap::new(),
                                            selected_idx: 0,
                                            entity_iid: m.iid,
                                            entity_kind: crate::app::EditEntityKind::EditMilestone,
                                            state: {
                                                let mut s = ListState::default();
                                                s.select(Some(0));
                                                s
                                            },
                                            workflow_inputs: vec![],
                                            cursor_pos: 0,
                                            editing: false,
                                            desc_scroll: 0,
                                        });
                                    }
                                }
                            }
                            crate::app::Tab::Releases => {
                                if let Some(release) = app.releases.cursor_row().cloned() {
                                    crate::entity_editor::open_release_edit_menu(app, &release);
                                }
                            }
                            _ => {
                                app.details_zoomed = false;
                            }
                        }
                    }
                }
            },
            _ if (key_event.code == KeyCode::Right
                || key_event.code == KeyCode::Char('l')
                || keybinding_matches(&app.config.keybindings.global.next_tab, key_event)) =>
            {
                app.next_tab();
                if let Some(client) = &app.gitlab_client {
                    if !app.loading_tabs.contains(&app.active_tab)
                        && !app.refreshed_tabs.contains(&app.active_tab)
                    {
                        if !app.loaded_tabs.contains(&app.active_tab) {
                            app.loading_tabs.insert(app.active_tab);
                        }
                        spawn_refresh_active_tab(
                            client,
                            &app.scope,
                            app.requested_state_queries(),
                            app.active_tab,
                            app.scoped_sender(tx.clone()),
                        );
                    }
                }
                if app.active_tab == crate::app::Tab::Issues {
                    maybe_fetch_related_mrs(app, &tx);
                }
                if app.active_tab == crate::app::Tab::MergeRequests {
                    maybe_fetch_mr_related_issues(app, &tx);
                }
            }
            _ if (key_event.code == KeyCode::Left
                || key_event.code == KeyCode::Char('h')
                || keybinding_matches(&app.config.keybindings.global.prev_tab, key_event)) =>
            {
                app.previous_tab();
                if let Some(client) = &app.gitlab_client {
                    if !app.loading_tabs.contains(&app.active_tab)
                        && !app.refreshed_tabs.contains(&app.active_tab)
                    {
                        if !app.loaded_tabs.contains(&app.active_tab) {
                            app.loading_tabs.insert(app.active_tab);
                        }
                        spawn_refresh_active_tab(
                            client,
                            &app.scope,
                            app.requested_state_queries(),
                            app.active_tab,
                            app.scoped_sender(tx.clone()),
                        );
                    }
                }
                if app.active_tab == crate::app::Tab::Issues {
                    maybe_fetch_related_mrs(app, &tx);
                }
                if app.active_tab == crate::app::Tab::MergeRequests {
                    maybe_fetch_mr_related_issues(app, &tx);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if app.details_zoomed {
                    app.detail_scroll = app.detail_scroll.saturating_add(1);
                } else {
                    app.detail_scroll = 0;
                    match app.active_tab {
                        crate::app::Tab::Issues => {
                            app.issues.next(app.filtered_issues().len());
                        }
                        crate::app::Tab::MergeRequests => {
                            app.mrs.next(app.filtered_mrs().len());
                        }
                        crate::app::Tab::Pipelines => {
                            app.pipelines.next(app.filtered_pipelines().len());
                        }
                        crate::app::Tab::Jobs => {
                            let len = app.filtered_jobs().len();
                            app.jobs.next(len);
                            app.job_trace = None;
                            app.job_trace_follow = false;
                        }
                        crate::app::Tab::Runners
                        | crate::app::Tab::Releases
                        | crate::app::Tab::Branches
                        | crate::app::Tab::Environments => {
                            if let Some(list) = app.list_navigation_mut(app.active_tab) {
                                list.next();
                            }
                        }
                        crate::app::Tab::Todos => {
                            app.todos.next(app.filtered_todos().len());
                        }
                        crate::app::Tab::Milestones => {
                            app.milestones.next(app.filtered_milestones().len());
                        }
                        crate::app::Tab::Terminal => {
                            app.terminal_scroll = app.terminal_scroll.saturating_sub(1);
                        }
                    }
                    if app.select_mode {
                        app.update_visual_selection();
                    }
                    if app.active_tab == crate::app::Tab::Issues {
                        maybe_fetch_related_mrs(app, &tx);
                    }
                    if app.active_tab == crate::app::Tab::MergeRequests {
                        maybe_fetch_mr_related_issues(app, &tx);
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if app.details_zoomed {
                    app.detail_scroll = app.detail_scroll.saturating_sub(1);
                } else {
                    app.detail_scroll = 0;
                    match app.active_tab {
                        crate::app::Tab::Issues => {
                            app.issues.previous(app.filtered_issues().len());
                        }
                        crate::app::Tab::MergeRequests => {
                            app.mrs.previous(app.filtered_mrs().len());
                        }
                        crate::app::Tab::Pipelines => {
                            app.pipelines.previous(app.filtered_pipelines().len());
                        }
                        crate::app::Tab::Jobs => {
                            let len = app.filtered_jobs().len();
                            app.jobs.previous(len);
                            app.job_trace = None;
                            app.job_trace_follow = false;
                        }
                        crate::app::Tab::Runners
                        | crate::app::Tab::Releases
                        | crate::app::Tab::Branches
                        | crate::app::Tab::Environments => {
                            if let Some(list) = app.list_navigation_mut(app.active_tab) {
                                list.previous();
                            }
                        }
                        crate::app::Tab::Todos => {
                            app.todos.previous(app.filtered_todos().len());
                        }
                        crate::app::Tab::Milestones => {
                            app.milestones.previous(app.filtered_milestones().len());
                        }
                        crate::app::Tab::Terminal => {
                            app.terminal_scroll = app.terminal_scroll.saturating_add(1);
                        }
                    }
                    if app.select_mode {
                        app.update_visual_selection();
                    }
                    if app.active_tab == crate::app::Tab::Issues {
                        maybe_fetch_related_mrs(app, &tx);
                    }
                    if app.active_tab == crate::app::Tab::MergeRequests {
                        maybe_fetch_mr_related_issues(app, &tx);
                    }
                }
            }
            KeyCode::Home => {
                if app.details_zoomed {
                    app.detail_scroll = 0;
                } else {
                    app.detail_scroll = 0;
                    match app.active_tab {
                        crate::app::Tab::Issues => {
                            app.issues.first(app.filtered_issues().len());
                        }
                        crate::app::Tab::MergeRequests => {
                            app.mrs.first(app.filtered_mrs().len());
                        }
                        crate::app::Tab::Pipelines => {
                            app.pipelines.first(app.filtered_pipelines().len());
                        }
                        crate::app::Tab::Jobs => {
                            let len = app.filtered_jobs().len();
                            app.jobs.first(len);
                            app.job_trace = None;
                            app.job_trace_follow = false;
                        }
                        crate::app::Tab::Runners
                        | crate::app::Tab::Releases
                        | crate::app::Tab::Branches
                        | crate::app::Tab::Environments => {
                            if let Some(list) = app.list_navigation_mut(app.active_tab) {
                                list.first();
                            }
                        }
                        crate::app::Tab::Todos => {
                            app.todos.first(app.filtered_todos().len());
                        }
                        crate::app::Tab::Milestones => {
                            app.milestones.first(app.filtered_milestones().len());
                        }
                        crate::app::Tab::Terminal => {
                            app.terminal_scroll = usize::MAX;
                        }
                    }
                    if app.select_mode {
                        app.update_visual_selection();
                    }
                    if app.active_tab == crate::app::Tab::Issues {
                        maybe_fetch_related_mrs(app, &tx);
                    }
                    if app.active_tab == crate::app::Tab::MergeRequests {
                        maybe_fetch_mr_related_issues(app, &tx);
                    }
                }
            }
            KeyCode::End => {
                if app.details_zoomed {
                    // Only the flag — the last line's index is the render
                    // pass's `max`, which no handler knows.
                    // `settle_detail_scroll` resolves it in the same frame.
                    app.detail_scroll_to_bottom = true;
                } else {
                    app.detail_scroll = 0;
                    match app.active_tab {
                        crate::app::Tab::Issues => {
                            app.issues.last(app.filtered_issues().len());
                        }
                        crate::app::Tab::MergeRequests => {
                            app.mrs.last(app.filtered_mrs().len());
                        }
                        crate::app::Tab::Pipelines => {
                            app.pipelines.last(app.filtered_pipelines().len());
                        }
                        crate::app::Tab::Jobs => {
                            let len = app.filtered_jobs().len();
                            app.jobs.last(len);
                            app.job_trace = None;
                            app.job_trace_follow = false;
                        }
                        crate::app::Tab::Runners
                        | crate::app::Tab::Releases
                        | crate::app::Tab::Branches
                        | crate::app::Tab::Environments => {
                            if let Some(list) = app.list_navigation_mut(app.active_tab) {
                                list.last();
                            }
                        }
                        crate::app::Tab::Todos => {
                            app.todos.last(app.filtered_todos().len());
                        }
                        crate::app::Tab::Milestones => {
                            app.milestones.last(app.filtered_milestones().len());
                        }
                        crate::app::Tab::Terminal => {
                            app.terminal_scroll = 0;
                        }
                    }
                    if app.select_mode {
                        app.update_visual_selection();
                    }
                    if app.active_tab == crate::app::Tab::Issues {
                        maybe_fetch_related_mrs(app, &tx);
                    }
                    if app.active_tab == crate::app::Tab::MergeRequests {
                        maybe_fetch_mr_related_issues(app, &tx);
                    }
                }
            }
            _ if !detail_scrolled => {
                handled = super::custom_commands::run_bound_command(app, key_event, terminal, &tx);
            }
            _ => {}
        }
    }

    handled
}

/// Switch to the Merge Requests tab, drop the filters that would hide the
/// target, and focus the given MR/PR. If the MR is already loaded, focus it
/// immediately; otherwise set `pending_mr_select` so the `Event::MrsFetched`
/// handler in `main.rs` can focus it once the in-flight tab refresh completes.
pub(crate) fn jump_to_mr_tab(
    app: &mut crate::app::App,
    mr_iid: u64,
    client: Option<crate::domain::client::GitlabClient>,
    tx: tokio::sync::mpsc::UnboundedSender<crate::event::Event>,
) {
    if !app.focus_mr(None, mr_iid) {
        app.pending_mr_select = Some(mr_iid);
    }
    app.detail_scroll = 0;
    if let Some(client) = client {
        crate::fetch::spawn_refresh_active_tab(
            &client,
            &app.scope,
            app.requested_state_queries(),
            crate::app::Tab::MergeRequests,
            app.scoped_sender(tx),
        );
    } else {
        app.show_error("No backend client available to load Merge Requests.".to_string());
    }
}

/// Public entry point used by the related-MRs selector.
pub fn jump_to_mr_tab_from_selector(
    app: &mut crate::app::App,
    mr_iid: u64,
    tx: tokio::sync::mpsc::UnboundedSender<crate::event::Event>,
    client: &crate::domain::client::GitlabClient,
) {
    jump_to_mr_tab(app, mr_iid, Some(client.clone()), tx);
}

/// How a pipeline was opened from the Pipelines tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineOpening {
    /// Only the pipeline's own jobs. None of them leaves the user on the
    /// Pipelines tab instead of stranding them on an empty Jobs tab.
    OwnJobs,
    /// The level below the pipeline: its downstream pipelines, or for a leaf
    /// pipeline its own jobs, entered even when there are none.
    Descend,
}

/// What lies below a pipeline opened from the Pipelines tab.
#[derive(Clone, Debug)]
pub enum PipelineContents {
    Downstream(crate::domain::pipelines::ChildLevel),
    Jobs(Vec<crate::domain::pipelines::Job>),
}

/// Fetches what lies below `pipeline_id` off the event loop and reports it as
/// `Event::PipelineOpened`, applied by `apply_opened_pipeline`.
fn spawn_open_pipeline(
    app: &mut App,
    tx: &UnboundedSender<Event>,
    pipeline_id: u64,
    project_context: String,
    opening: PipelineOpening,
) {
    let Some(client) = app.gitlab_client.clone() else {
        app.show_error("Failed to fetch jobs".to_string());
        return;
    };
    app.loading_tabs.insert(crate::app::Tab::Jobs);
    let tx = app.scoped_sender(tx.clone());
    tokio::spawn(async move {
        let result = fetch_pipeline_contents(&client, &project_context, pipeline_id, opening).await;
        let _ = tx.send(Event::PipelineOpened {
            pipeline_id,
            opening,
            result,
        });
    });
}

async fn fetch_pipeline_contents(
    client: &crate::domain::client::GitlabClient,
    project_context: &str,
    pipeline_id: u64,
    opening: PipelineOpening,
) -> Result<PipelineContents, String> {
    if opening == PipelineOpening::Descend {
        // Never fall back to the jobs path on failure: a failed fetch would
        // read as "no downstream pipelines", which is the silent emptiness
        // this drill-down exists to remove.
        let bridges =
            crate::domain::pipelines::list_pipeline_bridges(client, project_context, pipeline_id)
                .await
                .map_err(|_| "Failed to fetch downstream pipelines".to_string())?;
        if !bridges.is_empty() {
            return Ok(PipelineContents::Downstream(
                crate::domain::pipelines::bridges_to_level(bridges),
            ));
        }
    }
    crate::domain::pipelines::list_pipeline_jobs(client, project_context, pipeline_id)
        .await
        .map(PipelineContents::Jobs)
        .map_err(|_| "Failed to fetch jobs".to_string())
}

/// Applies an `Event::PipelineOpened` reply. A reply for a pipeline that is no
/// longer listed on the Pipelines tab (the user moved on while it was in
/// flight) only caches the jobs, so it never yanks the user to another view.
pub fn apply_opened_pipeline(
    app: &mut App,
    pipeline_id: u64,
    opening: PipelineOpening,
    result: Result<PipelineContents, String>,
) {
    app.loading_tabs.remove(&crate::app::Tab::Jobs);
    let contents = match result {
        Ok(contents) => contents,
        Err(message) => {
            app.show_error(message);
            return;
        }
    };
    let is_still_listed = app.active_tab == crate::app::Tab::Pipelines
        && app.pipelines.items.iter().any(|p| p.id() == pipeline_id);
    match contents {
        PipelineContents::Downstream(level) => {
            if is_still_listed {
                app.descend_into(pipeline_id, level);
            }
        }
        PipelineContents::Jobs(jobs) => {
            app.pipeline_jobs.insert(pipeline_id, jobs.clone());
            if !is_still_listed {
                return;
            }
            let has_jobs = !jobs.is_empty();
            app.jobs.items = jobs;
            app.active_pipeline_project = app.pipeline_project(pipeline_id).map(str::to_owned);
            app.active_pipeline_id = Some(pipeline_id);
            app.jobs.state.select(Some(0));
            app.detail_scroll = 0;
            app.job_trace = None;
            if has_jobs || opening == PipelineOpening::Descend {
                app.active_tab = crate::app::Tab::Jobs;
            } else {
                app.show_error(format!("Pipeline {pipeline_id} has no jobs of its own"));
            }
        }
    }
}

/// Switch to the Issues tab, drop the filters that would hide the target, and
/// focus the given Issue. If the Issue is already loaded, focus it
/// immediately; otherwise set `pending_issue_select` so the
/// `Event::IssuesFetched` handler in `main.rs` can focus it once the in-flight
/// tab refresh completes.
pub(crate) fn jump_to_issue_tab(
    app: &mut crate::app::App,
    issue_iid: u64,
    client: Option<crate::domain::client::GitlabClient>,
    tx: tokio::sync::mpsc::UnboundedSender<crate::event::Event>,
) {
    if !app.focus_issue(None, issue_iid) {
        app.pending_issue_select = Some(issue_iid);
    }
    app.detail_scroll = 0;
    if let Some(client) = client {
        crate::fetch::spawn_refresh_active_tab(
            &client,
            &app.scope,
            app.requested_state_queries(),
            crate::app::Tab::Issues,
            app.scoped_sender(tx),
        );
    } else {
        app.show_error("No backend client available to load Issues.".to_string());
    }
}

/// Public entry point used by the linked-issues selector.
pub fn jump_to_issue_tab_from_selector(
    app: &mut crate::app::App,
    issue_iid: u64,
    tx: tokio::sync::mpsc::UnboundedSender<crate::event::Event>,
    client: &crate::domain::client::GitlabClient,
) {
    jump_to_issue_tab(app, issue_iid, Some(client.clone()), tx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::event::ScopeGeneration;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use std::collections::{HashMap, HashSet};

    #[tokio::test]
    async fn failed_bulk_mutation_reverts_only_failed_rows_and_reports_once() {
        let (raw_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tx = ScopedSender::new(raw_tx, ScopeGeneration::default());
        let rows = vec![
            (1, Some("failed".to_string())),
            (2, Some("success".to_string())),
            (3, Some("failed".to_string())),
        ];

        run_optimistic_mutation(
            OptimisticTable::Pipelines,
            "retry pipeline",
            rows,
            &tx,
            |id| async move {
                if id == 2 {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("offline"))
                }
            },
        )
        .await;
        drop(tx);

        let mut reverted = Vec::new();
        let mut errors = Vec::new();
        while let Some(event) = rx.recv().await {
            match event
                .for_scope(ScopeGeneration::default())
                .expect("event from the current scope")
            {
                Event::PipelineStatusRevert {
                    pipeline_id,
                    status,
                } => reverted.push((pipeline_id, status)),
                Event::CommandCompleted(Tab::Pipelines, Err(message)) => errors.push(message),
                other => panic!("unexpected event {other:?}"),
            }
        }
        assert_eq!(
            reverted,
            vec![(1, "failed".to_string()), (3, "failed".to_string())]
        );
        assert_eq!(
            errors,
            vec!["Failed to retry pipeline #1: offline (+1 more failed)".to_string()]
        );
    }

    #[tokio::test]
    async fn successful_mutation_sends_nothing() {
        let (raw_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tx = ScopedSender::new(raw_tx, ScopeGeneration::default());
        run_optimistic_mutation(
            OptimisticTable::Jobs,
            "cancel job",
            vec![(7, Some("running".to_string()))],
            &tx,
            |_| async { Ok(()) },
        )
        .await;
        drop(tx);
        assert!(rx.recv().await.is_none());
    }

    #[test]
    fn second_flip_while_in_flight_keeps_the_revert_with_the_first() {
        let mut app = App::default();
        app.pipelines.items = vec![crate::domain::pipelines::Pipeline {
            id: 1,
            status: "failed".to_string(),
            r#ref: "main".to_string(),
            updated_at: String::new(),
            name: String::new(),
            display_title: String::new(),
            event: String::new(),
            head_sha: String::new(),
            actor_login: String::new(),
            duration_seconds: None,
            created_at: None,
            source: None,
            project_path: String::new(),
            web_url: None,
        }];

        let first = flip_pipeline_statuses(&mut app, &[1], "running");
        let second = flip_pipeline_statuses(&mut app, &[1], "running");

        assert_eq!(first, vec![(1, Some("failed".to_string()))]);
        assert_eq!(second, vec![(1, None)]);
        assert_eq!(app.pipelines.items[0].status, "running");
    }

    /// Constructs the real `AppTerminal` and dispatches `key_event` through
    /// `handle_active_tab_key`, discarding any events it sends.
    ///
    /// Uses `Viewport::Fixed` so construction never queries the backend's
    /// terminal size - `cargo test` has no controlling tty in CI.
    async fn dispatch(app: &mut App, key_event: &KeyEvent) {
        dispatch_with_pending(app, key_event, None).await;
    }

    /// Like `dispatch`, but with an explicit `pending` first key, to
    /// exercise the multi-key sequence branch at the top of
    /// `handle_active_tab_key`.
    async fn dispatch_with_pending(app: &mut App, key_event: &KeyEvent, pending: Option<char>) {
        let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
        let options = ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(0, 0, 80, 24)),
        };
        let mut terminal = ratatui::Terminal::with_options(backend, options)
            .expect("terminal construction failed");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        handle_active_tab_key(app, key_event, &mut terminal, tx, pending).await;
    }

    #[tokio::test]
    async fn j_and_k_scroll_the_detail_pane_by_one_line_with_default_config() {
        let mut app = App::default();
        app.detail_visible = true;
        app.detail_scroll = 5;

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT),
        )
        .await;
        assert_eq!(app.detail_scroll, 6);

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT),
        )
        .await;
        assert_eq!(app.detail_scroll, 5);
    }

    #[tokio::test]
    async fn j_and_k_are_ignored_while_the_detail_pane_is_hidden() {
        let mut app = App::default();
        app.detail_visible = false;
        app.detail_scroll = 5;

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT),
        )
        .await;
        assert_eq!(app.detail_scroll, 5);

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT),
        )
        .await;
        assert_eq!(app.detail_scroll, 5);
    }

    /// With `scroll_down` remapped away from the hardcoded "J", both halves
    /// of the merged condition remain reachable: the remapped key and the
    /// hardcoded 'J' fallback each move the pane by exactly one line.
    #[tokio::test]
    async fn remapped_scroll_down_and_hardcoded_j_each_scroll_one_line() {
        let mut app = App::default();
        app.detail_visible = true;
        app.detail_scroll = 5;
        app.config.keybindings.global.scroll_down = "z".to_string();

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
        )
        .await;
        assert_eq!(app.detail_scroll, 6);

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT),
        )
        .await;
        assert_eq!(app.detail_scroll, 7);
    }

    #[tokio::test]
    async fn home_and_end_navigate_table_selection() {
        let mut app = App::default();
        let mk_issue = |iid: u64| crate::domain::issues::Issue {
            iid,
            title: format!("Issue {iid}"),
            state: "opened".to_string(),
            labels: vec![],
            updated_at: String::new(),
            created_at: None,
            closed_at: None,
            author: crate::domain::issues::Author {
                username: "user".to_string(),
            },
            milestone: None,
            assignees: vec![],
            description: None,
            due_date: None,
            web_url: String::new(),
            project_path: String::new(),
            related_mrs: None,
        };

        app.issues.items = vec![mk_issue(1), mk_issue(2), mk_issue(3), mk_issue(4)];
        app.issues.state.select(Some(2));
        app.detail_scroll = 3;

        // End key jumps to last element and resets detail_scroll
        dispatch(&mut app, &KeyEvent::new(KeyCode::End, KeyModifiers::NONE)).await;
        assert_eq!(app.issues.state.selected(), Some(3));
        assert_eq!(app.detail_scroll, 0);

        // Home key jumps to first element
        dispatch(&mut app, &KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)).await;
        assert_eq!(app.issues.state.selected(), Some(0));
        assert_eq!(app.detail_scroll, 0);
    }

    /// `PageDown` advances `detail_scroll` by the detail pane's usable
    /// height (its `rect.height` minus the two border rows) — the convention
    /// `less` and `man` share.
    #[tokio::test]
    async fn page_down_scrolls_detail_pane_by_one_viewport() {
        let mut app = App::default();
        app.detail_visible = true;
        app.detail_rect = Some(ratatui::layout::Rect::new(0, 0, 40, 22));
        app.detail_scroll = 5;

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        )
        .await;
        assert_eq!(app.detail_scroll, 25);
    }

    /// While the Jobs tab shows a log under the job header, a page is the
    /// log pane's height, not the whole detail column's. A log left open on
    /// the Jobs tab does not change paging on other tabs.
    #[tokio::test]
    async fn page_down_on_a_shown_job_log_moves_by_the_log_pane_rows() {
        let mut app = App::default();
        app.active_tab = crate::app::Tab::Jobs;
        app.detail_visible = true;
        app.detail_rect = Some(ratatui::layout::Rect::new(0, 0, 40, 30));
        let mut view = crate::ui::job_trace::JobTraceView::new(
            crate::domain::job_trace::JobTrace::from_read("l\n".repeat(200).as_bytes(), false),
        );
        view.visible_lines(&crate::config::Theme::default(), None, 0, 20);
        app.job_trace = Some(view);
        let page_down = KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE);

        dispatch(&mut app, &page_down).await;
        assert_eq!(app.detail_scroll, 20);

        app.active_tab = crate::app::Tab::Issues;
        dispatch(&mut app, &page_down).await;
        assert_eq!(app.detail_scroll, 48);
    }

    /// `PageUp` is the symmetric `PageDown` — subtracts the same page size.
    #[tokio::test]
    async fn page_up_scrolls_detail_pane_back_by_one_viewport() {
        let mut app = App::default();
        app.detail_visible = true;
        app.detail_rect = Some(ratatui::layout::Rect::new(0, 0, 40, 22));
        app.detail_scroll = 25;

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
        )
        .await;
        assert_eq!(app.detail_scroll, 5);
    }

    /// The `2`-row border means `rect.height < 3` leaves no usable rows, so
    /// `PageUp` must not underflow and `PageDown` must step by exactly one.
    #[tokio::test]
    async fn page_scroll_clamps_to_one_line_for_short_detail_pane() {
        let mut app = App::default();
        app.detail_visible = true;
        app.detail_rect = Some(ratatui::layout::Rect::new(0, 0, 40, 2));
        app.detail_scroll = 0;

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
        )
        .await;
        assert_eq!(app.detail_scroll, 0);

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        )
        .await;
        assert_eq!(app.detail_scroll, 1);
    }

    /// Without a rendered `detail_rect` the handler has no viewport to step
    /// against, so the key must be a no-op rather than subtract from
    /// `detail_scroll` and risk wrapping.
    #[tokio::test]
    async fn page_scroll_keys_do_nothing_without_detail_rect() {
        let mut app = App::default();
        app.detail_visible = true;
        app.detail_rect = None;
        app.detail_scroll = 5;

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        )
        .await;
        assert_eq!(app.detail_scroll, 5);

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
        )
        .await;
        assert_eq!(app.detail_scroll, 5);
    }

    /// `PageDown` must only page-scroll, not also fire a tab handler that
    /// uses the bare key code - `PageDown` is not bound to any tab action
    /// today, so the existing `J`/`K` cascade is the only thing that could
    /// accidentally catch it.
    #[tokio::test]
    async fn page_down_does_not_also_navigate_the_table() {
        let mut app = App::default();
        app.detail_visible = true;
        app.detail_rect = Some(ratatui::layout::Rect::new(0, 0, 40, 22));
        app.issues.items = (1..=4)
            .map(|iid| crate::domain::issues::Issue {
                iid,
                title: format!("Issue {iid}"),
                state: "opened".to_string(),
                labels: vec![],
                updated_at: String::new(),
                created_at: None,
                closed_at: None,
                author: crate::domain::issues::Author {
                    username: "user".to_string(),
                },
                milestone: None,
                assignees: vec![],
                description: None,
                due_date: None,
                web_url: String::new(),
                project_path: String::new(),
                related_mrs: None,
            })
            .collect();
        app.issues.state.select(Some(0));
        app.detail_scroll = 5;

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        )
        .await;

        assert_eq!(app.issues.state.selected(), Some(0));
        assert_eq!(app.detail_scroll, 25);
    }

    /// `Ctrl+d` must not cancel the selected pipeline - the bare `d` arm
    /// matches on character alone and previously swallowed the Ctrl form,
    /// blocking any future vim half-page binding.
    #[tokio::test]
    async fn ctrl_d_does_not_cancel_the_selected_pipeline() {
        use crate::domain::pipelines::Pipeline;

        let mut app = App::default();
        app.active_tab = crate::app::Tab::Pipelines;
        let pipe = Pipeline {
            id: 42,
            status: "running".to_string(),
            r#ref: String::new(),
            updated_at: String::new(),
            name: String::new(),
            display_title: String::new(),
            event: String::new(),
            head_sha: String::new(),
            actor_login: String::new(),
            duration_seconds: None,
            created_at: None,
            source: None,
            project_path: String::new(),
            web_url: None,
        };
        app.pipelines.items = vec![pipe];
        app.pipelines.state.select(Some(0));

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        )
        .await;

        assert_eq!(
            app.pipelines.items[0].status, "running",
            "Ctrl+d must not flip the pipeline status to canceled"
        );
    }

    #[tokio::test]
    async fn d_on_a_pipeline_asks_for_confirmation_before_cancelling() {
        use crate::domain::pipelines::Pipeline;

        let mut app = App::default();
        app.active_tab = crate::app::Tab::Pipelines;
        app.pipelines.items = vec![Pipeline {
            id: 42,
            status: "running".to_string(),
            r#ref: String::new(),
            updated_at: String::new(),
            name: String::new(),
            display_title: String::new(),
            event: String::new(),
            head_sha: String::new(),
            actor_login: String::new(),
            duration_seconds: None,
            created_at: None,
            source: None,
            project_path: String::new(),
            web_url: None,
        }];
        app.pipelines.state.select(Some(0));

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
        )
        .await;

        let dialog = app.submit_dialog.as_ref().expect("a confirmation opens");
        assert!(matches!(
            dialog.action,
            crate::app::ConfirmAction::CancelPipeline(42)
        ));
        assert_eq!(app.pipelines.items[0].status, "running");
    }

    #[tokio::test]
    async fn cancelling_selected_jobs_asks_for_confirmation_first() {
        use crate::domain::pipelines::Job;

        let job = |id| Job {
            id,
            status: "running".to_string(),
            stage: "test".to_string(),
            name: format!("job-{id}"),
            matrix: None,
            duration_seconds: None,
            runner: None,
            ..Default::default()
        };
        let mut app = App::default();
        app.active_tab = crate::app::Tab::Jobs;
        app.active_pipeline_id = Some(7);
        app.jobs.items = vec![job(3), job(1), job(2)];
        app.jobs.state.select(Some(0));
        app.selected_jobs.extend([3, 1]);

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
        )
        .await;

        let dialog = app.submit_dialog.as_ref().expect("a confirmation opens");
        match &dialog.action {
            crate::app::ConfirmAction::CancelJobs {
                pipeline_id,
                job_ids,
            } => {
                assert_eq!(*pipeline_id, 7);
                assert_eq!(job_ids, &vec![1, 3]);
            }
            other => panic!("expected CancelJobs, got {other:?}"),
        }
        assert!(app.jobs.items.iter().all(|j| j.status == "running"));
        assert_eq!(app.selected_jobs.len(), 2);
    }

    /// `Ctrl+u` must not start a self-update check.
    #[tokio::test]
    async fn ctrl_u_does_not_trigger_a_self_update() {
        let mut app = App::default();
        app.error_message = None;

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        )
        .await;

        assert!(
            app.error_message.is_none()
                || !app
                    .error_message
                    .as_deref()
                    .unwrap_or("")
                    .contains("Checking for updates"),
            "Ctrl+u must not raise the 'Checking for updates' message",
        );
    }

    /// `Ctrl+f` must not open the inline search bar.
    #[tokio::test]
    async fn ctrl_f_does_not_open_inline_search() {
        let mut app = App::default();
        app.is_typing_search = false;

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        )
        .await;

        assert!(
            !app.is_typing_search,
            "Ctrl+f must not flip is_typing_search"
        );
    }

    /// The handler must not write `detail_scroll` itself: the last line's
    /// index is the render pass's `max`, which no handler knows. It raises
    /// the flag and `settle_detail_scroll` resolves it in the same frame.
    ///
    /// The hardcoded `End` arm runs in the same dispatch and resets
    /// `detail_scroll` to `0` (the existing #492 behaviour that jumps the
    /// table to its last row), so the post-dispatch value is `0` here. The
    /// render pass, not the test, is what consumes the flag and snaps the
    /// scroll to the pane's actual last line — that path is covered by the
    /// `render_tab_*_clamps_*` tests in `ui::tabs`.
    #[tokio::test]
    async fn scroll_to_end_raises_the_jump_flag() {
        let mut app = App::default();
        app.detail_visible = true;
        app.detail_scroll = 5;
        app.config.keybindings.global.scroll_to_end = "End".to_string();

        dispatch(&mut app, &KeyEvent::new(KeyCode::End, KeyModifiers::NONE)).await;

        assert!(app.detail_scroll_to_bottom);
    }

    /// `scroll_to_end` is a detail-pane action; without a visible pane it
    /// has nothing to scroll.
    #[tokio::test]
    async fn scroll_to_end_is_ignored_while_the_detail_pane_is_hidden() {
        let mut app = App::default();
        app.detail_visible = false;
        app.config.keybindings.global.scroll_to_end = "End".to_string();

        dispatch(&mut app, &KeyEvent::new(KeyCode::End, KeyModifiers::NONE)).await;

        assert!(!app.detail_scroll_to_bottom);
    }

    /// Remap `scroll_to_end` to a different key and confirm the new key fires
    /// while the old default does not. `Issues` already binds `G` to
    /// `drill_into_scope`, so the test moves the local key aside too —
    /// without that, the per-tab arm claims `G` before the global fallback
    /// runs.
    #[tokio::test]
    async fn scroll_to_end_remap_to_a_custom_key_works() {
        let mut app = App::default();
        app.detail_visible = true;
        app.detail_scroll = 5;
        app.scope = crate::scope::Scope::Repository("group/project".to_string());
        app.config.keybindings.global.scroll_to_end = "G".to_string();
        app.config.keybindings.issues.drill_into_scope = "P".to_string();

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT),
        )
        .await;

        assert!(app.detail_scroll_to_bottom);
        assert_eq!(app.detail_scroll, 5);
    }

    /// `scroll_top` writes the scroll directly: 0 is a known index, so
    /// unlike `scroll_to_end` there is no flag round-trip and the
    /// dispatch sets the value in one shot.
    #[tokio::test]
    async fn scroll_top_zeroes_the_scroll() {
        let mut app = App::default();
        app.detail_visible = true;
        app.detail_scroll = 5;
        app.config.keybindings.global.scroll_top = "Home".to_string();

        dispatch(&mut app, &KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)).await;

        assert_eq!(app.detail_scroll, 0);
    }

    /// The default binding ships as `Home`, which is the same physical key
    /// the hardcoded #492 arm already dispatches. The two paths land on
    /// the same `detail_scroll = 0`, so the compound behaviour is
    /// idempotent for the user.
    #[tokio::test]
    async fn scroll_top_default_does_not_raise_the_jump_to_bottom_flag() {
        let mut app = App::default();
        app.detail_visible = true;
        app.detail_scroll = 5;

        dispatch(&mut app, &KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)).await;

        assert_eq!(app.detail_scroll, 0);
        assert!(!app.detail_scroll_to_bottom);
    }

    /// `gg` jumps to a known index, so it writes the scroll directly and
    /// must not raise the jump-to-bottom flag on the way.
    #[tokio::test]
    async fn scroll_top_resolves_two_char_sequence_through_matches_with_pending() {
        let mut app = App::default();
        app.detail_visible = true;
        app.detail_scroll = 5;
        app.config.keybindings.global.scroll_top = "gg".to_string();

        dispatch_with_pending(
            &mut app,
            &KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
            Some('g'),
        )
        .await;

        assert_eq!(app.detail_scroll, 0);
        assert!(!app.detail_scroll_to_bottom);
    }

    /// The default `scroll_top = "Home"` shares its key with the hardcoded
    /// #492 table-navigation arm, which fires regardless of
    /// `detail_visible`. Confirm `dispatch` does not panic on the
    /// default binding while the detail pane is hidden — the table
    /// navigation either jumps to row 0 (when there's a row) or no-ops
    /// on an empty table, and the `scroll_top` arm itself does not run.
    #[tokio::test]
    async fn scroll_top_default_dispatches_without_panic_in_hidden_detail() {
        let mut app = App::default();
        app.detail_visible = false;
        app.config.keybindings.global.scroll_top = "Home".to_string();

        dispatch(&mut app, &KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)).await;

        // detail_scroll was never touched by `scroll_top` (which is
        // gated on detail_visible); the hardcoded Home arm runs but
        // resets to 0 unconditionally.
        assert_eq!(app.detail_scroll, 0);
    }

    /// When the binding is remapped to a key with no other handler,
    /// `scroll_top` is gated on `detail_visible` and stays out of the way
    /// while the pane is hidden.
    #[tokio::test]
    async fn scroll_top_remapped_to_unbound_key_ignores_hidden_detail() {
        let mut app = App::default();
        app.detail_visible = false;
        app.detail_scroll = 5;
        app.config.keybindings.global.scroll_top = "F1".to_string();

        dispatch(&mut app, &KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)).await;

        assert_eq!(app.detail_scroll, 5);
    }

    /// Full page scrolling defaults to PageDown / PageUp while half page
    /// scrolling defaults to Ctrl+d / Ctrl+u. These tests drive the
    /// Ctrl+f/b/d/u spelling, so we ensure the full page bindings are set.
    fn bind_vim_page_keys(app: &mut App) {
        app.config.keybindings.global.scroll_page_down = "Ctrl+f".to_string();
        app.config.keybindings.global.scroll_page_up = "Ctrl+b".to_string();
        app.config.keybindings.global.scroll_half_page_down = "Ctrl+d".to_string();
        app.config.keybindings.global.scroll_half_page_up = "Ctrl+u".to_string();
    }

    /// `Ctrl+f`/`Ctrl+b` move a full viewport, `Ctrl+d`/`Ctrl+u` half of it,
    /// against the usable height of `detail_rect` (its height minus the two
    /// border rows).
    #[tokio::test]
    async fn ctrl_f_b_d_u_scroll_by_full_and_half_viewport() {
        let mut app = App::default();
        bind_vim_page_keys(&mut app);
        app.detail_visible = true;
        app.detail_rect = Some(ratatui::layout::Rect::new(0, 0, 40, 22));
        app.detail_scroll = 5;

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        )
        .await;
        assert_eq!(app.detail_scroll, 25);

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
        )
        .await;
        assert_eq!(app.detail_scroll, 5);

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        )
        .await;
        assert_eq!(app.detail_scroll, 15);

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        )
        .await;
        assert_eq!(app.detail_scroll, 5);
    }

    /// Without a rendered `detail_rect`, there is no known viewport height,
    /// so page-scroll keys move nothing (Entscheidung 4).
    #[tokio::test]
    async fn ctrl_page_scroll_keys_do_nothing_without_detail_rect() {
        let mut app = App::default();
        bind_vim_page_keys(&mut app);
        app.detail_visible = true;
        app.detail_rect = None;
        app.detail_scroll = 5;

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        )
        .await;
        assert_eq!(app.detail_scroll, 5);

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        )
        .await;
        assert_eq!(app.detail_scroll, 5);
    }

    /// `Ctrl+b`/`Ctrl+u` do not underflow at the top of the pane, and a
    /// two-row-or-smaller rect (no usable height) does not panic.
    #[tokio::test]
    async fn page_scroll_up_does_not_underflow_at_top() {
        let mut app = App::default();
        bind_vim_page_keys(&mut app);
        app.detail_visible = true;
        app.detail_rect = Some(ratatui::layout::Rect::new(0, 0, 40, 2));
        app.detail_scroll = 0;

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
        )
        .await;
        assert_eq!(app.detail_scroll, 0);

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        )
        .await;
        assert_eq!(app.detail_scroll, 0);
    }

    /// A page-scroll key still scrolls even while a `g` sequence prefix is
    /// pending, instead of being swallowed by the sequence lapsing
    /// (Entscheidung 5: both handler sites stay wired).
    #[tokio::test]
    async fn ctrl_f_scrolls_even_with_pending_g_prefix() {
        let mut app = App::default();
        bind_vim_page_keys(&mut app);
        app.detail_visible = true;
        app.detail_rect = Some(ratatui::layout::Rect::new(0, 0, 40, 22));
        app.detail_scroll = 5;

        dispatch_with_pending(
            &mut app,
            &KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
            Some('g'),
        )
        .await;
        assert_eq!(app.detail_scroll, 25);
    }

    /// `Ctrl+f` must only page-scroll, not also trigger the hardcoded plain
    /// `f` shortcut (open inline search) that ignores modifiers.
    #[tokio::test]
    async fn ctrl_f_does_not_also_open_inline_search() {
        let mut app = App::default();
        bind_vim_page_keys(&mut app);
        app.detail_visible = true;
        app.detail_rect = Some(ratatui::layout::Rect::new(0, 0, 40, 22));

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        )
        .await;
        assert!(!app.is_typing_search);
    }

    /// `Ctrl+d` on the Pipelines tab must only page-scroll, not also trigger
    /// the hardcoded plain `d` shortcut (cancel the selected pipeline) that
    /// ignores modifiers.
    #[tokio::test]
    async fn ctrl_d_does_not_also_cancel_pipeline() {
        let mut app = App::default();
        bind_vim_page_keys(&mut app);
        app.active_tab = crate::app::Tab::Pipelines;
        app.pipelines.items = vec![crate::domain::pipelines::Pipeline {
            id: 1,
            status: "running".to_string(),
            r#ref: "main".to_string(),
            updated_at: "".to_string(),
            name: "".to_string(),
            display_title: "".to_string(),
            event: "".to_string(),
            head_sha: "".to_string(),
            actor_login: "".to_string(),
            duration_seconds: None,
            created_at: None,
            source: None,
            project_path: String::new(),
            web_url: None,
        }];
        app.pipelines.state.select(Some(0));
        app.detail_visible = true;
        app.detail_rect = Some(ratatui::layout::Rect::new(0, 0, 40, 22));
        app.detail_scroll = 5;

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        )
        .await;
        assert_eq!(app.pipelines.items[0].status, "running");
        assert_eq!(app.detail_scroll, 15);
    }

    /// `Ctrl+u` must only page-scroll, not also trigger the hardcoded plain
    /// `u` shortcut (check for updates) that ignores modifiers.
    #[tokio::test]
    async fn ctrl_u_does_not_also_trigger_self_update() {
        let mut app = App::default();
        bind_vim_page_keys(&mut app);
        app.detail_visible = true;
        app.detail_rect = Some(ratatui::layout::Rect::new(0, 0, 40, 22));

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        )
        .await;
        assert_eq!(app.error_message, None);
    }

    fn app_with_branches(count: usize) -> App {
        let mut app = App::default();
        app.active_tab = crate::app::Tab::Branches;
        app.branches.set_items(
            (0..count)
                .map(|i| crate::domain::branches::Branch {
                    name: format!("branch-{i:02}"),
                    default: false,
                    protected: false,
                    web_url: String::new(),
                    can_push: false,
                    commit_sha: String::new(),
                })
                .collect(),
        );
        app.sync_list_views();
        // 4 rows of table chrome leave 20 data rows, so half a page is 10.
        app.content_rect = Some(ratatui::layout::Rect::new(0, 0, 80, 24));
        app
    }

    /// With the detail pane hidden, `Ctrl+d`/`Ctrl+u` move the list
    /// selection by half the table rows and stop at both ends.
    #[tokio::test]
    async fn ctrl_d_u_move_the_list_by_half_a_page_when_detail_is_hidden() {
        let mut app = app_with_branches(25);
        app.detail_visible = false;
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);

        dispatch(&mut app, &ctrl('d')).await;
        assert_eq!(app.selected_branch_name().as_deref(), Some("branch-10"));
        dispatch(&mut app, &ctrl('d')).await;
        dispatch(&mut app, &ctrl('d')).await;
        assert_eq!(
            app.selected_branch_name().as_deref(),
            Some("branch-24"),
            "stops on the last row"
        );
        dispatch(&mut app, &ctrl('u')).await;
        assert_eq!(app.selected_branch_name().as_deref(), Some("branch-14"));
        dispatch(&mut app, &ctrl('u')).await;
        dispatch(&mut app, &ctrl('u')).await;
        assert_eq!(
            app.selected_branch_name().as_deref(),
            Some("branch-00"),
            "stops on the first row"
        );
    }

    /// While the detail pane shows, `Ctrl+d` keeps scrolling the pane and
    /// leaves the list selection alone.
    #[tokio::test]
    async fn ctrl_d_scrolls_the_visible_detail_pane_not_the_list() {
        let mut app = app_with_branches(25);
        app.detail_visible = true;
        app.detail_rect = Some(ratatui::layout::Rect::new(0, 0, 40, 22));

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        )
        .await;
        assert_eq!(app.selected_branch_name().as_deref(), Some("branch-00"));
        assert_eq!(app.detail_scroll, 10);
    }

    #[tokio::test]
    async fn milestone_close_in_group_scope_targets_the_selected_rows_project() {
        use crate::domain::milestones::Milestone;
        let mut app = App::default();
        app.scope = crate::scope::Scope::Group("acme".to_string());
        app.active_tab = crate::app::Tab::Milestones;
        let milestone = |project: &str| Milestone {
            iid: 3,
            title: format!("{project} v1"),
            state: "active".to_string(),
            project_path: project.to_string(),
            ..Milestone::default()
        };
        app.milestones.items = vec![milestone("acme/alpha"), milestone("acme/beta")];
        app.milestones.state.select(Some(1));

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
        )
        .await;

        let dialog = app.submit_dialog.expect("close opens a confirm dialog");
        assert!(matches!(
            dialog.action,
            crate::app::ConfirmAction::CloseMilestone(3)
        ));
        assert_eq!(dialog.project_path, "acme/beta");
    }

    fn mr_in(project: &str, iid: u64, author: &str) -> crate::domain::mr::MergeRequest {
        serde_json::from_value(serde_json::json!({
            "iid": iid,
            "title": format!("MR {iid}"),
            "state": "opened",
            "updated_at": "2026-01-01T00:00:00Z",
            "author": { "username": author },
            "target_branch": "main",
            "draft": false,
            "project_path": project,
        }))
        .expect("merge request fixture")
    }

    fn column_filter(column: &str, value: &str) -> HashMap<String, HashSet<String>> {
        HashMap::from([(column.to_string(), HashSet::from([value.to_string()]))])
    }

    #[tokio::test]
    async fn group_milestone_create_uses_project_of_highlighted_filtered_row() {
        let mut app = App::default();
        app.scope = crate::scope::Scope::Group("grp".to_string());
        app.active_tab = crate::app::Tab::Milestones;
        app.milestones.items = serde_json::from_value(serde_json::json!([
            { "iid": 1, "title": "Alpha", "description": null, "start_date": null,
              "due_date": null, "project_path": "grp/a" },
            { "iid": 2, "title": "Beta", "description": null, "start_date": null,
              "due_date": null, "project_path": "grp/b" },
        ]))
        .expect("milestone fixtures");
        app.column_filters
            .insert(crate::app::Tab::Milestones, column_filter("Title", "Beta"));
        app.milestones.state.select(Some(0));

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
        )
        .await;

        let menu = app.edit_menu.expect("create milestone menu");
        assert_eq!(menu.entity_project, "grp/b");
    }

    #[tokio::test]
    async fn select_stage_selects_visible_jobs_in_the_highlighted_stage() {
        let mut app = App::default();
        app.active_tab = crate::app::Tab::Jobs;
        app.jobs.items = serde_json::from_value(serde_json::json!([
            { "id": 1, "status": "success", "stage": "build", "name": "compile" },
            { "id": 2, "status": "success", "stage": "test", "name": "unit" },
            { "id": 3, "status": "failed", "stage": "test", "name": "e2e" },
        ]))
        .expect("job fixtures");
        app.column_filters
            .insert(crate::app::Tab::Jobs, column_filter("Name", "e2e"));
        app.jobs.state.select(Some(0));

        dispatch(
            &mut app,
            &KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
        )
        .await;

        assert_eq!(app.selected_jobs, HashSet::from([3]));
    }

    #[test]
    fn jump_to_mr_tab_highlights_target_mr_while_a_filter_hides_earlier_rows() {
        let mut app = App::default();
        app.scope = crate::scope::Scope::Repository("me/repo".to_string());
        app.mrs.items = vec![
            mr_in("me/repo", 1, "alice"),
            mr_in("me/repo", 2, "bob"),
            mr_in("me/repo", 3, "bob"),
        ];
        app.column_filters.insert(
            crate::app::Tab::MergeRequests,
            column_filter("Author", "bob"),
        );
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        jump_to_mr_tab(&mut app, 3, None, tx);

        assert_eq!(app.active_tab, crate::app::Tab::MergeRequests);
        assert_eq!(app.selected_mr_ref(), Some(("me/repo".to_string(), 3)));
    }

    fn listed_pipeline(id: u64) -> crate::domain::pipelines::Pipeline {
        serde_json::from_value(serde_json::json!({
            "id": id, "status": "success", "ref": "main", "updated_at": "",
            "name": "", "display_title": "", "event": "", "head_sha": "",
            "actor_login": "", "project_path": ""
        }))
        .unwrap()
    }

    fn job(id: u64) -> crate::domain::pipelines::Job {
        serde_json::from_value(serde_json::json!({
            "id": id, "status": "success", "stage": "test", "name": "unit"
        }))
        .unwrap()
    }

    /// The user left the Pipelines tab while the jobs fetch was in flight: the
    /// reply is cached but must not pull them onto the Jobs tab.
    #[test]
    fn jobs_reply_after_leaving_pipelines_is_cached_without_switching_tabs() {
        let mut app = App::default();
        app.pipelines.items = vec![listed_pipeline(7)];
        app.active_tab = crate::app::Tab::Issues;
        app.loading_tabs.insert(crate::app::Tab::Jobs);

        apply_opened_pipeline(
            &mut app,
            7,
            PipelineOpening::Descend,
            Ok(PipelineContents::Jobs(vec![job(70)])),
        );

        assert_eq!(app.active_tab, crate::app::Tab::Issues);
        assert_eq!(app.active_pipeline_id, None);
        assert!(app.jobs.items.is_empty());
        let cached: Vec<u64> = app.pipeline_jobs[&7].iter().map(|j| j.id()).collect();
        assert_eq!(cached, vec![70]);
        assert!(!app.loading_tabs.contains(&crate::app::Tab::Jobs));
    }

    /// A group-scope pipeline with no project of its own must not record the
    /// group as the Jobs tab's project: the jobs' web URLs name it instead.
    #[test]
    fn opening_a_pipeline_without_a_project_records_none_in_group_scope() {
        let mut app = App::default();
        app.scope = crate::scope::Scope::Group("acme".to_string());
        app.active_tab = crate::app::Tab::Pipelines;
        app.pipelines.items = vec![listed_pipeline(7)];

        apply_opened_pipeline(
            &mut app,
            7,
            PipelineOpening::Descend,
            Ok(PipelineContents::Jobs(vec![job(70)])),
        );

        assert_eq!(app.active_pipeline_id, Some(7));
        assert_eq!(app.active_pipeline_project, None);
        assert_eq!(app.jobs_project(), None);
        assert_eq!(app.project_path_for_jobs(), "acme");
    }

    /// Enter pressed twice before the first reply: the first reply descends,
    /// so the second finds its pipeline no longer listed and must not push a
    /// second level.
    #[test]
    fn downstream_reply_for_a_pipeline_no_longer_listed_does_not_descend() {
        let mut app = App::default();
        app.active_tab = crate::app::Tab::Pipelines;
        app.pipelines.items = vec![listed_pipeline(7)];
        let level = || crate::domain::pipelines::ChildLevel {
            children: vec![listed_pipeline(8)],
            pending: vec![],
        };

        for _ in 0..2 {
            apply_opened_pipeline(
                &mut app,
                7,
                PipelineOpening::Descend,
                Ok(PipelineContents::Downstream(level())),
            );
        }

        assert_eq!(app.nav_depth(), 1);
        assert_eq!(app.current_parent_id(), Some(7));
        let listed: Vec<u64> = app.pipelines.items.iter().map(|p| p.id()).collect();
        assert_eq!(listed, vec![8]);
    }

    fn deployment_json(id: u64, iid: u64, job: Option<u64>) -> serde_json::Value {
        let mut deployment = serde_json::json!({
            "id": id, "iid": iid, "ref_name": "main", "tag": false,
            "sha": format!("{id:0>40}"), "status": "success",
            "created_at": "2026-10-01T00:00:00Z", "updated_at": "2026-10-01T00:00:00Z"
        });
        if let Some(job) = job {
            deployment["deployable"] = serde_json::json!({ "id": job, "name": "deploy" });
        }
        deployment
    }

    /// `production` (highlighted) and `staging` beside an open detail pane,
    /// with `deployments` listed for production and focused unless the list
    /// is empty.
    fn app_with_focused_deployments(deployments: serde_json::Value) -> App {
        let mut app = App::default();
        app.active_tab = crate::app::Tab::Environments;
        app.detail_visible = true;
        app.detail_rect = Some(ratatui::layout::Rect::new(0, 0, 40, 22));
        app.environments.set_items(
            serde_json::from_value(serde_json::json!([
                {"id": 1, "name": "production", "state": "available"},
                {"id": 2, "name": "staging", "state": "available"}
            ]))
            .expect("environment fixture parses"),
        );
        app.sync_list_views();
        app.show_deployments(
            "production".to_string(),
            serde_json::from_value(deployments).expect("deployment fixture parses"),
        );
        app
    }

    fn two_deployments() -> serde_json::Value {
        serde_json::json!([
            deployment_json(11, 2, Some(501)),
            deployment_json(10, 1, Some(500))
        ])
    }

    fn key(code: KeyCode) -> KeyEvent {
        let modifiers = match code {
            KeyCode::Char(c) if c.is_ascii_uppercase() => KeyModifiers::SHIFT,
            _ => KeyModifiers::NONE,
        };
        KeyEvent::new(code, modifiers)
    }

    fn highlighted_environment(app: &App) -> Option<&str> {
        app.environments.cursor_row().map(|env| env.name.as_str())
    }

    #[tokio::test]
    async fn j_moves_through_focused_deployments_and_keeps_the_environment() {
        let mut app = app_with_focused_deployments(two_deployments());

        dispatch(&mut app, &key(KeyCode::Char('j'))).await;

        assert_eq!(app.selected_deployment().map(|d| d.id), Some(10));
        assert_eq!(highlighted_environment(&app), Some("production"));
    }

    #[tokio::test]
    async fn esc_hands_navigation_back_to_the_environment_list() {
        let mut app = app_with_focused_deployments(two_deployments());

        dispatch(&mut app, &key(KeyCode::Esc)).await;
        dispatch(&mut app, &key(KeyCode::Char('j'))).await;

        assert_eq!(highlighted_environment(&app), Some("staging"));
    }

    #[tokio::test]
    async fn view_key_refocuses_deployments_already_listed() {
        let mut app = app_with_focused_deployments(two_deployments());

        dispatch(&mut app, &key(KeyCode::Esc)).await;
        dispatch(&mut app, &key(KeyCode::Enter)).await;
        dispatch(&mut app, &key(KeyCode::Char('j'))).await;

        assert_eq!(app.selected_deployment().map(|d| d.id), Some(10));
        assert_eq!(highlighted_environment(&app), Some("production"));
    }

    #[tokio::test]
    async fn focus_key_toggles_between_the_list_and_the_deployments() {
        let mut app = app_with_focused_deployments(two_deployments());
        let tab = key(KeyCode::Tab);
        assert!(app.is_deployments_focus_key(&tab));

        dispatch(&mut app, &tab).await;
        dispatch(&mut app, &key(KeyCode::Char('j'))).await;
        assert_eq!(highlighted_environment(&app), Some("staging"));
    }

    #[tokio::test]
    async fn hiding_the_detail_pane_hands_navigation_back_to_the_environment_list() {
        let mut app = app_with_focused_deployments(two_deployments());

        dispatch(&mut app, &key(KeyCode::Char('q'))).await;
        dispatch(&mut app, &key(KeyCode::Char('j'))).await;

        assert!(!app.detail_visible);
        assert_eq!(highlighted_environment(&app), Some("staging"));
    }

    #[tokio::test]
    async fn deployments_left_off_a_narrow_screen_take_no_keys() {
        let mut app = app_with_focused_deployments(two_deployments());
        app.detail_rect = None;

        assert!(!app.is_deployments_focus_key(&key(KeyCode::Tab)));
        dispatch(&mut app, &key(KeyCode::Char('j'))).await;
        assert_eq!(highlighted_environment(&app), Some("staging"));
    }

    #[tokio::test]
    async fn rollback_key_confirms_a_rollback_to_the_selected_deployment() {
        let mut app = app_with_focused_deployments(two_deployments());

        dispatch(&mut app, &key(KeyCode::Char('j'))).await;
        dispatch(&mut app, &key(KeyCode::Char('R'))).await;

        let dialog = app.submit_dialog.as_ref().expect("a confirmation opens");
        let crate::app::ConfirmAction::RollbackDeployment {
            environment,
            deployment,
        } = &dialog.action
        else {
            panic!("expected a rollback confirmation, got {:?}", dialog.action);
        };
        assert_eq!((environment.as_str(), deployment.id), ("production", 10));
        assert!(dialog.is_on_cancel());
    }

    #[tokio::test]
    async fn rollback_key_without_listed_deployments_raises_an_error() {
        let mut app = app_with_focused_deployments(serde_json::json!([]));

        dispatch(&mut app, &key(KeyCode::Char('R'))).await;

        assert!(app.submit_dialog.is_none());
        assert_eq!(app.error_message.as_deref(), Some(NO_DEPLOYMENT_SELECTED));
    }

    #[tokio::test]
    async fn gitlab_rollback_of_a_deployment_without_a_job_is_refused() {
        let mut app =
            app_with_focused_deployments(serde_json::json!([deployment_json(10, 1, None)]));

        dispatch(&mut app, &key(KeyCode::Char('R'))).await;

        assert!(app.submit_dialog.is_none());
        assert_eq!(
            app.error_message.as_deref(),
            Some("Deployment #1 has no deploy job to re-run")
        );
    }

    #[tokio::test]
    async fn stop_key_confirms_before_stopping_the_highlighted_environment() {
        let mut app = app_with_focused_deployments(two_deployments());

        dispatch(&mut app, &key(KeyCode::Char('d'))).await;

        let dialog = app.submit_dialog.as_ref().expect("a confirmation opens");
        assert!(matches!(
            &dialog.action,
            crate::app::ConfirmAction::StopEnvironment(env) if env.id == 1
        ));
        assert!(dialog.is_on_cancel());
    }
}
