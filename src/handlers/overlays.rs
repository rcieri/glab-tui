use crate::AppTerminal;
use crate::app::App;
use crate::entity_editor::{apply_field_text_change, rebuild_edit_menu};
use crate::event::Event;
use crate::fetch::{spawn_fetch_repo_attributes, spawn_refresh_active_tab};
use crate::keybinding::keybinding_matches;
use crossterm::event::{KeyCode, KeyEvent};
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

    let shift = key_event
        .modifiers
        .contains(crossterm::event::KeyModifiers::SHIFT);
    match key_event.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if dialog.is_on_submit() || dialog.is_on_cancel() {
                if !dialog.options.is_empty() {
                    dialog.cursor_idx = dialog.options.len();
                }
            } else if dialog.cursor_idx > 1 {
                dialog.cursor_idx -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !dialog.is_on_cancel() && !dialog.is_on_submit() {
                if dialog.cursor_idx < dialog.options.len() {
                    dialog.cursor_idx += 1;
                } else {
                    dialog.cursor_idx = dialog.cancel_idx();
                }
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

    let _ = shift; // SHIFT is consumed above via BackTab
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
            app.pending_delete_milestone_iid = Some(iid);
            let client = app.gitlab_client.clone().unwrap();
            let project_path = app.project_context.clone();
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
            if let Some(m) = app.milestones.items.iter_mut().find(|m| m.iid == iid) {
                m.state = "closed".to_string();
            }
            app.project_cache.milestones = app.milestones.items.clone();
            let client = app.gitlab_client.clone().unwrap();
            let project_path = app.project_context.clone();
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
            if let Some(m) = app.milestones.items.iter_mut().find(|m| m.iid == iid) {
                m.state = "active".to_string();
            }
            app.project_cache.milestones = app.milestones.items.clone();
            let client = app.gitlab_client.clone().unwrap();
            let project_path = app.project_context.clone();
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
            app.pending_delete_release_tag = Some(tag_name.clone());
            let client = app.gitlab_client.clone().unwrap();
            let project_path = app.project_context.clone();
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
            let project_path = app.project_context.clone();
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
            if let Some(pos) = app.issues.items.iter().position(|i| i.iid == iid) {
                app.issues.items.remove(pos);
            }
            app.update_filter_selection();
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
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
            let project_path = app.project_context.clone();
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
            if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                item.state = "opened".to_string();
            }
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
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
            if let Some(pos) = app.mrs.items.iter().position(|m| m.iid == iid) {
                app.mrs.items.remove(pos);
            }
            app.update_filter_selection();
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
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
            let project_path = app.project_context.clone();
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
            if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                item.state = "opened".to_string();
            }
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
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
            if let Some(pos) = app.mrs.items.iter().position(|m| m.iid == iid) {
                app.mrs.items.remove(pos);
            }
            app.update_filter_selection();
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
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
                    )
                    .await;
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        crate::app::ConfirmAction::BulkMergeMrs(iids) => {
            let (squash, delete_branch, merge_strategy, auto_merge) = merge_options_from(&options);
            let iids: Vec<u64> = iids.clone();
            for mr_iid in &iids {
                if let Some(pos) = app.mrs.items.iter().position(|m| m.iid == *mr_iid) {
                    app.mrs.items.remove(pos);
                }
            }
            app.update_filter_selection();
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let mut failures = Vec::new();
                for mr_iid in iids {
                    match client
                        .merge_mr(
                            &project_path,
                            mr_iid,
                            squash,
                            delete_branch,
                            merge_strategy,
                            auto_merge,
                        )
                        .await
                    {
                        Ok(_) => {}
                        Err(e) => failures.push(format!("#{mr_iid}: {e}")),
                    }
                }
                let _ = tx2.send(Event::CommandCompleted(
                    crate::app::Tab::MergeRequests,
                    if failures.is_empty() {
                        Ok(())
                    } else {
                        Err(failures.join(", "))
                    },
                ));
            });
        }
        crate::app::ConfirmAction::RevokeMr(iid) => {
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
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
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
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
    if keybinding_matches(&app.config.keybindings.global.help, key_event)
        && app.text_input.is_none()
        && app.edit_menu.is_none()
        && app.selector.is_none()
        && !app.show_help
        && !app.focus_column_checklist
    {
        app.show_help = true;
        app.help_search_query.clear();
        return true;
    }
    false
}

pub fn handle_help_overlay(app: &mut App, key_event: &KeyEvent) -> bool {
    if app.show_help {
        match key_event.code {
            KeyCode::Esc | KeyCode::Enter => {
                app.show_help = false;
                app.help_search_query.clear();
            }
            KeyCode::Backspace => {
                app.help_search_query.pop();
            }
            KeyCode::Char(c) => {
                app.help_search_query.push(c);
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
                .contains(crossterm::event::KeyModifiers::CONTROL));

    if is_switch_repo
        && app.text_input.is_none()
        && app.edit_menu.is_none()
        && app.selector.is_none()
    {
        let items = crate::utils::cache::get_switchable_repos();

        app.selector = Some(crate::app::Selector {
            title: " Switch Repository ".to_string(),
            all_items: items,
            selected_items: {
                let mut s = std::collections::HashSet::new();
                if let Ok(cwd) = std::env::current_dir() {
                    if let Some(name) = cwd.file_name().and_then(|n| n.to_str()) {
                        s.insert(name.to_string());
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
    {
        *last_refresh = Instant::now();
        app.last_attr_refresh = Instant::now();
        if let Some(client) = app.gitlab_client.clone() {
            if !app.loading_tabs.contains(&app.active_tab) {
                app.start_loading_tab(app.active_tab);
                spawn_refresh_active_tab(&client, &app.project_context, app.active_tab, tx.clone());
            }
            spawn_fetch_repo_attributes(&client.muted(), &app.project_context, tx);
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

#[cfg(test)]
mod tests {
    use super::handle_help_overlay;
    use crate::app::App;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
}
