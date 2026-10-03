use crate::AppTerminal;
use crate::app::App;
use crate::entity_editor::{apply_field_text_change, rebuild_edit_menu};
use crate::event::Event;
use crate::fetch::{spawn_fetch_repo_attributes, spawn_refresh_active_tab};
use crate::keybinding::keybinding_matches;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

pub fn handle_submit_dialog(
    app: &mut App,
    key_event: &KeyEvent,
    tx: UnboundedSender<Event>,
) -> bool {
    let Some(mut dialog) = app.submit_dialog.take() else {
        return false;
    };

    let mut submit = false;
    let mut cancel = false;

    match key_event.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if dialog.is_on_submit() || dialog.is_on_cancel() {
                if !dialog.options.is_empty() {
                    dialog.cursor_idx = dialog.options.len(); // jump up to last option
                }
            } else if dialog.cursor_idx > 1 {
                dialog.cursor_idx -= 1; // move up
            } else {
                dialog.cursor_idx = dialog.cancel_idx(); // wrap around to buttons
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if dialog.is_on_submit() || dialog.is_on_cancel() {
                if !dialog.options.is_empty() {
                    dialog.cursor_idx = 1; // wrap around to first option
                }
            } else if dialog.cursor_idx < dialog.options.len() {
                dialog.cursor_idx += 1; // move down
            } else {
                dialog.cursor_idx = dialog.cancel_idx(); // jump down to buttons
            }
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if dialog.is_on_submit() || dialog.is_on_cancel() {
                dialog.cursor_idx = crate::app::SubmitDialog::SUBMIT_IDX; // Submit is left
            }
        }
        KeyCode::Right | KeyCode::Char('l') => {
            if dialog.is_on_submit() || dialog.is_on_cancel() {
                dialog.cursor_idx = dialog.cancel_idx(); // Cancel is right
            }
        }
        KeyCode::Tab => {
            dialog.move_next();
        }
        KeyCode::BackTab => {
            dialog.move_prev();
        }
        KeyCode::Char(' ') => {
            dialog.toggle_focused_option();
        }
        KeyCode::Enter => {
            if dialog.is_on_submit() {
                submit = true;
            } else if dialog.is_on_cancel() {
                cancel = true;
            } else {
                dialog.toggle_focused_option();
            }
        }
        KeyCode::Char('y') | KeyCode::Char('Y') if dialog.is_on_submit() => {
            submit = true;
        }
        KeyCode::Esc => {
            cancel = true;
        }
        _ => {
            // Consume all other keys while the dialog is open so they
            // don't fall through to the underlying tab.
        }
    }

    if submit {
        // Drain the dialog so we can inspect option toggles before
        // dispatching the API call.
        let action = dialog.action.clone();
        let options = std::mem::take(&mut dialog.options);
        run_submit_action(app, action, options, tx);
    } else if cancel {
        if matches!(dialog.action, crate::app::ConfirmAction::SubmitReview(_)) {
            app.draft_comments.clear();
            app.in_review_mode = false;
            app.diff_view = None;
        }
    } else {
        // Either the user navigated or toggled an option — keep the
        // dialog open.
        app.submit_dialog = Some(dialog);
    }

    true
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

