use std::borrow::Cow;

use ratatui::layout::{Alignment, Constraint};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Cell;

use super::{CellContext, ColumnDef, Row};
use crate::domain::releases::Release;
use crate::ui::helpers::render_fuzzy_cell;
use crate::utils::format::truncate;

const NAME_MAX_CHARS: usize = 100;
const DATE_MAX_CHARS: usize = 15;
const ASSETS_MAX_CHARS: usize = 50;
const NOTES_MAX_CHARS: usize = 80;

impl Row for Release {
    type Key = String;

    fn key(&self) -> String {
        self.tag_name.clone()
    }

    fn has_key(&self, key: &String) -> bool {
        self.tag_name == *key
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseColumn {
    Tag,
    Name,
    Date,
    Author,
    Assets,
    Notes,
}

impl ColumnDef for ReleaseColumn {
    type Row = Release;

    const ALL: &'static [Self] = &[
        Self::Tag,
        Self::Name,
        Self::Date,
        Self::Author,
        Self::Assets,
        Self::Notes,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Tag => "Tag",
            Self::Name => "Release Name",
            Self::Date => "Date",
            Self::Author => "Author",
            Self::Assets => "Assets",
            Self::Notes => "Release Notes",
        }
    }

    fn is_default(self) -> bool {
        matches!(self, Self::Tag | Self::Name | Self::Date)
    }

    fn width(self) -> Constraint {
        match self {
            Self::Tag => Constraint::Length(16),
            Self::Name => Constraint::Length(30),
            Self::Date => Constraint::Length(15),
            Self::Author => Constraint::Length(18),
            Self::Assets => Constraint::Length(12),
            Self::Notes => Constraint::Fill(1),
        }
    }

    fn text(self, row: &Release) -> Cow<'_, str> {
        Cow::Borrowed(match self {
            Self::Tag => &row.tag_name,
            Self::Name => &row.name,
            Self::Date => &row.released_at,
            Self::Author => row.author_name.as_deref().unwrap_or_default(),
            Self::Assets => row.assets_link.as_deref().unwrap_or_default(),
            Self::Notes => row.description.as_deref().unwrap_or_default(),
        })
    }

    fn cell(self, row: &Release, ctx: &CellContext) -> Cell<'static> {
        let theme = ctx.theme;
        let text = self.text(row);
        let (shown, style) = match self {
            Self::Tag => (
                text,
                Style::default()
                    .fg(theme.green)
                    .add_modifier(Modifier::BOLD),
            ),
            Self::Name => (
                Cow::Owned(truncate(&text, NAME_MAX_CHARS)),
                Style::default().fg(theme.text_normal),
            ),
            Self::Date => (
                Cow::Owned(truncate(&text, DATE_MAX_CHARS)),
                Style::default().fg(theme.yellow),
            ),
            Self::Author => (text, Style::default().fg(theme.blue)),
            Self::Assets => (
                Cow::Owned(truncate(&text, ASSETS_MAX_CHARS)),
                Style::default().fg(theme.blue),
            ),
            Self::Notes => (
                Cow::Owned(truncate(&text, NOTES_MAX_CHARS)),
                Style::default().fg(theme.text_muted),
            ),
        };
        render_fuzzy_cell(
            &shown,
            ctx.query,
            ctx.is_highlighted,
            false,
            style,
            Alignment::Left,
        )
    }
}
