//! Keys, clicks and the background submission of the submit-review overlay.

use crate::app::{App, REVIEW_SUBMIT_LOG_PREFIX};
use crate::effect::{EditPurpose, Effects, ForegroundJob};
use crate::event::Event;
use crate::keybinding::KeyPress;
use crate::review_submit::{ReviewSubmit, ReviewSubmitFocus, ReviewVerdict, SubmissionState};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;
use tokio::sync::mpsc::UnboundedSender;

pub fn open(app: &mut App, mr_iid: u64, project_path: String) {
    app.open_review_submit(ReviewSubmit::new(mr_iid, project_path));
}

/// Keys of the open overlay, which takes every key until it closes. While a
/// submission runs, keys are swallowed: the result decides whether the
/// drafts are gone. `Ctrl+E` hands the summary to `$EDITOR`.
pub fn handle_key(
    app: &mut App,
    key_event: &KeyPress,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) -> bool {
    let Some(mut overlay) = app.pop_review_submit() else {
        return false;
    };
    if overlay.is_submitting() {
        app.push_modal(crate::app::Modal::ReviewSubmit(Box::new(overlay)));
        return true;
    }
    match key_event.code {
        KeyCode::Char('e') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            fx.suspend(ForegroundJob::Edit {
                text: overlay.summary.value().to_string(),
                file_suffix: ".md".to_string(),
                purpose: EditPurpose::ReviewSummary,
            });
            overlay.focus = ReviewSubmitFocus::Summary;
            app.push_modal(crate::app::Modal::ReviewSubmit(Box::new(overlay)));
        }
        KeyCode::Esc => {
            app.close_review_submit();
            return true;
        }
        KeyCode::Tab => {
            overlay.focus = overlay.focus.stepped(1);
            app.push_modal(crate::app::Modal::ReviewSubmit(Box::new(overlay)));
        }
        KeyCode::BackTab => {
            overlay.focus = overlay.focus.stepped(-1);
            app.push_modal(crate::app::Modal::ReviewSubmit(Box::new(overlay)));
        }
        KeyCode::Enter if overlay.focus == ReviewSubmitFocus::CancelButton => {
            app.close_review_submit();
            return true;
        }
        KeyCode::Enter => {
            app.push_modal(crate::app::Modal::ReviewSubmit(Box::new(overlay)));
            submit(app, tx, fx);
            return true;
        }
        code => {
            move_or_edit(&mut overlay, code, key_event.modifiers);
            app.push_modal(crate::app::Modal::ReviewSubmit(Box::new(overlay)));
        }
    }
    true
}

fn move_or_edit(overlay: &mut ReviewSubmit, code: KeyCode, modifiers: KeyModifiers) {
    match overlay.focus {
        ReviewSubmitFocus::Verdicts => match code {
            KeyCode::Char('j') | KeyCode::Down => match overlay.verdict.below() {
                Some(verdict) => select_verdict(overlay, verdict),
                None => overlay.focus = ReviewSubmitFocus::Summary,
            },
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(verdict) = overlay.verdict.above() {
                    select_verdict(overlay, verdict);
                }
            }
            _ => {}
        },
        ReviewSubmitFocus::Summary => match code {
            KeyCode::Up => overlay.focus = ReviewSubmitFocus::Verdicts,
            KeyCode::Down => overlay.focus = ReviewSubmitFocus::SubmitButton,
            _ if is_shortcut(code, modifiers) => {}
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete => {
                overlay.summary.apply_edit_key(code);
                overlay.clear_outcome();
            }
            _ => overlay.summary.apply_edit_key(code),
        },
        ReviewSubmitFocus::SubmitButton | ReviewSubmitFocus::CancelButton => match code {
            KeyCode::Char('h') | KeyCode::Left => overlay.focus = ReviewSubmitFocus::SubmitButton,
            KeyCode::Char('l') | KeyCode::Right => overlay.focus = ReviewSubmitFocus::CancelButton,
            KeyCode::Char('k') | KeyCode::Up => overlay.focus = ReviewSubmitFocus::Summary,
            _ => {}
        },
    }
}

/// Windows reports AltGr as Ctrl+Alt with the composed character, so a German
/// `@` or a Nordic `{` arrives as Ctrl+Alt and is text, not a shortcut.
fn is_shortcut(code: KeyCode, modifiers: KeyModifiers) -> bool {
    let is_alt_gr_text = modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT)
        && matches!(code, KeyCode::Char(c) if !c.is_ascii_alphanumeric());
    modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) && !is_alt_gr_text
}

