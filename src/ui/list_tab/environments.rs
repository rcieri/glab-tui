use std::borrow::Cow;

use ratatui::layout::{Alignment, Constraint};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Cell;

use super::{CellContext, ColumnDef, Row};
use crate::domain::deployments::Environment;
use crate::ui::helpers::render_fuzzy_cell;
use crate::utils::format::sanitize_untrusted;

impl Row for Environment {
    type Key = u64;

    fn key(&self) -> u64 {
        self.id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentColumn {
    Name,
    State,
    DeploymentStatus,
    Url,
}

impl ColumnDef for EnvironmentColumn {
    type Row = Environment;

    const ALL: &'static [Self] = &[Self::Name, Self::State, Self::DeploymentStatus, Self::Url];

    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::State => "State",
            Self::DeploymentStatus => "Deployment Status",
            Self::Url => "URL",
        }
    }

    fn is_default(self) -> bool {
        !matches!(self, Self::Url)
    }

    fn width(self) -> Constraint {
        match self {
            Self::Name => Constraint::Length(24),
            Self::State => Constraint::Length(12),
            Self::DeploymentStatus => Constraint::Length(20),
            Self::Url => Constraint::Fill(1),
        }
    }

    fn text(self, row: &Environment) -> Cow<'_, str> {
        Cow::Borrowed(match self {
            Self::Name => &row.name,
            Self::State => &row.state,
            Self::DeploymentStatus => row
                .last_deployment
                .as_ref()
                .map_or("N/A", |d| d.status.as_str()),
            Self::Url => row.external_url.as_deref().unwrap_or("-"),
        })
    }

    fn cell(self, row: &Environment, ctx: &CellContext) -> Cell<'static> {
        match self {
            Self::Url => Cell::from(Span::styled(
                sanitize_untrusted(&self.text(row)).into_owned(),
                Style::default().fg(ctx.theme.blue),
            )),
            Self::Name | Self::State | Self::DeploymentStatus => render_fuzzy_cell(
                &self.text(row),
                ctx.query,
                ctx.is_highlighted,
                false,
                ctx.row_style,
                Alignment::Left,
            ),
        }
    }

    fn searchable(self) -> bool {
        !matches!(self, Self::Url)
    }
}
