use std::borrow::Cow;
use std::collections::HashSet;

use ratatui::layout::{Alignment, Constraint};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Cell;

use super::{CellContext, ColumnDef, Row, SortKey};
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

    fn header_alignment(self) -> Alignment {
        match self {
            Self::Default | Self::Protected | Self::AheadBehind => Alignment::Center,
            Self::Name | Self::Sha | Self::Author | Self::Date => Alignment::Left,
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

    fn sort_key(self, row: &Branch) -> SortKey {
        match self {
            Self::Date => SortKey::Number(
                row.commit_date
                    .as_deref()
                    .and_then(|d| d.parse::<chrono::DateTime<chrono::Utc>>().ok())
                    .map_or(0, |t| t.timestamp().max(0) as u64),
            ),
            Self::AheadBehind => {
                SortKey::Number(row.ahead_behind.map_or(0, |(ahead, _)| ahead as u64))
            }
            _ => SortKey::Text(self.text(row).into_owned()),
        }
    }

    fn cell(self, row: &Branch, ctx: &CellContext) -> Cell<'static> {
        let theme = ctx.theme;
        let icons = ctx.icons;
        let fuzzy = |text: &str, style: Style, alignment: Alignment| {
            render_fuzzy_cell(text, ctx.query, ctx.is_highlighted, false, style, alignment)
        };
        match self {
            Self::Name => {
                let mut style = Style::default().fg(theme.purple);
                if row.default {
                    style = style.add_modifier(Modifier::BOLD);
                }
                fuzzy(
                    &format!("{} {}", icons.label_branch, row.name),
                    style,
                    Alignment::Left,
                )
            }
            Self::Default => {
                if row.default {
                    let text = format!(" {} YES ", icons.radio_on);
                    let style = Style::default()
                        .fg(theme.green)
                        .bg(if ctx.is_highlighted {
                            theme.highlight_bg
                        } else {
                            theme.green_bg
                        })
                        .add_modifier(Modifier::BOLD);
                    fuzzy(&text, style, Alignment::Center)
                } else {
                    fuzzy(
                        "NO",
                        Style::default().fg(theme.text_muted),
                        Alignment::Center,
                    )
                }
            }
            Self::Protected => {
                if row.protected {
                    let text = format!(" {} YES ", icons.lock);
                    let style = Style::default()
                        .fg(theme.yellow)
                        .bg(if ctx.is_highlighted {
                            theme.highlight_bg
                        } else {
                            theme.yellow_bg
                        })
                        .add_modifier(Modifier::BOLD);
                    fuzzy(&text, style, Alignment::Center)
                } else {
                    fuzzy(
                        "NO",
                        Style::default().fg(theme.text_muted),
                        Alignment::Center,
                    )
                }
            }
            Self::Sha => {
                if row.commit_sha.is_empty() {
                    fuzzy("--", Style::default().fg(theme.text_muted), Alignment::Left)
                } else {
                    fuzzy(
                        &truncate(&row.commit_sha, SHA_SHOWN_CHARS),
                        Style::default().fg(theme.text_muted),
                        Alignment::Left,
                    )
                }
            }
            Self::Author => {
                let (author, style) = match row.author_name.as_deref() {
                    Some(name) => (name, Style::default().fg(theme.blue)),
                    None => ("--", Style::default().fg(theme.text_muted)),
                };
                fuzzy(author, style, Alignment::Left)
            }
            Self::Date => {
                let (date_text, style) = match row.commit_date.as_deref() {
                    Some(d) => (
                        crate::utils::format::time_ago(d),
                        Style::default().fg(theme.yellow),
                    ),
                    None => ("--".to_string(), Style::default().fg(theme.text_muted)),
                };
                fuzzy(&date_text, style, Alignment::Left)
            }
            Self::AheadBehind => match row.ahead_behind {
                Some((ahead, behind)) => {
                    let item_bg = if ctx.is_highlighted {
                        theme.highlight_bg
                    } else {
                        Color::Reset
                    };
                    let ahead_style = if ahead > 0 {
                        Style::default()
                            .fg(theme.green)
                            .bg(item_bg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text_muted).bg(item_bg)
                    };
                    let behind_style = if behind > 0 {
                        Style::default()
                            .fg(theme.red)
                            .bg(item_bg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text_muted).bg(item_bg)
                    };
                    let spans = vec![
                        Span::styled(format!("+{ahead}"), ahead_style),
                        Span::styled(" / ", Style::default().fg(theme.text_muted).bg(item_bg)),
                        Span::styled(format!("-{behind}"), behind_style),
                    ];
                    Cell::from(Line::from(spans).alignment(Alignment::Center))
                }
                None => fuzzy(
                    "--",
                    Style::default().fg(theme.text_muted),
                    Alignment::Center,
                ),
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