fn select_verdict(overlay: &mut ReviewSubmit, verdict: ReviewVerdict) {
    if overlay.verdict != verdict {
        overlay.verdict = verdict;
        overlay.clear_outcome();
    }
}

/// A click on the overlay: a verdict row selects it, the summary takes the
/// focus and a button acts like `Enter` on it.
pub fn click(app: &mut App, row: u16, col: u16, tx: &UnboundedSender<Event>, fx: &mut Effects) {
    let Some(overlay) = app.review_submit_mut() else {
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
        if let Some(&verdict) = ReviewVerdict::ALL.get(index) {
            select_verdict(overlay, verdict);
            overlay.focus = ReviewSubmitFocus::Verdicts;
        }
    } else if contains(layout.summary) {
        overlay.focus = ReviewSubmitFocus::Summary;
    } else if contains(layout.cancel_button) {
        app.close_review_submit();
    } else if contains(layout.submit_button) {
        overlay.focus = ReviewSubmitFocus::SubmitButton;
        submit(app, tx, fx);
    }
}

/// Posts the review the overlay describes, or keeps the drafts. The overlay
/// stays open while the review is posted and shows why it was refused or
/// failed.
fn submit(app: &mut App, tx: &UnboundedSender<Event>, fx: &mut Effects) {
    let draft_count = app.draft_comments.len();
    let event = match app.review_submit() {
        Some(overlay) => overlay.event_to_post(draft_count),
        None => return,
    };
    let event = match event {
        Ok(Some(event)) => event,
        Ok(None) => {
            app.close_review_submit();
            app.show_notice(app.kept_drafts_notice());
            return;
        }
        Err(reason) => {
            if let Some(overlay) = app.review_submit_mut() {
                overlay.state = SubmissionState::Refused(reason);
            }
            return;
        }
    };
    let Some(client) = app.gitlab_client.clone() else {
        if let Some(overlay) = app.review_submit_mut() {
            overlay.state =
                SubmissionState::Refused("No active backend to submit the review.".into());
        }
        return;
    };
    let (mr_iid, project, body) = match app.review_submit() {
        Some(overlay) => (
            overlay.mr_iid,
            overlay.project_path.clone(),
            overlay.summary.value().to_string(),
        ),
        None => return,
    };
    let comments = app.draft_comments.clone();
    let mr_short = app.kind().term("mr_short");
    let log_row = app.start_command(format!("{REVIEW_SUBMIT_LOG_PREFIX}{mr_short} #{mr_iid}"));
    if let Some(overlay) = app.review_submit_mut() {
        overlay.state = SubmissionState::Submitting {
            log_row,
            drafts: comments.clone(),
        };
    }
    let tx = tx.clone();
    fx.spawn(async move {
        let result = client
            .submit_review(&project, mr_iid, event, &body, &comments)
            .await
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(Event::ReviewSubmitted { mr_iid, result });
    });
}

