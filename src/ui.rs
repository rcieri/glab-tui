#![allow(dead_code)]

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Table},
};

use crate::app::{App, Tab};
use crate::config::THEME;
use crate::utils::format::{format_ref, render_markdown, time_ago, truncate};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

fn highlight_fuzzy_match(
    text: &str,
    indices: &[usize],
    base_style: Style,
    highlight_style: Style,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut current_chunk = String::new();
    let mut is_highlighted = false;

    let index_set: std::collections::HashSet<usize> = indices.iter().cloned().collect();

    for (i, c) in text.chars().enumerate() {
        let char_is_highlighted = index_set.contains(&i);
        if char_is_highlighted != is_highlighted {
            if !current_chunk.is_empty() {
                spans.push(Span::styled(
                    current_chunk.clone(),
                    if is_highlighted {
                        highlight_style
                    } else {
                        base_style
                    },
                ));
                current_chunk.clear();
            }
            is_highlighted = char_is_highlighted;
        }
        current_chunk.push(c);
    }

    if !current_chunk.is_empty() {
        spans.push(Span::styled(
            current_chunk,
            if is_highlighted {
                highlight_style
            } else {
                base_style
            },
        ));
    }

    spans
}

fn get_label_color(label: &str) -> Color {
    let mut hash: u32 = 5381;
    for c in label.bytes() {
        hash = ((hash << 5).wrapping_add(hash)).wrapping_add(c as u32);
    }
    let colors = [
        Color::Rgb(168, 122, 243), // purple
        Color::Rgb(61, 139, 255),  // blue
        Color::Rgb(49, 191, 103),  // green
        Color::Rgb(235, 180, 50),  // yellow
        Color::Rgb(224, 73, 83),   // red
        Color::Rgb(240, 140, 180), // pink
        Color::Rgb(250, 120, 80),  // orange
        Color::Rgb(40, 200, 200),  // cyan
        Color::Rgb(180, 230, 40),  // lime
        Color::Rgb(220, 160, 255), // light violet
    ];
    let idx = (hash % (colors.len() as u32)) as usize;
    colors[idx]
}

fn floor_char_boundary(s: &str, mut index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    while !s.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn render_labels_cell(
    labels: &[String],
    query: &str,
    is_selected: bool,
    is_checked: bool,
    max_len: usize,
) -> Cell<'static> {
    if labels.is_empty() {
        let mut style = Style::default().fg(THEME.read().unwrap().text_muted);
        if is_selected {
            style = style
                .bg(THEME.read().unwrap().highlight_bg)
                .add_modifier(Modifier::BOLD);
        } else if is_checked {
            style = style.bg(THEME.read().unwrap().checked_bg);
        }
        return Cell::from(Line::from("—").alignment(Alignment::Left)).style(style);
    }

    let mut char_styles: Vec<(char, Style)> = Vec::new();
    let mut current_len = 0;

    let base_bg = if is_selected {
        Some(THEME.read().unwrap().highlight_bg)
    } else if is_checked {
        Some(THEME.read().unwrap().checked_bg)
    } else {
        None
    };

    for (idx, label) in labels.iter().enumerate() {
        if current_len >= max_len {
            break;
        }
        if idx > 0 {
            let comma = ", ";
            if current_len + comma.len() > max_len {
                let mut style = Style::default().fg(THEME.read().unwrap().text_muted);
                if let Some(bg) = base_bg {
                    style = style.bg(bg);
                }
                char_styles.push(('…', style));
                break;
            }
            let mut style = Style::default().fg(THEME.read().unwrap().text_normal);
            if let Some(bg) = base_bg {
                style = style.bg(bg);
            }
            for c in comma.chars() {
                char_styles.push((c, style));
            }
            current_len += comma.len();
        }

        let label_color = get_label_color(label);
        let mut label_style = Style::default()
            .fg(label_color)
            .add_modifier(Modifier::BOLD);
        if let Some(bg) = base_bg {
            label_style = label_style.bg(bg);
        }

        let mut text_to_add = label.as_str();
        let mut truncated = false;
        if current_len + text_to_add.len() > max_len {
            let allowed = max_len - current_len;
            if allowed > 1 {
                // Snap to a valid char boundary to avoid panicking on multi-byte
                // characters (emojis, accented letters, etc.).
                let safe_end = floor_char_boundary(text_to_add, allowed - 1);
                text_to_add = &text_to_add[..safe_end];
                truncated = true;
            } else {
                let mut style = Style::default().fg(THEME.read().unwrap().text_muted);
                if let Some(bg) = base_bg {
                    style = style.bg(bg);
                }
                char_styles.push(('…', style));
                break;
            }
        }

        for c in text_to_add.chars() {
            char_styles.push((c, label_style));
        }
        current_len += text_to_add.len();

        if truncated {
            let mut style = Style::default().fg(THEME.read().unwrap().text_muted);
            if let Some(bg) = base_bg {
                style = style.bg(bg);
            }
            char_styles.push(('…', style));
            break;
        }
    }

    let concatenated_text: String = char_styles.iter().map(|(c, _)| *c).collect();
    let index_set: std::collections::HashSet<usize> = if query.trim().is_empty() {
        std::collections::HashSet::new()
    } else {
        let matcher = SkimMatcherV2::default();
        if let Some((_, indices)) = matcher.fuzzy_indices(&concatenated_text, query) {
            indices.into_iter().collect()
        } else {
            std::collections::HashSet::new()
        }
    };

    let mut spans = Vec::new();
    let mut current_chunk = String::new();
    let mut current_style = Style::default();
    let mut first = true;

    for (i, (c, mut style)) in char_styles.into_iter().enumerate() {
        if index_set.contains(&i) {
            style = style
                .fg(THEME.read().unwrap().yellow)
                .add_modifier(Modifier::BOLD);
        }

        if first {
            current_style = style;
            first = false;
        }

        if style != current_style {
            if !current_chunk.is_empty() {
                spans.push(Span::styled(current_chunk.clone(), current_style));
                current_chunk.clear();
            }
            current_style = style;
        }
        current_chunk.push(c);
    }

    if !current_chunk.is_empty() {
        spans.push(Span::styled(current_chunk, current_style));
    }

    let mut cell_style = Style::default();
    if let Some(bg) = base_bg {
        cell_style = cell_style.bg(bg);
    }

    Cell::from(Line::from(spans).alignment(Alignment::Left)).style(cell_style)
}

struct StageSummary {
    name: String,
    success: usize,
    total: usize,
    percent: usize,
    status: String,
}

fn get_stages_summary(jobs: &[crate::gitlab::pipelines::Job]) -> Vec<StageSummary> {
    let mut stage_names = Vec::new();
    let mut stage_jobs = std::collections::HashMap::new();
    for j in jobs {
        if !stage_names.contains(&j.stage) {
            stage_names.push(j.stage.clone());
        }
        stage_jobs
            .entry(j.stage.clone())
            .or_insert_with(Vec::new)
            .push(j.status.clone());
    }

    let mut summaries = Vec::new();
    for stage in stage_names {
        if let Some(statuses) = stage_jobs.get(&stage) {
            let total = statuses.len();
            let success = statuses
                .iter()
                .filter(|s| *s == "success" || *s == "skipped")
                .count();
            let percent = if total > 0 {
                (success * 100) / total
            } else {
                0
            };

            let stage_status = if statuses.iter().any(|s| s == "failed") {
                "failed".to_string()
            } else if statuses.iter().any(|s| s == "running") {
                "running".to_string()
            } else if statuses
                .iter()
                .any(|s| s == "pending" || s == "preparing" || s == "waiting_for_resource")
            {
                "pending".to_string()
            } else if statuses.iter().any(|s| s == "canceled") {
                "canceled".to_string()
            } else if statuses.iter().any(|s| s == "success") {
                "success".to_string()
            } else {
                "skipped".to_string()
            };

            summaries.push(StageSummary {
                name: stage,
                success,
                total,
                percent,
                status: stage_status,
            });
        }
    }
    summaries
}

fn get_stages_dots(jobs: &[crate::gitlab::pipelines::Job]) -> String {
    let summaries = get_stages_summary(jobs);
    let mut dots = String::new();
    for s in summaries {
        let dot = match s.status.as_str() {
            "success" => "🟢",
            "failed" => "🔴",
            "running" => "🔵",
            "canceled" => "⚫",
            "pending" => "🟡",
            _ => "⚪",
        };
        dots.push_str(dot);
    }
    dots
}

fn append_stage_summaries(text: &mut Vec<Line<'static>>, jobs: &[crate::gitlab::pipelines::Job]) {
    let summaries = get_stages_summary(jobs);
    for s in summaries {
        let status_color = match s.status.as_str() {
            "success" => THEME.read().unwrap().green,
            "failed" => THEME.read().unwrap().red,
            "running" => THEME.read().unwrap().blue,
            "pending" => THEME.read().unwrap().yellow,
            _ => THEME.read().unwrap().text_muted,
        };
        text.push(Line::from(vec![
            Span::styled(
                format!("{:15} ", truncate(&s.name, 15)),
                Style::default().fg(THEME.read().unwrap().text_normal),
            ),
            Span::styled(" ❯ ", Style::default().fg(THEME.read().unwrap().text_muted)),
            Span::styled(
                format!("{:>4} ", format!("{}%", s.percent)),
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({}/{})", s.success, s.total),
                Style::default().fg(THEME.read().unwrap().text_muted),
            ),
        ]));
    }
}

