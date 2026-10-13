use crate::app::{DiffLine, DiffView};
use crate::domain::client::GitlabClient;
use crate::domain::mr::{get_mr_diff, list_mr_notes};
use crate::domain::review::{DraftComment, ReviewEvent};
use crate::domain::review_threads::{ReviewThread, ThreadAnchor, group_threads};
use anyhow::{Context, Result, bail};
use clap::{Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Subcommand)]
pub enum ReviewCommand {
    /// Print the MR/PR's review threads as JSON
    Threads {
        /// MR/PR number
        iid: u64,
    },
    /// Post one inline comment on a diff line or line range
    Comment {
        /// MR/PR number
        iid: u64,
        /// File path as it appears in the diff
        #[arg(long)]
        file: String,
        /// Line number on the chosen side of the diff
        #[arg(long)]
        line: u32,
        /// Last line of a range, on the same side
        #[arg(long)]
        end_line: Option<u32>,
        /// Diff side the line numbers refer to
        #[arg(long, value_enum, default_value_t = Side::New)]
        side: Side,
        /// Comment text
        body: String,
    },
    /// Submit one review holding every comment from --input
    Submit {
        /// MR/PR number
        iid: u64,
        #[arg(long, value_enum)]
        event: ReviewEvent,
        /// Review summary
        #[arg(long, default_value = "")]
        body: String,
        /// JSON array of {file, line, end_line?, side?, body}; `-` reads stdin
        #[arg(long, value_name = "FILE")]
        input: Option<String>,
    },
    /// Reply to a review thread
    Reply {
        /// MR/PR number
        iid: u64,
        /// Thread id, as printed by `review threads`
        #[arg(long)]
        thread: String,
        /// Reply text
        body: String,
    },
    /// Resolve a review thread (GitLab only)
    Resolve {
        /// MR/PR number
        iid: u64,
        /// Thread id, as printed by `review threads`
        #[arg(long)]
        thread: String,
        /// Reopen the thread instead
        #[arg(long)]
        unresolve: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Old,
    #[default]
    New,
}

impl Side {
    fn as_str(self) -> &'static str {
        match self {
            Side::Old => "old",
            Side::New => "new",
        }
    }
}

/// One entry of the `submit --input` array.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputComment {
    file: String,
    line: u32,
    #[serde(default)]
    end_line: Option<u32>,
    #[serde(default)]
    side: Side,
    body: String,
}

impl InputComment {
    /// Checks the anchor against the diff and turns it into a draft. A single
    /// unchanged line gets its number on the other side too: GitLab rejects a
    /// note on an unchanged line unless both are given.
    fn anchor(self, diff: &DiffView) -> Result<DraftComment> {
        if self.body.trim().is_empty() {
            bail!("comment on {}:{} has an empty body", self.file, self.line);
        }
        let start = find_diff_line(diff, &self.file, self.side, self.line)?;
        let end_line = match self.end_line {
            Some(end) if end < self.line => bail!(
                "end_line {} is before line {} on {}",
                end,
                self.line,
                self.file
            ),
            Some(end) => {
                find_diff_line(diff, &self.file, self.side, end)?;
                Some(end).filter(|&end| end != self.line)
            }
            None => None,
        };
        let single_line = end_line.is_none();
        Ok(match self.side {
            Side::New => DraftComment {
                line_num: Some(self.line),
                old_line_num: start.old_line_num.filter(|_| single_line),
                end_line_num: end_line,
                end_old_line_num: None,
                file_path: self.file,
                body: self.body,
            },
            Side::Old => DraftComment {
                line_num: start.new_line_num.filter(|_| single_line),
                old_line_num: Some(self.line),
                end_line_num: None,
                end_old_line_num: end_line,
                file_path: self.file,
                body: self.body,
            },
        })
    }
}

fn find_diff_line<'a>(
    diff: &'a DiffView,
    file: &str,
    side: Side,
    line: u32,
) -> Result<&'a DiffLine> {
    diff.all_lines
        .iter()
        .find(|l| {
            l.file_path == file
                && match side {
                    Side::New => l.new_line_num == Some(line),
                    Side::Old => l.old_line_num == Some(line),
                }
        })
        .with_context(|| {
            format!(
                "{}:{} ({} side) is not part of the diff of #{}",
                file,
                line,
                side.as_str(),
                diff.mr_iid
            )
        })
}

#[derive(Serialize)]
struct ThreadOutput {
    id: String,
    classification: &'static str,
    anchor: Option<AnchorOutput>,
    resolvable: bool,
    resolved: bool,
    notes: Vec<NoteOutput>,
}

#[derive(Serialize)]
struct AnchorOutput {
    file: String,
    line: Option<u64>,
    side: Side,
}

#[derive(Serialize)]
struct NoteOutput {
    id: u64,
    author: String,
    body: String,
    created_at: String,
}

impl From<&ReviewThread> for ThreadOutput {
    fn from(thread: &ReviewThread) -> Self {
        let root = thread.root();
        let (classification, anchor) = match &thread.anchor {
            ThreadAnchor::General => ("general", None),
            ThreadAnchor::InDiff { file_path, line } => ("in-diff", Some((file_path, *line))),
            ThreadAnchor::Outdated { file_path, line } => ("outdated", Some((file_path, *line))),
        };
        let side = match root.position.as_ref() {
            Some(p) if p.new_line.is_none() && p.old_line.is_some() => Side::Old,
            _ => Side::New,
        };
        Self {
            id: root
                .discussion_id
                .clone()
                .unwrap_or_else(|| root.id.to_string()),
            classification,
            anchor: anchor.map(|(file, line)| AnchorOutput {
                file: file.clone(),
                line,
                side,
            }),
            resolvable: thread.is_resolvable(),
            resolved: thread.is_resolvable() && !thread.is_unresolved(),
            notes: thread
                .notes
                .iter()
                .map(|n| NoteOutput {
                    id: n.id,
                    author: n.author.username.clone(),
                    body: n.body.clone(),
                    created_at: n.created_at.clone(),
                })
                .collect(),
        }
    }
}