/// Applies the outcome of a submission. A posted review removes the drafts it
/// carried and closes the overlay. A failed one keeps both and shows the
/// error in the overlay. Returns whether a review was posted.
pub fn finish(app: &mut App, mr_iid: u64, result: Result<(), String>) -> bool {
    let matches = app
        .review_submit()
        .is_some_and(|overlay| overlay.mr_iid == mr_iid && overlay.is_submitting());
    if !matches {
        return false;
    }
    let Some(mut overlay) = app.pop_review_submit() else {
        return false;
    };
    let SubmissionState::Submitting { log_row, drafts } = std::mem::take(&mut overlay.state) else {
        return false;
    };
    app.settle_command(log_row, &result, None);
    match result {
        Ok(()) => {
            app.drop_sent_drafts(&drafts);
            app.persist_draft_comments(&overlay.project_path, mr_iid);
            app.in_review_mode = false;
            let mr_short = app.kind().term("mr_short");
            app.show_notice(format!("Review submitted on {mr_short} #{mr_iid}"));
            true
        }
        Err(error) => {
            overlay.state = SubmissionState::Failed(error);
            app.push_modal(crate::app::Modal::ReviewSubmit(Box::new(overlay)));
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
        press_with(app, code, KeyModifiers::NONE)
    }

    fn press_with(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> bool {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut fx = Effects::default();
        let key = KeyPress::Key(crossterm::event::KeyEvent::new(code, modifiers));
        handle_key(app, &key, &tx, &mut fx)
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
        let drafts = vec![draft("nit")];
        app.draft_comments = drafts.clone();
        app.in_review_mode = true;
        open(&mut app, 7, PROJECT.to_string());
        let log_row = app.start_command(format!("{REVIEW_SUBMIT_LOG_PREFIX}MR #7"));
        if let Some(overlay) = app.review_submit_mut() {
            overlay.summary.set_value("Looks good".to_string());
            overlay.state = SubmissionState::Submitting { log_row, drafts };
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
        let overlay = app.review_submit().expect("still open");
        assert_eq!(overlay.summary.value(), "jq");
        assert_eq!(overlay.verdict, ReviewVerdict::Comment);
    }

    #[test]
    fn down_past_the_last_verdict_reaches_the_summary_and_up_returns() {
        let mut app = App::default();
        open(&mut app, 7, PROJECT.to_string());
        for code in [
            KeyCode::Char('k'),
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Char('o'),
            KeyCode::Char('k'),
        ] {
            assert!(press(&mut app, code));
        }
        let overlay = app.review_submit().expect("still open");
        assert_eq!(overlay.summary.value(), "ok");
        assert_eq!(overlay.verdict, ReviewVerdict::KeepAsDraft);

        press(&mut app, KeyCode::Up);
        let overlay = app.review_submit().expect("still open");
        assert_eq!(overlay.focus, ReviewSubmitFocus::Verdicts);
        assert_eq!(overlay.verdict, ReviewVerdict::KeepAsDraft);
    }

    #[test]
    fn alt_gr_characters_reach_the_summary_but_ctrl_shortcuts_do_not() {
        let mut app = App::default();
        open(&mut app, 7, PROJECT.to_string());
        press(&mut app, KeyCode::Tab);
        let alt_gr = KeyModifiers::CONTROL | KeyModifiers::ALT;

        for (c, modifiers) in [
            ('@', alt_gr),
            ('€', alt_gr),
            ('a', KeyModifiers::CONTROL),
            ('a', alt_gr),
            ('b', KeyModifiers::ALT),
        ] {
            assert!(press_with(&mut app, KeyCode::Char(c), modifiers));
        }

        let overlay = app.review_submit().expect("still open");
        assert_eq!(overlay.summary.value(), "@€");
    }

    #[test]
    fn esc_closes_the_overlay_and_leaves_the_drafts_alone() {
        let mut app = App::default();
        app.draft_comments = vec![draft("nit")];
        open(&mut app, 7, PROJECT.to_string());

        assert!(press(&mut app, KeyCode::Esc));

        assert!(app.review_submit().is_none());
        assert_eq!(app.draft_comments.len(), 1);
    }

    #[test]
    fn an_empty_comment_review_is_refused_inside_the_overlay() {
        let mut app = App::default();
        open(&mut app, 7, PROJECT.to_string());

        assert!(press(&mut app, KeyCode::Enter));

        let overlay = app.review_submit().expect("stays open");
        assert_eq!(
            overlay.state,
            SubmissionState::Refused("A comment review needs a summary or draft comments.".into())
        );
    }

    #[test]
    fn keep_as_draft_closes_the_overlay_without_touching_the_drafts() {
        let mut app = App::default();
        app.draft_comments = vec![draft("nit")];
        app.in_review_mode = true;
        open(&mut app, 7, PROJECT.to_string());
        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('j'),
            KeyCode::Char('j'),
            KeyCode::Enter,
        ] {
            press(&mut app, code);
        }

        assert!(app.review_submit().is_none());
        assert_eq!(app.draft_comments.len(), 1);
        assert!(app.in_review_mode);
    }

    #[test]
    fn keep_as_draft_counts_the_drafts_whose_lines_left_the_diff() {
        let mut app = App::default();
        app.draft_comments = vec![draft("nit"), draft("typo")];
        app.stale_draft_comments = vec![draft("moved")];
        open(&mut app, 7, PROJECT.to_string());
        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('j'),
            KeyCode::Char('j'),
            KeyCode::Enter,
        ] {
            press(&mut app, code);
        }

        assert_eq!(
            app.header_notice().map(crate::notice::Notice::text),
            Some("Kept 3 draft comments for later")
        );
        assert_eq!(app.draft_comments.len(), 2);
        assert_eq!(app.stale_draft_comments.len(), 1);
    }

    #[test]
    fn keys_are_swallowed_while_the_review_is_posted() {
        let mut app = submitting_app();

        for code in [KeyCode::Esc, KeyCode::Enter, KeyCode::Char('x')] {
            assert!(press(&mut app, code));
        }
        assert!(press_with(
            &mut app,
            KeyCode::Char('e'),
            KeyModifiers::CONTROL
        ));

        let overlay = app.review_submit().expect("stays open");
        assert!(overlay.is_submitting());
        assert_eq!(overlay.summary.value(), "Looks good");
        assert_eq!(
            overlay.focus,
            ReviewSubmitFocus::Verdicts,
            "Ctrl+E opens no editor while the review is posted"
        );
    }

    #[test]
    fn a_failed_submission_keeps_the_drafts_and_shows_the_error_inline() {
        let mut app = submitting_app();

        let posted = finish(&mut app, 7, Err("HTTP 422: body required".to_string()));

        assert!(!posted);
        assert_eq!(app.draft_comments.len(), 1);
        assert!(app.in_review_mode);
        assert_eq!(app.toast_notice(), None, "the error is not only a toast");
        let overlay = app.review_submit().expect("stays open");
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
    fn changing_the_verdict_after_a_refusal_returns_to_editing() {
        let mut app = App::default();
        open(&mut app, 7, PROJECT.to_string());
        press(&mut app, KeyCode::Enter);

        press(&mut app, KeyCode::Char('j'));

        let overlay = app.review_submit().expect("stays open");
        assert_eq!(overlay.verdict, ReviewVerdict::Approve);
        assert_eq!(overlay.state, SubmissionState::Editing);
    }

    #[test]
    fn editing_the_summary_after_a_failure_returns_to_editing_but_moving_the_cursor_does_not() {
        let mut app = submitting_app();
        finish(&mut app, 7, Err("HTTP 422: body required".to_string()));
        if let Some(overlay) = app.review_submit_mut() {
            overlay.focus = ReviewSubmitFocus::Summary;
        }

        press(&mut app, KeyCode::Left);
        assert_eq!(
            app.review_submit().map(|overlay| overlay.state.clone()),
            Some(SubmissionState::Failed("HTTP 422: body required".into()))
        );

        press(&mut app, KeyCode::Backspace);
        let overlay = app.review_submit().expect("stays open");
        assert_eq!(overlay.summary.value(), "Looks god");
        assert_eq!(overlay.state, SubmissionState::Editing);
    }

    #[test]
    fn a_posted_review_clears_its_drafts_and_their_saved_copy_and_closes_the_overlay() {
        let _home = crate::config::isolate_home();
        let mut app = submitting_app();
        app.persist_draft_comments(PROJECT, 7);

        let posted = finish(&mut app, 7, Ok(()));

        assert!(posted);
        assert!(app.review_submit().is_none());
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
    fn a_posted_review_removes_the_drafts_it_sent_even_after_a_refetch_moved_them() {
        let _home = crate::config::isolate_home();
        let mut app = submitting_app();
        app.draft_comments = vec![draft("held back, now anchored again")];
        app.stale_draft_comments = vec![draft("nit")];
        app.persist_draft_comments(PROJECT, 7);

        assert!(finish(&mut app, 7, Ok(())));

        assert_eq!(app.draft_comments, [draft("held back, now anchored again")]);
        assert!(app.stale_draft_comments.is_empty());
        assert_eq!(
            crate::utils::cache::load_cache(app.scope.as_str())
                .draft_comments
                .get(PROJECT)
                .and_then(|drafts| drafts.get(&7))
                .cloned(),
            Some(vec![draft("held back, now anchored again")]),
            "the sent draft is not posted again and the unsent one survives"
        );
    }

    #[test]
    fn a_posted_review_removes_one_copy_per_sent_draft() {
        let _home = crate::config::isolate_home();
        let mut app = submitting_app();
        app.draft_comments = vec![draft("nit"), draft("nit")];

        assert!(finish(&mut app, 7, Ok(())));

        assert_eq!(app.draft_comments, [draft("nit")]);
    }

    #[test]
    fn a_result_for_another_mr_is_ignored() {
        let mut app = submitting_app();

        assert!(!finish(&mut app, 8, Ok(())));

        assert!(app.review_submit().is_some_and(|o| o.is_submitting()));
        assert_eq!(app.draft_comments.len(), 1);
    }
}
