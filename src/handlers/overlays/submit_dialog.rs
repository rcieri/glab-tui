use crate::app::{App, ConfirmAction, DialogChoice};
use crate::effect::Effects;
use crate::event::{CommandOutcome, Event};
use crate::handlers::tabs::{
    OptimisticTable, flip_job_statuses, flip_pipeline_statuses, run_optimistic_mutation,
    spawn_cancel_jobs,
};
use crate::keybinding::KeyPress;
use crate::keymap::{Action, KeyContext};
use tokio::sync::mpsc::UnboundedSender;

pub fn handle_submit_dialog(
    app: &mut App,
    key_event: &KeyPress,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) -> bool {
    let Some(mut dialog) = app.submit_dialog.take() else {
        return false;
    };

    let outcome = match app
        .config
        .keybindings
        .resolve(KeyContext::Dialog, key_event)
    {
        Some(Action::DialogPreviousField) => {
            if dialog.is_on_button() {
                if !dialog.options.is_empty() {
                    dialog.cursor_idx = dialog.options.len(); // jump up to last option
                }
            } else if dialog.cursor_idx > 1 {
                dialog.cursor_idx -= 1; // move up
            } else {
                dialog.cursor_idx = dialog.cancel_idx(); // wrap around to buttons
            }
            Outcome::StayOpen
        }
        Some(Action::DialogNextField) => {
            if dialog.is_on_button() {
                if !dialog.options.is_empty() {
                    dialog.cursor_idx = 1; // wrap around to first option
                }
            } else if dialog.cursor_idx < dialog.options.len() {
                dialog.cursor_idx += 1; // move down
            } else {
                dialog.cursor_idx = dialog.cancel_idx(); // jump down to buttons
            }
            Outcome::StayOpen
        }
        Some(Action::DialogPreviousButton) => {
            dialog.move_between_buttons(-1);
            Outcome::StayOpen
        }
        Some(Action::DialogNextButton) => {
            dialog.move_between_buttons(1);
            Outcome::StayOpen
        }
        Some(Action::DialogNext) => {
            dialog.move_next();
            Outcome::StayOpen
        }
        Some(Action::DialogPrevious) => {
            dialog.move_prev();
            Outcome::StayOpen
        }
        Some(Action::DialogToggleOption) => {
            dialog.toggle_focused_option();
            Outcome::StayOpen
        }
        Some(Action::DialogActivate) => activate(&mut dialog),
        Some(Action::DialogSubmit) if dialog.is_on_submit() => Outcome::Submit,
        Some(Action::DialogCancel) => Outcome::Cancel,
        // Consume all other keys while the dialog is open so they
        // don't fall through to the underlying tab.
        _ => Outcome::StayOpen,
    };

    apply_outcome(app, dialog, outcome, tx, fx);
    true
}

enum Outcome {
    Submit,
    Cancel,
    Choice(DialogChoice),
    StayOpen,
}

/// Submits, cancels or picks the focused button, or toggles the focused
/// option.
fn activate(dialog: &mut crate::app::SubmitDialog) -> Outcome {
    if dialog.is_on_submit() {
        Outcome::Submit
    } else if dialog.is_on_cancel() {
        Outcome::Cancel
    } else if let Some(focused) = dialog.focused_choice() {
        Outcome::Choice(focused)
    } else {
        dialog.toggle_focused_option();
        Outcome::StayOpen
    }
}

/// What `DialogActivate` does, for a mouse click that already moved the
/// cursor, so a click works whatever keys `DialogActivate` is bound to.
pub fn activate_focused_button(app: &mut App, tx: &UnboundedSender<Event>, fx: &mut Effects) {
    let Some(mut dialog) = app.submit_dialog.take() else {
        return;
    };
    let outcome = activate(&mut dialog);
    apply_outcome(app, dialog, outcome, tx, fx);
}

