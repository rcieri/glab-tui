use std::borrow::Cow;

use ratatui::layout::{Alignment, Constraint};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Cell;

use super::{CellContext, ColumnDef, Row, SortKey};
use crate::app::runner_status_display;
use crate::domain::runners::Runner;
use crate::ui::helpers::render_fuzzy_cell;
use crate::utils::format::truncate;

const DESCRIPTION_MAX_CHARS: usize = 100;

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
}

impl ColumnDef for RunnerColumn {
    type Row = Runner;

    const ALL: &'static [Self] = &[Self::Id, Self::Description, Self::Status, Self::Active];

    fn label(self) -> &'static str {
        match self {
            Self::Id => "ID",
            Self::Description => "Description",
            Self::Status => "Status",
            Self::Active => "Active",
        }
    }

    fn is_default(self) -> bool {
        true
    }

    fn width(self) -> Constraint {
        match self {
            Self::Id => Constraint::Length(8),
            Self::Description => Constraint::Fill(1),
            Self::Status => Constraint::Length(12),
            Self::Active => Constraint::Length(10),
        }
    }

    fn header_alignment(self) -> Alignment {
        match self {
            Self::Status | Self::Active => Alignment::Center,
            Self::Id | Self::Description => Alignment::Left,
        }
    }

    fn text(self, row: &Runner) -> Cow<'_, str> {
        match self {
            Self::Id => Cow::Owned(row.id.to_string()),
            Self::Description => {
                Cow::Borrowed(row.description.as_deref().unwrap_or("No description"))
            }
            Self::Status => Cow::Borrowed(runner_status_display(&row.status)),
            Self::Active => Cow::Borrowed(crate::app::runner_active_display(row.active)),
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
                let status = runner_status_display(&row.status);
                let (icon, color, bg) = match status {
                    "ONLINE" => (&ctx.icons.runner_online, theme.green, theme.green_bg),
                    "PAUSED" => (&ctx.icons.runner_paused, theme.yellow, theme.yellow_bg),
                    "OFFLINE" => (&ctx.icons.runner_offline, theme.red, theme.red_bg),
                    _ => (
                        &ctx.icons.status_unknown,
                        theme.text_muted,
                        theme.inactive_bg,
                    ),
                };
                fuzzy(
                    &format!("{icon} {status}"),
                    Style::default()
                        .fg(color)
                        .bg(if ctx.is_highlighted {
                            theme.highlight_bg
                        } else {
                            bg
                        })
                        .add_modifier(Modifier::BOLD),
                    Alignment::Center,
                )
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
        }
    }

    fn sort_key(self, row: &Runner) -> SortKey {
        match self {
            Self::Id => SortKey::Number(row.id),
            Self::Description | Self::Status | Self::Active => {
                SortKey::Text(self.text(row).into_owned())
            }
        }
    }

    fn matches_filter(self, row: &Runner, selected: &std::collections::HashSet<String>) -> bool {
        match self {
            Self::Active => {
                let legacy = if row.active { "true" } else { "false" };
                selected.contains(crate::app::runner_active_display(row.active))
                    || selected.contains(legacy)
            }
            Self::Id | Self::Description | Self::Status => {
                crate::app::column_filter_matches(self.label(), &self.filter_values(row), selected)
            }
        }
    }
}
