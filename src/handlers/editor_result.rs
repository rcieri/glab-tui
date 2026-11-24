use crate::app::{App, FieldType};
use crate::domain::mr::DiscussionNote;
use crate::effect::{EditPurpose, Effects};
use crate::event::Event;
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
            if let Some(text_input) = app.text_input_mut() {
                text_input.buffer.set_value(text);
            }
        }
        EditPurpose::DialogField { path } => {
            if let Some(dialog) = app.submit_dialog_mut() {
                dialog.form.apply_editor_text(path, &text);
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

/// Replaces the body of a pushed comment.
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
    let project = app.project_path_for_open_mr(mr_iid);
    let _ = tx.send(Event::CommandStarted(format!(
        "Editing comment MR #{mr_iid}"
    )));
    crate::handlers::diff_view::spawn_mr_review_task(fx, tx, async move {
        client.edit_comment(&project, mr_iid, &comment, &body).await
    });
}
