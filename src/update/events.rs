use crate::app::App;
use crate::effect::{Effects, ForegroundJob};
use crate::event::{CommandOutcome, Event};
use crate::fetch::spawn_refresh_active_tab;
use crate::{app, handlers};
use tokio::sync::mpsc::UnboundedSender;

/// Applies a background result or a mouse event.
pub(super) fn apply(app: &mut App, event: Event, tx: &UnboundedSender<Event>, fx: &mut Effects) {
    match event {
        Event::PipelineOpened {
            pipeline_id,
            opening,
            result,
        } => {
            handlers::tabs::apply_opened_pipeline(app, pipeline_id, opening, result);
        }
        Event::ClientReady { scope, result } => {
            handlers::scope::apply_client_ready(app, scope, result, tx, fx);
        }
        Event::ChildLevelFetched(parent_id, level) => {
            // Drop a reply that arrives after the user has already
            // moved to another level.
            if app.current_parent_id() == Some(parent_id) {
                let cursor = app.pipelines.state.selected();
                app.pipelines.items = level.children;
                app.pending_triggers = level.pending;
                let selected = if app.pipelines.items.is_empty() {
                    None
                } else {
                    Some(cursor.unwrap_or(0).min(app.pipelines.items.len() - 1))
                };
                app.pipelines.state.select(selected);
            }
        }
        Event::PipelineJobs(id, jobs) => {
            app.fetching_pipelines.remove(&id);
            app.pipeline_jobs.insert(id, jobs.clone());

            let mut is_active = false;
            if app.active_tab == app::Tab::Jobs && app.active_pipeline_id == Some(id) {
                is_active = true;
            } else if app.active_tab == app::Tab::Pipelines {
                if let Some(idx) = app.pipelines.state.selected() {
                    if app.filtered_pipelines().get(idx).map(|p| p.id()) == Some(id) {
                        is_active = true;
                    }
                }
            }

            if is_active {
                app.jobs.items = jobs;
                app.jobs.state.select(app.jobs.state.selected().or(Some(0)));
            }

            app.project_cache.pipeline_jobs = app.pipeline_jobs.clone();
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::JobsTabFetched(pipeline_id, jobs) => {
            app.complete_loading_tab(app::Tab::Jobs, "Success");
            app.status_message = None;
            app.loaded_tabs.insert(app::Tab::Jobs);
            app.jobs.items = jobs;
            app.active_pipeline_id = Some(pipeline_id);
            app.jobs.state.select(Some(0));
            app.detail_scroll = 0;
            app.job_trace = None;
        }
        Event::JobTraceFetched(job_id, result) => {
            app.job_trace_loading = false;
            let opens_editor = app.pending_job_trace_editor == Some(job_id);
            if opens_editor {
                app.pending_job_trace_editor = None;
            }
            let jumps_to_failure = app.pending_job_trace_failure == Some(job_id);
            if jumps_to_failure {
                app.pending_job_trace_failure = None;
            }
            if app.trace_job_id() == Some(job_id) {
                match result {
                    Ok(trace) => {
                        app.job_trace = Some(crate::ui::job_trace::JobTraceView::new(trace));
                        app.detail_scroll_to_bottom = app.job_trace_follow;
                        app.job_trace_last_refresh = std::time::Instant::now();
                        app.details_zoomed = true;
                        app.detail_visible = true;
                        if jumps_to_failure {
                            app.jump_to_trace_failure(None);
                        }
                        if opens_editor {
                            fx.suspend(ForegroundJob::ViewJobTrace { job_id });
                        }
                    }
                    Err(e) => {
                        app.show_error(e);
                        app.error_has_cli_detail = true;
                    }
                }
            }
        }
        Event::JobTraceFollowed(job_id, result) => {
            app.job_trace_poll_in_flight = false;
            app.job_trace_last_refresh = std::time::Instant::now();
            if app.trace_job_id() == Some(job_id) {
                match result {
                    Ok(update) => {
                        let has_new_text = update.has_new_text();
                        let is_applied = app
                            .job_trace
                            .as_mut()
                            .is_some_and(|view| view.apply(update));
                        if is_applied && has_new_text && app.job_trace_follow {
                            app.detail_scroll_to_bottom = true;
                        }
                    }
                    Err(e) => {
                        app.show_error(e);
                        app.error_has_cli_detail = true;
                    }
                }
            }
        }
        // Listed before the State filter widened: showing these rows
        // would pass a narrower list off as the wider view. The
        // refetch the filter change started replaces them.
        Event::IssuesFetched(listed, _) if !app.covers_state_filter(app::Tab::Issues, listed) => {}
        Event::MrsFetched(listed, _)
            if !app.covers_state_filter(app::Tab::MergeRequests, listed) => {}
        Event::IssuesFetched(listed, issues) => {
            app.complete_loading_tab(app::Tab::Issues, "Success");
            app.loaded_tabs.insert(app::Tab::Issues);
            app.refreshed_tabs.insert(app::Tab::Issues);
            app.stale_tabs.remove(&app::Tab::Issues);
            app.issues.items = issues;
            crate::fetch::sync_linked_references(&mut app.issues.items, &mut app.mrs.items);
            if let Some(target_iid) = app.pending_issue_select.take() {
                app.select_visible_issue(None, target_iid);
            }
            app.rebuild_milestone_progress_cache();
            app.update_filter_selection();
            crate::handlers::tabs::maybe_fetch_related_mrs(app, tx);
            app.project_cache.issues = app.issues.items.clone();
            app.project_cache.issues_state_query = listed;
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::MrsFetched(listed, mrs) => {
            app.complete_loading_tab(app::Tab::MergeRequests, "Success");
            app.loaded_tabs.insert(app::Tab::MergeRequests);
            app.refreshed_tabs.insert(app::Tab::MergeRequests);
            app.stale_tabs.remove(&app::Tab::MergeRequests);
            app.mrs.items = mrs;
            app.apply_pr_stacks();
            crate::fetch::sync_linked_references(&mut app.issues.items, &mut app.mrs.items);
            if let Some(target_iid) = app.pending_mr_select.take() {
                app.select_visible_mr(None, target_iid);
            }
            app.update_filter_selection();
            if app.is_stack_column_needed() {
                if let Some(client) = &app.gitlab_client {
                    crate::fetch::spawn_fetch_pr_stack_summaries(
                        fx,
                        client,
                        app.scope.as_str(),
                        app.mrs.items.iter().map(|mr| mr.iid).collect(),
                        app.scoped_sender(tx.clone()),
                    );
                }
            }
            crate::handlers::tabs::maybe_fetch_mr_related_issues(app, tx);
            app.project_cache.mrs = app.mrs.items.clone();
            app.project_cache.mrs_state_query = listed;
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::IssueFetched(iid, Ok(issue)) => {
            let project = issue.project_path.clone();
            let is_listed = app
                .issues
                .items
                .iter()
                .any(|i| i.iid == iid && app.is_listed_in(&i.project_path, Some(&project)));
            if !is_listed {
                app.issues.items.push(*issue);
                app.update_filter_selection();
            }
            app.focus_issue(Some(&project), iid);
            crate::handlers::tabs::maybe_fetch_related_mrs(app, tx);
            app.project_cache.issues = app.issues.items.clone();
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::IssueFetched(iid, Err(err)) => {
            app.show_error(format!("Failed to fetch issue #{iid}: {err}"));
        }
        Event::MrFetched(iid, Ok(mr)) => {
            let project = mr.project_path.clone();
            let is_listed = app
                .mrs
                .items
                .iter()
                .any(|m| m.iid == iid && app.is_listed_in(&m.project_path, Some(&project)));
            if !is_listed {
                app.mrs.items.push(*mr);
                app.apply_pr_stacks();
                app.update_filter_selection();
            }
            app.focus_mr(Some(&project), iid);
            crate::handlers::tabs::maybe_fetch_mr_related_issues(app, tx);
            app.project_cache.mrs = app.mrs.items.clone();
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::MrFetched(iid, Err(err)) => {
            app.show_error(format!("Failed to fetch MR #{iid}: {err}"));
        }
        Event::PrStackFetched {
            pr_number,
            project_path,
            result,
        } => {
            let pr = (project_path, pr_number);
            app.fetching_pr_stacks.remove(&pr);
            let awaited = app
                .pending_stack_action
                .take_if(|(pending, _)| *pending == pr)
                .map(|(_, action)| action);
            if awaited.is_some() {
                app.status_message = None;
            }
            match result {
                Ok(stack) => {
                    app.record_pr_stack(&pr.0, pr.1, stack);
                    app.update_filter_selection();
                    if let Some(action) = awaited {
                        if app.selector.is_none()
                            && app.submit_dialog.is_none()
                            && app.selected_mr_ref().as_ref() == Some(&pr)
                        {
                            crate::handlers::tabs::run_stack_action(app, &pr.0, pr.1, action);
                        }
                    }
                }
                // The inspector simply keeps showing no stack; only an
                // explicit key press is owed an answer.
                Err(e) if awaited.is_some() => {
                    app.show_error(format!("Failed to fetch the stack of PR #{}: {e}", pr.1));
                }
                Err(_) => {}
            }
        }
        Event::PrStackSummariesFetched {
            project_path,
            pr_numbers,
            stacks,
        } => {
            app.record_pr_stack_summaries(&project_path, &pr_numbers, stacks);
            app.update_filter_selection();
        }
        Event::PipelinesFetched(pipelines) => {
            app.complete_loading_tab(app::Tab::Pipelines, "Success");
            app.loaded_tabs.insert(app::Tab::Pipelines);
            app.refreshed_tabs.insert(app::Tab::Pipelines);
            app.stale_tabs.remove(&app::Tab::Pipelines);
            app.drop_jobs_of_changed_pipelines(&pipelines);
            if app.nav_depth() > 0 {
                // Inside a descent the visible list belongs to a child
                // level. A freshly fetched top-level list belongs to
                // the outermost frame, so it lands there and shows up
                // when the user ascends back out.
                app.replace_root_level(pipelines);
            } else {
                app.pipelines.items = pipelines;
                if let Some(pipe_id) = app.pending_pipeline_select.take() {
                    if let Some(idx) = app.pipelines.items.iter().position(|p| p.id() == pipe_id) {
                        app.pipelines.state.select(Some(idx));
                    }
                }
            }
            app.update_filter_selection();
            let new_ids: std::collections::HashSet<u64> =
                app.pipelines.items.iter().map(|p| p.id()).collect();
            app.pipeline_jobs.retain(|id, _| new_ids.contains(id));
            app.fetching_pipelines.clear();
            app.project_cache.pipelines = app.pipelines.items.clone();
            app.project_cache.pipeline_jobs = app.pipeline_jobs.clone();
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::TodosFetched { show_read, .. } if show_read != app.shows_read_todos() => {}
        Event::TodosFetched { notifications, .. } => {
            app.complete_loading_tab(app::Tab::Todos, "Success");
            app.loaded_tabs.insert(app::Tab::Todos);
            app.refreshed_tabs.insert(app::Tab::Todos);
            app.stale_tabs.remove(&app::Tab::Todos);
            app.set_todos(notifications);
            app.update_filter_selection();
            app.project_cache.todos = app.todos.items.clone();
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::TodoChanged(change, Ok(())) => {
            app.apply_todo_change(&change);
            app.project_cache.todos = app.todos.items.clone();
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::TodoChanged(change, Err(err)) => {
            app.show_error(change.failure_message(app.kind(), &err));
            // CLI failure: full stderr is in the terminal log.
            app.error_has_cli_detail = true;
        }
        Event::TodoDetailFetched {
            id,
            detail_api_path,
            result,
        } => {
            app.apply_todo_detail(&id, &detail_api_path, result);
        }
        Event::RunnersFetched(runners) => {
            app.complete_loading_tab(app::Tab::Runners, "Success");
            app.loaded_tabs.insert(app::Tab::Runners);
            app.refreshed_tabs.insert(app::Tab::Runners);
            app.stale_tabs.remove(&app::Tab::Runners);
            app.runners.set_items(runners);
            app.update_filter_selection();
            app.project_cache.runners = app.runners.items().to_vec();
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::ReleasesFetched(releases) => {
            app.complete_loading_tab(app::Tab::Releases, "Success");
            app.loaded_tabs.insert(app::Tab::Releases);
            app.refreshed_tabs.insert(app::Tab::Releases);
            app.stale_tabs.remove(&app::Tab::Releases);
            app.releases.set_items(releases);
            app.update_filter_selection();
            app.project_cache.releases = app.releases.items().to_vec();
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::MilestonesFetched(milestones) => {
            app.complete_loading_tab(app::Tab::Milestones, "Success");
            app.loaded_tabs.insert(app::Tab::Milestones);
            app.refreshed_tabs.insert(app::Tab::Milestones);
            app.stale_tabs.remove(&app::Tab::Milestones);
            app.milestones.items = milestones;
            app.update_filter_selection();
            app.project_cache.milestones = app.milestones.items.clone();
            let listed_iids: std::collections::HashSet<u64> =
                app.milestones.items.iter().map(|m| m.iid).collect();
            app.milestone_issues_cache
                .retain(|iid, _| listed_iids.contains(iid));
            app.project_cache
                .milestone_issues
                .retain(|iid, _| listed_iids.contains(iid));
            app.rebuild_milestone_progress_cache();
            if let Some(iid) = app.selected_milestone_iid {
                if !listed_iids.contains(&iid) {
                    app.selected_milestone_iid = None;
                    app.selected_milestone_issues = None;
                } else if let Some(cached) = app.milestone_issues_cache.get(&iid) {
                    app.selected_milestone_issues = Some(cached.clone());
                }
            }
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);

            if app.scope.is_repository() {
                let client = app.gitlab_client.clone();
                let tx = app.scoped_sender(tx.clone());
                let repo_path = app.scope.as_str().to_string();
                let milestones = app.milestones.items.clone();
                if let Some(client) = client {
                    let prefetch = fx.spawn_cancellable(async move {
                        for (i, m) in milestones.iter().filter(|m| m.iid > 0).enumerate() {
                            if i > 0 {
                                crate::backend::rate_limit::pace_bulk_operation().await;
                            }
                            let project_context = if !m.project_path.is_empty() {
                                &m.project_path
                            } else {
                                &repo_path
                            };
                            match crate::domain::milestones::list_milestone_issues(
                                &client,
                                project_context,
                                m.iid,
                                &m.title,
                            )
                            .await
                            {
                                Ok(issues) => {
                                    if tx
                                        .send(Event::MilestoneIssuesFetched(m.iid, issues))
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    let _ = tx.send(Event::CommandCompleted(
                                        app::Tab::Milestones,
                                        Err(format!(
                                            "Failed to fetch milestone issues for '{}': {e}",
                                            m.title
                                        )),
                                    ));
                                    break;
                                }
                            }
                        }
                    });
                    app.milestone_prefetch = Some(prefetch);
                }
            }
        }
        Event::MilestoneIssuesFetched(iid, issues) => {
            let total = issues.len();
            let closed = issues.iter().filter(|i| i.state == "closed").count();
            app.milestone_progress_cache.insert(iid, (closed, total));
            app.milestone_issues_cache.insert(iid, issues.clone());
            if app.selected_milestone_iid == Some(iid) {
                app.selected_milestone_issues = Some(issues.clone());
            }
            app.project_cache.milestone_issues.insert(iid, issues);
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::MilestoneUpdated | Event::MilestoneClosed | Event::MilestoneReopened => {
            app.status_message = None;
            app.project_cache.milestones = app.milestones.items.clone();
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::MilestoneDeleted => {
            app.complete_loading_tab(app::Tab::Milestones, "Success");
            app.status_message = None;
            if let Some((project_path, iid)) = app.pending_delete_milestone.take() {
                if let Some(pos) = app.milestone_index(&project_path, iid) {
                    app.milestones.items.remove(pos);
                }
                app.milestone_issues_cache.remove(&iid);
                app.project_cache.milestone_issues.remove(&iid);
                if app.selected_milestone_iid == Some(iid) {
                    app.selected_milestone_iid = None;
                    app.selected_milestone_issues = None;
                }
            }
            app.rebuild_milestone_progress_cache();
            app.project_cache.milestones = app.milestones.items.clone();
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::ReleaseUpdated => {
            app.status_message = None;
            app.project_cache.releases = app.releases.items().to_vec();
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::ReleaseDeleted => {
            app.complete_loading_tab(app::Tab::Releases, "Success");
            app.status_message = None;
            if let Some((project, tag)) = app.pending_delete_release.take() {
                app.releases
                    .retain(|r| r.tag_name != tag || r.project_or(&app.scope) != project);
            }
            app.project_cache.releases = app.releases.items().to_vec();
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::IssueDeleted => {
            app.complete_loading_tab(app::Tab::Issues, "Success");
            app.status_message = None;
            app.issues.items.clear();
            app.milestone_progress_cache.clear();
            if let Some(client) = app.gitlab_client.clone() {
                if !app.loading_tabs.contains(&app::Tab::Issues) {
                    app.start_loading_tab(app::Tab::Issues);
                }
                spawn_refresh_active_tab(
                    fx,
                    &client,
                    &app.scope,
                    app.requested_state_queries(),
                    app::Tab::Issues,
                    app.scoped_sender(tx.clone()),
                );
            }
        }
        Event::MrDeleted => {
            app.complete_loading_tab(app::Tab::MergeRequests, "Success");
            app.status_message = None;
            app.mrs.items.clear();
            if let Some(client) = app.gitlab_client.clone() {
                if !app.loading_tabs.contains(&app::Tab::MergeRequests) {
                    app.start_loading_tab(app::Tab::MergeRequests);
                }
                spawn_refresh_active_tab(
                    fx,
                    &client,
                    &app.scope,
                    app.requested_state_queries(),
                    app::Tab::MergeRequests,
                    app.scoped_sender(tx.clone()),
                );
            }
        }
        Event::BranchesFetched(branches) => {
            app.complete_loading_tab(app::Tab::Branches, "Success");
            app.loaded_tabs.insert(app::Tab::Branches);
            app.refreshed_tabs.insert(app::Tab::Branches);
            app.stale_tabs.remove(&app::Tab::Branches);
            app.branches.set_items(branches);
            app.update_filter_selection();
            app.project_cache.branches = app.branches.items().to_vec();
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::EnvironmentsFetched(envs) => {
            app.complete_loading_tab(app::Tab::Environments, "Success");
            app.loaded_tabs.insert(app::Tab::Environments);
            app.refreshed_tabs.insert(app::Tab::Environments);
            app.stale_tabs.remove(&app::Tab::Environments);
            app.environments.set_items(envs);
            app.update_filter_selection();
            app.project_cache.environments = app.environments.items().to_vec();
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::SelectorItemsFetched(items) => {
            let mut applied_from_cache = false;
            if items.is_empty() {
                // Determine which typed cache to fall back on
                if let Some(ref selector) = app.selector {
                    let fallback = match selector.field_type.as_str() {
                        "labels" => {
                            if !app.cached_labels.is_empty() {
                                Some(app.cached_labels.clone())
                            } else {
                                None
                            }
                        }
                        "assignees" | "reviewers" => {
                            if !app.cached_members.is_empty() {
                                Some(app.cached_members.clone())
                            } else {
                                None
                            }
                        }
                        "milestone" => Some(
                            app.milestones
                                .items
                                .iter()
                                .map(|m| m.title.clone())
                                .collect(),
                        ),
                        "source_branch" | "target_branch" | "pipeline_branch" | "create_from" => {
                            Some(
                                app.branches
                                    .items()
                                    .iter()
                                    .map(|b| b.name.clone())
                                    .collect(),
                            )
                        }
                        "project" => {
                            let mut known = Vec::new();
                            for i in &app.issues.items {
                                if !i.project_path.is_empty() {
                                    known.push(i.project_path.clone());
                                }
                            }
                            for m in &app.mrs.items {
                                if !m.project_path.is_empty() {
                                    known.push(m.project_path.clone());
                                }
                            }
                            for p in &app.pipelines.items {
                                if !p.project_path.is_empty() {
                                    known.push(p.project_path.clone());
                                }
                            }
                            for ms in &app.milestones.items {
                                if !ms.project_path.is_empty() {
                                    known.push(ms.project_path.clone());
                                }
                            }
                            known.sort();
                            known.dedup();
                            if !known.is_empty() { Some(known) } else { None }
                        }
                        _ => None,
                    };
                    if let Some(cached) = fallback {
                        if !cached.is_empty() {
                            app.show_notice("Offline fallback: cached selector items.");
                            if let Some(mut selector) = app.selector.take() {
                                selector.all_items = cached;
                                selector.is_loading = false;
                                app.selector = Some(selector);
                            }
                            applied_from_cache = true;
                        }
                    }
                }
            }
            if !applied_from_cache {
                // Update typed cache based on field type
                if let Some(ref selector) = app.selector {
                    match selector.field_type.as_str() {
                        "labels" => {
                            app.cached_labels = items.clone();
                            app.project_cache.labels = items.clone();
                        }
                        "assignees" | "reviewers" => {
                            app.cached_members = items.clone();
                            app.project_cache.members = items.clone();
                        }
                        _ => {}
                    }
                }
                crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
                if let Some(mut selector) = app.selector.take() {
                    if selector.field_type == "project" {
                        let mut combined = selector.all_items;
                        combined.extend(items);
                        combined.sort();
                        combined.dedup();
                        selector.all_items = combined;
                    } else {
                        selector.all_items = items;
                    }
                    selector.is_loading = false;
                    app.selector = Some(selector);
                }
            }
        }
        Event::RepoAttributesFetched { labels, members } => {
            if !labels.is_empty() {
                let names: Vec<String> = labels.iter().map(|l| l.name.clone()).collect();
                app.cached_labels = names.clone();
                app.project_cache.labels = names;
                if app.config.fetch_label_colors {
                    let colors: std::collections::HashMap<String, String> = labels
                        .iter()
                        .filter_map(|l| l.color.as_ref().map(|c| (l.name.clone(), c.clone())))
                        .collect();
                    app.project_cache.label_colors = colors.clone();
                    app.label_colors = colors
                        .iter()
                        .filter_map(|(name, hex)| {
                            crate::config::hex_to_color(hex).map(|c| (name.clone(), c))
                        })
                        .collect();
                }
            }
            if !members.is_empty() {
                app.cached_members = members.clone();
                app.project_cache.members = members;
            }
            crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
        }
        Event::ProjectAttributesFetched {
            project,
            labels,
            members,
            milestones,
            branches,
        } => {
            let mut proj_cache = crate::utils::cache::load_cache(&project);
            let mut cache_updated = false;

            if !labels.is_empty() {
                let names: Vec<String> = labels.iter().map(|l| l.name.clone()).collect();
                proj_cache.labels = names;
                if app.config.fetch_label_colors {
                    let colors: std::collections::HashMap<String, String> = labels
                        .iter()
                        .filter_map(|l| l.color.as_ref().map(|c| (l.name.clone(), c.clone())))
                        .collect();
                    proj_cache.label_colors = colors;
                }
                cache_updated = true;
            }
            if !members.is_empty() {
                proj_cache.members = members.clone();
                cache_updated = true;
            }
            if !milestones.is_empty() {
                proj_cache.milestones = milestones
                    .iter()
                    .map(|title| crate::domain::milestones::Milestone {
                        id: 0,
                        iid: 0,
                        title: title.clone(),
                        description: None,
                        state: "active".to_string(),
                        due_date: None,
                        start_date: None,
                        created_at: String::new(),
                        project_path: project.clone(),
                    })
                    .collect();
                cache_updated = true;
            }
            if !branches.is_empty() {
                proj_cache.branches = branches
                    .iter()
                    .map(|name| crate::domain::branches::Branch {
                        name: name.clone(),
                        default: false,
                        protected: false,
                        web_url: String::new(),
                        can_push: false,
                        commit_sha: String::new(),
                        ..Default::default()
                    })
                    .collect();
                cache_updated = true;
            }
            if cache_updated {
                crate::utils::cache::save_cache(&project, &proj_cache);
            }

            if let Some(mut sel) = app.selector.take() {
                let updated = match sel.field_type.as_str() {
                    "labels" if !labels.is_empty() => {
                        sel.all_items = labels.into_iter().map(|l| l.name).collect();
                        true
                    }
                    "assignees" | "reviewers" if !members.is_empty() => {
                        sel.all_items = members;
                        true
                    }
                    "milestone" if !milestones.is_empty() => {
                        sel.all_items = milestones;
                        true
                    }
                    "source_branch" | "target_branch" | "pipeline_branch" | "create_from"
                        if !branches.is_empty() =>
                    {
                        sel.all_items = branches;
                        true
                    }
                    _ => false,
                };
                if updated {
                    sel.is_loading = false;
                }
                app.selector = Some(sel);
            }
        }
        Event::DeploymentsFetched {
            environment,
            deployments,
        } => {
            app.status_message = None;
            if deployments.is_empty() {
                app.show_notice(format!("No deployments for {environment}"));
            }
            app.show_deployments(environment, deployments);
            app.update_filter_selection();
        }
        Event::DeploymentLogFetched {
            deployment_id,
            result,
        } if app.take_pending_deployment_log(deployment_id) => match result {
            Ok(log) => fx.suspend(ForegroundJob::ViewDeploymentLog { deployment_id, log }),
            Err(error) => {
                app.show_error(format!("Failed to fetch deployment log: {error}"));
                app.error_has_cli_detail = true;
            }
        },
        Event::DeploymentLogFetched { .. } => {}
        Event::RelatedMrsFetched { issue_iid, result } => {
            app.fetching_related_mrs.remove(&issue_iid);
            let new_state = match result {
                Ok(items) => {
                    if items.is_empty() {
                        crate::domain::issues::RelatedMrsState::Empty
                    } else {
                        crate::domain::issues::RelatedMrsState::Items(items)
                    }
                }
                Err(e) => crate::domain::issues::RelatedMrsState::Failed(e),
            };
            if let Some(issue) = app.issues.items.iter_mut().find(|i| i.iid == issue_iid) {
                issue.related_mrs = Some(new_state);
                crate::fetch::sync_linked_references(&mut app.issues.items, &mut app.mrs.items);
                app.project_cache.issues = app.issues.items.clone();
                crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
            }
        }
        Event::MrRelatedIssuesFetched { mr_iid, result } => {
            app.fetching_mr_related_issues.remove(&mr_iid);
            let new_state = match result {
                Ok(items) => {
                    if items.is_empty() {
                        crate::domain::mr::RelatedIssuesState::Empty
                    } else {
                        crate::domain::mr::RelatedIssuesState::Items(items)
                    }
                }
                Err(e) => crate::domain::mr::RelatedIssuesState::Failed(e),
            };
            if let Some(mr) = app.mrs.items.iter_mut().find(|m| m.iid == mr_iid) {
                mr.related_issues = Some(new_state);
                crate::fetch::sync_linked_references(&mut app.issues.items, &mut app.mrs.items);
                app.project_cache.mrs = app.mrs.items.clone();
                crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
            }
        }
        Event::FetchFailed(tab, err_msg) => {
            app.complete_loading_tab(tab, &format!("Failed: {err_msg}"));
            if app.has_rows(tab) {
                app.stale_tabs.insert(tab, app::StaleData::RefreshFailed);
            } else {
                app.show_error(err_msg);
                app.error_has_cli_detail = true;
            }
        }
        Event::DiffFetched {
            diff_view,
            comments,
        } => {
            app.diff_loading = false;
            let mut diff_view = *diff_view;
            let mr_iid = diff_view.mr_iid;
            let held_back = app.restore_draft_comments(&diff_view);
            // Restore the files marked as reviewed on an earlier pass.
            diff_view
                .restore_review_state(app.reviewed_files_for_mr(mr_iid), app.hide_reviewed_files);
            app.diff_view = Some(diff_view);
            if held_back > 0 {
                app.raise_error_toast(if held_back == 1 {
                    "1 draft comment no longer matches the diff and was set aside".to_string()
                } else {
                    format!(
                        "{held_back} draft comments no longer match the diff and were set aside"
                    )
                });
                app.error_has_cli_detail = false;
            }
            match comments {
                Ok(comments) => app.current_comments = comments,
                Err(err) => {
                    app.current_comments.clear();
                    app.raise_error_toast(format!("Failed to load review comments: {err}"));
                    app.error_has_cli_detail = true;
                }
            }
            if let (Some(overview), Some(diff_view)) =
                (app.review_threads.as_mut(), app.diff_view.as_ref())
            {
                overview.refresh(&app.current_comments, diff_view);
            }
            app.last_fetched_mr_iid = Some(mr_iid);
            app.in_review_mode = true;
            if let Some(row) = app
                .terminal_commands
                .find_latest(|cmd| cmd.command.contains("diff") && cmd.status == "Running")
            {
                app.terminal_commands.set_status(row, "Success".to_string());
            }
        }
        Event::CustomCommandFinished(report) => {
            handlers::custom_commands::report_runs(app, report);
        }
        Event::DiffFetchFailed(err_msg) => {
            app.diff_loading = false;
            app.show_error(err_msg);
            app.error_has_cli_detail = true;
        }
        Event::EditorClosed { purpose, text } => {
            crate::handlers::editor_result::apply(app, purpose, text, tx, fx);
        }
        Event::ViewerFailed(message) => app.show_error(message),
        Event::TerminalCommandLogged(entry) => {
            app.log_cli_command(*entry);
        }
        Event::CommandRerun {
            row,
            status,
            execution,
        } => {
            app.finish_rerun(row, status, *execution);
        }
        Event::CommandStarted(msg) => {
            app.show_notice(msg.clone());
            app.start_command(msg);
        }
        Event::CommandCompleted(tab, Ok(outcome)) => {
            if let Some(row) = app.latest_running_cli_command() {
                app.terminal_commands.set_status(row, "Success".to_string());
            }
            if outcome == CommandOutcome::Mutated {
                refetch_after_mutation(app, tab, tx, fx);
            }
        }
        Event::CommandCompleted(_, Err(err)) => {
            app.show_error(err);
            // CLI failure: full stderr is in the terminal log.
            app.error_has_cli_detail = true;
        }
        Event::Notice(msg) => {
            app.show_notice(msg);
        }
        Event::RunnerStateRevert {
            runner_id,
            status,
            active,
        } => {
            app.runners.update(&runner_id, |runner| {
                runner.status = status;
                runner.active = active;
            });
        }
        Event::PipelineStatusRevert {
            pipeline_id,
            status,
        } => {
            if let Some(pipeline) = app
                .pipelines
                .items
                .iter_mut()
                .find(|p| p.id() == pipeline_id)
            {
                pipeline.status = status;
            }
        }
        Event::JobStatusRevert { job_id, status } => {
            if let Some(job) = app.jobs.items.iter_mut().find(|j| j.id() == job_id) {
                job.status = status;
            }
        }
        Event::Mouse(mouse_event) => {
            handlers::mouse::handle_mouse_event(app, &mouse_event, tx, fx);
        }
        _ => {}
    }
}

/// Refetches `tab`, and the open diff, which a successful mutation may have
/// changed.
fn refetch_after_mutation(
    app: &mut App,
    tab: app::Tab,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    if let Some(client) = app.gitlab_client.clone() {
        app.start_loading_tab(tab);
        spawn_refresh_active_tab(
            fx,
            &client.muted(),
            &app.scope,
            app.requested_state_queries(),
            tab,
            app.scoped_sender(tx.clone()),
        );
    }
    let (Some(diff_view), Some(client)) = (&app.diff_view, app.gitlab_client.clone()) else {
        return;
    };
    let project_context = diff_view.project_path.clone();
    let mr_iid = diff_view.mr_iid;
    let tx = app.scoped_sender(tx.clone());
    fx.spawn(async move {
        if let Ok((diff_view, comments)) =
            crate::fetch::fetch_diff_view(&client, &project_context, mr_iid).await
        {
            let _ = tx.send(Event::DiffFetched {
                diff_view,
                comments,
            });
        }
    });
}