fn apply_outcome(
    app: &mut App,
    mut dialog: crate::app::SubmitDialog,
    outcome: Outcome,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    match outcome {
        Outcome::Submit => {
            // Drain the dialog so we can inspect option toggles before
            // dispatching the API call.
            let action = dialog.action.clone();
            let project_path = std::mem::take(&mut dialog.project_path);
            let options = std::mem::take(&mut dialog.options);
            run_submit_action(app, action, project_path, options, tx.clone(), fx);
        }
        Outcome::Choice(choice) => {
            run_dialog_choice(app, &dialog.action, &dialog.project_path, choice);
        }
        Outcome::Cancel => {}
        Outcome::StayOpen => app.submit_dialog = Some(dialog),
    }
}

fn run_dialog_choice(
    app: &mut App,
    action: &ConfirmAction,
    project_path: &str,
    choice: DialogChoice,
) {
    let ConfirmAction::SubmitReview(mr_iid) = *action else {
        return;
    };
    match choice {
        DialogChoice::KeepDrafts => {
            let notice = app.kept_drafts_notice();
            // The project cache already holds them, and opening this diff
            // again restores them from there.
            app.draft_comments.clear();
            app.stale_draft_comments.clear();
            app.show_notice(notice);
        }
        DialogChoice::DiscardDrafts => {
            let drafts = app.all_drafts_phrase();
            app.draft_comments.clear();
            app.stale_draft_comments.clear();
            app.persist_draft_comments(project_path, mr_iid);
            app.in_review_mode = false;
            app.show_notice(format!("Discarded {drafts}"));
        }
    }
    app.diff_view = None;
}

fn merge_options_from(
    options: &[crate::app::SubmitOption],
) -> (bool, bool, Option<&'static str>, bool) {
    let mut squash = false;
    let mut delete_branch = false;
    let mut strategy: Option<&'static str> = None;
    let mut auto_merge = false;
    for opt in options.iter().filter(|o| o.checked) {
        match opt.label.as_str() {
            "Strategy: Squash" => squash = true,
            "Delete source branch" => delete_branch = true,
            "Strategy: Merge commit" => strategy = Some("merge"),
            "Strategy: Rebase" => strategy = Some("rebase"),
            "Auto-merge" => auto_merge = true,
            _ => {}
        }
    }
    (squash, delete_branch, strategy, auto_merge)
}

/// The merge method the dialog's strategy options pick, with the precedence
/// `GhBackend::merge_args` gives them.
fn merge_method_from(options: &[crate::app::SubmitOption]) -> crate::domain::mr::MergeMethod {
    use crate::domain::mr::MergeMethod;
    match merge_options_from(options) {
        (true, ..) => MergeMethod::Squash,
        (false, _, Some("rebase"), _) => MergeMethod::Rebase,
        _ => MergeMethod::Merge,
    }
}

