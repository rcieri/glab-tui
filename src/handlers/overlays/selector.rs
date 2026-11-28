use crate::app::App;
use crate::effect::{EditPurpose, Effects, ForegroundJob};
use crate::entity_editor::{apply_selector_changes, rebuild_edit_menu};
use crate::event::Event;
use crate::git_helpers::slugify;
use crate::keybinding::KeyPress;
use crate::keymap::{Action, KeyContext};
use crate::templates::{BLANK_TEMPLATE_CHOICE, list_templates};
use crate::{app, domain, handlers};
use crossterm::event::KeyCode;
use tokio::sync::mpsc::UnboundedSender;

const SELECTOR_HALF_PAGE_ROWS: usize = 5;

/// "go to issue/MR by ID": when the target item isn't already loaded, kick off
/// a direct API lookup. Group scopes can't resolve a bare project item, so
/// surface a hint instead of silently failing.
fn jump_to_unloaded_issue(app: &mut App, iid: u64, tx: &UnboundedSender<Event>, fx: &mut Effects) {
    if app.scope.is_group() {
        app.show_error(format!(
            "Issue #{iid} isn't loaded and direct lookup needs a repository scope. Navigate to its project first."
        ));
        return;
    }
    let Some(client) = app.gitlab_client.clone() else {
        app.show_error("No active backend to look up the issue.".to_string());
        return;
    };
    let project = app.scope.as_str().to_string();
    crate::fetch::spawn_fetch_issue(fx, &client, &project, iid, app.scoped_sender(tx.clone()));
}

/// "go to issue/MR by ID": when the target MR/PR isn't already loaded, kick off
/// a direct API lookup. Group scopes can't resolve a bare project item, so
/// surface a hint instead of silently failing.
fn jump_to_unloaded_mr(app: &mut App, iid: u64, tx: &UnboundedSender<Event>, fx: &mut Effects) {
    if app.scope.is_group() {
        app.show_error(format!(
            "MR #{iid} isn't loaded and direct lookup needs a repository scope. Navigate to its project first."
        ));
        return;
    }
    let Some(client) = app.gitlab_client.clone() else {
        app.show_error("No active backend to look up the MR.".to_string());
        return;
    };
    let project = app.scope.as_str().to_string();
    crate::fetch::spawn_fetch_mr(fx, &client, &project, iid, app.scoped_sender(tx.clone()));
}

