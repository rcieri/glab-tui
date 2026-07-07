use crate::AppTerminal;
use crate::app::App;
use crate::cli::{UpdateCmd, app_cli};
use crate::editor::edit_in_editor;
use crate::event::Event;
use crate::templates::get_default_template;
use crossterm::event::KeyCode;

pub async fn apply_field_text_change(
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

            let client = app.gitlab_client.clone().unwrap();
            let project_path = app.project_context.clone();
            let tx_spawn = tx.clone();
            let _ = tx.send(Event::CommandStarted(format!(
                "Updating milestone #{}",
                iid
            )));
            tokio::spawn(async move {
                let res = crate::gitlab::milestones::update_milestone(
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

            let client = app.gitlab_client.clone().unwrap();
            let project_path = app.project_context.clone();
            let tx_spawn = tx.clone();
            let _ = tx.send(Event::CommandStarted(format!("Updating release {}", tag)));
            tokio::spawn(async move {
                let res = crate::gitlab::releases::update_release(
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

    let cli = app_cli(app);
    match field_type {
        "title" => {
            let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                .flag("--title", &value)
                .build();
            crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            if entity_type == "issue" {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.title = value;
                }
            } else if entity_type == "mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.title = value;
                }
            }
        }
        "target_branch" => {
            if entity_type == "mr" {
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag("--target-branch", &value)
                    .build();
                crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.target_branch = value;
                }
            }
        }
        "due_date" => {
            if entity_type == "issue" {
                let flag_value = if value == "YYYY-MM-DD" || value.trim().is_empty() {
                    ""
                } else {
                    &value
                };
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag("--due-date", flag_value)
                    .build();
                crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.due_date = if flag_value.is_empty() {
                        None
                    } else {
                        Some(flag_value.to_string())
                    };
                }
            }
        }
        "weight" => {
            if entity_type == "issue" {
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag("--weight", &value)
                    .build();
                crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            }
        }
        "runner_description" => {
            let args: Vec<String> = vec![
                "api".into(),
                "-X".into(),
                "PUT".into(),
                format!("runners/{}", iid),
                "-f".into(),
                format!("description={}", value),
            ];
            crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            if let Some(runner) = app.runners.items.iter_mut().find(|r| r.id == iid) {
                runner.description = Some(value);
            }
        }
        "description" => {
            let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                .flag("-d", &value)
                .build();
            crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            if entity_type == "issue" {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.description = Some(value);
                }
            } else if entity_type == "mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.description = Some(value);
                }
            }
        }
        _ => {}
    }
}