fn run_submit_action(
    app: &mut App,
    confirm_action: crate::app::ConfirmAction,
    options: Vec<crate::app::SubmitOption>,
    tx: UnboundedSender<Event>,
) {
    match confirm_action {
        crate::app::ConfirmAction::DeleteMilestone(iid) => {
            let project_path = app.project_path_for_milestone(iid);
            app.pending_delete_milestone_iid = Some(iid);
            let client = app.gitlab_client.clone().unwrap();
            tokio::spawn(async move {
                let res =
                    crate::domain::milestones::delete_milestone(&client, &project_path, iid).await;
                match res {
                    Ok(_) => {
                        let _ =
                            tx.send(Event::CommandCompleted(crate::app::Tab::Milestones, Ok(())));
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
            let project_path = app.project_path_for_milestone(iid);
            if let Some(m) = app.milestones.items.iter_mut().find(|m| m.iid == iid) {
                m.state = "closed".to_string();
            }
            app.project_cache.milestones = app.milestones.items.clone();
            let client = app.gitlab_client.clone().unwrap();
            let tx2 = tx.clone();
            tokio::spawn(async move {
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
            let project_path = app.project_path_for_milestone(iid);
            if let Some(m) = app.milestones.items.iter_mut().find(|m| m.iid == iid) {
                m.state = "active".to_string();
            }
            app.project_cache.milestones = app.milestones.items.clone();
            let client = app.gitlab_client.clone().unwrap();
            let tx2 = tx.clone();
            tokio::spawn(async move {
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
            let project_path = app.project_path_for_release(&tag_name);
            app.pending_delete_release_tag = Some(tag_name.clone());
            let client = app.gitlab_client.clone().unwrap();
            tokio::spawn(async move {
                let res =
                    crate::domain::releases::delete_release(&client, &project_path, &tag_name)
                        .await;
                match res {
                    Ok(_) => {
                        let _ = tx.send(Event::CommandCompleted(crate::app::Tab::Releases, Ok(())));
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
            let client = app.gitlab_client.clone().unwrap();
            let project_path = app.scope.as_str().to_string();
            tokio::spawn(async move {
                let res =
                    crate::domain::branches::delete_branch(&client, &project_path, &branch_name)
                        .await;
                match res {
                    Ok(_) => {
                        let _ = tx.send(Event::CommandCompleted(crate::app::Tab::Branches, Ok(())));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::Branches,
                            Err(format!("Failed to delete branch: {}", e)),
                        ));
                    }
                }
            });
        }
        crate::app::ConfirmAction::CloseIssue(iid) => {
            let project_path = app.project_path_for_issue(iid);
            if let Some(pos) = app.issues.items.iter().position(|i| i.iid == iid) {
                app.issues.items.remove(pos);
            }
            app.update_filter_selection();
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = client.close_issue(&project_path, iid).await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::Issues,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        crate::app::ConfirmAction::DeleteIssue(iid) => {
            let project_path = app.project_path_for_issue(iid);
            let client = app.gitlab_client.clone().unwrap();
            tokio::spawn(async move {
                let res = client.delete_issue(&project_path, iid).await;
                match res {
                    Ok(_) => {
                        let _ = tx.send(Event::CommandCompleted(crate::app::Tab::Issues, Ok(())));
                        let _ = tx.send(Event::IssueDeleted);
                    }
                    Err(e) => {
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::Issues,
                            Err(format!("Failed to delete issue: {}", e)),
                        ));
                    }
                }
            });
        }
        crate::app::ConfirmAction::ReopenIssue(iid) => {
            let project_path = app.project_path_for_issue(iid);
            if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                item.state = "opened".to_string();
            }
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = client.reopen_issue(&project_path, iid).await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::Issues,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        crate::app::ConfirmAction::CloseMr(iid) => {
            let project_path = app.project_path_for_mr(iid);
            if let Some(pos) = app.mrs.items.iter().position(|m| m.iid == iid) {
                app.mrs.items.remove(pos);
            }
            app.update_filter_selection();
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = client.close_mr(&project_path, iid).await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        crate::app::ConfirmAction::DeleteMr(iid) => {
            let project_path = app.project_path_for_mr(iid);
            let client = app.gitlab_client.clone().unwrap();
            tokio::spawn(async move {
                let res = client.delete_mr(&project_path, iid).await;
                match res {
                    Ok(_) => {
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::MergeRequests,
                            Ok(()),
                        ));
                        let _ = tx.send(Event::MrDeleted);
                    }
                    Err(e) => {
                        let _ = tx.send(Event::CommandCompleted(
                            crate::app::Tab::MergeRequests,
                            Err(format!("Failed to delete merge request: {}", e)),
                        ));
                    }
                }
            });
        }
        crate::app::ConfirmAction::ReopenMr(iid) => {
            let project_path = app.project_path_for_mr(iid);
            if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                item.state = "opened".to_string();
            }
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = client.reopen_mr(&project_path, iid).await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        crate::app::ConfirmAction::MergeMr(iid) => {
            let (squash, delete_branch, merge_strategy, auto_merge) = merge_options_from(&options);
            let project_path = app.project_path_for_mr(iid);
            // Capture the source-branch head SHA before removing the row so
            // `glab mr merge --sha=<sha>` can satisfy GitLab 19.2+ merge
            // requirements (#470). Pre-19.2 instances ignore it; GitHub's
            // `GhBackend::merge_mr` also ignores it.
            let mr_sha = app
                .mrs
                .items
                .iter()
                .find(|m| m.iid == iid)
                .and_then(|m| m.sha.clone());
            if let Some(pos) = app.mrs.items.iter().position(|m| m.iid == iid) {
                app.mrs.items.remove(pos);
            }
            app.update_filter_selection();
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = client
                    .merge_mr(
                        &project_path,
                        iid,
                        squash,
                        delete_branch,
                        merge_strategy,
                        auto_merge,
                        mr_sha.as_deref(),
                    )
                    .await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    result.map_err(|e| {
                        // Strip the verbose glab prefix so the toast stays readable.
                        e.to_string()
                            .trim_start_matches("glab command failed: ")
                            .lines()
                            .next()
                            .unwrap_or("merge failed")
                            .to_string()
                    }),
                ));
            });
        }
        crate::app::ConfirmAction::BulkMergeMrs(items) => {
            let (squash, delete_branch, merge_strategy, auto_merge) = merge_options_from(&options);
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
                items_with_sha.push((project_path.clone(), *mr_iid, sha));
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
            let scope = app.scope.as_str().to_string();
            let total = items.len();
            tokio::spawn(async move {
                let mut failures: Vec<(u64, String)> = Vec::new();
                for (i, (project_path, mr_iid, sha)) in items_with_sha.into_iter().enumerate() {
                    if i > 0 {
                        crate::backend::rate_limit::pace_bulk_operation().await;
                    }
                    let proj = if !project_path.is_empty() {
                        project_path
                    } else {
                        scope.clone()
                    };
                    match client
                        .merge_mr(
                            &proj,
                            mr_iid,
                            squash,
                            delete_branch,
                            merge_strategy,
                            auto_merge,
                            sha.as_deref(),
                        )
                        .await
                    {
                        Ok(_) => {}
                        Err(e) => failures.push((mr_iid, e.to_string())),
                    }
                }
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    if failures.is_empty() {
                        Ok(())
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
            let project_path = app.project_path_for_mr(iid);
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = client.revoke_mr(&project_path, iid).await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        crate::app::ConfirmAction::RebaseMr(iid) => {
            let project_path = app.project_path_for_mr(iid);
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = client.rebase_mr(&project_path, iid).await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        crate::app::ConfirmAction::SubmitReview(mr_iid) => {
            app.selector = Some(crate::app::Selector {
                title: " Submit Pull Request Review ".to_string(),
                all_items: vec![
                    "Approve".to_string(),
                    "Request Changes".to_string(),
                    "Comment".to_string(),
                ],
                selected_items: std::collections::HashSet::new(),
                cursor_idx: 0,
                search_query: String::new(),
                is_filtering: false,
                is_loading: false,
                entity_iid: mr_iid,
                entity_type: "mr".to_string(),
                field_type: "review_submit_status".to_string(),
                multi_select: false,
                state: ListState::default(),
            });
        }
    }
}

pub fn handle_help_keybinding(app: &mut App, key_event: &KeyEvent) -> bool {
    if app.show_help {
        return false;
    }

    let is_f1 = key_event.code == KeyCode::F(1);
    let is_help_key = is_f1 || keybinding_matches(&app.config.keybindings.global.help, key_event);

    if !is_help_key {
        return false;
    }

    // When the user is actively typing raw text into a text field, allow F1 to open help,
    // but keep regular character keys (like '?') so they can be typed into the field.
    let is_typing_text = app.text_input.is_some()
        || app.is_typing_search
        || app.job_trace_searching
        || app.edit_menu.as_ref().map_or(false, |m| m.editing)
        || app.diff_view.as_ref().map_or(false, |d| d.search_active)
        || app.selector.as_ref().map_or(false, |s| s.is_filtering);

    if is_typing_text && !is_f1 {
        return false;
    }

    app.show_help = true;
    app.help_search_query.clear();
    reset_help_selection(app);
    true
}

/// Rows a PageUp/PageDown in the help list moves by.
const HELP_PAGE_ROWS: usize = 10;

fn reset_help_selection(app: &mut App) {
    app.help_selected = 0;
    app.help_table_state = ratatui::widgets::TableState::default();
}

/// Moves the help selection; the renderer clamps it to the listed entries.
pub fn move_help_selection(app: &mut App, down: bool, rows: usize) {
    app.help_selected = if down {
        app.help_selected.saturating_add(rows)
    } else {
        app.help_selected.saturating_sub(rows)
    };
}

pub fn handle_help_overlay(app: &mut App, key_event: &KeyEvent) -> bool {
    if app.show_help {
        match key_event.code {
            KeyCode::Esc | KeyCode::Enter => {
                app.show_help = false;
                app.help_search_query.clear();
            }
            KeyCode::Down => move_help_selection(app, true, 1),
            KeyCode::Up => move_help_selection(app, false, 1),
            KeyCode::Char('n') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                move_help_selection(app, true, 1)
            }
            KeyCode::Char('p') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                move_help_selection(app, false, 1)
            }
            KeyCode::PageDown => move_help_selection(app, true, HELP_PAGE_ROWS),
            KeyCode::PageUp => move_help_selection(app, false, HELP_PAGE_ROWS),
            KeyCode::Home => app.help_selected = 0,
            KeyCode::End => app.help_selected = usize::MAX,
            KeyCode::Backspace => {
                app.help_search_query.pop();
                reset_help_selection(app);
            }
            KeyCode::Char(c) => {
                app.help_search_query.push(c);
                reset_help_selection(app);
            }
            _ => {}
        }
        return true;
    }
    false
}

pub fn handle_switch_repo(app: &mut App, key_event: &KeyEvent) -> bool {
    let is_switch_repo = (key_event.code == KeyCode::Char('s')
        && key_event
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL))
        || (key_event.code == KeyCode::Char('S')
            && key_event
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL))
        || crate::keybinding::keybinding_matches(
            &app.config.keybindings.global.switch_repo,
            key_event,
        );

    if is_switch_repo
        && app.text_input.is_none()
        && app.edit_menu.is_none()
        && app.selector.is_none()
        && !app.is_typing_search
        && !app.job_trace_searching
        && !app.diff_view.as_ref().is_some_and(|d| d.search_active)
    {
        let mut items = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut switch_repo_paths = std::collections::HashMap::new();
        let mut switch_repo_groups = std::collections::HashSet::new();

        // Pin the current context to the top so the active group (or the
        // group implied by the current repo) stays visible even before the
        // recent-group cache has been warmed.
        let current_group = match &app.scope {
            crate::scope::Scope::Group(g) => Some(g.clone()),
            crate::scope::Scope::Repository(r) => r.rsplit_once('/').map(|(g, _)| g.to_string()),
        };
        if let Some(g) = current_group {
            if !g.trim().is_empty() && seen.insert(g.clone()) {
                switch_repo_groups.insert(g.clone());
                items.push(g);
            }
        }

        // Then every group the user can reach — recently switched groups
        // plus the group implied by each cached repo's remote. Newly
        // discovered groups are persisted so the next opener doesn't need
        // the git/auth probes again. Persist in one batched write instead
        // of one read-modify-write per new group.
        let recent_groups: std::collections::HashSet<String> =
            crate::utils::cache::get_recent_groups()
                .into_iter()
                .collect();
        let mut new_groups: Vec<String> = Vec::new();
        for g in crate::utils::cache::get_available_groups() {
            if g.trim().is_empty() || !seen.insert(g.clone()) {
                continue;
            }
            switch_repo_groups.insert(g.clone());
            items.push(g.clone());
            if !recent_groups.contains(&g) {
                new_groups.push(g);
            }
        }
        crate::utils::cache::add_recent_groups(&new_groups);

        // Finally the repositories. A repo that shares a display name with
        // an already-listed group yields to the group.
        for repo in crate::utils::cache::get_switchable_repos() {
            if !seen.insert(repo.display.clone()) {
                continue;
            }
            switch_repo_paths.insert(repo.display.clone(), repo.absolute_path);
            items.push(repo.display);
        }
        app.switch_repo_paths = switch_repo_paths;
        app.switch_repo_groups = switch_repo_groups;

        app.selector = Some(crate::app::Selector {
            title: " Switch Repository / Group ".to_string(),
            all_items: items,
            selected_items: {
                let mut s = std::collections::HashSet::new();
                match &app.scope {
                    crate::scope::Scope::Group(g) => {
                        s.insert(g.clone());
                    }
                    crate::scope::Scope::Repository(_) => {
                        if let Ok(cwd) = std::env::current_dir() {
                            if let Some(name) = cwd.file_name().and_then(|n| n.to_str()) {
                                s.insert(name.to_string());
                            }
                        }
                    }
                }
                s
            },
            cursor_idx: 0,
            search_query: String::new(),
            is_filtering: false,
            is_loading: false,
            entity_iid: 0,
            entity_type: "app".to_string(),
            field_type: "switch_repo".to_string(),
            multi_select: false,
            state: {
                let mut s = ListState::default();
                s.select(Some(0));
                s
            },
        });
        return true;
    }
    false
}

pub fn handle_refresh(
    app: &mut App,
    key_event: &KeyEvent,
    last_refresh: &mut Instant,
    tx: UnboundedSender<Event>,
) -> bool {
    let is_refresh = key_event.code == KeyCode::F(5)
        || (key_event.code == KeyCode::Char('r')
            && key_event
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL))
        || (key_event.code == KeyCode::Char('R')
            && key_event
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL))
        || keybinding_matches(&app.config.keybindings.global.refresh, key_event);

    if is_refresh
        && app.text_input.is_none()
        && app.date_picker.is_none()
        && app.edit_menu.is_none()
        && app.selector.is_none()
        && !app.is_typing_search
        && !app.job_trace_searching
        && !app.diff_view.as_ref().is_some_and(|d| d.search_active)
    {
        *last_refresh = Instant::now();
        app.last_attr_refresh = Instant::now();
        if let Some(client) = app.gitlab_client.clone() {
            if !app.loading_tabs.contains(&app.active_tab) {
                app.start_loading_tab(app.active_tab);
                spawn_refresh_active_tab(&client, &app.scope, app.active_tab, tx.clone());
            }
            spawn_fetch_repo_attributes(&client.muted(), &app.scope, tx);
        }
        return true;
    }
    false
}

pub fn handle_date_picker(
    app: &mut App,
    key_event: &KeyEvent,
    terminal: &mut AppTerminal,
    tx: UnboundedSender<Event>,
) -> bool {
    if let Some(mut date_picker) = app.date_picker.take() {
        match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') => {}
            KeyCode::Char('h') | KeyCode::Left => {
                date_picker.move_day(-1);
                app.date_picker = Some(date_picker);
            }
            KeyCode::Char('l') | KeyCode::Right => {
                date_picker.move_day(1);
                app.date_picker = Some(date_picker);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                date_picker.move_day(-7);
                app.date_picker = Some(date_picker);
            }
            KeyCode::Char('j') | KeyCode::Down => {
                date_picker.move_day(7);
                app.date_picker = Some(date_picker);
            }
            KeyCode::Char('[') | KeyCode::PageUp => {
                date_picker.move_month(-1);
                app.date_picker = Some(date_picker);
            }
            KeyCode::Char(']') | KeyCode::PageDown => {
                date_picker.move_month(1);
                app.date_picker = Some(date_picker);
            }
            KeyCode::Enter => {
                let selected_val = date_picker.value_string();
                match date_picker.action {
                    crate::app::DatePickerAction::EditField {
                        entity_iid,
                        entity_type,
                        field_type,
                    } => {
                        let active_tab = app.active_tab;
                        apply_field_text_change(
                            app,
                            &entity_type,
                            entity_iid,
                            &field_type,
                            selected_val,
                            terminal,
                            tx,
                            active_tab,
                        );
                        rebuild_edit_menu(app, &entity_type, entity_iid);
                    }
                    crate::app::DatePickerAction::EditNewField { field_idx } => {
                        if let Some(ref mut menu) = app.edit_menu {
                            if field_idx < menu.fields.len() {
                                menu.fields[field_idx].value = selected_val;
                            }
                        }
                    }
                }
            }
            _ => {
                app.date_picker = Some(date_picker);
            }
        }
        return true;
    }
    false
}

/// Keys for the review threads overlay. Selectors and text inputs opened from
/// it (comment actions, replies) sit on top and take their keys first.
pub fn handle_review_threads(app: &mut App, key_event: &KeyEvent) -> bool {
    if app.selector.is_some() || app.text_input.is_some() {
        return false;
    }
    let Some(mut overview) = app.review_threads.take() else {
        return false;
    };

    let mut keep_open = true;
    match key_event.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('T') => keep_open = false,
        KeyCode::Char('j') | KeyCode::Down => overview.next(),
        KeyCode::Char('k') | KeyCode::Up => overview.previous(),
        KeyCode::Char('g') | KeyCode::Home => overview.first(),
        KeyCode::Char('G') | KeyCode::End => overview.last(),
        KeyCode::Char('J') => overview.preview_scroll = overview.preview_scroll.saturating_add(1),
        KeyCode::Char('K') => overview.preview_scroll = overview.preview_scroll.saturating_sub(1),
        KeyCode::Char('u') => overview.toggle_unresolved_only(),
        KeyCode::Enter => keep_open = !jump_to_review_thread(app, &overview),
        KeyCode::Char('a') => open_review_thread_actions(app, &overview),
        _ => {}
    }
    if keep_open {
        app.review_threads = Some(overview);
    }
    true
}

/// Moves the diff cursor onto the selected thread's anchor. Returns whether
/// the diff now shows it.
fn jump_to_review_thread(app: &mut App, overview: &crate::app::ReviewThreadsOverview) -> bool {
    let Some(thread) = overview.selected_thread() else {
        return false;
    };
    let position = match (&thread.anchor, thread.root().position.as_ref()) {
        (crate::domain::review_threads::ThreadAnchor::General, _) | (_, None) => {
            app.show_error("General comments are not attached to a line in the diff.".to_string());
            return false;
        }
        (_, Some(position)) => position,
    };
    let Some(diff_view) = app.diff_view.as_mut() else {
        return false;
    };
    let was_hiding_reviewed = diff_view.hide_reviewed;
    if diff_view.jump_to_anchor(position) {
        if was_hiding_reviewed && !diff_view.hide_reviewed {
            app.hide_reviewed_files = false;
            app.status_message = Some("Showing reviewed files to reach the thread".to_string());
        }
        return true;
    }
    let file_path = position
        .new_path
        .as_deref()
        .or(position.old_path.as_deref())
        .unwrap_or_default();
    app.show_error(format!("{file_path} is no longer part of this diff."));
    false
}

fn open_review_thread_actions(app: &mut App, overview: &crate::app::ReviewThreadsOverview) {
    let Some(thread) = overview.selected_thread() else {
        return;
    };
    let Some(mr_iid) = app.diff_view.as_ref().map(|d| d.mr_iid) else {
        return;
    };
    app.selector = Some(if thread.notes.len() == 1 {
        crate::app::Selector::comment_actions(thread.root(), mr_iid, app.is_github())
    } else {
        crate::app::Selector::comment_choice(
            thread
                .notes
                .iter()
                .map(crate::app::Selector::comment_choice_item)
                .collect(),
            mr_iid,
        )
    });
}

#[cfg(test)]
mod tests {
    use super::{handle_help_keybinding, handle_help_overlay, handle_review_threads};
    use crate::app::{App, DiffView, EditEntityKind, EditMenu, Selector};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn help_arrows_move_the_selection_and_typing_filters_from_the_top() {
        use crossterm::event::KeyModifiers;
        let mut app = App::default();
        app.show_help = true;
        let press = |code| KeyEvent::new(code, KeyModifiers::NONE);

        handle_help_overlay(&mut app, &press(KeyCode::Down));
        handle_help_overlay(&mut app, &press(KeyCode::Down));
        handle_help_overlay(&mut app, &press(KeyCode::PageDown));
        handle_help_overlay(&mut app, &press(KeyCode::Up));
        assert_eq!(app.help_selected, 11);

        handle_help_overlay(&mut app, &press(KeyCode::Char('j')));
        assert_eq!(app.help_search_query, "j", "letters still filter");
        assert_eq!(app.help_selected, 0, "a new filter starts from the top");
    }

    #[test]
    fn help_search_consumes_q_instead_of_closing_or_quitting() {
        let mut app = App::default();
        app.show_help = true;

        let handled = handle_help_overlay(
            &mut app,
            &KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );

        assert!(handled);
        assert_eq!(app.help_search_query, "q");
        assert!(app.show_help);
    }

    #[test]
    fn search_mode_blocks_switch_repo_and_refresh() {
        use std::time::Instant;
        use tokio::sync::mpsc::unbounded_channel;
        let (tx, _rx) = unbounded_channel();
        let mut app = App::default();
        app.is_typing_search = true;

        let mut last_refresh = Instant::now();
        let handled_refresh = super::handle_refresh(
            &mut app,
            &KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
            &mut last_refresh,
            tx,
        );
        assert!(!handled_refresh);

        let handled_switch = super::handle_switch_repo(
            &mut app,
            &KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
        );
        assert!(!handled_switch);
    }

    #[test]
    fn help_keybinding_accessible_in_all_views() {
        let mut app = App::default();

        // 1. Normal view with '?' and F1
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
        ));
        assert!(app.show_help);
        app.show_help = false;

        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE),
        ));
        assert!(app.show_help);
        app.show_help = false;

        // 2. Edit menu (Inspector / Form) - not actively editing text
        app.edit_menu = Some(EditMenu {
            entity_project: String::new(),
            entity_iid: 1,
            entity_kind: EditEntityKind::EditIssue,
            title: "Edit Issue".to_string(),
            fields: vec![],
            initial_fields: std::collections::HashMap::new(),
            selected_idx: 0,
            editing: false,
            cursor_pos: 0,
            state: ratatui::widgets::ListState::default(),
            desc_scroll: 0,
            workflow_inputs: vec![],
        });
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
        ));
        assert!(app.show_help);
        app.show_help = false;

        // Edit menu - actively editing text: '?' types into field, but F1 opens help
        if let Some(ref mut menu) = app.edit_menu {
            menu.editing = true;
        }
        assert!(!handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
        ));
        assert!(!app.show_help);
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE),
        ));
        assert!(app.show_help);
        app.show_help = false;
        app.edit_menu = None;

        // 3. Diff View - not searching
        app.diff_view = Some(DiffView::new(
            1,
            String::new(),
            "diff --git a/a b/b\n--- a/a\n+++ b/b\n@@ -1 +1 @@\n-old\n+new\n".to_string(),
        ));
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
        ));
        assert!(app.show_help);
        app.show_help = false;

        // Diff View - searching: '?' types into search, F1 opens help
        if let Some(ref mut diff_view) = app.diff_view {
            diff_view.search_active = true;
        }
        assert!(!handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
        ));
        assert!(!app.show_help);
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE),
        ));
        assert!(app.show_help);
        app.show_help = false;
        app.diff_view = None;

        // 4. Selector overlay
        app.selector = Some(Selector {
            title: "Select Label".to_string(),
            all_items: vec!["bug".to_string()],
            selected_items: std::collections::HashSet::new(),
            cursor_idx: 0,
            search_query: String::new(),
            is_filtering: false,
            is_loading: false,
            entity_iid: 1,
            entity_type: "issue".to_string(),
            field_type: "labels".to_string(),
            multi_select: true,
            state: ratatui::widgets::ListState::default(),
        });
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
        ));
        assert!(app.show_help);
        app.show_help = false;
        app.selector = None;

        // 5. Column checklist
        app.focus_column_checklist = true;
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
        ));
        assert!(app.show_help);
        app.show_help = false;
        app.focus_column_checklist = false;

        // 6. Date picker
        app.date_picker = Some(crate::app::DatePicker::new(
            "Select Date".to_string(),
            "2026-08-23",
            crate::app::DatePickerAction::EditNewField { field_idx: 0 },
        ));
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
        ));
        assert!(app.show_help);
        app.show_help = false;
        app.date_picker = None;
    }

    const REVIEW_DIFF: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,3 @@
 fn a() {
+    b();
 }
