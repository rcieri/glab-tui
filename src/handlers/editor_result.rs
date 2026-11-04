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
            if let Some(text_input) = &mut app.text_input {
                text_input.set_value(text);
            }
        }
        EditPurpose::MenuDescription => {
            let description = app.edit_menu.as_mut().and_then(|menu| {
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
            if let Some((project, mr_iid)) = app
                .diff_view
                .as_ref()
                .map(|d| (d.project_path.clone(), d.mr_iid))
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
    let comment_id = comment.id;
    let _ = tx.send(Event::CommandStarted(format!(
        "Editing comment MR #{mr_iid}"
    )));
    let tx = tx.clone();
    fx.spawn(async move {
        let output = if is_github {
            let endpoint = if comment.position.is_some() {
                format!("repos/{project_context}/pulls/comments/{comment_id}")
            } else {
                format!("repos/{project_context}/issues/comments/{comment_id}")
            };
            let payload = serde_json::json!({ "body": body });
            let temp_path = std::env::temp_dir().join(format!("glab-tui-edit-{comment_id}.json"));
            let _ = std::fs::write(&temp_path, payload.to_string());
            let temp_str = temp_path.to_string_lossy().to_string();

            let res = crate::backend::subprocess::output(
                tokio::process::Command::new("gh")
                    .args(["api", &endpoint, "--input", &temp_str, "-X", "PATCH"]),
                None,
            )
            .await;
            let _ = std::fs::remove_file(&temp_path);
            res
        } else {
            let encoded_path = project_context.replace("/", "%2F");
            let payload = serde_json::json!({ "body": body });
            let temp_path = std::env::temp_dir().join(format!("glab-tui-edit-{comment_id}.json"));
            let _ = std::fs::write(&temp_path, payload.to_string());
            let temp_str = temp_path.to_string_lossy().to_string();

            let res = crate::backend::subprocess::output(
                tokio::process::Command::new("glab").args([
                    "api",
                    &format!("projects/{encoded_path}/merge_requests/{mr_iid}/notes/{comment_id}"),
                    "--input",
                    &temp_str,
                    "-X",
                    "PUT",
                ]),
                None,
            )
            .await;
            let _ = std::fs::remove_file(&temp_path);
            res
        };

        let result = match output {
            Ok(out) if out.status.success() => Ok(CommandOutcome::Mutated),
            Ok(out) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(Event::CommandCompleted(Tab::MergeRequests, result));
    });
}
