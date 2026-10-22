use std::borrow::Cow;

use ratatui::layout::{Alignment, Constraint};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Cell;

use super::{CellContext, ColumnDef, Row, SortKey, cell_filter_values};
use crate::domain::runners::Runner;
use crate::ui::helpers::{badge_style_for, render_fuzzy_cell};
use crate::utils::format::truncate;

const DESCRIPTION_MAX_CHARS: usize = 100;
const TAGS_MAX_CHARS: usize = 60;
const UNKNOWN_VALUE: &str = "--";

impl Row for Runner {
    type Key = u64;

    fn key(&self) -> u64 {
        self.id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerColumn {
    Id,
    Description,
    Status,
    Active,
    Type,
    Tags,
    Version,
    LastContact,
}

impl ColumnDef for RunnerColumn {
    type Row = Runner;

    const ALL: &'static [Self] = &[
        Self::Id,
        Self::Description,
        Self::Status,
        Self::Active,
        Self::Type,
        Self::Tags,
        Self::Version,
        Self::LastContact,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Id => "ID",
            Self::Description => "Description",
            Self::Status => "Status",
            Self::Active => "Active",
            Self::Type => "Type",
            Self::Tags => "Tags",
            Self::Version => "Version",
            Self::LastContact => "Last Contact",
        }
    }

    fn is_default(self) -> bool {
        !matches!(self, Self::Type | Self::Version | Self::LastContact)
    }

    fn width(self) -> Constraint {
        match self {
            Self::Id => Constraint::Length(8),
            Self::Description => Constraint::Fill(2),
            Self::Status => Constraint::Length(20),
            Self::Active => Constraint::Length(10),
            Self::Type => Constraint::Length(10),
            Self::Tags => Constraint::Fill(1),
            Self::Version => Constraint::Length(12),
            Self::LastContact => Constraint::Length(16),
        }
    }

    fn header_alignment(self) -> Alignment {
        match self {
            Self::Status | Self::Active => Alignment::Center,
            _ => Alignment::Left,
        }
    }

    fn text(self, row: &Runner) -> Cow<'_, str> {
        match self {
            Self::Id => Cow::Owned(row.id.to_string()),
            Self::Description => {
                Cow::Borrowed(row.description.as_deref().unwrap_or("No description"))
            }
            Self::Status => Cow::Borrowed(row.status_kind().label()),
            Self::Active => Cow::Borrowed(crate::app::runner_active_display(row.active)),
            Self::Type => Cow::Borrowed(row.runner_type.map_or("", |kind| kind.label())),
            Self::Tags => Cow::Owned(row.tags.join(", ")),
            Self::Version => Cow::Borrowed(row.version.as_deref().unwrap_or_default()),
            Self::LastContact => row.contacted_at.as_deref().map_or(Cow::Borrowed(""), |at| {
                Cow::Owned(crate::utils::format::time_ago(at))
            }),
        }
    }

    fn cell(self, row: &Runner, ctx: &CellContext) -> Cell<'static> {
        let theme = ctx.theme;
        let fuzzy = |text: &str, style: Style, alignment: Alignment| {
            render_fuzzy_cell(text, ctx.query, ctx.is_highlighted, false, style, alignment)
        };
        match self {
            Self::Id => fuzzy(
                &format!("#{}", row.id),
                Style::default().fg(theme.text_normal),
                Alignment::Left,
            ),
            Self::Description => fuzzy(
                &truncate(&self.text(row), DESCRIPTION_MAX_CHARS),
                Style::default().fg(theme.text_normal),
                Alignment::Left,
            ),
            Self::Status => {
                let status = row.status_kind().label();
                let row_bg = ctx.row_style.bg.unwrap_or(theme.bg);
                let (fg, badge_bg, is_bold, pill) = badge_style_for(
                    self.label(),
                    status,
                    ctx.is_highlighted,
                    row_bg,
                    theme,
                    ctx.icons,
                );
                let mut style = Style::default().fg(fg).bg(badge_bg.unwrap_or(row_bg));
                if is_bold {
                    style = style.add_modifier(Modifier::BOLD);
                }
                fuzzy(pill.as_deref().unwrap_or(status), style, Alignment::Center)
            }
            Self::Active => {
                let active_text = crate::app::runner_active_display(row.active);
                let (active_text, active_style) = if row.active {
                    (
                        format!("{} {}", ctx.icons.check_on, active_text),
                        Style::default()
                            .fg(theme.green)
                            .bg(if ctx.is_highlighted {
                                theme.highlight_bg
                            } else {
                                theme.green_bg
                            })
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    (
                        active_text.to_string(),
                        Style::default().fg(theme.text_muted),
                    )
                };
                fuzzy(&active_text, active_style, Alignment::Center)
            }
            Self::Type => optional_cell(&self.text(row), Style::default().fg(theme.purple), ctx),
            Self::Tags => optional_cell(
                &truncate(&self.text(row), TAGS_MAX_CHARS),
                Style::default()
                    .fg(theme.purple)
                    .add_modifier(Modifier::BOLD),
                ctx,
            ),
            Self::Version | Self::LastContact => {
                optional_cell(&self.text(row), Style::default().fg(theme.text_normal), ctx)
            }
        }
    }

    fn sort_key(self, row: &Runner) -> SortKey {
        match self {
            Self::Id => SortKey::Number(row.id),
            Self::LastContact => SortKey::Text(row.contacted_at.clone().unwrap_or_default()),
            Self::Description
            | Self::Status
            | Self::Active
            | Self::Type
            | Self::Tags
            | Self::Version => SortKey::Text(self.text(row).into_owned()),
        }
    }

    fn filter_values(self, row: &Runner) -> Vec<String> {
        match self {
            Self::Tags => row.tags.clone(),
            _ => cell_filter_values(self.text(row)),
        }
    }

    fn matches_filter(self, row: &Runner, selected: &std::collections::HashSet<String>) -> bool {
        match self {
            Self::Active => {
                let legacy = if row.active { "true" } else { "false" };
                selected.contains(crate::app::runner_active_display(row.active))
                    || selected.contains(legacy)
            }
            Self::Tags => row.tags.iter().any(|tag| selected.contains(tag)),
            Self::Id
            | Self::Description
            | Self::Status
            | Self::Type
            | Self::Version
            | Self::LastContact => {
                crate::app::column_filter_matches(self.label(), &self.filter_values(row), selected)
            }
        }
    }
}

/// A cell whose value the backend may not provide: `--` in muted text when
/// it is missing.
fn optional_cell(text: &str, known_style: Style, ctx: &CellContext) -> Cell<'static> {
    let (text, style) = if text.is_empty() {
        (UNKNOWN_VALUE, Style::default().fg(ctx.theme.text_muted))
    } else {
        (text, known_style)
    };
    render_fuzzy_cell(
        text,
        ctx.query,
        ctx.is_highlighted,
        false,
        style,
        Alignment::Left,
    )
}