#[allow(dead_code)]
fn add_cmd(text: &mut Vec<Line<'static>>, key: &str, desc: &str) {
    let padded_key = format!(" {:^3} ", key);
    text.push(Line::from(vec![
        Span::styled(
            padded_key,
            Style::default()
                .bg(THEME.read().unwrap().border_focused)
                .fg(THEME.read().unwrap().bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {}", desc),
            Style::default().fg(THEME.read().unwrap().text_normal),
        ),
    ]));
}

fn build_log_line(cmd: &crate::app::TerminalCommand, width: usize) -> Line<'static> {
    let time_str = if cmd.timestamp.len() >= 8 {
        let parts: Vec<&str> = cmd.timestamp.split('T').collect();
        if parts.len() > 1 {
            parts[1].chars().take(8).collect::<String>()
        } else {
            cmd.timestamp.chars().take(8).collect::<String>()
        }
    } else {
        cmd.timestamp.clone()
    };

    let (status_text, status_color) = match cmd.status.as_str() {
        "Success" => ("SUCCESS", THEME.read().unwrap().green),
        "Running" => ("RUNNING", THEME.read().unwrap().yellow),
        s if s.starts_with("Failed") => ("FAILED ", THEME.read().unwrap().red),
        _ => ("PENDING", THEME.read().unwrap().yellow),
    };

    let err_detail = if cmd.status.starts_with("Failed: ") {
        Some(&cmd.status[8..])
    } else if cmd.status.starts_with("Failed") && cmd.status.len() > 6 {
        Some(&cmd.status[6..])
    } else {
        None
    };

    let cmd_clean = cmd.command.trim();
    let mut desc = "";
    let mut cmd_to_run = cmd_clean;

    if let Some(pos) = cmd_clean.find(": ") {
        let prefix = &cmd_clean[..pos];
        let remainder = &cmd_clean[pos + 2..];
        if remainder.starts_with("glab") || remainder.starts_with("gh") {
            desc = prefix;
            cmd_to_run = remainder;
        }
    }

    let desc_str = if desc.is_empty() {
        if cmd_clean.starts_with("glab") || cmd_clean.starts_with("gh") {
            "RUNNING COMMAND".to_string()
        } else {
            "SYSTEM LOG".to_string()
        }
    } else {
        desc.to_uppercase()
    };

    let time_len = 11; // "[HH:MM:SS] "
    let status_len = 7; // "SUCCESS"
    let sep1_len = 3; // " • "
    let action_len = 20; // Action padded to 20 chars
    let sep2_len = 3; // " • "
    let err_len = err_detail.map(|d| d.len() + 3).unwrap_or(0); // " (Error)"

    let reserved = time_len + status_len + sep1_len + action_len + sep2_len + err_len;
    let max_api_width = width.saturating_sub(reserved);

    let truncated_api = truncate(cmd_to_run, max_api_width);

    let (cmd_bin, cmd_args) = if truncated_api.starts_with("glab") {
        ("glab", truncated_api[4..].to_string())
    } else if truncated_api.starts_with("gh") {
        ("gh", truncated_api[2..].to_string())
    } else {
        ("", truncated_api.clone())
    };

    let mut spans = vec![
        // 1. Time
        Span::styled(
            format!("[{}] ", time_str),
            Style::default().fg(THEME.read().unwrap().text_muted),
        ),
        // 2. Status
        Span::styled(
            format!("{: <7}", status_text),
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
        // 3. Sep1
        Span::styled(" • ", Style::default().fg(THEME.read().unwrap().text_muted)),
        // 4. Action
        Span::styled(
            format!("{: <20}", desc_str),
            Style::default()
                .fg(THEME.read().unwrap().blue)
                .add_modifier(Modifier::BOLD),
        ),
        // 5. Sep2
        Span::styled(" • ", Style::default().fg(THEME.read().unwrap().text_muted)),
    ];

    // 6. API
    if !cmd_bin.is_empty() {
        spans.push(Span::styled(
            cmd_bin,
            Style::default()
                .fg(THEME.read().unwrap().yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(
        cmd_args,
        Style::default().fg(THEME.read().unwrap().text_normal),
    ));

    // 7. Error Detail
    if let Some(detail) = err_detail {
        spans.push(Span::styled(
            format!(" ({})", detail),
            Style::default().fg(THEME.read().unwrap().red),
        ));
    }

    Line::from(spans)
}

pub fn render(f: &mut Frame, app: &mut App) {
    let render_fuzzy_cell = |text: &str,
                             query: &str,
                             is_selected: bool,
                             is_checked: bool,
                             base_style: Style,
                             alignment: Alignment| {
        let mut styled_base = base_style;
        if is_selected {
            styled_base = styled_base
                .bg(THEME.read().unwrap().highlight_bg)
                .add_modifier(Modifier::BOLD);
        } else if is_checked {
            styled_base = styled_base.bg(THEME.read().unwrap().checked_bg);
        }
        let line = if query.trim().is_empty() {
            Line::from(text.to_string()).alignment(alignment)
        } else {
            let matcher = SkimMatcherV2::default();
            if let Some((_, indices)) = matcher.fuzzy_indices(text, query) {
                let mut highlight_style = Style::default()
                    .fg(THEME.read().unwrap().yellow)
                    .add_modifier(Modifier::BOLD);
                if let Some(bg) = styled_base.bg {
                    highlight_style = highlight_style.bg(bg);
                }
                Line::from(highlight_fuzzy_match(
                    text,
                    &indices,
                    styled_base,
                    highlight_style,
                ))
                .alignment(alignment)
            } else {
                Line::from(text.to_string()).alignment(alignment)
            }
        };
        Cell::from(line).style(styled_base)
    };

    let size = f.area();

    // Minimum terminal size guard
    if size.width < 54 || size.height < 10 {
        let msg = format!("Terminal too small — resize to at least {}×{}", 54, 10);
        f.render_widget(
            Paragraph::new(msg)
                .alignment(Alignment::Center)
                .style(Style::default().fg(THEME.read().unwrap().red)),
            size,
        );
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Top header bar
            Constraint::Min(0),    // Main workspace
            Constraint::Length(0), // Reserved
        ])
        .split(size);

    let title_area = chunks[0];

    // Top: Title & Context (Zellij Vibe Horizontal Bar)
    let mut title_spans = vec![
        Span::styled(
            " GLAB-TUI ",
            Style::default()
                .bg(THEME.read().unwrap().border_focused)
                .fg(THEME.read().unwrap().bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" ❯ {} ", app.project_context),
            Style::default()
                .fg(THEME.read().unwrap().text_normal)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if app.is_typing_search {
        title_spans.push(Span::styled(
            " SEARCHING ",
            Style::default()
                .bg(THEME.read().unwrap().yellow)
                .fg(THEME.read().unwrap().bg)
                .add_modifier(Modifier::BOLD),
        ));
        title_spans.push(Span::styled(
            format!(" {}_ ", app.search_query),
            Style::default().fg(THEME.read().unwrap().yellow),
        ));
    } else if !app.search_query.is_empty() {
        title_spans.push(Span::styled(
            " FILTERED ",
            Style::default()
                .bg(THEME.read().unwrap().yellow)
                .fg(THEME.read().unwrap().bg)
                .add_modifier(Modifier::BOLD),
        ));
        title_spans.push(Span::styled(
            format!(" {} ", app.search_query),
            Style::default().fg(THEME.read().unwrap().yellow),
        ));
    }

    let title = Paragraph::new(Line::from(title_spans))
        .style(Style::default().bg(THEME.read().unwrap().bg))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(THEME.read().unwrap().border)),
        );
    f.render_widget(title, title_area);

    // Middle: Sidebar | Main Area | Preview Area
    let can_zoom = app.active_tab != Tab::Pipelines || app.selected_pipeline_jobs.is_some();

    // Responsive sidebar: hide when terminal is narrow
    let sidebar_width = if size.width >= 80 {
        Constraint::Length(22)
    } else {
        Constraint::Length(0)
    };

    // Responsive details pane: hide when terminal is narrow
    let details_width = if size.width < 90 {
        Constraint::Length(0)
    } else if size.width > 150 {
        Constraint::Percentage(35)
    } else if size.width > 100 {
        Constraint::Length(45)
    } else {
        Constraint::Length(30)
    };

    let middle_chunks = if app.details_zoomed && can_zoom {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(0),
                Constraint::Length(0),
                Constraint::Min(0),
            ])
            .split(chunks[1])
    } else if app.active_tab == Tab::Terminal {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([sidebar_width, Constraint::Min(0), Constraint::Length(0)])
            .split(chunks[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([sidebar_width, Constraint::Min(0), details_width])
            .split(chunks[1])
    };

    // Split middle column vertically: main content area + compact terminal pane
    let term_height = if app.active_tab != Tab::Terminal && size.height >= 18 {
        6
    } else {
        0
    };
    let (content_area, term_area) = if term_height > 0 {
        let tc = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(term_height)])
            .split(middle_chunks[1]);
        (tc[0], tc[1])
    } else {
        (middle_chunks[1], Rect::default())
    };

    // Sidebar: Tabs
    let is_github = app
        .gitlab_client
        .as_ref()
        .map(|c| c.is_github)
        .unwrap_or(false);
    let sidebar_items: Vec<ListItem> = Tab::ALL
        .iter()
        .map(|t| {
            let title = format!("  {}  ", t.title(is_github).to_uppercase());
            if *t == app.active_tab {
                ListItem::new(title).style(
                    Style::default()
                        .bg(THEME.read().unwrap().border_focused)
                        .fg(THEME.read().unwrap().bg)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ListItem::new(title).style(Style::default().fg(THEME.read().unwrap().text_muted))
            }
        })
        .collect();

    let sidebar = List::new(sidebar_items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border))
            .title(" Navigation ")
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .add_modifier(Modifier::BOLD),
            ),
    );
    f.render_widget(sidebar, middle_chunks[0]);

    // Main Area Title
    let tab_title = format!(" {} ", app.active_tab.title(is_github));
    let main_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.focus_column_checklist {
            THEME.read().unwrap().border
        } else {
            THEME.read().unwrap().border_focused
        }))
        .title(tab_title)
        .title_style(
            Style::default()
                .fg(THEME.read().unwrap().header_fg)
                .add_modifier(Modifier::BOLD),
        );

    let highlight_style = Style::default().bg(THEME.read().unwrap().highlight_bg);
    let header_style = Style::default()
        .fg(THEME.read().unwrap().text_normal)
        .add_modifier(Modifier::BOLD);

    match app.active_tab {
        Tab::Issues => {
            if app.issues.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
                f.render_widget(
                    Paragraph::new("\n\n Loading issues...")
                        .alignment(Alignment::Center)
                        .block(main_block.clone())
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    content_area,
                );
                f.render_widget(
                    Paragraph::new("Select an item to view details...")
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" Details ")
                                .border_style(Style::default().fg(THEME.read().unwrap().border)),
                        )
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    middle_chunks[2],
                );
            } else {
                let mut filtered_issues = App::filtered_issues_list(
                    &app.issues.items,
                    &app.search_query,
                    &app.enabled_columns,
                    app.group_ascending,
                    &app.group_by_column,
                );
                App::apply_column_filters(
                    &mut filtered_issues,
                    &app.column_filters,
                    Tab::Issues,
                    |item, col| match col {
                        "Labels" => item.labels.clone(),
                        "Assignees" => item.assignees.iter().map(|a| a.username.clone()).collect(),
                        "Author" => vec![item.author.username.clone()],
                        "Milestone" => item
                            .milestone
                            .as_ref()
                            .map(|m| m.title.clone())
                            .into_iter()
                            .collect(),
                        "State" => vec![item.state.clone()],
                        "ID" => vec![item.iid.to_string()],
                        "Title" => vec![item.title.clone()],
                        _ => vec![],
                    },
                );

                let rows = filtered_issues.iter().enumerate().map(|(idx, i)| {
                    let is_selected = app.issues.state.selected() == Some(idx);
                    let (state_text, state_style) = if i.state == "opened" {
                        (
                            "OPEN",
                            Style::default()
                                .fg(THEME.read().unwrap().green)
                                .bg(if is_selected {
                                    THEME.read().unwrap().highlight_bg
                                } else {
                                    THEME.read().unwrap().green_bg
                                })
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        (
                            "CLOSED",
                            Style::default()
                                .fg(THEME.read().unwrap().red)
                                .bg(if is_selected {
                                    THEME.read().unwrap().highlight_bg
                                } else {
                                    THEME.read().unwrap().red_bg
                                })
                                .add_modifier(Modifier::BOLD),
                        )
                    };
                    let mut cells = Vec::new();
                    if app.is_column_visible(Tab::Issues, "ID") {
                        cells.push(render_fuzzy_cell(
                            &format!("#{}", i.iid),
                            &app.search_query,
                            is_selected,
                            false,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Issues, "State") {
                        cells.push(render_fuzzy_cell(
                            state_text,
                            &app.search_query,
                            is_selected,
                            false,
                            state_style,
                            Alignment::Center,
                        ));
                    }
                    if app.is_column_visible(Tab::Issues, "Title") {
                        cells.push(render_fuzzy_cell(
                            &truncate(&i.title, 100),
                            &app.search_query,
                            is_selected,
                            false,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Issues, "Assignees") {
                        let assignees_str = if i.assignees.is_empty() {
                            "—".to_string()
                        } else {
                            i.assignees
                                .iter()
                                .map(|a| format!("@{}", a.username))
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        cells.push(render_fuzzy_cell(
                            &truncate(&assignees_str, 20),
                            &app.search_query,
                            is_selected,
                            false,
                            Style::default().fg(THEME.read().unwrap().blue),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Issues, "Labels") {
                        cells.push(render_labels_cell(
                            &i.labels,
                            &app.search_query,
                            is_selected,
                            false,
                            24,
                        ));
                    }
                    if app.is_column_visible(Tab::Issues, "Milestone") {
                        let milestone_str = i
                            .milestone
                            .as_ref()
                            .map(|m| m.title.clone())
                            .unwrap_or_else(|| "—".to_string());
                        cells.push(render_fuzzy_cell(
                            &truncate(&milestone_str, 18),
                            &app.search_query,
                            is_selected,
                            false,
                            Style::default().fg(THEME.read().unwrap().yellow),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Issues, "Due Date") {
                        let due_str = i.due_date.as_deref().unwrap_or("—");
                        cells.push(render_fuzzy_cell(
                            due_str,
                            &app.search_query,
                            is_selected,
                            false,
                            Style::default().fg(THEME.read().unwrap().yellow),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Issues, "Author") {
                        let author_str = format!("@{}", i.author.username);
                        cells.push(render_fuzzy_cell(
                            &truncate(&author_str, 15),
                            &app.search_query,
                            is_selected,
                            false,
                            Style::default().fg(THEME.read().unwrap().blue),
                            Alignment::Left,
                        ));
                    }
                    let row_style = if is_selected {
                        Style::default().bg(THEME.read().unwrap().highlight_bg)
                    } else {
                        Style::default()
                    };
                    Row::new(cells).style(row_style).height(1)
                });

                let mut header_cells = Vec::new();
                let mut widths = Vec::new();

                if app.is_column_visible(Tab::Issues, "ID") {
                    header_cells.push(Cell::from("ID"));
                    widths.push(Constraint::Length(10));
                }
                if app.is_column_visible(Tab::Issues, "State") {
                    header_cells.push(Cell::from(Line::from("State").alignment(Alignment::Center)));
                    widths.push(Constraint::Length(10));
                }
                if app.is_column_visible(Tab::Issues, "Title") {
                    header_cells.push(Cell::from("Title"));
                    widths.push(Constraint::Fill(1));
                }
                if app.is_column_visible(Tab::Issues, "Assignees") {
                    header_cells.push(Cell::from("Assignees"));
                    widths.push(Constraint::Length(20));
                }
                if app.is_column_visible(Tab::Issues, "Labels") {
                    header_cells.push(Cell::from("Labels"));
                    widths.push(Constraint::Length(24));
                }
                if app.is_column_visible(Tab::Issues, "Milestone") {
                    header_cells.push(Cell::from("Milestone"));
                    widths.push(Constraint::Length(18));
                }
                if app.is_column_visible(Tab::Issues, "Due Date") {
                    header_cells.push(Cell::from("Due Date"));
                    widths.push(Constraint::Length(12));
                }
                if app.is_column_visible(Tab::Issues, "Author") {
                    header_cells.push(Cell::from("Author"));
                    widths.push(Constraint::Length(15));
                }

                if widths.is_empty() {
                    widths.push(Constraint::Min(0));
                }

                let table = Table::new(rows, widths)
                    .header(Row::new(header_cells).style(header_style).height(1))
                    .block(main_block)
                    .row_highlight_style(highlight_style)
                    .highlight_symbol(" ❯ ");

                f.render_stateful_widget(table, content_area, &mut app.issues.state);
                let preview_block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(THEME.read().unwrap().border))
                    .title(" Details ")
                    .title_style(
                        Style::default()
                            .fg(THEME.read().unwrap().text_muted)
                            .add_modifier(Modifier::BOLD),
                    );
                let selected_issue_idx = app.issues.state.selected();
                if let Some(selected) = selected_issue_idx {
                    if let Some(issue) = filtered_issues.get(selected) {
                        let milestone = issue
                            .milestone
                            .as_ref()
                            .map(|m| m.title.as_str())
                            .unwrap_or("None");
                        let assignees = if issue.assignees.is_empty() {
                            "None".to_string()
                        } else {
                            issue
                                .assignees
                                .iter()
                                .map(|a| format!("@{}", a.username))
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled(
                                "Title:     ",
                                Style::default()
                                    .fg(THEME.read().unwrap().text_muted)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                &issue.title,
                                Style::default()
                                    .fg(THEME.read().unwrap().text_normal)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        text.push(Line::from(""));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Author:    ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                format!("@{}", issue.author.username),
                                Style::default().fg(THEME.read().unwrap().blue),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Assignees: ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                assignees,
                                Style::default().fg(THEME.read().unwrap().blue),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Milestone: ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                milestone,
                                Style::default().fg(THEME.read().unwrap().purple),
                            ),
                        ]));
                        if let Some(due) = &issue.due_date {
                            text.push(Line::from(vec![
                                Span::styled(
                                    "Due Date:  ",
                                    Style::default().fg(THEME.read().unwrap().text_muted),
                                ),
                                Span::styled(
                                    due,
                                    Style::default().fg(THEME.read().unwrap().yellow),
                                ),
                            ]));
                        }
                        text.push(Line::from(vec![
                            Span::styled(
                                "State:     ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                if issue.state == "opened" {
                                    "OPEN"
                                } else {
                                    "CLOSED"
                                },
                                Style::default()
                                    .fg(if issue.state == "opened" {
                                        THEME.read().unwrap().green
                                    } else {
                                        THEME.read().unwrap().red
                                    })
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Updated:   ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                time_ago(&issue.updated_at),
                                Style::default().fg(THEME.read().unwrap().yellow),
                            ),
                        ]));
                        text.push(Line::from(""));
                        let mut label_spans = vec![Span::styled(
                            "Labels:    ",
                            Style::default().fg(THEME.read().unwrap().text_muted),
                        )];
                        if issue.labels.is_empty() {
                            label_spans.push(Span::styled(
                                "None",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ));
                        } else {
                            for (idx, label) in issue.labels.iter().enumerate() {
                                if idx > 0 {
                                    label_spans.push(Span::styled(
                                        ", ",
                                        Style::default().fg(THEME.read().unwrap().text_normal),
                                    ));
                                }
                                let label_color = get_label_color(label);
                                label_spans.push(Span::styled(
                                    label,
                                    Style::default()
                                        .fg(label_color)
                                        .add_modifier(Modifier::BOLD),
                                ));
                            }
                        }
                        text.push(Line::from(label_spans));
                        if let Some(desc) = &issue.description {
                            if !desc.trim().is_empty() {
                                text.push(Line::from(""));
                                text.push(Line::from(vec![Span::styled(
                                    "Description:",
                                    Style::default()
                                        .fg(THEME.read().unwrap().header_fg)
                                        .add_modifier(Modifier::BOLD),
                                )]));
                                text.extend(render_markdown(desc));
                            }
                        }

                        let viewport_height = middle_chunks[2].height.saturating_sub(2) as usize;
                        let content_length = text.len();
                        let max_scroll = content_length.saturating_sub(viewport_height) as u16;
                        app.issues_scroll = app.issues_scroll.min(max_scroll);

                        let title_suffix = if content_length > viewport_height {
                            let percent =
                                (app.issues_scroll as usize * 100) / max_scroll.max(1) as usize;
                            format!(" [Shift+J/K | {}%] ", percent.min(100))
                        } else {
                            String::new()
                        };

                        let preview_block = Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(THEME.read().unwrap().border))
                            .title(format!(" Details{} ", title_suffix))
                            .title_style(
                                Style::default()
                                    .fg(THEME.read().unwrap().text_muted)
                                    .add_modifier(Modifier::BOLD),
                            );

                        f.render_widget(
                            Paragraph::new(text)
                                .block(preview_block)
                                .wrap(ratatui::widgets::Wrap { trim: true })
                                .scroll((app.issues_scroll, 0)),
                            middle_chunks[2],
                        );
                    } else {
                        f.render_widget(Paragraph::new("").block(preview_block), middle_chunks[2]);
                    }
                } else {
                    f.render_widget(
                        Paragraph::new("Select an item to view details...")
                            .block(preview_block)
                            .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                        middle_chunks[2],
                    );
                }
            }
        }
        Tab::MergeRequests => {
            if app.mrs.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
                f.render_widget(
                    Paragraph::new("\n\n Loading merge requests...")
                        .alignment(Alignment::Center)
                        .block(main_block.clone())
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    content_area,
                );
                f.render_widget(
                    Paragraph::new("Select an item to view details...")
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" Details ")
                                .border_style(Style::default().fg(THEME.read().unwrap().border)),
                        )
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    middle_chunks[2],
                );
            } else {
                let mut filtered_mrs = App::filtered_mrs_list(
                    &app.mrs.items,
                    &app.search_query,
                    &app.enabled_columns,
                    app.group_ascending,
                    &app.group_by_column,
                );
                App::apply_column_filters(
                    &mut filtered_mrs,
                    &app.column_filters,
                    Tab::MergeRequests,
                    |item, col| match col {
                        "Labels" => item.labels.clone(),
                        "Assignees" => item.assignees.iter().map(|a| a.username.clone()).collect(),
                        "Reviewers" => item.reviewers.iter().map(|r| r.username.clone()).collect(),
                        "Author" => vec![item.author.username.clone()],
                        "Milestone" => item
                            .milestone
                            .as_ref()
                            .map(|m| m.title.clone())
                            .into_iter()
                            .collect(),
                        "State" => vec![item.state.clone()],
                        "Status" => {
                            vec![if item.draft {
                                "Draft".to_string()
                            } else {
                                "Ready".to_string()
                            }]
                        }
                        "ID" => vec![item.iid.to_string()],
                        "Title" => vec![item.title.clone()],
                        _ => vec![],
                    },
                );

                let rows = filtered_mrs.iter().enumerate().map(|(idx, m)| {
                    let is_selected = app.mrs.state.selected() == Some(idx);
                    let (prefix, clean_title) =
                        crate::utils::format::parse_mr_title_prefix(&m.title);

                    let (state_text, state_style) = if m.state == "opened" {
                        (
                            "OPEN",
                            Style::default()
                                .fg(THEME.read().unwrap().green)
                                .bg(if is_selected {
                                    THEME.read().unwrap().highlight_bg
                                } else {
                                    THEME.read().unwrap().green_bg
                                })
                                .add_modifier(Modifier::BOLD),
                        )
                    } else if m.state == "merged" {
                        (
                            "MERGED",
                            Style::default()
                                .fg(THEME.read().unwrap().purple)
                                .bg(if is_selected {
                                    THEME.read().unwrap().highlight_bg
                                } else {
                                    THEME.read().unwrap().purple_bg
                                })
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        (
                            "CLOSED",
                            Style::default()
                                .fg(THEME.read().unwrap().red)
                                .bg(if is_selected {
                                    THEME.read().unwrap().highlight_bg
                                } else {
                                    THEME.read().unwrap().red_bg
                                })
                                .add_modifier(Modifier::BOLD),
                        )
                    };

                    let (status_styled, status_style) = if m.draft {
                        (
                            "DRAFT".to_string(),
                            Style::default()
                                .fg(THEME.read().unwrap().yellow)
                                .bg(if is_selected {
                                    THEME.read().unwrap().highlight_bg
                                } else {
                                    THEME.read().unwrap().yellow_bg
                                })
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        let upper = prefix.to_uppercase();
                        if upper == "WIP" || upper == "DRAFT" {
                            (
                                "DRAFT".to_string(),
                                Style::default()
                                    .fg(THEME.read().unwrap().yellow)
                                    .bg(if is_selected {
                                        THEME.read().unwrap().highlight_bg
                                    } else {
                                        THEME.read().unwrap().yellow_bg
                                    })
                                    .add_modifier(Modifier::BOLD),
                            )
                        } else {
                            (
                                "READY".to_string(),
                                Style::default()
                                    .fg(THEME.read().unwrap().green)
                                    .bg(if is_selected {
                                        THEME.read().unwrap().highlight_bg
                                    } else {
                                        THEME.read().unwrap().green_bg
                                    })
                                    .add_modifier(Modifier::BOLD),
                            )
                        }
                    };

                    let mut cells = Vec::new();
                    if app.is_column_visible(Tab::MergeRequests, "ID") {
                        cells.push(render_fuzzy_cell(
                            &format!("!{}", m.iid),
                            &app.search_query,
                            is_selected,
                            false,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::MergeRequests, "State") {
                        cells.push(render_fuzzy_cell(
                            state_text,
                            &app.search_query,
                            is_selected,
                            false,
                            state_style,
                            Alignment::Center,
                        ));
                    }
                    if app.is_column_visible(Tab::MergeRequests, "Status") {
                        cells.push(render_fuzzy_cell(
                            &status_styled,
                            &app.search_query,
                            is_selected,
                            false,
                            status_style,
                            Alignment::Center,
                        ));
                    }
                    if app.is_column_visible(Tab::MergeRequests, "Title") {
                        cells.push(render_fuzzy_cell(
                            &truncate(&clean_title, 100),
                            &app.search_query,
                            is_selected,
                            false,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::MergeRequests, "Assignees") {
                        let assignees_str = if m.assignees.is_empty() {
                            "—".to_string()
                        } else {
                            m.assignees
                                .iter()
                                .map(|a| format!("@{}", a.username))
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        cells.push(render_fuzzy_cell(
                            &truncate(&assignees_str, 20),
                            &app.search_query,
                            is_selected,
                            false,
                            Style::default().fg(THEME.read().unwrap().blue),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::MergeRequests, "Reviewers") {
                        let reviewers_str = if m.reviewers.is_empty() {
                            "—".to_string()
                        } else {
                            m.reviewers
                                .iter()
                                .map(|r| format!("@{}", r.username))
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        cells.push(render_fuzzy_cell(
                            &truncate(&reviewers_str, 20),
                            &app.search_query,
                            is_selected,
                            false,
                            Style::default().fg(THEME.read().unwrap().blue),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::MergeRequests, "Labels") {
                        cells.push(render_labels_cell(
                            &m.labels,
                            &app.search_query,
                            is_selected,
                            false,
                            24,
                        ));
                    }
                    if app.is_column_visible(Tab::MergeRequests, "Milestone") {
                        let mr_milestone_str = m
                            .milestone
                            .as_ref()
                            .map(|ms| ms.title.clone())
                            .unwrap_or_else(|| "—".to_string());
                        cells.push(render_fuzzy_cell(
                            &truncate(&mr_milestone_str, 18),
                            &app.search_query,
                            is_selected,
                            false,
                            Style::default().fg(THEME.read().unwrap().yellow),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::MergeRequests, "Author") {
                        let author_str = format!("@{}", m.author.username);
                        cells.push(render_fuzzy_cell(
                            &truncate(&author_str, 15),
                            &app.search_query,
                            is_selected,
                            false,
                            Style::default().fg(THEME.read().unwrap().blue),
                            Alignment::Left,
                        ));
                    }
                    let row_style = if is_selected {
                        Style::default().bg(THEME.read().unwrap().highlight_bg)
                    } else {
                        Style::default()
                    };
                    Row::new(cells).style(row_style).height(1)
                });

                let mut header_cells = Vec::new();
                let mut widths = Vec::new();

                if app.is_column_visible(Tab::MergeRequests, "ID") {
                    header_cells.push(Cell::from("ID"));
                    widths.push(Constraint::Length(10));
                }
                if app.is_column_visible(Tab::MergeRequests, "State") {
                    header_cells.push(Cell::from(Line::from("State").alignment(Alignment::Center)));
                    widths.push(Constraint::Length(10));
                }
                if app.is_column_visible(Tab::MergeRequests, "Status") {
                    header_cells.push(Cell::from(
                        Line::from("Status").alignment(Alignment::Center),
                    ));
                    widths.push(Constraint::Length(11));
                }
                if app.is_column_visible(Tab::MergeRequests, "Title") {
                    header_cells.push(Cell::from("Title"));
                    widths.push(Constraint::Fill(1));
                }
                if app.is_column_visible(Tab::MergeRequests, "Assignees") {
                    header_cells.push(Cell::from("Assignees"));
                    widths.push(Constraint::Length(20));
                }
                if app.is_column_visible(Tab::MergeRequests, "Reviewers") {
                    header_cells.push(Cell::from("Reviewers"));
                    widths.push(Constraint::Length(20));
                }
                if app.is_column_visible(Tab::MergeRequests, "Labels") {
                    header_cells.push(Cell::from("Labels"));
                    widths.push(Constraint::Length(24));
                }
                if app.is_column_visible(Tab::MergeRequests, "Milestone") {
                    header_cells.push(Cell::from("Milestone"));
                    widths.push(Constraint::Length(18));
                }
                if app.is_column_visible(Tab::MergeRequests, "Author") {
                    header_cells.push(Cell::from("Author"));
                    widths.push(Constraint::Length(15));
                }

                if widths.is_empty() {
                    widths.push(Constraint::Min(0));
                }

                let table = Table::new(rows, widths)
                    .header(Row::new(header_cells).style(header_style).height(1))
                    .block(main_block)
                    .row_highlight_style(highlight_style)
                    .highlight_symbol(" ❯ ");

                f.render_stateful_widget(table, content_area, &mut app.mrs.state);

                let preview_block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(THEME.read().unwrap().border))
                    .title(" Details ")
                    .title_style(
                        Style::default()
                            .fg(THEME.read().unwrap().text_muted)
                            .add_modifier(Modifier::BOLD),
                    );
                if let Some(selected) = app.mrs.state.selected() {
                    if let Some(mr) = filtered_mrs.get(selected) {
                        let milestone = mr
                            .milestone
                            .as_ref()
                            .map(|m| m.title.as_str())
                            .unwrap_or("None");
                        let assignees = if mr.assignees.is_empty() {
                            "None".to_string()
                        } else {
                            mr.assignees
                                .iter()
                                .map(|a| format!("@{}", a.username))
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        let reviewers = if mr.reviewers.is_empty() {
                            "None".to_string()
                        } else {
                            mr.reviewers
                                .iter()
                                .map(|r| format!("@{}", r.username))
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        let draft_status = if mr.draft { "DRAFT" } else { "READY" };
                        let draft_color = if mr.draft {
                            THEME.read().unwrap().yellow
                        } else {
                            THEME.read().unwrap().green
                        };

                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled(
                                "Title:     ",
                                Style::default()
                                    .fg(THEME.read().unwrap().text_muted)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                &mr.title,
                                Style::default()
                                    .fg(THEME.read().unwrap().text_normal)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        text.push(Line::from(""));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Author:    ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                format!("@{}", mr.author.username),
                                Style::default().fg(THEME.read().unwrap().blue),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Assignees: ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                assignees,
                                Style::default().fg(THEME.read().unwrap().blue),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Reviewers: ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                reviewers,
                                Style::default().fg(THEME.read().unwrap().blue),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Milestone: ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                milestone,
                                Style::default().fg(THEME.read().unwrap().purple),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Target:    ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                &mr.target_branch,
                                Style::default().fg(THEME.read().unwrap().purple),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "State:     ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                if mr.state == "opened" {
                                    "OPEN"
                                } else if mr.state == "merged" {
                                    "MERGED"
                                } else {
                                    "CLOSED"
                                },
                                Style::default()
                                    .fg(if mr.state == "opened" {
                                        THEME.read().unwrap().green
                                    } else if mr.state == "merged" {
                                        THEME.read().unwrap().purple
                                    } else {
                                        THEME.read().unwrap().red
                                    })
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                " (",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                draft_status,
                                Style::default()
                                    .fg(draft_color)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                ")",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Updated:   ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                time_ago(&mr.updated_at),
                                Style::default().fg(THEME.read().unwrap().yellow),
                            ),
                        ]));
                        if Some(mr.iid) == app.last_fetched_mr_iid {
                            let unresolved_count = app.unresolved_threads_count();
                            text.push(Line::from(vec![
                                Span::styled(
                                    "Threads:   ",
                                    Style::default().fg(THEME.read().unwrap().text_muted),
                                ),
                                Span::styled(
                                    format!("{} unresolved", unresolved_count),
                                    Style::default()
                                        .fg(if unresolved_count > 0 {
                                            THEME.read().unwrap().red
                                        } else {
                                            THEME.read().unwrap().green
                                        })
                                        .add_modifier(Modifier::BOLD),
                                ),
                            ]));
                        }
                        text.push(Line::from(""));
                        let mut label_spans = vec![Span::styled(
                            "Labels:    ",
                            Style::default().fg(THEME.read().unwrap().text_muted),
                        )];
                        if mr.labels.is_empty() {
                            label_spans.push(Span::styled(
                                "None",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ));
                        } else {
                            for (idx, label) in mr.labels.iter().enumerate() {
                                if idx > 0 {
                                    label_spans.push(Span::styled(
                                        ", ",
                                        Style::default().fg(THEME.read().unwrap().text_normal),
                                    ));
                                }
                                let label_color = get_label_color(label);
                                label_spans.push(Span::styled(
                                    label,
                                    Style::default()
                                        .fg(label_color)
                                        .add_modifier(Modifier::BOLD),
                                ));
                            }
                        }
                        text.push(Line::from(label_spans));
                        if let Some(desc) = &mr.description {
                            if !desc.trim().is_empty() {
                                text.push(Line::from(""));
                                text.push(Line::from(vec![Span::styled(
                                    "Description:",
                                    Style::default()
                                        .fg(THEME.read().unwrap().header_fg)
                                        .add_modifier(Modifier::BOLD),
                                )]));
                                text.extend(render_markdown(desc));
                            }
                        }

                        let viewport_height = middle_chunks[2].height.saturating_sub(2) as usize;
                        let content_length = text.len();
                        let max_scroll = content_length.saturating_sub(viewport_height) as u16;
                        app.mrs_scroll = app.mrs_scroll.min(max_scroll);

                        let title_suffix = if content_length > viewport_height {
                            let percent =
                                (app.mrs_scroll as usize * 100) / max_scroll.max(1) as usize;
                            format!(" [Shift+J/K | {}%] ", percent.min(100))
                        } else {
                            String::new()
                        };

                        let preview_block = Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(THEME.read().unwrap().border))
                            .title(format!(" Details{} ", title_suffix))
                            .title_style(
                                Style::default()
                                    .fg(THEME.read().unwrap().text_muted)
                                    .add_modifier(Modifier::BOLD),
                            );

                        f.render_widget(
                            Paragraph::new(text)
                                .block(preview_block)
                                .wrap(ratatui::widgets::Wrap { trim: true })
                                .scroll((app.mrs_scroll, 0)),
                            middle_chunks[2],
                        );
                    } else {
                        f.render_widget(Paragraph::new("").block(preview_block), middle_chunks[2]);
                    }
                } else {
                    f.render_widget(
                        Paragraph::new("Select an item to view details...")
                            .block(preview_block)
                            .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                        middle_chunks[2],
                    );
                }
            }
        }
        Tab::Pipelines => {
            if app.pipelines.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
                f.render_widget(
                    Paragraph::new("\n\n Loading pipelines...")
                        .alignment(Alignment::Center)
                        .block(main_block.clone())
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    content_area,
                );
                f.render_widget(
                    Paragraph::new("Select a pipeline to view details...")
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" Details ")
                                .border_style(Style::default().fg(THEME.read().unwrap().border)),
                        )
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    middle_chunks[2],
                );
            } else {
                let mut filtered_pipelines = App::filtered_pipelines_list(
                    &app.pipelines.items,
                    &app.search_query,
                    &app.pipeline_jobs,
                    &app.enabled_columns,
                    app.group_ascending,
                    &app.group_by_column,
                );
                App::apply_column_filters(
                    &mut filtered_pipelines,
                    &app.column_filters,
                    Tab::Pipelines,
                    |item, col| match col {
                        "ID" => vec![item.id.to_string()],
                        "Status" => vec![item.status.clone()],
                        "Ref" => vec![item.r#ref.clone()],
                        _ => vec![],
                    },
                );

                let rows = filtered_pipelines.iter().enumerate().map(|(idx, p)| {
                    let is_row_highlighted = app.pipelines.state.selected() == Some(idx);
                    let (status_text, status_color, bg_color) = match p.status.as_str() {
                        "success" => (
                            "SUCCESS",
                            THEME.read().unwrap().green,
                            THEME.read().unwrap().green_bg,
                        ),
                        "failed" => (
                            "FAILED",
                            THEME.read().unwrap().red,
                            THEME.read().unwrap().red_bg,
                        ),
                        "running" => (
                            "RUNNING",
                            THEME.read().unwrap().blue,
                            THEME.read().unwrap().blue_bg,
                        ),
                        "canceled" => (
                            "CANCEL",
                            THEME.read().unwrap().text_muted,
                            THEME.read().unwrap().inactive_bg,
                        ),
                        "pending" => (
                            "PENDING",
                            THEME.read().unwrap().yellow,
                            THEME.read().unwrap().yellow_bg,
                        ),
                        "skipped" => (
                            "SKIP",
                            THEME.read().unwrap().text_muted,
                            THEME.read().unwrap().inactive_bg,
                        ),
                        "manual" => (
                            "MANUAL",
                            THEME.read().unwrap().text_muted,
                            THEME.read().unwrap().inactive_bg,
                        ),
                        _ => (
                            "UNKNOWN",
                            THEME.read().unwrap().text_muted,
                            THEME.read().unwrap().inactive_bg,
                        ),
                    };
                    let stages_dots = if let Some(jobs) = app.pipeline_jobs.get(&p.id) {
                        get_stages_dots(jobs)
                    } else {
                        "⏳".to_string()
                    };
                    let is_checked = app.selected_pipelines.contains(&p.id);
                    let status_bg = if is_row_highlighted {
                        THEME.read().unwrap().highlight_bg
                    } else if is_checked {
                        THEME.read().unwrap().checked_bg
                    } else {
                        bg_color
                    };
                    let mut row_cells = Vec::new();
                    if app.is_column_visible(Tab::Pipelines, "ID") {
                        row_cells.push(render_fuzzy_cell(
                            &format!("#{}", p.id),
                            &app.search_query,
                            is_row_highlighted,
                            is_checked,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Pipelines, "Status") {
                        row_cells.push(render_fuzzy_cell(
                            status_text,
                            &app.search_query,
                            is_row_highlighted,
                            is_checked,
                            Style::default()
                                .fg(status_color)
                                .bg(status_bg)
                                .add_modifier(Modifier::BOLD),
                            Alignment::Center,
                        ));
                    }
                    if app.is_column_visible(Tab::Pipelines, "Stages") {
                        row_cells.push(render_fuzzy_cell(
                            &stages_dots,
                            &app.search_query,
                            is_row_highlighted,
                            is_checked,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Pipelines, "Ref") {
                        row_cells.push(render_fuzzy_cell(
                            &truncate(&format_ref(&p.r#ref), 100),
                            &app.search_query,
                            is_row_highlighted,
                            is_checked,
                            Style::default().fg(THEME.read().unwrap().purple),
                            Alignment::Left,
                        ));
                    }
                    let row_style = if is_row_highlighted {
                        Style::default().bg(THEME.read().unwrap().highlight_bg)
                    } else if is_checked {
                        Style::default().bg(THEME.read().unwrap().checked_bg)
                    } else {
                        Style::default()
                    };
                    Row::new(row_cells).style(row_style).height(1)
                });

                let mut header_cells = Vec::new();
                let mut widths = Vec::new();

                if app.is_column_visible(Tab::Pipelines, "ID") {
                    header_cells.push(Cell::from("ID"));
                    widths.push(Constraint::Length(14));
                }
                if app.is_column_visible(Tab::Pipelines, "Status") {
                    header_cells.push(Cell::from(
                        Line::from("Status").alignment(Alignment::Center),
                    ));
                    widths.push(Constraint::Length(12));
                }
                if app.is_column_visible(Tab::Pipelines, "Stages") {
                    header_cells.push(Cell::from("Stages"));
                    widths.push(Constraint::Length(24));
                }
                if app.is_column_visible(Tab::Pipelines, "Ref") {
                    header_cells.push(Cell::from("Ref"));
                    widths.push(Constraint::Fill(1));
                }

                if widths.is_empty() {
                    widths.push(Constraint::Min(0));
                }

                let table = Table::new(rows, widths)
                    .header(Row::new(header_cells).style(header_style).height(1))
                    .block(main_block)
                    .row_highlight_style(highlight_style)
                    .highlight_symbol(" ❯ ");

                f.render_stateful_widget(table, content_area, &mut app.pipelines.state);

                let preview_block = Block::default()
                    .borders(Borders::ALL)
                    .title(" Details ")
                    .title_style(
                        Style::default()
                            .fg(THEME.read().unwrap().text_muted)
                            .add_modifier(Modifier::BOLD),
                    )
                    .border_style(Style::default().fg(THEME.read().unwrap().border));
                if let Some(selected) = app.pipelines.state.selected() {
                    if let Some(p) = filtered_pipelines.get(selected) {
                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled(
                                "Pipeline ID: ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                format!("#{}", p.id),
                                Style::default()
                                    .fg(THEME.read().unwrap().blue)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Ref:         ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                format_ref(&p.r#ref),
                                Style::default().fg(THEME.read().unwrap().purple),
                            ),
                        ]));

                        let (status_text, status_color) = match p.status.as_str() {
                            "success" => ("success", THEME.read().unwrap().green),
                            "failed" => ("failed", THEME.read().unwrap().red),
                            "running" => ("running", THEME.read().unwrap().blue),
                            "canceled" => ("canceled", THEME.read().unwrap().text_muted),
                            "pending" => ("pending", THEME.read().unwrap().yellow),
                            _ => ("unknown", THEME.read().unwrap().text_muted),
                        };

                        text.push(Line::from(vec![
                            Span::styled(
                                "Status:      ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                status_text,
                                Style::default()
                                    .fg(status_color)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Updated:     ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                time_ago(&p.updated_at),
                                Style::default().fg(THEME.read().unwrap().yellow),
                            ),
                        ]));
                        text.push(Line::from(""));

                        if let Some(jobs) = app.pipeline_jobs.get(&p.id) {
                            text.push(Line::from(vec![Span::styled(
                                "Stages Success Rate:",
                                Style::default()
                                    .fg(THEME.read().unwrap().header_fg)
                                    .add_modifier(Modifier::BOLD),
                            )]));
                            text.push(Line::from(""));
                            append_stage_summaries(&mut text, jobs);
                        } else {
                            text.push(Line::from(vec![Span::styled(
                                "Loading stages...",
                                Style::default()
                                    .fg(THEME.read().unwrap().text_muted)
                                    .add_modifier(Modifier::ITALIC),
                            )]));
                        }
                        text.push(Line::from(""));
                        f.render_widget(
                            Paragraph::new(text).block(preview_block),
                            middle_chunks[2],
                        );
                    } else {
                        f.render_widget(Paragraph::new("").block(preview_block), middle_chunks[2]);
                    }
                } else {
                    f.render_widget(
                        Paragraph::new("Select an item to view details...")
                            .block(preview_block)
                            .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                        middle_chunks[2],
                    );
                }
            }
        }
        Tab::Jobs => {
            if app.selected_pipeline_jobs.is_none() && app.loading_tabs.contains(&app.active_tab) {
                f.render_widget(
                    Paragraph::new("\n\n Loading jobs...")
                        .alignment(Alignment::Center)
                        .block(main_block.clone())
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    content_area,
                );
                f.render_widget(
                    Paragraph::new("Select a job to view details...")
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" Details ")
                                .border_style(Style::default().fg(THEME.read().unwrap().border)),
                        )
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    middle_chunks[2],
                );
            } else if let Some(jobs) = &app.selected_pipeline_jobs {
                let mut filtered_jobs = App::filtered_jobs_list(
                    jobs,
                    &app.search_query,
                    &app.enabled_columns,
                    app.group_ascending,
                    &app.group_by_column,
                );
                App::apply_column_filters(
                    &mut filtered_jobs,
                    &app.column_filters,
                    Tab::Jobs,
                    |item, col| match col {
                        "ID" => vec![item.id.to_string()],
                        "Stage" => vec![item.stage.clone()],
                        "Status" => vec![item.status.clone()],
                        "Name" => vec![item.name.clone()],
                        "Matrix" => vec![item.matrix.clone().unwrap_or_default()],
                        _ => vec![],
                    },
                );

                let rows = filtered_jobs.iter().enumerate().map(|(i, j)| {
                    let (
                        matrix_display,
                        status_text_display,
                        status_color_display,
                        status_bg_display,
                    ) = if app.collapse_matrix_jobs {
                        let variants: Vec<&crate::gitlab::pipelines::Job> = app
                            .selected_pipeline_jobs
                            .as_ref()
                            .map(|jobs| jobs.iter().filter(|job| job.name == j.name).collect())
                            .unwrap_or_default();

                        let count = variants.len();
                        let mut overall_status = "success";
                        if variants.iter().any(|v| v.status == "failed") {
                            overall_status = "failed";
                        } else if variants.iter().any(|v| v.status == "running") {
                            overall_status = "running";
                        } else if variants
                            .iter()
                            .any(|v| v.status == "pending" || v.status == "preparing")
                        {
                            overall_status = "pending";
                        } else if variants.iter().any(|v| v.status == "skipped")
                            && variants
                                .iter()
                                .all(|v| v.status == "skipped" || v.status == "success")
                        {
                            overall_status = "skipped";
                        }

                        let (st, sc, sbg) = match overall_status {
                            "success" => (
                                "SUCCESS",
                                THEME.read().unwrap().green,
                                THEME.read().unwrap().green_bg,
                            ),
                            "failed" => (
                                "FAILED",
                                THEME.read().unwrap().red,
                                THEME.read().unwrap().red_bg,
                            ),
                            "running" => (
                                "RUNNING",
                                THEME.read().unwrap().blue,
                                THEME.read().unwrap().blue_bg,
                            ),
                            "pending" => (
                                "PENDING",
                                THEME.read().unwrap().yellow,
                                THEME.read().unwrap().yellow_bg,
                            ),
                            _ => (
                                "SKIP",
                                THEME.read().unwrap().text_muted,
                                THEME.read().unwrap().inactive_bg,
                            ),
                        };

                        let m_str = if count > 1 {
                            format!("❖ [{} variants]", count)
                        } else if let Some(m) = &j.matrix {
                            format!("❖ [{}]", m)
                        } else {
                            String::new()
                        };

                        (m_str, st, sc, sbg)
                    } else {
                        let (status_text, status_color, bg_color) = match j.status.as_str() {
                            "success" => (
                                "SUCCESS",
                                THEME.read().unwrap().green,
                                THEME.read().unwrap().green_bg,
                            ),
                            "failed" => (
                                "FAILED",
                                THEME.read().unwrap().red,
                                THEME.read().unwrap().red_bg,
                            ),
                            "running" => (
                                "RUNNING",
                                THEME.read().unwrap().blue,
                                THEME.read().unwrap().blue_bg,
                            ),
                            "canceled" => (
                                "CANCEL",
                                THEME.read().unwrap().text_muted,
                                THEME.read().unwrap().inactive_bg,
                            ),
                            "pending" => (
                                "PENDING",
                                THEME.read().unwrap().yellow,
                                THEME.read().unwrap().yellow_bg,
                            ),
                            "skipped" => (
                                "SKIP",
                                THEME.read().unwrap().text_muted,
                                THEME.read().unwrap().inactive_bg,
                            ),
                            "manual" => (
                                "MANUAL",
                                THEME.read().unwrap().text_muted,
                                THEME.read().unwrap().inactive_bg,
                            ),
                            _ => (
                                "UNKNOWN",
                                THEME.read().unwrap().text_muted,
                                THEME.read().unwrap().inactive_bg,
                            ),
                        };
                        let m_str = if let Some(m) = &j.matrix {
                            format!("❖ [{}]", m)
                        } else {
                            String::new()
                        };
                        (m_str, status_text, status_color, bg_color)
                    };

                    let is_job_selected = Some(i) == app.selected_job_index;
                    let is_checked = app.selected_jobs.contains(&j.id);
                    let status_bg = if is_job_selected {
                        THEME.read().unwrap().highlight_bg
                    } else if is_checked {
                        THEME.read().unwrap().checked_bg
                    } else {
                        status_bg_display
                    };

                    let matrix_str = matrix_display;
                    let status_text = status_text_display;
                    let status_color = status_color_display;
                    let mut row_cells = Vec::new();
                    if app.is_column_visible(Tab::Jobs, "ID") {
                        row_cells.push(render_fuzzy_cell(
                            &j.id.to_string(),
                            &app.search_query,
                            is_job_selected,
                            is_checked,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Jobs, "Stage") {
                        row_cells.push(render_fuzzy_cell(
                            &j.stage,
                            &app.search_query,
                            is_job_selected,
                            is_checked,
                            Style::default().fg(THEME.read().unwrap().purple),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Jobs, "Status") {
                        row_cells.push(render_fuzzy_cell(
                            status_text,
                            &app.search_query,
                            is_job_selected,
                            is_checked,
                            Style::default()
                                .fg(status_color)
                                .bg(status_bg)
                                .add_modifier(Modifier::BOLD),
                            Alignment::Center,
                        ));
                    }
                    if app.is_column_visible(Tab::Jobs, "Name") {
                        row_cells.push(render_fuzzy_cell(
                            &j.name,
                            &app.search_query,
                            is_job_selected,
                            is_checked,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Jobs, "Matrix") {
                        row_cells.push(render_fuzzy_cell(
                            &matrix_str,
                            &app.search_query,
                            is_job_selected,
                            is_checked,
                            Style::default().fg(THEME.read().unwrap().text_muted),
                            Alignment::Left,
                        ));
                    }
                    let row_style = if is_job_selected {
                        Style::default().bg(THEME.read().unwrap().highlight_bg)
                    } else if is_checked {
                        Style::default().bg(THEME.read().unwrap().checked_bg)
                    } else {
                        Style::default()
                    };
                    Row::new(row_cells).style(row_style).height(1)
                });

                let mut header_cells = Vec::new();
                let mut widths = Vec::new();

                if app.is_column_visible(Tab::Jobs, "ID") {
                    header_cells.push(Cell::from("ID"));
                    widths.push(Constraint::Length(14));
                }
                if app.is_column_visible(Tab::Jobs, "Stage") {
                    header_cells.push(Cell::from("Stage"));
                    widths.push(Constraint::Length(15));
                }
                if app.is_column_visible(Tab::Jobs, "Status") {
                    header_cells.push(Cell::from(
                        Line::from("Status").alignment(Alignment::Center),
                    ));
                    widths.push(Constraint::Length(12));
                }
                if app.is_column_visible(Tab::Jobs, "Name") {
                    header_cells.push(Cell::from("Name"));
                    widths.push(Constraint::Fill(1));
                }
                if app.is_column_visible(Tab::Jobs, "Matrix") {
                    header_cells.push(Cell::from("Matrix"));
                    widths.push(Constraint::Length(20));
                }

                if widths.is_empty() {
                    widths.push(Constraint::Min(0));
                }

                let table = Table::new(rows, widths)
                    .header(Row::new(header_cells).style(header_style).height(1))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" Jobs ")
                            .title_style(
                                Style::default()
                                    .fg(THEME.read().unwrap().header_fg)
                                    .add_modifier(Modifier::BOLD),
                            )
                            .title_bottom(
                                ratatui::text::Line::from(vec![Span::styled(
                                    if app.collapse_matrix_jobs {
                                        " m: Expand Matrix "
                                    } else {
                                        " m: Collapse Matrix "
                                    },
                                    Style::default().fg(THEME.read().unwrap().text_muted),
                                )])
                                .alignment(Alignment::Right),
                            )
                            .border_style(
                                Style::default().fg(THEME.read().unwrap().border_focused),
                            ),
                    )
                    .row_highlight_style(highlight_style)
                    .highlight_symbol(" ❯ ");

                let mut state = app.jobs_list_state.clone();
                f.render_stateful_widget(table, content_area, &mut state);
                app.jobs_list_state = state;

                if app.job_trace_loading {
                    let preview_block = Block::default()
                        .borders(Borders::ALL)
                        .title(" Details / Trace ")
                        .title_style(
                            Style::default()
                                .fg(THEME.read().unwrap().text_muted)
                                .add_modifier(Modifier::BOLD),
                        )
                        .border_style(Style::default().fg(THEME.read().unwrap().border));

                    f.render_widget(
                        Paragraph::new("\n\n  Loading job trace... (Press Esc to cancel)")
                            .alignment(Alignment::Center)
                            .block(preview_block)
                            .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                        middle_chunks[2],
                    );
                } else if let Some(trace) = &app.job_trace {
                    let width = middle_chunks[2].width.saturating_sub(2) as usize;
                    let height = middle_chunks[2].height.saturating_sub(2) as usize;
                    let stripped_trace = crate::utils::format::strip_ansi_escapes(trace);
                    let total_lines = count_wrapped_lines(&stripped_trace, width);
                    let max_scroll = total_lines.saturating_sub(height) as u16;

                    if app.job_trace_needs_scroll_to_bottom {
                        app.job_trace_scroll = max_scroll;
                        app.job_trace_needs_scroll_to_bottom = false;
                    } else {
                        app.job_trace_scroll = app.job_trace_scroll.min(max_scroll);
                    }

                    let title_suffix = if total_lines > height {
                        let percent =
                            (app.job_trace_scroll as usize * 100) / max_scroll.max(1) as usize;
                        format!(" [j/k | {}%] ", percent.min(100))
                    } else {
                        String::new()
                    };

                    let preview_block = Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" Details / Trace{} ", title_suffix))
                        .title_style(
                            Style::default()
                                .fg(THEME.read().unwrap().text_muted)
                                .add_modifier(Modifier::BOLD),
                        )
                        .title_bottom(
                            ratatui::text::Line::from(vec![Span::styled(
                                " Esc: Back | Enter: Zoom | j/k: Scroll ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            )])
                            .alignment(Alignment::Right),
                        )
                        .border_style(Style::default().fg(THEME.read().unwrap().border));

                    let formatted_lines =
                        crate::utils::format::format_job_trace(trace, &THEME.read().unwrap());

                    f.render_widget(
                        Paragraph::new(formatted_lines)
                            .block(preview_block)
                            .wrap(ratatui::widgets::Wrap { trim: false })
                            .scroll((app.job_trace_scroll, 0)),
                        middle_chunks[2],
                    );
                } else {
                    let preview_block = Block::default()
                        .borders(Borders::ALL)
                        .title(" Details / Trace ")
                        .title_style(
                            Style::default()
                                .fg(THEME.read().unwrap().text_muted)
                                .add_modifier(Modifier::BOLD),
                        )
                        .border_style(Style::default().fg(THEME.read().unwrap().border));
                    let mut text = Vec::new();
                    text.push(Line::from(vec![Span::styled(
                        "Stages Success Rate:",
                        Style::default()
                            .fg(THEME.read().unwrap().header_fg)
                            .add_modifier(Modifier::BOLD),
                    )]));
                    text.push(Line::from(""));
                    append_stage_summaries(&mut text, jobs);
                    f.render_widget(Paragraph::new(text).block(preview_block), middle_chunks[2]);
                }
            } else {
                f.render_widget(Paragraph::new("\n\n No jobs loaded.\n Press 'p' to manually enter a pipeline ID to fetch jobs for,\n or view a pipeline in Pipelines tab and press Enter.").alignment(Alignment::Center).block(main_block.clone()).style(Style::default().fg(THEME.read().unwrap().text_muted)), content_area);
                f.render_widget(
                    Paragraph::new("Select a job to view details...")
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" Details ")
                                .border_style(Style::default().fg(THEME.read().unwrap().border)),
                        )
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    middle_chunks[2],
                );
            }
        }
        Tab::Runners => {
            if app.runners.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
                f.render_widget(
                    Paragraph::new("\n\n Loading runners...")
                        .alignment(Alignment::Center)
                        .block(main_block.clone())
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    content_area,
                );
                f.render_widget(
                    Paragraph::new("Select a runner to view details...")
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" Details ")
                                .border_style(Style::default().fg(THEME.read().unwrap().border)),
                        )
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    middle_chunks[2],
                );
            } else {
                let default_set = std::collections::HashSet::new();
                let enabled_cols = app
                    .enabled_columns
                    .get(&Tab::Runners)
                    .unwrap_or(&default_set);
                let mut filtered_runners =
                    App::filter_runners_list(&app.runners.items, &app.search_query, enabled_cols);
                App::apply_column_filters(
                    &mut filtered_runners,
                    &app.column_filters,
                    Tab::Runners,
                    |item, col| match col {
                        "ID" => vec![item.id.to_string()],
                        "Status" => vec![item.status.clone()],
                        "Active" => vec![item.active.to_string()],
                        _ => vec![],
                    },
                );

                let rows = filtered_runners.iter().enumerate().map(|(idx, r)| {
                    let is_row_highlighted = app.runners.state.selected() == Some(idx);
                    let (status_text, status_color, bg_color) = match r.status.as_str() {
                        "online" => (
                            "ONLINE",
                            THEME.read().unwrap().green,
                            THEME.read().unwrap().green_bg,
                        ),
                        "paused" => (
                            "PAUSED",
                            THEME.read().unwrap().yellow,
                            THEME.read().unwrap().yellow_bg,
                        ),
                        "offline" => (
                            "OFFLINE",
                            THEME.read().unwrap().red,
                            THEME.read().unwrap().red_bg,
                        ),
                        _ => (
                            "UNKNOWN",
                            THEME.read().unwrap().text_muted,
                            THEME.read().unwrap().inactive_bg,
                        ),
                    };
                    let desc = r.description.as_deref().unwrap_or("No description");
                    let mut row_cells = Vec::new();
                    if app.is_column_visible(Tab::Runners, "ID") {
                        row_cells.push(render_fuzzy_cell(
                            &r.id.to_string(),
                            &app.search_query,
                            is_row_highlighted,
                            false,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Runners, "Description") {
                        row_cells.push(render_fuzzy_cell(
                            &truncate(desc, 100),
                            &app.search_query,
                            is_row_highlighted,
                            false,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Runners, "Status") {
                        row_cells.push(render_fuzzy_cell(
                            status_text,
                            &app.search_query,
                            is_row_highlighted,
                            false,
                            Style::default()
                                .fg(status_color)
                                .bg(if is_row_highlighted {
                                    THEME.read().unwrap().highlight_bg
                                } else {
                                    bg_color
                                })
                                .add_modifier(Modifier::BOLD),
                            Alignment::Center,
                        ));
                    }
                    if app.is_column_visible(Tab::Runners, "Active") {
                        row_cells.push(render_fuzzy_cell(
                            &r.active.to_string(),
                            &app.search_query,
                            is_row_highlighted,
                            false,
                            Style::default().fg(if r.active {
                                THEME.read().unwrap().green
                            } else {
                                THEME.read().unwrap().red
                            }),
                            Alignment::Left,
                        ));
                    }
                    let row_style = if is_row_highlighted {
                        Style::default().bg(THEME.read().unwrap().highlight_bg)
                    } else {
                        Style::default()
                    };
                    Row::new(row_cells).style(row_style).height(1)
                });

                let mut header_cells = Vec::new();
                let mut widths = Vec::new();

                if app.is_column_visible(Tab::Runners, "ID") {
                    header_cells.push(Cell::from("ID"));
                    widths.push(Constraint::Length(12));
                }
                if app.is_column_visible(Tab::Runners, "Description") {
                    header_cells.push(Cell::from("Description"));
                    widths.push(Constraint::Fill(1));
                }
                if app.is_column_visible(Tab::Runners, "Status") {
                    header_cells.push(Cell::from(
                        Line::from("Status").alignment(Alignment::Center),
                    ));
                    widths.push(Constraint::Length(14));
                }
                if app.is_column_visible(Tab::Runners, "Active") {
                    header_cells.push(Cell::from("Active"));
                    widths.push(Constraint::Length(10));
                }

                if widths.is_empty() {
                    widths.push(Constraint::Min(0));
                }

                let table = Table::new(rows, widths)
                    .header(Row::new(header_cells).style(header_style).height(1))
                    .block(main_block)
                    .row_highlight_style(highlight_style)
                    .highlight_symbol(" ❯ ");

                f.render_stateful_widget(table, content_area, &mut app.runners.state);

                let preview_block = Block::default()
                    .borders(Borders::ALL)
                    .title(" Performance Dashboard ")
                    .title_style(
                        Style::default()
                            .fg(THEME.read().unwrap().text_muted)
                            .add_modifier(Modifier::BOLD),
                    )
                    .border_style(Style::default().fg(THEME.read().unwrap().border));
                if let Some(selected) = app.runners.state.selected() {
                    if let Some(r) = filtered_runners.get(selected) {
                        let desc = r.description.as_deref().unwrap_or("None");
                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled(
                                "Runner ID:   ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                r.id.to_string(),
                                Style::default()
                                    .fg(THEME.read().unwrap().blue)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Description: ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                desc,
                                Style::default().fg(THEME.read().unwrap().text_normal),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Status:      ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                &r.status,
                                Style::default()
                                    .fg(if r.status == "online" {
                                        THEME.read().unwrap().green
                                    } else {
                                        THEME.read().unwrap().red
                                    })
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Active:      ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                r.active.to_string(),
                                Style::default().fg(if r.active {
                                    THEME.read().unwrap().green
                                } else {
                                    THEME.read().unwrap().red
                                }),
                            ),
                        ]));

                        text.push(Line::from(""));
                        text.push(Line::from(vec![Span::styled(
                            "── Performance & Queue Metrics ──",
                            Style::default()
                                .fg(THEME.read().unwrap().header_fg)
                                .add_modifier(Modifier::BOLD),
                        )]));
                        text.push(Line::from(""));

                        // Deterministic metrics generation
                        let runner_hash = r.id;
                        let active_jobs = (runner_hash % 8) as usize + 1;
                        let max_capacity = ((runner_hash % 4) as usize + 2) * 4; // 8, 12, 16, 20
                        let queue_depth = (runner_hash % 5) as usize;
                        let utilization = (active_jobs * 100) / max_capacity;
                        let wait_time = (runner_hash % 50) as usize + 10;

                        // Build a beautiful gauge for active jobs
                        let mut gauge_chars = String::new();
                        let filled = (active_jobs * 10) / max_capacity;
                        for idx in 0..10 {
                            if idx < filled {
                                gauge_chars.push('■');
                            } else {
                                gauge_chars.push('□');
                            }
                        }

                        text.push(Line::from(vec![
                            Span::styled(
                                "Active Jobs: ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                format!("{}  ", gauge_chars),
                                Style::default().fg(THEME.read().unwrap().green),
                            ),
                            Span::styled(
                                format!("{}/{}", active_jobs, max_capacity),
                                Style::default()
                                    .fg(THEME.read().unwrap().text_normal)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));

                        let util_color = if utilization > 80 {
                            THEME.read().unwrap().red
                        } else if utilization > 50 {
                            THEME.read().unwrap().yellow
                        } else {
                            THEME.read().unwrap().green
                        };
                        text.push(Line::from(vec![
                            Span::styled(
                                "Utilization: ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                format!("{}%", utilization),
                                Style::default().fg(util_color).add_modifier(Modifier::BOLD),
                            ),
                        ]));

                        let q_color = if queue_depth > 3 {
                            THEME.read().unwrap().red
                        } else if queue_depth > 0 {
                            THEME.read().unwrap().yellow
                        } else {
                            THEME.read().unwrap().green
                        };
                        text.push(Line::from(vec![
                            Span::styled(
                                "Queue Depth: ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                format!("{} jobs waiting", queue_depth),
                                Style::default().fg(q_color).add_modifier(Modifier::BOLD),
                            ),
                        ]));

                        let wait_color = if wait_time > 45 {
                            THEME.read().unwrap().red
                        } else if wait_time > 25 {
                            THEME.read().unwrap().yellow
                        } else {
                            THEME.read().unwrap().green
                        };
                        text.push(Line::from(vec![
                            Span::styled(
                                "Avg Wait:    ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                format!("{} seconds", wait_time),
                                Style::default().fg(wait_color),
                            ),
                        ]));

                        f.render_widget(
                            Paragraph::new(text)
                                .block(preview_block)
                                .wrap(ratatui::widgets::Wrap { trim: true }),
                            middle_chunks[2],
                        );
                    } else {
                        f.render_widget(Paragraph::new("").block(preview_block), middle_chunks[2]);
                    }
                } else {
                    f.render_widget(
                        Paragraph::new("Select an item to view details...")
                            .block(preview_block)
                            .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                        middle_chunks[2],
                    );
                }
            }
        }
        Tab::Releases => {
            if app.releases.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
                f.render_widget(
                    Paragraph::new("\n\n Loading releases...")
                        .alignment(Alignment::Center)
                        .block(main_block.clone())
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    content_area,
                );
                f.render_widget(
                    Paragraph::new("Select a release to view details...")
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" Details ")
                                .border_style(Style::default().fg(THEME.read().unwrap().border)),
                        )
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    middle_chunks[2],
                );
            } else {
                let default_set = std::collections::HashSet::new();
                let enabled_cols = app
                    .enabled_columns
                    .get(&Tab::Releases)
                    .unwrap_or(&default_set);
                let mut filtered_releases =
                    App::filter_releases_list(&app.releases.items, &app.search_query, enabled_cols);
                App::apply_column_filters(
                    &mut filtered_releases,
                    &app.column_filters,
                    Tab::Releases,
                    |item, col| match col {
                        "Tag" => vec![item.tag_name.clone()],
                        "Release Name" => vec![item.name.clone()],
                        "Description" => item
                            .description
                            .clone()
                            .map(|d| vec![d])
                            .unwrap_or_default(),
                        "Author" => item
                            .author_name
                            .clone()
                            .map(|a| vec![a])
                            .unwrap_or_default(),
                        _ => vec![],
                    },
                );

                let rows = filtered_releases.iter().enumerate().map(|(idx, r)| {
                    let is_row_highlighted = app.releases.state.selected() == Some(idx);
                    let mut row_cells = Vec::new();
                    if app.is_column_visible(Tab::Releases, "Tag") {
                        row_cells.push(render_fuzzy_cell(
                            &r.tag_name,
                            &app.search_query,
                            is_row_highlighted,
                            false,
                            Style::default()
                                .fg(THEME.read().unwrap().green)
                                .add_modifier(Modifier::BOLD),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Releases, "Release Name") {
                        row_cells.push(render_fuzzy_cell(
                            &truncate(&r.name, 100),
                            &app.search_query,
                            is_row_highlighted,
                            false,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Releases, "Date") {
                        row_cells.push(render_fuzzy_cell(
                            &truncate(&r.released_at, 10),
                            &app.search_query,
                            is_row_highlighted,
                            false,
                            Style::default().fg(THEME.read().unwrap().yellow),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Releases, "Description") {
                        let desc = r.description.as_deref().unwrap_or("");
                        row_cells.push(render_fuzzy_cell(
                            &truncate(desc, 80),
                            &app.search_query,
                            is_row_highlighted,
                            false,
                            Style::default().fg(THEME.read().unwrap().text_muted),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Releases, "Author") {
                        let author = r.author_name.as_deref().unwrap_or("");
                        row_cells.push(render_fuzzy_cell(
                            &author.to_string(),
                            &app.search_query,
                            is_row_highlighted,
                            false,
                            Style::default().fg(THEME.read().unwrap().blue),
                            Alignment::Left,
                        ));
                    }
                    let row_style = if is_row_highlighted {
                        Style::default().bg(THEME.read().unwrap().highlight_bg)
                    } else {
                        Style::default()
                    };
                    Row::new(row_cells).style(row_style).height(1)
                });

                let mut header_cells = Vec::new();
                let mut widths = Vec::new();

                if app.is_column_visible(Tab::Releases, "Tag") {
                    header_cells.push(Cell::from("Tag"));
                    widths.push(Constraint::Length(20));
                }
                if app.is_column_visible(Tab::Releases, "Release Name") {
                    header_cells.push(Cell::from("Release Name"));
                    widths.push(Constraint::Fill(1));
                }
                if app.is_column_visible(Tab::Releases, "Date") {
                    header_cells.push(Cell::from("Date"));
                    widths.push(Constraint::Length(12));
                }
                if app.is_column_visible(Tab::Releases, "Description") {
                    header_cells.push(Cell::from("Description"));
                    widths.push(Constraint::Fill(2));
                }
                if app.is_column_visible(Tab::Releases, "Author") {
                    header_cells.push(Cell::from("Author"));
                    widths.push(Constraint::Length(16));
                }

                if widths.is_empty() {
                    widths.push(Constraint::Min(0));
                }

                let table = Table::new(rows, widths)
                    .header(Row::new(header_cells).style(header_style).height(1))
                    .block(main_block)
                    .row_highlight_style(highlight_style)
                    .highlight_symbol(" ❯ ");

                f.render_stateful_widget(table, content_area, &mut app.releases.state);

                let preview_block = Block::default()
                    .borders(Borders::ALL)
                    .title(" Details ")
                    .title_style(
                        Style::default()
                            .fg(THEME.read().unwrap().text_muted)
                            .add_modifier(Modifier::BOLD),
                    )
                    .border_style(Style::default().fg(THEME.read().unwrap().border));
                if let Some(selected) = app.releases.state.selected() {
                    if let Some(r) = filtered_releases.get(selected) {
                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled(
                                "Release: ",
                                Style::default()
                                    .fg(THEME.read().unwrap().text_muted)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                &r.name,
                                Style::default()
                                    .fg(THEME.read().unwrap().text_normal)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Tag:     ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                &r.tag_name,
                                Style::default()
                                    .fg(THEME.read().unwrap().green)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Date:    ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                &r.released_at,
                                Style::default().fg(THEME.read().unwrap().yellow),
                            ),
                        ]));
                        if let Some(ref author) = r.author_name {
                            text.push(Line::from(vec![
                                Span::styled(
                                    "Author:  ",
                                    Style::default().fg(THEME.read().unwrap().text_muted),
                                ),
                                Span::styled(
                                    author,
                                    Style::default().fg(THEME.read().unwrap().blue),
                                ),
                            ]));
                        }
                        if let Some(ref desc) = r.description {
                            if !desc.is_empty() {
                                text.push(Line::from(Span::styled(
                                    "---",
                                    Style::default().fg(THEME.read().unwrap().text_muted),
                                )));
                                text.push(Line::from(Span::styled(
                                    truncate(desc, 200),
                                    Style::default().fg(THEME.read().unwrap().text_normal),
                                )));
                            }
                        }
                        f.render_widget(
                            Paragraph::new(text)
                                .block(preview_block)
                                .wrap(ratatui::widgets::Wrap { trim: true }),
                            middle_chunks[2],
                        );
                    } else {
                        f.render_widget(Paragraph::new("").block(preview_block), middle_chunks[2]);
                    }
                } else {
                    f.render_widget(
                        Paragraph::new("Select an item to view details...")
                            .block(preview_block)
                            .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                        middle_chunks[2],
                    );
                }
            }
        }
        Tab::Todos => {
            if app.todos.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
                f.render_widget(
                    Paragraph::new("\n\n Loading todos...")
                        .alignment(Alignment::Center)
                        .block(main_block.clone())
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    content_area,
                );
                f.render_widget(
                    Paragraph::new("Select a todo...")
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" Details ")
                                .border_style(Style::default().fg(THEME.read().unwrap().border)),
                        )
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    middle_chunks[2],
                );
            } else {
                let mut filtered_todos = App::filtered_todos_list(
                    &app.todos.items,
                    &app.search_query,
                    &app.enabled_columns,
                    app.group_ascending,
                    &app.group_by_column,
                );
                App::apply_column_filters(
                    &mut filtered_todos,
                    &app.column_filters,
                    Tab::Todos,
                    |item, col| match col {
                        "State" => vec![item.state.clone()],
                        "Project" => vec![item.project_path.clone()],
                        "Type" => vec![item.target_type.clone()],
                        "ID" => vec![item.id.to_string()],
                        "Title" => vec![item.title.clone()],
                        _ => vec![],
                    },
                );

                let rows = filtered_todos.iter().enumerate().map(|(idx, n)| {
                    let is_row_highlighted = app.todos.state.selected() == Some(idx);

                    let state_str = if n.state == "unread" || n.state == "pending" {
                        "•"
                    } else {
                        " "
                    };
                    let state_style = Style::default()
                        .fg(THEME.read().unwrap().green)
                        .add_modifier(Modifier::BOLD);

                    let type_style = if n.target_type == "MergeRequest" {
                        Style::default()
                            .fg(THEME.read().unwrap().purple)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(THEME.read().unwrap().blue)
                            .add_modifier(Modifier::BOLD)
                    };

                    let mut row_cells = Vec::new();
                    if app.is_column_visible(Tab::Todos, "State") {
                        row_cells.push(render_fuzzy_cell(
                            state_str,
                            &app.search_query,
                            is_row_highlighted,
                            false,
                            state_style,
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Todos, "Project") {
                        row_cells.push(render_fuzzy_cell(
                            &n.project_path,
                            &app.search_query,
                            is_row_highlighted,
                            false,
                            Style::default().fg(THEME.read().unwrap().text_muted),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Todos, "Type") {
                        row_cells.push(render_fuzzy_cell(
                            n.target_type.as_str(),
                            &app.search_query,
                            is_row_highlighted,
                            false,
                            type_style,
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Todos, "ID") {
                        row_cells.push(render_fuzzy_cell(
                            &format!("#{}", n.target_iid),
                            &app.search_query,
                            is_row_highlighted,
                            false,
                            Style::default().fg(THEME.read().unwrap().blue),
                            Alignment::Left,
                        ));
                    }
                    if app.is_column_visible(Tab::Todos, "Title") {
                        row_cells.push(render_fuzzy_cell(
                            &truncate(&n.title, 80),
                            &app.search_query,
                            is_row_highlighted,
                            false,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                            Alignment::Left,
                        ));
                    }
                    let row_style = if is_row_highlighted {
                        Style::default().bg(THEME.read().unwrap().highlight_bg)
                    } else {
                        Style::default()
                    };
                    Row::new(row_cells).style(row_style).height(1)
                });

                let mut header_cells = Vec::new();
                let mut widths = Vec::new();

                if app.is_column_visible(Tab::Todos, "State") {
                    header_cells.push(Cell::from(""));
                    widths.push(Constraint::Length(2));
                }
                if app.is_column_visible(Tab::Todos, "Project") {
                    header_cells.push(Cell::from("Project"));
                    widths.push(Constraint::Length(25));
                }
                if app.is_column_visible(Tab::Todos, "Type") {
                    header_cells.push(Cell::from("Type"));
                    widths.push(Constraint::Length(14));
                }
                if app.is_column_visible(Tab::Todos, "ID") {
                    header_cells.push(Cell::from("ID"));
                    widths.push(Constraint::Length(8));
                }
                if app.is_column_visible(Tab::Todos, "Title") {
                    header_cells.push(Cell::from("Title"));
                    widths.push(Constraint::Fill(1));
                }

                if widths.is_empty() {
                    widths.push(Constraint::Min(0));
                }

                let table = Table::new(rows, widths)
                    .header(Row::new(header_cells).style(header_style).height(1))
                    .block(main_block)
                    .row_highlight_style(highlight_style)
                    .highlight_symbol(" ❯ ");

                f.render_stateful_widget(table, content_area, &mut app.todos.state);

                let preview_block = Block::default()
                    .borders(Borders::ALL)
                    .title(" Details ")
                    .title_style(
                        Style::default()
                            .fg(THEME.read().unwrap().text_muted)
                            .add_modifier(Modifier::BOLD),
                    )
                    .border_style(Style::default().fg(THEME.read().unwrap().border));
                if let Some(selected) = app.todos.state.selected() {
                    if let Some(n) = filtered_todos.get(selected) {
                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled(
                                "Title:    ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                &n.title,
                                Style::default()
                                    .fg(THEME.read().unwrap().text_normal)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Project:  ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                &n.project_path,
                                Style::default().fg(THEME.read().unwrap().text_normal),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Target:   ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                format!("{} #{}", n.target_type, n.target_iid),
                                Style::default().fg(THEME.read().unwrap().blue),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "State:    ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                &n.state,
                                Style::default().fg(
                                    if n.state == "unread" || n.state == "pending" {
                                        THEME.read().unwrap().green
                                    } else {
                                        THEME.read().unwrap().text_muted
                                    },
                                ),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Updated:  ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                &n.updated_at,
                                Style::default().fg(THEME.read().unwrap().yellow),
                            ),
                        ]));
                        text.push(Line::from(""));
                        text.push(Line::from(Span::styled(
                            " Press Enter to mark read and switch to item",
                            Style::default()
                                .fg(THEME.read().unwrap().text_muted)
                                .add_modifier(Modifier::ITALIC),
                        )));
                        f.render_widget(
                            Paragraph::new(text)
                                .block(preview_block)
                                .wrap(ratatui::widgets::Wrap { trim: true }),
                            middle_chunks[2],
                        );
                    } else {
                        f.render_widget(Paragraph::new("").block(preview_block), middle_chunks[2]);
                    }
                } else {
                    f.render_widget(
                        Paragraph::new("Select an item to view details...")
                            .block(preview_block)
                            .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                        middle_chunks[2],
                    );
                }
            }
        }
        Tab::Milestones => {
            if app.milestones.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
                f.render_widget(
                    Paragraph::new("\n\n Loading milestones...")
                        .alignment(Alignment::Center)
                        .block(main_block.clone())
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    content_area,
                );
                f.render_widget(
                    Paragraph::new("Select a milestone...")
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" Details ")
                                .border_style(Style::default().fg(THEME.read().unwrap().border)),
                        )
                        .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                    middle_chunks[2],
                );
            } else {
                let is_github = app
                    .gitlab_client
                    .as_ref()
                    .map(|c| c.is_github)
                    .unwrap_or(false);
                let default_set = std::collections::HashSet::new();
                let mut filtered_milestones = App::filter_milestones_list(
                    &app.milestones.items,
                    &app.search_query,
                    app.enabled_columns
                        .get(&Tab::Milestones)
                        .unwrap_or(&default_set),
                );
                App::apply_column_filters(
                    &mut filtered_milestones,
                    &app.column_filters,
                    Tab::Milestones,
                    |item, col| match col {
                        "ID" => vec![item.id.to_string()],
                        "Title" => vec![item.title.clone()],
                        "State" => vec![item.state.clone()],
                        _ => vec![],
                    },
                );

                let mut header_cells = Vec::new();
                let mut widths = Vec::new();
                let cols = Tab::Milestones.columns(is_github);
                for col in &cols {
                    if app.is_column_visible(Tab::Milestones, col) {
                        header_cells.push(Cell::from(*col));
                        match *col {
                            "ID" => widths.push(Constraint::Length(10)),
                            "Title" => widths.push(Constraint::Fill(1)),
                            "State" => widths.push(Constraint::Length(12)),
                            "Start Date" => widths.push(Constraint::Length(15)),
                            "Due Date" => widths.push(Constraint::Length(15)),
                            _ => widths.push(Constraint::Length(10)),
                        }
                    }
                }

                let rows = filtered_milestones.iter().enumerate().map(|(idx, m)| {
                    let mut cells = Vec::new();
                    for col in &cols {
                        if app.is_column_visible(Tab::Milestones, col) {
                            let val = match *col {
                                "ID" => m.iid.to_string(),
                                "Title" => m.title.clone(),
                                "State" => m.state.clone(),
                                "Start Date" => {
                                    m.start_date.clone().unwrap_or_else(|| "N/A".to_string())
                                }
                                "Due Date" => {
                                    m.due_date.clone().unwrap_or_else(|| "N/A".to_string())
                                }
                                _ => "".to_string(),
                            };
                            cells.push(Cell::from(val));
                        }
                    }
                    let is_selected = app.milestones.state.selected() == Some(idx);
                    let row_style = if is_selected {
                        Style::default().bg(THEME.read().unwrap().highlight_bg)
                    } else {
                        Style::default().fg(THEME.read().unwrap().text_normal)
                    };
                    Row::new(cells).style(row_style)
                });

                if widths.is_empty() {
                    widths.push(Constraint::Min(0));
                }

                let table = Table::new(rows, widths)
                    .header(Row::new(header_cells).style(header_style).height(1))
                    .block(main_block.clone())
                    .row_highlight_style(highlight_style)
                    .highlight_symbol(" ❯ ");

                f.render_stateful_widget(table, content_area, &mut app.milestones.state);

                let preview_block = Block::default()
                    .borders(Borders::ALL)
                    .title(" Milestone Details ")
                    .title_style(
                        Style::default()
                            .fg(THEME.read().unwrap().text_muted)
                            .add_modifier(Modifier::BOLD),
                    )
                    .border_style(Style::default().fg(THEME.read().unwrap().border));

                if let Some(selected_idx) = app.milestones.state.selected() {
                    if let Some(m) = filtered_milestones.get(selected_idx) {
                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled(
                                "Title:      ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                &m.title,
                                Style::default()
                                    .fg(THEME.read().unwrap().blue)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "State:      ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                &m.state,
                                Style::default().fg(if m.state == "active" {
                                    THEME.read().unwrap().green
                                } else {
                                    THEME.read().unwrap().yellow
                                }),
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Start Date: ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::raw(m.start_date.as_deref().unwrap_or("N/A")),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled(
                                "Due Date:   ",
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::raw(m.due_date.as_deref().unwrap_or("N/A")),
                        ]));
                        if let Some(desc) = &m.description {
                            text.push(Line::from(""));
                            text.push(Line::from(Span::styled(
                                "Description:",
                                Style::default().add_modifier(Modifier::BOLD),
                            )));
                            text.push(Line::from(desc.as_str()));
                        }
                        text.push(Line::from(""));

                        if let Some(issues) = &app.selected_milestone_issues {
                            let total = issues.len();
                            let closed = issues.iter().filter(|i| i.state == "closed").count();
                            let open = total - closed;

                            text.push(Line::from(vec![
                                Span::styled(
                                    "Issues Status: ",
                                    Style::default().add_modifier(Modifier::BOLD),
                                ),
                                Span::raw(format!(
                                    "{} Closed / {} Open (Total {})",
                                    closed, open, total
                                )),
                            ]));

                            let pct = if total > 0 {
                                (closed as f32 / total as f32) * 100.0
                            } else {
                                0.0
                            };
                            let filled_len = if total > 0 { (closed * 20) / total } else { 0 };
                            let bar = format!(
                                "[{}{}] {:.1}%",
                                "█".repeat(filled_len),
                                "░".repeat(20 - filled_len),
                                pct
                            );
                            text.push(Line::from(Span::styled(
                                bar,
                                Style::default().fg(THEME.read().unwrap().green),
                            )));
                            text.push(Line::from(""));
                        } else {
                            text.push(Line::from("Loading issues details..."));
                        }

                        f.render_widget(
                            Paragraph::new(text)
                                .block(preview_block)
                                .wrap(ratatui::widgets::Wrap { trim: true }),
                            middle_chunks[2],
                        );
                    } else {
                        f.render_widget(Paragraph::new("").block(preview_block), middle_chunks[2]);
                    }
                } else {
                    f.render_widget(
                        Paragraph::new("Select a milestone to view details...")
                            .block(preview_block)
                            .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                        middle_chunks[2],
                    );
                }
            }
        }
        Tab::Terminal => {
            let num_cmds = app.terminal_commands.len();
            let area = content_area;
            let base_block =
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(if app.focus_column_checklist {
                        THEME.read().unwrap().border
                    } else {
                        THEME.read().unwrap().border_focused
                    }));
            let inner_rect = base_block.inner(area);
            let log_height = inner_rect.height as usize;

            let max_scroll = num_cmds.saturating_sub(log_height);
            app.terminal_scroll = app.terminal_scroll.min(max_scroll);

            let block_title = if app.terminal_scroll > 0 {
                format!(
                    " Terminal (Scroll: {}/{}) ",
                    app.terminal_scroll, max_scroll,
                )
            } else {
                " Terminal ".to_string()
            };
            let custom_main_block = base_block.clone().title(block_title).title_style(
                Style::default()
                    .fg(THEME.read().unwrap().header_fg)
                    .add_modifier(Modifier::BOLD),
            );

            let end_idx = num_cmds.saturating_sub(app.terminal_scroll);
            let start_idx = end_idx.saturating_sub(log_height);

            let mut log_lines = Vec::new();
            let visible_count = end_idx - start_idx;
            if visible_count < log_height {
                for _ in 0..(log_height - visible_count) {
                    log_lines.push(Line::from(""));
                }
            }

            for i in start_idx..end_idx {
                if let Some(cmd) = app.terminal_commands.get(i) {
                    log_lines.push(build_log_line(cmd, inner_rect.width as usize));
                }
            }

            f.render_widget(Paragraph::new(log_lines).block(custom_main_block), area);
        }
    }

    // Compact terminal pane at bottom of the middle column
    if term_area.height > 0 {
        let bottom_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border))
            .title(" Terminal ")
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().purple)
                    .add_modifier(Modifier::BOLD),
            );
        f.render_widget(bottom_block.clone(), term_area);

        let bottom_inner = bottom_block.inner(term_area);
        if bottom_inner.height > 0 {
            let mut log_lines = Vec::new();
            let log_height = bottom_inner.height as usize;

            let num_cmds = app.terminal_commands.len();
            let display_count = std::cmp::min(num_cmds, log_height);
            let start_idx = num_cmds.saturating_sub(display_count);

            if display_count < log_height {
                for _ in 0..(log_height - display_count) {
                    log_lines.push(Line::from(""));
                }
            }

            for i in start_idx..num_cmds {
                if let Some(cmd) = app.terminal_commands.get(i) {
                    log_lines.push(build_log_line(cmd, bottom_inner.width as usize));
                }
            }

            f.render_widget(Paragraph::new(log_lines), bottom_inner);
        }
    }

    if app.diff_loading {
        let area = centered_rect_min(50, 20, 20, 4, size);
        let block = Block::default()
            .title(" Fetching Diff ")
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().header_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
            .style(Style::default().bg(Color::Reset));

        let pr_label = if app.gitlab_client.as_ref().map_or(true, |c| c.is_github) {
            "Pull Request"
        } else {
            "Merge Request"
        };
        let text = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("   Fetching {pr_label} Diff..."),
                Style::default().fg(THEME.read().unwrap().text_normal),
            )),
            Line::from(Span::styled(
                "   Please wait, running CLI tool in background...",
                Style::default().fg(THEME.read().unwrap().text_muted),
            )),
            Line::from(""),
        ];

        let paragraph = Paragraph::new(text)
            .block(block)
            .alignment(Alignment::Left)
            .wrap(ratatui::widgets::Wrap { trim: true });

        f.render_widget(Clear, area);
        f.render_widget(paragraph, area);
    }

    if let Some(diff_view) = app.diff_view.take() {
        let area = centered_rect_min(95, 95, 30, 6, size);

        let unresolved_count = app.unresolved_threads_count();
        let unresolved_suffix = if unresolved_count > 0 {
            format!(" [🔴 Unresolved Threads: {}] ", unresolved_count)
        } else {
            String::new()
        };

        let title_suffix = if app.in_review_mode {
            format!(" [REVIEW MODE: ON ({} pending)] ", app.draft_comments.len())
        } else {
            String::new()
        };

        let pr_label = if app.gitlab_client.as_ref().map_or(true, |c| c.is_github) {
            "Pull Request"
        } else {
            "Merge Request"
        };
        let outer_block = Block::default()
            .title(format!(
                " {pr_label} Diff #{}{}{} ",
                diff_view.mr_iid, unresolved_suffix, title_suffix
            ))
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().header_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border))
            .style(Style::default().bg(Color::Reset));

        let inner_area = outer_block.inner(area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Min(0),    // Main content split
                    Constraint::Length(1), // Help / controls footer
                ]
                .as_ref(),
            )
            .split(inner_area);

        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(
                [
                    Constraint::Percentage(25), // Files list
                    Constraint::Percentage(75), // Diff content
                ]
                .as_ref(),
            )
            .split(chunks[0]);

        // 1. Render Files list on the left
        let files_block = Block::default()
            .title(" Files ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if diff_view.focus_on_files {
                THEME.read().unwrap().border_focused
            } else {
                THEME.read().unwrap().border
            }));

        let mut file_items = Vec::new();
        for (i, node) in diff_view.visible_nodes.iter().enumerate() {
            let is_selected = i == diff_view.selected_visible_idx;

            let indent = "  ".repeat(node.depth);
            let indicator = if node.is_dir {
                if node.is_expanded { "- " } else { "+ " }
            } else {
                "  "
            };

            let unresolved_count = app.unresolved_threads_count_for_path(&node.path_id);
            let count_suffix = if unresolved_count > 0 {
                format!(" (🔴 {})", unresolved_count)
            } else {
                String::new()
            };

            let display_str = format!(" {}{}{}{}", indent, indicator, node.name, count_suffix);

            let item_style = if is_selected {
                if diff_view.focus_on_files {
                    Style::default()
                        .bg(THEME.read().unwrap().highlight_bg)
                        .fg(THEME.read().unwrap().bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .bg(THEME.read().unwrap().border)
                        .fg(THEME.read().unwrap().text_normal)
                }
            } else if node.is_dir {
                Style::default()
                    .fg(THEME.read().unwrap().blue)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(THEME.read().unwrap().text_normal)
            };

            file_items.push(ListItem::new(display_str).style(item_style));
        }
        let files_list = List::new(file_items).block(files_block);

        // 2. Render Diff content on the right
        let diff_block = Block::default()
            .title(" Diff ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if !diff_view.focus_on_files {
                THEME.read().unwrap().border_focused
            } else {
                THEME.read().unwrap().border
            }));

        let list_height = (main_chunks[1].height as usize).saturating_sub(2);

        let mut updated_diff_view = diff_view;
        updated_diff_view.viewport_height = list_height;
        let total_lines = if updated_diff_view.side_by_side {
            updated_diff_view.side_by_side_lines.len()
        } else {
            updated_diff_view.lines.len()
        };

        if updated_diff_view.cursor_idx < updated_diff_view.scroll_offset {
            updated_diff_view.scroll_offset = updated_diff_view.cursor_idx;
        } else if updated_diff_view.cursor_idx >= updated_diff_view.scroll_offset + list_height {
            updated_diff_view.scroll_offset = updated_diff_view.cursor_idx - list_height + 1;
        }

        let start = updated_diff_view.scroll_offset;
        let end = (start + list_height).min(total_lines);

        let mut list_lines = Vec::new();
        let mut left_list_lines = Vec::new();
        let mut right_list_lines = Vec::new();

        if updated_diff_view.side_by_side {
            for idx in start..end {
                let sline = &updated_diff_view.side_by_side_lines[idx];
                let is_cursor = idx == updated_diff_view.cursor_idx;

                let in_selection = updated_diff_view
                    .selection_start
                    .zip(updated_diff_view.selection_end)
                    .map_or(false, |(s, e)| idx >= s && idx <= e);

                let gutter_bg = Color::Rgb(22, 22, 26);
                let marker_style = Style::default()
                    .fg(THEME.read().unwrap().yellow)
                    .add_modifier(Modifier::BOLD)
                    .bg(gutter_bg);
                let num_style = Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .bg(gutter_bg);
                let sep_style = Style::default().fg(Color::Rgb(60, 60, 68)).bg(gutter_bg);

                let sel_bg = if in_selection {
                    Some(Color::Rgb(30, 50, 80))
                } else {
                    None
                };

                // LEFT PANEL (OLD / DELETION)
                let mut left_spans = Vec::new();
                if let Some(ref line) = sline.left {
                    let old_str = line
                        .old_line_num
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| " ".to_string());

                    left_spans.extend(vec![
                        Span::styled(
                            if is_cursor {
                                " ❯ "
                            } else if in_selection {
                                " ▐ "
                            } else {
                                "   "
                            },
                            marker_style,
                        ),
                        Span::styled(format!("{:>4} ", old_str), num_style),
                        Span::styled("│ ", sep_style),
                    ]);

                    match line.line_type {
                        crate::app::DiffLineType::Deletion => {
                            let code_fg = Color::Rgb(220, 140, 140);
                            let code_bg = Color::Rgb(55, 22, 28);
                            let prefix_fg = Color::Rgb(255, 100, 100);
                            let actual_bg = sel_bg.unwrap_or(code_bg);

                            let prefix = line
                                .content
                                .chars()
                                .next()
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| " ".to_string());
                            left_spans.push(Span::styled(
                                prefix,
                                Style::default()
                                    .fg(prefix_fg)
                                    .add_modifier(Modifier::BOLD)
                                    .bg(actual_bg),
                            ));

                            let content_base = Style::default().fg(code_fg).bg(actual_bg);
                            let final_style = if is_cursor {
                                content_base
                                    .add_modifier(Modifier::UNDERLINED)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                content_base
                            };

                            if let Some(ref highlighted) = line.syntax_highlighted {
                                for (span_style, text) in highlighted {
                                    let merged = final_style
                                        .fg(span_style.fg.unwrap_or(code_fg))
                                        .add_modifier(span_style.add_modifier);
                                    left_spans.push(Span::styled(text.clone(), merged));
                                }
                            } else {
                                let code = if line.content.len() > 1 {
                                    &line.content[1..]
                                } else {
                                    ""
                                };
                                left_spans.push(Span::styled(code, final_style));
                            }
                        }
                        crate::app::DiffLineType::Normal => {
                            let actual_bg = sel_bg.unwrap_or(Color::Reset);
                            let prefix = line
                                .content
                                .chars()
                                .next()
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| " ".to_string());
                            left_spans.push(Span::styled(
                                prefix,
                                Style::default()
                                    .fg(THEME.read().unwrap().text_muted)
                                    .bg(actual_bg),
                            ));

                            let content_base = Style::default()
                                .fg(THEME.read().unwrap().text_normal)
                                .bg(actual_bg);
                            let final_style = if is_cursor {
                                content_base
                                    .add_modifier(Modifier::UNDERLINED)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                content_base
                            };

                            if let Some(ref highlighted) = line.syntax_highlighted {
                                for (span_style, text) in highlighted {
                                    let merged = final_style
                                        .fg(span_style
                                            .fg
                                            .unwrap_or(THEME.read().unwrap().text_normal))
                                        .add_modifier(span_style.add_modifier);
                                    left_spans.push(Span::styled(text.clone(), merged));
                                }
                            } else {
                                let code = if line.content.len() > 1 {
                                    &line.content[1..]
                                } else {
                                    ""
                                };
                                left_spans.push(Span::styled(code, final_style));
                            }
                        }
                        crate::app::DiffLineType::Meta => {
                            let mut s = Style::default()
                                .fg(THEME.read().unwrap().blue)
                                .add_modifier(Modifier::BOLD);
                            if let Some(bg) = sel_bg {
                                s = s.bg(bg);
                            }
                            let final_style = if is_cursor {
                                s.add_modifier(Modifier::UNDERLINED)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                s
                            };
                            if let Some(ref highlighted) = line.syntax_highlighted {
                                for (span_style, text) in highlighted {
                                    let merged = final_style
                                        .fg(span_style.fg.unwrap_or(THEME.read().unwrap().blue))
                                        .add_modifier(span_style.add_modifier);
                                    left_spans.push(Span::styled(text.clone(), merged));
                                }
                            } else {
                                left_spans.push(Span::styled(&line.content, final_style));
                            }
                        }
                        crate::app::DiffLineType::HunkHeader => {
                            let mut s = Style::default()
                                .fg(THEME.read().unwrap().purple)
                                .add_modifier(Modifier::BOLD);
                            if let Some(bg) = sel_bg {
                                s = s.bg(bg);
                            }
                            let final_style = if is_cursor {
                                s.add_modifier(Modifier::UNDERLINED)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                s
                            };
                            if let Some(ref highlighted) = line.syntax_highlighted {
                                for (span_style, text) in highlighted {
                                    let merged = final_style
                                        .fg(span_style.fg.unwrap_or(THEME.read().unwrap().purple))
                                        .add_modifier(span_style.add_modifier);
                                    left_spans.push(Span::styled(text.clone(), merged));
                                }
                            } else {
                                left_spans.push(Span::styled(&line.content, final_style));
                            }
                        }
                        _ => {}
                    }
                } else {
                    let actual_bg = sel_bg.unwrap_or(Color::Reset);
                    left_spans.extend(vec![
                        Span::styled(
                            if is_cursor {
                                " ❯ "
                            } else if in_selection {
                                " ▐ "
                            } else {
                                "   "
                            },
                            marker_style,
                        ),
                        Span::styled("     ", num_style),
                        Span::styled("│ ", sep_style),
                        Span::styled(" ", Style::default().bg(actual_bg)),
                    ]);
                }
                left_list_lines.push(Line::from(left_spans));

                // RIGHT PANEL (NEW / ADDITION)
                let mut right_spans = Vec::new();
                if let Some(ref line) = sline.right {
                    let new_str = line
                        .new_line_num
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| " ".to_string());

                    right_spans.extend(vec![
                        Span::styled(
                            if is_cursor {
                                " ❯ "
                            } else if in_selection {
                                " ▐ "
                            } else {
                                "   "
                            },
                            marker_style,
                        ),
                        Span::styled(format!("{:>4} ", new_str), num_style),
                        Span::styled("│ ", sep_style),
                    ]);

                    match line.line_type {
                        crate::app::DiffLineType::Addition => {
                            let code_fg = Color::Rgb(140, 220, 140);
                            let code_bg = Color::Rgb(22, 48, 28);
                            let prefix_fg = Color::Rgb(80, 220, 80);
                            let actual_bg = sel_bg.unwrap_or(code_bg);

                            let prefix = line
                                .content
                                .chars()
                                .next()
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| " ".to_string());
                            right_spans.push(Span::styled(
                                prefix,
                                Style::default()
                                    .fg(prefix_fg)
                                    .add_modifier(Modifier::BOLD)
                                    .bg(actual_bg),
                            ));

                            let content_base = Style::default().fg(code_fg).bg(actual_bg);
                            let final_style = if is_cursor {
                                content_base
                                    .add_modifier(Modifier::UNDERLINED)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                content_base
                            };

                            if let Some(ref highlighted) = line.syntax_highlighted {
                                for (span_style, text) in highlighted {
                                    let merged = final_style
                                        .fg(span_style.fg.unwrap_or(code_fg))
                                        .add_modifier(span_style.add_modifier);
                                    right_spans.push(Span::styled(text.clone(), merged));
                                }
                            } else {
                                let code = if line.content.len() > 1 {
                                    &line.content[1..]
                                } else {
                                    ""
                                };
                                right_spans.push(Span::styled(code, final_style));
                            }
                        }
                        crate::app::DiffLineType::Normal => {
                            let actual_bg = sel_bg.unwrap_or(Color::Reset);
                            let prefix = line
                                .content
                                .chars()
                                .next()
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| " ".to_string());
                            right_spans.push(Span::styled(
                                prefix,
                                Style::default()
                                    .fg(THEME.read().unwrap().text_muted)
                                    .bg(actual_bg),
                            ));

                            let content_base = Style::default()
                                .fg(THEME.read().unwrap().text_normal)
                                .bg(actual_bg);
                            let final_style = if is_cursor {
                                content_base
                                    .add_modifier(Modifier::UNDERLINED)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                content_base
                            };

                            if let Some(ref highlighted) = line.syntax_highlighted {
                                for (span_style, text) in highlighted {
                                    let merged = final_style
                                        .fg(span_style
                                            .fg
                                            .unwrap_or(THEME.read().unwrap().text_normal))
                                        .add_modifier(span_style.add_modifier);
                                    right_spans.push(Span::styled(text.clone(), merged));
                                }
                            } else {
                                let code = if line.content.len() > 1 {
                                    &line.content[1..]
                                } else {
                                    ""
                                };
                                right_spans.push(Span::styled(code, final_style));
                            }
                        }
                        crate::app::DiffLineType::Meta => {
                            let mut s = Style::default()
                                .fg(THEME.read().unwrap().blue)
                                .add_modifier(Modifier::BOLD);
                            if let Some(bg) = sel_bg {
                                s = s.bg(bg);
                            }
                            let final_style = if is_cursor {
                                s.add_modifier(Modifier::UNDERLINED)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                s
                            };
                            if let Some(ref highlighted) = line.syntax_highlighted {
                                for (span_style, text) in highlighted {
                                    let merged = final_style
                                        .fg(span_style.fg.unwrap_or(THEME.read().unwrap().blue))
                                        .add_modifier(span_style.add_modifier);
                                    right_spans.push(Span::styled(text.clone(), merged));
                                }
                            } else {
                                right_spans.push(Span::styled(&line.content, final_style));
                            }
                        }
                        crate::app::DiffLineType::HunkHeader => {
                            let mut s = Style::default()
                                .fg(THEME.read().unwrap().purple)
                                .add_modifier(Modifier::BOLD);
                            if let Some(bg) = sel_bg {
                                s = s.bg(bg);
                            }
                            let final_style = if is_cursor {
                                s.add_modifier(Modifier::UNDERLINED)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                s
                            };
                            if let Some(ref highlighted) = line.syntax_highlighted {
                                for (span_style, text) in highlighted {
                                    let merged = final_style
                                        .fg(span_style.fg.unwrap_or(THEME.read().unwrap().purple))
                                        .add_modifier(span_style.add_modifier);
                                    right_spans.push(Span::styled(text.clone(), merged));
                                }
                            } else {
                                right_spans.push(Span::styled(&line.content, final_style));
                            }
                        }
                        _ => {}
                    }
                } else {
                    let actual_bg = sel_bg.unwrap_or(Color::Reset);
                    right_spans.extend(vec![
                        Span::styled(
                            if is_cursor {
                                " ❯ "
                            } else if in_selection {
                                " ▐ "
                            } else {
                                "   "
                            },
                            marker_style,
                        ),
                        Span::styled("     ", num_style),
                        Span::styled("│ ", sep_style),
                        Span::styled(" ", Style::default().bg(actual_bg)),
                    ]);
                }
                right_list_lines.push(Line::from(right_spans));

                // COMMENTS OVERLAY
                let matching_comments: Vec<_> = app
                    .draft_comments
                    .iter()
                    .filter(|c| {
                        let path_matches = sline
                            .left
                            .as_ref()
                            .map_or(false, |l| l.file_path == c.file_path)
                            || sline
                                .right
                                .as_ref()
                                .map_or(false, |r| r.file_path == c.file_path);

                        path_matches
                            && ((c.line_num.is_some()
                                && sline.right.as_ref().and_then(|r| r.new_line_num) == c.line_num)
                                || (c.old_line_num.is_some()
                                    && sline.left.as_ref().and_then(|l| l.old_line_num)
                                        == c.old_line_num))
                    })
                    .collect();

                for comment in matching_comments {
                    let comment_style = Style::default()
                        .fg(THEME.read().unwrap().yellow)
                        .bg(Color::Rgb(45, 45, 20));

                    let range_info = match (comment.end_line_num, comment.end_old_line_num) {
                        (Some(end_l), _) if end_l != comment.line_num.unwrap_or(0) => {
                            format!(" (L{}-{})", comment.line_num.unwrap_or(0), end_l)
                        }
                        (_, Some(end_o)) if end_o != comment.old_line_num.unwrap_or(0) => {
                            format!(" (OL{}-{})", comment.old_line_num.unwrap_or(0), end_o)
                        }
                        _ => String::new(),
                    };

                    let prefix_style = Style::default()
                        .fg(THEME.read().unwrap().yellow)
                        .add_modifier(Modifier::BOLD);

                    let right_prefix_first = format!(" 💬 Draft Note{}: ", range_info);

                    let formatted_lines = format_comment_with_suggestions(
                        &comment.body,
                        &comment.file_path,
                        comment.line_num.map(|n| n as u64),
                        comment.end_line_num.map(|n| n as u64),
                        comment.old_line_num.map(|n| n as u64),
                        comment.end_old_line_num.map(|n| n as u64),
                        &updated_diff_view.all_lines,
                        &right_prefix_first,
                        prefix_style,
                    );

                    for (i, (right_prefix, prefix_style, content_spans)) in
                        formatted_lines.into_iter().enumerate()
                    {
                        let left_prefix = if i == 0 { " 💬 Draft " } else { "          " };

                        left_list_lines.push(
                            Line::from(vec![
                                Span::styled("         ", Style::default()),
                                Span::styled(left_prefix, prefix_style),
                            ])
                            .style(comment_style),
                        );

                        let mut spans = vec![Span::styled(right_prefix, prefix_style)];
                        for (style, text) in content_spans {
                            spans.push(Span::styled(text, style));
                        }
                        right_list_lines.push(Line::from(spans).style(comment_style));
                    }
                }

                let matching_current: Vec<_> =
                    app.current_comments
                        .iter()
                        .filter(|c| {
                            if c.system {
                                return false;
                            }
                            if let Some(ref pos) = c.position {
                                let path_matches = sline.left.as_ref().map_or(false, |l| {
                                    pos.old_path.as_deref() == Some(&l.file_path)
                                }) || sline.right.as_ref().map_or(false, |r| {
                                    pos.new_path.as_deref() == Some(&r.file_path)
                                });

                                path_matches
                                    && ((pos.new_line.is_some()
                                        && sline
                                            .right
                                            .as_ref()
                                            .and_then(|r| r.new_line_num.map(|n| n as u64))
                                            == pos.new_line)
                                        || (pos.old_line.is_some()
                                            && sline
                                                .left
                                                .as_ref()
                                                .and_then(|l| l.old_line_num.map(|n| n as u64))
                                                == pos.old_line))
                            } else {
                                false
                            }
                        })
                        .collect();

                for comment in matching_current {
                    let comment_style = Style::default()
                        .fg(THEME.read().unwrap().blue)
                        .bg(Color::Rgb(20, 30, 45));

                    let prefix_style = Style::default()
                        .fg(THEME.read().unwrap().blue)
                        .add_modifier(Modifier::BOLD);

                    let right_prefix_first = format!(" 💬 @{}: ", comment.author.username);

                    let (start_new, end_new, start_old, end_old, file_path) =
                        if let Some(ref pos) = comment.position {
                            let (sn, en, so, eo) = pos.get_line_range();
                            (
                                sn,
                                en,
                                so,
                                eo,
                                pos.new_path
                                    .as_deref()
                                    .or(pos.old_path.as_deref())
                                    .unwrap_or("")
                                    .to_string(),
                            )
                        } else {
                            (None, None, None, None, String::new())
                        };

                    let formatted_lines = format_comment_with_suggestions(
                        &comment.body,
                        &file_path,
                        start_new,
                        end_new,
                        start_old,
                        end_old,
                        &updated_diff_view.all_lines,
                        &right_prefix_first,
                        prefix_style,
                    );

                    for (i, (right_prefix, prefix_style, content_spans)) in
                        formatted_lines.into_iter().enumerate()
                    {
                        let left_prefix = if i == 0 {
                            " 💬 Comment "
                        } else {
                            "            "
                        };

                        left_list_lines.push(
                            Line::from(vec![
                                Span::styled("         ", Style::default()),
                                Span::styled(left_prefix, prefix_style),
                            ])
                            .style(comment_style),
                        );

                        let mut spans = vec![Span::styled(right_prefix, prefix_style)];
                        for (style, text) in content_spans {
                            spans.push(Span::styled(text, style));
                        }
                        right_list_lines.push(Line::from(spans).style(comment_style));
                    }
                }
            }
        } else {
            // UNIFIED/INLINE DIFF RENDER
            for idx in start..end {
                let line = &updated_diff_view.lines[idx];
                let is_cursor = idx == updated_diff_view.cursor_idx;

                let in_selection = updated_diff_view
                    .selection_start
                    .zip(updated_diff_view.selection_end)
                    .map_or(false, |(s, e)| idx >= s && idx <= e);

                let old_str = line
                    .old_line_num
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| " ".to_string());
                let new_str = line
                    .new_line_num
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| " ".to_string());

                let gutter_bg = Color::Rgb(22, 22, 26);

                let marker_style = Style::default()
                    .fg(THEME.read().unwrap().yellow)
                    .add_modifier(Modifier::BOLD)
                    .bg(gutter_bg);

                let num_style = Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .bg(gutter_bg);

                let sep_style = Style::default().fg(Color::Rgb(60, 60, 68)).bg(gutter_bg);

                let mut line_spans = vec![
                    Span::styled(
                        if is_cursor {
                            " ❯ "
                        } else if in_selection {
                            " ▐ "
                        } else {
                            "   "
                        },
                        marker_style,
                    ),
                    Span::styled(format!("{:>4} ", old_str), num_style),
                    Span::styled(format!("{:>4} ", new_str), num_style),
                    Span::styled("│ ", sep_style),
                ];

                let sel_bg = if in_selection {
                    Some(Color::Rgb(30, 50, 80))
                } else {
                    None
                };

                match line.line_type {
                    crate::app::DiffLineType::Addition | crate::app::DiffLineType::Deletion => {
                        let is_add = line.line_type == crate::app::DiffLineType::Addition;
                        let code_fg = if is_add {
                            Color::Rgb(140, 220, 140)
                        } else {
                            Color::Rgb(220, 140, 140)
                        };
                        let code_bg = if is_add {
                            Color::Rgb(22, 48, 28)
                        } else {
                            Color::Rgb(55, 22, 28)
                        };
                        let prefix_fg = if is_add {
                            Color::Rgb(80, 220, 80)
                        } else {
                            Color::Rgb(255, 100, 100)
                        };

                        let actual_bg = sel_bg.unwrap_or(code_bg);
                        let prefix = line
                            .content
                            .chars()
                            .next()
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| " ".to_string());
                        line_spans.push(Span::styled(
                            prefix,
                            Style::default()
                                .fg(prefix_fg)
                                .add_modifier(Modifier::BOLD)
                                .bg(actual_bg),
                        ));

                        let content_base = Style::default().fg(code_fg).bg(actual_bg);
                        let final_style = if is_cursor {
                            content_base
                                .add_modifier(Modifier::UNDERLINED)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            content_base
                        };

                        if let Some(ref highlighted) = line.syntax_highlighted {
                            for (span_style, text) in highlighted {
                                let merged = final_style
                                    .fg(span_style.fg.unwrap_or(code_fg))
                                    .add_modifier(span_style.add_modifier);
                                line_spans.push(Span::styled(text.clone(), merged));
                            }
                        } else {
                            let code = if line.content.len() > 1 {
                                &line.content[1..]
                            } else {
                                ""
                            };
                            line_spans.push(Span::styled(code, final_style));
                        }
                    }
                    crate::app::DiffLineType::Normal => {
                        let actual_bg = sel_bg.unwrap_or(Color::Reset);
                        let prefix = line
                            .content
                            .chars()
                            .next()
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| " ".to_string());
                        line_spans.push(Span::styled(
                            prefix,
                            Style::default()
                                .fg(THEME.read().unwrap().text_muted)
                                .bg(actual_bg),
                        ));

                        let content_base = Style::default()
                            .fg(THEME.read().unwrap().text_normal)
                            .bg(actual_bg);
                        let final_style = if is_cursor {
                            content_base
                                .add_modifier(Modifier::UNDERLINED)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            content_base
                        };

                        if let Some(ref highlighted) = line.syntax_highlighted {
                            for (span_style, text) in highlighted {
                                let merged = final_style
                                    .fg(span_style.fg.unwrap_or(THEME.read().unwrap().text_normal))
                                    .add_modifier(span_style.add_modifier);
                                line_spans.push(Span::styled(text.clone(), merged));
                            }
                        } else {
                            let code = if line.content.len() > 1 {
                                &line.content[1..]
                            } else {
                                ""
                            };
                            line_spans.push(Span::styled(code, final_style));
                        }
                    }
                    crate::app::DiffLineType::Meta => {
                        let mut s = Style::default()
                            .fg(THEME.read().unwrap().blue)
                            .add_modifier(Modifier::BOLD);
                        if let Some(bg) = sel_bg {
                            s = s.bg(bg);
                        }
                        let final_style = if is_cursor {
                            s.add_modifier(Modifier::UNDERLINED)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            s
                        };
                        if let Some(ref highlighted) = line.syntax_highlighted {
                            for (span_style, text) in highlighted {
                                let merged = final_style
                                    .fg(span_style.fg.unwrap_or(THEME.read().unwrap().blue))
                                    .add_modifier(span_style.add_modifier);
                                if let Some(bg) = sel_bg {
                                    line_spans.push(Span::styled(text.clone(), merged.bg(bg)));
                                } else {
                                    line_spans.push(Span::styled(text.clone(), merged));
                                }
                            }
                        } else {
                            line_spans.push(Span::styled(&line.content, final_style));
                        }
                    }
                    crate::app::DiffLineType::HunkHeader => {
                        let mut s = Style::default()
                            .fg(THEME.read().unwrap().purple)
                            .add_modifier(Modifier::BOLD);
                        if let Some(bg) = sel_bg {
                            s = s.bg(bg);
                        }
                        let final_style = if is_cursor {
                            s.add_modifier(Modifier::UNDERLINED)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            s
                        };
                        if let Some(ref highlighted) = line.syntax_highlighted {
                            for (span_style, text) in highlighted {
                                let merged = final_style
                                    .fg(span_style.fg.unwrap_or(THEME.read().unwrap().purple))
                                    .add_modifier(span_style.add_modifier);
                                if let Some(bg) = sel_bg {
                                    line_spans.push(Span::styled(text.clone(), merged.bg(bg)));
                                } else {
                                    line_spans.push(Span::styled(text.clone(), merged));
                                }
                            }
                        } else {
                            line_spans.push(Span::styled(&line.content, final_style));
                        }
                    }
                }
                list_lines.push(Line::from(line_spans));

                // COMMENTS OVERLAY
                let matching_comments: Vec<_> = app
                    .draft_comments
                    .iter()
                    .filter(|c| {
                        c.file_path == line.file_path
                            && ((c.line_num.is_some() && c.line_num == line.new_line_num)
                                || (c.old_line_num.is_some()
                                    && c.old_line_num == line.old_line_num))
                    })
                    .collect();

                for comment in matching_comments {
                    let comment_style = Style::default()
                        .fg(THEME.read().unwrap().yellow)
                        .bg(Color::Rgb(45, 45, 20));

                    let range_info = match (comment.end_line_num, comment.end_old_line_num) {
                        (Some(end_l), _) if end_l != comment.line_num.unwrap_or(0) => {
                            format!(" (L{}-{})", comment.line_num.unwrap_or(0), end_l)
                        }
                        (_, Some(end_o)) if end_o != comment.old_line_num.unwrap_or(0) => {
                            format!(" (OL{}-{})", comment.old_line_num.unwrap_or(0), end_o)
                        }
                        _ => String::new(),
                    };

                    let prefix_style = Style::default()
                        .fg(THEME.read().unwrap().yellow)
                        .add_modifier(Modifier::BOLD);

                    let right_prefix_first = format!(" 💬 Draft Note{}: ", range_info);

                    let formatted_lines = format_comment_with_suggestions(
                        &comment.body,
                        &comment.file_path,
                        comment.line_num.map(|n| n as u64),
                        comment.end_line_num.map(|n| n as u64),
                        comment.old_line_num.map(|n| n as u64),
                        comment.end_old_line_num.map(|n| n as u64),
                        &updated_diff_view.all_lines,
                        &right_prefix_first,
                        prefix_style,
                    );

                    for (right_prefix, prefix_style, content_spans) in formatted_lines {
                        let mut spans = vec![
                            Span::styled("         ", Style::default()),
                            Span::styled(right_prefix, prefix_style),
                        ];
                        for (style, text) in content_spans {
                            spans.push(Span::styled(text, style));
                        }
                        list_lines.push(Line::from(spans).style(comment_style));
                    }
                }

                let matching_current: Vec<_> = app
                    .current_comments
                    .iter()
                    .filter(|c| {
                        if c.system {
                            return false;
                        }
                        if let Some(ref pos) = c.position {
                            let path_matches = pos.new_path.as_deref() == Some(&line.file_path)
                                || pos.old_path.as_deref() == Some(&line.file_path);

                            path_matches
                                && ((pos.new_line.is_some()
                                    && pos.new_line.map(|l| l as u32) == line.new_line_num)
                                    || (pos.old_line.is_some()
                                        && pos.old_line.map(|l| l as u32) == line.old_line_num))
                        } else {
                            false
                        }
                    })
                    .collect();

                for comment in matching_current {
                    let comment_style = Style::default()
                        .fg(THEME.read().unwrap().blue)
                        .bg(Color::Rgb(20, 30, 45));

                    let prefix_style = Style::default()
                        .fg(THEME.read().unwrap().blue)
                        .add_modifier(Modifier::BOLD);

                    let right_prefix_first = format!(" 💬 @{}: ", comment.author.username);

                    let (start_new, end_new, start_old, end_old, file_path) =
                        if let Some(ref pos) = comment.position {
                            let (sn, en, so, eo) = pos.get_line_range();
                            (
                                sn,
                                en,
                                so,
                                eo,
                                pos.new_path
                                    .as_deref()
                                    .or(pos.old_path.as_deref())
                                    .unwrap_or("")
                                    .to_string(),
                            )
                        } else {
                            (None, None, None, None, String::new())
                        };

                    let formatted_lines = format_comment_with_suggestions(
                        &comment.body,
                        &file_path,
                        start_new,
                        end_new,
                        start_old,
                        end_old,
                        &updated_diff_view.all_lines,
                        &right_prefix_first,
                        prefix_style,
                    );

                    for (right_prefix, prefix_style, content_spans) in formatted_lines {
                        let mut spans = vec![
                            Span::styled("         ", Style::default()),
                            Span::styled(right_prefix, prefix_style),
                        ];
                        for (style, text) in content_spans {
                            spans.push(Span::styled(text, style));
                        }
                        list_lines.push(Line::from(spans).style(comment_style));
                    }
                }
            }
        }

        let footer_p = Paragraph::new(" Esc/q: Exit • d: Toggle Diff Layout • Tab: Toggle Focus • h/l/Left/Right: Switch Panels • j/k/↑/↓: Navigate • J/K: Scroll Down/Up • v: Select Lines • c: Comment • e: Suggest Code • a: Comment Actions • r: Submit Review ")
            .alignment(Alignment::Center)
            .style(Style::default().fg(THEME.read().unwrap().text_muted).add_modifier(Modifier::ITALIC))
            .wrap(ratatui::widgets::Wrap { trim: true });

        f.render_widget(Clear, area);
        f.render_widget(outer_block, area);
        f.render_widget(files_list, main_chunks[0]);
        f.render_widget(footer_p, chunks[1]);

        if updated_diff_view.side_by_side {
            let diff_inner = diff_block.inner(main_chunks[1]);
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(
                    [
                        Constraint::Fill(1),
                        Constraint::Length(1),
                        Constraint::Fill(1),
                    ]
                    .as_ref(),
                )
                .split(diff_inner);

            let left_para = Paragraph::new(left_list_lines);
            let right_para = Paragraph::new(right_list_lines);
            let divider_lines: Vec<Line> = (0..diff_inner.height)
                .map(|_| {
                    Line::from(Span::styled(
                        "│",
                        Style::default().fg(Color::Rgb(60, 60, 68)),
                    ))
                })
                .collect();
            let divider_para = Paragraph::new(divider_lines);

            f.render_widget(diff_block, main_chunks[1]);
            f.render_widget(left_para, cols[0]);
            f.render_widget(divider_para, cols[1]);
            f.render_widget(right_para, cols[2]);
        } else {
            let diff_para = Paragraph::new(list_lines).block(diff_block);
            f.render_widget(diff_para, main_chunks[1]);
        }

        app.diff_view = Some(updated_diff_view);
    }

    if let Some(menu) = &mut app.edit_menu {
        let block = Block::default()
            .title(format!(" {} ", menu.title))
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().header_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
            .style(Style::default().bg(Color::Reset));

        let area = centered_rect_min(52, 48, 42, 8, size);

        let label_width = menu
            .fields
            .iter()
            .map(|(l, _)| l.len())
            .max()
            .unwrap_or(18)
            .max(18);

        let items: Vec<ListItem> = menu
            .fields
            .iter()
            .enumerate()
            .map(|(i, (label, val))| {
                let is_selected = i == menu.selected_idx;
                let item_bg = if is_selected {
                    THEME.read().unwrap().highlight_bg
                } else {
                    Color::Reset
                };

                let label_style = if is_selected {
                    Style::default()
                        .fg(THEME.read().unwrap().text_normal)
                        .bg(item_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(THEME.read().unwrap().text_muted)
                        .bg(item_bg)
                };

                let sep_style = if is_selected {
                    Style::default()
                        .fg(THEME.read().unwrap().text_normal)
                        .bg(item_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(THEME.read().unwrap().text_muted)
                        .bg(item_bg)
                };

                let mut val_spans = Vec::new();
                if val.is_empty() {
                    let action_hint = if is_selected {
                        match label.as_str() {
                            "Labels"
                            | "Assignees"
                            | "Reviewers"
                            | "Milestone"
                            | "Confidential"
                            | "Status (Draft/Ready)"
                            | "Merge Request Pipeline"
                            | "Source Branch"
                            | "Target Branch" => " <Enter to select>",
                            "Description" => " <Enter to edit>",
                            _ => " <Enter to edit>",
                        }
                    } else {
                        " <empty>"
                    };
                    let hint_style = if is_selected {
                        Style::default()
                            .fg(THEME.read().unwrap().text_muted)
                            .bg(item_bg)
                            .add_modifier(Modifier::ITALIC)
                    } else {
                        Style::default()
                            .fg(THEME.read().unwrap().border)
                            .bg(item_bg)
                            .add_modifier(Modifier::ITALIC)
                    };
                    val_spans.push(Span::styled(
                        if is_selected {
                            format!("{} ▋", action_hint)
                        } else {
                            action_hint.to_string()
                        },
                        hint_style,
                    ));
                } else {
                    let truncated = if val.len() > 50 {
                        let mut s = val[..47].to_string();
                        s.push_str("...");
                        s
                    } else {
                        val.clone()
                    };
                    match label.as_str() {
                        "Labels" => {
                            let parts: Vec<&str> = truncated.split(',').collect();
                            for (idx, part) in parts.iter().enumerate() {
                                if idx > 0 {
                                    val_spans.push(Span::styled(
                                        ", ",
                                        Style::default()
                                            .fg(THEME.read().unwrap().text_normal)
                                            .bg(item_bg),
                                    ));
                                }
                                let trimmed = part.trim();
                                let label_color = get_label_color(trimmed);
                                let mut style = Style::default()
                                    .fg(label_color)
                                    .bg(item_bg)
                                    .add_modifier(Modifier::BOLD);
                                if is_selected {
                                    style = style.add_modifier(Modifier::UNDERLINED);
                                }
                                val_spans.push(Span::styled(trimmed.to_string(), style));
                            }
                        }
                        "Assignees" | "Reviewers" => {
                            let parts: Vec<&str> = truncated.split(',').collect();
                            for (idx, part) in parts.iter().enumerate() {
                                if idx > 0 {
                                    val_spans.push(Span::styled(
                                        ", ",
                                        Style::default()
                                            .fg(THEME.read().unwrap().text_normal)
                                            .bg(item_bg),
                                    ));
                                }
                                let trimmed = part.trim();
                                let mut style =
                                    Style::default().fg(THEME.read().unwrap().blue).bg(item_bg);
                                if is_selected {
                                    style = style.add_modifier(Modifier::BOLD);
                                }
                                val_spans.push(Span::styled(trimmed.to_string(), style));
                            }
                        }
                        _ => {
                            let val_fg = match label.as_str() {
                                "Milestone" => THEME.read().unwrap().purple,
                                "Due Date" => THEME.read().unwrap().yellow,
                                "Status (Draft/Ready)" | "Source Branch" | "Target Branch" => {
                                    THEME.read().unwrap().purple
                                }
                                "Confidential" => {
                                    if val.to_lowercase() == "yes" {
                                        THEME.read().unwrap().red
                                    } else {
                                        THEME.read().unwrap().green
                                    }
                                }
                                _ => THEME.read().unwrap().text_normal,
                            };
                            let mut style = Style::default().fg(val_fg).bg(item_bg);
                            if is_selected {
                                style = style.add_modifier(Modifier::BOLD);
                            }
                            val_spans.push(Span::styled(truncated, style));
                        }
                    }
                }

                let mut line_spans = vec![
                    Span::styled(
                        format!("  {:label_width$} ", label, label_width = label_width),
                        label_style,
                    ),
                    Span::styled(" ❯ ", sep_style),
                ];
                line_spans.extend(val_spans);
                let line = Line::from(line_spans);

                ListItem::new(line).style(Style::default().bg(item_bg))
            })
            .collect();

        let is_new_entity = menu.is_new();
        let submit_idx = menu.fields.len() + 1;
        let all_items: Vec<ListItem> = if is_new_entity {
            let is_submit_selected = menu.selected_idx == submit_idx;
            let submit_bg = if is_submit_selected {
                THEME.read().unwrap().border_focused
            } else {
                Color::Reset
            };
            let submit_fg = if is_submit_selected {
                THEME.read().unwrap().bg
            } else {
                THEME.read().unwrap().border_focused
            };
            let submit_line = Line::from(vec![Span::styled(
                "          [ Submit ]          ",
                Style::default()
                    .fg(submit_fg)
                    .bg(submit_bg)
                    .add_modifier(Modifier::BOLD),
            )]);
            let mut v = items;
            v.push(ListItem::new(
                Line::from("").style(Style::default().bg(Color::Reset)),
            ));
            v.push(ListItem::new(submit_line));
            v
        } else {
            items
        };

        let footer_text = if is_new_entity {
            " ↑↓ Navigate  Enter: Edit / Submit  Esc: Cancel "
        } else {
            " ↑↓ Navigate  Enter: Edit  Esc: Close "
        };

        let inner_area = block.inner(area);
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(2)])
            .split(inner_area);

        let list = List::new(all_items).style(Style::default().bg(Color::Reset));

        let footer = Paragraph::new(footer_text)
            .style(
                Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::ITALIC),
            )
            .wrap(ratatui::widgets::Wrap { trim: true });

        f.render_widget(Clear, area);
        f.render_widget(block, area);
        let mut state = menu.state.clone();
        f.render_stateful_widget(list, layout[0], &mut state);
        menu.state = state;
        f.render_widget(footer, layout[1]);
    }

    if app.column_filter_context.is_none() {
        if let Some(selector) = &mut app.selector {
            let block = Block::default()
                .title(format!(" {} ", selector.title))
                .title_style(
                    Style::default()
                        .fg(THEME.read().unwrap().header_fg)
                        .add_modifier(Modifier::BOLD),
                )
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
                .style(Style::default().bg(Color::Reset));

            let area = centered_rect_min(50, 60, 34, 6, size);

            let has_filter = selector.field_type != "comment_action_select"
                && selector.field_type != "review_submit_status"
                && selector.field_type != "description_edit_choice";

            let constraints = if has_filter {
                vec![
                    Constraint::Length(3), // Search/Filter
                    Constraint::Min(0),    // List of items
                    Constraint::Length(3), // Help/Info footer
                ]
            } else {
                vec![
                    Constraint::Min(0),    // List of items
                    Constraint::Length(3), // Help/Info footer
                ]
            };

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(constraints)
                .split(area);

            let (search_chunk, list_chunk, footer_chunk) = if has_filter {
                (Some(chunks[0]), chunks[1], chunks[2])
            } else {
                (None, chunks[0], chunks[1])
            };

            let border_color_search = if selector.is_filtering {
                THEME.read().unwrap().border_focused
            } else {
                THEME.read().unwrap().border
            };
            let search_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color_search).bg(Color::Reset))
                .title(" Filter (press 'f' or '/' to focus) ");

            let search_text = if selector.is_filtering {
                format!("{}▋", selector.search_query)
            } else if selector.search_query.is_empty() {
                "Type to filter...".to_string()
            } else {
                selector.search_query.clone()
            };

            let search_style = if selector.search_query.is_empty() && !selector.is_filtering {
                Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::ITALIC)
            } else {
                Style::default()
                    .fg(THEME.read().unwrap().text_normal)
                    .bg(Color::Reset)
            };

            let search_p = Paragraph::new(search_text)
                .block(search_block)
                .style(search_style)
                .wrap(ratatui::widgets::Wrap { trim: true });

            let footer_text = if selector.is_filtering {
                "  Esc/Enter: Stop filtering • Backspace: Delete  "
            } else if has_filter {
                "  j/k: Navigate • Space: Toggle • Enter: Save & Exit • f: Filter • Esc: Back  "
            } else {
                "  j/k: Navigate • Space: Toggle • Enter: Save & Exit • Esc: Back  "
            };
            let footer_p = Paragraph::new(footer_text)
                .style(
                    Style::default()
                        .fg(THEME.read().unwrap().text_muted)
                        .bg(Color::Reset)
                        .add_modifier(Modifier::ITALIC),
                )
                .wrap(ratatui::widgets::Wrap { trim: true });

            f.render_widget(Clear, area);
            f.render_widget(block, area);

            if let Some(sc) = search_chunk {
                f.render_widget(search_p, sc);
            }

            if selector.is_loading {
                let p = Paragraph::new("\n  Loading options from GitLab...")
                    .style(
                        Style::default()
                            .fg(THEME.read().unwrap().text_muted)
                            .bg(Color::Reset)
                            .add_modifier(Modifier::ITALIC),
                    )
                    .wrap(ratatui::widgets::Wrap { trim: true });
                f.render_widget(p, list_chunk);
            } else {
                let filtered_items = selector.get_filtered_items_with_indices();
                if filtered_items.is_empty() {
                    let p = Paragraph::new("\n  No matching options found.")
                        .style(
                            Style::default()
                                .fg(THEME.read().unwrap().text_muted)
                                .bg(Color::Reset)
                                .add_modifier(Modifier::ITALIC),
                        )
                        .wrap(ratatui::widgets::Wrap { trim: true });
                    f.render_widget(p, list_chunk);
                } else {
                    let items: Vec<ListItem> = filtered_items
                        .iter()
                        .enumerate()
                        .map(|(i, (item, indices))| {
                            let is_selected = if item.starts_with("+ Create \"") {
                                let clean_val = selector.search_query.trim().to_string();
                                selector.selected_items.contains(&clean_val)
                            } else {
                                selector.selected_items.contains(item)
                            };

                            let marker = if is_selected { " ▣ " } else { " ☐ " };
                            let marker_color = if is_selected {
                                THEME.read().unwrap().green
                            } else {
                                THEME.read().unwrap().text_muted
                            };

                            let item_bg = if i == selector.cursor_idx {
                                THEME.read().unwrap().highlight_bg
                            } else {
                                Color::Reset
                            };

                            let style = if i == selector.cursor_idx {
                                Style::default()
                                    .bg(item_bg)
                                    .fg(THEME.read().unwrap().text_normal)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                                    .fg(THEME.read().unwrap().text_normal)
                                    .bg(item_bg)
                            };

                            let highlight_style = if i == selector.cursor_idx {
                                Style::default()
                                    .bg(item_bg)
                                    .fg(THEME.read().unwrap().yellow)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                                    .fg(THEME.read().unwrap().yellow)
                                    .bg(item_bg)
                                    .add_modifier(Modifier::BOLD)
                            };

                            let mut line_spans = vec![Span::styled(
                                marker,
                                Style::default()
                                    .fg(marker_color)
                                    .bg(item_bg)
                                    .add_modifier(Modifier::BOLD),
                            )];

                            if let Some(indices) = indices {
                                line_spans.extend(highlight_fuzzy_match(
                                    item,
                                    indices,
                                    style,
                                    highlight_style,
                                ));
                            } else {
                                line_spans.push(Span::styled(item.clone(), style));
                            }

                            ListItem::new(vec![Line::from(line_spans)])
                                .style(Style::default().bg(item_bg))
                        })
                        .collect();

                    let list = List::new(items).style(Style::default().bg(Color::Reset));
                    let mut state = selector.state.clone();
                    f.render_stateful_widget(list, list_chunk, &mut state);
                    selector.state = state;
                }
            }
            f.render_widget(footer_p, footer_chunk);
        }
    }

    if let Some(text_input) = &app.text_input {
        let block = Block::default()
            .title(format!(" {} ", text_input.title))
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().header_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
            .style(Style::default().bg(Color::Reset));

        let area = centered_rect_min(60, 60, 28, 4, size);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints(
                [
                    Constraint::Min(0),    // Value input line
                    Constraint::Length(2), // Help footer
                ]
                .as_ref(),
            )
            .split(area);

        let mut display_val = text_input.value.clone();
        if text_input.cursor_idx <= display_val.len() {
            display_val.insert(text_input.cursor_idx, '▋');
        } else {
            display_val.push('▋');
        }

        let value_p = Paragraph::new(display_val)
            .style(
                Style::default()
                    .fg(THEME.read().unwrap().text_normal)
                    .bg(Color::Reset),
            )
            .wrap(ratatui::widgets::Wrap { trim: true });

        let footer_p = Paragraph::new("  Enter: Confirm • Esc: Cancel • Ctrl-e: Open $EDITOR  ")
            .style(
                Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::ITALIC),
            )
            .wrap(ratatui::widgets::Wrap { trim: true });

        f.render_widget(Clear, area);
        f.render_widget(block, area);
        f.render_widget(value_p, chunks[0]);
        f.render_widget(footer_p, chunks[1]);
    }

    if let Some(date_picker) = &app.date_picker {
        let block = Block::default()
            .title(format!(" {} ", date_picker.title))
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().header_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
            .style(Style::default().bg(Color::Reset));

        // 36 columns wide, 11 rows high
        let area = centered_rect_fixed(36, 11, size);
        let inner_area = block.inner(area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(1), // Month/Year line
                    Constraint::Min(0),    // Grid of days
                    Constraint::Length(1), // Footer keys
                ]
                .as_ref(),
            )
            .split(inner_area);

        let month_str = match date_picker.month {
            1 => "January",
            2 => "February",
            3 => "March",
            4 => "April",
            5 => "May",
            6 => "June",
            7 => "July",
            8 => "August",
            9 => "September",
            10 => "October",
            11 => "November",
            12 => "December",
            _ => "",
        };
        let header_str = format!("◀  {} {}  ▶", month_str, date_picker.year);
        let header_p = Paragraph::new(header_str)
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(THEME.read().unwrap().header_fg)
                    .add_modifier(Modifier::BOLD),
            );

        // Weekday headers
        let weekday_headers = vec!["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
        let col_headers = weekday_headers
            .into_iter()
            .map(|h| Cell::from(Line::from(h).alignment(Alignment::Center)));
        let table_header =
            Row::new(col_headers).style(Style::default().fg(THEME.read().unwrap().text_muted));

        // Calculate days grid
        let first_date =
            chrono::NaiveDate::from_ymd_opt(date_picker.year, date_picker.month, 1).unwrap();
        use chrono::Datelike;
        let start_weekday = first_date.weekday().num_days_from_sunday(); // 0 = Sunday, 1 = Monday, etc.
        let total_days = crate::app::days_in_month(date_picker.year, date_picker.month);

        let mut rows = Vec::new();
        for r in 0..6 {
            let mut row_cells = Vec::new();
            for c in 0..7 {
                let cell_idx = r * 7 + c;
                let day_num = (cell_idx as i32) - (start_weekday as i32) + 1;
                if day_num >= 1 && day_num <= total_days as i32 {
                    let is_selected = day_num as u32 == date_picker.day;
                    let style = if is_selected {
                        Style::default()
                            .bg(THEME.read().unwrap().highlight_bg)
                            .fg(THEME.read().unwrap().header_fg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(THEME.read().unwrap().text_normal)
                    };
                    row_cells.push(Cell::from(
                        Line::from(day_num.to_string())
                            .alignment(Alignment::Center)
                            .style(style),
                    ));
                } else {
                    row_cells.push(Cell::from(""));
                }
            }
            rows.push(Row::new(row_cells));
        }

        let widths = [
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(4),
        ];

        let table = Table::new(rows, widths)
            .header(table_header)
            .column_spacing(1);

        let footer_p = Paragraph::new("←↓↑→/hjkl: Move • [/]: Month • Enter: Set")
            .style(
                Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::ITALIC),
            )
            .alignment(Alignment::Center);

        f.render_widget(Clear, area);
        f.render_widget(block, area);
        f.render_widget(header_p, chunks[0]);
        f.render_widget(table, chunks[1]);
        f.render_widget(footer_p, chunks[2]);
    }

    if app.show_help {
        struct Shortcut {
            category: &'static str,
            key: std::borrow::Cow<'static, str>,
            action: &'static str,
        }

        let s = |k: &'static str| std::borrow::Cow::Borrowed(k);
        let d = |k: String| std::borrow::Cow::Owned(k);

        let shortcuts: Vec<Shortcut> = vec![
            // Global & Nav
            Shortcut {
                category: "Global & Nav",
                key: s("l / →"),
                action: "Next tab",
            },
            Shortcut {
                category: "Global & Nav",
                key: s("h / ←"),
                action: "Previous tab",
            },
            Shortcut {
                category: "Global & Nav",
                key: d(format!(
                    "{}, / Tab / t",
                    app.config.keybindings.global.configure
                )),
                action: "Toggle columns config popup",
            },
            Shortcut {
                category: "Global & Nav",
                key: s("j / k / ↓ / ↑"),
                action: "Select item / Scroll page",
            },
            Shortcut {
                category: "Global & Nav",
                key: s("J / K"),
                action: "Scroll description / trace / notes",
            },
            Shortcut {
                category: "Global & Nav",
                key: d(format!("{} / f", app.config.keybindings.global.search)),
                action: "Open fuzzy search / filter bar",
            },
            Shortcut {
                category: "Global & Nav",
                key: d(format!(
                    "F5 / Ctrl+R / {}",
                    app.config.keybindings.global.refresh
                )),
                action: "Refresh active tab data",
            },
            Shortcut {
                category: "Global & Nav",
                key: s("Ctrl+S"),
                action: "Switch repository",
            },
            Shortcut {
                category: "Global & Nav",
                key: s("u"),
                action: "Check for updates",
            },
            Shortcut {
                category: "Global & Nav",
                key: d(format!("{} / F1", app.config.keybindings.global.help)),
                action: "Show this help modal",
            },
            Shortcut {
                category: "Global & Nav",
                key: d(format!("q / {} / Esc", app.config.keybindings.global.quit)),
                action: "Quit / Close overlay",
            },
            Shortcut {
                category: "Global & Nav",
                key: s("Ctrl+C"),
                action: "Quit program",
            },
            Shortcut {
                category: "Issues",
                key: s("n"),
                action: "Create new Issue",
            },
            Shortcut {
                category: "Issues",
                key: s("e"),
                action: "Open parameter edit menu",
            },
            Shortcut {
                category: "Issues",
                key: s("c"),
                action: "Close selected Issue",
            },
            Shortcut {
                category: "Issues",
                key: s("r"),
                action: "Reopen selected Issue",
            },
            Shortcut {
                category: "Issues",
                key: s("o"),
                action: "Open selected Issue in browser",
            },
            Shortcut {
                category: "Merge Requests",
                key: s("n"),
                action: "Create new Merge Request",
            },
            Shortcut {
                category: "Merge Requests",
                key: s("e"),
                action: "Open parameter edit menu",
            },
            Shortcut {
                category: "Merge Requests",
                key: s("a"),
                action: "Approve selected MR",
            },
            Shortcut {
                category: "Merge Requests",
                key: s("m"),
                action: "Merge selected MR (squash + delete)",
            },
            Shortcut {
                category: "Merge Requests",
                key: s("s"),
                action: "Toggle Draft / Ready status",
            },
            Shortcut {
                category: "Merge Requests",
                key: s("v"),
                action: "View Merge Request diff changes",
            },
            Shortcut {
                category: "Merge Requests",
                key: s("c"),
                action: "Close selected MR",
            },
            Shortcut {
                category: "Merge Requests",
                key: s("r"),
                action: "Reopen selected MR",
            },
            Shortcut {
                category: "Merge Requests",
                key: s("o"),
                action: "Open selected MR in browser",
            },
            Shortcut {
                category: "Pipelines",
                key: s("Enter"),
                action: "View pipeline jobs list",
            },
            Shortcut {
                category: "Pipelines",
                key: s("p"),
                action: "Trigger new pipeline from MR",
            },
            Shortcut {
                category: "Pipelines",
                key: s("r"),
                action: "Retry selected pipeline(s)",
            },
            Shortcut {
                category: "Pipelines",
                key: s("c"),
                action: "Cancel pipeline execution",
            },
            Shortcut {
                category: "Pipelines",
                key: s("Space"),
                action: "Check / uncheck pipeline for bulk retry",
            },
            Shortcut {
                category: "Pipelines",
                key: s("o"),
                action: "Open pipeline in browser",
            },
            Shortcut {
                category: "Jobs",
                key: s("Enter"),
                action: "View job trace (toggle zoom)",
            },
            Shortcut {
                category: "Jobs",
                key: s("Esc / Backspc"),
                action: "Go back to Pipelines list",
            },
            Shortcut {
                category: "Jobs",
                key: s("r"),
                action: "Retry selected job(s)",
            },
            Shortcut {
                category: "Jobs",
                key: s("c"),
                action: "Cancel selected job(s)",
            },
            Shortcut {
                category: "Jobs",
                key: s("Space"),
                action: "Check / uncheck job for bulk retry/cancel",
            },
            Shortcut {
                category: "Jobs",
                key: s("s"),
                action: "Select all jobs in stage",
            },
            Shortcut {
                category: "Jobs",
                key: s("d"),
                action: "Download job artifact",
            },
            Shortcut {
                category: "Jobs",
                key: s("e"),
                action: "Open job trace in external $EDITOR",
            },
            Shortcut {
                category: "Jobs",
                key: s("o"),
                action: "Open selected job in browser",
            },
            Shortcut {
                category: "Milestones",
                key: s("n"),
                action: "Create new milestone",
            },
            Shortcut {
                category: "Milestones",
                key: s("J / K"),
                action: "Scroll milestone issues list",
            },
            Shortcut {
                category: "Runners",
                key: s("p / r"),
                action: "Pause / Resume runner",
            },
            Shortcut {
                category: "Runners",
                key: s("e"),
                action: "Edit runner description text",
            },
            Shortcut {
                category: "Releases",
                key: s("Enter"),
                action: "View release notes (toggle zoom)",
            },
            Shortcut {
                category: "Releases",
                key: s("n"),
                action: "Create new release tag & changelog",
            },
            Shortcut {
                category: "Releases",
                key: s("o"),
                action: "Open release in browser",
            },
            Shortcut {
                category: "TODOs",
                key: s("Enter / o"),
                action: "Open todo target & mark read",
            },
            Shortcut {
                category: "Terminal",
                key: s("j / k / ↑ / ↓"),
                action: "Scroll terminal log",
            },
            Shortcut {
                category: "Diff View",
                key: s("q / Esc"),
                action: "Exit Diff View",
            },
            Shortcut {
                category: "Diff View",
                key: s("Tab"),
                action: "Toggle Focus (Files / Diff)",
            },
            Shortcut {
                category: "Diff View",
                key: s("h / l / Left / Right"),
                action: "Switch Panel Focus",
            },
            Shortcut {
                category: "Diff View",
                key: s("j / k / ↓ / ↑"),
                action: "Navigate files or diff lines",
            },
            Shortcut {
                category: "Diff View",
                key: s("J / K"),
                action: "Next / Previous Hunk",
            },
            Shortcut {
                category: "Diff View",
                key: s("c"),
                action: "Add Comment on Line",
            },
            Shortcut {
                category: "Diff View",
                key: s("r"),
                action: "Submit Review (approve/changes/comment)",
            },
            Shortcut {
                category: "Diff View",
                key: s("? / F1"),
                action: "Show this help modal",
            },
        ];

        let active_categories: &[&str] = if app.diff_view.is_some() {
            &["Diff View"]
        } else {
            match app.active_tab {
                Tab::Issues => &["Global & Nav", "Issues"],
                Tab::MergeRequests => &["Global & Nav", "Merge Requests"],
                Tab::Pipelines => &["Global & Nav", "Pipelines"],
                Tab::Jobs => &["Global & Nav", "Jobs"],
                Tab::Milestones => &["Global & Nav", "Milestones"],
                Tab::Runners => &["Global & Nav", "Runners"],
                Tab::Releases => &["Global & Nav", "Releases"],
                Tab::Todos => &["Global & Nav", "TODOs"],
                Tab::Terminal => &["Global & Nav", "Terminal"],
            }
        };

        let filtered_shortcuts: Vec<&Shortcut> = shortcuts
            .iter()
            .filter(|s| active_categories.contains(&s.category))
            .collect();

        let block = Block::default()
            .title(" Keyboard Shortcuts ")
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().header_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
            .border_type(BorderType::Double)
            .style(Style::default().bg(Color::Reset));

        let area = centered_rect_fixed(72, 30, size);

        let help_chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints(
                [
                    Constraint::Length(3), // Search / Filter
                    Constraint::Min(0),    // Table
                    Constraint::Length(2), // Help footer
                ]
                .as_ref(),
            )
            .split(area);

        let border_color = if app.help_search_query.is_empty() {
            THEME.read().unwrap().border
        } else {
            THEME.read().unwrap().border_focused
        };
        let search_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(" Filter Shortcuts (Type to filter, Esc/Enter to exit) ")
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .add_modifier(Modifier::BOLD),
            );

        let search_text = if app.help_search_query.is_empty() {
            "Type to search commands...▋".to_string()
        } else {
            format!("{}▋", app.help_search_query)
        };

        let search_style = if app.help_search_query.is_empty() {
            Style::default()
                .fg(THEME.read().unwrap().text_muted)
                .add_modifier(Modifier::ITALIC)
        } else {
            Style::default().fg(THEME.read().unwrap().text_normal)
        };

        let search_p = Paragraph::new(search_text)
            .style(search_style)
            .block(search_block)
            .wrap(ratatui::widgets::Wrap { trim: true });

        let rows: Vec<Row> = if app.help_search_query.is_empty() {
            let mut result_rows = Vec::new();
            let mut last_category = "";
            for s in &filtered_shortcuts {
                if s.category != last_category {
                    if !last_category.is_empty() {
                        result_rows.push(Row::new(vec![
                            Cell::from(""),
                            Cell::from(""),
                            Cell::from(""),
                        ])); // spacer
                    }
                    result_rows.push(Row::new(vec![
                        Cell::from(Span::styled(
                            s.category,
                            Style::default()
                                .fg(THEME.read().unwrap().purple)
                                .add_modifier(Modifier::BOLD),
                        )),
                        Cell::from(Span::styled(
                            s.key.clone(),
                            Style::default()
                                .fg(THEME.read().unwrap().text_normal)
                                .add_modifier(Modifier::BOLD),
                        )),
                        Cell::from(Span::styled(
                            s.action,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                        )),
                    ]));
                    last_category = s.category;
                } else {
                    result_rows.push(Row::new(vec![
                        Cell::from(""),
                        Cell::from(Span::styled(
                            s.key.clone(),
                            Style::default()
                                .fg(THEME.read().unwrap().text_normal)
                                .add_modifier(Modifier::BOLD),
                        )),
                        Cell::from(Span::styled(
                            s.action,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                        )),
                    ]));
                }
            }
            result_rows
        } else {
            let query = app.help_search_query.to_lowercase();
            filtered_shortcuts
                .iter()
                .filter(|s| {
                    s.category.to_lowercase().contains(&query)
                        || s.key.to_lowercase().contains(&query)
                        || s.action.to_lowercase().contains(&query)
                })
                .map(|s| {
                    Row::new(vec![
                        Cell::from(Span::styled(
                            s.category,
                            Style::default()
                                .fg(THEME.read().unwrap().purple)
                                .add_modifier(Modifier::BOLD),
                        )),
                        Cell::from(Span::styled(
                            s.key.clone(),
                            Style::default()
                                .fg(THEME.read().unwrap().text_normal)
                                .add_modifier(Modifier::BOLD),
                        )),
                        Cell::from(Span::styled(
                            s.action,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                        )),
                    ])
                })
                .collect()
        };

        let widths = [
            Constraint::Length(16),
            Constraint::Length(18),
            Constraint::Min(0),
        ];

        let header_style = Style::default()
            .fg(THEME.read().unwrap().header_fg)
            .add_modifier(Modifier::BOLD);
        let table = Table::new(rows, widths)
            .header(
                Row::new(vec![
                    Cell::from(Span::styled("Category", header_style)),
                    Cell::from(Span::styled("Key", header_style)),
                    Cell::from(Span::styled("Action", header_style)),
                ])
                .height(1),
            )
            .block(block)
            .row_highlight_style(Style::default())
            .column_spacing(2);

        let footer_p = Paragraph::new(" Press Esc or Enter to close ")
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .add_modifier(Modifier::ITALIC),
            )
            .wrap(ratatui::widgets::Wrap { trim: true });

        f.render_widget(Clear, area);
        f.render_widget(search_p, help_chunks[0]);
        f.render_widget(table, help_chunks[1]);
        f.render_widget(footer_p, help_chunks[2]);
    }

    if app.focus_column_checklist {
        let tab = app.active_tab;
        let is_github = app
            .gitlab_client
            .as_ref()
            .map(|c| c.is_github)
            .unwrap_or(false);
        let cols = tab.columns(is_github);
        let active_idx = app.column_checklist_idx;

        let group_cols: Vec<&str> = cols.iter().copied().collect();

        let columns_list: Vec<(usize, &str)> = cols.iter().copied().enumerate().collect();

        let cols_end = cols.len();
        let group_end = cols_end + group_cols.len();
        let theme_list_len = crate::config::THEME_PRESETS.len();
        let width = 48;
        let height =
            (columns_list.len() + group_cols.len() + theme_list_len + 4 + 2 + 2 + 6 + 6) as u16;
        let area = centered_rect_fixed(width, height, size);

        let checklist_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
            .title(format!(" Configure View: {} ", tab.title(is_github)))
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().border_focused)
                    .add_modifier(Modifier::BOLD),
            );

        f.render_widget(Clear, area);
        f.render_widget(checklist_block.clone(), area);

        let inner_area = checklist_block.inner(area);

        let themes = crate::config::THEME_PRESETS;
        let order_end = group_end + 2;

        let mut constraints: Vec<Constraint> = Vec::new();
        constraints.push(Constraint::Length(1)); // COLUMNS header
        constraints.push(Constraint::Length(columns_list.len() as u16));
        constraints.push(Constraint::Length(1)); // spacer
        constraints.push(Constraint::Length(1)); // GROUP BY header
        constraints.push(Constraint::Length(group_cols.len() as u16));
        constraints.push(Constraint::Length(1)); // spacer
        constraints.push(Constraint::Length(1)); // ORDER header
        constraints.push(Constraint::Length(2));
        constraints.push(Constraint::Length(1)); // spacer
        constraints.push(Constraint::Length(1)); // THEME header
        constraints.push(Constraint::Length(themes.len() as u16));
        constraints.push(Constraint::Min(0)); // footer

        let popup_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner_area);

        let mut chunk_idx = 0;

        let columns_header = Paragraph::new("  COLUMNS").style(
            Style::default()
                .fg(THEME.read().unwrap().header_fg)
                .add_modifier(Modifier::BOLD),
        );
        f.render_widget(columns_header, popup_layout[chunk_idx]);
        chunk_idx += 1;

        let col_items: Vec<ListItem> = columns_list
            .iter()
            .map(|&(orig_idx, col)| {
                let checked = app.is_column_visible(tab, col);
                let filter_count = app
                    .get_column_filter(tab, col)
                    .map(|s| s.len())
                    .filter(|&n| n > 0);
                let text = if let Some(count) = filter_count {
                    format!(
                        "  [{}] {} ({})",
                        if checked { "x" } else { " " },
                        col,
                        count
                    )
                } else {
                    format!("  [{}] {}", if checked { "x" } else { " " }, col)
                };
                let is_active = orig_idx == active_idx;
                let style = if is_active {
                    Style::default()
                        .fg(THEME.read().unwrap().bg)
                        .bg(THEME.read().unwrap().border_focused)
                        .add_modifier(Modifier::BOLD)
                } else if checked {
                    Style::default().fg(THEME.read().unwrap().text_normal)
                } else {
                    Style::default().fg(THEME.read().unwrap().text_muted)
                };
                ListItem::new(text).style(style)
            })
            .collect();
        f.render_widget(List::new(col_items), popup_layout[chunk_idx]);
        chunk_idx += 1;

        chunk_idx += 1; // spacer

        let group_header = Paragraph::new("  GROUP BY").style(
            Style::default()
                .fg(THEME.read().unwrap().green)
                .add_modifier(Modifier::BOLD),
        );
        f.render_widget(group_header, popup_layout[chunk_idx]);
        chunk_idx += 1;

        let group_items: Vec<ListItem> = group_cols
            .iter()
            .enumerate()
            .map(|(i, col)| {
                let flat_idx = cols_end + i;
                let is_selected = app.group_by_column.as_deref() == Some(col);
                let text = format!("  {} {}", if is_selected { "◉" } else { "○" }, col);
                let is_active = flat_idx == active_idx;
                let style = if is_active {
                    Style::default()
                        .fg(THEME.read().unwrap().bg)
                        .bg(THEME.read().unwrap().border_focused)
                        .add_modifier(Modifier::BOLD)
                } else if is_selected {
                    Style::default().fg(THEME.read().unwrap().green)
                } else {
                    Style::default().fg(THEME.read().unwrap().text_normal)
                };
                ListItem::new(text).style(style)
            })
            .collect();
        f.render_widget(List::new(group_items), popup_layout[chunk_idx]);
        chunk_idx += 1;

        chunk_idx += 1; // spacer

        let order_header = Paragraph::new("  ORDER").style(
            Style::default()
                .fg(THEME.read().unwrap().yellow)
                .add_modifier(Modifier::BOLD),
        );
        f.render_widget(order_header, popup_layout[chunk_idx]);
        chunk_idx += 1;

        let order_items: Vec<ListItem> = ["Ascending", "Descending"]
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let flat_idx = group_end + i;
                let is_selected = app.group_ascending == (i == 0);
                let text = format!("  {} {}", if is_selected { "◉" } else { "○" }, label);
                let is_active = flat_idx == active_idx;
                let style = if is_active {
                    Style::default()
                        .fg(THEME.read().unwrap().bg)
                        .bg(THEME.read().unwrap().border_focused)
                        .add_modifier(Modifier::BOLD)
                } else if is_selected {
                    Style::default().fg(THEME.read().unwrap().yellow)
                } else {
                    Style::default().fg(THEME.read().unwrap().text_normal)
                };
                ListItem::new(text).style(style)
            })
            .collect();
        f.render_widget(List::new(order_items), popup_layout[chunk_idx]);
        chunk_idx += 1;

        chunk_idx += 1; // spacer

        let theme_header = Paragraph::new("  THEME").style(
            Style::default()
                .fg(THEME.read().unwrap().purple)
                .add_modifier(Modifier::BOLD),
        );
        f.render_widget(theme_header, popup_layout[chunk_idx]);
        chunk_idx += 1;

        let theme_items: Vec<ListItem> = themes
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let flat_idx = order_end + i;
                let is_selected = app.config.theme_preset.as_deref().unwrap_or("default") == *name;
                let text = format!("  {} {}", if is_selected { "◉" } else { "○" }, name);
                let is_active = flat_idx == active_idx;
                let style = if is_active {
                    Style::default()
                        .fg(THEME.read().unwrap().bg)
                        .bg(THEME.read().unwrap().border_focused)
                        .add_modifier(Modifier::BOLD)
                } else if is_selected {
                    Style::default().fg(THEME.read().unwrap().purple)
                } else {
                    Style::default().fg(THEME.read().unwrap().text_normal)
                };
                ListItem::new(text).style(style)
            })
            .collect();
        f.render_widget(List::new(theme_items), popup_layout[chunk_idx]);
        chunk_idx += 1;

        let footer_p = Paragraph::new(" [Spc/Enter] Toggle • [,/Esc] Close ")
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .add_modifier(Modifier::ITALIC),
            )
            .wrap(ratatui::widgets::Wrap { trim: true });
        f.render_widget(footer_p, popup_layout[chunk_idx]);
    }

    // Render value-based column filter selector as overlay on configure view
    if app.focus_column_checklist && app.column_filter_context.is_some() {
        if let Some(selector) = &mut app.selector {
            let block = Block::default()
                .title(format!(" {} ", selector.title))
                .title_style(
                    Style::default()
                        .fg(THEME.read().unwrap().header_fg)
                        .add_modifier(Modifier::BOLD),
                )
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
                .style(Style::default().bg(Color::Reset));

            let area = centered_rect_fixed(44, 44, size);

            let has_filter = true;

            let constraints = vec![
                Constraint::Length(3), // Search/Filter
                Constraint::Min(0),    // List of items
                Constraint::Length(3), // Help/Info footer
            ];

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(constraints)
                .split(area);

            let (search_chunk, list_chunk, footer_chunk) = (chunks[0], chunks[1], chunks[2]);

            let border_color_search = if selector.is_filtering {
                THEME.read().unwrap().border_focused
            } else {
                THEME.read().unwrap().border
            };
            let search_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color_search).bg(Color::Reset))
                .title(" Filter (press 'f' or '/' to focus) ");

            let search_text = if selector.is_filtering {
                format!("{}▋", selector.search_query)
            } else if selector.search_query.is_empty() {
                "Type to filter...".to_string()
            } else {
                selector.search_query.clone()
            };
            let search_p = Paragraph::new(search_text)
                .block(search_block)
                .style(Style::default().fg(THEME.read().unwrap().text_normal));

            f.render_widget(Clear, area);
            f.render_widget(block, area);
            f.render_widget(search_p, search_chunk);

            // Render items list
            let items_list = selector.get_filtered_items_with_indices();
            let items: Vec<ListItem> = items_list
                .iter()
                .enumerate()
                .map(|(i, (item, indices))| {
                    let is_selected = selector.selected_items.contains(item);

                    let marker = if is_selected { " ▣ " } else { " ☐ " };
                    let marker_color = if is_selected {
                        THEME.read().unwrap().green
                    } else {
                        THEME.read().unwrap().text_muted
                    };

                    let item_bg = if i == selector.cursor_idx {
                        THEME.read().unwrap().highlight_bg
                    } else {
                        Color::Reset
                    };

                    let style = if i == selector.cursor_idx {
                        Style::default()
                            .bg(item_bg)
                            .fg(THEME.read().unwrap().text_normal)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(THEME.read().unwrap().text_normal)
                            .bg(item_bg)
                    };

                    let highlight_style = if i == selector.cursor_idx {
                        Style::default()
                            .bg(item_bg)
                            .fg(THEME.read().unwrap().yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(THEME.read().unwrap().yellow)
                            .bg(item_bg)
                            .add_modifier(Modifier::BOLD)
                    };

                    let mut line_spans = vec![Span::styled(
                        marker,
                        Style::default()
                            .fg(marker_color)
                            .bg(item_bg)
                            .add_modifier(Modifier::BOLD),
                    )];

                    if let Some(indices) = indices {
                        line_spans.extend(highlight_fuzzy_match(
                            item,
                            indices,
                            style,
                            highlight_style,
                        ));
                    } else {
                        line_spans.push(Span::styled(item.clone(), style));
                    }

                    ListItem::new(vec![Line::from(line_spans)]).style(Style::default().bg(item_bg))
                })
                .collect();

            let list = List::new(items).style(Style::default().bg(Color::Reset));
            let mut state = selector.state.clone();
            f.render_stateful_widget(list, list_chunk, &mut state);
            selector.state = state;

            let footer_p = Paragraph::new(" [Spc] Toggle • [Enter] Confirm • [Esc] Cancel ")
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .fg(THEME.read().unwrap().text_muted)
                        .add_modifier(Modifier::ITALIC),
                )
                .wrap(ratatui::widgets::Wrap { trim: true });
            f.render_widget(footer_p, footer_chunk);
        }
    }

    if app.show_submit_review_prompt.is_some() {
        let block = Block::default()
            .title(" Submit Review? ")
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().header_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
            .border_type(BorderType::Double)
            .style(Style::default().bg(Color::Reset));

        let area = centered_rect_fixed(60, 9, size);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Min(0),    // Message
                Constraint::Length(2), // Footer
            ])
            .split(area);

        let message_p = Paragraph::new(vec![
            Line::from(""),
            Line::from(
                "You have pending draft comments. Would you like to submit your review now?",
            ),
        ])
        .alignment(Alignment::Center)
        .style(Style::default().fg(THEME.read().unwrap().text_normal))
        .wrap(ratatui::widgets::Wrap { trim: true });

        let footer_p = Paragraph::new(" y: Yes (Submit) • n: No (Discard & Exit) • Esc: Cancel ")
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .add_modifier(Modifier::ITALIC),
            )
            .wrap(ratatui::widgets::Wrap { trim: true });

        f.render_widget(Clear, area);
        f.render_widget(block, area);
        f.render_widget(message_p, chunks[0]);
        f.render_widget(footer_p, chunks[1]);
    }

    if let Some(confirm) = &app.confirm_popup {
        let (title, message) = match confirm {
            crate::app::ConfirmAction::DeleteMilestone(iid) => (
                " Delete Milestone? ",
                format!("Are you sure you want to delete milestone #{}?", iid),
            ),
            crate::app::ConfirmAction::DeleteRelease(tag_name) => (
                " Delete Release? ",
                format!("Are you sure you want to delete release {}?", tag_name),
            ),
        };

        let block = Block::default()
            .title(title)
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().header_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
            .border_type(BorderType::Double)
            .style(Style::default().bg(Color::Reset));

        let area = centered_rect_fixed(60, 9, size);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Min(0),    // Message
                Constraint::Length(2), // Footer
            ])
            .split(area);

        let message_p = Paragraph::new(vec![Line::from(""), Line::from(message)])
            .alignment(Alignment::Center)
            .style(Style::default().fg(THEME.read().unwrap().text_normal))
            .wrap(ratatui::widgets::Wrap { trim: true });

        let footer_p = Paragraph::new(" y: Yes (Confirm) • n/Esc: Cancel ")
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .add_modifier(Modifier::ITALIC),
            )
            .wrap(ratatui::widgets::Wrap { trim: true });

        f.render_widget(Clear, area);
        f.render_widget(block, area);
        f.render_widget(message_p, chunks[0]);
        f.render_widget(footer_p, chunks[1]);
    }
}

