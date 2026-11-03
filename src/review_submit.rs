//! The submit-review overlay opened from the diff view: the verdict, the
//! summary and the state of the submission it started.

use crate::app::TextBuffer;
use crate::command_log::LogRow;
use crate::domain::review::{DraftComment, ReviewEvent};
use ratatui::layout::Rect;

/// The outcomes the overlay offers, top to bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReviewVerdict {
    #[default]
    Comment,
    Approve,
    RequestChanges,
    KeepAsDraft,
}

impl ReviewVerdict {
    pub const ALL: [Self; 4] = [
        Self::Comment,
        Self::Approve,
        Self::RequestChanges,
        Self::KeepAsDraft,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Comment => "Comment",
            Self::Approve => "Approve",
            Self::RequestChanges => "Request changes",
            Self::KeepAsDraft => "Keep as draft",
        }
    }

    /// The event the review is posted with. Keeping the drafts posts nothing.
    pub fn event(self) -> Option<ReviewEvent> {
        match self {
            Self::Comment => Some(ReviewEvent::Comment),
            Self::Approve => Some(ReviewEvent::Approve),
            Self::RequestChanges => Some(ReviewEvent::RequestChanges),
            Self::KeepAsDraft => None,
        }
    }

    /// The verdict one row down, `None` on the last row, where the focus
    /// moves on to the summary instead.
    pub fn below(self) -> Option<Self> {
        Self::ALL.get(self.position() + 1).copied()
    }

    /// The verdict one row up, `None` on the first row.
    pub fn above(self) -> Option<Self> {
        self.position().checked_sub(1).map(|row| Self::ALL[row])
    }

    fn position(self) -> usize {
        Self::ALL
            .iter()
            .position(|&verdict| verdict == self)
            .unwrap_or_default()
    }
}

/// The part of the overlay that takes the keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReviewSubmitFocus {
    #[default]
    Verdicts,
    Summary,
    SubmitButton,
    CancelButton,
}

impl ReviewSubmitFocus {
    const ORDER: [Self; 4] = [
        Self::Verdicts,
        Self::Summary,
        Self::SubmitButton,
        Self::CancelButton,
    ];

    /// The focus `Tab` (positive `step`) or `Shift+Tab` moves to, wrapping
    /// around.
    pub fn stepped(self, step: isize) -> Self {
        let position = Self::ORDER
            .iter()
            .position(|&focus| focus == self)
            .unwrap_or_default() as isize;
        Self::ORDER[(position + step).rem_euclid(Self::ORDER.len() as isize) as usize]
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SubmissionState {
    #[default]
    Editing,
    /// The review is being posted with `drafts`. Its Terminal-tab entry is
    /// `log_row`.
    Submitting {
        log_row: LogRow,
        drafts: Vec<DraftComment>,
    },
    /// Nothing was sent: why the review cannot be submitted as it stands.
    Refused(String),
    /// The server rejected the last attempt and the drafts are still pending.
    Failed(String),
}

/// Where the last frame drew the overlay's clickable parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReviewSubmitLayout {
    pub verdicts: Rect,
    pub summary: Rect,
    pub submit_button: Rect,
    pub cancel_button: Rect,
}

#[derive(Clone, Debug, Default)]
pub struct ReviewSubmit {
    pub mr_iid: u64,
    /// Project of the diff whose drafts the review posts.
    pub project_path: String,
    pub verdict: ReviewVerdict,
    pub summary: TextBuffer,
    pub focus: ReviewSubmitFocus,
    pub state: SubmissionState,
    pub layout: Option<ReviewSubmitLayout>,
}

impl ReviewSubmit {
    pub fn new(mr_iid: u64, project_path: String) -> Self {
        Self {
            mr_iid,
            project_path,
            ..Self::default()
        }
    }

    pub fn is_submitting(&self) -> bool {
        matches!(self.state, SubmissionState::Submitting { .. })
    }

