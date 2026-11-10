//! Opens the confirmation dialog for a destructive action's current
//! selection, for every [`Action`] whose registry row is `destructive`.
//! `handlers::tabs::handle_active_tab_key` runs this before the per-tab
//! dispatch, so remapping a destructive action can never skip its dialog.

use crate::app::{App, ConfirmAction, SubmitDialog};
use crate::handlers::tabs::NO_DEPLOYMENT_SELECTED;
use crate::keymap::Action;

/// Builds and opens the dialog for `action`'s current selection. Returns
/// `false` only when `action` is not one this module knows, in which case
/// the caller should fall through to its own handling.
pub fn confirmation(app: &mut App, action: Action) -> bool {
    match action {
        Action::IssuesCloseEntity => selected_issue(app, ConfirmAction::CloseIssue),
        Action::IssuesReopenEntity => selected_issue(app, ConfirmAction::ReopenIssue),
        Action::IssuesDeleteEntity => selected_issue(app, ConfirmAction::DeleteIssue),
        Action::MrsCloseEntity => selected_mr(app, ConfirmAction::CloseMr),
        Action::MrsReopenEntity => selected_mr(app, ConfirmAction::ReopenMr),
        Action::MrsDeleteEntity => delete_mr(app),
        Action::MrsRevokeMr => revoke_mr(app),
        Action::PipelinesCancel => cancel_pipeline(app),
        Action::JobsCancel => cancel_jobs(app),
        Action::ReleasesDeleteRelease => delete_release(app),
        Action::MilestonesCloseMilestone => selected_milestone(app, ConfirmAction::CloseMilestone),
        Action::MilestonesReopenMilestone => {
            selected_milestone(app, ConfirmAction::ReopenMilestone)
        }
        Action::MilestonesDeleteMilestone => {
            selected_milestone(app, ConfirmAction::DeleteMilestone)
        }
        Action::BranchesDeleteBranch => delete_branch(app),
        Action::EnvironmentsStopEnvironment => stop_environment(app),
        Action::EnvironmentsRollbackDeployment => rollback_deployment(app),
        Action::TodosMarkAllAsRead => {
            crate::handlers::todos::confirm_mark_all_read(app);
            true
        }
        _ => false,
    }
}

fn selected_issue(app: &mut App, build: fn(u64) -> ConfirmAction) -> bool {
    let Some(selected_idx) = app.issues.state.selected() else {
        return true;
    };
    let filtered = app.filtered_issues();
    let Some(issue) = filtered.get(selected_idx) else {
        return true;
    };
    let issue_iid = issue.iid;
    let project_path = app.scope.project_of(&issue.project_path).to_string();
    app.submit_dialog = Some(SubmitDialog::build_with_project(
        build(issue_iid),
        project_path,
        app,
    ));
    true
}

fn selected_mr(app: &mut App, build: fn(u64) -> ConfirmAction) -> bool {
    let Some(selected_idx) = app.mrs.state.selected() else {
        return true;
    };
    let Some(mr) = app.filtered_mrs().get(selected_idx).cloned().cloned() else {
        return true;
    };
    let project_path = app.scope.project_of(&mr.project_path).to_string();
    app.submit_dialog = Some(SubmitDialog::build_with_project(
        build(mr.iid),
        project_path,
        app,
    ));
    true
}

fn revoke_mr(app: &mut App) -> bool {
    let is_github = app
        .gitlab_client
        .as_ref()
        .map(|c| c.is_github)
        .unwrap_or(false);
    if is_github {
        app.show_error("Revoking approval isn't supported on GitHub".to_string());
        return true;
    }
    selected_mr(app, ConfirmAction::RevokeMr)
}

fn delete_mr(app: &mut App) -> bool {
    let is_github = app
        .gitlab_client
        .as_ref()
        .map(|c| c.is_github)
        .unwrap_or(false);
    if is_github {
        app.show_error("GitHub does not support deleting pull requests".to_string());
        return true;
    }
    selected_mr(app, ConfirmAction::DeleteMr)
}

fn cancel_pipeline(app: &mut App) -> bool {
    let Some(selected_idx) = app.pipelines.state.selected() else {
        return true;
    };
    let Some(pipe_id) = app.filtered_pipelines().get(selected_idx).map(|p| p.id()) else {
        return true;
    };
    let project_path = app.project_path_for_pipeline(pipe_id);
    app.submit_dialog = Some(SubmitDialog::build_with_project(
        ConfirmAction::CancelPipeline(pipe_id),
        project_path,
        app,
    ));
    true
}

