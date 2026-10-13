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
    /// Maps the labels of the TUI's submit-review selector.
    pub fn from_label(label: &str) -> Self {
        match label {
            "Approve" => Self::Approve,
            "Request Changes" => Self::RequestChanges,
            _ => Self::Comment,
        }
    }
}

/// An inline comment waiting to be submitted as part of a review.
///
/// Lines are diff line numbers: `line_num` on the new side, `old_line_num` on
/// the old side. An unchanged line carries both. A range sets the matching
/// `end_*` field to its last line.
#[derive(Clone, Debug)]
pub struct DraftComment {
    pub file_path: String,
    pub line_num: Option<u32>,
    pub old_line_num: Option<u32>,
    pub end_line_num: Option<u32>,
    pub end_old_line_num: Option<u32>,
    pub body: String,
}