    /// A refusal or failure describes the verdict and summary it was raised
    /// for, so it goes once either changes. A running submission is kept.
    pub fn clear_outcome(&mut self) {
        if matches!(
            self.state,
            SubmissionState::Refused(_) | SubmissionState::Failed(_)
        ) {
            self.state = SubmissionState::Editing;
        }
    }

    /// The event to post the review with, `Ok(None)` when the drafts are
    /// kept instead, or why the review cannot be submitted as it stands.
    pub fn event_to_post(&self, draft_count: usize) -> Result<Option<ReviewEvent>, String> {
        let Some(event) = self.verdict.event() else {
            return Ok(None);
        };
        if event.is_empty_review(self.summary.value(), draft_count) {
            return Err(format!(
                "A {} review needs a summary or draft comments.",
                self.verdict.label().to_lowercase()
            ));
        }
        Ok(Some(event))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_stop_at_the_first_and_last_rows() {
        assert_eq!(ReviewVerdict::Comment.above(), None);
        assert_eq!(ReviewVerdict::KeepAsDraft.below(), None);
        assert_eq!(
            ReviewVerdict::Approve.below(),
            Some(ReviewVerdict::RequestChanges)
        );
        assert_eq!(ReviewVerdict::Approve.above(), Some(ReviewVerdict::Comment));
    }

    #[test]
    fn focus_steps_through_verdicts_summary_and_both_buttons() {
        let mut focus = ReviewSubmitFocus::Verdicts;
        let mut seen = vec![focus];
        for _ in 0..4 {
            focus = focus.stepped(1);
            seen.push(focus);
        }
        assert_eq!(
            seen,
            [
                ReviewSubmitFocus::Verdicts,
                ReviewSubmitFocus::Summary,
                ReviewSubmitFocus::SubmitButton,
                ReviewSubmitFocus::CancelButton,
                ReviewSubmitFocus::Verdicts,
            ]
        );
        assert_eq!(
            ReviewSubmitFocus::Verdicts.stepped(-1),
            ReviewSubmitFocus::CancelButton
        );
    }

    #[test]
    fn a_comment_without_summary_or_drafts_is_refused_but_an_approval_is_not() {
        let mut overlay = ReviewSubmit::new(7, "acme/widget".to_string());
        assert_eq!(
            overlay.event_to_post(0),
            Err("A comment review needs a summary or draft comments.".to_string())
        );
        assert_eq!(overlay.event_to_post(1), Ok(Some(ReviewEvent::Comment)));

        overlay.verdict = ReviewVerdict::RequestChanges;
        overlay.summary.set_value("   ".to_string());
        assert_eq!(
            overlay.event_to_post(0),
            Err("A request changes review needs a summary or draft comments.".to_string())
        );
        overlay.summary.set_value("Please rename it".to_string());
        assert_eq!(
            overlay.event_to_post(0),
            Ok(Some(ReviewEvent::RequestChanges))
        );

        overlay.verdict = ReviewVerdict::Approve;
        overlay.summary.set_value(String::new());
        assert_eq!(overlay.event_to_post(0), Ok(Some(ReviewEvent::Approve)));
    }

    #[test]
    fn keeping_drafts_posts_nothing() {
        let overlay = ReviewSubmit {
            verdict: ReviewVerdict::KeepAsDraft,
            ..ReviewSubmit::new(7, "acme/widget".to_string())
        };
        assert_eq!(overlay.event_to_post(0), Ok(None));
    }

    #[test]
    fn clearing_the_outcome_drops_a_refusal_or_failure_but_keeps_a_running_submission() {
        let mut overlay = ReviewSubmit::new(7, "acme/widget".to_string());
        for outcome in [
            SubmissionState::Refused("needs a summary".into()),
            SubmissionState::Failed("HTTP 422".into()),
        ] {
            overlay.state = outcome;
            overlay.clear_outcome();
            assert_eq!(overlay.state, SubmissionState::Editing);
        }

        let log_row = crate::app::App::default().start_command("Submit review: MR #7".into());
        let running = SubmissionState::Submitting {
            log_row,
            drafts: Vec::new(),
        };
        overlay.state = running.clone();
        overlay.clear_outcome();
        assert_eq!(overlay.state, running);
    }
}
