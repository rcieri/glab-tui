//! Opens the confirmation dialog for a destructive action's current
//! selection, for every [`Action`] whose registry row is `destructive`.
//! `handlers::tabs::handle_active_tab_key` runs this before the per-tab
//! dispatch, so remapping a destructive action can never skip its dialog.

use crate::app::{App, ConfirmAction, SubmitDialog};
use crate::domain::milestones::Milestone;
use crate::handlers::tabs::NO_DEPLOYMENT_SELECTED;
use crate::keymap::Action;

/// Builds and opens the dialog for `action`'s current selection.
pub fn confirmation(app: &mut App, action: Action) {
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
        Action::TodosMarkAllAsRead => crate::handlers::todos::confirm_mark_all_read(app),
        _ => {}
    }
}

fn selected_issue(app: &mut App, build: fn(u64) -> ConfirmAction) {
    let Some(selected_idx) = app.issues.state.selected() else {
        return;
    };
    let filtered = app.filtered_issues();
    let Some(issue) = filtered.get(selected_idx) else {
        return;
    };
    let issue_iid = issue.iid;
    let project_path = app.scope.project_of(&issue.project_path).to_string();
    app.open_submit_dialog(SubmitDialog::build_with_project(
        build(issue_iid),
        project_path,
        app,
    ));
}

fn selected_mr(app: &mut App, build: fn(u64) -> ConfirmAction) {
    let Some(selected_idx) = app.mrs.state.selected() else {
        return;
    };
    let Some(mr) = app.filtered_mrs().get(selected_idx).cloned().cloned() else {
        return;
    };
    let project_path = app.scope.project_of(&mr.project_path).to_string();
    app.open_submit_dialog(SubmitDialog::build_with_project(
        build(mr.iid),
        project_path,
        app,
    ));
}

fn revoke_mr(app: &mut App) {
    if app.is_github() {
        app.show_error("Revoking approval isn't supported on GitHub".to_string());
        return;
    }
    selected_mr(app, ConfirmAction::RevokeMr)
}

fn delete_mr(app: &mut App) {
    if app.is_github() {
        app.show_error("GitHub does not support deleting pull requests".to_string());
        return;
    }
    selected_mr(app, ConfirmAction::DeleteMr)
}

fn cancel_pipeline(app: &mut App) {
    let Some(selected_idx) = app.pipelines.state.selected() else {
        return;
    };
    let Some(pipe_id) = app.filtered_pipelines().get(selected_idx).map(|p| p.id()) else {
        return;
    };
    let project_path = app.project_path_for_pipeline(pipe_id);
    app.open_submit_dialog(SubmitDialog::build_with_project(
        ConfirmAction::CancelPipeline(pipe_id),
        project_path,
        app,
    ));
}

fn cancel_jobs(app: &mut App) {
    let Some(idx) = app.jobs.state.selected() else {
        return;
    };
    let Some(job_id) = app.filtered_jobs().get(idx).map(|j| j.id()) else {
        return;
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
    app.open_submit_dialog(SubmitDialog::build_with_project(
        ConfirmAction::CancelJobs {
            pipeline_id: pipe_id,
            job_ids,
        },
        project_path,
        app,
    ));
}

fn delete_release(app: &mut App) {
    let Some(release) = app.releases.cursor_row() else {
        return;
    };
    app.open_submit_dialog(SubmitDialog::build_with_project(
        ConfirmAction::DeleteRelease(release.tag_name.clone()),
        release.project_or(&app.scope).to_string(),
        app,
    ));
}

fn selected_milestone(app: &mut App, build: fn(Milestone) -> ConfirmAction) {
    let Some(selected_idx) = app.milestones.state.selected() else {
        return;
    };
    let filtered = app.filtered_milestones();
    let Some(milestone) = filtered.get(selected_idx) else {
        return;
    };
    let milestone = (*milestone).clone();
    let project_path = app.scope.project_of(&milestone.project_path).to_string();
    app.open_submit_dialog(SubmitDialog::build_with_project(
        build(milestone),
        project_path,
        app,
    ));
}

fn delete_branch(app: &mut App) {
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
            app.open_submit_dialog(SubmitDialog::build(
                ConfirmAction::DeleteBranch(names.remove(0)),
                app,
            ));
        } else if !names.is_empty() {
            app.open_submit_dialog(SubmitDialog::build(
                ConfirmAction::BulkDeleteBranches(names),
                app,
            ));
        }
    } else if let Some(branch) = app.branches.cursor_row() {
        app.open_submit_dialog(SubmitDialog::build(
            ConfirmAction::DeleteBranch(branch.name.clone()),
            app,
        ));
    }
}

fn stop_environment(app: &mut App) {
    let Some(environment) = app.environments.cursor_row().cloned() else {
        return;
    };
    app.open_submit_dialog(SubmitDialog::build(
        ConfirmAction::StopEnvironment(Box::new(environment)),
        app,
    ));
}

fn rollback_deployment(app: &mut App) {
    let Some(environment) = app.environments.cursor_row().cloned() else {
        return;
    };
    match app.selected_deployment().cloned() {
        Some(deployment) if !app.is_github() && deployment.job_id().is_none() => {
            let iid = deployment.iid;
            app.show_error(format!("Deployment #{iid} has no deploy job to re-run"));
        }
        Some(deployment) => {
            app.open_submit_dialog(SubmitDialog::build(
                ConfirmAction::RollbackDeployment {
                    environment: environment.name,
                    deployment: Box::new(deployment),
                },
                app,
            ));
        }
        None => app.show_error(NO_DEPLOYMENT_SELECTED.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::tabs::tests::{app_for_destructive_action, dispatch};
    use crate::keymap::KeyContext;
    use crossterm::event::{KeyCode, KeyEvent};

    #[test]
    fn every_destructive_action_remapped_to_f9_opens_a_destructive_dialog() {
        for action in Action::ALL.iter().filter(|a| a.is_destructive()) {
            let mut app = app_for_destructive_action(*action);
            assert_eq!(
                action.context(),
                KeyContext::Tab(app.active_tab),
                "{action:?} is not a tab action"
            );
            app.config.keybindings.set(*action, &["F9"]);

            dispatch(&mut app, &KeyEvent::from(KeyCode::F(9)));

            let dialog = app
                .submit_dialog()
                .unwrap_or_else(|| panic!("{action:?} opened no dialog"));
            assert!(
                dialog.action.is_destructive(),
                "{action:?} opened a non-destructive dialog"
            );
        }
    }
}
