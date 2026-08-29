use crate::AppTerminal;
use crate::app::App;
use crate::entity_editor::rebuild_edit_menu;
use crate::event::Event;
use crate::fetch::spawn_refresh_active_tab;
use crate::git_helpers::{get_default_branch, slugify};
use crate::keybinding::keybinding_matches;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::ListState;
use tokio::sync::mpsc::UnboundedSender;

/// Insert the currently-highlighted Issue/MR into its selection set. Used by
/// select mode: toggling the mode on and moving the cursor both mark items.
fn mark_current_selected(app: &mut App) {
    match app.active_tab {
        crate::app::Tab::Issues => {
            if let Some(idx) = app.issues.state.selected() {
                if let Some(iid) = app.filtered_issues().get(idx).map(|i| i.iid) {
                    app.selected_issues.insert(iid);
                }
            }
        }
        crate::app::Tab::MergeRequests => {
            if let Some(idx) = app.mrs.state.selected() {
                if let Some(iid) = app.filtered_mrs().get(idx).map(|m| m.iid) {
                    app.selected_mrs.insert(iid);
                }
            }
        }
        _ => {}
    }
}

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
                let is_github = app.is_github();
                let fields = crate::entity_editor::issue_fields(
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    "No".to_string(),
                    String::new(),
                    "0".to_string(),
                    String::new(),
                    is_github,
                );
                app.open_edit_menu(crate::app::EditMenu {
                    title: "Create Issue".to_string(),
                    fields,
                    initial_fields: std::collections::HashMap::new(),
                    selected_idx: 0,
                    entity_iid: 0,
                    entity_kind: crate::app::EditEntityKind::CreateIssue,
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                    workflow_inputs: vec![],
                    cursor_pos: 0,
                    editing: false,
                    desc_scroll: 0,
                });
            }
            _ if keybinding_matches(&app.config.keybindings.issues.edit_entity, key_event) => {
                if app.selected_issues.len() > 1 {
                    let count = app.selected_issues.len();
                    app.open_edit_menu(crate::app::EditMenu {
                        title: format!("Bulk Edit {} Issues", count),
                        fields: vec![
                            crate::app::Field::multi_select("Assignees", String::new()),
                            crate::app::Field::multi_select("Milestone", String::new()),
                            crate::app::Field::multi_select("Labels", String::new()),
                        ],
                        // Bulk-edit forms start empty; `open_edit_menu` will snapshot all
                        // blank values as the baseline. The `IssueUpdate::is_empty()` guard
                        // in the dispatcher handles the true no-op case (nothing filled in).
                        initial_fields: std::collections::HashMap::new(),
                        selected_idx: 0,
                        entity_iid: 0,
                        entity_kind: crate::app::EditEntityKind::BulkEditIssues,
                        state: {
                            let mut s = ListState::default();
                            s.select(Some(0));
                            s
                        },
                        workflow_inputs: vec![],
                        cursor_pos: 0,
                        editing: false,
                        desc_scroll: 0,
                    });
                } else if let Some(selected_idx) = app.issues.state.selected() {
                    let filtered = app.filtered_issues();
                    if let Some(issue) = filtered.get(selected_idx) {
                        let is_github = app.is_github();
                        let mut doc = crate::entity_editor::build_issue_document(issue, is_github);
                        doc.fields.push(crate::app::Field::text(
                            "Description",
                            issue.description.clone().unwrap_or_default(),
                        ));
                        app.open_edit_menu(crate::app::EditMenu {
                            title: format!("Edit Issue #{}", issue.iid),
                            fields: doc.fields,
                            initial_fields: std::collections::HashMap::new(),
                            selected_idx: 0,
                            entity_iid: issue.iid,
                            entity_kind: crate::app::EditEntityKind::EditIssue,
                            state: {
                                let mut s = ListState::default();
                                s.select(Some(0));
                                s
                            },
                            workflow_inputs: vec![],
                            cursor_pos: 0,
                            editing: false,
                            desc_scroll: 0,
                        });
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.close_entity, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    let filtered = app.filtered_issues();
                    if let Some(issue) = filtered.get(selected_idx) {
                        let issue_iid = issue.iid;
                        app.submit_dialog = Some(crate::app::SubmitDialog::build(
                            crate::app::ConfirmAction::CloseIssue(issue_iid),
                            app,
                        ));
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.delete_entity, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    let filtered = app.filtered_issues();
                    if let Some(issue) = filtered.get(selected_idx) {
                        let issue_iid = issue.iid;
                        app.submit_dialog = Some(crate::app::SubmitDialog::build(
                            crate::app::ConfirmAction::DeleteIssue(issue_iid),
                            app,
                        ));
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.copy_reference, key_event) => {
                if let Err(error) = app.copy_selected_issue_reference() {
                    app.show_error(format!("Failed to copy issue reference: {error}"));
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.open_in_browser, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    if let Some(issue) = app.filtered_issues().get(selected_idx) {
                        let Some(client) = app.gitlab_client.clone() else {
                            return;
                        };
                        let project_path = app.project_context.clone();
                        let iid_str = issue.iid.to_string();
                        let tx2 = tx.clone();
                        tokio::spawn(async move {
                            let result = client
                                .open_in_browser(&project_path, "issue", &iid_str)
                                .await;
                            let _ = tx2.send(Event::CommandCompleted(
                                crate::app::Tab::Issues,
                                result.map_err(|e| e.to_string()),
                            ));
                        });
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.reopen_entity, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    let filtered = app.filtered_issues();
                    if let Some(issue) = filtered.get(selected_idx) {
                        let issue_iid = issue.iid;
                        app.submit_dialog = Some(crate::app::SubmitDialog::build(
                            crate::app::ConfirmAction::ReopenIssue(issue_iid),
                            app,
                        ));
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.select_issue, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    let iid = app.filtered_issues().get(selected_idx).map(|i| i.iid);
                    if let Some(iid) = iid {
                        if app.selected_issues.contains(&iid) {
                            app.selected_issues.remove(&iid);
                        } else {
                            app.selected_issues.insert(iid);
                        }
                    }
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.selection_toggle, key_event) => {
                app.select_mode = !app.select_mode;
                if app.select_mode {
                    mark_current_selected(app);
                }
            }
            _ if keybinding_matches(&app.config.keybindings.issues.create_mr, key_event) => {
                if let Some(selected_idx) = app.issues.state.selected() {
                    let filtered = app.filtered_issues();
                    if let Some(issue) = filtered.get(selected_idx) {
                        let is_github = app.is_github();
                        let pr_suffix = if is_github {
                            "Pull Request"
                        } else {
                            "Merge Request"
                        };

                        let title_val = issue.title.clone();
                        let source_branch_val = format!("{}-{}", issue.iid, slugify(&issue.title));
                        let labels_val = if issue.labels.is_empty() {
                            String::new()
                        } else {
                            issue.labels.join(", ")
                        };
                        let assignees_val = if issue.assignees.is_empty() {
                            String::new()
                        } else {
                            issue
                                .assignees
                                .iter()
                                .map(|a| format!("@{}", a.username))
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        let milestone_val = issue
                            .milestone
                            .as_ref()
                            .map(|m| m.title.clone())
                            .unwrap_or_default();
                        let target_branch_val =
                            get_default_branch().unwrap_or_else(|| "main".to_string());
                        let create_from_val = format!("#{} {}", issue.iid, issue.title);
                        let mut fields = crate::entity_editor::mr_fields(
                            title_val,
                            labels_val,
                            assignees_val,
                            String::new(),
                            milestone_val,
                            target_branch_val,
                            "Draft".to_string(),
                            String::new(),
                            is_github,
                        );
                        // Pre-fill the "Create from Issue" row since we launched
                        // the form directly from this issue.
                        if let Some(f) = fields.iter_mut().find(|f| f.label == "Create from Issue")
                        {
                            f.value = create_from_val;
                        }
                        app.open_edit_menu(crate::app::EditMenu {
                            title: format!("Create {} from #{}", pr_suffix, issue.iid),
                            fields,
                            initial_fields: std::collections::HashMap::new(),
                            selected_idx: 0,
                            entity_iid: issue.iid,
                            entity_kind: crate::app::EditEntityKind::CreateMr,
                            state: {
                                let mut s = ListState::default();
                                s.select(Some(0));
                                s
                            },
                            workflow_inputs: vec![],
                            cursor_pos: 0,
                            editing: false,
                            desc_scroll: 0,
                        });
                    }
                }
            }
            _ => handled = false,
        },
        crate::app::Tab::MergeRequests => {
            if keybinding_matches(&app.config.keybindings.mrs.create_mr, key_event) {
                let is_github = app.is_github();
                let pr_suffix = if is_github {
                    "Pull Request"
                } else {
                    "Merge Request"
                };
                let target_branch_val = get_default_branch().unwrap_or_else(|| "main".to_string());
                let fields = crate::entity_editor::mr_fields(
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    target_branch_val,
                    "Draft".to_string(),
                    String::new(),
                    is_github,
                );
                app.open_edit_menu(crate::app::EditMenu {
                    title: format!("Create {}", pr_suffix),
                    fields,
                    initial_fields: std::collections::HashMap::new(),
                    selected_idx: 0,
                    entity_iid: 0,
                    entity_kind: crate::app::EditEntityKind::CreateMr,
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                    workflow_inputs: vec![],
                    cursor_pos: 0,
                    editing: false,
                    desc_scroll: 0,
                });
            } else if keybinding_matches(&app.config.keybindings.mrs.select_mr, key_event) {
                if let Some(selected_idx) = app.mrs.state.selected() {
                    let iid = app.filtered_mrs().get(selected_idx).map(|m| m.iid);
                    if let Some(iid) = iid {
                        if app.selected_mrs.contains(&iid) {
                            app.selected_mrs.remove(&iid);
                        } else {
                            app.selected_mrs.insert(iid);
                        }
                    }
                }
            } else if keybinding_matches(&app.config.keybindings.mrs.selection_toggle, key_event) {
                app.select_mode = !app.select_mode;
                if app.select_mode {
                    mark_current_selected(app);
                }
            } else if keybinding_matches(&app.config.keybindings.mrs.edit_entity, key_event) {
                if app.selected_mrs.len() > 1 {
                    let count = app.selected_mrs.len();
                    let pr_suffix = if app.is_github() { "PR" } else { "MR" };
                    app.open_edit_menu(crate::app::EditMenu {
                        title: format!("Bulk Edit {} {}s", count, pr_suffix),
                        fields: vec![
                            crate::app::Field::multi_select("Assignees", String::new()),
                            crate::app::Field::multi_select("Milestone", String::new()),
                            crate::app::Field::multi_select("Labels", String::new()),
                        ],
                        // Bulk-edit forms start empty; `open_edit_menu` will snapshot all
                        // blank values as the baseline. The `MrUpdate::is_empty()` guard
                        // in the dispatcher handles the true no-op case (nothing filled in).
                        initial_fields: std::collections::HashMap::new(),
                        selected_idx: 0,
                        entity_iid: 0,
                        entity_kind: crate::app::EditEntityKind::BulkEditMrs,
                        state: {
                            let mut s = ListState::default();
                            s.select(Some(0));
                            s
                        },
                        workflow_inputs: vec![],
                        cursor_pos: 0,
                        editing: false,
                        desc_scroll: 0,
                    });
                } else if let Some(selected_idx) = app.mrs.state.selected() {
                    let filtered = app.filtered_mrs();
                    if let Some(mr) = filtered.get(selected_idx) {
                        let is_github = app.is_github();
                        let pr_suffix = if is_github { "PR" } else { "MR" };
                        let unresolved = if app.diff_view.as_ref().map(|d| d.mr_iid) == Some(mr.iid)
                        {
                            Some(app.unresolved_threads_count())
                        } else {
                            None
                        };
                        let mut doc =
                            crate::entity_editor::build_mr_document(mr, is_github, unresolved);
                        doc.fields.push(crate::app::Field::text(
                            "Description",
                            mr.description.clone().unwrap_or_default(),
                        ));
                        app.open_edit_menu(crate::app::EditMenu {
                            title: format!("Edit {} #{}", pr_suffix, mr.iid),
                            fields: doc.fields,
                            initial_fields: std::collections::HashMap::new(),
                            selected_idx: 0,
                            entity_iid: mr.iid,
                            entity_kind: crate::app::EditEntityKind::EditMr,
                            state: {
                                let mut s = ListState::default();
                                s.select(Some(0));
                                s
                            },
                            workflow_inputs: vec![],
                            cursor_pos: 0,
                            editing: false,
                            desc_scroll: 0,
                        });
                    }
                }
            } else if app.selected_mrs.len() > 1
                && keybinding_matches(&app.config.keybindings.mrs.merge_mr, key_event)
            {
                let iids: Vec<u64> = app.selected_mrs.iter().copied().collect();
                app.submit_dialog = Some(crate::app::SubmitDialog::build(
                    crate::app::ConfirmAction::BulkMergeMrs(iids),
                    app,
                ));
            } else if let Some(selected_idx) = app.mrs.state.selected() {
                let filtered = app.filtered_mrs();
                let mr_ref = filtered.get(selected_idx);
                if let Some(mr) = mr_ref {
                    let mr_iid = mr.iid;
                    let mr_title = mr.title.clone();
                    match key_event.code {
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.approve_mr,
                            key_event,
                        ) =>
                        {
                            if let Some(client) = app.gitlab_client.clone() {
                                let project_path = app.project_context.clone();
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let result = client.approve_mr(&project_path, mr_iid).await;
                                    let _ = tx2.send(Event::CommandCompleted(
                                        crate::app::Tab::MergeRequests,
                                        result.map_err(|e| e.to_string()),
                                    ));
                                });
                            }
                        }
                        _ if (key_event.code == KeyCode::Char('A')
                            || keybinding_matches(
                                &app.config.keybindings.mrs.revoke_mr,
                                key_event,
                            )) =>
                        {
                            let is_github = app
                                .gitlab_client
                                .as_ref()
                                .map(|c| c.is_github)
                                .unwrap_or(false);
                            if is_github {
                                app.error_message =
                                    Some("Revoking approval isn't supported on GitHub".to_string());
                                app.error_message_at = Some(std::time::Instant::now());
                            } else {
                                app.submit_dialog = Some(crate::app::SubmitDialog::build(
                                    crate::app::ConfirmAction::RevokeMr(mr_iid),
                                    app,
                                ));
                            }
                        }
                        _ if (key_event.code == KeyCode::Char('R')
                            || keybinding_matches(
                                &app.config.keybindings.mrs.rebase_mr,
                                key_event,
                            )) =>
                        {
                            use crate::domain::mr_state::{RebaseGate, rebase_gate};
                            match rebase_gate(mr.mergeability.as_ref()) {
                                RebaseGate::Allowed => {
                                    app.submit_dialog = Some(crate::app::SubmitDialog::build(
                                        crate::app::ConfirmAction::RebaseMr(mr_iid),
                                        app,
                                    ));
                                }
                                RebaseGate::ResolveLocally => {
                                    app.show_error(
                                        "Resolve conflicts locally; rebase can't fix them"
                                            .to_string(),
                                    );
                                }
                                RebaseGate::NotNeeded => {
                                    app.show_error("This MR doesn't need a rebase".to_string());
                                }
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.merge_mr,
                            key_event,
                        ) =>
                        {
                            app.submit_dialog = Some(crate::app::SubmitDialog::build(
                                crate::app::ConfirmAction::MergeMr(mr_iid),
                                app,
                            ));
                        }
                        _ if (key_event.code == KeyCode::Char('D')
                            || keybinding_matches(
                                &app.config.keybindings.mrs.view_diff,
                                key_event,
                            )) =>
                        {
                            app.diff_loading = true;
                            let tx = tx.clone();
                            let mr_iid = mr_iid;
                            let client = app.gitlab_client.clone();
                            let project_context = app.project_context.clone();
                            tokio::spawn(async move {
                                let Some(client) = client else {
                                    let _ = tx.send(Event::DiffFetchFailed(
                                        "No backend client available to fetch diff".to_string(),
                                    ));
                                    return;
                                };

                                let (diff_res, comments_res) = tokio::join!(
                                    client.get_mr_diff(&project_context, mr_iid),
                                    client.list_mr_notes(&project_context, mr_iid)
                                );

                                match diff_res {
                                    Ok(raw_diff) => {
                                        let comments = comments_res.unwrap_or_default();
                                        let _ = tx.send(Event::DiffFetched {
                                            mr_iid,
                                            raw_diff,
                                            comments,
                                        });
                                    }
                                    Err(err) => {
                                        let _ = tx.send(Event::DiffFetchFailed(format!(
                                            "Failed to fetch diff: {}",
                                            err
                                        )));
                                    }
                                }
                            });
                        }
                        _ if (key_event.code == KeyCode::Char('P')
                            || keybinding_matches(
                                &app.config.keybindings.mrs.view_related_pipelines,
                                key_event,
                            )) =>
                        {
                            let pipe_id = mr.head_pipeline.as_ref().map(|p| p.id()).or_else(|| {
                                app.pipelines
                                    .items
                                    .iter()
                                    .find(|p| p.ref_branch() == mr.source_branch)
                                    .map(|p| p.id())
                            });
                            app.active_tab = crate::app::Tab::Pipelines;
                            app.pending_pipeline_select = pipe_id;
                            if let Some(client) = &app.gitlab_client {
                                crate::fetch::spawn_refresh_active_tab(
                                    client,
                                    &app.project_context,
                                    crate::app::Tab::Pipelines,
                                    tx.clone(),
                                );
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.open_in_browser,
                            key_event,
                        ) =>
                        {
                            let is_github = app.is_github();
                            let entity = if is_github { "pr" } else { "mr" };
                            let Some(client) = app.gitlab_client.clone() else {
                                return;
                            };
                            let project_path = app.project_context.clone();
                            let tx2 = tx.clone();
                            let iid_str = mr_iid.to_string();
                            let _ = tokio::spawn(async move {
                                let result = client
                                    .open_in_browser(&project_path, entity, &iid_str)
                                    .await;
                                let _ = tx2.send(Event::CommandCompleted(
                                    crate::app::Tab::MergeRequests,
                                    result.map_err(|e| e.to_string()),
                                ));
                            });
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.toggle_draft,
                            key_event,
                        ) =>
                        {
                            let is_draft = app
                                .mrs
                                .items
                                .iter()
                                .find(|m| m.iid == mr_iid)
                                .map(|m| m.draft)
                                .unwrap_or_else(|| {
                                    mr_title.starts_with("Draft:") || mr_title.starts_with("WIP:")
                                });
                            if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == mr_iid) {
                                item.draft = !is_draft;
                            }
                            if let Some(client) = app.gitlab_client.clone() {
                                let project_path = app.project_context.clone();
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let result = client
                                        .toggle_mr_draft(&project_path, mr_iid, !is_draft)
                                        .await;
                                    let _ = tx2.send(Event::CommandCompleted(
                                        crate::app::Tab::MergeRequests,
                                        result.map_err(|e| e.to_string()),
                                    ));
                                });
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.close_entity,
                            key_event,
                        ) =>
                        {
                            app.submit_dialog = Some(crate::app::SubmitDialog::build(
                                crate::app::ConfirmAction::CloseMr(mr_iid),
                                app,
                            ));
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
                                app.submit_dialog = Some(crate::app::SubmitDialog::build(
                                    crate::app::ConfirmAction::DeleteMr(mr_iid),
                                    app,
                                ));
                            } else {
                                app.show_error(
                                    "GitHub does not support deleting pull requests".to_string(),
                                );
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.mrs.reopen_entity,
                            key_event,
                        ) =>
                        {
                            app.submit_dialog = Some(crate::app::SubmitDialog::build(
                                crate::app::ConfirmAction::ReopenMr(mr_iid),
                                app,
                            ));
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
            if keybinding_matches(&app.config.keybindings.pipelines.run_new, key_event) {
                let current_branch =
                    crate::git_helpers::get_current_branch().unwrap_or_else(|| "main".to_string());

                let is_github = app.is_github();
                let mut fields = vec![crate::app::Field::text(
                    "Branch / Ref",
                    current_branch.clone(),
                )];
                if is_github {
                    fields.push(crate::app::Field::text("Workflow File", String::new()));
                } else {
                    fields.push(crate::app::Field::toggle(
                        "Merge Request Pipeline",
                        "No".to_string(),
                    ));
                }
                fields.push(crate::app::Field::text("Inputs", String::new()));
                fields.push(crate::app::Field::text("Variables", String::new()));

                app.open_edit_menu(crate::app::EditMenu {
                    title: "Run Pipeline".to_string(),
                    fields,
                    initial_fields: std::collections::HashMap::new(),
                    selected_idx: 0,
                    entity_iid: 0,
                    entity_kind: crate::app::EditEntityKind::CreatePipeline,
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                    workflow_inputs: vec![],
                    cursor_pos: 0,
                    editing: false,
                    desc_scroll: 0,
                });
            } else if keybinding_matches(
                &app.config.keybindings.pipelines.trigger_pipeline,
                &key_event,
            ) {
                if let Some(client) = app.gitlab_client.clone() {
                    let branch = crate::git_helpers::get_current_branch()
                        .unwrap_or_else(|| "main".to_string());
                    let project_path = app.project_context.clone();
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        let result = client
                            .run_pipeline(&project_path, &branch, false, &vec![], &vec![], "")
                            .await;
                        let _ = tx2.send(Event::CommandCompleted(
                            crate::app::Tab::Pipelines,
                            result.map_err(|e| e.to_string()),
                        ));
                    });
                }
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
                                            p.status = "running".to_string();
                                        }
                                    }
                                    app.selected_pipelines.clear();
                                    tokio::spawn(async move {
                                        for (i, p_id) in pipe_ids.iter().enumerate() {
                                            if i > 0 {
                                                crate::backend::rate_limit::pace_bulk_operation()
                                                    .await;
                                            }
                                            let _ = client_clone
                                                .retry_pipeline(&project_context, *p_id)
                                                .await;
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
                                        p.status = "running".to_string();
                                    }
                                    let tx = tx.clone();
                                    tokio::spawn(async move {
                                        let _ = client_clone
                                            .retry_pipeline(&project_context, pipe_id)
                                            .await;
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
                                p.status = "canceled".to_string();
                            }
                            if let Some(client) = &app.gitlab_client {
                                let client_clone = client.clone();
                                let project_context = app.project_context.clone();
                                let tx = tx.clone();
                                let active_tab = app.active_tab;
                                tokio::spawn(async move {
                                    let _ = client_clone
                                        .cancel_pipeline(&project_context, pipe_id)
                                        .await;
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
                        _ if (key_event.code == KeyCode::Char('W')
                            || keybinding_matches(
                                &app.config.keybindings.pipelines.open_workflow,
                                key_event,
                            )) =>
                        {
                            if !app.is_github() {
                                app.show_error(
                                    "Workflow browser is only available for GitHub Actions"
                                        .to_string(),
                                );
                                return;
                            }
                            let workflow = app
                                .pipelines
                                .items
                                .iter()
                                .find(|pipeline| pipeline.id() == pipe_id)
                                .map(|pipeline| pipeline.name().to_string())
                                .filter(|name| !name.is_empty());
                            let Some(workflow) = workflow else {
                                app.error_message =
                                    Some("Selected pipeline has no workflow name".to_string());
                                return;
                            };
                            let Some(client) = app.gitlab_client.clone() else {
                                return;
                            };
                            let project_context = app.project_context.clone();
                            let tx2 = tx.clone();
                            tokio::spawn(async move {
                                let result = client
                                    .backend
                                    .open_workflow_in_browser(&project_context, &workflow)
                                    .await;
                                let _ = tx2.send(Event::CommandCompleted(
                                    crate::app::Tab::Pipelines,
                                    result.map_err(|e| e.to_string()),
                                ));
                            });
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.pipelines.open_in_browser,
                            key_event,
                        ) =>
                        {
                            let is_github = app.is_github();
                            let Some(client) = app.gitlab_client.clone() else {
                                return;
                            };
                            let project_path = app.project_context.clone();
                            let pid_str = pipe_id.to_string();
                            let tx2 = tx.clone();
                            let _ = tokio::spawn(async move {
                                let result = client
                                    .open_pipeline_in_browser(&project_path, &pid_str)
                                    .await;
                                let _ = tx2.send(Event::CommandCompleted(
                                    crate::app::Tab::Pipelines,
                                    result.map_err(|e| e.to_string()),
                                ));
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
                                            j.status = "running".to_string();
                                        }
                                    }
                                    app.selected_jobs.clear();
                                    tokio::spawn(async move {
                                        for (i, j_id) in job_ids.iter().enumerate() {
                                            if i > 0 {
                                                crate::backend::rate_limit::pace_bulk_operation()
                                                    .await;
                                            }
                                            let _ = client_clone
                                                .retry_job(&project_context, *j_id)
                                                .await;
                                        }
                                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                        if let Ok(jobs) =
                                            crate::domain::pipelines::list_pipeline_jobs(
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
                                        j.status = "running".to_string();
                                    }
                                    tokio::spawn(async move {
                                        let _ =
                                            client_clone.retry_job(&project_context, job_id).await;
                                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                        if let Ok(jobs) =
                                            crate::domain::pipelines::list_pipeline_jobs(
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
                        _ if key_event.code == KeyCode::Char('S')
                            || keybinding_matches(
                                &app.config.keybindings.jobs.start_job,
                                key_event,
                            ) =>
                        {
                            if app.is_github() {
                                app.error_message =
                                    Some("Manual job start is not supported on GitHub".to_string());
                            } else if let Some(client) = &app.gitlab_client {
                                let client_clone = client.clone();
                                let project_context = app.project_context.clone();
                                let pipe_id = app.active_pipeline_id.unwrap_or(0);
                                let tx = tx.clone();

                                if let Some(j) = app.jobs.items.get_mut(idx) {
                                    if j.status == "manual" {
                                        j.status = "running".to_string();
                                    }
                                }
                                tokio::spawn(async move {
                                    let _ = client_clone.start_job(&project_context, job_id).await;
                                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                    if let Ok(jobs) = crate::domain::pipelines::list_pipeline_jobs(
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
                                            j.status = "canceled".to_string();
                                        }
                                    }
                                    app.selected_jobs.clear();
                                    tokio::spawn(async move {
                                        for j_id in &job_ids {
                                            let _ = client_clone
                                                .cancel_job(&project_context, *j_id)
                                                .await;
                                        }
                                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                        if let Ok(jobs) =
                                            crate::domain::pipelines::list_pipeline_jobs(
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
                                        j.status = "canceled".to_string();
                                    }
                                    tokio::spawn(async move {
                                        let _ =
                                            client_clone.cancel_job(&project_context, job_id).await;
                                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                        if let Ok(jobs) =
                                            crate::domain::pipelines::list_pipeline_jobs(
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
                            if let Some(client) = app.gitlab_client.clone() {
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
                                let project_path = app.project_context.clone();
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let result = client
                                        .download_artifact(&project_path, &ref_name, &job_name)
                                        .await;
                                    let _ = tx2.send(Event::CommandCompleted(
                                        crate::app::Tab::Jobs,
                                        result.map_err(|e| e.to_string()),
                                    ));
                                });
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.open_in_browser,
                            key_event,
                        ) =>
                        {
                            let Some(client) = app.gitlab_client.clone() else {
                                return;
                            };
                            let project_path = app.project_context.clone();
                            let jid_str = job_id.to_string();
                            let tx2 = tx.clone();
                            let _ = tokio::spawn(async move {
                                let result =
                                    client.open_job_in_browser(&project_path, &jid_str).await;
                                let _ = tx2.send(Event::CommandCompleted(
                                    crate::app::Tab::Jobs,
                                    result.map_err(|e| e.to_string()),
                                ));
                            });
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
                            let _ = crossterm::terminal::disable_raw_mode();
                            let _ = crossterm::execute!(
                                std::io::stdout(),
                                crossterm::terminal::LeaveAlternateScreen,
                                crossterm::event::DisableMouseCapture
                            );
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
                            let _ = crossterm::terminal::enable_raw_mode();
                            let _ = crossterm::execute!(
                                std::io::stdout(),
                                crossterm::terminal::EnterAlternateScreen,
                                crossterm::event::EnableMouseCapture
                            );
                            let _ = terminal.clear();
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
                                    let res = crate::domain::pipelines::get_job_trace(
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
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.toggle_trace_wrap,
                            key_event,
                        ) =>
                        {
                            app.job_trace_wrap = !app.job_trace_wrap;
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.jobs.toggle_trace_follow,
                            key_event,
                        ) =>
                        {
                            app.job_trace_follow = !app.job_trace_follow;
                            if app.job_trace_follow {
                                app.job_trace_needs_scroll_to_bottom = true;
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
                            if let Some(runner) =
                                app.runners.items.iter_mut().find(|r| r.id == runner_id)
                            {
                                runner.status = "paused".to_string();
                                runner.active = false;
                            }
                            if let Some(client) = app.gitlab_client.clone() {
                                let project_path = app.project_context.clone();
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let result =
                                        client.pause_runner(&project_path, runner_id).await;
                                    let _ = tx2.send(Event::CommandCompleted(
                                        crate::app::Tab::Runners,
                                        result.map_err(|e| e.to_string()),
                                    ));
                                });
                            }
                        }
                        _ if keybinding_matches(
                            &app.config.keybindings.runners.resume,
                            key_event,
                        ) =>
                        {
                            if let Some(runner) =
                                app.runners.items.iter_mut().find(|r| r.id == runner_id)
                            {
                                runner.status = "online".to_string();
                                runner.active = true;
                            }
                            if let Some(client) = app.gitlab_client.clone() {
                                let project_path = app.project_context.clone();
                                let tx2 = tx.clone();
                                tokio::spawn(async move {
                                    let result =
                                        client.resume_runner(&project_path, runner_id).await;
                                    let _ = tx2.send(Event::CommandCompleted(
                                        crate::app::Tab::Runners,
                                        result.map_err(|e| e.to_string()),
                                    ));
                                });
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
                app.open_edit_menu(crate::app::EditMenu {
                    title: "Create Release".to_string(),
                    fields: vec![
                        crate::app::Field::section("Details"),
                        crate::app::Field::ref_field("Tag", String::new()),
                        crate::app::Field::text("Release Name", String::new()),
                        crate::app::Field::section("Release Notes"),
                        crate::app::Field::text("Release Notes", String::new()),
                    ],
                    initial_fields: std::collections::HashMap::new(),
                    selected_idx: 0,
                    entity_iid: 0,
                    entity_kind: crate::app::EditEntityKind::CreateRelease,
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                    workflow_inputs: vec![],
                    cursor_pos: 0,
                    editing: false,
                    desc_scroll: 0,
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
                        app.submit_dialog = Some(crate::app::SubmitDialog::build(
                            crate::app::ConfirmAction::DeleteRelease(release.tag_name.clone()),
                            app,
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
                        let is_github = app.is_github();
                        let Some(client) = app.gitlab_client.clone() else {
                            return;
                        };
                        let project_path = app.project_context.clone();
                        let tag_name = release.tag_name.clone();
                        let tx2 = tx.clone();

                        tokio::spawn(async move {
                            let result = client
                                .open_in_browser(&project_path, "release", tag_name.as_str())
                                .await;
                            let _ = tx2.send(Event::CommandCompleted(
                                crate::app::Tab::Releases,
                                result.map_err(|e| e.to_string()),
                            ));
                        });
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
                                        crate::domain::notifications::mark_notification_as_read(
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
                            let is_github = app.is_github();
                            let entity = if item.target_type.contains("MergeRequest") {
                                if is_github { "pr" } else { "mr" }
                            } else {
                                "issue"
                            };
                            let Some(client) = app.gitlab_client.clone() else {
                                return;
                            };
                            let project_path = app.project_context.clone();
                            let target_iid = item.target_iid.to_string();
                            let tx2 = tx.clone();
                            let _ = tokio::spawn(async move {
                                let result = client
                                    .open_in_browser(&project_path, &entity, &target_iid)
                                    .await;
                                let _ = tx2.send(Event::CommandCompleted(
                                    crate::app::Tab::Todos,
                                    result.map_err(|e| e.to_string()),
                                ));
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
        crate::app::Tab::Milestones => match key_event.code {
            _ if keybinding_matches(
                &app.config.keybindings.milestones.create_milestone,
                key_event,
            ) =>
            {
                let is_github = app.is_github();
                let fields = crate::entity_editor::milestone_fields(
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    is_github,
                );
                app.open_edit_menu(crate::app::EditMenu {
                    title: "Create Milestone".to_string(),
                    fields,
                    initial_fields: std::collections::HashMap::new(),
                    selected_idx: 0,
                    entity_iid: 0,
                    entity_kind: crate::app::EditEntityKind::CreateMilestone,
                    state: {
                        let mut s = ListState::default();
                        s.select(Some(0));
                        s
                    },
                    workflow_inputs: vec![],
                    cursor_pos: 0,
                    editing: false,
                    desc_scroll: 0,
                });
            }
            _ if keybinding_matches(
                &app.config.keybindings.milestones.edit_milestone,
                key_event,
            ) =>
            {
                if let Some(selected_idx) = app.milestones.state.selected() {
                    let is_github = app.is_github();
                    let milestone_opt: Option<crate::domain::milestones::Milestone> = app
                        .filtered_milestones()
                        .get(selected_idx)
                        .map(|m| (*m).clone());
                    if let Some(m) = milestone_opt {
                        let issues: Option<Vec<crate::domain::issues::Issue>> = app
                            .selected_milestone_issues
                            .clone()
                            .or_else(|| app.milestone_issues_cache.get(&m.iid).cloned());
                        let issues_ref: Option<&[crate::domain::issues::Issue]> = issues.as_deref();
                        let mut doc = crate::entity_editor::build_milestone_document(
                            &m, issues_ref, is_github,
                        );
                        doc.fields.push(crate::app::Field::text(
                            "Description",
                            m.description.clone().unwrap_or_default(),
                        ));
                        app.open_edit_menu(crate::app::EditMenu {
                            title: format!("Edit Milestone %{}", m.iid),
                            fields: doc.fields,
                            initial_fields: std::collections::HashMap::new(),
                            selected_idx: 0,
                            entity_iid: m.iid,
                            entity_kind: crate::app::EditEntityKind::EditMilestone,
                            state: {
                                let mut s = ratatui::widgets::ListState::default();
                                s.select(Some(0));
                                s
                            },
                            workflow_inputs: vec![],
                            cursor_pos: 0,
                            editing: false,
                            desc_scroll: 0,
                        });
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
                        app.submit_dialog = Some(crate::app::SubmitDialog::build(
                            crate::app::ConfirmAction::CloseMilestone(milestone.iid),
                            app,
                        ));
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
                        app.submit_dialog = Some(crate::app::SubmitDialog::build(
                            crate::app::ConfirmAction::ReopenMilestone(milestone.iid),
                            app,
                        ));
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
                        app.submit_dialog = Some(crate::app::SubmitDialog::build(
                            crate::app::ConfirmAction::DeleteMilestone(milestone.iid),
                            app,
                        ));
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
                        let is_github = app.is_github();
                        let Some(client) = app.gitlab_client.clone() else {
                            return;
                        };
                        let project_path = app.project_context.clone();
                        let mid_str = milestone.iid.to_string();
                        let tx2 = tx.clone();

                        tokio::spawn(async move {
                            let result = client
                                .open_milestone_in_browser(&project_path, &mid_str)
                                .await;
                            let _ = tx2.send(Event::CommandCompleted(
                                crate::app::Tab::Milestones,
                                result.map_err(|e| e.to_string()),
                            ));
                        });
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
                        let create_from = branch_name.clone();
                        let fields =
                            crate::entity_editor::branch_fields(String::new(), create_from);
                        app.open_edit_menu(crate::app::EditMenu {
                            title: "Create Branch".to_string(),
                            fields,
                            initial_fields: std::collections::HashMap::new(),
                            selected_idx: 0,
                            entity_iid: 0,
                            entity_kind: crate::app::EditEntityKind::CreateBranch,
                            state: {
                                let mut s = ListState::default();
                                s.select(Some(0));
                                s
                            },
                            workflow_inputs: vec![],
                            cursor_pos: 0,
                            editing: false,
                            desc_scroll: 0,
                        });
                    } else if keybinding_matches(
                        &app.config.keybindings.branches.delete_branch,
                        key_event,
                    ) {
                        app.submit_dialog = Some(crate::app::SubmitDialog::build(
                            crate::app::ConfirmAction::DeleteBranch(branch_name.clone()),
                            app,
                        ));
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
                                match crate::domain::deployments::list_deployments(
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
            if keybinding_matches(&app.config.keybindings.terminal.toggle_wrap, &key_event) {
                app.terminal_wrap = !app.terminal_wrap;
                app.terminal_scroll = 0;
            } else {
                handled = false;
            }
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
            KeyCode::Char('J') if app.detail_visible => {
                app.detail_scroll = app.detail_scroll.saturating_add(1);
            }
            KeyCode::Char('K') if app.detail_visible => {
                app.detail_scroll = app.detail_scroll.saturating_sub(1);
            }
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
            KeyCode::Esc | KeyCode::Backspace => {
                let has_selections = !app.selected_issues.is_empty()
                    || !app.selected_mrs.is_empty()
                    || !app.selected_pipelines.is_empty()
                    || !app.selected_jobs.is_empty();
                if has_selections {
                    app.selected_issues.clear();
                    app.selected_mrs.clear();
                    app.selected_pipelines.clear();
                    app.selected_jobs.clear();
                    app.select_mode = false;
                } else if app.job_trace_loading {
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
                                        crate::domain::notifications::mark_notification_as_read(
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
                                if let Ok(jobs) = crate::domain::pipelines::list_pipeline_jobs(
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
                                    app.show_error("Failed to fetch jobs".to_string());
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
                                    let res = crate::domain::pipelines::get_job_trace(
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
                    } else if !app.details_zoomed {
                        app.details_zoomed = true;
                    } else {
                        // Already zoomed in! Second Enter enters edit mode
                        match app.active_tab {
                            crate::app::Tab::Issues => {
                                if let Some(selected_idx) = app.issues.state.selected() {
                                    let filtered = app.filtered_issues();
                                    if let Some(issue) = filtered.get(selected_idx) {
                                        let is_github = app.is_github();
                                        let mut doc = crate::entity_editor::build_issue_document(
                                            issue, is_github,
                                        );
                                        doc.fields.push(crate::app::Field::text(
                                            "Description",
                                            issue.description.clone().unwrap_or_default(),
                                        ));
                                        app.open_edit_menu(crate::app::EditMenu {
                                            title: format!("Edit Issue #{}", issue.iid),
                                            fields: doc.fields,
                                            initial_fields: std::collections::HashMap::new(),
                                            selected_idx: 0,
                                            entity_iid: issue.iid,
                                            entity_kind: crate::app::EditEntityKind::EditIssue,
                                            state: {
                                                let mut s = ListState::default();
                                                s.select(Some(0));
                                                s
                                            },
                                            workflow_inputs: vec![],
                                            cursor_pos: 0,
                                            editing: false,
                                            desc_scroll: 0,
                                        });
                                    }
                                }
                            }
                            crate::app::Tab::MergeRequests => {
                                if let Some(selected_idx) = app.mrs.state.selected() {
                                    let filtered = app.filtered_mrs();
                                    if let Some(mr) = filtered.get(selected_idx) {
                                        let is_github = app.is_github();
                                        let pr_suffix = if is_github { "PR" } else { "MR" };
                                        let unresolved = if app.diff_view.as_ref().map(|d| d.mr_iid)
                                            == Some(mr.iid)
                                        {
                                            Some(app.unresolved_threads_count())
                                        } else {
                                            None
                                        };
                                        let mut doc = crate::entity_editor::build_mr_document(
                                            mr, is_github, unresolved,
                                        );
                                        doc.fields.push(crate::app::Field::text(
                                            "Description",
                                            mr.description.clone().unwrap_or_default(),
                                        ));
                                        app.open_edit_menu(crate::app::EditMenu {
                                            title: format!("Edit {} #{}", pr_suffix, mr.iid),
                                            fields: doc.fields,
                                            initial_fields: std::collections::HashMap::new(),
                                            selected_idx: 0,
                                            entity_iid: mr.iid,
                                            entity_kind: crate::app::EditEntityKind::EditMr,
                                            state: {
                                                let mut s = ListState::default();
                                                s.select(Some(0));
                                                s
                                            },
                                            workflow_inputs: vec![],
                                            cursor_pos: 0,
                                            editing: false,
                                            desc_scroll: 0,
                                        });
                                    }
                                }
                            }
                            crate::app::Tab::Milestones => {
                                if let Some(selected_idx) = app.milestones.state.selected() {
                                    let filtered = app.filtered_milestones();
                                    if let Some(m) = filtered.get(selected_idx) {
                                        let is_github = app.is_github();
                                        let issues = app
                                            .selected_milestone_issues
                                            .as_deref()
                                            .or_else(|| {
                                                app.milestone_issues_cache
                                                    .get(&m.iid)
                                                    .map(|v| v.as_slice())
                                            });
                                        let mut doc =
                                            crate::entity_editor::build_milestone_document(
                                                m, issues, is_github,
                                            );
                                        doc.fields.push(crate::app::Field::text(
                                            "Description",
                                            m.description.clone().unwrap_or_default(),
                                        ));
                                        app.open_edit_menu(crate::app::EditMenu {
                                            title: format!("Edit Milestone %{}", m.iid),
                                            fields: doc.fields,
                                            initial_fields: std::collections::HashMap::new(),
                                            selected_idx: 0,
                                            entity_iid: m.iid,
                                            entity_kind: crate::app::EditEntityKind::EditMilestone,
                                            state: {
                                                let mut s = ListState::default();
                                                s.select(Some(0));
                                                s
                                            },
                                            workflow_inputs: vec![],
                                            cursor_pos: 0,
                                            editing: false,
                                            desc_scroll: 0,
                                        });
                                    }
                                }
                            }
                            crate::app::Tab::Releases => {
                                if let Some(selected_idx) = app.releases.state.selected() {
                                    let filtered = app.filtered_releases();
                                    if let Some(release) = filtered.get(selected_idx) {
                                        let mut doc =
                                            crate::entity_editor::build_release_document(release);
                                        doc.fields.push(crate::app::Field::text(
                                            "Description",
                                            release.description.clone().unwrap_or_default(),
                                        ));
                                        app.open_edit_menu(crate::app::EditMenu {
                                            title: format!("Edit Release {}", release.tag_name),
                                            fields: doc.fields,
                                            initial_fields: std::collections::HashMap::new(),
                                            selected_idx: 0,
                                            entity_iid: 0,
                                            entity_kind: crate::app::EditEntityKind::EditRelease,
                                            state: {
                                                let mut s = ListState::default();
                                                s.select(Some(0));
                                                s
                                            },
                                            workflow_inputs: vec![],
                                            cursor_pos: 0,
                                            editing: false,
                                            desc_scroll: 0,
                                        });
                                    }
                                }
                            }
                            _ => {
                                app.details_zoomed = false;
                            }
                        }
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
                            app.job_trace_follow = false;
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
                    if app.select_mode {
                        mark_current_selected(app);
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
                            app.job_trace_follow = false;
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
                    if app.select_mode {
                        mark_current_selected(app);
                    }
                }
            }
            _ => {}
        }
    }
}