pub async fn apply_selector_changes(
    app: &mut App,
    entity_type: &str,
    iid: u64,
    field_type: &str,
    values: Vec<String>,
    terminal: &mut AppTerminal,
) {
    let cli = app_cli(app);
    let tx = app.tx.clone().unwrap();
    let tab = app.active_tab;
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

            let mut args = UpdateCmd::new(cli.is_github, entity_type, iid);
            for label in &to_add {
                args = args.flag("--label", label);
            }
            for label in &to_remove {
                args = args.flag("--unlabel", label);
            }
            let final_args = args.build();
            if !final_args.is_empty() {
                crate::run_cli(&cli, &final_args, terminal, tx.clone(), tab).await;
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

            let mut args = UpdateCmd::new(cli.is_github, entity_type, iid);
            for assignee in &to_add {
                args = args.flag("--assignee", assignee);
            }
            for assignee in &to_remove {
                args = args.flag("--unassign", assignee);
            }
            let final_args = args.build();
            if !final_args.is_empty() {
                crate::run_cli(&cli, &final_args, terminal, tx.clone(), tab).await;
            }

            if entity_type == "issue" {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.assignees = clean_values
                        .iter()
                        .map(|username| crate::gitlab::issues::Assignee {
                            username: username.clone(),
                        })
                        .collect();
                }
            } else if entity_type == "mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.assignees = clean_values
                        .iter()
                        .map(|username| crate::gitlab::mr::Assignee {
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

                let mut args = UpdateCmd::new(cli.is_github, entity_type, iid);
                for reviewer in &to_add {
                    args = args.flag("--reviewer", reviewer);
                }
                for reviewer in &to_remove {
                    args = args.flag("--unreviewer", reviewer);
                }
                let final_args = args.build();
                if !final_args.is_empty() {
                    crate::run_cli(&cli, &final_args, terminal, tx.clone(), tab).await;
                }

                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.reviewers = clean_values
                        .iter()
                        .map(|username| crate::gitlab::mr::Reviewer {
                            username: username.clone(),
                        })
                        .collect();
                }
            }
        }
        "milestone" => {
            // For milestones, cli expects the title, not the id
            let first_val = values.first().cloned().unwrap_or_default();
            let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                .flag("--milestone", &first_val)
                .build();
            crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            if entity_type == "issue" {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    let m = crate::gitlab::issues::Milestone {
                        title: first_val.clone(),
                    };
                    item.milestone = Some(m);
                }
            } else if entity_type == "mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    let m = crate::gitlab::mr::Milestone {
                        title: first_val.clone(),
                    };
                    item.milestone = Some(m);
                }
            }
        }
        "confidential" => {
            if entity_type == "issue" {
                let is_confidential = values.iter().any(|v| v == "Yes" || v == "true");
                let flag = if is_confidential {
                    "--confidential"
                } else {
                    "--public"
                };
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag_bool(flag)
                    .build();
                crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            }
        }
        "description" => {
            let choice = values.first().cloned().unwrap_or_default();

            if iid == 0 {
                if let Some(ref mut menu) = app.edit_menu {
                    if let Some(f) = menu.fields.iter_mut().find(|f| f.0 == "Description") {
                        if choice == "Edit (basic)" {
                            app.text_input = Some(crate::app::TextInput {
                                title: " Edit Description ".to_string(),
                                value: f.1.clone(),
                                cursor_idx: f.1.len(),
                                action: crate::app::TextInputAction::EditNewField {
                                    field_idx: menu
                                        .fields
                                        .iter()
                                        .position(|f| f.0 == "Description")
                                        .unwrap_or(0),
                                },
                            });
                        } else {
                            let current_val = if f.1.trim().is_empty() {
                                let template_type = if entity_type == "new_mr" {
                                    "mr"
                                } else {
                                    "issue"
                                };
                                get_default_template(template_type).unwrap_or_default()
                            } else {
                                f.1.clone()
                            };
                            if let Some(new_desc) = edit_in_editor(&current_val, terminal) {
                                f.1 = new_desc;
                            }
                        }
                    }
                }
            } else {
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

                if choice == "Edit (basic)" {
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
                } else {
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
                        let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                            .flag("-d", &new_desc)
                            .build();
                        crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                    }
                    if let Some(client) = &app.gitlab_client {
                        if entity_type == "issue" {
                            if let Ok(updated) =
                                crate::gitlab::issues::get_issue(client, &app.project_context, iid)
                                    .await
                            {
                                if let Some(item) =
                                    app.issues.items.iter_mut().find(|i| i.iid == iid)
                                {
                                    *item = updated;
                                }
                            }
                        } else if entity_type == "mr" {
                            if let Ok(updated) =
                                crate::gitlab::mr::get_mr(client, &app.project_context, iid).await
                            {
                                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid)
                                {
                                    *item = updated;
                                }
                            }
                        }
                    }
                }
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

            let cli = app_cli(app);
            let mut fields = vec![
                ("Title".to_string(), issue.title.clone()),
                ("Labels".to_string(), labels),
                ("Assignees".to_string(), assignees),
                ("Milestone".to_string(), milestone),
            ];
            if !cli.is_github {
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

            let cli = app_cli(app);
            let is_github = cli.is_github;
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
    let cli = app_cli(app);
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
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag("--title", &new_title)
                    .build();
                crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
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
                let args = if cli.is_github && is_draft {
                    vec!["pr".to_string(), "ready".to_string(), iid.to_string()]
                } else {
                    let action = if is_draft { "--ready" } else { "--draft" };
                    UpdateCmd::new(cli.is_github, entity_type, iid)
                        .flag_bool(action)
                        .build()
                };
                crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
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
                    let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                        .flag("--target-branch", &target)
                        .build();
                    crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
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
                    let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                        .flag_bool(flag)
                        .build();
                    crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
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
                    let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                        .flag("--due-date", flag_value)
                        .build();
                    crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                }
            }
        }
        KeyCode::Char('w') => {
            if entity_type == "issue" {
                if let Some(weight) = edit_in_editor("0", terminal) {
                    let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                        .flag("--weight", &weight)
                        .build();
                    crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
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
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag("-d", &new_desc)
                    .build();
                crate::run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            }
        }
        _ => {}
    }
}