fn cancel_jobs(app: &mut App) -> bool {
    let Some(idx) = app.jobs.state.selected() else {
        return true;
    };
    let Some(job_id) = app.filtered_jobs().get(idx).map(|j| j.id()) else {
        return true;
    };
    let pipe_id = app.active_pipeline_id.unwrap_or(0);
    let project_path = app.project_path_for_pipeline(pipe_id);
    let job_ids = if app.selected_jobs.is_empty() {
        vec![job_id]
    } else {
        let mut ids: Vec<u64> = app.selected_jobs.iter().copied().collect();
        ids.sort_unstable();
        ids
    };
    app.submit_dialog = Some(SubmitDialog::build_with_project(
        ConfirmAction::CancelJobs {
            pipeline_id: pipe_id,
            job_ids,
        },
        project_path,
        app,
    ));
    true
}

fn delete_release(app: &mut App) -> bool {
    let Some(release) = app.releases.cursor_row() else {
        return true;
    };
    app.submit_dialog = Some(SubmitDialog::build_with_project(
        ConfirmAction::DeleteRelease(release.tag_name.clone()),
        release.project_or(&app.scope).to_string(),
        app,
    ));
    true
}

fn selected_milestone(app: &mut App, build: fn(u64) -> ConfirmAction) -> bool {
    let Some(selected_idx) = app.milestones.state.selected() else {
        return true;
    };
    let filtered = app.filtered_milestones();
    let Some(milestone) = filtered.get(selected_idx) else {
        return true;
    };
    let milestone_iid = milestone.iid;
    let project_path = app.scope.project_of(&milestone.project_path).to_string();
    app.submit_dialog = Some(SubmitDialog::build_with_project(
        build(milestone_iid),
        project_path,
        app,
    ));
    true
}

fn delete_branch(app: &mut App) -> bool {
    if !app.selected_branches.is_empty() {
        let default_name = app
            .branches
            .items()
            .iter()
            .find(|b| b.default)
            .map(|b| b.name.as_str());
        let mut names: Vec<String> = app
            .selected_branches
            .iter()
            .filter(|n| default_name != Some(n.as_str()))
            .cloned()
            .collect();
        names.sort();
        if names.len() == 1 {
            app.submit_dialog = Some(SubmitDialog::build(
                ConfirmAction::DeleteBranch(names.remove(0)),
                app,
            ));
        } else if !names.is_empty() {
            app.submit_dialog = Some(SubmitDialog::build(
                ConfirmAction::BulkDeleteBranches(names),
                app,
            ));
        }
    } else if let Some(branch) = app.branches.cursor_row() {
        app.submit_dialog = Some(SubmitDialog::build(
            ConfirmAction::DeleteBranch(branch.name.clone()),
            app,
        ));
    }
    true
}

fn stop_environment(app: &mut App) -> bool {
    let Some(environment) = app.environments.cursor_row().cloned() else {
        return true;
    };
    app.submit_dialog = Some(SubmitDialog::build(
        ConfirmAction::StopEnvironment(Box::new(environment)),
        app,
    ));
    true
}

fn rollback_deployment(app: &mut App) -> bool {
    let Some(environment) = app.environments.cursor_row().cloned() else {
        return true;
    };
    match app.selected_deployment().cloned() {
        Some(deployment) if !app.is_github() && deployment.job_id().is_none() => {
            let iid = deployment.iid;
            app.show_error(format!("Deployment #{iid} has no deploy job to re-run"));
        }
        Some(deployment) => {
            app.submit_dialog = Some(SubmitDialog::build(
                ConfirmAction::RollbackDeployment {
                    environment: environment.name,
                    deployment: Box::new(deployment),
                },
                app,
            ));
        }
        None => app.show_error(NO_DEPLOYMENT_SELECTED.to_string()),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::KeyContext;

    #[test]
    fn every_destructive_action_opens_a_destructive_dialog() {
        for action in Action::ALL.iter().filter(|a| a.is_destructive()) {
            let mut app = super::super::tabs::tests::app_for_destructive_action(*action);
            assert_eq!(
                action.context(),
                KeyContext::Tab(app.active_tab),
                "{action:?} is not a tab action"
            );
            assert!(
                confirmation(&mut app, *action),
                "{action:?} did not open a dialog"
            );
            let dialog = app
                .submit_dialog
                .as_ref()
                .unwrap_or_else(|| panic!("{action:?} opened no dialog"));
            assert!(
                dialog.action.is_destructive(),
                "{action:?} opened a non-destructive dialog"
            );
        }
    }
}
