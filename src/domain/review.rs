/// The verdict a submitted review carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewEvent {
    Comment,
    Approve,
    /// GitLab has no request-changes verdict, so there the review only
    /// publishes its comments and summary.
    RequestChanges,
}

impl ReviewEvent {
    /// Whether a review would post nothing: an approval stands on its own,
    /// any other verdict needs a summary or an inline comment.
    pub fn is_empty_review(self, body: &str, comment_count: usize) -> bool {
        self != Self::Approve && comment_count == 0 && body.trim().is_empty()
    }
}

/// An inline comment waiting to be submitted as part of a review.
///
/// Lines are diff line numbers: `line_num` on the new side, `old_line_num` on
/// the old side. An unchanged line carries both. A range sets the matching
/// `end_*` field to its last line.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DraftComment {
    pub file_path: String,
    pub line_num: Option<u32>,
    pub old_line_num: Option<u32>,
    pub end_line_num: Option<u32>,
    pub end_old_line_num: Option<u32>,
    pub body: String,
}

impl DraftComment {
    /// The diff positions the draft is anchored to: its first line and, for a
    /// range, its last.
    pub fn anchor_positions(&self) -> impl Iterator<Item = crate::domain::mr::NotePosition> + '_ {
        let start = (self.line_num, self.old_line_num);
        let end = (self.end_line_num, self.end_old_line_num);
        std::iter::once(start)
            .chain((end != (None, None)).then_some(end))
            .map(|(new_line, old_line)| crate::domain::mr::NotePosition {
                new_path: Some(self.file_path.clone()),
                old_path: Some(self.file_path.clone()),
                new_line: new_line.map(u64::from),
                old_line: old_line.map(u64::from),
                start_line: None,
                line_range: None,
            })
    }
}
