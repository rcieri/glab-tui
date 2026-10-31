//! The submit-review overlay opened from the diff view: the verdict, the
//! summary and the state of the submission it started.

use crate::app::TextBuffer;
use crate::command_log::LogRow;
use crate::domain::review::ReviewEvent;
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

    fn position(self) -> usize {
        Self::ALL
            .iter()
            .position(|&v| v == self)
            .unwrap_or_default()
    }

    /// The verdict `step` rows below (or above, when negative), wrapping
    /// around like the selectors do.
    pub fn stepped(self, step: isize) -> Self {
        let len = Self::ALL.len() as isize;
        let index = (self.position() as isize + step).rem_euclid(len);
        Self::ALL[index as usize]
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
    pub fn cycled(self, step: isize) -> Self {
        let len = Self::ORDER.len() as isize;
        let position = Self::ORDER
            .iter()
            .position(|&f| f == self)
            .unwrap_or_default();
        Self::ORDER[(position as isize + step).rem_euclid(len) as usize]
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SubmissionState {
    #[default]
    Editing,
    /// The review is being posted. Its Terminal-tab entry is `log_row`.
    Submitting { log_row: LogRow },
    /// The last attempt failed and the drafts are still pending.
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
    fn verdicts_wrap_around_in_both_directions() {
        assert_eq!(
            ReviewVerdict::KeepAsDraft.stepped(1),
            ReviewVerdict::Comment
        );
        assert_eq!(
            ReviewVerdict::Comment.stepped(-1),
            ReviewVerdict::KeepAsDraft
        );
        assert_eq!(
            ReviewVerdict::Approve.stepped(1),
            ReviewVerdict::RequestChanges
        );
    }

    #[test]
    fn focus_cycles_through_verdicts_summary_and_both_buttons() {
        let mut focus = ReviewSubmitFocus::Verdicts;
        let mut seen = vec![focus];
        for _ in 0..4 {
            focus = focus.cycled(1);
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
            ReviewSubmitFocus::Verdicts.cycled(-1),
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
}
