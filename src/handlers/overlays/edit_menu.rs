use crate::app::App;
use crate::effect::{EditPurpose, Effects, ForegroundJob};
use crate::event::{CommandOutcome, Event};
use crate::git_helpers::get_workflow_files;
use crate::keybinding::KeyPress;
use crate::keybinding::keybinding_matches;
use crate::templates::list_templates;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::widgets::ListState;
use tokio::sync::mpsc::UnboundedSender;

fn parse_key_value_pairs(input: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut current = String::new();
    let mut in_parens: i32 = 0;
    for c in input.chars() {
        match c {
            '(' => {
                in_parens += 1;
                current.push(c);
            }
            ')' => {
                in_parens = in_parens.saturating_sub(1);
                current.push(c);
            }
            ',' if in_parens == 0 => {
                if !current.trim().is_empty() {
                    if let Some(pos) = current.find(':').or_else(|| current.find('=')) {
                        let k = current[..pos].trim().to_string();
                        let v = current[pos + 1..].trim().to_string();
                        if !k.is_empty() {
                            pairs.push((k, v));
                        }
                    }
                }
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }
    if !current.trim().is_empty() {
        if let Some(pos) = current.find(':').or_else(|| current.find('=')) {
            let k = current[..pos].trim().to_string();
            let v = current[pos + 1..].trim().to_string();
            if !k.is_empty() {
                pairs.push((k, v));
            }
        }
    }
    pairs
}

/// Resolve the project a create/edit mutation should target, and validate it
/// when the app is in group scope (where there is no implicit project).
fn resolve_target_project(
    menu: &crate::app::EditMenu,
    scope: &crate::scope::Scope,
) -> Result<String, String> {
    let project = menu
        .fields
        .iter()
        .find(|f| f.label == "Project")
        .map(|f| f.value.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if !menu.entity_project.is_empty() {
                menu.entity_project.clone()
            } else {
                scope.as_str().to_string()
            }
        });
    if scope.is_group() {
        crate::backend::glab::validate_project_path(&project).map_err(|_| {
            "Please specify a project path (e.g. owner/project) in group view".to_string()
        })?;
    }
    Ok(project)
}

/// Keys while an edit or create form is open.
pub fn handle_key(
    app: &mut App,
    key_event: &KeyPress,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    let Some(mut menu) = app.edit_menu.take() else {
        return;
    };
    let is_ctrl_x = (key_event.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(
            key_event.code,
            KeyCode::Char('x') | KeyCode::Char('X') | KeyCode::Char('\x18')
        ))
        || key_event.code == KeyCode::Char('\x18');
    let is_submit_edit =
        keybinding_matches(&app.config.keybindings.global.submit_edit, key_event) || is_ctrl_x;

    if !is_submit_edit && menu.editing {
        match key_event.code {
            KeyCode::Esc | KeyCode::Enter => {
                menu.editing = false;
                app.edit_menu = Some(menu);
            }
            KeyCode::Left => {
                if menu.selected_idx < menu.fields.len()
                    && menu.fields[menu.selected_idx].kind == crate::app::FieldType::Text
                {
                    if let Some(f) = menu.fields.get(menu.selected_idx) {
                        if let Some((byte_idx, _)) =
                            f.value[..menu.cursor_pos].char_indices().next_back()
                        {
                            menu.cursor_pos = byte_idx;
                        } else {
                            menu.cursor_pos = 0;
                        }
                    }
                }
                app.edit_menu = Some(menu);
            }
            KeyCode::Right => {
                if menu.selected_idx < menu.fields.len()
                    && menu.fields[menu.selected_idx].kind == crate::app::FieldType::Text
                {
                    if let Some(f) = menu.fields.get(menu.selected_idx) {
                        let char_len = f.value[menu.cursor_pos..]
                            .chars()
                            .next()
                            .map(|c| c.len_utf8())
                            .unwrap_or(0);
                        if char_len > 0 && menu.cursor_pos + char_len <= f.value.len() {
                            menu.cursor_pos += char_len;
                        }
                    }
                }
                app.edit_menu = Some(menu);
            }
            KeyCode::Home => {
                if menu.selected_idx < menu.fields.len()
                    && menu.fields[menu.selected_idx].kind == crate::app::FieldType::Text
                {
                    menu.cursor_pos = 0;
                }
                app.edit_menu = Some(menu);
            }
            KeyCode::End => {
                if menu.selected_idx < menu.fields.len()
                    && menu.fields[menu.selected_idx].kind == crate::app::FieldType::Text
                {
                    menu.cursor_pos = menu.fields[menu.selected_idx].value.len();
                }
                app.edit_menu = Some(menu);
            }
            KeyCode::Backspace => {
                if let Some(f) = menu.fields.get_mut(menu.selected_idx) {
                    menu.cursor_pos = menu.cursor_pos.min(f.value.len());
                    if let Some((byte_idx, _)) =
                        f.value[..menu.cursor_pos].char_indices().next_back()
                    {
                        f.value.remove(byte_idx);
                        menu.cursor_pos = byte_idx;
                    }
                }
                app.edit_menu = Some(menu);
            }
            KeyCode::Delete => {
                if let Some(f) = menu.fields.get_mut(menu.selected_idx) {
                    menu.cursor_pos = menu.cursor_pos.min(f.value.len());
                    if menu.cursor_pos < f.value.len() {
                        f.value.remove(menu.cursor_pos);
                    }
                }
                app.edit_menu = Some(menu);
            }
            KeyCode::Char(c) if key_event.modifiers.contains(KeyModifiers::CONTROL) && c == 'e' => {
                let field_name = if menu.selected_idx < menu.fields.len() {
                    menu.fields[menu.selected_idx].label.clone()
                } else {
                    String::new()
                };
                if field_name == "Description" || field_name == "Release Notes" {
                    let desc = if let Some(f) = menu.fields.iter().find(|f| {
                        (f.label == "Description" || f.label == "Release Notes")
                            && f.kind == crate::app::FieldType::Text
                    }) {
                        f.value.clone()
                    } else {
                        String::new()
                    };
                    app.edit_menu = Some(menu);
                    fx.suspend(ForegroundJob::Edit {
                        text: desc,
                        file_suffix: ".md".to_string(),
                        purpose: EditPurpose::MenuDescription,
                    });
                } else {
                    app.edit_menu = Some(menu);
                }
            }
            KeyCode::Char(c) => {
                if menu.selected_idx < menu.fields.len()
                    && menu.fields[menu.selected_idx].kind == crate::app::FieldType::Text
                {
                    if let Some(f) = menu.fields.get_mut(menu.selected_idx) {
                        f.value.insert(menu.cursor_pos, c);
                        menu.cursor_pos += c.len_utf8();
                    }
                }
                app.edit_menu = Some(menu);
            }
            _ => {
                // Disable all navigation buttons in edit mode
                app.edit_menu = Some(menu);
            }
        }
        return;
    }

    // Navigation mode (!menu.editing):
    match key_event.code {
        KeyCode::Esc => {
            // Close the edit form and restore the zoom state
            // captured when the menu opened: back to PREVIEW
            // if entered via double-Enter, or back to NORMAL
            // if entered via `e`.
            app.details_zoomed = app.prev_details_zoomed;
            // Close menu (drop menu by not reassigning)
        }
        KeyCode::Char('j') | KeyCode::Down | KeyCode::Tab => {
            let is_new = menu.entity_iid == 0 || menu.entity_kind.needs_submit();
            let max_idx = if is_new {
                menu.fields.len() + 1 // fields + spacer + submit
            } else {
                menu.fields.len() - 1
            };
            menu.selected_idx = if menu.selected_idx >= max_idx {
                0
            } else {
                menu.selected_idx + 1
            };
            // Skip section headers and ReadOnly fields.
            while menu.selected_idx < menu.fields.len()
                && (menu.fields[menu.selected_idx].kind == crate::app::FieldType::Section
                    || menu.fields[menu.selected_idx].kind == crate::app::FieldType::ReadOnly)
            {
                menu.selected_idx += 1;
            }
            if is_new && menu.selected_idx == menu.fields.len() {
                menu.selected_idx += 1;
            }
            menu.state.select(Some(menu.selected_idx));
            // Reset cursor for inline-editable fields
            if menu.selected_idx < menu.fields.len() {
                menu.cursor_pos = menu.fields[menu.selected_idx].value.len();
            }
            app.edit_menu = Some(menu);
        }
        // J/K: scroll description pane
        KeyCode::Char('J') => {
            menu.desc_scroll = menu.desc_scroll.saturating_add(1);
            app.edit_menu = Some(menu);
        }
        KeyCode::Char('K') => {
            menu.desc_scroll = menu.desc_scroll.saturating_sub(1);
            app.edit_menu = Some(menu);
        }
        KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => {
            let is_new = menu.entity_iid == 0 || menu.entity_kind.needs_submit();
            let max_idx = if is_new {
                menu.fields.len() + 1
            } else {
                menu.fields.len() - 1
            };
            menu.selected_idx = if menu.selected_idx == 0 {
                max_idx
            } else {
                menu.selected_idx - 1
            };
            // Skip section headers and ReadOnly fields.
            while menu.selected_idx < menu.fields.len()
                && (menu.fields[menu.selected_idx].kind == crate::app::FieldType::Section
                    || menu.fields[menu.selected_idx].kind == crate::app::FieldType::ReadOnly)
                && menu.selected_idx > 0
            {
                menu.selected_idx -= 1;
            }
            // Skip spacer row
            if is_new && menu.selected_idx == menu.fields.len() {
                menu.selected_idx = menu.fields.len().saturating_sub(1);
            }
            menu.state.select(Some(menu.selected_idx));
            // Reset cursor for inline-editable fields
            if menu.selected_idx < menu.fields.len() {
                menu.cursor_pos = menu.fields[menu.selected_idx].value.len();
            }
            app.edit_menu = Some(menu);
        }
        _ if is_submit_edit || key_event.code == KeyCode::Enter => {
            if is_submit_edit {
                menu.editing = false;
            }
            let entity_iid = menu.entity_iid;
            let entity_type = menu.entity_kind.legacy_string();
            let is_on_submit = (menu.selected_idx == menu.fields.len() + 1) || is_submit_edit;

            if is_on_submit {
                if entity_type == "new_issue" {
                    let project = match resolve_target_project(&menu, &app.scope) {
                        Ok(p) => p,
                        Err(e) => {
                            app.show_error(e);
                            app.edit_menu = Some(menu);
                            return;
                        }
                    };
                    let title = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Title")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let description = menu
                        .fields
                        .iter()
                        .find(|f| {
                            (f.label == "Description" || f.label == "Release Notes")
                                && f.kind == crate::app::FieldType::Text
                        })
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let labels = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Labels")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let assignees = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Assignees")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let milestone = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Milestone")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let due_date = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Due Date")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let weight = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Weight")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();

                    app.edit_menu = None;
                    let Some(client) = app.gitlab_client.clone() else {
                        app.show_error("Backend not initialized".to_string());
                        return;
                    };
                    let tx = tx.clone();
                    let tab = app.active_tab;
                    fx.spawn(async move {
                        match client
                            .create_issue(
                                &project,
                                &crate::backend::NewIssue {
                                    title: &title,
                                    description: &description,
                                    labels: &labels,
                                    assignees: &assignees,
                                    milestone: &milestone,
                                    due_date: &due_date,
                                    weight: &weight,
                                },
                            )
                            .await
                        {
                            Ok(_) => {
                                let _ = tx.send(Event::CommandCompleted(
                                    tab,
                                    Ok(CommandOutcome::Mutated),
                                ));
                            }
                            Err(e) => {
                                let _ = tx.send(Event::CommandCompleted(tab, Err(e.to_string())));
                            }
                        }
                    });
                    return;
                } else if entity_type == "new_mr" {
                    let project = match resolve_target_project(&menu, &app.scope) {
                        Ok(p) => p,
                        Err(e) => {
                            app.show_error(e);
                            app.edit_menu = Some(menu);
                            return;
                        }
                    };
                    let title = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Title")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let source = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Source Branch")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let target = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Target Branch")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let labels = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Labels")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let assignees = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Assignees")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let reviewers = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Reviewers")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let milestone = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Milestone")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let description = menu
                        .fields
                        .iter()
                        .find(|f| {
                            (f.label == "Description" || f.label == "Release Notes")
                                && f.kind == crate::app::FieldType::Text
                        })
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();

                    let issue_iid = if menu.entity_iid > 0 {
                        Some(menu.entity_iid)
                    } else {
                        None
                    };
                    let source_on_forge = menu.source_on_forge;

                    app.edit_menu = None;
                    let Some(client) = app.gitlab_client.clone() else {
                        app.show_error("Backend not initialized".to_string());
                        return;
                    };
                    let tx = tx.clone();
                    let tab = app.active_tab;
                    fx.spawn(async move {
                        if !source.is_empty() && !source_on_forge {
                            let branch = source.clone();
                            let push_result = tokio::task::spawn_blocking(move || {
                                crate::git_helpers::ensure_source_branch_pushed(&branch)
                            })
                            .await;
                            match push_result {
                                Ok(Ok(())) => {}
                                Ok(Err(e)) => {
                                    let _ =
                                        tx.send(Event::CommandCompleted(tab, Err(e.to_string())));
                                    return;
                                }
                                Err(e) => {
                                    let _ =
                                        tx.send(Event::CommandCompleted(tab, Err(e.to_string())));
                                    return;
                                }
                            }
                        }

                        match client
                            .create_mr(
                                &project,
                                &crate::backend::NewMr {
                                    title: &title,
                                    description: &description,
                                    source_branch: &source,
                                    target_branch: &target,
                                    labels: &labels,
                                    assignees: &assignees,
                                    reviewers: &reviewers,
                                    milestone: &milestone,
                                    issue_iid,
                                    source_on_forge,
                                },
                            )
                            .await
                        {
                            Ok(_) => {
                                let _ = tx.send(Event::CommandCompleted(
                                    tab,
                                    Ok(CommandOutcome::Mutated),
                                ));
                            }
                            Err(e) => {
                                let _ = tx.send(Event::CommandCompleted(tab, Err(e.to_string())));
                            }
                        }
                    });
                    return;
                } else if entity_type == "new_bulk_edit_issues" {
                    let labels = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Labels")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let assignees = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Assignees")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let milestone = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Milestone")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();

                    let add_labels: Vec<String> = if !labels.is_empty() && labels != "--" {
                        labels
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    } else {
                        vec![]
                    };
                    let add_assignees: Vec<String> = if !assignees.is_empty() && assignees != "--" {
                        assignees
                            .split(',')
                            .map(|s| s.trim().trim_start_matches('@').to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    } else {
                        vec![]
                    };
                    let milestone_opt = if milestone.is_empty() {
                        None
                    } else if milestone.eq_ignore_ascii_case("none") || milestone == "0" {
                        Some("--".to_string())
                    } else {
                        Some(milestone.clone())
                    };
                    let update = crate::backend::IssueUpdate {
                        title: None,
                        description: None,
                        add_labels,
                        remove_labels: vec![],
                        add_assignees,
                        remove_assignees: vec![],
                        milestone: milestone_opt,
                        due_date: None,
                        weight: None,
                        confidential: None,
                    };

                    if update.is_empty() {
                        app.edit_menu = None;
                        return;
                    }

                    let selected: Vec<(String, u64)> =
                        app.selected_issues.iter().cloned().collect();
                    app.edit_menu = None;
                    app.selected_issues.clear();
                    let client = app.gitlab_client.clone().unwrap();
                    let project = app.scope.as_str().to_string();
                    let tx = tx.clone();
                    let tab = app.active_tab;
                    fx.spawn(async move {
                        for (i, (proj_path, iid)) in selected.iter().enumerate() {
                            if i > 0 {
                                crate::backend::rate_limit::pace_bulk_operation().await;
                            }
                            let p = if !proj_path.is_empty() {
                                proj_path.as_str()
                            } else {
                                &project
                            };
                            if let Err(e) = client.update_issue(p, *iid, &update).await {
                                let _ = tx.send(Event::CommandCompleted(tab, Err(e.to_string())));
                                return;
                            }
                        }
                        let _ = tx.send(Event::CommandCompleted(tab, Ok(CommandOutcome::Mutated)));
                    });
                    return;
                } else if entity_type == "new_bulk_edit_mrs" {
                    let labels = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Labels")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let assignees = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Assignees")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let milestone = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Milestone")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();

                    let add_labels: Vec<String> = if !labels.is_empty() && labels != "--" {
                        labels
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    } else {
                        vec![]
                    };
                    let add_assignees: Vec<String> = if !assignees.is_empty() && assignees != "--" {
                        assignees
                            .split(',')
                            .map(|s| s.trim().trim_start_matches('@').to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    } else {
                        vec![]
                    };
                    let milestone_opt = if milestone.is_empty() {
                        None
                    } else if milestone.eq_ignore_ascii_case("none") || milestone == "0" {
                        Some("--".to_string())
                    } else {
                        Some(milestone.clone())
                    };
                    let update = crate::backend::MrUpdate {
                        title: None,
                        description: None,
                        add_labels,
                        remove_labels: vec![],
                        add_assignees,
                        remove_assignees: vec![],
                        add_reviewers: vec![],
                        remove_reviewers: vec![],
                        milestone: milestone_opt,
                        target_branch: None,
                        draft: None,
                    };

                    if update.is_empty() {
                        app.edit_menu = None;
                        return;
                    }

                    let selected: Vec<(String, u64)> = app.selected_mrs.iter().cloned().collect();
                    app.edit_menu = None;
                    app.selected_mrs.clear();
                    let client = app.gitlab_client.clone().unwrap();
                    let project = app.scope.as_str().to_string();
                    let tx = tx.clone();
                    let tab = app.active_tab;
                    fx.spawn(async move {
                        for (i, (proj_path, iid)) in selected.iter().enumerate() {
                            if i > 0 {
                                crate::backend::rate_limit::pace_bulk_operation().await;
                            }
                            let p = if !proj_path.is_empty() {
                                proj_path.as_str()
                            } else {
                                &project
                            };
                            if let Err(e) = client.update_mr(p, *iid, &update).await {
                                let _ = tx.send(Event::CommandCompleted(tab, Err(e.to_string())));
                                return;
                            }
                        }
                        let _ = tx.send(Event::CommandCompleted(tab, Ok(CommandOutcome::Mutated)));
                    });
                    return;
                } else if entity_type == "new_milestone" {
                    let project = match resolve_target_project(&menu, &app.scope) {
                        Ok(p) => p,
                        Err(e) => {
                            app.show_error(e);
                            app.edit_menu = Some(menu);
                            return;
                        }
                    };
                    let title = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Title")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let description = menu
                        .fields
                        .iter()
                        .find(|f| {
                            (f.label == "Description" || f.label == "Release Notes")
                                && f.kind == crate::app::FieldType::Text
                        })
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let start_date = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Start Date")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let due_date = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Due Date")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();

                    app.edit_menu = None;
                    let Some(client) = app.gitlab_client.clone() else {
                        app.show_error("Backend not initialized".to_string());
                        return;
                    };
                    let tx = tx.clone();
                    let tab = app.active_tab;
                    fx.spawn(async move {
                        let sd = if start_date.is_empty() || start_date == "YYYY-MM-DD" {
                            None
                        } else {
                            Some(start_date.as_str())
                        };
                        let dd = if due_date.is_empty() || due_date == "YYYY-MM-DD" {
                            None
                        } else {
                            Some(due_date.as_str())
                        };
                        match client
                            .create_milestone(&project, &title, &description, sd, dd)
                            .await
                        {
                            Ok(_) => {
                                let _ = tx.send(Event::CommandCompleted(
                                    tab,
                                    Ok(CommandOutcome::Mutated),
                                ));
                                let _ = tx.send(Event::MilestoneUpdated);
                            }
                            Err(e) => {
                                let _ = tx.send(Event::CommandCompleted(tab, Err(e.to_string())));
                            }
                        }
                    });
                    return;
                } else if entity_type == "new_pipeline" {
                    let project = match resolve_target_project(&menu, &app.scope) {
                        Ok(p) => p,
                        Err(e) => {
                            app.show_error(e);
                            app.edit_menu = Some(menu);
                            return;
                        }
                    };
                    let branch = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Branch / Ref")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let mr = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Merge Request Pipeline")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let variables = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Variables")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let inputs = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Inputs")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let workflow = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Workflow File")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();

                    let var_pairs = parse_key_value_pairs(&variables);
                    // Collect per-input fields when workflow_dispatch inputs
                    // were detected; otherwise fall back to the generic
                    // "Inputs" field.
                    let per_input_fields: Vec<&crate::app::Field> = menu
                        .fields
                        .iter()
                        .filter(|f| f.label.starts_with("Input: "))
                        .collect();
                    let input_pairs: Vec<(String, String)> = if !per_input_fields.is_empty() {
                        per_input_fields
                            .iter()
                            .map(|f| {
                                let name = f.label.strip_prefix("Input: ").unwrap_or(&f.label);
                                (name.to_string(), f.value.trim().to_string())
                            })
                            .filter(|(_, v)| !v.is_empty())
                            .collect()
                    } else {
                        parse_key_value_pairs(&inputs)
                    };
                    let mr_flag = mr.to_lowercase() == "yes";

                    app.edit_menu = None;
                    let Some(client) = app.gitlab_client.clone() else {
                        app.show_error("Backend not initialized".to_string());
                        return;
                    };
                    let tx = tx.clone();
                    let tab = app.active_tab;
                    fx.spawn(async move {
                        match client
                            .run_pipeline(
                                &project,
                                &branch,
                                mr_flag,
                                &var_pairs,
                                &input_pairs,
                                &workflow,
                            )
                            .await
                        {
                            Ok(_) => {
                                let _ = tx.send(Event::CommandCompleted(
                                    tab,
                                    Ok(CommandOutcome::Mutated),
                                ));
                            }
                            Err(e) => {
                                let _ = tx.send(Event::CommandCompleted(tab, Err(e.to_string())));
                            }
                        }
                    });
                } else if entity_type == "new_release" {
                    let project = match resolve_target_project(&menu, &app.scope) {
                        Ok(p) => p,
                        Err(e) => {
                            app.show_error(e);
                            app.edit_menu = Some(menu);
                            return;
                        }
                    };
                    let tag = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Tag")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let name = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Release Name")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let description = menu
                        .fields
                        .iter()
                        .find(|f| {
                            f.label == "Release Notes" && f.kind == crate::app::FieldType::Text
                        })
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();

                    if !tag.is_empty() {
                        app.edit_menu = None;
                        let Some(client) = app.gitlab_client.clone() else {
                            app.show_error("Backend not initialized".to_string());
                            return;
                        };
                        let tx = tx.clone();
                        let tab = app.active_tab;
                        fx.spawn(async move {
                            match client
                                .create_release(&project, &tag, &name, &description)
                                .await
                            {
                                Ok(_) => {
                                    let _ = tx.send(Event::CommandCompleted(
                                        tab,
                                        Ok(CommandOutcome::Mutated),
                                    ));
                                }
                                Err(e) => {
                                    let _ =
                                        tx.send(Event::CommandCompleted(tab, Err(e.to_string())));
                                }
                            }
                        });
                    }
                    return;
                } else if entity_type == "new_branch" {
                    let project = match resolve_target_project(&menu, &app.scope) {
                        Ok(p) => p,
                        Err(e) => {
                            app.show_error(e);
                            app.edit_menu = Some(menu);
                            return;
                        }
                    };
                    let branch_name = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Branch Name")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let create_from = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Create From")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();

                    app.edit_menu = None;
                    let Some(client) = app.gitlab_client.clone() else {
                        app.show_error("Backend not initialized".to_string());
                        return;
                    };
                    let tx = tx.clone();
                    let tab = app.active_tab;
                    fx.spawn(async move {
                        match crate::domain::branches::create_branch(
                            &client,
                            &project,
                            &branch_name,
                            &create_from,
                        )
                        .await
                        {
                            Ok(_) => {
                                let _ = tx.send(Event::CommandCompleted(
                                    tab,
                                    Ok(CommandOutcome::Mutated),
                                ));
                            }
                            Err(e) => {
                                let _ = tx.send(Event::CommandCompleted(tab, Err(e.to_string())));
                            }
                        }
                    });
                    return;
                } else if entity_type == "release" {
                    let tag = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Tag")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let name = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Release Name")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let description = menu
                        .fields
                        .iter()
                        .find(|f| {
                            f.label == "Release Notes" && f.kind == crate::app::FieldType::Text
                        })
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();

                    let initial_tag = menu
                        .initial_fields
                        .get("Tag")
                        .map(|s| s.trim())
                        .unwrap_or_default();
                    let initial_name = menu
                        .initial_fields
                        .get("Release Name")
                        .map(|s| s.trim())
                        .unwrap_or_default();
                    let initial_desc = menu
                        .initial_fields
                        .get("Release Notes")
                        .or_else(|| menu.initial_fields.get("Description"))
                        .map(|s| s.trim())
                        .unwrap_or_default();

                    if !tag.is_empty() {
                        app.edit_menu = None;
                        if !menu.initial_fields.is_empty()
                            && tag == initial_tag
                            && name == initial_name
                            && description == initial_desc
                        {
                            return;
                        }
                        let Some(client) = app.gitlab_client.clone() else {
                            app.show_error("Backend not initialized".to_string());
                            return;
                        };
                        let project = if !menu.entity_project.is_empty() {
                            menu.entity_project.clone()
                        } else {
                            app.scope.as_str().to_string()
                        };
                        let tx = tx.clone();
                        let tab = app.active_tab;
                        let _ = tx.send(Event::CommandStarted(format!("Updating Release: {tag}")));
                        fx.spawn(async move {
                            match crate::domain::releases::update_release(
                                &client,
                                &project,
                                &tag,
                                &name,
                                &description,
                            )
                            .await
                            {
                                Ok(_) => {
                                    let _ = tx.send(Event::CommandCompleted(
                                        tab,
                                        Ok(CommandOutcome::Mutated),
                                    ));
                                    let _ = tx.send(Event::ReleaseUpdated);
                                }
                                Err(e) => {
                                    let _ =
                                        tx.send(Event::CommandCompleted(tab, Err(e.to_string())));
                                }
                            }
                        });
                    }
                    return;
                } else if crate::entity_editor::entity_is_issue(&entity_type) {
                    let title = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Title")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let description = menu
                        .fields
                        .iter()
                        .find(|f| {
                            (f.label == "Description" || f.label == "Release Notes")
                                && f.kind == crate::app::FieldType::Text
                        })
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let labels = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Labels")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let assignees = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Assignees")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let milestone = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Milestone")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let due_date = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Due Date")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let weight = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Weight")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let confidential = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Confidential")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();

                    app.edit_menu = None;
                    let new_milestone = if milestone.is_empty() || milestone == "--" {
                        None
                    } else {
                        Some(milestone.clone())
                    };
                    if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == entity_iid) {
                        if !title.is_empty() {
                            item.title = title.clone();
                        }
                        item.description = Some(description.clone());
                        item.milestone =
                            new_milestone
                                .clone()
                                .map(|t| crate::domain::issues::Milestone {
                                    title: t,
                                    iid: 0,
                                    id: 0,
                                    state: String::new(),
                                });
                    }

                    let initial_title = menu
                        .initial_fields
                        .get("Title")
                        .map(|s| s.trim())
                        .unwrap_or_default();
                    let initial_description = menu
                        .initial_fields
                        .get("Description")
                        .or_else(|| menu.initial_fields.get("Release Notes"))
                        .map(|s| s.trim())
                        .unwrap_or_default();
                    let initial_milestone = menu
                        .initial_fields
                        .get("Milestone")
                        .map(|s| s.trim())
                        .unwrap_or_default();
                    let initial_due_date = menu
                        .initial_fields
                        .get("Due Date")
                        .map(|s| s.trim())
                        .unwrap_or_default();
                    let initial_weight = menu
                        .initial_fields
                        .get("Weight")
                        .map(|s| s.trim())
                        .unwrap_or_default();
                    let initial_confidential = menu
                        .initial_fields
                        .get("Confidential")
                        .map(|s| s.trim())
                        .unwrap_or_default();

                    let initial_labels: std::collections::HashSet<String> = menu
                        .initial_fields
                        .get("Labels")
                        .map(|s| {
                            s.split(',')
                                .map(|x| x.trim().to_string())
                                .filter(|x| !x.is_empty() && x != "--")
                                .collect()
                        })
                        .unwrap_or_default();
                    let cur_labels: std::collections::HashSet<String> = labels
                        .split(',')
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty() && x != "--")
                        .collect();
                    let add_labels: Vec<String> =
                        cur_labels.difference(&initial_labels).cloned().collect();
                    let remove_labels: Vec<String> =
                        initial_labels.difference(&cur_labels).cloned().collect();

                    let initial_assignees: std::collections::HashSet<String> = menu
                        .initial_fields
                        .get("Assignees")
                        .map(|s| {
                            s.split(',')
                                .map(|x| x.trim().trim_start_matches('@').to_string())
                                .filter(|x| !x.is_empty() && x != "--")
                                .collect()
                        })
                        .unwrap_or_default();
                    let cur_assignees: std::collections::HashSet<String> = assignees
                        .split(',')
                        .map(|x| x.trim().trim_start_matches('@').to_string())
                        .filter(|x| !x.is_empty() && x != "--")
                        .collect();
                    let add_assignees: Vec<String> = cur_assignees
                        .difference(&initial_assignees)
                        .cloned()
                        .collect();
                    let remove_assignees: Vec<String> = initial_assignees
                        .difference(&cur_assignees)
                        .cloned()
                        .collect();

                    let update = crate::backend::IssueUpdate {
                        title: if !title.is_empty() && title != initial_title {
                            Some(title.clone())
                        } else {
                            None
                        },
                        description: if description != initial_description {
                            Some(description.clone())
                        } else {
                            None
                        },
                        add_labels,
                        remove_labels,
                        add_assignees,
                        remove_assignees,
                        milestone: if milestone != initial_milestone {
                            Some(milestone.clone())
                        } else {
                            None
                        },
                        due_date: if !due_date.is_empty()
                            && due_date != "YYYY-MM-DD"
                            && due_date != initial_due_date
                        {
                            Some(due_date.clone())
                        } else {
                            None
                        },
                        weight: if !weight.is_empty() && weight != initial_weight {
                            Some(weight.clone())
                        } else {
                            None
                        },
                        confidential: if !confidential.is_empty()
                            && confidential != initial_confidential
                        {
                            Some(
                                confidential.to_lowercase() == "confidential"
                                    || confidential.to_lowercase() == "yes",
                            )
                        } else {
                            None
                        },
                    };

                    app.edit_menu = None;
                    let new_milestone = if milestone.is_empty() || milestone == "--" {
                        None
                    } else {
                        Some(milestone.clone())
                    };
                    if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == entity_iid) {
                        if !title.is_empty() {
                            item.title = title.clone();
                        }
                        item.description = Some(description.clone());
                        item.milestone =
                            new_milestone
                                .clone()
                                .map(|t| crate::domain::issues::Milestone {
                                    title: t,
                                    iid: 0,
                                    id: 0,
                                    state: String::new(),
                                });
                    }

                    let client = app.gitlab_client.clone().unwrap();
                    let project = if !menu.entity_project.is_empty() {
                        menu.entity_project.clone()
                    } else {
                        app.scope.as_str().to_string()
                    };
                    let tx = tx.clone();
                    let tab = app.active_tab;
                    fx.spawn(async move {
                        if update.is_empty() {
                            let _ =
                                tx.send(Event::CommandCompleted(tab, Ok(CommandOutcome::Mutated)));
                            return;
                        }
                        let res = client.update_issue(&project, entity_iid, &update).await;
                        let _ =
                            tx.send(Event::CommandCompleted(tab, CommandOutcome::mutation(res)));
                    });
                    return;
                } else if crate::entity_editor::entity_is_mr(&entity_type) {
                    let title = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Title")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let description = menu
                        .fields
                        .iter()
                        .find(|f| {
                            (f.label == "Description" || f.label == "Release Notes")
                                && f.kind == crate::app::FieldType::Text
                        })
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let labels = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Labels")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let assignees = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Assignees")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let reviewers = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Reviewers")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let milestone = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Milestone")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let target_branch = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Target Branch")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let status_draft = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Status (Draft/Ready)" || f.label == "Status")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();

                    let initial_title = menu
                        .initial_fields
                        .get("Title")
                        .map(|s| s.trim())
                        .unwrap_or_default();
                    let initial_description = menu
                        .initial_fields
                        .get("Description")
                        .or_else(|| menu.initial_fields.get("Release Notes"))
                        .map(|s| s.trim())
                        .unwrap_or_default();
                    let initial_milestone = menu
                        .initial_fields
                        .get("Milestone")
                        .map(|s| s.trim())
                        .unwrap_or_default();
                    let initial_target_branch = menu
                        .initial_fields
                        .get("Target Branch")
                        .map(|s| s.trim())
                        .unwrap_or_default();
                    let initial_status_draft = menu
                        .initial_fields
                        .get("Status (Draft/Ready)")
                        .or_else(|| menu.initial_fields.get("Status"))
                        .map(|s| s.trim())
                        .unwrap_or_default();

                    let initial_labels: std::collections::HashSet<String> = menu
                        .initial_fields
                        .get("Labels")
                        .map(|s| {
                            s.split(',')
                                .map(|x| x.trim().to_string())
                                .filter(|x| !x.is_empty() && x != "--")
                                .collect()
                        })
                        .unwrap_or_default();
                    let cur_labels: std::collections::HashSet<String> = labels
                        .split(',')
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty() && x != "--")
                        .collect();
                    let add_labels: Vec<String> =
                        cur_labels.difference(&initial_labels).cloned().collect();
                    let remove_labels: Vec<String> =
                        initial_labels.difference(&cur_labels).cloned().collect();

                    let initial_assignees: std::collections::HashSet<String> = menu
                        .initial_fields
                        .get("Assignees")
                        .map(|s| {
                            s.split(',')
                                .map(|x| x.trim().trim_start_matches('@').to_string())
                                .filter(|x| !x.is_empty() && x != "--")
                                .collect()
                        })
                        .unwrap_or_default();
                    let cur_assignees: std::collections::HashSet<String> = assignees
                        .split(',')
                        .map(|x| x.trim().trim_start_matches('@').to_string())
                        .filter(|x| !x.is_empty() && x != "--")
                        .collect();
                    let add_assignees: Vec<String> = cur_assignees
                        .difference(&initial_assignees)
                        .cloned()
                        .collect();
                    let remove_assignees: Vec<String> = initial_assignees
                        .difference(&cur_assignees)
                        .cloned()
                        .collect();

                    let initial_reviewers: std::collections::HashSet<String> = menu
                        .initial_fields
                        .get("Reviewers")
                        .map(|s| {
                            s.split(',')
                                .map(|x| x.trim().trim_start_matches('@').to_string())
                                .filter(|x| !x.is_empty() && x != "--")
                                .collect()
                        })
                        .unwrap_or_default();
                    let cur_reviewers: std::collections::HashSet<String> = reviewers
                        .split(',')
                        .map(|x| x.trim().trim_start_matches('@').to_string())
                        .filter(|x| !x.is_empty() && x != "--")
                        .collect();
                    let add_reviewers: Vec<String> = cur_reviewers
                        .difference(&initial_reviewers)
                        .cloned()
                        .collect();
                    let remove_reviewers: Vec<String> = initial_reviewers
                        .difference(&cur_reviewers)
                        .cloned()
                        .collect();

                    let update = crate::backend::MrUpdate {
                        title: if !title.is_empty() && title != initial_title {
                            Some(title.clone())
                        } else {
                            None
                        },
                        description: if description != initial_description {
                            Some(description.clone())
                        } else {
                            None
                        },
                        add_labels,
                        remove_labels,
                        add_assignees,
                        remove_assignees,
                        add_reviewers,
                        remove_reviewers,
                        milestone: if milestone != initial_milestone {
                            Some(milestone.clone())
                        } else {
                            None
                        },
                        target_branch: if !target_branch.is_empty()
                            && target_branch != initial_target_branch
                        {
                            Some(target_branch.clone())
                        } else {
                            None
                        },
                        draft: if !status_draft.is_empty() && status_draft != initial_status_draft {
                            Some(status_draft.to_lowercase() == "draft")
                        } else {
                            None
                        },
                    };

                    app.edit_menu = None;
                    let new_milestone = if milestone.is_empty() || milestone == "--" {
                        None
                    } else {
                        Some(milestone.clone())
                    };
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == entity_iid) {
                        if !title.is_empty() {
                            item.title = title.clone();
                        }
                        item.description = Some(description.clone());
                        item.milestone = new_milestone
                            .clone()
                            .map(|t| crate::domain::mr::Milestone { title: t });
                    }

                    let client = app.gitlab_client.clone().unwrap();
                    let project = if !menu.entity_project.is_empty() {
                        menu.entity_project.clone()
                    } else {
                        app.scope.as_str().to_string()
                    };
                    let tx = tx.clone();
                    let tab = app.active_tab;
                    fx.spawn(async move {
                        if update.is_empty() {
                            let _ =
                                tx.send(Event::CommandCompleted(tab, Ok(CommandOutcome::Mutated)));
                            return;
                        }
                        let res = client.update_mr(&project, entity_iid, &update).await;
                        let _ =
                            tx.send(Event::CommandCompleted(tab, CommandOutcome::mutation(res)));
                    });
                    return;
                } else if crate::entity_editor::entity_is_milestone(&entity_type) {
                    let title = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Title")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let description = menu
                        .fields
                        .iter()
                        .find(|f| {
                            (f.label == "Description" || f.label == "Release Notes")
                                && f.kind == crate::app::FieldType::Text
                        })
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let start_date = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Start Date")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();
                    let due_date = menu
                        .fields
                        .iter()
                        .find(|f| f.label == "Due Date")
                        .map(|f| f.value.trim().to_string())
                        .unwrap_or_default();

                    let initial_title = menu
                        .initial_fields
                        .get("Title")
                        .map(|s| s.trim())
                        .unwrap_or_default();
                    let initial_desc = menu
                        .initial_fields
                        .get("Description")
                        .or_else(|| menu.initial_fields.get("Release Notes"))
                        .map(|s| s.trim())
                        .unwrap_or_default();
                    let initial_sd = menu
                        .initial_fields
                        .get("Start Date")
                        .map(|s| s.trim())
                        .unwrap_or_default();
                    let initial_dd = menu
                        .initial_fields
                        .get("Due Date")
                        .map(|s| s.trim())
                        .unwrap_or_default();

                    app.edit_menu = None;
                    if !menu.initial_fields.is_empty()
                        && title == initial_title
                        && description == initial_desc
                        && start_date == initial_sd
                        && due_date == initial_dd
                    {
                        return;
                    }

                    let client = app.gitlab_client.clone().unwrap();
                    let project = if !menu.entity_project.is_empty() {
                        menu.entity_project.clone()
                    } else {
                        app.scope.as_str().to_string()
                    };
                    let tx = tx.clone();
                    let tab = app.active_tab;
                    fx.spawn(async move {
                        let sd = if start_date.is_empty()
                            || start_date == "YYYY-MM-DD"
                            || start_date == "Set"
                        {
                            None
                        } else {
                            Some(start_date.as_str())
                        };
                        let dd =
                            if due_date.is_empty() || due_date == "YYYY-MM-DD" || due_date == "Set"
                            {
                                None
                            } else {
                                Some(due_date.as_str())
                            };
                        match client
                            .update_milestone(&project, entity_iid, &title, &description, sd, dd)
                            .await
                        {
                            Ok(_) => {
                                let _ = tx.send(Event::CommandCompleted(
                                    tab,
                                    Ok(CommandOutcome::Mutated),
                                ));
                                let _ = tx.send(Event::MilestoneUpdated);
                            }
                            Err(e) => {
                                let _ = tx.send(Event::CommandCompleted(tab, Err(e.to_string())));
                            }
                        }
                    });
                    return;
                }
            }

            // Not on submit — act on the currently selected field.
            // Non-editable fields (ReadOnly / Section) and the
            // spacer row are no-ops: pressing Enter on them must
            // never crash and must never open an editor.
            if menu.selected_idx < menu.fields.len() {
                let f = &menu.fields[menu.selected_idx];
                if f.kind == crate::app::FieldType::ReadOnly
                    || f.kind == crate::app::FieldType::Section
                {
                    app.edit_menu = Some(menu);
                    return;
                }
            } else if menu.selected_idx != menu.fields.len() + 1 {
                // Spacer row between fields and submit.
                app.edit_menu = Some(menu);
                return;
            }

            let field_name = if menu.selected_idx < menu.fields.len() {
                menu.fields[menu.selected_idx].label.clone()
            } else {
                String::new()
            };

            // Text fields: Enter toggles inline edit mode. Ref rows open a selector instead.
            if menu.selected_idx < menu.fields.len()
                && menu.fields[menu.selected_idx].kind == crate::app::FieldType::Text
            {
                if menu.editing {
                    menu.editing = false;
                } else {
                    menu.editing = true;
                    if let Some(f) = menu.fields.get(menu.selected_idx) {
                        menu.cursor_pos = f.value.len();
                    }
                }
                app.edit_menu = Some(menu);
                return;
            }

            if field_name == "Labels"
                || field_name == "Assignees"
                || field_name == "Reviewers"
                || field_name == "Milestone"
                || field_name == "Confidential"
                || field_name == "Status (Draft/Ready)"
                || field_name == "Merge Request Pipeline"
                || field_name == "Source Branch"
                || field_name == "Target Branch"
                || field_name == "Branch / Ref"
                || field_name == "Create From"
                || field_name == "Workflow File"
                || field_name == "Tag"
                || field_name == "Create from Issue"
                || field_name == "Description Template"
                || (field_name == "Project"
                    && menu.selected_idx < menu.fields.len()
                    && menu.fields[menu.selected_idx].kind == crate::app::FieldType::Ref)
                || field_name.starts_with("Input: ")
            {
                let mut current_set = std::collections::HashSet::new();
                let field_type = match field_name.as_str() {
                    "Labels" => "labels",
                    "Assignees" => "assignees",
                    "Reviewers" => "reviewers",
                    "Milestone" => "milestone",
                    "Confidential" => "confidential",
                    "Status (Draft/Ready)" => "draft_status",
                    "Merge Request Pipeline" => "mr_pipeline",
                    "Source Branch" => "source_branch",
                    "Target Branch" => "target_branch",
                    "Branch / Ref" => "pipeline_branch",
                    "Create From" => "create_from",
                    "Create from Issue" => "create_mr_from_issue",
                    "Description Template" => "description_template",
                    "Workflow File" => "workflow_file",
                    "Tag" => "tag",
                    "Project" => "project",
                    _ => "",
                };
                let multi_select = matches!(field_type, "labels" | "assignees" | "reviewers");

                let target_project = menu
                    .fields
                    .iter()
                    .find(|f| f.label == "Project")
                    .map(|f| f.value.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| {
                        if !menu.entity_project.is_empty() {
                            menu.entity_project.clone()
                        } else {
                            String::new()
                        }
                    });
                let target_cache = if !target_project.is_empty() && target_project.contains('/') {
                    Some(crate::utils::cache::load_cache(&target_project))
                } else {
                    None
                };

                let mut all_items = Vec::new();
                let mut is_loading = true;

                if field_type == "confidential" {
                    all_items = vec!["Public".to_string(), "Confidential".to_string()];
                    is_loading = false;
                } else if field_type == "draft_status" {
                    all_items = vec!["Draft".to_string(), "Ready".to_string()];
                    is_loading = false;
                    let is_new_entity = entity_iid == 0 || entity_type.starts_with("new_");
                    if is_new_entity {
                        let current_val = menu.fields[menu.selected_idx].value.clone();
                        if !current_val.is_empty() {
                            current_set.insert(current_val);
                        } else {
                            current_set.insert("Ready".to_string());
                        }
                    } else if let Some(mr) = app.mrs.items.iter().find(|m| m.iid == entity_iid) {
                        current_set.insert(if mr.draft {
                            "Draft".to_string()
                        } else {
                            "Ready".to_string()
                        });
                    }
                } else if field_type == "mr_pipeline" {
                    all_items = vec!["Yes".to_string(), "No".to_string()];
                    is_loading = false;
                    let is_new_entity = entity_iid == 0 || entity_type.starts_with("new_");
                    if is_new_entity {
                        let current_val = menu.fields[menu.selected_idx].value.clone();
                        if !current_val.is_empty() {
                            current_set.insert(current_val);
                        } else {
                            current_set.insert("No".to_string());
                        }
                    }
                } else if field_type == "labels" {
                    if let Some(ref tc) = target_cache {
                        if !tc.labels.is_empty() {
                            all_items = tc.labels.clone();
                            is_loading = false;
                        }
                    } else if !app.cached_labels.is_empty() {
                        all_items = app.cached_labels.clone();
                        is_loading = false;
                    }
                } else if field_type == "assignees" || field_type == "reviewers" {
                    if let Some(ref tc) = target_cache {
                        if !tc.members.is_empty() {
                            all_items = tc.members.clone();
                            is_loading = false;
                        }
                    } else if !app.cached_members.is_empty() {
                        all_items = app.cached_members.clone();
                        is_loading = false;
                    }
                } else if field_type == "milestone" {
                    if let Some(ref tc) = target_cache {
                        if !tc.milestones.is_empty() {
                            all_items = tc
                                .milestones
                                .iter()
                                .map(|m| m.title.clone())
                                .filter(|t| t != "--")
                                .collect();
                            is_loading = false;
                        }
                    }
                    if all_items.is_empty() {
                        all_items = app
                            .milestones
                            .items
                            .iter()
                            .filter(|m| {
                                target_project.is_empty()
                                    || m.project_path == target_project
                                    || m.project_path.is_empty()
                            })
                            .map(|m| m.title.clone())
                            .filter(|t| t != "--")
                            .collect();
                        if !all_items.is_empty() && !target_project.contains('/') {
                            is_loading = false;
                        }
                    }
                } else if field_type == "source_branch"
                    || field_type == "target_branch"
                    || field_type == "pipeline_branch"
                    || field_type == "create_from"
                {
                    let mut branch_names: Vec<String> = if let Some(ref tc) = target_cache {
                        tc.branches.iter().map(|b| b.name.clone()).collect()
                    } else {
                        app.branches
                            .items()
                            .iter()
                            .map(|b| b.name.clone())
                            .collect()
                    };
                    if branch_names.is_empty() && !app.scope.is_group() && target_project.is_empty()
                    {
                        if let Ok(output) = std::process::Command::new("git")
                            .args(["branch", "-a", "--format=%(refname:short)"])
                            .output()
                        {
                            if output.status.success() {
                                for line in String::from_utf8_lossy(&output.stdout).lines() {
                                    let b = line.trim();
                                    let b = b.strip_prefix("origin/").unwrap_or(b);
                                    if !b.is_empty()
                                        && b != "HEAD"
                                        && !b.starts_with("origin/")
                                        && !branch_names.contains(&b.to_string())
                                    {
                                        branch_names.push(b.to_string());
                                    }
                                }
                            }
                        }
                    }
                    if field_type == "pipeline_branch"
                        || field_type == "target_branch"
                        || field_type == "source_branch"
                        || field_type == "create_from"
                    {
                        let current_val = menu.fields[menu.selected_idx].value.clone();
                        if !current_val.is_empty() {
                            current_set.insert(current_val);
                        }
                    }
                    if !branch_names.is_empty() {
                        all_items = branch_names;
                        if !target_project.contains('/') {
                            is_loading = false;
                        }
                    }
                } else if field_type == "workflow_file" {
                    all_items = get_workflow_files(app.is_github());
                    is_loading = false;
                    // Pre-select any already-typed value
                    let current_val = menu.fields[menu.selected_idx].value.clone();
                    if !current_val.is_empty() {
                        current_set.insert(current_val);
                    }
                } else if field_name.starts_with("Input: ") {
                    let input_name = field_name.strip_prefix("Input: ").unwrap_or("");
                    if let Some(input) = menu.workflow_inputs.iter().find(|i| i.name == input_name)
                    {
                        use crate::domain::workflow_inputs::WorkflowInputType;
                        match input.input_type {
                            WorkflowInputType::Choice => {
                                all_items = input.options.clone();
                            }
                            WorkflowInputType::Boolean => {
                                all_items = vec!["true".to_string(), "false".to_string()];
                            }
                            _ => {}
                        }
                        is_loading = false;
                    }
                    let current_val = menu.fields[menu.selected_idx].value.clone();
                    if !current_val.is_empty() {
                        current_set.insert(current_val);
                    }
                } else if field_type == "tag" {
                    // Collect existing tags from releases cache + git tag
                    let mut tags: Vec<String> = app
                        .releases
                        .items()
                        .iter()
                        .map(|r| r.tag_name.clone())
                        .collect();
                    if let Ok(output) =
                        // Safe on the UI thread: pure local ref read.
                        std::process::Command::new("git").args(["tag"]).output()
                    {
                        if output.status.success() {
                            for line in String::from_utf8_lossy(&output.stdout).lines() {
                                let t = line.trim().to_string();
                                if !t.is_empty() && !tags.contains(&t) {
                                    tags.push(t);
                                }
                            }
                        }
                    }
                    tags.sort();
                    all_items = tags;
                    is_loading = false;
                    let current_val = menu.fields[menu.selected_idx].value.clone();
                    if !current_val.is_empty() {
                        current_set.insert(current_val);
                    }
                } else if field_type == "create_mr_from_issue" {
                    let issues: Vec<String> = app
                        .issues
                        .items
                        .iter()
                        .filter(|i| i.state == "opened" || i.state == "open")
                        .map(|i| format!("#{} {}", i.iid, i.title))
                        .collect();
                    all_items = issues;
                    is_loading = false;
                } else if field_type == "description_template" {
                    let template_type = if entity_type == "new_mr" {
                        "mr"
                    } else {
                        "issue"
                    };
                    let templates = list_templates(template_type);
                    let template_names: Vec<String> = std::iter::once("-- (blank)".to_string())
                        .chain(templates.iter().map(|(n, _)| n.clone()))
                        .collect();
                    all_items = template_names;
                    is_loading = false;
                } else if field_type == "project" {
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
                    all_items = known;
                    if !app.scope.is_group() {
                        is_loading = false;
                    }
                    let current_val = menu.fields[menu.selected_idx].value.clone();
                    if !current_val.is_empty() {
                        current_set.insert(current_val);
                    }
                }

                if entity_iid == 0 || entity_type.starts_with("new_") {
                    let current_val = menu.fields[menu.selected_idx].value.clone();
                    if !current_val.is_empty()
                        && field_type != "draft_status"
                        && field_type != "mr_pipeline"
                        && field_type != "project"
                    {
                        if multi_select {
                            for item in current_val.split(',') {
                                let trimmed = item.trim().to_string();
                                if !trimmed.is_empty() {
                                    current_set.insert(trimmed);
                                }
                            }
                        } else {
                            current_set.insert(current_val);
                        }
                    }
                } else if entity_type == "issue" {
                    if let Some(issue) = app.issues.items.iter().find(|i| i.iid == entity_iid) {
                        match field_type {
                            "labels" => {
                                for l in &issue.labels {
                                    current_set.insert(l.clone());
                                }
                            }
                            "assignees" => {
                                for a in &issue.assignees {
                                    current_set.insert(format!("@{}", a.username));
                                }
                            }
                            "milestone" => {
                                if let Some(m) = &issue.milestone {
                                    current_set.insert(m.title.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                } else if entity_type == "mr" {
                    if let Some(mr) = app.mrs.items.iter().find(|m| m.iid == entity_iid) {
                        match field_type {
                            "labels" => {
                                for l in &mr.labels {
                                    current_set.insert(l.clone());
                                }
                            }
                            "assignees" => {
                                for a in &mr.assignees {
                                    current_set.insert(format!("@{}", a.username));
                                }
                            }
                            "reviewers" => {
                                for r in &mr.reviewers {
                                    current_set.insert(format!("@{}", r.username));
                                }
                            }
                            "milestone" => {
                                if let Some(m) = &mr.milestone {
                                    current_set.insert(m.title.clone());
                                }
                            }
                            "target_branch" => {
                                current_set.insert(mr.target_branch.clone());
                            }
                            _ => {}
                        }
                    }
                }

                let start_idx = if multi_select {
                    0
                } else {
                    current_set
                        .iter()
                        .next()
                        .and_then(|sel| all_items.iter().position(|a| a == sel))
                        .unwrap_or(0)
                };

                app.selector = Some(crate::app::Selector {
                    title: format!("Select {field_name}"),
                    all_items,
                    selected_items: current_set,
                    cursor_idx: start_idx,
                    search_query: String::new(),
                    is_filtering: false,
                    is_loading,
                    entity_iid,
                    entity_type: entity_type.clone(),
                    field_type: if field_name.starts_with("Input: ") {
                        field_name.clone()
                    } else {
                        field_type.to_string()
                    },
                    multi_select,
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                });

                app.edit_menu = Some(menu);

                if is_loading {
                    if let Some(client) = &app.gitlab_client {
                        let client = client.clone();
                        let scope = if field_type != "project"
                            && !target_project.is_empty()
                            && target_project.contains('/')
                        {
                            crate::scope::Scope::Repository(target_project)
                        } else {
                            app.scope.clone()
                        };
                        let field_type = field_type.to_string();
                        let tx = app.scoped_sender(tx.clone());
                        fx.spawn(async move {
                            let res = match field_type.as_str() {
                                "labels" => client
                                    .fetch_labels(&scope)
                                    .await
                                    .map(|labels| labels.into_iter().map(|l| l.name).collect()),
                                "assignees" | "reviewers" => client.fetch_members(&scope).await,
                                "milestone" => client.fetch_milestones(&scope).await,
                                "source_branch" | "target_branch" | "pipeline_branch"
                                | "create_from" => client.fetch_branches(&scope).await,
                                "project" => client.fetch_projects(&scope).await,
                                _ => Ok(Vec::new()),
                            };
                            if let Ok(items) = res {
                                let _ = tx.send(Event::SelectorItemsFetched(items));
                            } else {
                                let _ = tx.send(Event::SelectorItemsFetched(Vec::new()));
                            }
                        });
                    }
                }
                return;
            }

            if field_name == "Description" || field_name == "Release Notes" {
                // For new entities with empty description, show template selector
                if entity_iid == 0 || entity_type.starts_with("new_") {
                    let raw_val = menu.fields[menu.selected_idx].value.clone();
                    if raw_val.trim().is_empty() {
                        let template_type = if entity_type == "new_mr" {
                            "mr"
                        } else {
                            "issue"
                        };
                        let templates = list_templates(template_type);
                        if !templates.is_empty() {
                            let template_names: Vec<String> =
                                std::iter::once("-- (blank)".to_string())
                                    .chain(templates.iter().map(|(n, _)| n.clone()))
                                    .collect();
                            let field_type = if entity_type == "new_mr" {
                                "mr_template_selector"
                            } else {
                                "issue_template_selector"
                            };
                            app.selector = Some(crate::app::Selector {
                                title: format!(
                                    " Select {} Template ",
                                    if template_type == "mr" {
                                        "Merge Request"
                                    } else {
                                        "Issue"
                                    }
                                ),
                                all_items: template_names,
                                selected_items: std::collections::HashSet::new(),
                                cursor_idx: 0,
                                search_query: String::new(),
                                is_filtering: false,
                                is_loading: false,
                                entity_iid: 0,
                                entity_type: entity_type.clone(),
                                field_type: if field_name.starts_with("Input: ") {
                                    field_name.clone()
                                } else {
                                    field_type.to_string()
                                },
                                multi_select: false,
                                state: {
                                    let mut s = ListState::default();
                                    s.select(Some(0));
                                    s
                                },
                            });
                            app.edit_menu = Some(menu);
                            return;
                        }
                    }
                }
                // Otherwise inline editing handles Description — Enter does nothing
                app.edit_menu = Some(menu);
                return;
            }

            if field_name == "Due Date" || field_name == "Start Date" {
                let current_val = if entity_iid == 0 || entity_type.starts_with("new_") {
                    menu.fields[menu.selected_idx].value.clone()
                } else if entity_type == "issue" {
                    app.issues
                        .items
                        .iter()
                        .find(|i| i.iid == entity_iid)
                        .and_then(|i| i.due_date.clone())
                        .unwrap_or_default()
                } else if entity_type == "milestone" {
                    let m = app.milestones.items.iter().find(|m| m.iid == entity_iid);
                    if field_name == "Start Date" {
                        m.and_then(|m| m.start_date.clone()).unwrap_or_default()
                    } else {
                        m.and_then(|m| m.due_date.clone()).unwrap_or_default()
                    }
                } else {
                    String::new()
                };
                let action = if entity_iid != 0 && !entity_type.starts_with("new_") {
                    let ft = match field_name.as_str() {
                        "Due Date" => "due_date",
                        "Start Date" => "start_date",
                        _ => "",
                    };
                    crate::app::DatePickerAction::EditField {
                        entity_iid,
                        entity_type: entity_type.clone(),
                        field_type: ft.to_string(),
                    }
                } else {
                    crate::app::DatePickerAction::EditNewField {
                        field_idx: menu.selected_idx,
                    }
                };
                app.date_picker = Some(crate::app::DatePicker::new(
                    format!(" Select {field_name}"),
                    &current_val,
                    action,
                ));
                app.edit_menu = Some(menu);
                return;
            }

            if field_name == "Weight"
                || field_name == "Variables"
                || field_name == "Inputs"
                || field_name == "Release Name"
            {
                let current_val = if entity_iid == 0 || entity_type.starts_with("new_") {
                    menu.fields[menu.selected_idx].value.clone()
                } else {
                    let field_type = match field_name.as_str() {
                        "Title" => "title",
                        "Target Branch" => "target_branch",
                        "Weight" => "weight",
                        _ => "",
                    };
                    match field_type {
                        "title" => {
                            if entity_type == "issue" {
                                app.issues
                                    .items
                                    .iter()
                                    .find(|i| i.iid == entity_iid)
                                    .map(|i| i.title.clone())
                                    .unwrap_or_default()
                            } else if entity_type == "milestone" {
                                app.milestones
                                    .items
                                    .iter()
                                    .find(|m| m.iid == entity_iid)
                                    .map(|m| m.title.clone())
                                    .unwrap_or_default()
                            } else {
                                app.mrs
                                    .items
                                    .iter()
                                    .find(|m| m.iid == entity_iid)
                                    .map(|m| m.title.clone())
                                    .unwrap_or_default()
                            }
                        }
                        "target_branch" => app
                            .mrs
                            .items
                            .iter()
                            .find(|m| m.iid == entity_iid)
                            .map(|m| m.target_branch.clone())
                            .unwrap_or_default(),
                        "weight" => "0".to_string(),
                        _ => String::new(),
                    }
                };

                let action = crate::app::TextInputAction::EditNewField {
                    field_idx: menu.selected_idx,
                };

                app.text_input = Some(crate::app::TextInput::new(
                    format!("Edit {field_name}"),
                    current_val,
                    action,
                ));

                app.edit_menu = Some(menu);
            }
        }
        KeyCode::Backspace => {
            if menu.editing {
                let field_name = if menu.selected_idx < menu.fields.len() {
                    menu.fields[menu.selected_idx].label.clone()
                } else {
                    String::new()
                };
                if field_name == "Title"
                    || field_name == "Name"
                    || field_name == "Release Name"
                    || field_name == "Branch Name"
                    || field_name == "Description"
                    || field_name == "Release Notes"
                {
                    if let Some(f) = menu.fields.get_mut(menu.selected_idx) {
                        if let Some((byte_idx, _)) =
                            f.value[..menu.cursor_pos].char_indices().next_back()
                        {
                            f.value.remove(byte_idx);
                            menu.cursor_pos = byte_idx;
                        }
                    }
                }
            }
            app.edit_menu = Some(menu);
        }
        KeyCode::Char(c) if key_event.modifiers.contains(KeyModifiers::CONTROL) && c == 'e' => {
            let field_name = if menu.selected_idx < menu.fields.len() {
                menu.fields[menu.selected_idx].label.clone()
            } else {
                String::new()
            };
            if field_name == "Description" || field_name == "Release Notes" {
                let desc = if let Some(f) = menu.fields.iter().find(|f| {
                    (f.label == "Description" || f.label == "Release Notes")
                        && f.kind == crate::app::FieldType::Text
                }) {
                    f.value.clone()
                } else {
                    String::new()
                };
                app.edit_menu = Some(menu);
                fx.suspend(ForegroundJob::Edit {
                    text: desc,
                    file_suffix: ".md".to_string(),
                    purpose: EditPurpose::MenuDescription,
                });
            } else {
                app.edit_menu = Some(menu);
            }
        }
        KeyCode::Char(c) => {
            if menu.editing {
                let field_name = if menu.selected_idx < menu.fields.len() {
                    menu.fields[menu.selected_idx].label.clone()
                } else {
                    String::new()
                };
                if field_name == "Title"
                    || field_name == "Name"
                    || field_name == "Release Name"
                    || field_name == "Branch Name"
                    || field_name == "Description"
                    || field_name == "Release Notes"
                {
                    if let Some(f) = menu.fields.get_mut(menu.selected_idx) {
                        f.value.insert(menu.cursor_pos, c);
                        menu.cursor_pos += c.len_utf8();
                    }
                }
            }
            app.edit_menu = Some(menu);
        }
        _ => {
            app.edit_menu = Some(menu);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_value_pairs() {
        let input = "key1:val1,key2:val2, replicas:int(3), debug:bool(false) ";
        let pairs = parse_key_value_pairs(input);
        assert_eq!(
            pairs,
            vec![
                ("key1".to_string(), "val1".to_string()),
                ("key2".to_string(), "val2".to_string()),
                ("replicas".to_string(), "int(3)".to_string()),
                ("debug".to_string(), "bool(false)".to_string())
            ]
        );
    }
}
