use crate::app::App;
use crate::domain::review::{DraftComment, ReviewEvent};
use crate::effect::{EditPurpose, Effects, ForegroundJob};
use crate::entity_editor::{apply_field_text_change, rebuild_edit_menu};
use crate::event::{CommandOutcome, Event};
use crate::fetch::spawn_refresh_active_tab;
use crate::keybinding::KeyPress;
use crate::{app, domain, handlers};
use crossterm::event::KeyCode;
use tokio::sync::mpsc::UnboundedSender;

/// Keys while a text input is open.
pub fn handle_key(
    app: &mut App,
    key_event: &KeyPress,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    let Some(mut text_input) = app.text_input.take() else {
        return;
    };
    if key_event
        .modifiers
        .contains(crossterm::event::KeyModifiers::CONTROL)
        && key_event.code == KeyCode::Char('e')
    {
        fx.suspend(ForegroundJob::Edit {
            text: text_input.value().to_string(),
            file_suffix: ".md".to_string(),
            purpose: EditPurpose::TextInput,
        });
        app.text_input = Some(text_input);
        return;
    }
    match key_event.code {
        KeyCode::Esc => {
            // Cancel
        }
        KeyCode::Backspace => {
            text_input.delete_char_before_cursor();
            app.text_input = Some(text_input);
        }
        KeyCode::Delete => {
            text_input.delete_char_at_cursor();
            app.text_input = Some(text_input);
        }
        KeyCode::Left => {
            text_input.move_cursor_left();
            app.text_input = Some(text_input);
        }
        KeyCode::Right => {
            text_input.move_cursor_right();
            app.text_input = Some(text_input);
        }
        KeyCode::Char(c) => {
            text_input.insert_char(c);
            app.text_input = Some(text_input);
        }
        KeyCode::Enter => {
            let value = text_input.value().to_string();
            match text_input.action {
                crate::app::TextInputAction::EditPageSize => {
                    if let Ok(new_size) = value.trim().parse::<usize>() {
                        if new_size > 0 {
                            app.config.page_size = new_size;
                            app.page_size = new_size;
                            if let Some(ref mut client) = app.gitlab_client {
                                client.page_size = new_size;
                            }
                            if let Some(client) = app.gitlab_client.clone() {
                                app.start_loading_tab(app.active_tab);
                                spawn_refresh_active_tab(
                                    fx,
                                    &client,
                                    &app.scope,
                                    app.requested_state_queries(),
                                    app.active_tab,
                                    app.scoped_sender(tx.clone()),
                                );
                            }
                        }
                    }
                }
                crate::app::TextInputAction::EditField {
                    entity_iid,
                    entity_type,
                    field_type,
                } => {
                    apply_field_text_change(
                        app,
                        &entity_type,
                        entity_iid,
                        &field_type,
                        value,
                        tx.clone(),
                        fx,
                    );
                    rebuild_edit_menu(app, &entity_type, entity_iid);
                }
                crate::app::TextInputAction::CreateIssue => {
                    if !value.trim().is_empty() {
                        let client = app.gitlab_client.clone().unwrap();
                        let project = app.scope.as_str().to_string();
                        let title = value.clone();
                        let tx = tx.clone();
                        let tab = app.active_tab;
                        fx.spawn(async move {
                            match client
                                .create_issue(
                                    &project,
                                    &crate::backend::NewIssue {
                                        title: &title,
                                        ..Default::default()
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
                                    let _ =
                                        tx.send(Event::CommandCompleted(tab, Err(e.to_string())));
                                }
                            }
                        });
                    }
                }
                crate::app::TextInputAction::AddReviewComment {
                    mr_iid,
                    file_path,
                    line_num,
                    old_line_num,
                    end_line_num,
                    end_old_line_num,
                } => {
                    if !value.trim().is_empty() {
                        let comment = DraftComment {
                            file_path,
                            line_num,
                            old_line_num,
                            end_line_num,
                            end_old_line_num,
                            body: value,
                        };
                        if app.in_review_mode {
                            app.draft_comments.push(comment);
                            if let Some(project) =
                                app.diff_view.as_ref().map(|diff| diff.project_path.clone())
                            {
                                app.persist_draft_comments(&project, mr_iid);
                            }
                            app.show_notice(format!(
                                "Added draft comment. ({} pending)",
                                app.draft_comments.len()
                            ));
                        } else {
                            handlers::diff_view::post_review_comment(app, mr_iid, comment, tx, fx);
                        }
                    }
                }
                crate::app::TextInputAction::EnterPipelineId => {
                    if let Ok(pipeline_id) = value.trim().parse::<u64>() {
                        if let Some(client) = &app.gitlab_client {
                            app.loading_tabs.insert(app::Tab::Jobs);
                            let client_clone = client.clone();
                            let project_context = app.project_path_for_pipeline(pipeline_id);
                            app.active_pipeline_project =
                                app.pipeline_project(pipeline_id).map(str::to_owned);
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
                                        let _ = tx.send(Event::JobsTabFetched(
                                            pipeline_id,
                                            jobs,
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = tx.send(Event::FetchFailed(app::Tab::Jobs, format!("Failed to fetch jobs for pipeline {pipeline_id}: {e}")));
                                    }
                                }
                            });
                        }
                    } else {
                        app.error_message = Some("Invalid pipeline ID".to_string());
                    }
                }
                crate::app::TextInputAction::CreateBranch(ref ref_branch) => {
                    if !value.trim().is_empty() {
                        let branch_name = value.trim().to_string();
                        let client = app.gitlab_client.clone();
                        let project_context = app.scope.as_str().to_string();
                        let ref_branch = ref_branch.clone();
                        let tx = tx.clone();
                        let _ = tx.send(Event::CommandStarted(format!(
                            "Creating branch: {branch_name} from {ref_branch}"
                        )));
                        fx.spawn(async move {
                            if let Some(client) = client {
                                match crate::domain::branches::create_branch(
                                    &client,
                                    &project_context,
                                    &branch_name,
                                    &ref_branch,
                                )
                                .await
                                {
                                    Ok(_) => {
                                        let _ = tx.send(Event::CommandCompleted(
                                            app::Tab::Branches,
                                            Ok(CommandOutcome::Mutated),
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = tx.send(Event::CommandCompleted(
                                            app::Tab::Branches,
                                            Err(format!("Failed: {e}")),
                                        ));
                                    }
                                }
                            }
                        });
                    }
                }
                crate::app::TextInputAction::CreateMilestone => {
                    if !value.trim().is_empty() {
                        let title = value.trim().to_string();
                        let client = app.gitlab_client.clone().unwrap();
                        let project = app.scope.as_str().to_string();
                        let tx = tx.clone();
                        let tab = app.active_tab;
                        fx.spawn(async move {
                            match client
                                .create_milestone(&project, &title, "", None, None)
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
                }
                crate::app::TextInputAction::ReplyToComment {
                    mr_iid,
                    comment_id,
                    ref discussion_id,
                } => {
                    if !value.trim().is_empty() {
                        if let Some(client) = app.gitlab_client.clone() {
                            let project = app.project_path_for_open_mr(mr_iid);
                            let thread_id = discussion_id.clone();
                            let body = value.clone();
                            let tx = tx.clone();
                            let _ = tx.send(Event::CommandStarted(format!(
                                "Replying to comment ID {comment_id} in MR #{mr_iid}"
                            )));
                            handlers::diff_view::spawn_mr_review_task(fx, &tx, async move {
                                client
                                    .reply_to_thread(&project, mr_iid, &thread_id, &body)
                                    .await
                            });
                        }
                    }
                }
                crate::app::TextInputAction::SubmitReviewFinal { mr_iid, status } => {
                    let comments = std::mem::take(&mut app.draft_comments);
                    let project = app.project_path_for_open_mr(mr_iid);
                    app.persist_draft_comments(&project, mr_iid);
                    app.in_review_mode = false;
                    if let Some(client) = app.gitlab_client.clone() {
                        let event = ReviewEvent::from_label(&status);
                        let body = value.clone();
                        let tx = tx.clone();
                        let _ = tx.send(Event::CommandStarted(format!(
                            "Submitting review on MR #{mr_iid}"
                        )));
                        handlers::diff_view::spawn_mr_review_task(fx, &tx, async move {
                            client
                                .submit_review(&project, mr_iid, event, &body, &comments)
                                .await
                        });
                    }
                }
                crate::app::TextInputAction::EditNewField { field_idx } => {
                    // Write the value directly into the edit_menu fields
                    // (no CLI call — iid==0 means this entity is not yet created)
                    if let Some(ref mut menu) = app.edit_menu {
                        if let Some(field) = menu.fields.get_mut(field_idx) {
                            field.value = value.clone();
                        }
                    }
                }
            }
        }
        _ => {
            app.text_input = Some(text_input);
        }
    }
}
