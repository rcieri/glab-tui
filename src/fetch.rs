use crate::app;
use crate::domain;
use crate::event::Event;
use crate::git_helpers::get_current_branch;

/// Derive `workflow` for every MR in place.
///
/// Called from three sites: the live fetch path below, and both cache-load
/// paths in `main.rs`. `workflow` is `#[serde(skip)]` (it is a derived value,
/// never persisted), so an `MergeRequest` deserialized straight from the
/// on-disk cache always arrives with `workflow: None` — even though
/// `approval`, which the cascade reads from, *is* persisted and survives the
/// round trip. Without calling this after a cache load, a permanently
/// offline session would show real cached Approval/Mergeable values next to
/// a uniformly `—` Workflow column, which reads as "could not determine"
/// when the data to determine it was sitting right there.
pub fn derive_workflow(mrs: &mut [crate::domain::mr::MergeRequest]) {
    for mr in mrs.iter_mut() {
        let ap = mr.approval.as_ref();
        let assignees: Vec<String> = mr.assignees.iter().map(|a| a.username.clone()).collect();
        let reviewers: Vec<String> = mr.reviewers.iter().map(|r| r.username.clone()).collect();
        mr.workflow =
            crate::domain::mr_state::workflow_status(&crate::domain::mr_state::WorkflowInputs {
                current_user: ap.and_then(|a| a.current_user.as_deref()),
                author: &mr.author.username,
                assignees: &assignees,
                reviewers: &reviewers,
                changes_requested: ap.map(|a| a.changes_requested).unwrap_or(false),
                approved: ap.map(|a| a.approved).unwrap_or(false),
                you_approved: ap.map(|a| a.you_approved).unwrap_or(false),
                you_reviewed: ap.map(|a| a.you_reviewed).unwrap_or(false),
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::mr::{Author, MergeRequest};
    use crate::domain::mr_state::{ApprovalState, WorkflowStatus};

    fn mr_fixture(iid: u64, author: &str, approval: Option<ApprovalState>) -> MergeRequest {
        MergeRequest {
            iid,
            title: format!("mr {iid}"),
            state: "opened".to_string(),
            labels: vec![],
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            author: Author {
                username: author.to_string(),
            },
            milestone: None,
            assignees: vec![],
            reviewers: vec![],
            target_branch: "main".to_string(),
            source_branch: "feature".to_string(),
            draft: false,
            description: None,
            head_pipeline: None,
            blocking_discussions_resolved: None,
            approval,
            mergeability: None,
            workflow: None,
        }
    }

    #[test]
    fn derive_workflow_fills_in_a_status_from_cached_approval_state() {
        // The cache-load regression: `workflow` is `#[serde(skip)]`, so a
        // deserialized MR always arrives with `workflow: None`, even when
        // its `approval` (which the cascade reads) survived the round trip
        // intact. This must not stay `—` forever offline.
        let mut mrs = vec![mr_fixture(
            1,
            "chandler.anderson",
            Some(ApprovalState {
                current_user: Some("chandler.anderson".to_string()),
                ..Default::default()
            }),
        )];

        derive_workflow(&mut mrs);

        assert_eq!(mrs[0].workflow, Some(WorkflowStatus::YourMergeRequest));
    }

    #[test]
    fn derive_workflow_leaves_none_when_approval_state_is_unknown() {
        // No `approval` means no `current_user`, so the cascade is
        // unanswerable and must stay `None` — never a guessed status.
        let mut mrs = vec![mr_fixture(2, "someone", None)];

        derive_workflow(&mut mrs);

        assert_eq!(mrs[0].workflow, None);
    }
}

pub fn spawn_fetch_repo_attributes(
    client: &domain::client::GitlabClient,
    project_context: &str,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let client = client.clone();
    let project_context = project_context.to_string();
    tokio::spawn(async move {
        let (labels_res, members_res) = tokio::join!(
            client.fetch_labels(&project_context),
            client.fetch_members(&project_context),
        );
        let labels = labels_res.unwrap_or_default();
        let members = members_res.unwrap_or_default();
        let _ = tx.send(Event::RepoAttributesFetched { labels, members });
    });
}

pub fn spawn_refresh_active_tab(
    client: &domain::client::GitlabClient,
    project_context: &str,
    tab: app::Tab,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let mut client = client.clone();
    client.tx = None; // suppress terminal log for background fetches
    let project_context = project_context.to_string();
    tokio::spawn(async move {
        match tab {
            app::Tab::Issues => {
                match domain::issues::list_issues(&client, &project_context, true).await {
                    Ok(issues) => {
                        let _ = tx.send(Event::IssuesFetched(issues));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch issues: {}", e),
                        ));
                    }
                }
            }
            app::Tab::MergeRequests => {
                match domain::mr::list_mrs(&client, &project_context, true).await {
                    Ok(mut mrs) => {
                        // GitHub already populated both axes during list_mrs.
                        // GitLab needs one bulk GraphQL call for the same iids.
                        if !client.is_github && !mrs.is_empty() {
                            let iids: Vec<u64> = mrs.iter().map(|m| m.iid).collect();
                            // A failure here leaves both axes None, which renders
                            // as "—". Deliberately not surfaced as an error: on an
                            // unsupported GitLab this would fire every refresh.
                            if let Ok(state) = client.list_mr_state(&project_context, &iids).await {
                                for mr in mrs.iter_mut() {
                                    if let Some((approval, mergeability)) = state.get(&mr.iid) {
                                        mr.approval = approval.clone();
                                        mr.mergeability = mergeability.clone();
                                    }
                                }
                            }
                        }
                        // Derive the workflow status once the approval state
                        // is merged, since the cascade reads from it.
                        derive_workflow(&mut mrs);
                        let _ = tx.send(Event::MrsFetched(mrs));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch MRs: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Pipelines => {
                match domain::pipelines::list_pipelines(&client, &project_context).await {
                    Ok(pipelines) => {
                        let _ = tx.send(Event::PipelinesFetched(pipelines));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch pipelines: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Runners => {
                match domain::runners::list_runners(&client, &project_context).await {
                    Ok(runners) => {
                        let _ = tx.send(Event::RunnersFetched(runners));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch runners: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Releases => {
                match domain::releases::list_releases(&client, &project_context).await {
                    Ok(releases) => {
                        let _ = tx.send(Event::ReleasesFetched(releases));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch releases: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Todos => {
                match domain::notifications::list_notifications(&client, true).await {
                    Ok(notifs) => {
                        let _ = tx.send(Event::TodosFetched(notifs));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch notifications: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Jobs => {
                let branch_name = get_current_branch();
                let mut found_pipeline_id = None;

                if let Some(branch) = &branch_name {
                    let mr_iid = match domain::mr::list_mrs(&client, &project_context, false).await
                    {
                        Ok(mrs) => mrs
                            .into_iter()
                            .find(|m| &m.source_branch == branch)
                            .map(|m| m.iid),
                        Err(_) => None,
                    };

                    if let Ok(pipelines) =
                        domain::pipelines::list_pipelines(&client, &project_context).await
                    {
                        let target_ref =
                            mr_iid.map(|iid| format!("refs/merge-requests/{}/head", iid));
                        if let Some(pipeline) = pipelines.into_iter().find(|p| {
                            p.ref_branch() == branch
                                || target_ref.as_ref().map_or(false, |tr| p.ref_branch() == tr)
                        }) {
                            found_pipeline_id = Some(pipeline.id());
                        }
                    }
                }

                if let Some(pipeline_id) = found_pipeline_id {
                    match domain::pipelines::list_pipeline_jobs(
                        &client,
                        &project_context,
                        pipeline_id,
                    )
                    .await
                    {
                        Ok(jobs) => {
                            let _ = tx.send(Event::JobsTabFetched(pipeline_id, jobs));
                        }
                        Err(e) => {
                            let _ = tx.send(Event::FetchFailed(
                                tab,
                                format!("Failed to fetch jobs for pipeline {}: {}", pipeline_id, e),
                            ));
                        }
                    }
                } else {
                    let _ = tx.send(Event::FetchFailed(
                        tab,
                        "No pipeline found for the current branch/MR.".to_string(),
                    ));
                }
            }
            app::Tab::Milestones => {
                match domain::milestones::list_milestones(&client, &project_context).await {
                    Ok(milestones) => {
                        let _ = tx.send(Event::MilestonesFetched(milestones));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch milestones: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Branches => {
                match domain::branches::list_branches(&client, &project_context).await {
                    Ok(branches) => {
                        let _ = tx.send(Event::BranchesFetched(branches));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch branches: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Environments => {
                match domain::deployments::list_environments(&client, &project_context).await {
                    Ok(envs) => {
                        let _ = tx.send(Event::EnvironmentsFetched(envs));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch environments: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Terminal => {}
        }
    });
}
