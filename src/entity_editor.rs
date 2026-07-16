use crate::AppTerminal;
use crate::app::App;
use crate::editor::edit_in_editor;
use crate::event::Event;
use crossterm::event::KeyCode;

pub fn apply_field_text_change(
    app: &mut App,
    entity_type: &str,
    iid: u64,
    field_type: &str,
    value: String,
    terminal: &mut AppTerminal,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    tab: crate::app::Tab,
) {
    if entity_type == "milestone" {
        if let Some(item) = app.milestones.items.iter_mut().find(|m| m.iid == iid) {
            match field_type {
                "title" => item.title = value.clone(),
                "start_date" => item.start_date = Some(value.clone()),
                "due_date" => item.due_date = Some(value.clone()),
                "description" => item.description = Some(value.clone()),
                _ => {}
            }
        }
        let milestone_opt = app.milestones.items.iter().find(|m| m.iid == iid).cloned();
        if let Some(milestone) = milestone_opt {
            let mut title = milestone.title.clone();
            let mut start_date = milestone.start_date.clone();
            let mut due_date = milestone.due_date.clone();
            let mut description = milestone.description.clone().unwrap_or_default();

            match field_type {
                "title" => title = value.clone(),
                "start_date" => start_date = Some(value.clone()),
                "due_date" => due_date = Some(value.clone()),
                "description" => description = value.clone(),
                _ => {}
            }

            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
            let tx_spawn = tx.clone();
            tokio::spawn(async move {
                let res = crate::domain::milestones::update_milestone(
                    &client,
                    &project_path,
                    iid,
                    &title,
                    &description,
                    start_date.as_deref(),
                    due_date.as_deref(),
                )
                .await;
                match res {
                    Ok(_) => {
                        let _ = tx_spawn.send(Event::MilestoneUpdated);
                    }
                    Err(e) => {
                        let _ = tx_spawn.send(Event::CommandCompleted(
                            crate::app::Tab::Milestones,
                            Err(e.to_string()),
                        ));
                    }
                }
            });
        }
        return;
    }

    if entity_type == "release" {
        let release_opt = app.releases.items.get(iid as usize).cloned();
        if let Some(release) = release_opt {
            let mut name = release.name.clone();
            let mut tag = release.tag_name.clone();
            let mut description = release.description.clone().unwrap_or_default();

            match field_type {
                "title" | "release_name" => name = value.clone(),
                "tag" => tag = value.clone(),
                "description" => description = value.clone(),
                _ => {}
            }

            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
            let tx_spawn = tx.clone();
            tokio::spawn(async move {
                let res = crate::domain::releases::update_release(
                    &client,
                    &project_path,
                    &tag,
                    &name,
                    &description,
                )
                .await;
                match res {
                    Ok(_) => {
                        let _ = tx_spawn.send(Event::ReleaseUpdated);
                    }
                    Err(e) => {
                        let _ = tx_spawn.send(Event::CommandCompleted(
                            crate::app::Tab::Releases,
                            Err(e.to_string()),
                        ));
                    }
                }
            });
        }
        return;
    }

    match field_type {
        "title" => {
            if entity_type == "issue" {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.title = value.clone();
                }
            } else if entity_type == "mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.title = value.clone();
                }
            }
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
            let et = entity_type.to_string();
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = if et == "issue" {
                    client.update_issue_title(&project_path, iid, &value).await
                } else {
                    client.update_mr_title(&project_path, iid, &value).await
                };
                let _ = tx2.send(Event::CommandCompleted(
                    tab,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        "target_branch" => {
            if entity_type == "mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.target_branch = value.clone();
                }
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .update_mr_target_branch(&project_path, iid, &value)
                        .await;
                    let _ = tx2.send(Event::CommandCompleted(
                        tab,
                        result.map_err(|e| e.to_string()),
                    ));
                });
            }
        }
        "due_date" => {
            if entity_type == "issue" {
                let flag_value = if value == "YYYY-MM-DD" || value.trim().is_empty() {
                    String::new()
                } else {
                    value.clone()
                };
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.due_date = if flag_value.is_empty() {
                        None
                    } else {
                        Some(flag_value.clone())
                    };
                }
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .update_issue_due_date(&project_path, iid, &flag_value)
                        .await;
                    let _ = tx2.send(Event::CommandCompleted(
                        tab,
                        result.map_err(|e| e.to_string()),
                    ));
                });
            }
        }
        "weight" => {
            if entity_type == "issue" {
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let result = client.update_issue_weight(&project_path, iid, &value).await;
                    let _ = tx2.send(Event::CommandCompleted(
                        tab,
                        result.map_err(|e| e.to_string()),
                    ));
                });
            }
        }
        "runner_description" => {
            if let Some(runner) = app.runners.items.iter_mut().find(|r| r.id == iid) {
                runner.description = Some(value.clone());
            }
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = client
                    .backend
                    .update_runner_description(&project_path, iid, &value)
                    .await;
                let _ = tx2.send(Event::CommandCompleted(
                    tab,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        "description" => {
            if entity_type == "issue" {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.description = Some(value.clone());
                }
            } else if entity_type == "mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.description = Some(value.clone());
                }
            }
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
            let et = entity_type.to_string();
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = if et == "issue" {
                    client
                        .update_issue_description(&project_path, iid, &value)
                        .await
                } else {
                    client
                        .update_mr_description(&project_path, iid, &value)
                        .await
                };
                let _ = tx2.send(Event::CommandCompleted(
                    tab,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        _ => {}
    }
}

pub fn apply_selector_changes(
    app: &mut App,
    entity_type: &str,
    iid: u64,
    field_type: &str,
    values: Vec<String>,
    terminal: &mut AppTerminal,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    tab: crate::app::Tab,
) {
    match field_type {
        "labels" => {
            let current_labels: Vec<String> = if entity_type == "issue" {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .map(|i| i.labels.clone())
                    .unwrap_or_default()
            } else {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.labels.clone())
                    .unwrap_or_default()
            };

            // Determine which labels to add and which to remove
            let value_set: std::collections::HashSet<String> = values.iter().cloned().collect();
            let current_set: std::collections::HashSet<String> =
                current_labels.iter().cloned().collect();

            let to_add: Vec<&String> = value_set.difference(&current_set).collect();
            let to_remove: Vec<&String> = current_set.difference(&value_set).collect();

            if !to_add.is_empty() || !to_remove.is_empty() {
                let to_add: Vec<String> = to_add.iter().map(|s| (*s).clone()).collect();
                let to_remove: Vec<String> = to_remove.iter().map(|s| (*s).clone()).collect();
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let et = entity_type.to_string();
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let result = if et == "issue" {
                        client
                            .update_issue_labels(&project_path, iid, &to_add, &to_remove)
                            .await
                    } else {
                        client
                            .update_mr_labels(&project_path, iid, &to_add, &to_remove)
                            .await
                    };
                    let _ = tx2.send(Event::CommandCompleted(
                        tab,
                        result.map_err(|e| e.to_string()),
                    ));
                });
            }

            if entity_type == "issue" {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.labels = values;
                }
            } else if entity_type == "mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.labels = values;
                }
            }
        }
        "assignees" => {
            let clean_values: Vec<String> = values
                .iter()
                .map(|v| v.trim_start_matches('@').to_string())
                .collect();
            let current_assignees: Vec<String> = if entity_type == "issue" {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .map(|i| i.assignees.iter().map(|a| a.username.clone()).collect())
                    .unwrap_or_default()
            } else {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.assignees.iter().map(|a| a.username.clone()).collect())
                    .unwrap_or_default()
            };

            let value_set: std::collections::HashSet<String> =
                clean_values.iter().cloned().collect();
            let current_set: std::collections::HashSet<String> =
                current_assignees.iter().cloned().collect();

            let to_add: Vec<&String> = value_set.difference(&current_set).collect();
            let to_remove: Vec<&String> = current_set.difference(&value_set).collect();

            if !to_add.is_empty() || !to_remove.is_empty() {
                let to_add: Vec<String> = to_add.iter().map(|s| (*s).clone()).collect();
                let to_remove: Vec<String> = to_remove.iter().map(|s| (*s).clone()).collect();
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let et = entity_type.to_string();
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let result = if et == "issue" {
                        client
                            .update_issue_assignees(&project_path, iid, &to_add, &to_remove)
                            .await
                    } else {
                        client
                            .update_mr_assignees(&project_path, iid, &to_add, &to_remove)
                            .await
                    };
                    let _ = tx2.send(Event::CommandCompleted(
                        tab,
                        result.map_err(|e| e.to_string()),
                    ));
                });
            }

            if entity_type == "issue" {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.assignees = clean_values
                        .iter()
                        .map(|username| crate::domain::issues::Assignee {
                            username: username.clone(),
                        })
                        .collect();
                }
            } else if entity_type == "mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.assignees = clean_values
                        .iter()
                        .map(|username| crate::domain::mr::Assignee {
                            username: username.clone(),
                        })
                        .collect();
                }
            }
        }
        "reviewers" => {
            if entity_type == "mr" {
                let clean_values: Vec<String> = values
                    .iter()
                    .map(|v| v.trim_start_matches('@').to_string())
                    .collect();
                let current_reviewers: Vec<String> = app
                    .mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.reviewers.iter().map(|r| r.username.clone()).collect())
                    .unwrap_or_default();

                let value_set: std::collections::HashSet<String> =
                    clean_values.iter().cloned().collect();
                let current_set: std::collections::HashSet<String> =
                    current_reviewers.iter().cloned().collect();

                let to_add: Vec<&String> = value_set.difference(&current_set).collect();
                let to_remove: Vec<&String> = current_set.difference(&value_set).collect();

                if !to_add.is_empty() || !to_remove.is_empty() {
                    let to_add: Vec<String> = to_add.iter().map(|s| (*s).clone()).collect();
                    let to_remove: Vec<String> = to_remove.iter().map(|s| (*s).clone()).collect();
                    let Some(client) = app.gitlab_client.clone() else {
                        return;
                    };
                    let project_path = app.project_context.clone();
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        let result = client
                            .update_mr_reviewers(&project_path, iid, &to_add, &to_remove)
                            .await;
                        let _ = tx2.send(Event::CommandCompleted(
                            tab,
                            result.map_err(|e| e.to_string()),
                        ));
                    });
                }

                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.reviewers = clean_values
                        .iter()
                        .map(|username| crate::domain::mr::Reviewer {
                            username: username.clone(),
                        })
                        .collect();
                }
            }
        }
        "milestone" => {
            let first_val = values.first().cloned().unwrap_or_default();
            if entity_type == "issue" {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    let m = crate::domain::issues::Milestone {
                        title: first_val.clone(),
                    };
                    item.milestone = Some(m);
                }
            } else if entity_type == "mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    let m = crate::domain::mr::Milestone {
                        title: first_val.clone(),
                    };
                    item.milestone = Some(m);
                }
            }
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
            let et = entity_type.to_string();
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = if et == "issue" {
                    client
                        .update_issue_milestone(&project_path, iid, &first_val)
                        .await
                } else {
                    client
                        .update_mr_milestone(&project_path, iid, &first_val)
                        .await
                };
                let _ = tx2.send(Event::CommandCompleted(
                    tab,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        "confidential" => {
            if entity_type == "issue" {
                let is_confidential = values.iter().any(|v| v == "Yes" || v == "true");
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .update_issue_confidential(&project_path, iid, is_confidential)
                        .await;
                    let _ = tx2.send(Event::CommandCompleted(
                        tab,
                        result.map_err(|e| e.to_string()),
                    ));
                });
            }
        }
        _ => {}
    }
}