";

    fn line_position(path: &str, new_line: u64) -> crate::domain::mr::NotePosition {
        crate::domain::mr::NotePosition {
            new_path: Some(path.to_string()),
            old_path: Some(path.to_string()),
            new_line: Some(new_line),
            old_line: None,
            start_line: None,
            line_range: None,
        }
    }

    fn review_note(
        id: u64,
        discussion: &str,
        position: Option<crate::domain::mr::NotePosition>,
    ) -> crate::domain::mr::DiscussionNote {
        crate::domain::mr::DiscussionNote {
            id,
            body: format!("note {id}"),
            author: crate::domain::mr::Author {
                username: "alice".to_string(),
            },
            created_at: format!("2026-09-2{id}T10:00:00Z"),
            system: false,
            position,
            discussion_id: Some(discussion.to_string()),
            resolved: Some(false),
            resolvable: Some(true),
        }
    }

    /// Diff view open on `REVIEW_DIFF` with the threads overlay showing `notes`.
    fn app_with_review_threads(notes: Vec<crate::domain::mr::DiscussionNote>) -> App {
        let mut app = App::default();
        let diff_view = DiffView::new(7, "acme/widget".to_string(), REVIEW_DIFF.to_string());
        app.review_threads = Some(crate::app::ReviewThreadsOverview::new(&notes, &diff_view));
        app.diff_view = Some(diff_view);
        app.current_comments = notes;
        app
    }

    fn press(app: &mut App, code: KeyCode) -> bool {
        handle_review_threads(app, &KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn selected_root_id(app: &App) -> Option<u64> {
        app.review_threads
            .as_ref()
            .and_then(|o| o.selected_thread())
            .map(|t| t.root().id)
    }

    #[test]
    fn review_threads_navigation_wraps_around_both_ends() {
        let mut app = app_with_review_threads(vec![
            review_note(1, "first", None),
            review_note(2, "second", Some(line_position("src/lib.rs", 2))),
            review_note(3, "third", Some(line_position("src/lib.rs", 3))),
        ]);
        assert_eq!(selected_root_id(&app), Some(1));

        assert!(press(&mut app, KeyCode::Char('k')));
        assert_eq!(
            selected_root_id(&app),
            Some(3),
            "k on the first row wraps to the last"
        );

        press(&mut app, KeyCode::Char('j'));
        assert_eq!(
            selected_root_id(&app),
            Some(1),
            "j on the last row wraps to the first"
        );

        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(selected_root_id(&app), Some(1));

        press(&mut app, KeyCode::Up);
        assert_eq!(selected_root_id(&app), Some(3));
    }

    #[test]
    fn review_threads_enter_reaches_a_reviewed_file_hidden_by_the_filter() {
        let mut app = app_with_review_threads(vec![review_note(
            1,
            "anchored",
            Some(line_position("src/lib.rs", 2)),
        )]);
        app.hide_reviewed_files = true;
        if let Some(diff_view) = &mut app.diff_view {
            diff_view.restore_review_state(
                std::collections::HashSet::from(["src/lib.rs".to_string()]),
                true,
            );
        }

        assert!(press(&mut app, KeyCode::Enter));

        assert!(
            app.review_threads.is_none(),
            "a successful jump closes the overlay"
        );
        assert!(
            !app.hide_reviewed_files,
            "the session filter must follow, or the next re-fetch hides the file again"
        );
        let diff_view = app.diff_view.as_ref().unwrap();
        let line = &diff_view.lines[diff_view.cursor_idx];
        assert_eq!(
            (line.file_path.as_str(), line.new_line_num),
            ("src/lib.rs", Some(2))
        );
    }

    #[test]
    fn review_threads_enter_on_a_general_comment_keeps_the_overlay_open() {
        let mut app = app_with_review_threads(vec![review_note(1, "general", None)]);

        assert!(press(&mut app, KeyCode::Enter));

        assert!(app.review_threads.is_some());
        assert!(app.error_message.is_some());
    }

    #[test]
    fn review_threads_actions_open_a_selector_that_takes_the_keys() {
        let mut app = app_with_review_threads(vec![
            review_note(1, "thread", Some(line_position("src/lib.rs", 2))),
            review_note(2, "thread", Some(line_position("src/lib.rs", 2))),
        ]);

        assert!(press(&mut app, KeyCode::Char('a')));

        let selector = app.selector.as_ref().expect("a opens the comment picker");
        assert_eq!(selector.field_type, "comment_select");
        assert_eq!(selector.all_items.len(), 2);
        assert!(
            !press(&mut app, KeyCode::Char('j')),
            "keys belong to the selector on top"
        );
        assert!(app.review_threads.is_some());
    }
}