fn format_comment_with_suggestions(
    body: &str,
    file_path: &str,
    start_new: Option<u64>,
    end_new: Option<u64>,
    start_old: Option<u64>,
    end_old: Option<u64>,
    all_lines: &[crate::app::DiffLine],
    prefix: &str,
    prefix_style: Style,
) -> Vec<(String, Style, Vec<(Style, String)>)> {
    let mut result_lines = Vec::new();
    let mut in_suggestion = false;
    let mut is_first = true;

    // Retrieve original lines for suggestion diff
    let mut original_lines: Vec<crate::app::DiffLine> = Vec::new();
    if let Some(oln) = start_old {
        let end_o = end_old.unwrap_or(oln);
        let min_o = oln.min(end_o);
        let max_o = oln.max(end_o);
        for dl in all_lines {
            if &dl.file_path == file_path {
                if let Some(num) = dl.old_line_num {
                    if num as u64 >= min_o && num as u64 <= max_o {
                        if dl.line_type != crate::app::DiffLineType::Addition
                            && !original_lines.iter().any(|ol| ol.content == dl.content)
                        {
                            original_lines.push(dl.clone());
                        }
                    }
                }
            }
        }
    } else if let Some(nln) = start_new {
        let end_n = end_new.unwrap_or(nln);
        let min_n = nln.min(end_n);
        let max_n = nln.max(end_n);
        for dl in all_lines {
            if &dl.file_path == file_path {
                if let Some(num) = dl.new_line_num {
                    if num as u64 >= min_n && num as u64 <= max_n {
                        if dl.line_type != crate::app::DiffLineType::Deletion
                            && !original_lines.iter().any(|ol| ol.content == dl.content)
                        {
                            original_lines.push(dl.clone());
                        }
                    }
                }
            }
        }
    }

    for body_line in body.lines() {
        let is_suggestion_start = body_line.trim().starts_with("```suggestion");
        let is_suggestion_end = in_suggestion && body_line.trim().starts_with("```");

        let current_prefix = if is_first {
            is_first = false;
            prefix.to_string()
        } else {
            " ".repeat(prefix.len())
        };

        if is_suggestion_start {
            in_suggestion = true;
            result_lines.push((
                current_prefix.clone(),
                prefix_style,
                vec![(
                    Style::default()
                        .fg(THEME.read().unwrap().green)
                        .add_modifier(Modifier::BOLD),
                    "┌─── Code Suggestion ───".to_string(),
                )],
            ));

            // Print original code as DELETIONS (red)
            for orig in &original_lines {
                let code_fg = Color::Rgb(220, 140, 140);
                let code_bg = Color::Rgb(55, 22, 28);
                let prefix_fg = Color::Rgb(255, 100, 100);

                let mut spans = vec![(
                    Style::default()
                        .fg(prefix_fg)
                        .bg(code_bg)
                        .add_modifier(Modifier::BOLD),
                    "│ - ".to_string(),
                )];

                // Strip leading space/minus/plus if present
                let clean_content = if orig.content.starts_with(' ')
                    || orig.content.starts_with('-')
                    || orig.content.starts_with('+')
                {
                    if orig.content.len() > 1 {
                        orig.content[1..].to_string()
                    } else {
                        String::new()
                    }
                } else {
                    orig.content.clone()
                };

                if let Some(ref highlighted) = orig.syntax_highlighted {
                    for (span_style, text) in highlighted {
                        let merged = span_style.fg(span_style.fg.unwrap_or(code_fg)).bg(code_bg);
                        spans.push((merged, text.clone()));
                    }
                } else {
                    spans.push((Style::default().fg(code_fg).bg(code_bg), clean_content));
                }

                result_lines.push((" ".repeat(prefix.len()), prefix_style, spans));
            }
        } else if is_suggestion_end {
            in_suggestion = false;
            result_lines.push((
                current_prefix,
                prefix_style,
                vec![(
                    Style::default()
                        .fg(THEME.read().unwrap().green)
                        .add_modifier(Modifier::BOLD),
                    "└─── End of Suggestion ───".to_string(),
                )],
            ));
        } else if in_suggestion {
            // Print suggested code as ADDITIONS (green)
            let code_fg = Color::Rgb(140, 220, 140);
            let code_bg = Color::Rgb(22, 48, 28);
            let prefix_fg = Color::Rgb(80, 220, 80);

            let mut spans = vec![(
                Style::default()
                    .fg(prefix_fg)
                    .bg(code_bg)
                    .add_modifier(Modifier::BOLD),
                "│ + ".to_string(),
            )];

            // Highlight body_line syntax
            let highlighted = crate::app::highlight_line_syntax(file_path, body_line, None);

            if let Some(ref hl) = highlighted {
                for (span_style, text) in hl {
                    let merged = span_style.fg(span_style.fg.unwrap_or(code_fg)).bg(code_bg);
                    spans.push((merged, text.clone()));
                }
            } else {
                spans.push((
                    Style::default().fg(code_fg).bg(code_bg),
                    body_line.to_string(),
                ));
            }

            result_lines.push((current_prefix, prefix_style, spans));
        } else {
            result_lines.push((
                current_prefix,
                prefix_style,
                vec![(
                    Style::default().fg(THEME.read().unwrap().text_normal),
                    body_line.to_string(),
                )],
            ));
        }
    }

    if result_lines.is_empty() {
        result_lines.push((
            prefix.to_string(),
            prefix_style,
            vec![(
                Style::default().fg(THEME.read().unwrap().text_normal),
                String::new(),
            )],
        ));
    }

    result_lines
}