pub fn rebuild_edit_menu(app: &mut App, entity_type: &str, entity_iid: u64) {
    if entity_type == "issue" {
        if let Some(issue) = app.issues.items.iter().find(|i| i.iid == entity_iid) {
            let labels = if issue.labels.is_empty() {
                "None".to_string()
            } else {
                issue.labels.join(", ")
            };
            let milestone = issue
                .milestone
                .as_ref()
                .map(|m| m.title.clone())
                .unwrap_or_else(|| "None".to_string());
            let assignees = if issue.assignees.is_empty() {
                "None".to_string()
            } else {
                issue
                    .assignees
                    .iter()
                    .map(|a| format!("@{}", a.username))
                    .collect::<Vec<_>>()
                    .join(", ")
            };

            let selected_idx = app.edit_menu.as_ref().map(|m| m.selected_idx).unwrap_or(0);

            let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
            let program = if is_github { "gh" } else { "glab" };
            let mut fields = vec![
                ("Title".to_string(), issue.title.clone()),
                ("Labels".to_string(), labels),
                ("Assignees".to_string(), assignees),
                ("Milestone".to_string(), milestone),
            ];
            if !is_github {
                fields.push(("Confidential".to_string(), "Toggle/Set".to_string()));
                fields.push((
                    "Due Date".to_string(),
                    issue.due_date.clone().unwrap_or_else(|| "Set".to_string()),
                ));
                fields.push(("Weight".to_string(), "Set".to_string()));
            }
            fields.push((
                "Description".to_string(),
                issue.description.clone().unwrap_or_default(),
            ));

            app.edit_menu = Some(crate::app::EditMenu {
                title: format!("Edit Issue #{}", issue.iid),
                fields,
                selected_idx,
                entity_iid: issue.iid,
                entity_type: "issue".to_string(),
                state: {
                    let mut s = ratatui::widgets::ListState::default();
                    s.select(Some(selected_idx));
                    s
                },
            });
        }
    } else if entity_type == "mr" {
        if let Some(mr) = app.mrs.items.iter().find(|m| m.iid == entity_iid) {
            let labels = if mr.labels.is_empty() {
                "None".to_string()
            } else {
                mr.labels.join(", ")
            };
            let milestone = mr
                .milestone
                .as_ref()
                .map(|m| m.title.clone())
                .unwrap_or_else(|| "None".to_string());
            let assignees = if mr.assignees.is_empty() {
                "None".to_string()
            } else {
                mr.assignees
                    .iter()
                    .map(|a| format!("@{}", a.username))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let reviewers = if mr.reviewers.is_empty() {
                "None".to_string()
            } else {
                mr.reviewers
                    .iter()
                    .map(|r| format!("@{}", r.username))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let draft_status = if mr.draft { "Draft" } else { "Ready" };

            let selected_idx = app.edit_menu.as_ref().map(|m| m.selected_idx).unwrap_or(0);

            let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
            let program = if is_github { "gh" } else { "glab" };
            let is_github = is_github;
            let mut fields = vec![
                ("Title".to_string(), mr.title.clone()),
                ("Labels".to_string(), labels),
                ("Assignees".to_string(), assignees),
                ("Reviewers".to_string(), reviewers),
                ("Milestone".to_string(), milestone),
            ];
            if !is_github {
                fields.push(("Target Branch".to_string(), mr.target_branch.clone()));
                fields.push(("Draft Status".to_string(), draft_status.to_string()));
            }
            fields.push((
                "Description".to_string(),
                mr.description.clone().unwrap_or_default(),
            ));

            app.edit_menu = Some(crate::app::EditMenu {
                title: format!("Edit MR #{}", mr.iid),
                fields,
                selected_idx,
                entity_iid: mr.iid,
                entity_type: "mr".to_string(),
                state: {
                    let mut s = ratatui::widgets::ListState::default();
                    s.select(Some(selected_idx));
                    s
                },
            });
        }
    } else if entity_type == "milestone" {
        if let Some(milestone) = app.milestones.items.iter().find(|m| m.iid == entity_iid) {
            let is_github = app
                .gitlab_client
                .as_ref()
                .map(|c| c.is_github)
                .unwrap_or(false);
            let selected_idx = app.edit_menu.as_ref().map(|m| m.selected_idx).unwrap_or(0);
            let mut fields = vec![("Title".to_string(), milestone.title.clone())];
            if !is_github {
                fields.push((
                    "Start Date".to_string(),
                    milestone
                        .start_date
                        .clone()
                        .unwrap_or_else(|| "Set".to_string()),
                ));
            }
            fields.push((
                "Due Date".to_string(),
                milestone
                    .due_date
                    .clone()
                    .unwrap_or_else(|| "Set".to_string()),
            ));
            fields.push((
                "Description".to_string(),
                milestone.description.clone().unwrap_or_default(),
            ));

            app.edit_menu = Some(crate::app::EditMenu {
                title: format!("Edit Milestone #{}", milestone.iid),
                fields,
                selected_idx,
                entity_iid: milestone.iid,
                entity_type: "milestone".to_string(),
                state: {
                    let mut s = ratatui::widgets::ListState::default();
                    s.select(Some(selected_idx));
                    s
                },
            });
        }
    }
}

pub async fn handle_entity_update(
    app: &mut App,
    entity_type: &str,
    iid: u64,
    code: KeyCode,
    terminal: &mut AppTerminal,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    tab: crate::app::Tab,
) {
    match code {
        KeyCode::Char('t') => {
            let current_title = if entity_type == "issue" {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .map(|i| i.title.clone())
                    .unwrap_or_default()
            } else {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.title.clone())
                    .unwrap_or_default()
            };

            if let Some(new_title) = edit_in_editor(&current_title, terminal) {
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let result = if entity_type == "issue" {
                    client
                        .update_issue_title(&project_path, iid, &new_title)
                        .await
                } else {
                    client.update_mr_title(&project_path, iid, &new_title).await
                };
                if let Err(e) = result {
                    app.error_message = Some(format!("Failed to update title: {}", e));
                    return;
                }
                if entity_type == "issue" {
                    if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                        item.title = new_title;
                    }
                } else if entity_type == "mr" {
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.title = new_title;
                    }
                }
            }
        }
        KeyCode::Char('s') => {
            if entity_type == "mr" {
                let is_draft = app
                    .mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.draft)
                    .unwrap_or(false);
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                if let Err(e) = client.toggle_mr_draft(&project_path, iid, is_draft).await {
                    app.error_message = Some(format!("Failed to toggle draft: {}", e));
                    return;
                }
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.draft = !is_draft;
                }
            }
        }
        KeyCode::Char('g') => {
            if entity_type == "mr" {
                let current_branch = app
                    .mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.target_branch.clone())
                    .unwrap_or_default();
                if let Some(target) = edit_in_editor(&current_branch, terminal) {
                    let Some(client) = app.gitlab_client.clone() else {
                        return;
                    };
                    let project_path = app.project_context.clone();
                    if let Err(e) = client
                        .update_mr_target_branch(&project_path, iid, &target)
                        .await
                    {
                        app.error_message = Some(format!("Failed to update target branch: {}", e));
                        return;
                    }
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.target_branch = target;
                    }
                }
            }
        }
        KeyCode::Char('c') => {
            if entity_type == "issue" {
                if let Some(res) = edit_in_editor("public", terminal) {
                    let flag = if res.to_lowercase().contains("confidential") {
                        "--confidential"
                    } else {
                        "--public"
                    };
                    let Some(client) = app.gitlab_client.clone() else {
                        return;
                    };
                    let ppc = app.project_context.clone();
                    let confidential_val = flag == "--confidential";
                    if let Err(e) = client
                        .update_issue_confidential(&ppc, iid, confidential_val)
                        .await
                    {
                        app.error_message =
                            Some(format!("Failed to update confidentiality: {}", e));
                    }
                }
            }
        }
        KeyCode::Char('u') => {
            if entity_type == "issue" {
                if let Some(due_date) = edit_in_editor("YYYY-MM-DD", terminal) {
                    let flag_value = if due_date == "YYYY-MM-DD" || due_date.is_empty() {
                        ""
                    } else {
                        &due_date
                    };
                    let Some(client) = app.gitlab_client.clone() else {
                        return;
                    };
                    let project_path = app.project_context.clone();
                    if let Err(e) = client
                        .update_issue_due_date(&project_path, iid, flag_value)
                        .await
                    {
                        app.error_message = Some(format!("Failed to update due date: {}", e));
                    }
                }
            }
        }
        KeyCode::Char('w') => {
            if entity_type == "issue" {
                if let Some(weight) = edit_in_editor("0", terminal) {
                    let Some(client) = app.gitlab_client.clone() else {
                        return;
                    };
                    let project_path = app.project_context.clone();
                    if let Err(e) = client
                        .update_issue_weight(&project_path, iid, &weight)
                        .await
                    {
                        app.error_message = Some(format!("Failed to update weight: {}", e));
                    }
                }
            }
        }
        KeyCode::Char('d') => {
            let current_desc = if entity_type == "issue" {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .and_then(|i| i.description.clone())
                    .unwrap_or_default()
            } else {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .and_then(|m| m.description.clone())
                    .unwrap_or_default()
            };
            app.text_input = Some(crate::app::TextInput {
                title: " Edit Description ".to_string(),
                value: current_desc.clone(),
                cursor_idx: current_desc.len(),
                action: crate::app::TextInputAction::EditField {
                    entity_iid: iid,
                    entity_type: entity_type.to_string(),
                    field_type: "description".to_string(),
                },
            });
        }
        KeyCode::Char('D') => {
            let current_desc = if entity_type == "issue" {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .and_then(|i| i.description.clone())
                    .unwrap_or_default()
            } else {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .and_then(|m| m.description.clone())
                    .unwrap_or_default()
            };
            if let Some(new_desc) = edit_in_editor(&current_desc, terminal) {
                if entity_type == "issue" {
                    if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                        item.description = Some(new_desc.clone());
                    }
                } else if entity_type == "mr" {
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.description = Some(new_desc.clone());
                    }
                }
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let result = if entity_type == "issue" {
                    client
                        .update_issue_description(&project_path, iid, &new_desc)
                        .await
                } else {
                    client
                        .update_mr_description(&project_path, iid, &new_desc)
                        .await
                };
                if let Err(e) = result {
                    app.error_message = Some(format!("Failed to update description: {}", e));
                }
            }
        }
        _ => {}
    }
}
