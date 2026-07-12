use crate::AppTerminal;
use crate::app::App;
use crate::entity_editor::rebuild_edit_menu;
use crate::event::Event;
use crate::fetch::spawn_refresh_active_tab;
use crate::keybinding::keybinding_matches;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::ListState;
use tokio::sync::mpsc::UnboundedSender;

pub async fn handle_active_tab_key(
    app: &mut App,
    key_event: &KeyEvent,
    terminal: &mut AppTerminal,
    tx: UnboundedSender<Event>,
) {
    let mut handled = true;
    match app.active_tab {
        crate::app::Tab::Issues => match key_event.code {
            _ if keybinding_matches(&app.config.keybindings.issues.create_issue, key_event) => {
                let is_github = app
                    .gitlab_client
                    .as_ref()
                    .map(|c| c.is_github)
                    .unwrap_or(false);
                let mut fields = vec![
                    ("Title".to_string(), String::new()),
                    ("Labels".to_string(), String::new()),
                    ("Assignees".to_string(), String::new()),
                    ("Milestone".to_string(), String::new()),
                ];
                if !is_github {
                    fields.push(("Confidential".to_string(), "No".to_string()));
                    fields.push(("Due Date".to_string(), String::new()));
                    fields.push(("Weight".to_string(), "0".to_string()));
                }
                fields.push(("Description".to_string(), String::new()));
                app.edit_menu = Some(crate::app::EditMenu {
                    title: "Create Issue".to_string(),
                    fields,
                    selected_idx: 0,
                    entity_iid: 0,
                    entity_type: "new_issue".to_string(),
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                });
            }
            _ if keybinding_matches(&app.config.keybindings.issues.edit_entity, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    let filtered = app.filtered_issues();
                    if let Some(issue) = filtered.get(selected_idx) {
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
                        let is_github = app
                            .gitlab_client
                            .as_ref()
                            .map(|c| c.is_github)
                            .unwrap_or(false);
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
                            selected_idx: 0,
                            entity_iid: issue.iid,
                            entity_type: "issue".to_string(),
                            state: {
                                let mut s = ListState::default();
                                s.select(Some(0));
                                s
                            },
                        });
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.close_entity, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    let filtered = app.filtered_issues();
                    if let Some(issue) = filtered.get(selected_idx) {
                        let issue_iid = issue.iid;
                        app.confirm_popup = Some(crate::app::ConfirmAction::CloseIssue(issue_iid));
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.delete_entity, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    let filtered = app.filtered_issues();
                    if let Some(issue) = filtered.get(selected_idx) {
                        let issue_iid = issue.iid;
                        app.confirm_popup = Some(crate::app::ConfirmAction::DeleteIssue(issue_iid));
                    }
                }
            }
            KeyCode::Char('o') => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    if let Some(issue) = app.filtered_issues().get(selected_idx) {
                        let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                        let args = vec![
                            "issue".to_string(),
                            "view".to_string(),
                            issue.iid.to_string(),
                            if is_github { "--web" } else { "-w" }.to_string(),
                        ];
                        crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.reopen_entity, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    let filtered = app.filtered_issues();
                    if let Some(issue) = filtered.get(selected_idx) {
                        let issue_iid = issue.iid;
                        let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                        let args = vec![
                            "issue".to_string(),
                            "reopen".to_string(),
                            issue_iid.to_string(),
                        ];
                        crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
                    }
                }
            }
            _ => handled = false,
        },
        crate::app::Tab::MergeRequests => {
            if keybinding_matches(&app.config.keybindings.mrs.create_mr, key_event) {
                let is_github = app
                    .gitlab_client
                    .as_ref()
                    .map(|c| c.is_github)
                    .unwrap_or(false);
                let pr_suffix = if is_github {
                    "Pull Request"
                } else {
                    "Merge Request"
                };

                let mut all_items = vec!["Create blank (No issue)".to_string()];
                let is_loading = app.issues.items.is_empty();
                if !is_loading {
                    for issue in &app.issues.items {
                        if issue.state == "opened" || issue.state == "open" {
                            all_items.push(format!("#{} {}", issue.iid, issue.title));
                        }
                    }
                }

                app.selector = Some(crate::app::Selector {
                    title: format!(" Select Issue to Base {} On ", pr_suffix),
                    all_items,
                    selected_items: std::collections::HashSet::new(),
                    cursor_idx: 0,
                    search_query: String::new(),
                    is_filtering: false,
                    is_loading,
                    entity_iid: 0,
                    entity_type: "new_mr_selector".to_string(),
                    field_type: "create_mr".to_string(),
                    multi_select: false,
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                });

                if is_loading {
                    if let Some(client) = &app.gitlab_client {
                        spawn_refresh_active_tab(
                            client,
                            &app.project_context,
                            crate::app::Tab::Issues,
                            tx.clone(),
                        );
                    }
                }
            } else if let Some(selected_idx) = app.mrs.state.selected() {
                let filtered = app.filtered_mrs();
                let mr_info = filtered
                    .get(selected_idx)
                    .map(|item| (item.iid, item.title.clone()));
                if let Some((mr_iid, mr_title)) = mr_info {
                    match key_event.code {
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.edit_entity,
                            key_event,
                        ) =>
                        {
                            let mr = filtered.get(selected_idx).unwrap();
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
                            let pr_suffix = if app
                                .gitlab_client
                                .as_ref()
                                .map(|c| c.is_github)
                                .unwrap_or(false)
                            {
                                "PR"
                            } else {
                                "MR"
                            };
                            app.edit_menu = Some(crate::app::EditMenu {
                                title: format!("Edit {} #{}", pr_suffix, mr.iid),
                                fields: vec![
                                    ("Title".to_string(), mr.title.clone()),
                                    ("Labels".to_string(), labels),
                                    ("Assignees".to_string(), assignees),
                                    ("Reviewers".to_string(), reviewers),
                                    ("Milestone".to_string(), milestone),
                                    ("Target Branch".to_string(), mr.target_branch.clone()),
                                    ("Status (Draft/Ready)".to_string(), draft_status.to_string()),
                                    (
                                        "Description".to_string(),
                                        mr.description.clone().unwrap_or_default(),
                                    ),
                                ],
                                selected_idx: 0,
                                entity_iid: mr.iid,
                                entity_type: "mr".to_string(),
                                state: {
                                    let mut s = ListState::default();
                                    s.select(Some(0));
                                    s
                                },
                            });
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.approve_mr,
                            key_event,
                        ) =>
                        {
                            let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                            let args = if is_github {
                                vec![
                                    "pr".to_string(),
                                    "review".to_string(),
                                    mr_iid.to_string(),
                                    "--approve".to_string(),
                                ]
                            } else {
                                vec!["mr".to_string(), "approve".to_string(), mr_iid.to_string()]
                            };
                            crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.merge_mr,
                            key_event,
                        ) =>
                        {
                            let is_github = app
                                .gitlab_client
                                .as_ref()
                                .map(|c| c.is_github)
                                .unwrap_or(false);
                            let all_items = if is_github {
                                vec![
                                    "Squash".to_string(),
                                    "Delete source branch".to_string(),
                                    "Create merge commit".to_string(),
                                    "Rebase and merge".to_string(),
                                ]
                            } else {
                                vec!["Squash".to_string(), "Delete source branch".to_string()]
                            };
                            app.selector = Some(crate::app::Selector {
                                title: format!(" Merge MR/PR #{} - Options ", mr_iid),
                                all_items,
                                selected_items: {
                                    let mut s = std::collections::HashSet::new();
                                    s.insert("Squash".to_string());
                                    s.insert("Delete source branch".to_string());
                                    s
                                },
                                cursor_idx: 0,
                                search_query: String::new(),
                                is_filtering: false,
                                is_loading: false,
                                entity_iid: mr_iid,
                                entity_type: "mr".to_string(),
                                field_type: "merge_options".to_string(),
                                multi_select: true,
                                state: ListState::default(),
                            });
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.view_diff,
                            key_event,
                        ) =>
                        {
                            app.diff_loading = true;
                            let tx = tx.clone();
                            let mr_iid = mr_iid;
                            let mr_iid_str = mr_iid.to_string();
                            let client = app.gitlab_client.clone();
                            let project_context = app.project_context.clone();
                            tokio::spawn(async move {
                                let is_github = match tokio::process::Command::new("git")
                                    .args(["remote", "get-url", "origin"])
                                    .output()
                                    .await
                                    .map(|o| {
                                        String::from_utf8_lossy(&o.stdout).contains("github.com")
                                    }) {
                                    Ok(true) => true,
                                    _ => false,
                                };

                                let program = if is_github { "gh" } else { "glab" };
                                let (entity, sub) = if is_github {
                                    ("pr", "diff")
                                } else {
                                    ("mr", "diff")
                                };
                                let cmd_args =
                                    vec![entity.to_string(), sub.to_string(), mr_iid_str.clone()];
                                let status_msg =
                                    format!("Fetching Diff: {} {}", program, cmd_args.join(" "));
                                let _ = tx.send(Event::CommandStarted(status_msg));

                                let mut cmd = tokio::process::Command::new(program);
                                cmd.args(&cmd_args);

                                let diff_res = cmd.output().await;

                                let comments = if let Some(ref c) = client {
                                    crate::gitlab::mr::list_mr_notes(c, &project_context, mr_iid)
                                        .await
                                        .unwrap_or_default()
                                } else {
                                    vec![]
                                };

                                match diff_res {
                                    Ok(output) => {
                                        if output.status.success() {
                                            let raw_diff = String::from_utf8_lossy(&output.stdout)
                                                .into_owned();
                                            let _ = tx.send(Event::DiffFetched {
                                                mr_iid,
                                                raw_diff,
                                                comments,
                                            });
                                        } else {
                                            let err_msg = String::from_utf8_lossy(&output.stderr);
                                            let _ = tx.send(Event::DiffFetchFailed(format!(
                                                "Failed to fetch diff: {}",
                                                err_msg
                                            )));
                                        }
                                    }
                                    Err(_) => {
                                        let _ = tx.send(Event::DiffFetchFailed(
                                            "Failed to execute CLI tool to fetch diff".to_string(),
                                        ));
                                    }
                                }
                            });
                        }
                        KeyCode::Char('o') => {
                            let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                            let args = vec![
                                if is_github { "pr" } else { "mr" }.to_string(),
                                "view".to_string(),
                                mr_iid.to_string(),
                                if is_github { "--web" } else { "-w" }.to_string(),
                            ];
                            crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.toggle_draft,
                            key_event,
                        ) =>
                        {
                            let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                            let is_draft = app
                                .mrs
                                .items
                                .iter()
                                .find(|m| m.iid == mr_iid)
                                .map(|m| m.draft)
                                .unwrap_or_else(|| {
                                    mr_title.starts_with("Draft:") || mr_title.starts_with("WIP:")
                                });
                            // GitHub uses `gh pr ready <iid>` to mark ready;
                            // `gh pr edit --ready` is not a valid flag.
                            let args = if is_github && is_draft {
                                vec!["pr".to_string(), "ready".to_string(), mr_iid.to_string()]
                            } else {
                                let action = if is_draft { "--ready" } else { "--draft" };
                                let entity = if is_github { "pr" } else { "mr" };
                                let sub = if is_github { "edit" } else { "update" };
                                vec![entity.to_string(), sub.to_string(), mr_iid.to_string(), action.to_string()]
                            };
                            crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
                            if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == mr_iid) {
                                item.draft = !is_draft;
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.close_entity,
                            key_event,
                        ) =>
                        {
                            app.confirm_popup = Some(crate::app::ConfirmAction::CloseMr(mr_iid));
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.delete_entity,
                            key_event,
                        ) =>
                        {
                            if !app
                                .gitlab_client
                                .as_ref()
                                .map(|c| c.is_github)
                                .unwrap_or(false)
                            {
                                app.confirm_popup =
                                    Some(crate::app::ConfirmAction::DeleteMr(mr_iid));
                            } else {
                                app.error_message = Some(
                                    "GitHub does not support deleting pull requests".to_string(),
                                );
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.reopen_entity,
                            key_event,
                        ) =>
                        {
                            let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                            let args = vec![
                                if is_github { "pr" } else { "mr" }.to_string(),
                                "reopen".to_string(),
                                mr_iid.to_string(),
                            ];
                            crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
                        }
                        _ => handled = false,
                    }
                } else {
                    handled = false;
                }
            } else {
                handled = false;
            }
        }
        crate::app::Tab::Pipelines => {
            if key_event.code == KeyCode::Char('n') {
                let current_branch =
                    crate::git_helpers::get_current_branch().unwrap_or_else(|| "main".to_string());

                app.edit_menu = Some(crate::app::EditMenu {
                    title: "Run Pipeline".to_string(),
                    fields: vec![
                        ("Branch / Ref".to_string(), current_branch.clone()),
                        ("Merge Request Pipeline".to_string(), "No".to_string()),
                        ("Variables".to_string(), String::new()),
                        ("Inputs".to_string(), String::new()),
                        ("Workflow / CI File (GitHub)".to_string(), String::new()),
                    ],
                    selected_idx: 0,
                    entity_iid: 0,
                    entity_type: "new_pipeline".to_string(),
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                });
            } else if key_event.code == KeyCode::Char('p')
                || keybinding_matches(
                    &app.config.keybindings.pipelines.trigger_pipeline,
                    &key_event,
                )
            {
                let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                let args = if is_github {
                    vec!["workflow".to_string(), "run".to_string()]
                } else {
                    vec!["ci".to_string(), "run".to_string(), "--mr".to_string()]
                };
                crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
            } else if let Some(selected_idx) = app.pipelines.state.selected() {
                if let Some(item) = app.filtered_pipelines().get(selected_idx) {
                    let pipe_id = item.id();
                    match key_event.code {
                        KeyCode::Char(' ') => {
                            if app.selected_pipelines.contains(&pipe_id) {
                                app.selected_pipelines.remove(&pipe_id);
                            } else {
                                app.selected_pipelines.insert(pipe_id);
                            }
                        }
                        _ if (key_event.code == KeyCode::Char('r')
                            || keybinding_matches(
                                &app.config.keybindings.pipelines.retry,
                                &key_event,
                            )) =>
                        {
                            if let Some(client) = &app.gitlab_client {
                                let client_clone = client.clone();
                                let project_context = app.project_context.clone();
                                let tx = tx.clone();
                                let active_tab = app.active_tab;
                                if !app.selected_pipelines.is_empty() {
                                    let pipe_ids: Vec<u64> =
                                        app.selected_pipelines.iter().cloned().collect();
                                    for p_id in &pipe_ids {
                                        if let Some(p) = app
                                            .pipelines
                                            .items
                                            .iter_mut()
                                            .find(|pipe| pipe.id() == *p_id)
                                        {
                                            match p {
                                                crate::gitlab::pipelines::PipelineItem::Gitlab(p) => p.status = "running".to_string(),
                                                crate::gitlab::pipelines::PipelineItem::Github { effective_status, .. } => *effective_status = "running".to_string(),
                                                _ => {}
                                            }
                                        }
                                    }
                                    app.selected_pipelines.clear();
                                    tokio::spawn(async move {
                                        for p_id in pipe_ids {
                                            let endpoint = format!(
                                                "projects/{}/pipelines/{}/retry",
                                                project_context.replace("/", "%2F"),
                                                p_id
                                            );
                                            let _ = client_clone.fetch_raw_api(&endpoint).await;
                                        }
                                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                        spawn_refresh_active_tab(
                                            &client_clone,
                                            &project_context,
                                            active_tab,
                                            tx.clone(),
                                        );
                                    });
                                } else {
                                    if let Some(p) = app
                                        .pipelines
                                        .items
                                        .iter_mut()
                                        .find(|pipe| pipe.id() == pipe_id)
                                    {
                                        match p {
                                            crate::gitlab::pipelines::PipelineItem::Gitlab(p) => {
                                                p.status = "running".to_string()
                                            }
                                            crate::gitlab::pipelines::PipelineItem::Github {
                                                effective_status,
                                                ..
                                            } => *effective_status = "running".to_string(),
                                            _ => {}
                                        }
                                    }
                                    let tx = tx.clone();
                                    tokio::spawn(async move {
                                        let endpoint = format!(
                                            "projects/{}/pipelines/{}/retry",
                                            project_context.replace("/", "%2F"),
                                            pipe_id
                                        );
                                        let _ = client_clone.fetch_raw_api(&endpoint).await;
                                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                        spawn_refresh_active_tab(
                                            &client_clone,
                                            &project_context,
                                            active_tab,
                                            tx,
                                        );
                                    });
                                }
                            }
                        }
                        _ if (key_event.code == KeyCode::Char('d')
                            || keybinding_matches(
                                &app.config.keybindings.pipelines.cancel,
                                &key_event,
                            )) =>
                        {
                            if let Some(p) = app
                                .pipelines
                                .items
                                .iter_mut()
                                .find(|pipe| pipe.id() == pipe_id)
                            {
                                match p {
                                    crate::gitlab::pipelines::PipelineItem::Gitlab(p) => {
                                        p.status = "canceled".to_string()
                                    }
                                    crate::gitlab::pipelines::PipelineItem::Github {
                                        effective_status,
                                        ..
                                    } => *effective_status = "canceled".to_string(),
                                    _ => {}
                                }
                            }
                            if let Some(client) = &app.gitlab_client {
                                let client_clone = client.clone();
                                let project_context = app.project_context.clone();
                                let tx = tx.clone();
                                let active_tab = app.active_tab;
                                tokio::spawn(async move {
                                    let endpoint = format!(
                                        "projects/{}/pipelines/{}/cancel",
                                        project_context.replace("/", "%2F"),
                                        pipe_id
                                    );
                                    let _ = client_clone.fetch_raw_api(&endpoint).await;
                                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                    spawn_refresh_active_tab(
                                        &client_clone,
                                        &project_context,
                                        active_tab,
                                        tx,
                                    );
                                });
                            }
                        }
                        KeyCode::Char('o') => {
                            let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                            let (entity, sub) = if is_github {
                                ("run", "view")
                            } else {
                                ("ci", "view")
                            };
                            let args = vec![
                                entity.to_string(),
                                sub.to_string(),
                                pipe_id.to_string(),
                                if is_github { "--web" } else { "-w" }.to_string(),
                            ];
                            crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
                        }
                        _ => handled = false,
                    }
                } else {
                    handled = false;
                }
            } else {
                handled = false;
            }
        }
        crate::app::Tab::Jobs => {
            if keybinding_matches(&app.config.keybindings.jobs.enter_pipeline, key_event) {
                let pipelines: Vec<String> = app
                    .pipelines
                    .items
                    .iter()
                    .map(|p| format!("#{} — {} ({})", p.id(), p.ref_branch(), p.status()))
                    .collect();
                let mut pre_selected = std::collections::HashSet::new();
                if let Some(active_id) = app.active_pipeline_id {
                    if let Some(i) = app.pipelines.items.iter().position(|p| p.id() == active_id) {
                        if let Some(p) = pipelines.get(i) {
                            pre_selected.insert(p.clone());
                        }
                    }
                }
                let start_idx = pre_selected
                    .iter()
                    .next()
                    .and_then(|sel| pipelines.iter().position(|p| p == sel))
                    .unwrap_or(0);
                app.selector = Some(crate::app::Selector {
                    title: " Select Pipeline ".to_string(),
                    all_items: pipelines,
                    selected_items: pre_selected,
                    cursor_idx: start_idx,
                    search_query: String::new(),
                    is_filtering: false,
                    is_loading: false,
                    entity_iid: 0,
                    entity_type: String::new(),
                    field_type: "pipeline_select".to_string(),
                    multi_select: false,
                    state: {
                        let mut s = ratatui::widgets::ListState::default();
                        s.select(Some(start_idx));
                        s
                    },
                });
            } else if let Some(idx) = app.jobs.state.selected() {
                let job_info = app
                    .filtered_jobs()
                    .get(idx)
                    .map(|j| (j.id(), j.name().to_string()));
                if let Some((job_id, job_name)) = job_info {
                    match key_event.code {
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.select_job,
                            key_event,
                        ) =>
                        {
                            if app.selected_jobs.contains(&job_id) {
                                app.selected_jobs.remove(&job_id);
                            } else {
                                app.selected_jobs.insert(job_id);
                            }
                        }
                        _ if keybinding_matches(&app.config.keybindings.jobs.retry, key_event) => {
                            if let Some(client) = &app.gitlab_client {
                                let client_clone = client.clone();
                                let project_context = app.project_context.clone();
                                let pipe_id = app.active_pipeline_id.unwrap_or(0);
                                let tx = tx.clone();

                                if !app.selected_jobs.is_empty() {
                                    let job_ids: Vec<u64> =
                                        app.selected_jobs.iter().cloned().collect();
                                    for j in app.jobs.items.iter_mut() {
                                        if app.selected_jobs.contains(&j.id()) {
                                            match j {
                                                crate::gitlab::pipelines::JobItem::Gitlab(j) => {
                                                    j.status = "running".to_string()
                                                }
                                                crate::gitlab::pipelines::JobItem::Github {
                                                    effective_status,
                                                    ..
                                                } => *effective_status = "running".to_string(),
                                                _ => {}
                                            }
                                        }
                                    }
                                    app.selected_jobs.clear();
                                    tokio::spawn(async move {
                                        for j_id in job_ids {
                                            let endpoint = format!(
                                                "projects/{}/jobs/{}/retry",
                                                project_context.replace("/", "%2F"),
                                                j_id
                                            );
                                            let _ = client_clone.fetch_raw_api(&endpoint).await;
                                        }
                                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                        if let Ok(jobs) =
                                            crate::gitlab::pipelines::list_pipeline_jobs(
                                                &client_clone,
                                                &project_context,
                                                pipe_id,
                                            )
                                            .await
                                        {
                                            let _ = tx.send(Event::PipelineJobs(pipe_id, jobs));
                                        }
                                    });
                                } else {
                                    if let Some(j) = app.jobs.items.get_mut(idx) {
                                        match j {
                                            crate::gitlab::pipelines::JobItem::Gitlab(j) => {
                                                j.status = "running".to_string()
                                            }
                                            crate::gitlab::pipelines::JobItem::Github {
                                                effective_status,
                                                ..
                                            } => *effective_status = "running".to_string(),
                                            _ => {}
                                        }
                                    }
                                    tokio::spawn(async move {
                                        let endpoint = format!(
                                            "projects/{}/jobs/{}/retry",
                                            project_context.replace("/", "%2F"),
                                            job_id
                                        );
                                        let _ = client_clone.fetch_raw_api(&endpoint).await;
                                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                        if let Ok(jobs) =
                                            crate::gitlab::pipelines::list_pipeline_jobs(
                                                &client_clone,
                                                &project_context,
                                                pipe_id,
                                            )
                                            .await
                                        {
                                            let _ = tx.send(Event::PipelineJobs(pipe_id, jobs));
                                        }
                                    });
                                }
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.select_stage,
                            key_event,
                        ) =>
                        {
                            let jobs = &app.jobs.items;
                            if let Some(highlighted_job) = jobs.get(idx) {
                                let stage_name = highlighted_job.stage();
                                for job in jobs {
                                    if job.stage() == stage_name {
                                        app.selected_jobs.insert(job.id());
                                    }
                                }
                                app.status_message =
                                    Some(format!("Selected all jobs in stage '{}'", stage_name));
                            }
                        }
                        _ if keybinding_matches(&app.config.keybindings.jobs.cancel, key_event) => {
                            if let Some(client) = &app.gitlab_client {
                                let client_clone = client.clone();
                                let project_context = app.project_context.clone();
                                let pipe_id = app.active_pipeline_id.unwrap_or(0);
                                let tx = tx.clone();

                                if !app.selected_jobs.is_empty() {
                                    let job_ids: Vec<u64> =
                                        app.selected_jobs.iter().cloned().collect();
                                    for j in app.jobs.items.iter_mut() {
                                        if app.selected_jobs.contains(&j.id()) {
                                            match j {
                                                crate::gitlab::pipelines::JobItem::Gitlab(j) => {
                                                    j.status = "canceled".to_string()
                                                }
                                                crate::gitlab::pipelines::JobItem::Github {
                                                    effective_status,
                                                    ..
                                                } => *effective_status = "canceled".to_string(),
                                                _ => {}
                                            }
                                        }
                                    }
                                    app.selected_jobs.clear();
                                    tokio::spawn(async move {
                                        if client_clone.is_github {
                                            let endpoint = format!(
                                                "projects/{}/pipelines/{}/cancel",
                                                project_context.replace("/", "%2F"),
                                                pipe_id
                                            );
                                            let _ = client_clone.fetch_raw_api(&endpoint).await;
                                        } else {
                                            for j_id in job_ids {
                                                let endpoint = format!(
                                                    "projects/{}/jobs/{}/cancel",
                                                    project_context.replace("/", "%2F"),
                                                    j_id
                                                );
                                                let _ = client_clone.fetch_raw_api(&endpoint).await;
                                            }
                                        }
                                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                        if let Ok(jobs) =
                                            crate::gitlab::pipelines::list_pipeline_jobs(
                                                &client_clone,
                                                &project_context,
                                                pipe_id,
                                            )
                                            .await
                                        {
                                            let _ = tx.send(Event::PipelineJobs(pipe_id, jobs));
                                        }
                                    });
                                } else {
                                    if let Some(j) = app.jobs.items.get_mut(idx) {
                                        match j {
                                            crate::gitlab::pipelines::JobItem::Gitlab(j) => {
                                                j.status = "canceled".to_string()
                                            }
                                            crate::gitlab::pipelines::JobItem::Github {
                                                effective_status,
                                                ..
                                            } => *effective_status = "canceled".to_string(),
                                            _ => {}
                                        }
                                    }
                                    tokio::spawn(async move {
                                        let endpoint = if client_clone.is_github {
                                            format!(
                                                "projects/{}/pipelines/{}/cancel",
                                                project_context.replace("/", "%2F"),
                                                pipe_id
                                            )
                                        } else {
                                            format!(
                                                "projects/{}/jobs/{}/cancel",
                                                project_context.replace("/", "%2F"),
                                                job_id
                                            )
                                        };
                                        let _ = client_clone.fetch_raw_api(&endpoint).await;
                                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                        if let Ok(jobs) =
                                            crate::gitlab::pipelines::list_pipeline_jobs(
                                                &client_clone,
                                                &project_context,
                                                pipe_id,
                                            )
                                            .await
                                        {
                                            let _ = tx.send(Event::PipelineJobs(pipe_id, jobs));
                                        }
                                    });
                                }
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.download_artifact,
                            key_event,
                        ) =>
                        {
                            let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                            let args = if is_github {
                                vec![
                                    "run".to_string(),
                                    "download".to_string(),
                                    "--pattern".to_string(),
                                    job_name,
                                ]
                            } else {
                                let ref_name = app
                                    .active_pipeline_id
                                    .and_then(|pipe_id| {
                                        app.pipelines
                                            .items
                                            .iter()
                                            .find(|p| p.id() == pipe_id)
                                            .map(|p| p.ref_branch().to_string())
                                    })
                                    .unwrap_or_else(|| "master".to_string());
                                vec![
                                    "job".to_string(),
                                    "artifact".to_string(),
                                    ref_name,
                                    job_name,
                                ]
                            };
                            crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.open_in_browser,
                            key_event,
                        ) =>
                        {
                            let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                            let args = if is_github {
                                if let Some(pipe_id) = app.active_pipeline_id {
                                    vec![
                                        "run".to_string(),
                                        "view".to_string(),
                                        pipe_id.to_string(),
                                        if is_github { "--web" } else { "-w" }.to_string(),
                                    ]
                                } else {
                                    vec![
                                        "run".to_string(),
                                        "view".to_string(),
                                        job_id.to_string(),
                                        if is_github { "--web" } else { "-w" }.to_string(),
                                    ]
                                }
                            } else {
                                vec![
                                    "job".to_string(),
                                    "view".to_string(),
                                    job_id.to_string(),
                                    if is_github { "--web" } else { "-w" }.to_string(),
                                ]
                            };
                            crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.view_trace_editor,
                            key_event,
                        ) =>
                        {
                            let temp_file =
                                std::env::temp_dir().join(format!("job_{}_trace.txt", job_id));
                            if let Some(trace) = &app.job_trace {
                                let _ = std::fs::write(&temp_file, trace);
                            } else if let Some(_) = &app.gitlab_client {
                                let _ = std::fs::write(&temp_file, "Trace will be here");
                            }
                            crate::event::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
                            crossterm::terminal::disable_raw_mode().unwrap();
                            crossterm::execute!(
                                std::io::stdout(),
                                crossterm::terminal::LeaveAlternateScreen,
                                crossterm::event::DisableMouseCapture
                            )
                            .unwrap();
                            let editor = std::env::var("EDITOR")
                                .or_else(|_| std::env::var("VISUAL"))
                                .unwrap_or_else(|_| "helix".to_string());
                            let mut cmd = std::process::Command::new(&editor);
                            cmd.arg(temp_file.as_os_str());
                            cmd.stdin(std::process::Stdio::inherit());
                            cmd.stdout(std::process::Stdio::inherit());
                            cmd.stderr(std::process::Stdio::inherit());
                            if let Ok(mut child) = cmd.spawn() {
                                let _ = child.wait();
                            }
                            crossterm::terminal::enable_raw_mode().unwrap();
                            crossterm::execute!(
                                std::io::stdout(),
                                crossterm::terminal::EnterAlternateScreen,
                                crossterm::event::EnableMouseCapture
                            )
                            .unwrap();
                            terminal.clear().unwrap();
                            crate::event::PAUSED.store(false, std::sync::atomic::Ordering::Relaxed);
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.view_trace,
                            key_event,
                        ) =>
                        {
                            if app.job_trace.is_some() {
                                app.details_zoomed = !app.details_zoomed;
                            } else if let Some(client) = &app.gitlab_client {
                                let client = client.clone();
                                let project_context = app.project_context.clone();
                                let tx = tx.clone();
                                app.job_trace_loading = true;
                                tokio::spawn(async move {
                                    let res = crate::gitlab::pipelines::get_job_trace(
                                        &client,
                                        &project_context,
                                        job_id,
                                    )
                                    .await;
                                    let _ = tx.send(Event::JobTraceFetched(
                                        job_id,
                                        res.map_err(|e| e.to_string()),
                                    ));
                                });
                            }
                        }
                        _ => handled = false,
                    }
                } else {
                    handled = false;
                }
            } else {
                handled = false;
            }
        }
        crate::app::Tab::Runners => {
            if let Some(selected_idx) = app.runners.state.selected() {
                if let Some(item) = app.filtered_runners().get(selected_idx) {
                    let runner_id = item.id;
                    match key_event.code {
                        _ if keybinding_matches(
                            &app.config.keybindings.runners.pause,
                            key_event,
                        ) =>
                        {
                            let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                            let args: Vec<String> = vec![
                                "api".into(),
                                "-X".into(),
                                "PUT".into(),
                                format!("runners/{}", runner_id),
                                "-f".into(),
                                "paused=true".into(),
                            ];
                            crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
                            if let Some(runner) =
                                app.runners.items.iter_mut().find(|r| r.id == runner_id)
                            {
                                runner.status = "paused".to_string();
                                runner.active = false;
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.runners.resume,
                            key_event,
                        ) =>
                        {
                            let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                            let args: Vec<String> = vec![
                                "api".into(),
                                "-X".into(),
                                "PUT".into(),
                                format!("runners/{}", runner_id),
                                "-f".into(),
                                "paused=false".into(),
                            ];
                            crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
                            if let Some(runner) =
                                app.runners.items.iter_mut().find(|r| r.id == runner_id)
                            {
                                runner.status = "online".to_string();
                                runner.active = true;
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.runners.edit_description,
                            key_event,
                        ) =>
                        {
                            let current_desc = item.description.clone().unwrap_or_default();
                            app.text_input = Some(crate::app::TextInput {
                                title: " Edit Runner Description ".to_string(),
                                cursor_idx: current_desc.len(),
                                value: current_desc,
                                action: crate::app::TextInputAction::EditField {
                                    entity_iid: runner_id,
                                    entity_type: "runner".to_string(),
                                    field_type: "runner_description".to_string(),
                                },
                            });
                        }
                        _ => handled = false,
                    }
                } else {
                    handled = false;
                }
            } else {
                handled = false;
            }
        }
        crate::app::Tab::Releases => match key_event.code {
            _ if keybinding_matches(&app.config.keybindings.releases.create_release, key_event) => {
                app.edit_menu = Some(crate::app::EditMenu {
                    title: "Create Release".to_string(),
                    fields: vec![
                        ("Tag".to_string(), String::new()),
                        ("Release Name".to_string(), String::new()),
                        ("Description".to_string(), String::new()),
                    ],
                    selected_idx: 0,
                    entity_iid: 0,
                    entity_type: "new_release".to_string(),
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                });
            }
            _ if keybinding_matches(&app.config.keybindings.releases.edit_release, key_event) => {
                if let Some(selected_idx) = app.releases.state.selected() {
                    let release_tag = {
                        let filtered = app.filtered_releases();
                        filtered.get(selected_idx).map(|r| r.tag_name.clone())
                    };
                    if let Some(tag_name) = release_tag {
                        if let Some(idx) = app
                            .releases
                            .items
                            .iter()
                            .position(|r| r.tag_name == tag_name)
                        {
                            rebuild_edit_menu(app, "release", idx as u64);
                        }
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.releases.delete_release, key_event) => {
                if let Some(selected_idx) = app.releases.state.selected() {
                    let filtered = app.filtered_releases();
                    if let Some(release) = filtered.get(selected_idx) {
                        app.confirm_popup = Some(crate::app::ConfirmAction::DeleteRelease(
                            release.tag_name.clone(),
                        ));
                    }
                }
            }
            _ if keybinding_matches(
                &app.config.keybindings.releases.open_in_browser,
                key_event,
            ) =>
            {
                if let Some(selected_idx) = app.releases.state.selected() {
                    let filtered = app.filtered_releases();
                    if let Some(release) = filtered.get(selected_idx) {
                        let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                        let args = vec![
                            "release".to_string(),
                            "view".to_string(),
                            release.tag_name.clone(),
                            if is_github { "--web" } else { "-w" }.to_string(),
                        ];
                        crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
                    }
                }
            }
            _ => handled = false,
        },
        crate::app::Tab::Todos => {
            if let Some(selected_idx) = app.todos.state.selected() {
                if let Some(item) = app.filtered_todos().get(selected_idx) {
                    match key_event.code {
                        _ if keybinding_matches(
                            &app.config.keybindings.todos.mark_as_read,
                            key_event,
                        ) =>
                        {
                            let n_id = item.id.clone();
                            let target_iid = item.target_iid;
                            let target_type = item.target_type.clone();
                            let client_opt = app.gitlab_client.clone();
                            if let Some(client) = client_opt {
                                tokio::spawn(async move {
                                    let _ =
                                        crate::gitlab::notifications::mark_notification_as_read(
                                            &client, &n_id,
                                        )
                                        .await;
                                });
                            }
                            app.active_tab = match target_type.as_str() {
                                "MergeRequest" => crate::app::Tab::MergeRequests,
                                _ => crate::app::Tab::Issues,
                            };
                            app.update_filter_selection();
                            match app.active_tab {
                                crate::app::Tab::Issues => {
                                    if let Some(pos) =
                                        app.issues.items.iter().position(|i| i.iid == target_iid)
                                    {
                                        app.issues.state.select(Some(pos));
                                    }
                                }
                                crate::app::Tab::MergeRequests => {
                                    if let Some(pos) =
                                        app.mrs.items.iter().position(|m| m.iid == target_iid)
                                    {
                                        app.mrs.state.select(Some(pos));
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.todos.open_in_browser,
                            key_event,
                        ) =>
                        {
                            let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                            let entity = if item.target_type.contains("MergeRequest") {
                                if is_github { "pr" } else { "mr" }
                            } else {
                                "issue"
                            };
                            let args = vec![
                                entity.to_string(),
                                "view".to_string(),
                                item.target_iid.to_string(),
                                if is_github { "--web" } else { "-w" }.to_string(),
                            ];
                            crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
                        }
                        _ => handled = false,
                    }
                } else {
                    handled = false;
                }
            } else {
                handled = false;
            }
        }
        crate::app::Tab::Milestones => match key_event.code {
            _ if keybinding_matches(
                &app.config.keybindings.milestones.create_milestone,
                key_event,
            ) =>
            {
                let is_github = app
                    .gitlab_client
                    .as_ref()
                    .map(|c| c.is_github)
                    .unwrap_or(false);
                let mut fields = vec![
                    ("Title".to_string(), String::new()),
                    ("Description".to_string(), String::new()),
                ];
                if !is_github {
                    fields.push(("Start Date".to_string(), String::new()));
                }
                fields.push(("Due Date".to_string(), String::new()));
                app.edit_menu = Some(crate::app::EditMenu {
                    title: "Create Milestone".to_string(),
                    fields,
                    selected_idx: 0,
                    entity_iid: 0,
                    entity_type: "new_milestone".to_string(),
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                });
            }
            _ if keybinding_matches(
                &app.config.keybindings.milestones.edit_milestone,
                key_event,
            ) =>
            {
                if let Some(selected_idx) = app.milestones.state.selected() {
                    let milestone_iid = {
                        let filtered = app.filtered_milestones();
                        filtered.get(selected_idx).map(|m| m.iid)
                    };
                    if let Some(iid) = milestone_iid {
                        rebuild_edit_menu(app, "milestone", iid);
                    }
                }
            }
            _ if keybinding_matches(
                &app.config.keybindings.milestones.close_milestone,
                key_event,
            ) =>
            {
                if let Some(selected_idx) = app.milestones.state.selected() {
                    let filtered = app.filtered_milestones();
                    if let Some(milestone) = filtered.get(selected_idx) {
                        let client = app.gitlab_client.clone().unwrap();
                        let project_path = app.project_context.clone();
                        let milestone_iid = milestone.iid;
                        let tx = tx.clone();
                        let _ = tx.send(Event::CommandStarted(format!(
                            "Closing milestone #{}",
                            milestone_iid
                        )));
                        tokio::spawn(async move {
                            let res = crate::gitlab::milestones::update_milestone_state(
                                &client,
                                &project_path,
                                milestone_iid,
                                true,
                            )
                            .await;
                            match res {
                                Ok(_) => {
                                    let _ = tx.send(Event::MilestoneClosed);
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
                }
            }
            _ if keybinding_matches(
                &app.config.keybindings.milestones.reopen_milestone,
                key_event,
            ) =>
            {
                if let Some(selected_idx) = app.milestones.state.selected() {
                    let filtered = app.filtered_milestones();
                    if let Some(milestone) = filtered.get(selected_idx) {
                        let client = app.gitlab_client.clone().unwrap();
                        let project_path = app.project_context.clone();
                        let milestone_iid = milestone.iid;
                        let tx = tx.clone();
                        let _ = tx.send(Event::CommandStarted(format!(
                            "Reopening milestone #{}",
                            milestone_iid
                        )));
                        tokio::spawn(async move {
                            let res = crate::gitlab::milestones::update_milestone_state(
                                &client,
                                &project_path,
                                milestone_iid,
                                false,
                            )
                            .await;
                            match res {
                                Ok(_) => {
                                    let _ = tx.send(Event::MilestoneReopened);
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
                }
            }
            _ if keybinding_matches(
                &app.config.keybindings.milestones.delete_milestone,
                key_event,
            ) =>
            {
                if let Some(selected_idx) = app.milestones.state.selected() {
                    let filtered = app.filtered_milestones();
                    if let Some(milestone) = filtered.get(selected_idx) {
                        app.confirm_popup =
                            Some(crate::app::ConfirmAction::DeleteMilestone(milestone.iid));
                    }
                }
            }
            _ if keybinding_matches(
                &app.config.keybindings.milestones.open_in_browser,
                key_event,
            ) =>
            {
                if let Some(selected_idx) = app.milestones.state.selected() {
                    let filtered = app.filtered_milestones();
                    if let Some(milestone) = filtered.get(selected_idx) {
                        let is_github = app
                            .gitlab_client
                            .as_ref()
                            .map(|c| c.is_github)
                            .unwrap_or(false);
                        let is_github = app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
        let program = if is_github { "gh" } else { "glab" };
                        let args = if is_github {
                            vec!["browse".to_string(), format!("milestone/{}", milestone.iid)]
                        } else {
                            vec![
                                "milestone".to_string(),
                                "view".to_string(),
                                milestone.iid.to_string(),
                                if is_github { "--web" } else { "-w" }.to_string(),
                            ]
                        };
                        crate::run_cli(program, &args, terminal, tx.clone(), app.active_tab).await;
                    }
                }
            }
            _ => handled = false,
        },
        crate::app::Tab::Branches => {
            if let Some(selected_idx) = app.branches.state.selected() {
                let filtered = app.filtered_branches();
                if let Some(branch) = filtered.get(selected_idx) {
                    let branch_name = branch.name.clone();
                    if keybinding_matches(&app.config.keybindings.branches.create_branch, key_event)
                    {
                        app.text_input = Some(crate::app::TextInput {
                            title: " New Branch Name ".to_string(),
                            value: String::new(),
                            cursor_idx: 0,
                            action: crate::app::TextInputAction::CreateBranch(branch_name),
                        });
                    } else if keybinding_matches(
                        &app.config.keybindings.branches.delete_branch,
                        key_event,
                    ) {
                        app.confirm_popup =
                            Some(crate::app::ConfirmAction::DeleteBranch(branch_name.clone()));
                    }
                }
            }
            handled = false;
        }
        crate::app::Tab::Environments => {
            let mut matched = false;
            if let Some(selected_idx) = app.environments.state.selected() {
                if keybinding_matches(
                    &app.config.keybindings.environments.view_deployments,
                    key_event,
                ) {
                    matched = true;
                    let filtered = app.filtered_environments();
                    if let Some(env) = filtered.get(selected_idx) {
                        let env_name = env.name.clone();
                        let _ = tx.send(Event::CommandStarted(format!(
                            "Fetching deployments for {}",
                            env_name
                        )));
                        let client = app.gitlab_client.clone();
                        let project_context = app.project_context.clone();
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            if let Some(client) = client {
                                match crate::gitlab::deployments::list_deployments(
                                    &client,
                                    &project_context,
                                    Some(&env_name),
                                )
                                .await
                                {
                                    Ok(deployments) => {
                                        let _ = tx.send(Event::DeploymentsFetched(deployments));
                                    }
                                    Err(e) => {
                                        let _ = tx.send(Event::CommandCompleted(
                                            crate::app::Tab::Environments,
                                            Err(format!("Failed to fetch deployments: {}", e)),
                                        ));
                                        let _ = tx.send(Event::FetchFailed(
                                            crate::app::Tab::Environments,
                                            format!("Failed to fetch deployments: {}", e),
                                        ));
                                    }
                                }
                            }
                        });
                    }
                }
            }
            if !matched {
                handled = false;
            }
        }
        crate::app::Tab::Terminal => {
            handled = false;
        }
    }

    if !handled {
        if app.detail_visible
            && keybinding_matches(&app.config.keybindings.global.scroll_down, &key_event)
        {
            app.detail_scroll = app.detail_scroll.saturating_add(1);
        } else if app.detail_visible
            && keybinding_matches(&app.config.keybindings.global.scroll_up, &key_event)
        {
            app.detail_scroll = app.detail_scroll.saturating_sub(1);
        }

        match key_event.code {
            KeyCode::Char('?') | KeyCode::F(1) => {
                app.show_help = true;
            }
            KeyCode::Char('u') => {
                app.error_message = Some("Checking for updates...".to_string());
                let tx = tx.clone();
                tokio::spawn(async move {
                    match crate::utils::update::perform_self_update().await {
                        Ok(true) => {
                            let _ = tx.send(Event::FetchFailed(
                                crate::app::Tab::Todos,
                                "Update complete! Please restart glab-tui.".to_string(),
                            ));
                        }
                        Ok(false) => {
                            let _ = tx.send(Event::FetchFailed(
                                crate::app::Tab::Todos,
                                "Already up to date.".to_string(),
                            ));
                        }
                        Err(e) => {
                            let _ = tx.send(Event::FetchFailed(
                                crate::app::Tab::Todos,
                                format!("Update failed: {}", e),
                            ));
                        }
                    }
                });
            }
            KeyCode::Char('q') => {
                if app.details_zoomed {
                    app.details_zoomed = false;
                } else if app.detail_visible {
                    app.detail_visible = false;
                } else {
                    app.quit();
                }
            }
            KeyCode::Char('m') | KeyCode::Char('M')
                if app.active_tab == crate::app::Tab::Jobs && app.job_trace.is_none() =>
            {
                app.collapse_matrix_jobs = !app.collapse_matrix_jobs;
                app.jobs.state.select(Some(0));
            }

            KeyCode::Esc | KeyCode::Backspace => {
                if app.job_trace_loading {
                    app.job_trace_loading = false;
                } else if app.details_zoomed {
                    app.details_zoomed = false;
                    app.job_trace = None;
                } else if app.detail_visible {
                    app.detail_visible = false;
                } else if app.active_tab == crate::app::Tab::Jobs {
                    if app.job_trace.is_some() {
                        app.job_trace = None;
                    } else {
                        app.active_tab = crate::app::Tab::Pipelines;
                    }
                } else if app.active_tab == crate::app::Tab::Pipelines && !app.jobs.items.is_empty()
                {
                    if app.job_trace.is_some() {
                        app.job_trace = None;
                    } else {
                        app.jobs.items.clear();
                        app.jobs.state.select(None);
                        app.selected_jobs.clear();
                    }
                }
            }
            KeyCode::Char('f') => {
                app.is_typing_search = true;
            }
            KeyCode::Enter => match app.active_tab {
                crate::app::Tab::Todos => {
                    if let Some(idx) = app.todos.state.selected() {
                        if let Some(n) = app.filtered_todos().get(idx) {
                            let n_id = n.id.clone();
                            let target_iid = n.target_iid;
                            let target_type = n.target_type.clone();
                            let client_opt = app.gitlab_client.clone();
                            if let Some(client) = client_opt {
                                tokio::spawn(async move {
                                    let _ =
                                        crate::gitlab::notifications::mark_notification_as_read(
                                            &client, &n_id,
                                        )
                                        .await;
                                });
                            }
                            app.active_tab = match target_type.as_str() {
                                "MergeRequest" => crate::app::Tab::MergeRequests,
                                _ => crate::app::Tab::Issues,
                            };
                            app.update_filter_selection();
                            match app.active_tab {
                                crate::app::Tab::Issues => {
                                    if let Some(pos) =
                                        app.issues.items.iter().position(|i| i.iid == target_iid)
                                    {
                                        app.issues.state.select(Some(pos));
                                    }
                                }
                                crate::app::Tab::MergeRequests => {
                                    if let Some(pos) =
                                        app.mrs.items.iter().position(|m| m.iid == target_iid)
                                    {
                                        app.mrs.state.select(Some(pos));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                crate::app::Tab::Pipelines => {
                    if let Some(idx) = app.pipelines.state.selected() {
                        let pipe_id = app.filtered_pipelines().get(idx).map(|p| p.id());
                        if let Some(pipeline_id) = pipe_id {
                            if let Some(client) = &app.gitlab_client {
                                app.loading_tabs.insert(crate::app::Tab::Jobs);
                                if let Ok(jobs) = crate::gitlab::pipelines::list_pipeline_jobs(
                                    client,
                                    &app.project_context,
                                    pipeline_id,
                                )
                                .await
                                {
                                    app.pipeline_jobs.insert(pipeline_id, jobs.clone());
                                    app.jobs.items = jobs;
                                    app.active_pipeline_id = Some(pipeline_id);
                                    app.jobs.state.select(Some(0));
                                    app.detail_scroll = 0;
                                    app.job_trace = None;
                                    app.active_tab = crate::app::Tab::Jobs;
                                    app.loading_tabs.remove(&crate::app::Tab::Jobs);
                                } else {
                                    app.error_message = Some("Failed to fetch jobs".to_string());
                                    app.loading_tabs.remove(&crate::app::Tab::Jobs);
                                }
                            }
                        }
                    }
                }
                crate::app::Tab::Jobs => {
                    if app.job_trace.is_some() {
                        app.details_zoomed = !app.details_zoomed;
                    } else if let Some(idx) = app.jobs.state.selected() {
                        let job_info = app
                            .filtered_jobs()
                            .get(idx)
                            .map(|j| (j.id(), j.name().to_string()));
                        if let Some((job_id, _)) = job_info {
                            if let Some(client) = &app.gitlab_client {
                                let client = client.clone();
                                let project_context = app.project_context.clone();
                                let tx = tx.clone();
                                app.job_trace_loading = true;
                                tokio::spawn(async move {
                                    let res = crate::gitlab::pipelines::get_job_trace(
                                        &client,
                                        &project_context,
                                        job_id,
                                    )
                                    .await;
                                    let _ = tx.send(Event::JobTraceFetched(
                                        job_id,
                                        res.map_err(|e| e.to_string()),
                                    ));
                                });
                            }
                        }
                    }
                }
                _ => {
                    if !app.detail_visible {
                        app.detail_visible = true;
                        app.details_zoomed = false;
                    } else {
                        app.details_zoomed = !app.details_zoomed;
                    }
                }
            },
            _ if (key_event.code == KeyCode::Right
                || key_event.code == KeyCode::Char('l')
                || keybinding_matches(&app.config.keybindings.global.next_tab, &key_event)) =>
            {
                app.next_tab();
                if let Some(client) = &app.gitlab_client {
                    if !app.loading_tabs.contains(&app.active_tab)
                        && !app.refreshed_tabs.contains(&app.active_tab)
                    {
                        if !app.loaded_tabs.contains(&app.active_tab) {
                            app.loading_tabs.insert(app.active_tab);
                        }
                        spawn_refresh_active_tab(
                            client,
                            &app.project_context,
                            app.active_tab,
                            tx.clone(),
                        );
                    }
                }
            }
            _ if (key_event.code == KeyCode::Left
                || key_event.code == KeyCode::Char('h')
                || keybinding_matches(&app.config.keybindings.global.prev_tab, &key_event)) =>
            {
                app.previous_tab();
                if let Some(client) = &app.gitlab_client {
                    if !app.loading_tabs.contains(&app.active_tab)
                        && !app.refreshed_tabs.contains(&app.active_tab)
                    {
                        if !app.loaded_tabs.contains(&app.active_tab) {
                            app.loading_tabs.insert(app.active_tab);
                        }
                        spawn_refresh_active_tab(
                            client,
                            &app.project_context,
                            app.active_tab,
                            tx.clone(),
                        );
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if app.details_zoomed {
                    app.detail_scroll = app.detail_scroll.saturating_add(1);
                } else {
                    app.detail_scroll = 0;
                    match app.active_tab {
                        crate::app::Tab::Issues => {
                            app.issues.next(app.filtered_issues().len());
                        }
                        crate::app::Tab::MergeRequests => {
                            app.mrs.next(app.filtered_mrs().len());
                        }
                        crate::app::Tab::Pipelines => {
                            app.pipelines.next(app.filtered_pipelines().len());
                        }
                        crate::app::Tab::Jobs => {
                            let len = app.filtered_jobs().len();
                            app.jobs.next(len);
                            app.job_trace = None;
                        }
                        crate::app::Tab::Runners => {
                            app.runners.next(app.filtered_runners().len());
                        }
                        crate::app::Tab::Releases => {
                            app.releases.next(app.filtered_releases().len());
                        }
                        crate::app::Tab::Todos => {
                            app.todos.next(app.filtered_todos().len());
                        }
                        crate::app::Tab::Milestones => {
                            app.milestones.next(app.filtered_milestones().len());
                        }
                        crate::app::Tab::Branches => {
                            app.branches.next(app.filtered_branches().len());
                        }
                        crate::app::Tab::Environments => {
                            app.environments.next(app.filtered_environments().len());
                        }
                        crate::app::Tab::Terminal => {
                            app.terminal_scroll = app.terminal_scroll.saturating_sub(1);
                        }
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if app.details_zoomed {
                    app.detail_scroll = app.detail_scroll.saturating_sub(1);
                } else {
                    app.detail_scroll = 0;
                    match app.active_tab {
                        crate::app::Tab::Issues => {
                            app.issues.previous(app.filtered_issues().len());
                        }
                        crate::app::Tab::MergeRequests => {
                            app.mrs.previous(app.filtered_mrs().len());
                        }
                        crate::app::Tab::Pipelines => {
                            app.pipelines.previous(app.filtered_pipelines().len());
                        }
                        crate::app::Tab::Jobs => {
                            let len = app.filtered_jobs().len();
                            app.jobs.previous(len);
                            app.job_trace = None;
                        }
                        crate::app::Tab::Runners => {
                            app.runners.previous(app.filtered_runners().len());
                        }
                        crate::app::Tab::Releases => {
                            app.releases.previous(app.filtered_releases().len());
                        }
                        crate::app::Tab::Todos => {
                            app.todos.previous(app.filtered_todos().len());
                        }
                        crate::app::Tab::Milestones => {
                            app.milestones.previous(app.filtered_milestones().len());
                        }
                        crate::app::Tab::Branches => {
                            app.branches.previous(app.filtered_branches().len());
                        }
                        crate::app::Tab::Environments => {
                            app.environments.previous(app.filtered_environments().len());
                        }
                        crate::app::Tab::Terminal => {
                            app.terminal_scroll = app.terminal_scroll.saturating_add(1);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
