//! Keys, clicks and the background submission of the submit-review overlay.

use crate::app::App;
use crate::event::Event;
use crate::review_submit::{ReviewSubmit, ReviewSubmitFocus, SubmissionState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use tokio::sync::mpsc::UnboundedSender;

/// Opens the overlay for the drafts of MR/PR `mr_iid` in `project_path`.
pub fn open(app: &mut App, mr_iid: u64, project_path: String) {
    app.review_submit = Some(ReviewSubmit::new(mr_iid, project_path));
}

/// Keys of the open overlay, which takes every key until it closes. While a
/// submission runs, keys are swallowed: the result decides whether the
/// drafts are gone.
pub fn handle_key(app: &mut App, key_event: &KeyEvent, tx: UnboundedSender<Event>) -> bool {
    let Some(mut overlay) = app.review_submit.take() else {
        return false;
    };
    if overlay.is_submitting() {
        app.review_submit = Some(overlay);
        return true;
    }
    match key_event.code {
        KeyCode::Esc => return true,
        KeyCode::Tab => overlay.focus = overlay.focus.cycled(1),
        KeyCode::BackTab => overlay.focus = overlay.focus.cycled(-1),
        KeyCode::Enter if overlay.focus == ReviewSubmitFocus::CancelButton => return true,
        KeyCode::Enter => {
            app.review_submit = Some(overlay);
            submit(app, tx);
            return true;
        }
        code => move_or_edit(&mut overlay, code, key_event.modifiers),
    }
    app.review_submit = Some(overlay);
    true
}

fn move_or_edit(overlay: &mut ReviewSubmit, code: KeyCode, modifiers: KeyModifiers) {
    match overlay.focus {
        ReviewSubmitFocus::Verdicts => match code {
            KeyCode::Char('j') | KeyCode::Down => overlay.verdict = overlay.verdict.stepped(1),
            KeyCode::Char('k') | KeyCode::Up => overlay.verdict = overlay.verdict.stepped(-1),
            _ => {}
        },
        ReviewSubmitFocus::Summary => match code {
            KeyCode::Up => overlay.focus = ReviewSubmitFocus::Verdicts,
            KeyCode::Down => overlay.focus = ReviewSubmitFocus::SubmitButton,
            _ if modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {}
            _ => {
                overlay.summary.apply_edit_key(code);
            }
        },
        ReviewSubmitFocus::SubmitButton | ReviewSubmitFocus::CancelButton => match code {
            KeyCode::Char('h') | KeyCode::Left => overlay.focus = ReviewSubmitFocus::SubmitButton,
            KeyCode::Char('l') | KeyCode::Right => overlay.focus = ReviewSubmitFocus::CancelButton,
            KeyCode::Char('k') | KeyCode::Up => overlay.focus = ReviewSubmitFocus::Summary,
            _ => {}
        },
    }
}

/// A click on the overlay: a verdict row selects it, the summary takes the
/// focus and a button acts like `Enter` on it.
pub fn click(app: &mut App, row: u16, col: u16) {
    let Some(overlay) = app.review_submit.as_mut() else {
        return;
    };
    let Some(layout) = overlay.layout else {
        return;
    };
    if overlay.is_submitting() {
        return;
    }
    let contains = |rect: Rect| rect.contains((col, row).into());
    if contains(layout.verdicts) {
        let index = usize::from(row - layout.verdicts.y);
        if let Some(&verdict) = crate::review_submit::ReviewVerdict::ALL.get(index) {
            overlay.verdict = verdict;
            overlay.focus = ReviewSubmitFocus::Verdicts;
        }
    } else if contains(layout.summary) {
        overlay.focus = ReviewSubmitFocus::Summary;
    } else if contains(layout.cancel_button) {
        app.review_submit = None;
    } else if contains(layout.submit_button) {
        overlay.focus = ReviewSubmitFocus::SubmitButton;
        if let Some(tx) = app.tx.clone() {
            submit(app, tx);
        }
    }
}

/// Posts the review the overlay describes, or keeps the drafts. The overlay
/// stays open while the review is posted and shows why it was refused or
/// failed.
fn submit(app: &mut App, tx: UnboundedSender<Event>) {
    let draft_count = app.draft_comments.len();
    let Some(overlay) = app.review_submit.as_mut() else {
        return;
    };
    let event = match overlay.event_to_post(draft_count) {
        Ok(Some(event)) => event,
        Ok(None) => {
            app.review_submit = None;
            let noun = if draft_count == 1 {
                "comment"
            } else {
                "comments"
            };
            app.show_notice(format!("Kept {draft_count} draft {noun} for later"));
            return;
        }
        Err(reason) => {
            overlay.state = SubmissionState::Failed(reason);
            return;
        }
    };
    let Some(client) = app.gitlab_client.clone() else {
        overlay.state = SubmissionState::Failed("No active backend to submit the review.".into());
        return;
    };
    let mr_iid = overlay.mr_iid;
    let project = overlay.project_path.clone();
    let body = overlay.summary.value().to_string();
    let comments = app.draft_comments.clone();
    let mr_short = app.kind().term("mr_short");
    let log_row = app.start_command(format!("Submitting review on {mr_short} #{mr_iid}"));
    if let Some(overlay) = app.review_submit.as_mut() {
        overlay.state = SubmissionState::Submitting { log_row };
    }
    tokio::spawn(async move {
        let result = client
            .submit_review(&project, mr_iid, event, &body, &comments)
            .await
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(Event::ReviewSubmitted { mr_iid, result });
    });
}

/// Applies the outcome of a submission. A posted review clears the drafts it
/// carried and closes the overlay. A failed one keeps both and shows the
/// error in the overlay. Returns whether a review was posted.
pub fn finish(app: &mut App, mr_iid: u64, result: Result<(), String>) -> bool {
    let Some(mut overlay) = app
        .review_submit
        .take_if(|overlay| overlay.mr_iid == mr_iid && overlay.is_submitting())
    else {
        return false;
    };
    if let SubmissionState::Submitting { log_row } = overlay.state {
        app.finish_command(log_row, &result);
    }
    match result {
        Ok(()) => {
            app.draft_comments.clear();
            app.persist_draft_comments(&overlay.project_path, mr_iid);
            app.in_review_mode = false;
            let mr_short = app.kind().term("mr_short");
            app.show_notice(format!("Review submitted on {mr_short} #{mr_iid}"));
            true
        }
        Err(error) => {
            overlay.state = SubmissionState::Failed(error);
            app.review_submit = Some(overlay);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::review::DraftComment;
    use crate::review_submit::ReviewVerdict;

    const PROJECT: &str = "acme/widget";

    fn press(app: &mut App, code: KeyCode) -> bool {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        handle_key(app, &KeyEvent::new(code, KeyModifiers::NONE), tx)
    }

    fn draft(body: &str) -> DraftComment {
        DraftComment {
            file_path: "src/lib.rs".to_string(),
            line_num: Some(3),
            old_line_num: None,
            end_line_num: None,
            end_old_line_num: None,
            body: body.to_string(),
        }
    }

    fn submitting_app() -> App {
        let mut app = App::default();
        app.draft_comments = vec![draft("nit")];
        app.in_review_mode = true;
        open(&mut app, 7, PROJECT.to_string());
        let log_row = app.start_command("Submitting review on MR #7".to_string());
        if let Some(overlay) = app.review_submit.as_mut() {
            overlay.summary.set_value("Looks good".to_string());
            overlay.state = SubmissionState::Submitting { log_row };
        }
        app
    }

    #[test]
    fn typing_in_the_summary_does_not_move_the_verdict_or_close_the_overlay() {
        let mut app = App::default();
        open(&mut app, 7, PROJECT.to_string());
        for code in [KeyCode::Tab, KeyCode::Char('j'), KeyCode::Char('q')] {
            assert!(press(&mut app, code));
        }
        let overlay = app.review_submit.as_ref().expect("still open");
        assert_eq!(overlay.summary.value(), "jq");
        assert_eq!(overlay.verdict, ReviewVerdict::Comment);
    }

    #[test]
    fn esc_closes_the_overlay_and_leaves_the_drafts_alone() {
        let mut app = App::default();
        app.draft_comments = vec![draft("nit")];
        open(&mut app, 7, PROJECT.to_string());

        assert!(press(&mut app, KeyCode::Esc));

        assert!(app.review_submit.is_none());
        assert_eq!(app.draft_comments.len(), 1);
    }

    #[test]
    fn an_empty_comment_review_is_refused_inside_the_overlay() {
        let mut app = App::default();
        open(&mut app, 7, PROJECT.to_string());

        assert!(press(&mut app, KeyCode::Enter));

        let overlay = app.review_submit.as_ref().expect("stays open");
        assert_eq!(
            overlay.state,
            SubmissionState::Failed("A comment review needs a summary or draft comments.".into())
        );
    }

    #[test]
    fn keep_as_draft_closes_the_overlay_without_touching_the_drafts() {
        let mut app = App::default();
        app.draft_comments = vec![draft("nit")];
        app.in_review_mode = true;
        open(&mut app, 7, PROJECT.to_string());
        for code in [KeyCode::Char('k'), KeyCode::Enter] {
            press(&mut app, code);
        }

        assert!(app.review_submit.is_none());
        assert_eq!(app.draft_comments.len(), 1);
        assert!(app.in_review_mode);
    }

    #[test]
    fn keys_are_swallowed_while_the_review_is_posted() {
        let mut app = submitting_app();

        for code in [KeyCode::Esc, KeyCode::Enter, KeyCode::Char('x')] {
            assert!(press(&mut app, code));
        }

        let overlay = app.review_submit.as_ref().expect("stays open");
        assert!(overlay.is_submitting());
        assert_eq!(overlay.summary.value(), "Looks good");
    }

    #[test]
    fn a_failed_submission_keeps_the_drafts_and_shows_the_error_inline() {
        let mut app = submitting_app();

        let posted = finish(&mut app, 7, Err("HTTP 422: body required".to_string()));

        assert!(!posted);
        assert_eq!(app.draft_comments.len(), 1);
        assert!(app.in_review_mode);
        assert_eq!(app.error_message, None, "the error is not only a toast");
        let overlay = app.review_submit.as_ref().expect("stays open");
        assert_eq!(
            overlay.state,
            SubmissionState::Failed("HTTP 422: body required".into())
        );
        assert_eq!(overlay.summary.value(), "Looks good");
        assert_eq!(
            app.terminal_commands
                .iter()
                .last()
                .map(|entry| entry.status.as_str()),
            Some("Failed: HTTP 422: body required")
        );
    }

    #[test]
    fn a_posted_review_clears_its_drafts_and_their_saved_copy_and_closes_the_overlay() {
        let _home = crate::config::isolate_home();
        let mut app = submitting_app();
        app.persist_draft_comments(PROJECT, 7);

        let posted = finish(&mut app, 7, Ok(()));

        assert!(posted);
        assert!(app.review_submit.is_none());
        assert!(app.draft_comments.is_empty());
        assert!(!app.in_review_mode);
        assert_eq!(
            app.terminal_commands
                .iter()
                .last()
                .map(|entry| entry.status.as_str()),
            Some("Success")
        );
        assert!(
            crate::utils::cache::load_cache(app.scope.as_str())
                .draft_comments
                .is_empty(),
            "the posted drafts must not come back when the diff reopens"
        );
    }

    #[test]
    fn a_result_for_another_mr_is_ignored() {
        let mut app = submitting_app();

        assert!(!finish(&mut app, 8, Ok(())));

        assert!(
            app.review_submit
                .as_ref()
                .is_some_and(|o| o.is_submitting())
        );
        assert_eq!(app.draft_comments.len(), 1);
    }
}
