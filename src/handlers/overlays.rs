use crate::AppTerminal;
use crate::app::App;
use crate::cli::app_cli;
use crate::entity_editor::{apply_field_text_change, rebuild_edit_menu};
use crate::event::Event;
use crate::fetch::spawn_refresh_active_tab;
use crate::keybinding::keybinding_matches;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::ListState;
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

pub fn handle_submit_review_prompt(app: &mut App, key_event: &KeyEvent) -> bool {
    if let Some(mr_iid) = app.show_submit_review_prompt {
        match key_event.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                app.show_submit_review_prompt = None;
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
            KeyCode::Char('n') | KeyCode::Char('N') => {
                app.show_submit_review_prompt = None;
                app.draft_comments.clear();
                app.in_review_mode = false;
                app.diff_view = None;
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                app.show_submit_review_prompt = None;
            }
            _ => {}
        }
        return true;
    }
    false
}

pub async fn handle_confirm_popup(
    app: &mut App,
    key_event: &KeyEvent,
    terminal: &mut AppTerminal,
    tx: UnboundedSender<Event>,
) -> bool {
    if let Some(confirm_action) = app.confirm_popup.take() {
        match key_event.code {
            KeyCode::Left | KeyCode::Char('h') => {
                app.confirm_popup_selected_yes = true;
                app.confirm_popup = Some(confirm_action);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                app.confirm_popup_selected_yes = false;
                app.confirm_popup = Some(confirm_action);
            }
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                if key_event.code == KeyCode::Enter && !app.confirm_popup_selected_yes {
                    // Cancel
                    return true;
                }
                match confirm_action {
                    crate::app::ConfirmAction::DeleteMilestone(iid) => {
                        let client = app.gitlab_client.clone().unwrap();
                        let project_path = app.project_context.clone();
                        let _ = tx.send(Event::CommandStarted(format!(
                            "Deleting milestone #{}",
                            iid
                        )));
                        tokio::spawn(async move {
                            let res = crate::gitlab::milestones::delete_milestone(
                                &client,
                                &project_path,
                                iid,
                            )
                            .await;
                            match res {
                                Ok(_) => {
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
                    crate::app::ConfirmAction::DeleteRelease(tag_name) => {
                        let client = app.gitlab_client.clone().unwrap();
                        let project_path = app.project_context.clone();
                        let _ = tx.send(Event::CommandStarted(format!(
                            "Deleting release {}",
                            tag_name
                        )));
                        tokio::spawn(async move {
                            let res = crate::gitlab::releases::delete_release(
                                &client,
                                &project_path,
                                &tag_name,
                            )
                            .await;
                            match res {
                                Ok(_) => {
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
                        let _ = tx.send(Event::CommandStarted(format!(
                            "Deleting branch: {}",
                            branch_name
                        )));
                        tokio::spawn(async move {
                            let res = crate::gitlab::branches::delete_branch(
                                &client,
                                &project_path,
                                &branch_name,
                            )
                            .await;
                            match res {
                                Ok(_) => {
                                    let _ = tx.send(Event::CommandCompleted(
                                        crate::app::Tab::Branches,
                                        Ok(()),
                                    ));
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
                        let cli = app_cli(app);
                        let args = vec!["issue".to_string(), "close".to_string(), iid.to_string()];
                        let active_tab = app.active_tab;
                        crate::run_cli(&cli, &args, terminal, tx.clone(), active_tab).await;
                        if let Some(pos) = app.issues.items.iter().position(|i| i.iid == iid) {
                            app.issues.items.remove(pos);
                        }
                        app.update_filter_selection();
                    }
                    crate::app::ConfirmAction::DeleteIssue(iid) => {
                        let cli = app_cli(app);
                        let is_github = cli.is_github;
                        let project_path = app.project_context.clone();
                        let client = app.gitlab_client.clone().unwrap();
                        let _ = tx.send(Event::CommandStarted(format!("Deleting issue #{}", iid)));
                        tokio::spawn(async move {
                            let res = if is_github {
                                client
                                    .execute_raw_command(
                                        "gh",
                                        &[
                                            "issue",
                                            "delete",
                                            &iid.to_string(),
                                            "-R",
                                            &project_path,
                                            "--yes",
                                        ],
                                        "Deleting Issue",
                                    )
                                    .await
                            } else {
                                let encoded_path = project_path.replace("/", "%2F");
                                let endpoint = format!("/projects/{}/issues/{}", encoded_path, iid);
                                client.execute_gitlab_api(&endpoint, "DELETE", None).await
                            };
                            match res {
                                Ok(_) => {
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
                    crate::app::ConfirmAction::CloseMr(iid) => {
                        let cli = app_cli(app);
                        let args = vec![
                            cli.entity("mr").to_string(),
                            "close".to_string(),
                            iid.to_string(),
                        ];
                        let active_tab = app.active_tab;
                        crate::run_cli(&cli, &args, terminal, tx.clone(), active_tab).await;
                        if let Some(pos) = app.mrs.items.iter().position(|m| m.iid == iid) {
                            app.mrs.items.remove(pos);
                        }
                        app.update_filter_selection();
                    }
                    crate::app::ConfirmAction::DeleteMr(iid) => {
                        let project_path = app.project_context.clone();
                        let client = app.gitlab_client.clone().unwrap();
                        let _ = tx.send(Event::CommandStarted(format!("Deleting MR #{}", iid)));
                        tokio::spawn(async move {
                            let encoded_path = project_path.replace("/", "%2F");
                            let endpoint =
                                format!("/projects/{}/merge_requests/{}", encoded_path, iid);
                            let res = client.execute_gitlab_api(&endpoint, "DELETE", None).await;
                            match res {
                                Ok(_) => {
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
                    crate::app::ConfirmAction::MergeMr(iid) => {
                        let cli = app_cli(app);
                        let args = if cli.is_github {
                            vec![
                                "pr".to_string(),
                                "merge".to_string(),
                                iid.to_string(),
                                "--delete-branch".to_string(),
                                "--squash".to_string(),
                            ]
                        } else {
                            vec![
                                "mr".to_string(),
                                "merge".to_string(),
                                iid.to_string(),
                                "--remove-source-branch".to_string(),
                                "--squash".to_string(),
                            ]
                        };
                        let active_tab = app.active_tab;
                        crate::run_cli(&cli, &args, terminal, tx.clone(), active_tab).await;
                        if let Some(pos) = app.mrs.items.iter().position(|m| m.iid == iid) {
                            app.mrs.items.remove(pos);
                        }
                        app.update_filter_selection();
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {}
            _ => {
                app.confirm_popup = Some(confirm_action);
            }
        }
        return true;
    }
    false
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
            is_filtering: true,
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
        if let Some(client) = app.gitlab_client.clone() {
            if !app.loading_tabs.contains(&app.active_tab) {
                app.start_loading_tab(app.active_tab);
                spawn_refresh_active_tab(&client, &app.project_context, app.active_tab, tx);
            }
        }
        return true;
    }
    false
}

pub async fn handle_date_picker(
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
                        )
                        .await;
                        rebuild_edit_menu(app, &entity_type, entity_iid);
                    }
                    crate::app::DatePickerAction::EditNewField { field_idx } => {
                        if let Some(ref mut menu) = app.edit_menu {
                            if field_idx < menu.fields.len() {
                                menu.fields[field_idx].1 = selected_val;
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