/// The PRs among `items` (project path, number) that sit in a stack, which
/// only the stack merge can merge. A failed lookup leaves a project's PRs to
/// the plain merge, whose error then names the problem.
async fn stacked_prs(
    client: &crate::domain::client::GitlabClient,
    items: &[(String, u64, Option<String>)],
) -> std::collections::HashSet<(String, u64)> {
    let mut by_project: std::collections::HashMap<&str, Vec<u64>> =
        std::collections::HashMap::new();
    for (project, iid, _) in items {
        by_project.entry(project).or_default().push(*iid);
    }
    let mut stacked = std::collections::HashSet::new();
    for (project, iids) in by_project {
        if let Ok(stacks) = client.list_pr_stack_summaries(project, &iids).await {
            stacked.extend(stacks.into_keys().map(|iid| (project.to_string(), iid)));
        }
    }
    stacked
}
/// Runs `confirm_action` against `project_path`, the project the dialog was
/// opened for, never one re-resolved from the iid.
fn run_submit_action(
    app: &mut App,
    confirm_action: crate::app::ConfirmAction,
    project_path: String,
    options: Vec<crate::app::SubmitOption>,
    tx: UnboundedSender<Event>,
    fx: &mut Effects,
) {
    match confirm_action {
        crate::app::ConfirmAction::DeleteMilestone(iid) => {
            app.pending_delete_milestone = Some((project_path.clone(), iid));
            let client = app.gitlab_client.clone().unwrap();
            fx.spawn(async move {
                let res =
                    crate::domain::milestones::delete_milestone(&client, &project_path, iid).await;
                match res {
                    Ok(_) => {
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::Milestones,
                            Ok(CommandOutcome::Mutated),
                        ));
                        let _ = tx.send(Event::MilestoneDeleted);
                    }
                    Err(e) => {
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::Milestones,
                            Err(e.to_string()),
                        ));
                    }
                }
            });
        }
        crate::app::ConfirmAction::CloseMilestone(iid) => {
            if let Some(pos) = app.milestone_index(&project_path, iid) {
                app.milestones.items[pos].state = "closed".to_string();
            }
            app.project_cache.milestones = app.milestones.items.clone();
            let client = app.gitlab_client.clone().unwrap();
            let tx2 = tx.clone();
            fx.spawn(async move {
                let res = crate::domain::milestones::update_milestone_state(
                    &client,
                    &project_path,
                    iid,
                    true,
                )
                .await;
                match res {
                    Ok(_) => {
                        let _ = tx2.send(Event::MilestoneClosed);
                    }
                    Err(e) => {
                        let _ = tx2.send(Event::CommandCompleted(
                            crate::app::Tab::Milestones,
                            Err(e.to_string()),
                        ));
                    }
                }
            });
        }
        crate::app::ConfirmAction::ReopenMilestone(iid) => {
            if let Some(pos) = app.milestone_index(&project_path, iid) {
                app.milestones.items[pos].state = "active".to_string();
            }
            app.project_cache.milestones = app.milestones.items.clone();
            let client = app.gitlab_client.clone().unwrap();
            let tx2 = tx.clone();
            fx.spawn(async move {
                let res = crate::domain::milestones::update_milestone_state(
                    &client,
                    &project_path,
                    iid,
                    false,
                )
                .await;
                match res {
                    Ok(_) => {
                        let _ = tx2.send(Event::MilestoneReopened);
                    }
                    Err(e) => {
                        let _ = tx2.send(Event::CommandCompleted(
                            crate::app::Tab::Milestones,
                            Err(e.to_string()),
                        ));
                    }
                }
            });
        }
        crate::app::ConfirmAction::DeleteRelease(tag_name) => {
            app.pending_delete_release = Some((project_path.clone(), tag_name.clone()));
            let client = app.gitlab_client.clone().unwrap();
            fx.spawn(async move {
                let res =
                    crate::domain::releases::delete_release(&client, &project_path, &tag_name)
                        .await;
                match res {
                    Ok(_) => {
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::Releases,
                            Ok(CommandOutcome::Mutated),
                        ));
                        let _ = tx.send(Event::ReleaseDeleted);
                    }
                    Err(e) => {
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::Releases,
                            Err(e.to_string()),
                        ));
                    }
                }
            });
        }
        crate::app::ConfirmAction::DeleteBranch(branch_name) => {
            app.selected_branches.remove(&branch_name);
            app.branches.retain(|b| b.name != branch_name);
            let client = app.gitlab_client.clone().unwrap();
            fx.spawn(async move {
                let res =
                    crate::domain::branches::delete_branch(&client, &project_path, &branch_name)
                        .await;
                match res {
                    Ok(_) => {
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::Branches,
                            Ok(CommandOutcome::Mutated),
                        ));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::Branches,
                            Ok(CommandOutcome::Mutated),
                        ));
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::Branches,
                            Err(format!("Failed to delete branch: {e}")),
                        ));
                    }
                }
            });
        }
        crate::app::ConfirmAction::BulkDeleteBranches(branch_names) => {
            app.selected_branches.clear();
            app.branches.retain(|b| !branch_names.contains(&b.name));
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            fx.spawn(async move {
                let mut errors = Vec::new();
                for (i, branch_name) in branch_names.iter().enumerate() {
                    if i > 0 {
                        crate::backend::rate_limit::pace_bulk_operation().await;
                    }
                    if let Err(e) =
                        crate::domain::branches::delete_branch(&client, &project_path, branch_name)
                            .await
                    {
                        errors.push(format!("{branch_name}: {e}"));
                    }
                }
                let _ = tx.send(Event::CommandCompleted(
                    crate::app::Tab::Branches,
                    Ok(CommandOutcome::Mutated),
                ));
                if !errors.is_empty() {
                    let _ = tx.send(Event::CommandCompleted(
                        crate::app::Tab::Branches,
                        Err(format!("Failed to delete branches: {}", errors.join("; "))),
                    ));
                }
            });
        }
        crate::app::ConfirmAction::CloseIssue(iid) => {
            if let Some(pos) = app.issue_index(&project_path, iid) {
                app.issues.items.remove(pos);
            }
            app.update_filter_selection();
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            fx.spawn(async move {
                let result = client.close_issue(&project_path, iid).await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::Issues,
                    CommandOutcome::mutation(result),
                ));
            });
        }
        crate::app::ConfirmAction::DeleteIssue(iid) => {
            let client = app.gitlab_client.clone().unwrap();
            fx.spawn(async move {
                let res = client.delete_issue(&project_path, iid).await;
                match res {
                    Ok(_) => {
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::Issues,
                            Ok(CommandOutcome::Mutated),
                        ));
                        let _ = tx.send(Event::IssueDeleted);
                    }
                    Err(e) => {
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::Issues,
                            Err(format!("Failed to delete issue: {e}")),
                        ));
                    }
                }
            });
        }
        crate::app::ConfirmAction::ReopenIssue(iid) => {
            if let Some(pos) = app.issue_index(&project_path, iid) {
                app.issues.items[pos].state = "opened".to_string();
            }
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            fx.spawn(async move {
                let result = client.reopen_issue(&project_path, iid).await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::Issues,
                    CommandOutcome::mutation(result),
                ));
            });
        }
        crate::app::ConfirmAction::CloseMr(iid) => {
            if let Some(pos) = app.mr_index(&project_path, iid) {
                app.mrs.items.remove(pos);
            }
            app.update_filter_selection();
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            fx.spawn(async move {
                let result = client.close_mr(&project_path, iid).await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    CommandOutcome::mutation(result),
                ));
            });
        }
        crate::app::ConfirmAction::DeleteMr(iid) => {
            let client = app.gitlab_client.clone().unwrap();
            fx.spawn(async move {
                let res = client.delete_mr(&project_path, iid).await;
                match res {
                    Ok(_) => {
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::MergeRequests,
                            Ok(CommandOutcome::Mutated),
                        ));
                        let _ = tx.send(Event::MrDeleted);
                    }
                    Err(e) => {
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::MergeRequests,
                            Err(format!("Failed to delete merge request: {e}")),
                        ));
                    }
                }
            });
        }
        crate::app::ConfirmAction::ReopenMr(iid) => {
            if let Some(pos) = app.mr_index(&project_path, iid) {
                app.mrs.items[pos].state = "opened".to_string();
            }
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            fx.spawn(async move {
                let result = client.reopen_mr(&project_path, iid).await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    CommandOutcome::mutation(result),
                ));
            });
        }
        crate::app::ConfirmAction::MergeMr(iid) => {
            let (squash, delete_branch, merge_strategy, auto_merge) = merge_options_from(&options);
            // Capture the source-branch head SHA before removing the row so
            // `glab mr merge --sha=<sha>` can satisfy GitLab 19.2+ merge
            // requirements (#470). Pre-19.2 instances ignore it; GitHub's
            // `GhBackend::merge_mr` also ignores it.
            let mr_sha = app
                .mr_index(&project_path, iid)
                .and_then(|pos| app.mrs.items.remove(pos).sha);
            app.update_filter_selection();
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            fx.spawn(async move {
                let result = client
                    .merge_mr(
                        &project_path,
                        iid,
                        &crate::backend::MergeOptions {
                            squash,
                            delete_branch,
                            strategy: merge_strategy,
                            auto_merge,
                            sha: mr_sha.as_deref(),
                        },
                    )
                    .await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    result
                        .map_err(|e| {
                            // Strip the verbose glab prefix so the toast stays readable.
                            e.to_string()
                                .trim_start_matches("glab command failed: ")
                                .lines()
                                .next()
                                .unwrap_or("merge failed")
                                .to_string()
                        })
                        .map(|()| CommandOutcome::Mutated),
                ));
            });
        }
        crate::app::ConfirmAction::MergeStack(pr_number) => {
            let method = merge_method_from(&options);
            let merged: Vec<u64> = match app.pr_stack(&project_path, pr_number) {
                Some(crate::domain::mr::StackLookup::Full(stack)) => {
                    stack.downstack_open_entries().map(|e| e.number).collect()
                }
                _ => vec![pr_number],
            };
            app.mrs.items.retain(|m| !merged.contains(&m.iid));
            app.update_filter_selection();
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            fx.spawn(async move {
                let result = client
                    .merge_pr_stack(&project_path, pr_number, method)
                    .await;
                let _ = tx.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    CommandOutcome::mutation(result),
                ));
            });
        }
        crate::app::ConfirmAction::BulkMergeMrs(items) => {
            let (squash, delete_branch, merge_strategy, auto_merge) = merge_options_from(&options);
            let method = merge_method_from(&options);
            // Snapshot each MR's head SHA before removing the rows so the
            // async merge loop can forward `--sha` to GitLab 19.2+ (#470).
            let mut items_with_sha: Vec<(String, u64, Option<String>)> =
                Vec::with_capacity(items.len());
            for (project_path, mr_iid) in &items {
                let sha = app
                    .mrs
                    .items
                    .iter()
                    .find(|m| {
                        m.iid == *mr_iid
                            && (project_path.is_empty() || m.project_path == *project_path)
                    })
                    .and_then(|m| m.sha.clone());
                let project = if project_path.is_empty() {
                    app.scope.as_str().to_string()
                } else {
                    project_path.clone()
                };
                items_with_sha.push((project, *mr_iid, sha));
                if let Some(pos) = app.mrs.items.iter().position(|m| {
                    m.iid == *mr_iid && (project_path.is_empty() || m.project_path == *project_path)
                }) {
                    app.mrs.items.remove(pos);
                }
            }
            app.update_filter_selection();
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            let total = items.len();
            fx.spawn(async move {
                let stacked = stacked_prs(&client, &items_with_sha).await;
                let mut failures: Vec<(u64, String)> = Vec::new();
                for (i, (proj, mr_iid, sha)) in items_with_sha.into_iter().enumerate() {
                    if i > 0 {
                        crate::backend::rate_limit::pace_bulk_operation().await;
                    }
                    let result = if stacked.contains(&(proj.clone(), mr_iid)) {
                        client.merge_pr_stack(&proj, mr_iid, method).await
                    } else {
                        client
                            .merge_mr(
                                &proj,
                                mr_iid,
                                &crate::backend::MergeOptions {
                                    squash,
                                    delete_branch,
                                    strategy: merge_strategy,
                                    auto_merge,
                                    sha: sha.as_deref(),
                                },
                            )
                            .await
                    };
                    if let Err(e) = result {
                        failures.push((mr_iid, e.to_string()));
                    }
                }
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    if failures.is_empty() {
                        Ok(CommandOutcome::Mutated)
                    } else {
                        let succeeded = total - failures.len();
                        // Build a concise summary: "2/5 merged. Failed: #12: ..., #34: ..."
                        // Truncate individual error messages so the toast stays readable.
                        let detail: Vec<String> = failures
                            .iter()
                            .map(|(iid, err)| {
                                // Trim the verbose glab prefix from error strings.
                                let trimmed = err
                                    .trim_start_matches("glab command failed: ")
                                    .lines()
                                    .next()
                                    .unwrap_or(err.as_str());
                                format!("#{iid}: {trimmed}")
                            })
                            .collect();
                        Err(format!(
                            "{succeeded}/{total} merged. {} failed: {}",
                            failures.len(),
                            detail.join(", ")
                        ))
                    },
                ));
            });
        }
        crate::app::ConfirmAction::RevokeMr(iid) => {
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            fx.spawn(async move {
                let result = client.revoke_mr(&project_path, iid).await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    CommandOutcome::mutation(result),
                ));
            });
        }
        crate::app::ConfirmAction::RebaseMr(iid) => {
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            fx.spawn(async move {
                let result = client.rebase_mr(&project_path, iid).await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    CommandOutcome::mutation(result),
                ));
            });
        }
        crate::app::ConfirmAction::SubmitReview(mr_iid) => {
            crate::handlers::review_submit::open(app, mr_iid, project_path);
        }
        crate::app::ConfirmAction::MarkAllTodosRead => {
            crate::handlers::todos::mark_all_read(app, &tx, fx);
        }
        crate::app::ConfirmAction::RerunCommand(invocation) => {
            crate::handlers::terminal::rerun(app, invocation, tx, fx);
        }
        crate::app::ConfirmAction::CancelPipeline(pipeline_id) => {
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let rows = flip_pipeline_statuses(app, &[pipeline_id], "canceled");
            let scope = app.scope.clone();
            let active_tab = app.active_tab;
            let states = app.requested_state_queries();
            let tx = app.scoped_sender(tx);
            fx.spawn(async move {
                run_optimistic_mutation(
                    OptimisticTable::Pipelines,
                    "cancel pipeline",
                    rows,
                    &tx,
                    |id| client.cancel_pipeline(&project_path, id),
                )
                .await;
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                crate::fetch::refresh_tab(client, scope, states, active_tab, tx).await;
            });
        }
        crate::app::ConfirmAction::CancelJobs {
            pipeline_id,
            job_ids,
        } => {
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let rows = flip_job_statuses(app, &job_ids, "canceled");
            app.selected_jobs.clear();
            app.clear_info_notices();
            spawn_cancel_jobs(
                client,
                app.scoped_sender(tx.clone()),
                project_path,
                pipeline_id,
                rows,
                fx,
            );
        }
        crate::app::ConfirmAction::StopEnvironment(environment) => {
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx = app.scoped_sender(tx);
            fx.spawn(async move {
                let result = crate::domain::deployments::stop_environment(
                    &client,
                    &project_path,
                    &environment,
                )
                .await;
                let _ = tx.send(Event::CommandCompleted(
                    crate::app::Tab::Environments,
                    result
                        .map_err(|e| {
                            format!("Failed to stop environment {}: {e}", environment.name)
                        })
                        .map(|()| CommandOutcome::Mutated),
                ));
            });
        }
        crate::app::ConfirmAction::RollbackDeployment {
            environment,
            deployment,
        } => {
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let scope = app.scope.clone();
            let tx = app.scoped_sender(tx);
            fx.spawn(async move {
                let result = crate::domain::deployments::rollback_deployment(
                    &client,
                    &project_path,
                    &environment,
                    &deployment,
                )
                .await;
                if let Err(e) = result {
                    let _ = tx.send(Event::CommandCompleted(
                        crate::app::Tab::Environments,
                        Err(format!("Failed to roll back {environment}: {e}")),
                    ));
                    return;
                }
                let _ = tx.send(Event::CommandCompleted(
                    crate::app::Tab::Environments,
                    Ok(CommandOutcome::Mutated),
                ));
                match crate::domain::deployments::list_deployments(
                    &client,
                    &scope,
                    Some(&environment),
                )
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
    }
}
