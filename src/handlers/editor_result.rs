use crate::app::{App, FieldType, Tab};
use crate::domain::mr::DiscussionNote;
use crate::effect::{EditPurpose, Effects};
use crate::event::{CommandOutcome, Event};
use crate::handlers::diff_view::add_diff_comment;
use tokio::sync::mpsc::UnboundedSender;

/// Applies the text an editor saved for `purpose`. `None` (the editor failed
/// or saved nothing) leaves everything as it was.
pub fn apply(
    app: &mut App,
    purpose: EditPurpose,
    text: Option<String>,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    let Some(text) = text else {
        return;
    };
    match purpose {
        EditPurpose::TextInput => {
            if let Some(buffer) = app.text_input_mut().and_then(|t| t.buffer_mut()) {
                buffer.set_value(text);
            }
        }
        EditPurpose::ReviewSummary => {
            if let Some(overlay) = app.review_submit_mut() {
                overlay.summary.set_value(text);
                overlay.clear_outcome();
                overlay.focus = crate::review_submit::ReviewSubmitFocus::Summary;
            }
        }
        EditPurpose::MenuDescription => {
            let description = app.edit_menu_mut().and_then(|menu| {
                menu.fields.iter_mut().find(|f| {
                    (f.label == "Description" || f.label == "Release Notes")
                        && f.kind == FieldType::Text
                })
            });
            if let Some(field) = description {
                field.value = text;
            }
        }
        EditPurpose::DraftBody { index } => {
            let Some(draft) = app.draft_comments.get_mut(index) else {
                return;
            };
            draft.body = text;
            if let Some((project, mr_iid)) =
                app.diff_view().map(|d| (d.project_path.clone(), d.mr_iid))
            {
                app.persist_draft_comments(&project, mr_iid);
            }
        }
        EditPurpose::CommentBody { mr_iid, comment } => {
            if text != comment.body {
                spawn_comment_edit(app, mr_iid, *comment, text, tx, fx);
            }
        }
        EditPurpose::DiffComment { mut draft } => {
            draft.body = text;
            add_diff_comment(app, draft, "draft comment", tx, fx);
        }
        EditPurpose::DiffSuggestion { mut draft } => {
            draft.body = format!("```suggestion\n{text}\n```");
            add_diff_comment(app, draft, "suggestion draft", tx, fx);
        }
    }
}

/// Replaces the body of a pushed comment. Neither `gh` nor `glab` has a
/// subcommand to edit a review comment, so this goes through their `api`.
fn spawn_comment_edit(
    app: &App,
    mr_iid: u64,
    comment: DiscussionNote,
    body: String,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    let Some(client) = app.gitlab_client.clone() else {
        return;
    };
    let is_github = client.is_github;
    let project_context = app.project_path_for_open_mr(mr_iid);
    let Some(comment_id) = comment.id else {
        return;
    };
    let _ = tx.send(Event::CommandStarted(format!(
        "Editing comment MR #{mr_iid}"
    )));
    let tx = tx.clone();
    fx.spawn(async move {
        let (program, endpoint, method) = if is_github {
            let kind = if comment.position.is_some() {
                "pulls"
            } else {
                "issues"
            };
            (
                "gh",
                format!("repos/{project_context}/{kind}/comments/{comment_id}"),
                "PATCH",
            )
        } else {
            let encoded_path = project_context.replace('/', "%2F");
            (
                "glab",
                format!("projects/{encoded_path}/merge_requests/{mr_iid}/notes/{comment_id}"),
                "PUT",
            )
        };
        let payload = serde_json::json!({ "body": body }).to_string();
        let output = crate::backend::subprocess::output(
            tokio::process::Command::new(program)
                .args(["api", &endpoint, "--input", "-", "-X", method]),
            Some(payload.as_bytes()),
        )
        .await;

        let result = match output {
            Ok(out) if out.status.success() => Ok(CommandOutcome::Mutated),
            Ok(out) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(Event::CommandCompleted(Tab::MergeRequests, result));
    });
}