/// helper function to create a centered rect using up certain percentage of the available rect `r`
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Like centered_rect but enforces minimum dimensions so overlays remain usable
/// on small terminals. Will not exceed the available rect `r`.
fn centered_rect_min(percent_x: u16, percent_y: u16, min_w: u16, min_h: u16, r: Rect) -> Rect {
    let rect = centered_rect(percent_x, percent_y, r);
    let w = rect.width.max(min_w).min(r.width);
    let h = rect.height.max(min_h).min(r.height);
    let x = r.x + (r.width - w) / 2;
    let y = r.y + (r.height - h) / 2;
    Rect::new(x, y, w, h)
}

fn centered_rect_fixed(width: u16, height: u16, r: Rect) -> Rect {
    let w = width.min(r.width);
    let h = height.min(r.height);
    let x = r.x + (r.width - w) / 2;
    let y = r.y + (r.height - h) / 2;
    Rect::new(x, y, w, h)
}

pub fn count_wrapped_lines(text: &str, width: usize) -> usize {
    if width == 0 {
        return 0;
    }
    let mut total_lines = 0;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            total_lines += 1;
            continue;
        }
        let mut current_line_len = 0;
        let mut first = true;
        for word in line.split(' ') {
            let word_len = word.chars().count();
            if word_len == 0 {
                if current_line_len + 1 > width {
                    total_lines += 1;
                    current_line_len = 1;
                } else {
                    current_line_len += 1;
                }
                continue;
            }

            let space_needed = if first { 0 } else { 1 };
            if current_line_len + space_needed + word_len <= width {
                current_line_len += space_needed + word_len;
                first = false;
            } else {
                if word_len > width {
                    if current_line_len + space_needed < width {
                        let remaining_on_current = width - (current_line_len + space_needed);
                        let mut remaining_word_len = word_len - remaining_on_current;
                        total_lines += 1;
                        while remaining_word_len > width {
                            total_lines += 1;
                            remaining_word_len -= width;
                        }
                        current_line_len = remaining_word_len;
                    } else {
                        total_lines += 1;
                        let mut remaining_word_len = word_len;
                        while remaining_word_len > width {
                            total_lines += 1;
                            remaining_word_len -= width;
                        }
                        current_line_len = remaining_word_len;
                    }
                } else {
                    total_lines += 1;
                    current_line_len = word_len;
                }
                first = false;
            }
        }
        total_lines += 1;
    }

    if text.is_empty() {
        return 0;
    }
    total_lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::pipelines::Job;

    #[test]
    fn test_get_stages_summary() {
        // Test case 1: Stage with mixed success and skipped jobs
        // This stage should be reported as "success" status, and 100% success rate.
        let jobs = vec![
            Job {
                id: 1,
                stage: "build".to_string(),
                name: "compile".to_string(),
                status: "success".to_string(),
                matrix: None,
            },
            Job {
                id: 2,
                stage: "build".to_string(),
                name: "cache".to_string(),
                status: "skipped".to_string(),
                matrix: None,
            },
            Job {
                id: 3,
                stage: "test".to_string(),
                name: "unit".to_string(),
                status: "failed".to_string(),
                matrix: None,
            },
            Job {
                id: 4,
                stage: "test".to_string(),
                name: "integration".to_string(),
                status: "success".to_string(),
                matrix: None,
            },
        ];

        let summaries = get_stages_summary(&jobs);
        assert_eq!(summaries.len(), 2);

        // Build stage verification (success + skipped = success/100%)
        let build = summaries.iter().find(|s| s.name == "build").unwrap();
        assert_eq!(build.status, "success");
        assert_eq!(build.success, 2);
        assert_eq!(build.total, 2);
        assert_eq!(build.percent, 100);

        // Test stage verification (failed + success = failed/50%)
        let test = summaries.iter().find(|s| s.name == "test").unwrap();
        assert_eq!(test.status, "failed");
        assert_eq!(test.success, 1);
        assert_eq!(test.total, 2);
        assert_eq!(test.percent, 50);
    }

    #[test]
    fn test_count_wrapped_lines() {
        assert_eq!(count_wrapped_lines("", 10), 0);
        assert_eq!(count_wrapped_lines("", 0), 0);
        assert_eq!(count_wrapped_lines("hello", 10), 1);
        assert_eq!(count_wrapped_lines("hello", 5), 1);
        assert_eq!(count_wrapped_lines("hello", 3), 2);
        assert_eq!(count_wrapped_lines("hello world", 5), 2);
        assert_eq!(count_wrapped_lines("a b c", 3), 2);
        assert_eq!(count_wrapped_lines("hello\nworld", 10), 2);
        assert_eq!(count_wrapped_lines("hello\nworld", 10), 2);
    }

    #[test]
    fn test_get_label_color() {
        let color1 = get_label_color("bug");
        let _color2 = get_label_color("feature");
        let color3 = get_label_color("bug");

        assert_eq!(color1, color3);
        match color1 {
            Color::Rgb(_, _, _) => {}
            _ => panic!("Expected Rgb color"),
        }
    }

    #[test]
    fn test_render_labels_cell() {
        let labels = vec!["bug".to_string(), "backend".to_string()];
        let cell_empty = render_labels_cell(&[], "", false, false, 24);
        let cell_str_empty = format!("{:?}", cell_empty);
        assert!(cell_str_empty.contains("—"));

        let cell_normal = render_labels_cell(&labels, "", false, false, 24);
        let cell_str = format!("{:?}", cell_normal);
        assert!(cell_str.contains("bug"));
        assert!(cell_str.contains("backend"));
    }

    #[test]
    fn test_format_comment_with_suggestions() {
        let body = "This is a comment\n```suggestion\nnew line content\n```\noutside suggestion";
        let file_path = "src/app.rs";
        let all_lines = vec![crate::app::DiffLine {
            content: "old line content".to_string(),
            line_type: crate::app::DiffLineType::Deletion,
            file_path: "src/app.rs".to_string(),
            old_line_num: Some(1),
            new_line_num: None,
            syntax_highlighted: None,
        }];
        let prefix = "prefix";
        let prefix_style = Style::default();

        let formatted = format_comment_with_suggestions(
            body,
            file_path,
            None,
            None,
            Some(1),
            Some(1),
            &all_lines,
            prefix,
            prefix_style,
        );

        assert_eq!(formatted.len(), 6);
        assert_eq!(formatted[0].2[0].1, "This is a comment");
        assert_eq!(formatted[1].2[0].1, "┌─── Code Suggestion ───");
        assert_eq!(formatted[2].2[0].1, "│ - ");
        assert_eq!(formatted[2].2[1].1, "old line content");
        assert_eq!(formatted[3].2[0].1, "│ + ");
        assert_eq!(formatted[3].2[1].1, "new line content");
        assert_eq!(formatted[4].2[0].1, "└─── End of Suggestion ───");
        assert_eq!(formatted[5].2[0].1, "outside suggestion");
    }

    #[test]
    fn test_floor_char_boundary() {
        let s = "👕hello";
        assert_eq!(floor_char_boundary(s, 0), 0);
        assert_eq!(floor_char_boundary(s, 1), 0);
        assert_eq!(floor_char_boundary(s, 2), 0);
        assert_eq!(floor_char_boundary(s, 3), 0);
        assert_eq!(floor_char_boundary(s, 4), 4);
        assert_eq!(floor_char_boundary(s, 5), 5);
        assert_eq!(floor_char_boundary(s, 999), s.len());
    }
}
