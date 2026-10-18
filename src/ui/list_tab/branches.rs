use std::borrow::Cow;
use std::collections::HashSet;

use ratatui::layout::{Alignment, Constraint};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Cell;

use super::{CellContext, ColumnDef, Row};
use crate::domain::branches::Branch;
use crate::ui::helpers::render_fuzzy_cell;
use crate::utils::format::truncate;

const SHA_SHOWN_CHARS: usize = 8;

impl Row for Branch {
    type Key = String;

    fn key(&self) -> String {
        self.name.clone()
    }

    fn has_key(&self, key: &String) -> bool {
        self.name == *key
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchColumn {
    Name,
    Default,
    Protected,
    Sha,
    Author,
    Date,
    AheadBehind,
}

impl BranchColumn {
    fn flag(self, row: &Branch) -> bool {
        match self {
            Self::Default => row.default,
            Self::Protected => row.protected,
            Self::Name | Self::Sha | Self::Author | Self::Date | Self::AheadBehind => false,
        }
    }
}

fn yes_no(flag: bool) -> &'static str {
    if flag { "YES" } else { "NO" }
}

fn badge(text: String, color: Color, bg: Color, ctx: &CellContext) -> Cell<'static> {
    Cell::from(Span::styled(
        text,
        Style::default()
            .fg(color)
            .bg(if ctx.is_highlighted {
                ctx.theme.highlight_bg
            } else {
                bg
            })
            .add_modifier(Modifier::BOLD),
    ))
}

impl ColumnDef for BranchColumn {
    type Row = Branch;

    const ALL: &'static [Self] = &[
        Self::Name,
        Self::Default,
        Self::Protected,
        Self::Sha,
        Self::Author,
        Self::Date,
        Self::AheadBehind,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Default => "Default",
            Self::Protected => "Protected",
            Self::Sha => "SHA",
            Self::Author => "Author",
            Self::Date => "Date",
            Self::AheadBehind => "Ahead/Behind",
        }
    }

    fn is_default(self) -> bool {
        matches!(
            self,
            Self::Name | Self::Default | Self::Protected | Self::Author | Self::Date
        )
    }

    fn width(self) -> Constraint {
        match self {
            Self::Name => Constraint::Fill(1),
            Self::Default => Constraint::Length(12),
            Self::Protected => Constraint::Length(14),
            Self::Sha => Constraint::Length(12),
            Self::Author => Constraint::Length(16),
            Self::Date => Constraint::Length(14),
            Self::AheadBehind => Constraint::Length(14),
        }
    }

    fn text(self, row: &Branch) -> Cow<'_, str> {
        match self {
            Self::Name => Cow::Borrowed(&row.name),
            Self::Default | Self::Protected => Cow::Borrowed(yes_no(self.flag(row))),
            Self::Sha => Cow::Borrowed(&row.commit_sha),
            Self::Author => Cow::Borrowed(row.author_name.as_deref().unwrap_or("--")),
            Self::Date => row
                .commit_date
                .as_deref()
                .map(|d| Cow::Owned(crate::utils::format::time_ago(d)))
                .unwrap_or(Cow::Borrowed("--")),
            Self::AheadBehind => match row.ahead_behind {
                Some((a, b)) => Cow::Owned(format!("+{a} / -{b}")),
                None => Cow::Borrowed("--"),
            },
        }
    }

    fn cell(self, row: &Branch, ctx: &CellContext) -> Cell<'static> {
        let theme = ctx.theme;
        let icons = ctx.icons;
        let fuzzy = |text: &str| {
            render_fuzzy_cell(
                text,
                ctx.query,
                ctx.is_highlighted,
                false,
                ctx.row_style,
                Alignment::Left,
            )
        };
        match self {
            Self::Name => fuzzy(&format!("{} {}", icons.label_branch, row.name)),
            Self::Default if row.default => badge(
                format!(" {} YES ", icons.radio_on),
                theme.green,
                theme.green_bg,
                ctx,
            ),
            Self::Protected if row.protected => badge(
                format!(" {} YES ", icons.lock),
                theme.yellow,
                theme.yellow_bg,
                ctx,
            ),
            Self::Default | Self::Protected => {
                Cell::from(Span::styled(" NO ", Style::default().fg(theme.text_muted)))
            }
            Self::Sha if row.commit_sha.is_empty() => fuzzy("--"),
            Self::Sha => fuzzy(&truncate(&row.commit_sha, SHA_SHOWN_CHARS)),
            Self::Author => fuzzy(row.author_name.as_deref().unwrap_or("--")),
            Self::Date => {
                let text = row
                    .commit_date
                    .as_deref()
                    .map(crate::utils::format::time_ago)
                    .unwrap_or_else(|| "--".to_string());
                fuzzy(&text)
            }
            Self::AheadBehind => match row.ahead_behind {
                Some((ahead, behind)) => fuzzy(&format!("+{ahead} / -{behind}")),
                None => fuzzy("--"),
            },
        }
    }

    fn searchable(self) -> bool {
        matches!(self, Self::Name | Self::Sha | Self::Author)
    }

    fn matches_filter(self, row: &Branch, selected: &HashSet<String>) -> bool {
        match self {
            // Filters saved before these columns showed YES/NO hold the raw
            // "true"/"false" values.
            Self::Default | Self::Protected => {
                let flag = self.flag(row);
                let legacy = if flag { "true" } else { "false" };
                selected.contains(yes_no(flag)) || selected.contains(legacy)
            }
            Self::Name | Self::Sha | Self::Author | Self::Date | Self::AheadBehind => {
                crate::app::column_filter_matches(self.label(), &self.filter_values(row), selected)
            }
        }
    }
}