/// Runs one review subcommand and returns the JSON document it prints.
pub async fn run(command: ReviewCommand, repo: Option<String>) -> Result<serde_json::Value> {
    let project = match repo {
        Some(repo) => repo,
        None => crate::domain::client::get_project_context().await?,
    };
    if project == "unknown/unknown" {
        bail!("cannot tell which repository to use; run inside a checkout or pass --repo");
    }
    let client = GitlabClient::new(&crate::config::Config::load()).await?;

    match command {
        ReviewCommand::Threads { iid } => {
            let (raw_diff, notes) = tokio::try_join!(
                get_mr_diff(&client, &project, iid),
                list_mr_notes(&client, &project, iid),
            )?;
            let diff = DiffView::new(iid, project, raw_diff);
            let threads: Vec<ThreadOutput> = group_threads(&notes, |p| diff.contains_anchor(p))
                .iter()
                .map(ThreadOutput::from)
                .collect();
            Ok(serde_json::to_value(threads)?)
        }
        ReviewCommand::Comment {
            iid,
            file,
            line,
            end_line,
            side,
            body,
        } => {
            let input = InputComment {
                file,
                line,
                end_line,
                side,
                body,
            };
            let comments = anchor_comments(&client, &project, iid, vec![input]).await?;
            client
                .submit_review(&project, iid, ReviewEvent::Comment, "", &comments)
                .await?;
            Ok(json!({ "iid": iid, "comments": comments.len() }))
        }
        ReviewCommand::Submit {
            iid,
            event,
            body,
            input,
        } => {
            let inputs = match input {
                Some(source) => read_input(&source)?,
                None => Vec::new(),
            };
            if inputs.is_empty() && body.trim().is_empty() && event != ReviewEvent::Approve {
                bail!("nothing to submit: pass --body or comments with --input");
            }
            let comments = if inputs.is_empty() {
                Vec::new()
            } else {
                anchor_comments(&client, &project, iid, inputs).await?
            };
            client
                .submit_review(&project, iid, event, &body, &comments)
                .await?;
            Ok(json!({ "iid": iid, "event": event, "comments": comments.len() }))
        }
        ReviewCommand::Reply { iid, thread, body } => {
            if body.trim().is_empty() {
                bail!("reply body is empty");
            }
            client
                .reply_to_thread(&project, iid, &thread, &body)
                .await?;
            Ok(json!({ "iid": iid, "thread": thread }))
        }
        ReviewCommand::Resolve {
            iid,
            thread,
            unresolve,
        } => {
            client
                .set_thread_resolved(&project, iid, &thread, !unresolve)
                .await?;
            Ok(json!({ "iid": iid, "thread": thread, "resolved": !unresolve }))
        }
    }
}

async fn anchor_comments(
    client: &GitlabClient,
    project: &str,
    iid: u64,
    inputs: Vec<InputComment>,
) -> Result<Vec<DraftComment>> {
    let raw_diff = get_mr_diff(client, project, iid).await?;
    let diff = DiffView::new(iid, project.to_string(), raw_diff);
    inputs.into_iter().map(|c| c.anchor(&diff)).collect()
}

fn read_input(source: &str) -> Result<Vec<InputComment>> {
    let raw = if source == "-" {
        std::io::read_to_string(std::io::stdin()).context("reading comments from stdin")?
    } else {
        std::fs::read_to_string(source).with_context(|| format!("reading {}", source))?
    };
    serde_json::from_str(&raw)
        .context("--input must be a JSON array of {file, line, end_line?, side?, body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,4 +1,4 @@
 fn a() {
-    old();
+    new();
 }
";

    fn input(side: Side, line: u32, end_line: Option<u32>) -> InputComment {
        InputComment {
            file: "src/lib.rs".to_string(),
            line,
            end_line,
            side,
            body: "note".to_string(),
        }
    }

    fn diff() -> DiffView {
        DiffView::new(7, "group/project".to_string(), DIFF.to_string())
    }

    #[test]
    fn unchanged_line_carries_both_line_numbers() {
        let draft = input(Side::New, 1, None).anchor(&diff()).unwrap();
        assert_eq!((draft.line_num, draft.old_line_num), (Some(1), Some(1)));
    }

    #[test]
    fn removed_line_is_anchored_on_the_old_side_only() {
        let draft = input(Side::Old, 2, None).anchor(&diff()).unwrap();
        assert_eq!((draft.line_num, draft.old_line_num), (None, Some(2)));
    }

    #[test]
    fn range_keeps_to_its_side() {
        let draft = input(Side::New, 1, Some(3)).anchor(&diff()).unwrap();
        assert_eq!(
            (draft.line_num, draft.end_line_num, draft.old_line_num),
            (Some(1), Some(3), None)
        );
    }

    #[test]
    fn line_outside_the_diff_is_rejected() {
        let err = input(Side::New, 40, None).anchor(&diff()).unwrap_err();
        assert!(err.to_string().contains("src/lib.rs:40 (new side)"));
    }

    #[test]
    fn added_line_has_no_old_side_number() {
        let draft = input(Side::New, 2, None).anchor(&diff()).unwrap();
        assert_eq!((draft.line_num, draft.old_line_num), (Some(2), None));
    }

    #[test]
    fn end_line_before_line_is_rejected() {
        assert!(input(Side::New, 3, Some(1)).anchor(&diff()).is_err());
    }
}