/// Keys while a selector overlay is open.
pub fn handle_key(
    app: &mut App,
    key_event: &KeyPress,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    let Some(mut selector) = app.pop_selector() else {
        return;
    };
    if selector.is_filtering {
        match key_event.code {
            KeyCode::Enter | KeyCode::Esc => {
                selector.is_filtering = false;
                app.push_modal(crate::app::Modal::Selector(selector));
            }
            KeyCode::Backspace => {
                selector.search_query.pop();
                selector.cursor_idx = 0;
                selector.state.select(Some(0));
                app.push_modal(crate::app::Modal::Selector(selector));
            }
            KeyCode::Char(c) => {
                selector.search_query.push(c);
                selector.cursor_idx = 0;
                selector.state.select(Some(0));
                app.push_modal(crate::app::Modal::Selector(selector));
            }
            _ => {
                app.push_modal(crate::app::Modal::Selector(selector));
            }
        }
    } else {
        let filtered_items = selector.get_filtered_items();
        match app
            .config
            .keybindings
            .resolve(KeyContext::Selector, key_event)
        {
            Some(Action::SelectorClose) => {
                // First Esc with an active query clears it (the
                // user already exited filter mode via the inner
                // match arm). Second Esc closes the selector.
                if !selector.search_query.is_empty() {
                    selector.search_query.clear();
                    selector.cursor_idx = 0;
                    selector.state.select(Some(0));
                    app.push_modal(crate::app::Modal::Selector(selector));
                }
                // else: leave the selector taken out so the
                // caller closes it (EditMenu stays open).
            }
            Some(Action::SelectorFilter) => {
                if selector.purpose.has_filter_bar() {
                    selector.is_filtering = true;
                }
                app.push_modal(crate::app::Modal::Selector(selector));
            }
            Some(Action::SelectorNext) => {
                if !filtered_items.is_empty() {
                    selector.cursor_idx = (selector.cursor_idx + 1) % filtered_items.len();
                    selector.state.select(Some(selector.cursor_idx));
                }
                app.push_modal(crate::app::Modal::Selector(selector));
            }
            Some(Action::SelectorPrevious) => {
                if !filtered_items.is_empty() {
                    if selector.cursor_idx == 0 {
                        selector.cursor_idx = filtered_items.len() - 1;
                    } else {
                        selector.cursor_idx -= 1;
                    }
                    selector.state.select(Some(selector.cursor_idx));
                }
                app.push_modal(crate::app::Modal::Selector(selector));
            }
            // The selector does not store its list height, so a
            // half page is a fixed number of rows.
            _ if app
                .config
                .keybindings
                .matches(Action::GlobalScrollHalfPageDown, key_event) =>
            {
                selector.move_by(true, SELECTOR_HALF_PAGE_ROWS);
                app.push_modal(crate::app::Modal::Selector(selector));
            }
            _ if app
                .config
                .keybindings
                .matches(Action::GlobalScrollHalfPageUp, key_event) =>
            {
                selector.move_by(false, SELECTOR_HALF_PAGE_ROWS);
                app.push_modal(crate::app::Modal::Selector(selector));
            }
            Some(Action::SelectorToggle) => {
                if !filtered_items.is_empty() {
                    let item = &filtered_items[selector.cursor_idx];
                    if item.starts_with("+ Create \"") {
                        let clean_val = selector.search_query.trim().to_string();
                        if !clean_val.is_empty() {
                            if selector.multi_select {
                                if selector.selected_items.contains(&clean_val) {
                                    selector.selected_items.remove(&clean_val);
                                } else {
                                    selector.selected_items.insert(clean_val);
                                }
                            } else {
                                selector.selected_items.clear();
                                selector.selected_items.insert(clean_val);
                            }
                        }
                    } else if selector.multi_select {
                        if selector.selected_items.contains(item) {
                            selector.selected_items.remove(item);
                        } else {
                            selector.selected_items.insert(item.clone());
                        }
                    } else if selector.selected_items.contains(item) {
                        selector.selected_items.remove(item);
                    } else {
                        selector.selected_items.clear();
                        selector.selected_items.insert(item.clone());
                    }
                }
                app.push_modal(crate::app::Modal::Selector(selector));
            }
            Some(Action::SelectorConfirm) => {
                match selector.purpose.clone() {
                    app::SelectorPurpose::JumpToId => {
                        let filtered_items = selector.get_filtered_items();
                        let selected_val = if !selector.selected_items.is_empty() {
                            selector.selected_items.iter().next().cloned()
                        } else if !filtered_items.is_empty() {
                            Some(filtered_items[selector.cursor_idx].clone())
                        } else {
                            None
                        };
                        let mut handle_jump =
                            |app: &mut App, kind: Option<crate::app::JumpKind>, id: u64| match kind
                            {
                                Some(crate::app::JumpKind::Issue) => {
                                    if !app.focus_issue(None, id) {
                                        jump_to_unloaded_issue(app, id, tx, fx);
                                    }
                                }
                                Some(crate::app::JumpKind::Mr) => {
                                    if !app.focus_mr(None, id) {
                                        jump_to_unloaded_mr(app, id, tx, fx);
                                    }
                                }
                                None => {
                                    if !app.focus_issue(None, id) && !app.focus_mr(None, id) {
                                        match app.active_tab {
                                            crate::app::Tab::Issues => {
                                                jump_to_unloaded_issue(app, id, tx, fx)
                                            }
                                            crate::app::Tab::MergeRequests => {
                                                jump_to_unloaded_mr(app, id, tx, fx)
                                            }
                                            _ => app.show_error(format!(
                                                "Item #{id} not found in current view"
                                            )),
                                        }
                                    }
                                }
                            };
                        let parse_row_kind = |val: &str| {
                            crate::app::parse_jump_input(
                                val.split(':').next().unwrap_or(val).trim(),
                            )
                        };
                        let query_val = selector.search_query.trim().to_string();
                        let mut handled = false;
                        if let Some((kind, id)) = crate::app::parse_jump_input(&query_val) {
                            handle_jump(app, kind, id);
                            handled = true;
                        }
                        if !handled
                            && let Some(val) = selected_val
                            && let Some((kind, id)) =
                                parse_row_kind(&val).or_else(|| crate::app::parse_jump_input(&val))
                        {
                            handle_jump(app, kind, id);
                            handled = true;
                        }
                        if !handled {
                            app.show_error(
                                "Not found. Use #123 (issue) / !123 (MR), or pick a listed item."
                                    .to_string(),
                            );
                        }
                    }
                    app::SelectorPurpose::ColumnFilter { tab, column } => {
                        app.apply_column_filter_picker(
                            tab,
                            column,
                            selector.selected_items.clone(),
                        );
                    }
                    app::SelectorPurpose::SwitchRepo => {
                        let filtered_items = selector.get_filtered_items();
                        let mut selected_val = selector.selected_items.iter().next().cloned();
                        if selected_val.is_none() && !filtered_items.is_empty() {
                            selected_val = Some(filtered_items[selector.cursor_idx].clone());
                        }

                        if let Some(mut path) = selected_val {
                            if path.starts_with("+ Create \"") {
                                path = selector.search_query.trim().to_string();
                            }

                            // Resolve the short basename back to its
                            // absolute on-disk path. Synthetic items
                            // ("+ Create …", group entries) are not in
                            // the map, so they fall through to the
                            // repos_dir fallback below.
                            let resolved = app
                                .switch_repo_paths
                                .get(&path)
                                .cloned()
                                .unwrap_or_else(|| path.clone());
                            let repos_dir = crate::utils::cache::get_repos_dir();
                            let target_path = if std::path::Path::new(&resolved).is_absolute() {
                                std::path::PathBuf::from(&resolved)
                            } else {
                                repos_dir.join(&resolved)
                            };
                            let target_path_str = target_path.to_string_lossy().into_owned();

                            // Fallback heuristic for top-level group/org
                            // names typed without a leading slash: a bare
                            // token that does not resolve to a local git
                            // repo is treated as a group scope. Subgroups
                            // (e.g. "group/subgroup") fall through and
                            // surface a set_current_dir error toast.
                            let is_group = app.switch_repo_groups.contains(&path)
                                || (!path.contains('/')
                                    && !crate::utils::cache::is_git_repo(&target_path_str));

                            if is_group {
                                let group_name = path.trim().to_string();
                                if group_name.is_empty() {
                                    return;
                                }
                                app.scope = crate::scope::Scope::Group(group_name.clone());
                                app.reset_on_scope_change();
                                crate::utils::cache::add_recent_group(&group_name);
                                let cache = crate::utils::cache::load_cache(app.scope.as_str());
                                app.project_cache = cache.clone();
                                app.restore_cached_issues(
                                    cache.data.issues,
                                    cache.data.issues_state_query,
                                );
                                app.restore_cached_mrs(cache.data.mrs, cache.data.mrs_state_query);
                                crate::fetch::derive_workflow(&mut app.mrs.items);
                                crate::fetch::sync_linked_references(
                                    &mut app.issues.items,
                                    &mut app.mrs.items,
                                );
                                app.pipelines.items = cache.data.pipelines;
                                app.runners.set_items(cache.data.runners);
                                app.releases.set_items(cache.data.releases);
                                app.todos.items = cache.data.todos;
                                app.milestones.items = cache.data.milestones;
                                app.pipeline_jobs = cache.data.pipeline_jobs;
                                app.branches.set_items(cache.data.branches);
                                app.environments.set_items(cache.data.environments);
                                app.milestone_issues_cache = cache.data.milestone_issues;
                                app.rebuild_milestone_progress_cache();
                                app.start_loading_tab(app.active_tab);
                                handlers::scope::spawn_client_for_scope(app, tx, fx);
                                return;
                            }

                            if crate::utils::cache::is_git_repo(&target_path_str) {
                                if std::env::set_current_dir(&target_path).is_ok() {
                                    crate::utils::cache::add_recent_repo(&target_path_str);
                                    app.config = crate::config::Config::load();
                                    app.apply_config();
                                    app.land_on_available_tab();
                                    let problems = app.load_keybindings();
                                    app.report_config_problems(problems);
                                    crate::config::reload_command_timeout();

                                    if let Ok(context) = domain::client::get_project_context() {
                                        app.scope = crate::scope::Scope::Repository(context);
                                    }
                                    app.reset_on_scope_change();

                                    app.loaded_tabs.clear();
                                    app.loading_tabs.clear();
                                    app.refreshed_tabs.clear();
                                    app.issues.items.clear();
                                    app.milestone_progress_cache.clear();
                                    app.mrs.items.clear();
                                    app.pipelines.items.clear();
                                    app.runners.set_items(Vec::new());
                                    app.releases.set_items(Vec::new());
                                    app.todos.items.clear();
                                    app.milestones.items.clear();
                                    app.pipeline_jobs.clear();
                                    app.fetching_pipelines.clear();
                                    app.close_edit_menu();
                                    app.selected_milestone_issues = None;
                                    app.selected_milestone_iid = None;

                                    let cache = crate::utils::cache::load_cache(app.scope.as_str());
                                    app.project_cache = cache.clone();
                                    app.restore_cached_issues(
                                        cache.data.issues,
                                        cache.data.issues_state_query,
                                    );
                                    app.restore_cached_mrs(
                                        cache.data.mrs,
                                        cache.data.mrs_state_query,
                                    );
                                    // Cached rows arrive with `workflow` unset: it is #[serde(skip)].
                                    crate::fetch::derive_workflow(&mut app.mrs.items);
                                    app.pipelines.items = cache.data.pipelines;
                                    app.runners.set_items(cache.data.runners);
                                    app.releases.set_items(cache.data.releases);
                                    app.todos.items = cache.data.todos;
                                    app.milestones.items = cache.data.milestones;
                                    app.pipeline_jobs = cache.data.pipeline_jobs;
                                    app.branches.set_items(cache.data.branches);
                                    app.environments.set_items(cache.data.environments);
                                    app.milestone_issues_cache = cache.data.milestone_issues;
                                    app.rebuild_milestone_progress_cache();
                                    app.cached_labels = cache.data.labels;
                                    app.cached_members = cache.data.members;

                                    app.mark_cached_tabs_stale();

                                    if !app.issues.items.is_empty() {
                                        app.loaded_tabs.insert(app::Tab::Issues);
                                    }
                                    if !app.mrs.items.is_empty() {
                                        app.loaded_tabs.insert(app::Tab::MergeRequests);
                                    }
                                    if !app.pipelines.items.is_empty() {
                                        app.loaded_tabs.insert(app::Tab::Pipelines);
                                    }
                                    if !app.runners.items().is_empty() {
                                        app.loaded_tabs.insert(app::Tab::Runners);
                                    }
                                    if !app.releases.items().is_empty() {
                                        app.loaded_tabs.insert(app::Tab::Releases);
                                    }
                                    if !app.todos.items.is_empty() {
                                        app.loaded_tabs.insert(app::Tab::Todos);
                                    }
                                    if !app.milestones.items.is_empty() {
                                        app.loaded_tabs.insert(app::Tab::Milestones);
                                    }

                                    app.issues.state.select(if app.issues.items.is_empty() {
                                        None
                                    } else {
                                        Some(0)
                                    });
                                    app.mrs.state.select(if app.mrs.items.is_empty() {
                                        None
                                    } else {
                                        Some(0)
                                    });
                                    app.pipelines
                                        .state
                                        .select(if app.pipelines.items.is_empty() {
                                            None
                                        } else {
                                            Some(0)
                                        });
                                    app.update_filter_selection();

                                    let has_cached = match app.active_tab {
                                        app::Tab::Issues => !app.issues.items.is_empty(),
                                        app::Tab::MergeRequests => !app.mrs.items.is_empty(),
                                        app::Tab::Pipelines => !app.pipelines.items.is_empty(),
                                        app::Tab::Runners => !app.runners.items().is_empty(),
                                        app::Tab::Releases => !app.releases.items().is_empty(),
                                        app::Tab::Todos => !app.todos.items.is_empty(),
                                        app::Tab::Milestones => !app.milestones.items.is_empty(),
                                        _ => false,
                                    };
                                    if !has_cached {
                                        app.loading_tabs.insert(app.active_tab);
                                    }
                                    handlers::scope::spawn_client_for_scope(app, tx, fx);
                                } else {
                                    app.show_error(format!(
                                        "Could not change directory to: {path}"
                                    ));
                                }
                            } else {
                                app.show_error(format!("Not a valid git repository: {path}"));
                            }
                        }
                    }

                    app::SelectorPurpose::Theme => {
                        let picked = selector.highlighted_theme();
                        app.close_selector();
                        if let Some(name) = picked {
                            app.config.theme_preset = Some(name);
                            app.apply_theme();
                        }
                    }

                    app::SelectorPurpose::RelatedMrs => {
                        // Pick the iid from the formatted item
                        // ("!{iid} [{STATE}] {title}"). Falls
                        // back to the cursor's index when the
                        // fuzzy filter has reordered entries.
                        let filtered_items = selector.get_filtered_items();
                        let picked = selector
                            .selected_items
                            .iter()
                            .next()
                            .cloned()
                            .or_else(|| filtered_items.get(selector.cursor_idx).cloned());
                        app.close_selector();
                        if let Some(item) = picked {
                            if let Some(iid_str) = item
                                .strip_prefix('!')
                                .and_then(|s| s.split_whitespace().next())
                                && let Ok(mr_iid) = iid_str.parse::<u64>()
                            {
                                if let Some(client) = app.gitlab_client.clone() {
                                    crate::handlers::tabs::jump_to_mr_tab_from_selector(
                                        app,
                                        mr_iid,
                                        tx.clone(),
                                        &client,
                                        fx,
                                    );
                                }
                                return;
                            }
                            app.show_error("Could not parse the selected MR/PR iid".to_string());
                        }
                    }

                    app::SelectorPurpose::StackEntries => {
                        // Each line is formatted
                        // "<pos>. #<iid>: <title> (STATE) ... [draft] <nav_prev> (current)".
                        // Titles may contain '#', so take the first whitespace
                        // token shaped "#<iid>".
                        let filtered_items = selector.get_filtered_items();
                        let picked = selector
                            .selected_items
                            .iter()
                            .next()
                            .cloned()
                            .or_else(|| filtered_items.get(selector.cursor_idx).cloned());
                        app.close_selector();
                        if let Some(item) = picked {
                            let iid_str = item.split_whitespace().find_map(|tok| {
                                tok.strip_prefix('#')
                                    .and_then(|s| s.trim_end_matches(':').parse::<u64>().ok())
                            });
                            if let Some(target) = iid_str {
                                if let Some(idx) =
                                    app.mrs.items.iter().position(|m| m.iid == target)
                                {
                                    app.mrs.state.select(Some(idx));
                                    app.detail_scroll = 0;
                                    app.details_zoomed = false;
                                    return;
                                }
                                if let Some(client) = app.gitlab_client.clone() {
                                    crate::handlers::tabs::jump_to_mr_tab_from_selector(
                                        app,
                                        target,
                                        tx.clone(),
                                        &client,
                                        fx,
                                    );
                                    return;
                                }
                                app.show_error(
                                    "No backend client available to jump to the stack entry."
                                        .to_string(),
                                );
                            } else {
                                app.show_error(
                                    "Could not parse the selected stack entry iid".to_string(),
                                );
                            }
                        }
                    }

                    app::SelectorPurpose::LinkedIssues => {
                        let filtered_items = selector.get_filtered_items();
                        let picked = selector
                            .selected_items
                            .iter()
                            .next()
                            .cloned()
                            .or_else(|| filtered_items.get(selector.cursor_idx).cloned());
                        app.close_selector();
                        if let Some(item) = picked {
                            if let Some(iid_str) = item
                                .strip_prefix('#')
                                .and_then(|s| s.split_whitespace().next())
                                && let Ok(issue_iid) = iid_str.parse::<u64>()
                            {
                                if let Some(client) = app.gitlab_client.clone() {
                                    crate::handlers::tabs::jump_to_issue_tab_from_selector(
                                        app,
                                        issue_iid,
                                        tx.clone(),
                                        &client,
                                        fx,
                                    );
                                }
                                return;
                            }
                            app.show_error("Could not parse the selected Issue iid".to_string());
                        }
                    }

                    app::SelectorPurpose::PipelineSelect => {
                        let filtered_items = selector.get_filtered_items();
                        let mut selected_val = selector.selected_items.iter().next().cloned();
                        if selected_val.is_none() && !filtered_items.is_empty() {
                            selected_val = Some(filtered_items[selector.cursor_idx].clone());
                        }
                        if let Some(val) = selected_val
                            && let Some(id_str) =
                                val.strip_prefix('#').and_then(|s| s.split(" — ").next())
                            && let Ok(pipeline_id) = id_str.parse::<u64>()
                        {
                            if app.gitlab_client.is_some() {
                                app.open_pipeline_in_jobs_tab(pipeline_id);
                            }
                            if let Some(client) = &app.gitlab_client {
                                let project_context = app.project_path_for_jobs();
                                app.jobs.state.select(Some(0));
                                app.jobs.items.clear();
                                app.loading_tabs.insert(app::Tab::Jobs);
                                let client_clone = client.clone();
                                let tx = app.scoped_sender(tx.clone());
                                fx.spawn(async move {
                                        match domain::pipelines::list_pipeline_jobs(
                                            &client_clone,
                                            &project_context,
                                            pipeline_id,
                                        )
                                        .await
                                        {
                                            Ok(jobs) => {
                                                let _ =
                                                    tx.send(Event::JobsTabFetched(
                                                        domain::client::JobsSource::Opened(
                                                            pipeline_id,
                                                        ),
                                                        pipeline_id,
                                                        jobs,
                                                    ));
                                            }
                                            Err(e) => {
                                                let _ = tx.send(
                                                    Event::FetchFailed(
                                                        app::Tab::Jobs,
                                                        format!(
                                                            "Failed to fetch jobs for pipeline {pipeline_id}: {e}"
                                                        ),
                                                    ),
                                                );
                                            }
                                        }
                                    });
                            }
                        }
                    }

                    app::SelectorPurpose::CommentChoice { mr_iid } => {
                        let filtered_items = selector.get_filtered_items();
                        let mut selected_val = selector.selected_items.iter().next().cloned();
                        if selected_val.is_none() && !filtered_items.is_empty() {
                            selected_val = Some(filtered_items[selector.cursor_idx].clone());
                        }

                        if let Some(val) = selected_val {
                            if let Some(idx_str) = val
                                .strip_prefix("DRAFT #")
                                .and_then(|s| s.split('|').next())
                            {
                                if let Ok(n) = idx_str.trim().parse::<usize>() {
                                    let draft_idx = n.saturating_sub(1);
                                    if draft_idx < app.draft_comments.len() {
                                        app.open_selector(crate::app::Selector::draft_actions(
                                            draft_idx,
                                        ));
                                        return;
                                    }
                                }
                            } else if let Some(id_str) =
                                val.strip_prefix("ID: ").and_then(|s| s.split(" |").next())
                                && let Ok(comment_id) = id_str.parse::<u64>()
                            {
                                if let Some(comment) = app
                                    .current_comments
                                    .iter()
                                    .find(|c| c.id == Some(comment_id))
                                    .cloned()
                                    && let Some(actions) = crate::app::Selector::comment_actions(
                                        &comment,
                                        mr_iid,
                                        app.is_github(),
                                    )
                                {
                                    app.open_selector(actions);
                                    return;
                                }
                                app.close_edit_menu();
                                return;
                            }
                        }
                        app.close_selector();
                    }

                    app::SelectorPurpose::DraftActions { index: draft_idx } => {
                        let filtered_items = selector.get_filtered_items();
                        let mut selected_val = selector.selected_items.iter().next().cloned();
                        if selected_val.is_none() && !filtered_items.is_empty() {
                            selected_val = Some(filtered_items[selector.cursor_idx].clone());
                        }

                        app.close_selector();

                        if let Some(action_str) = selected_val
                            && draft_idx < app.draft_comments.len()
                        {
                            match action_str.as_str() {
                                "Edit Draft" => {
                                    fx.suspend(ForegroundJob::Edit {
                                        text: app.draft_comments[draft_idx].body.clone(),
                                        file_suffix: ".md".to_string(),
                                        purpose: EditPurpose::DraftBody { index: draft_idx },
                                    });
                                }
                                "Delete Draft" => {
                                    app.draft_comments.remove(draft_idx);
                                    if let Some((project, mr_iid)) =
                                        app.diff_view().map(|d| (d.project_path.clone(), d.mr_iid))
                                    {
                                        app.persist_draft_comments(&project, mr_iid);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }

                    app::SelectorPurpose::CommentActions { comment_id, mr_iid } => {
                        let filtered_items = selector.get_filtered_items();
                        let mut selected_val = selector.selected_items.iter().next().cloned();
                        if selected_val.is_none() && !filtered_items.is_empty() {
                            selected_val = Some(filtered_items[selector.cursor_idx].clone());
                        }

                        app.close_selector();

                        if let Some(action_str) = selected_val {
                            let comment = app
                                .current_comments
                                .iter()
                                .find(|c| c.id == Some(comment_id))
                                .cloned();

                            if let Some(comment) = comment {
                                match action_str.as_str() {
                                    "Reply to Thread" => {
                                        let discussion_id = comment
                                            .discussion_id
                                            .clone()
                                            .unwrap_or_else(|| comment_id.to_string());
                                        app.open_text_input(crate::app::TextInput::new(
                                            format!(" Reply to @{} ", comment.author.username),
                                            String::new(),
                                            crate::app::TextInputAction::ReplyToComment {
                                                mr_iid,
                                                comment_id,
                                                discussion_id,
                                            },
                                        ));
                                    }
                                    "Resolve Thread" | "Unresolve Thread" => {
                                        let is_resolve = action_str == "Resolve Thread";
                                        if let Some(client) = app.gitlab_client.clone() {
                                            let project = app.project_path_for_open_mr(mr_iid);
                                            let thread_id =
                                                comment.discussion_id.clone().unwrap_or_default();
                                            let status_desc = if is_resolve {
                                                "Resolving"
                                            } else {
                                                "Unresolving"
                                            };
                                            let tx = tx.clone();
                                            let _ = tx.send(Event::CommandStarted(format!(
                                                "{status_desc} thread MR #{mr_iid}"
                                            )));
                                            handlers::diff_view::spawn_mr_review_task(
                                                fx,
                                                &tx,
                                                async move {
                                                    client
                                                        .set_thread_resolved(
                                                            &project, mr_iid, &thread_id,
                                                            is_resolve,
                                                        )
                                                        .await
                                                },
                                            );
                                        }
                                    }
                                    "Edit Comment" => {
                                        app.show_notice("Opening editor to edit comment...");
                                        let ext = std::path::Path::new(
                                            comment
                                                .position
                                                .as_ref()
                                                .and_then(|p| p.new_path.as_ref())
                                                .map(|s| s.as_str())
                                                .unwrap_or("md"),
                                        )
                                        .extension()
                                        .and_then(|s| s.to_str())
                                        .unwrap_or("md");
                                        fx.suspend(ForegroundJob::Edit {
                                            text: comment.body.clone(),
                                            file_suffix: format!(".{ext}"),
                                            purpose: EditPurpose::CommentBody {
                                                mr_iid,
                                                comment: Box::new(comment),
                                            },
                                        });
                                    }
                                    "Delete Comment" => {
                                        if let Some(client) = app.gitlab_client.clone() {
                                            let project = app.project_path_for_open_mr(mr_iid);
                                            let _ = tx.send(Event::CommandStarted(format!(
                                                "Deleting comment MR #{mr_iid}"
                                            )));
                                            handlers::diff_view::spawn_mr_review_task(
                                                fx,
                                                tx,
                                                async move {
                                                    client
                                                        .delete_comment(&project, mr_iid, &comment)
                                                        .await
                                                },
                                            );
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    app::SelectorPurpose::DescriptionTemplate { kind } => {
                        let filtered_items = selector.get_filtered_items();
                        let mut selected_val = selector.selected_items.iter().next().cloned();
                        if selected_val.is_none() && !filtered_items.is_empty() {
                            selected_val = Some(filtered_items[selector.cursor_idx].clone());
                        }
                        let choice = selected_val.unwrap_or_default();
                        let mut desc_val = String::new();
                        if choice != BLANK_TEMPLATE_CHOICE
                            && let Some(content) = list_templates(kind)
                                .iter()
                                .find(|(n, _)| n == &choice)
                                .map(|(_, c)| c)
                        {
                            desc_val = content.clone();
                        }
                        if let Some(ref mut menu) = app.edit_menu_mut() {
                            if let Some(f) = menu
                                .fields
                                .iter_mut()
                                .find(|f| f.label == "Description" || f.label == "Release Notes")
                            {
                                f.value = desc_val;
                            }
                            if let Some(f) = menu
                                .fields
                                .iter_mut()
                                .find(|f| f.label == "Description Template")
                            {
                                f.value = if choice == BLANK_TEMPLATE_CHOICE {
                                    String::new()
                                } else {
                                    choice
                                };
                            }
                        }
                        app.close_selector();
                    }

                    app::SelectorPurpose::CreateMrFromIssue => {
                        let filtered_items = selector.get_filtered_items();
                        let mut selected_val = selector.selected_items.iter().next().cloned();
                        if selected_val.is_none() && !filtered_items.is_empty() {
                            selected_val = Some(filtered_items[selector.cursor_idx].clone());
                        }
                        if let Some(val) = selected_val {
                            let current_val = app
                                .edit_menu()
                                .and_then(|m| {
                                    m.fields.iter().find(|f| f.label == "Create from Issue")
                                })
                                .map(|f| f.value.clone())
                                .unwrap_or_default();

                            if val == current_val {
                                // Deselect if it's already selected
                                if let Some(ref mut menu) = app.edit_menu_mut() {
                                    for label in [
                                        "Title",
                                        "Source Branch",
                                        "Assignees",
                                        "Milestone",
                                        "Labels",
                                        "Create from Issue",
                                    ] {
                                        if let Some(f) =
                                            menu.fields.iter_mut().find(|f| f.label == label)
                                        {
                                            f.value = String::new();
                                        }
                                    }
                                    menu.entity_iid = 0;
                                }
                            } else {
                                let parsed_iid = val
                                    .strip_prefix('#')
                                    .and_then(|s| s.split(|c: char| !c.is_numeric()).next())
                                    .and_then(|s| s.parse::<u64>().ok());
                                let issue_details = parsed_iid.and_then(|iid| {
                                    app.issues.items.iter().find(|i| i.iid == iid).map(|issue| {
                                        let title = issue.title.clone();
                                        let source_branch =
                                            format!("{}-{}", issue.iid, slugify(&issue.title));
                                        let labels = if issue.labels.is_empty() {
                                            String::new()
                                        } else {
                                            issue.labels.join(", ")
                                        };
                                        let assignees = if issue.assignees.is_empty() {
                                            String::new()
                                        } else {
                                            issue
                                                .assignees
                                                .iter()
                                                .map(|a| format!("@{}", a.username))
                                                .collect::<Vec<_>>()
                                                .join(", ")
                                        };
                                        let milestone = issue
                                            .milestone
                                            .as_ref()
                                            .map(|m| m.title.clone())
                                            .unwrap_or_default();
                                        (
                                            issue.iid,
                                            title,
                                            source_branch,
                                            labels,
                                            assignees,
                                            milestone,
                                        )
                                    })
                                });
                                if let Some((
                                    issue_iid,
                                    title,
                                    source_branch,
                                    labels,
                                    assignees,
                                    milestone,
                                )) = issue_details
                                    && let Some(ref mut menu) = app.edit_menu_mut()
                                {
                                    for (label, value) in [
                                        ("Title", title),
                                        ("Source Branch", source_branch),
                                        ("Assignees", assignees),
                                        ("Milestone", milestone),
                                        ("Labels", labels),
                                    ] {
                                        if let Some(f) =
                                            menu.fields.iter_mut().find(|f| f.label == label)
                                        {
                                            f.value = value;
                                        }
                                    }
                                    if let Some(f) = menu
                                        .fields
                                        .iter_mut()
                                        .find(|f| f.label == "Create from Issue")
                                    {
                                        f.value = val.clone();
                                    }
                                    menu.entity_iid = issue_iid;
                                }
                            }
                        }
                        app.close_selector();
                    }

                    app::SelectorPurpose::Field(field) => {
                        let entity_type = selector.entity_type.clone();
                        let entity_iid = selector.entity_iid;
                        let filtered_items = selector.get_filtered_items();
                        let mut selected_list: Vec<String> =
                            selector.selected_items.iter().cloned().collect();

                        // Include highlighted item in selection if nothing auto-selected
                        if !filtered_items.is_empty() {
                            let item = &filtered_items[selector.cursor_idx];
                            if item.starts_with("+ Create \"") {
                                let query = selector.search_query.trim().to_string();
                                if !query.is_empty() {
                                    if selector.multi_select {
                                        if !selected_list.contains(&query) {
                                            selected_list.push(query);
                                        }
                                    } else {
                                        selected_list = vec![query];
                                    }
                                }
                            } else if !selector.multi_select
                                && selected_list.is_empty()
                                && field != app::SelectorField::Milestone
                            {
                                selected_list.push(item.clone());
                            }
                        }

                        if app.edit_menu().is_some() {
                            let mut project_fetch_val = None;
                            // Write the values directly to the active field of app.edit_menu
                            if let Some(ref mut menu) = app.edit_menu_mut() {
                                let target_field_name = field.label();
                                if let Some(f) = menu
                                    .fields
                                    .iter_mut()
                                    .find(|f| f.label == target_field_name)
                                {
                                    let display_val = match field {
                                        app::SelectorField::Confidential
                                        | app::SelectorField::MrPipeline => selected_list
                                            .first()
                                            .cloned()
                                            .unwrap_or_else(|| "No".to_string()),
                                        app::SelectorField::DraftStatus => selected_list
                                            .first()
                                            .cloned()
                                            .unwrap_or_else(|| "Ready".to_string()),
                                        _ => selected_list.join(", "),
                                    };

                                    let is_workflow_file = field
                                        == app::SelectorField::WorkflowFile
                                        && !display_val.is_empty();

                                    f.value = display_val.clone();

                                    if field == app::SelectorField::Project {
                                        menu.entity_project = display_val.clone();
                                        if !display_val.is_empty()
                                            && crate::backend::glab::validate_project_path(
                                                &display_val,
                                            )
                                            .is_ok()
                                        {
                                            project_fetch_val = Some(display_val.clone());
                                        }
                                    }

                                    let _ = f; // release borrow before modifying fields

                                    // When a workflow file is selected, parse
                                    // its workflow_dispatch inputs and rebuild
                                    // the edit menu fields to show per-input fields.
                                    if is_workflow_file
                                        && let Some(root) = crate::git_helpers::repo_root()
                                    {
                                        let yaml_path =
                                            format!("{root}/.github/workflows/{display_val}");
                                        if let Some(inputs) =
                                            crate::domain::workflow_inputs::parse_workflow_inputs(
                                                &yaml_path,
                                            )
                                        {
                                            menu.workflow_inputs = inputs.clone();
                                            menu.fields.retain(|f| f.label != "Inputs");
                                            let insert_pos = menu
                                                .fields
                                                .iter()
                                                .position(|f| f.label == "Variables")
                                                .unwrap_or(menu.fields.len());
                                            for input in inputs.iter().rev() {
                                                let label = format!("Input: {}", input.name);
                                                let default_val =
                                                    input.default.clone().unwrap_or_default();
                                                menu.fields.insert(
                                                    insert_pos,
                                                    crate::app::Field::text(&label, default_val),
                                                );
                                            }
                                        }
                                    }
                                }
                            }

                            if let Some(display_val) = project_fetch_val
                                && let Some(client) = &app.gitlab_client
                            {
                                let client = client.clone();
                                let proj_scope =
                                    crate::scope::Scope::Repository(display_val.clone());
                                let tx = crate::event::ScopedSender::new(
                                    tx.clone(),
                                    app.scope_generation,
                                );
                                let (c1, c2, c3, c4) = (
                                    client.clone(),
                                    client.clone(),
                                    client.clone(),
                                    client.clone(),
                                );
                                let (p1, p2, p3, p4) = (
                                    proj_scope.clone(),
                                    proj_scope.clone(),
                                    proj_scope.clone(),
                                    proj_scope.clone(),
                                );
                                let (d1, d2, d3, d4) = (
                                    display_val.clone(),
                                    display_val.clone(),
                                    display_val.clone(),
                                    display_val.clone(),
                                );
                                let (tx1, tx2, tx3, tx4) =
                                    (tx.clone(), tx.clone(), tx.clone(), tx.clone());

                                fx.spawn(async move {
                                    if let Ok(labels) = c1.fetch_labels(&p1).await {
                                        let _ = tx1.send(Event::ProjectAttributesFetched {
                                            project: d1,
                                            labels,
                                            members: Vec::new(),
                                            milestones: Vec::new(),
                                            branches: Vec::new(),
                                        });
                                    }
                                });
                                fx.spawn(async move {
                                    if let Ok(members) = c2.fetch_members(&p2).await {
                                        let _ = tx2.send(Event::ProjectAttributesFetched {
                                            project: d2,
                                            labels: Vec::new(),
                                            members,
                                            milestones: Vec::new(),
                                            branches: Vec::new(),
                                        });
                                    }
                                });
                                fx.spawn(async move {
                                    if let Ok(milestones) = c3.fetch_milestones(&p3).await {
                                        let _ = tx3.send(Event::ProjectAttributesFetched {
                                            project: d3,
                                            labels: Vec::new(),
                                            members: Vec::new(),
                                            milestones,
                                            branches: Vec::new(),
                                        });
                                    }
                                });
                                fx.spawn(async move {
                                    if let Ok(branches) = c4.fetch_branches(&p4).await {
                                        let _ = tx4.send(Event::ProjectAttributesFetched {
                                            project: d4,
                                            labels: Vec::new(),
                                            members: Vec::new(),
                                            milestones: Vec::new(),
                                            branches,
                                        });
                                    }
                                });
                            }
                        } else {
                            apply_selector_changes(
                                app,
                                &entity_type,
                                entity_iid,
                                &field,
                                selected_list,
                                tx.clone(),
                                fx,
                            );

                            rebuild_edit_menu(app, &entity_type, entity_iid);
                        }
                    }
                }
            }
            _ => {
                app.push_modal(crate::app::Modal::Selector(selector));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{EditEntityKind, EditMenu, Field, Selector, SelectorPurpose};
    use ratatui::widgets::ListState;

    fn new_mr_menu() -> EditMenu {
        EditMenu {
            title: "New MR".to_string(),
            entity_project: "g/p".to_string(),
            fields: vec![
                Field::text("Description Template", String::new()),
                Field::text("Description", String::new()),
            ],
            initial_fields: std::collections::HashMap::new(),
            selected_idx: 0,
            entity_iid: 0,
            entity_kind: EditEntityKind::CreateMr,
            state: ListState::default(),
            workflow_inputs: vec![],
            cursor_pos: 0,
            editing: false,
            desc_scroll: 0,
            source_on_forge: false,
        }
    }

    /// The template picker of a new MR fills the Description from the
    /// repository's pull request templates, not its issue templates.
    #[test]
    fn picking_an_mr_template_fills_the_description_with_it() {
        let _config = crate::config::isolate_config();
        for (dir, body) in [
            (".github/PULL_REQUEST_TEMPLATE", "PR template"),
            (".github/ISSUE_TEMPLATE", "issue template"),
        ] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(std::path::Path::new(dir).join("feature.md"), body).unwrap();
        }
        let mut app = App::default();
        app.open_edit_menu(new_mr_menu());
        app.open_selector(Selector {
            title: "Template".to_string(),
            all_items: vec![BLANK_TEMPLATE_CHOICE.to_string(), "feature".to_string()],
            selected_items: std::collections::HashSet::new(),
            cursor_idx: 1,
            search_query: String::new(),
            is_filtering: false,
            is_loading: false,
            entity_iid: 0,
            entity_type: String::new(),
            purpose: SelectorPurpose::DescriptionTemplate {
                kind: EditEntityKind::CreateMr.template_kind(),
            },
            multi_select: false,
            state: ListState::default(),
            request: None,
        });
        let tx = app.tx.clone();
        let mut fx = Effects::default();
        let enter = KeyPress::from(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        handle_key(&mut app, &enter, &tx, &mut fx);

        let description = app
            .edit_menu()
            .and_then(|menu| menu.fields.iter().find(|f| f.label == "Description"))
            .map(|f| f.value.clone());
        assert_eq!(description, Some("PR template".to_string()));
    }

    /// Picking the blank template clears the Description Template row.
    #[test]
    fn a_blank_pick_from_the_description_clears_the_template_row() {
        let mut menu = new_mr_menu();
        menu.fields[0].value = "feature".to_string();
        let mut app = App::default();
        app.open_edit_menu(menu);
        app.open_selector(Selector {
            title: "Template".to_string(),
            all_items: vec![BLANK_TEMPLATE_CHOICE.to_string()],
            selected_items: std::collections::HashSet::new(),
            cursor_idx: 0,
            search_query: String::new(),
            is_filtering: false,
            is_loading: false,
            entity_iid: 0,
            entity_type: String::new(),
            purpose: SelectorPurpose::DescriptionTemplate {
                kind: EditEntityKind::CreateMr.template_kind(),
            },
            multi_select: false,
            state: ListState::default(),
            request: None,
        });
        let tx = app.tx.clone();
        let mut fx = Effects::default();
        let enter = KeyPress::from(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        handle_key(&mut app, &enter, &tx, &mut fx);

        let template_row = app
            .edit_menu()
            .and_then(|menu| {
                menu.fields
                    .iter()
                    .find(|f| f.label == "Description Template")
            })
            .map(|f| f.value.clone());
        assert_eq!(template_row, Some(String::new()));
    }
}
