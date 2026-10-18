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
}

impl BranchColumn {
    fn flag(self, row: &Branch) -> bool {
        match self {
            Self::Default => row.default,
            Self::Protected => row.protected,
            Self::Name | Self::Sha => false,
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

    const ALL: &'static [Self] = &[Self::Name, Self::Default, Self::Protected, Self::Sha];

    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Default => "Default",
            Self::Protected => "Protected",
            Self::Sha => "SHA",
        }
    }

    fn is_default(self) -> bool {
        !matches!(self, Self::Sha)
    }

    fn width(self) -> Constraint {
        match self {
            Self::Name => Constraint::Fill(1),
            Self::Default => Constraint::Length(12),
            Self::Protected => Constraint::Length(14),
            Self::Sha => Constraint::Length(12),
        }
    }

    fn text(self, row: &Branch) -> Cow<'_, str> {
        match self {
            Self::Name => Cow::Borrowed(&row.name),
            Self::Default | Self::Protected => Cow::Borrowed(yes_no(self.flag(row))),
            Self::Sha => Cow::Borrowed(&row.commit_sha),
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
        }
    }

    fn searchable(self) -> bool {
        matches!(self, Self::Name | Self::Sha)
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
            Self::Name | Self::Sha => {
                crate::app::column_filter_matches(self.label(), &self.filter_values(row), selected)
            }
        }
    }
}
