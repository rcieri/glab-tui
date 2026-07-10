use crate::app;
use crate::event::Event;
use crate::git_helpers::get_current_branch;
use crate::gitlab;

pub fn spawn_refresh_active_tab(
    client: &gitlab::client::GitlabClient,
    project_context: &str,
    tab: app::Tab,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let client = client.clone();
    let project_context = project_context.to_string();
    tokio::spawn(async move {
        match tab {
            app::Tab::Issues => {
                match gitlab::issues::list_issues(&client, &project_context, true).await {
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
                let client_for_pipelines = client.clone();
                let project_context_for_pipelines = project_context.clone();
                let tx_for_pipelines = tx.clone();
                match gitlab::mr::list_mrs(&client, &project_context, true).await {
                    Ok(mrs) => {
                        let _ = tx.send(Event::MrsFetched(mrs));
                        if client_for_pipelines.is_github {
                            tokio::spawn(async move {
                                if let Ok(pipelines) = gitlab::pipelines::list_pipelines(
                                    &client_for_pipelines,
                                    &project_context_for_pipelines,
                                )
                                .await
                                {
                                    let _ =
                                        tx_for_pipelines.send(Event::PipelinesFetched(pipelines));
                                }
                            });
                        }
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
                match gitlab::pipelines::list_pipelines(&client, &project_context).await {
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
                match gitlab::runners::list_runners(&client, &project_context).await {
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
                match gitlab::releases::list_releases(&client, &project_context).await {
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
                match gitlab::notifications::list_notifications(&client, true).await {
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
                    let mr_iid = match gitlab::mr::list_mrs(&client, &project_context, false).await
                    {
                        Ok(mrs) => mrs
                            .into_iter()
                            .find(|m| &m.source_branch == branch)
                            .map(|m| m.iid),
                        Err(_) => None,
                    };

                    if let Ok(pipelines) =
                        gitlab::pipelines::list_pipelines(&client, &project_context).await
                    {
                        let target_ref =
                            mr_iid.map(|iid| format!("refs/merge-requests/{}/head", iid));
                        if let Some(pipeline) = pipelines.into_iter().find(|p| {
                            &p.r#ref == branch
                                || target_ref.as_ref().map_or(false, |tr| &p.r#ref == tr)
                        }) {
                            found_pipeline_id = Some(pipeline.id);
                        }
                    }
                }

                if let Some(pipeline_id) = found_pipeline_id {
                    match gitlab::pipelines::list_pipeline_jobs(
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
                match gitlab::milestones::list_milestones(&client, &project_context).await {
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
                match gitlab::branches::list_branches(&client, &project_context).await {
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
                match gitlab::deployments::list_environments(&client, &project_context).await {
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

/// Fetch comments for the currently selected item if the detail pane is open.
/// Called on tab refresh (every ~60s) and when the detail pane is first opened.
pub fn spawn_comment_refresh(
    app: &mut crate::app::App,
    client: &gitlab::client::GitlabClient,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    match app.active_tab {
        app::Tab::Issues => {
            if let Some(idx) = app.issues.state.selected() {
                if let Some(issue) = app.filtered_issues().get(idx) {
                    let iid = issue.iid;
                    if !app.issue_comments.contains_key(&iid)
                        && app.fetching_issue_comments != Some(iid)
                    {
                        app.fetching_issue_comments = Some(iid);
                        let client = client.clone();
                        let project_context = app.project_context.clone();
                        tokio::spawn(async move {
                            let discussions = gitlab::discussions::list_issue_discussions(
                                &client,
                                &project_context,
                                iid,
                            )
                            .await
                            .unwrap_or_default();
                            let _ = tx.send(Event::IssueCommentsFetched { iid, discussions });
                        });
                    }
                }
            }
        }
        app::Tab::MergeRequests => {
            if let Some(idx) = app.mrs.state.selected() {
                if let Some(mr) = app.filtered_mrs().get(idx) {
                    let iid = mr.iid;
                    if !app.mr_comments.contains_key(&iid) && app.fetching_mr_comments != Some(iid)
                    {
                        app.fetching_mr_comments = Some(iid);
                        let client = client.clone();
                        let project_context = app.project_context.clone();
                        tokio::spawn(async move {
                            let discussions = gitlab::discussions::list_mr_discussions(
                                &client,
                                &project_context,
                                iid,
                            )
                            .await
                            .unwrap_or_default();
                            let _ = tx.send(Event::MrCommentsFetched { iid, discussions });
                        });
                    }
                }
            }
        }
        _ => {}
    }
}
