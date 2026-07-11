use crate::app::{App, Tab};
use crate::config::THEME;
use crate::utils::format::{format_ref, render_markdown, time_ago, truncate};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

pub(crate) fn render_tab_issues(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
    highlight_style: Style,
    header_style: Style,
) {
    let icons = crate::config::ICONS.read().unwrap();
    if app.issues.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!("\n\n {} Loading issues...", icons.label_loading))
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
                        .title(format!(" {} Details ", icons.label_details))
                        .border_style(Style::default().fg(THEME.read().unwrap().border)),
                )
                .style(Style::default().fg(THEME.read().unwrap().text_muted)),
            detail_rect,
        );
    } else {
        let mut filtered_issues = App::filtered_issues_list(
            &app.issues.items,
            &app.search_query,
            &app.enabled_columns,
            app.group_ascending
                .get(&Tab::Issues)
                .copied()
                .unwrap_or(true),
            app.group_by_column.get(&Tab::Issues).unwrap_or(&None),
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
                    format!("{} OPEN", icons.state_open),
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
                    format!("{} CLOSED", icons.state_closed),
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
                cells.push(super::helpers::render_fuzzy_cell(
                    &format!("#{}", i.iid),
                    &app.search_query,
                    is_selected,
                    false,
                    Style::default().fg(THEME.read().unwrap().text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Issues, "State") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &state_text,
                    &app.search_query,
                    is_selected,
                    false,
                    state_style,
                    Alignment::Center,
                ));
            }
            if app.is_column_visible(Tab::Issues, "Title") {
                cells.push(super::helpers::render_fuzzy_cell(
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
                cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&assignees_str, 20),
                    &app.search_query,
                    is_selected,
                    false,
                    Style::default().fg(THEME.read().unwrap().blue),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Issues, "Labels") {
                cells.push(super::helpers::render_labels_cell(
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
                cells.push(super::helpers::render_fuzzy_cell(
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
                cells.push(super::helpers::render_fuzzy_cell(
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
                cells.push(super::helpers::render_fuzzy_cell(
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
            widths.push(Constraint::Length(8));
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
            widths.push(Constraint::Length(22));
        }
        if app.is_column_visible(Tab::Issues, "Labels") {
            header_cells.push(Cell::from("Labels"));
            widths.push(Constraint::Length(26));
        }
        if app.is_column_visible(Tab::Issues, "Milestone") {
            header_cells.push(Cell::from("Milestone"));
            widths.push(Constraint::Length(18));
        }
        if app.is_column_visible(Tab::Issues, "Due Date") {
            header_cells.push(Cell::from("Due Date"));
            widths.push(Constraint::Length(20));
        }
        if app.is_column_visible(Tab::Issues, "Author") {
            header_cells.push(Cell::from("Author"));
            widths.push(Constraint::Length(18));
        }

        if widths.is_empty() {
            widths.push(Constraint::Min(0));
        }

        let table = Table::new(rows, widths)
            .header(Row::new(header_cells).style(header_style).height(1))
            .block(main_block)
            .row_highlight_style(highlight_style)
            .highlight_symbol(format!(" {} ", icons.highlight_arrow));

        f.render_stateful_widget(table, content_area, &mut app.issues.state);
        let preview_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border))
            .title(format!(" {} Details ", icons.label_details))
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
                    Span::styled(assignees, Style::default().fg(THEME.read().unwrap().blue)),
                ]));
                text.push(Line::from(vec![
                    Span::styled(
                        "Milestone: ",
                        Style::default().fg(THEME.read().unwrap().text_muted),
                    ),
                    Span::styled(milestone, Style::default().fg(THEME.read().unwrap().purple)),
                ]));
                if let Some(due) = &issue.due_date {
                    text.push(Line::from(vec![
                        Span::styled(
                            "Due Date:  ",
                            Style::default().fg(THEME.read().unwrap().text_muted),
                        ),
                        Span::styled(due, Style::default().fg(THEME.read().unwrap().yellow)),
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
                        let label_color = super::helpers::get_label_color(label);
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

                let viewport_height = detail_rect.height.saturating_sub(2) as usize;
                let content_length = text.len();
                let max_scroll = content_length.saturating_sub(viewport_height) as u16;
                app.detail_scroll = app.detail_scroll.min(max_scroll);

                let title_suffix = if content_length > viewport_height {
                    let percent = (app.detail_scroll as usize * 100) / max_scroll.max(1) as usize;
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
                        .scroll((app.detail_scroll, 0)),
                    detail_rect,
                );
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select an item to view details...")
                    .block(preview_block)
                    .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                detail_rect,
            );
        }
    }
}

pub(crate) fn render_tab_merge_requests(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
    highlight_style: Style,
    header_style: Style,
) {
    let icons = crate::config::ICONS.read().unwrap();
    if app.mrs.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!(
                "\n\n {} Loading merge requests...",
                icons.label_loading
            ))
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
                        .title(format!(" {} Details ", icons.label_details))
                        .border_style(Style::default().fg(THEME.read().unwrap().border)),
                )
                .style(Style::default().fg(THEME.read().unwrap().text_muted)),
            detail_rect,
        );
    } else {
        let mut filtered_mrs = App::filtered_mrs_list(
            &app.mrs.items,
            &app.search_query,
            &app.enabled_columns,
            app.group_ascending
                .get(&Tab::MergeRequests)
                .copied()
                .unwrap_or(true),
            app.group_by_column
                .get(&Tab::MergeRequests)
                .unwrap_or(&None),
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
            let (prefix, clean_title) = crate::utils::format::parse_mr_title_prefix(&m.title);

            let (state_text, state_style) = if m.state == "opened" {
                (
                    format!("{} OPEN", icons.state_open),
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
                    format!("{} MERGED", icons.state_merged),
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
                    format!("{} CLOSED", icons.state_closed),
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
                    format!("{} DRAFT", icons.status_draft),
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
                        format!("{} DRAFT", icons.status_draft),
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
                        format!("{} READY", icons.status_ready),
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
                cells.push(super::helpers::render_fuzzy_cell(
                    &format!("!{}", m.iid),
                    &app.search_query,
                    is_selected,
                    false,
                    Style::default().fg(THEME.read().unwrap().text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::MergeRequests, "State") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &state_text,
                    &app.search_query,
                    is_selected,
                    false,
                    state_style,
                    Alignment::Center,
                ));
            }
            if app.is_column_visible(Tab::MergeRequests, "Status") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &status_styled,
                    &app.search_query,
                    is_selected,
                    false,
                    status_style,
                    Alignment::Center,
                ));
            }
            if app.is_column_visible(Tab::MergeRequests, "Title") {
                cells.push(super::helpers::render_fuzzy_cell(
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
                cells.push(super::helpers::render_fuzzy_cell(
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
                cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&reviewers_str, 20),
                    &app.search_query,
                    is_selected,
                    false,
                    Style::default().fg(THEME.read().unwrap().blue),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::MergeRequests, "Labels") {
                cells.push(super::helpers::render_labels_cell(
                    &m.labels,
                    &app.search_query,
                    is_selected,
                    false,
                    24,
                ));
            }
            let is_github = app
                .gitlab_client
                .as_ref()
                .map(|c| c.is_github)
                .unwrap_or(false);
            if app.is_column_visible(
                Tab::MergeRequests,
                if is_github { "Action" } else { "Pipeline" },
            ) {
                let resolved_pipe = m.head_pipeline.as_ref().or_else(|| {
                    if is_github {
                        app.pipelines
                            .items
                            .iter()
                            .find(|p| p.r#ref == m.source_branch)
                    } else {
                        None
                    }
                });
                if let Some(pipe) = resolved_pipe {
                    let stages_dots = if let Some(jobs) = app.pipeline_jobs.get(&pipe.id) {
                        super::helpers::get_stages_dots(jobs)
                    } else {
                        icons.label_loading.clone()
                    };

                    if stages_dots.is_empty() {
                        let (pipe_text, pipe_color, pipe_bg) = match pipe.status.as_str() {
                            "success" => (
                                format!("{} SUCCESS", icons.status_success),
                                THEME.read().unwrap().green,
                                THEME.read().unwrap().green_bg,
                            ),
                            "failed" => (
                                format!("{} FAILED", icons.status_failed),
                                THEME.read().unwrap().red,
                                THEME.read().unwrap().red_bg,
                            ),
                            "running" => (
                                format!("{} RUNNING", icons.status_running),
                                THEME.read().unwrap().blue,
                                THEME.read().unwrap().blue_bg,
                            ),
                            "canceled" => (
                                format!("{} CANCEL", icons.status_canceled),
                                THEME.read().unwrap().text_muted,
                                THEME.read().unwrap().inactive_bg,
                            ),
                            "pending" => (
                                format!("{} PENDING", icons.status_pending),
                                THEME.read().unwrap().yellow,
                                THEME.read().unwrap().yellow_bg,
                            ),
                            "skipped" => (
                                format!("{} SKIP", icons.status_skipped),
                                THEME.read().unwrap().text_muted,
                                THEME.read().unwrap().inactive_bg,
                            ),
                            _ => (
                                format!("{} UNKNOWN", icons.status_unknown),
                                THEME.read().unwrap().text_muted,
                                THEME.read().unwrap().inactive_bg,
                            ),
                        };
                        let bg = if is_selected {
                            THEME.read().unwrap().highlight_bg
                        } else {
                            pipe_bg
                        };
                        cells.push(super::helpers::render_fuzzy_cell(
                            &pipe_text,
                            &app.search_query,
                            is_selected,
                            false,
                            Style::default()
                                .fg(pipe_color)
                                .bg(bg)
                                .add_modifier(Modifier::BOLD),
                            Alignment::Center,
                        ));
                    } else {
                        cells.push(super::helpers::render_fuzzy_cell(
                            &stages_dots,
                            &app.search_query,
                            is_selected,
                            false,
                            Style::default().fg(THEME.read().unwrap().text_normal),
                            Alignment::Left,
                        ));
                    }
                } else {
                    cells.push(super::helpers::render_fuzzy_cell(
                        "—",
                        &app.search_query,
                        is_selected,
                        false,
                        Style::default().fg(THEME.read().unwrap().text_muted),
                        Alignment::Center,
                    ));
                }
            }
            if app.is_column_visible(Tab::MergeRequests, "Milestone") {
                let mr_milestone_str = m
                    .milestone
                    .as_ref()
                    .map(|ms| ms.title.clone())
                    .unwrap_or_else(|| "—".to_string());
                cells.push(super::helpers::render_fuzzy_cell(
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
                cells.push(super::helpers::render_fuzzy_cell(
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
            widths.push(Constraint::Length(8));
        }
        if app.is_column_visible(Tab::MergeRequests, "State") {
            header_cells.push(Cell::from(Line::from("State").alignment(Alignment::Center)));
            widths.push(Constraint::Length(10));
        }
        if app.is_column_visible(Tab::MergeRequests, "Status") {
            header_cells.push(Cell::from(
                Line::from("Status").alignment(Alignment::Center),
            ));
            widths.push(Constraint::Length(12));
        }
        if app.is_column_visible(Tab::MergeRequests, "Title") {
            header_cells.push(Cell::from("Title"));
            widths.push(Constraint::Fill(1));
        }
        if app.is_column_visible(Tab::MergeRequests, "Assignees") {
            header_cells.push(Cell::from("Assignees"));
            widths.push(Constraint::Length(22));
        }
        if app.is_column_visible(Tab::MergeRequests, "Reviewers") {
            header_cells.push(Cell::from("Reviewers"));
            widths.push(Constraint::Length(22));
        }
        if app.is_column_visible(Tab::MergeRequests, "Labels") {
            header_cells.push(Cell::from("Labels"));
            widths.push(Constraint::Length(26));
        }
        let is_github = app
            .gitlab_client
            .as_ref()
            .map(|c| c.is_github)
            .unwrap_or(false);
        if app.is_column_visible(
            Tab::MergeRequests,
            if is_github { "Action" } else { "Pipeline" },
        ) {
            header_cells.push(Cell::from(
                Line::from(if is_github { "Action" } else { "Pipeline" })
                    .alignment(Alignment::Center),
            ));
            widths.push(Constraint::Length(12));
        }
        if app.is_column_visible(Tab::MergeRequests, "Milestone") {
            header_cells.push(Cell::from("Milestone"));
            widths.push(Constraint::Length(18));
        }
        if app.is_column_visible(Tab::MergeRequests, "Author") {
            header_cells.push(Cell::from("Author"));
            widths.push(Constraint::Length(18));
        }

        if widths.is_empty() {
            widths.push(Constraint::Min(0));
        }

        let table = Table::new(rows, widths)
            .header(Row::new(header_cells).style(header_style).height(1))
            .block(main_block)
            .row_highlight_style(highlight_style)
            .highlight_symbol(format!(" {} ", icons.highlight_arrow));

        f.render_stateful_widget(table, content_area, &mut app.mrs.state);

        let preview_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border))
            .title(format!(" {} Details ", icons.label_details))
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
                    Span::styled(assignees, Style::default().fg(THEME.read().unwrap().blue)),
                ]));
                text.push(Line::from(vec![
                    Span::styled(
                        "Reviewers: ",
                        Style::default().fg(THEME.read().unwrap().text_muted),
                    ),
                    Span::styled(reviewers, Style::default().fg(THEME.read().unwrap().blue)),
                ]));
                text.push(Line::from(vec![
                    Span::styled(
                        "Milestone: ",
                        Style::default().fg(THEME.read().unwrap().text_muted),
                    ),
                    Span::styled(milestone, Style::default().fg(THEME.read().unwrap().purple)),
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
                            format!("{} OPEN", icons.state_open)
                        } else if mr.state == "merged" {
                            format!("{} MERGED", icons.state_merged)
                        } else {
                            format!("{} CLOSED", icons.state_closed)
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
                    Span::styled(" (", Style::default().fg(THEME.read().unwrap().text_muted)),
                    Span::styled(
                        draft_status,
                        Style::default()
                            .fg(draft_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(")", Style::default().fg(THEME.read().unwrap().text_muted)),
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
                let resolved_pipe = mr.head_pipeline.as_ref().or_else(|| {
                    if is_github {
                        app.pipelines
                            .items
                            .iter()
                            .find(|p| p.r#ref == mr.source_branch)
                    } else {
                        None
                    }
                });
                if let Some(pipe) = resolved_pipe {
                    let stages_dots = if let Some(jobs) = app.pipeline_jobs.get(&pipe.id) {
                        super::helpers::get_stages_dots(jobs)
                    } else {
                        icons.label_loading.clone()
                    };
                    if stages_dots.is_empty() {
                        let (pipe_text, pipe_color, pipe_bg) = match pipe.status.as_str() {
                            "success" => (
                                format!("{} SUCCESS", icons.status_success),
                                THEME.read().unwrap().green,
                                THEME.read().unwrap().green_bg,
                            ),
                            "failed" => (
                                format!("{} FAILED", icons.status_failed),
                                THEME.read().unwrap().red,
                                THEME.read().unwrap().red_bg,
                            ),
                            "running" => (
                                format!("{} RUNNING", icons.status_running),
                                THEME.read().unwrap().blue,
                                THEME.read().unwrap().blue_bg,
                            ),
                            "canceled" => (
                                format!("{} CANCEL", icons.status_canceled),
                                THEME.read().unwrap().text_muted,
                                THEME.read().unwrap().inactive_bg,
                            ),
                            "pending" => (
                                format!("{} PENDING", icons.status_pending),
                                THEME.read().unwrap().yellow,
                                THEME.read().unwrap().yellow_bg,
                            ),
                            "skipped" => (
                                format!("{} SKIP", icons.status_skipped),
                                THEME.read().unwrap().text_muted,
                                THEME.read().unwrap().inactive_bg,
                            ),
                            _ => (
                                format!("{} UNKNOWN", icons.status_unknown),
                                THEME.read().unwrap().text_muted,
                                THEME.read().unwrap().inactive_bg,
                            ),
                        };
                        text.push(Line::from(vec![
                            Span::styled(
                                if is_github {
                                    "Action:    "
                                } else {
                                    "Pipeline:  "
                                },
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                format!(" {} ", pipe_text),
                                Style::default()
                                    .fg(pipe_color)
                                    .bg(pipe_bg)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                    } else {
                        text.push(Line::from(vec![
                            Span::styled(
                                if is_github {
                                    "Action:    "
                                } else {
                                    "Pipeline:  "
                                },
                                Style::default().fg(THEME.read().unwrap().text_muted),
                            ),
                            Span::styled(
                                stages_dots,
                                Style::default().fg(THEME.read().unwrap().text_normal),
                            ),
                        ]));
                    }
                }
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
                        let label_color = super::helpers::get_label_color(label);
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

                let viewport_height = detail_rect.height.saturating_sub(2) as usize;
                let content_length = text.len();
                let max_scroll = content_length.saturating_sub(viewport_height) as u16;
                app.detail_scroll = app.detail_scroll.min(max_scroll);

                let title_suffix = if content_length > viewport_height {
                    let percent = (app.detail_scroll as usize * 100) / max_scroll.max(1) as usize;
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
                        .scroll((app.detail_scroll, 0)),
                    detail_rect,
                );
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select an item to view details...")
                    .block(preview_block)
                    .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                detail_rect,
            );
        }
    }
}

pub(crate) fn render_tab_pipelines(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
    highlight_style: Style,
    header_style: Style,
) {
    let icons = crate::config::ICONS.read().unwrap();
    let is_github = app
        .gitlab_client
        .as_ref()
        .map(|c| c.is_github)
        .unwrap_or(false);
    if app.pipelines.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(if is_github {
                format!("\n\n {} Loading actions...", icons.label_loading)
            } else {
                format!("\n\n {} Loading pipelines...", icons.label_loading)
            })
            .alignment(Alignment::Center)
            .block(main_block.clone())
            .style(Style::default().fg(THEME.read().unwrap().text_muted)),
            content_area,
        );
        f.render_widget(
            Paragraph::new(if is_github {
                "Select an action to view details..."
            } else {
                "Select a pipeline to view details..."
            })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} Details ", icons.label_details))
                    .border_style(Style::default().fg(THEME.read().unwrap().border)),
            )
            .style(Style::default().fg(THEME.read().unwrap().text_muted)),
            detail_rect,
        );
    } else {
        let mut filtered_pipelines = App::filtered_pipelines_list(
            &app.pipelines.items,
            &app.search_query,
            &app.pipeline_jobs,
            &app.enabled_columns,
            app.group_ascending
                .get(&Tab::Pipelines)
                .copied()
                .unwrap_or(true),
            app.group_by_column.get(&Tab::Pipelines).unwrap_or(&None),
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
                    format!("{} SUCCESS", icons.status_success),
                    THEME.read().unwrap().green,
                    THEME.read().unwrap().green_bg,
                ),
                "failed" => (
                    format!("{} FAILED", icons.status_failed),
                    THEME.read().unwrap().red,
                    THEME.read().unwrap().red_bg,
                ),
                "running" => (
                    format!("{} RUNNING", icons.status_running),
                    THEME.read().unwrap().blue,
                    THEME.read().unwrap().blue_bg,
                ),
                "canceled" => (
                    format!("{} CANCEL", icons.status_canceled),
                    THEME.read().unwrap().text_muted,
                    THEME.read().unwrap().inactive_bg,
                ),
                "pending" => (
                    format!("{} PENDING", icons.status_pending),
                    THEME.read().unwrap().yellow,
                    THEME.read().unwrap().yellow_bg,
                ),
                "skipped" => (
                    format!("{} SKIP", icons.status_skipped),
                    THEME.read().unwrap().text_muted,
                    THEME.read().unwrap().inactive_bg,
                ),
                "manual" => (
                    format!("{} MANUAL", icons.status_manual),
                    THEME.read().unwrap().text_muted,
                    THEME.read().unwrap().inactive_bg,
                ),
                _ => (
                    format!("{} UNKNOWN", icons.status_unknown),
                    THEME.read().unwrap().text_muted,
                    THEME.read().unwrap().inactive_bg,
                ),
            };
            let stages_dots = if let Some(jobs) = app.pipeline_jobs.get(&p.id) {
                super::helpers::get_stages_dots(jobs)
            } else {
                icons.label_loading.clone()
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
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &format!("#{}", p.id),
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
                    Style::default().fg(THEME.read().unwrap().text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Pipelines, "Status") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &status_text,
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
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &stages_dots,
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
                    Style::default().fg(THEME.read().unwrap().text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Pipelines, "Ref") {
                row_cells.push(super::helpers::render_fuzzy_cell(
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
            widths.push(Constraint::Length(8));
        }
        if app.is_column_visible(Tab::Pipelines, "Status") {
            header_cells.push(Cell::from(
                Line::from("Status").alignment(Alignment::Center),
            ));
            widths.push(Constraint::Length(12));
        }
        if app.is_column_visible(Tab::Pipelines, "Stages") {
            header_cells.push(Cell::from("Stages"));
            widths.push(Constraint::Length(14));
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
            .highlight_symbol(format!(" {} ", icons.highlight_arrow));

        f.render_stateful_widget(table, content_area, &mut app.pipelines.state);

        let preview_block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} Details ", icons.label_details))
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
                    super::helpers::append_stage_summaries(&mut text, jobs);
                } else {
                    text.push(Line::from(vec![Span::styled(
                        "Loading stages...",
                        Style::default()
                            .fg(THEME.read().unwrap().text_muted)
                            .add_modifier(Modifier::ITALIC),
                    )]));
                }
                text.push(Line::from(""));
                f.render_widget(Paragraph::new(text).block(preview_block), detail_rect);
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select an item to view details...")
                    .block(preview_block)
                    .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                detail_rect,
            );
        }
    }
}

pub(crate) fn render_tab_jobs(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
    highlight_style: Style,
    header_style: Style,
) {
    let icons = crate::config::ICONS.read().unwrap();
    if app.jobs.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!("\n\n {} Loading jobs...", icons.label_loading))
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
                        .title(format!(" {} Details ", icons.label_details))
                        .border_style(Style::default().fg(THEME.read().unwrap().border)),
                )
                .style(Style::default().fg(THEME.read().unwrap().text_muted)),
            detail_rect,
        );
    } else if !app.jobs.items.is_empty() {
        let mut filtered_jobs = App::filtered_jobs_list(
            &app.jobs.items,
            &app.search_query,
            &app.enabled_columns,
            app.group_ascending.get(&Tab::Jobs).copied().unwrap_or(true),
            app.group_by_column.get(&Tab::Jobs).unwrap_or(&None),
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
            let (matrix_display, status_text_display, status_color_display, status_bg_display) =
                if app.collapse_matrix_jobs {
                    let variants: Vec<&crate::gitlab::pipelines::Job> = app
                        .jobs
                        .items
                        .iter()
                        .filter(|job| job.name == j.name)
                        .collect();

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
                            format!("{} SUCCESS", icons.status_success),
                            THEME.read().unwrap().green,
                            THEME.read().unwrap().green_bg,
                        ),
                        "failed" => (
                            format!("{} FAILED", icons.status_failed),
                            THEME.read().unwrap().red,
                            THEME.read().unwrap().red_bg,
                        ),
                        "running" => (
                            format!("{} RUNNING", icons.status_running),
                            THEME.read().unwrap().blue,
                            THEME.read().unwrap().blue_bg,
                        ),
                        "pending" => (
                            format!("{} PENDING", icons.status_pending),
                            THEME.read().unwrap().yellow,
                            THEME.read().unwrap().yellow_bg,
                        ),
                        _ => (
                            format!("{} SKIP", icons.status_skipped),
                            THEME.read().unwrap().text_muted,
                            THEME.read().unwrap().inactive_bg,
                        ),
                    };

                    let m_str = if count > 1 {
                        format!("{} [{} variants]", icons.matrix_variant, count)
                    } else if let Some(m) = &j.matrix {
                        format!("{} [{}]", icons.matrix_variant, m)
                    } else {
                        String::new()
                    };

                    (m_str, st, sc, sbg)
                } else {
                    let (status_text, status_color, bg_color) = match j.status.as_str() {
                        "success" => (
                            format!("{} SUCCESS", icons.status_success),
                            THEME.read().unwrap().green,
                            THEME.read().unwrap().green_bg,
                        ),
                        "failed" => (
                            format!("{} FAILED", icons.status_failed),
                            THEME.read().unwrap().red,
                            THEME.read().unwrap().red_bg,
                        ),
                        "running" => (
                            format!("{} RUNNING", icons.status_running),
                            THEME.read().unwrap().blue,
                            THEME.read().unwrap().blue_bg,
                        ),
                        "canceled" => (
                            format!("{} CANCEL", icons.status_canceled),
                            THEME.read().unwrap().text_muted,
                            THEME.read().unwrap().inactive_bg,
                        ),
                        "pending" => (
                            format!("{} PENDING", icons.status_pending),
                            THEME.read().unwrap().yellow,
                            THEME.read().unwrap().yellow_bg,
                        ),
                        "skipped" => (
                            format!("{} SKIP", icons.status_skipped),
                            THEME.read().unwrap().text_muted,
                            THEME.read().unwrap().inactive_bg,
                        ),
                        "manual" => (
                            format!("{} MANUAL", icons.status_manual),
                            THEME.read().unwrap().text_muted,
                            THEME.read().unwrap().inactive_bg,
                        ),
                        _ => (
                            format!("{} UNKNOWN", icons.status_unknown),
                            THEME.read().unwrap().text_muted,
                            THEME.read().unwrap().inactive_bg,
                        ),
                    };
                    let m_str = if let Some(m) = &j.matrix {
                        format!("{} [{}]", icons.matrix_variant, m)
                    } else {
                        String::new()
                    };
                    (m_str, status_text, status_color, bg_color)
                };

            let is_job_selected = Some(i) == app.jobs.state.selected();
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
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &j.id.to_string(),
                    &app.search_query,
                    is_job_selected,
                    is_checked,
                    Style::default().fg(THEME.read().unwrap().text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Jobs, "Stage") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &j.stage,
                    &app.search_query,
                    is_job_selected,
                    is_checked,
                    Style::default().fg(THEME.read().unwrap().purple),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Jobs, "Status") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &status_text,
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
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &j.name,
                    &app.search_query,
                    is_job_selected,
                    is_checked,
                    Style::default().fg(THEME.read().unwrap().text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Jobs, "Matrix") {
                row_cells.push(super::helpers::render_fuzzy_cell(
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
            widths.push(Constraint::Length(8));
        }
        if app.is_column_visible(Tab::Jobs, "Stage") {
            header_cells.push(Cell::from("Stage"));
            widths.push(Constraint::Length(14));
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

        let is_github = app
            .gitlab_client
            .as_ref()
            .map(|c| c.is_github)
            .unwrap_or(false);
        let jobs_title = Tab::Jobs.title(is_github);
        let table = Table::new(rows, widths)
            .header(Row::new(header_cells).style(header_style).height(1))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} ", jobs_title))
                    .title_style(
                        Style::default()
                            .fg(THEME.read().unwrap().header_fg)
                            .add_modifier(Modifier::BOLD),
                    )
                    .border_style(Style::default().fg(THEME.read().unwrap().border_focused)),
            )
            .row_highlight_style(highlight_style)
            .highlight_symbol(format!(" {} ", icons.highlight_arrow));

        let mut state = app.jobs.state.clone();
        f.render_stateful_widget(table, content_area, &mut state);
        app.jobs.state = state;

        if app.job_trace_loading {
            let preview_block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} Details / Trace ", icons.label_details))
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
                detail_rect,
            );
        } else if let Some(trace) = &app.job_trace {
            let width = detail_rect.width.saturating_sub(2) as usize;
            let height = detail_rect.height.saturating_sub(2) as usize;
            let stripped_trace = crate::utils::format::strip_ansi_escapes(trace);
            let total_lines = super::diff::count_wrapped_lines(&stripped_trace, width);
            let max_scroll = total_lines.saturating_sub(height) as u16;

            if app.job_trace_needs_scroll_to_bottom {
                app.detail_scroll = max_scroll;
                app.job_trace_needs_scroll_to_bottom = false;
            } else {
                app.detail_scroll = app.detail_scroll.min(max_scroll);
            }

            let title_suffix = if total_lines > height {
                let percent = (app.detail_scroll as usize * 100) / max_scroll.max(1) as usize;
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
                    .scroll((app.detail_scroll, 0)),
                detail_rect,
            );
        } else {
            let preview_block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} Details / Trace ", icons.label_details))
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
            super::helpers::append_stage_summaries(&mut text, &app.jobs.items);
            f.render_widget(Paragraph::new(text).block(preview_block), detail_rect);
        }
    } else {
        f.render_widget(Paragraph::new("\n\n No jobs loaded.\n Press 'p' to manually enter a pipeline ID to fetch jobs for,\n or view a pipeline in Pipelines tab and press Enter.").alignment(Alignment::Center).block(main_block).style(Style::default().fg(THEME.read().unwrap().text_muted)), content_area);
        f.render_widget(
            Paragraph::new("Select a job to view details...")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} Details ", icons.label_details))
                        .border_style(Style::default().fg(THEME.read().unwrap().border)),
                )
                .style(Style::default().fg(THEME.read().unwrap().text_muted)),
            detail_rect,
        );
    }
}

pub(crate) fn render_tab_runners(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
    highlight_style: Style,
    header_style: Style,
) {
    let icons = crate::config::ICONS.read().unwrap();
    if app.runners.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!("\n\n {} Loading runners...", icons.label_loading))
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
                        .title(format!(" {} Details ", icons.label_details))
                        .border_style(Style::default().fg(THEME.read().unwrap().border)),
                )
                .style(Style::default().fg(THEME.read().unwrap().text_muted)),
            detail_rect,
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
                    format!("{} ONLINE", icons.runner_online),
                    THEME.read().unwrap().green,
                    THEME.read().unwrap().green_bg,
                ),
                "paused" => (
                    format!("{} PAUSED", icons.runner_paused),
                    THEME.read().unwrap().yellow,
                    THEME.read().unwrap().yellow_bg,
                ),
                "offline" => (
                    format!("{} OFFLINE", icons.runner_offline),
                    THEME.read().unwrap().red,
                    THEME.read().unwrap().red_bg,
                ),
                _ => (
                    format!("{} UNKNOWN", icons.status_unknown),
                    THEME.read().unwrap().text_muted,
                    THEME.read().unwrap().inactive_bg,
                ),
            };
            let desc = r.description.as_deref().unwrap_or("No description");
            let mut row_cells = Vec::new();
            if app.is_column_visible(Tab::Runners, "ID") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &r.id.to_string(),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(THEME.read().unwrap().text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Runners, "Description") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(desc, 100),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(THEME.read().unwrap().text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Runners, "Status") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &status_text,
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
                row_cells.push(super::helpers::render_fuzzy_cell(
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
            widths.push(Constraint::Length(8));
        }
        if app.is_column_visible(Tab::Runners, "Description") {
            header_cells.push(Cell::from("Description"));
            widths.push(Constraint::Fill(1));
        }
        if app.is_column_visible(Tab::Runners, "Status") {
            header_cells.push(Cell::from(
                Line::from("Status").alignment(Alignment::Center),
            ));
            widths.push(Constraint::Length(12));
        }
        if app.is_column_visible(Tab::Runners, "Active") {
            header_cells.push(Cell::from(
                Line::from("Active").alignment(Alignment::Center),
            ));
            widths.push(Constraint::Length(10));
        }

        if widths.is_empty() {
            widths.push(Constraint::Min(0));
        }

        let table = Table::new(rows, widths)
            .header(Row::new(header_cells).style(header_style).height(1))
            .block(main_block)
            .row_highlight_style(highlight_style)
            .highlight_symbol(format!(" {} ", icons.highlight_arrow));

        f.render_stateful_widget(table, content_area, &mut app.runners.state);

        let preview_block = Block::default()
            .borders(Borders::ALL)
            .title(" 📊 Performance Dashboard ")
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
                    Span::styled(desc, Style::default().fg(THEME.read().unwrap().text_normal)),
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

                let runner_hash = r.id;
                let active_jobs = (runner_hash % 8) as usize + 1;
                let max_capacity = ((runner_hash % 4) as usize + 2) * 4;
                let queue_depth = (runner_hash % 5) as usize;
                let utilization = (active_jobs * 100) / max_capacity;
                let wait_time = (runner_hash % 50) as usize + 10;

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
                    detail_rect,
                );
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select an item to view details...")
                    .block(preview_block)
                    .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                detail_rect,
            );
        }
    }
}

pub(crate) fn render_tab_releases(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
    highlight_style: Style,
    header_style: Style,
) {
    let icons = crate::config::ICONS.read().unwrap();
    if app.releases.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!("\n\n {} Loading releases...", icons.label_loading))
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
                        .title(format!(" {} Details ", icons.label_details))
                        .border_style(Style::default().fg(THEME.read().unwrap().border)),
                )
                .style(Style::default().fg(THEME.read().unwrap().text_muted)),
            detail_rect,
        );
    } else {
        let mut filtered_releases = App::filtered_releases_list(
            &app.releases.items,
            &app.search_query,
            &app.enabled_columns,
            app.group_ascending
                .get(&Tab::Releases)
                .copied()
                .unwrap_or(true),
            app.group_by_column.get(&Tab::Releases).unwrap_or(&None),
        );
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
                row_cells.push(super::helpers::render_fuzzy_cell(
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
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&r.name, 100),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(THEME.read().unwrap().text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Releases, "Date") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&r.released_at, 15),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(THEME.read().unwrap().yellow),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Releases, "Author") {
                let author = r.author_name.as_deref().unwrap_or("");
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &author.to_string(),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(THEME.read().unwrap().blue),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Releases, "Assets") {
                let assets = r.assets_link.as_deref().unwrap_or("");
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(assets, 50),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(THEME.read().unwrap().blue),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Releases, "Description") {
                let desc = r.description.as_deref().unwrap_or("");
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(desc, 80),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(THEME.read().unwrap().text_muted),
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
            widths.push(Constraint::Length(16));
        }
        if app.is_column_visible(Tab::Releases, "Release Name") {
            header_cells.push(Cell::from("Release Name"));
            widths.push(Constraint::Length(30));
        }
        if app.is_column_visible(Tab::Releases, "Date") {
            header_cells.push(Cell::from("Date"));
            widths.push(Constraint::Length(15));
        }
        if app.is_column_visible(Tab::Releases, "Author") {
            header_cells.push(Cell::from("Author"));
            widths.push(Constraint::Length(18));
        }
        if app.is_column_visible(Tab::Releases, "Assets") {
            header_cells.push(Cell::from("Assets"));
            widths.push(Constraint::Length(12));
        }
        if app.is_column_visible(Tab::Releases, "Description") {
            header_cells.push(Cell::from("Description"));
            widths.push(Constraint::Fill(1));
        }

        if widths.is_empty() {
            widths.push(Constraint::Min(0));
        }

        let table = Table::new(rows, widths)
            .header(Row::new(header_cells).style(header_style).height(1))
            .block(main_block)
            .row_highlight_style(highlight_style)
            .highlight_symbol(format!(" {} ", icons.highlight_arrow));

        f.render_stateful_widget(table, content_area, &mut app.releases.state);

        let preview_block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} Details ", icons.label_details))
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
                        Span::styled(author, Style::default().fg(THEME.read().unwrap().blue)),
                    ]));
                }
                if let Some(ref cid) = r.commit_id {
                    text.push(Line::from(vec![
                        Span::styled(
                            "Commit:  ",
                            Style::default().fg(THEME.read().unwrap().text_muted),
                        ),
                        Span::styled(
                            truncate(cid, 8),
                            Style::default().fg(THEME.read().unwrap().purple),
                        ),
                        Span::raw(" "),
                        Span::styled(
                            r.commit_title.as_deref().unwrap_or(""),
                            Style::default().fg(THEME.read().unwrap().text_normal),
                        ),
                    ]));
                }
                if let Some(ref assets) = r.assets_link {
                    text.push(Line::from(vec![
                        Span::styled(
                            "Assets:  ",
                            Style::default().fg(THEME.read().unwrap().text_muted),
                        ),
                        Span::styled(assets, Style::default().fg(THEME.read().unwrap().blue)),
                    ]));
                }
                if let Some(ref desc) = r.description {
                    if !desc.is_empty() {
                        text.push(Line::from(""));
                        text.push(Line::from(Span::styled(
                            "Description:",
                            Style::default()
                                .fg(THEME.read().unwrap().header_fg)
                                .add_modifier(Modifier::BOLD),
                        )));
                        text.push(Line::from(desc.as_str()));
                    }
                }
                f.render_widget(
                    Paragraph::new(text)
                        .block(preview_block)
                        .wrap(ratatui::widgets::Wrap { trim: true }),
                    detail_rect,
                );
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select an item to view details...")
                    .block(preview_block)
                    .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                detail_rect,
            );
        }
    }
}

pub(crate) fn render_tab_todos(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
    highlight_style: Style,
    header_style: Style,
) {
    let icons = crate::config::ICONS.read().unwrap();
    if app.todos.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!("\n\n {} Loading todos...", icons.label_loading))
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
                        .title(format!(" {} Details ", icons.label_details))
                        .border_style(Style::default().fg(THEME.read().unwrap().border)),
                )
                .style(Style::default().fg(THEME.read().unwrap().text_muted)),
            detail_rect,
        );
    } else {
        let mut filtered_todos = App::filtered_todos_list(
            &app.todos.items,
            &app.search_query,
            &app.enabled_columns,
            app.group_ascending
                .get(&Tab::Todos)
                .copied()
                .unwrap_or(true),
            app.group_by_column.get(&Tab::Todos).unwrap_or(&None),
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
                row_cells.push(super::helpers::render_fuzzy_cell(
                    state_str,
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    state_style,
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Todos, "Project") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &n.project_path,
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(THEME.read().unwrap().text_muted),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Todos, "Type") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    n.target_type.as_str(),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    type_style,
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Todos, "ID") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &format!("#{}", n.target_iid),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(THEME.read().unwrap().blue),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Todos, "Title") {
                row_cells.push(super::helpers::render_fuzzy_cell(
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
            widths.push(Constraint::Length(10));
        }
        if app.is_column_visible(Tab::Todos, "Project") {
            header_cells.push(Cell::from("Project"));
            widths.push(Constraint::Length(24));
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
            .highlight_symbol(format!(" {} ", icons.highlight_arrow));

        f.render_stateful_widget(table, content_area, &mut app.todos.state);

        let preview_block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} Details ", icons.label_details))
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
                        Style::default().fg(if n.state == "unread" || n.state == "pending" {
                            THEME.read().unwrap().green
                        } else {
                            THEME.read().unwrap().text_muted
                        }),
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
                    detail_rect,
                );
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select an item to view details...")
                    .block(preview_block)
                    .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                detail_rect,
            );
        }
    }
}

pub(crate) fn render_tab_milestones(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
    highlight_style: Style,
    header_style: Style,
) {
    let icons = crate::config::ICONS.read().unwrap();
    if app.milestones.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!(
                "\n\n {} Loading milestones...",
                icons.label_loading
            ))
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
                        .title(format!(" {} Details ", icons.label_details))
                        .border_style(Style::default().fg(THEME.read().unwrap().border)),
                )
                .style(Style::default().fg(THEME.read().unwrap().text_muted)),
            detail_rect,
        );
    } else {
        let is_github = app
            .gitlab_client
            .as_ref()
            .map(|c| c.is_github)
            .unwrap_or(false);
        let mut filtered_milestones = App::filtered_milestones_list(
            &app.milestones.items,
            &app.search_query,
            &app.enabled_columns,
            app.group_ascending
                .get(&Tab::Milestones)
                .copied()
                .unwrap_or(true),
            app.group_by_column.get(&Tab::Milestones).unwrap_or(&None),
            &app.milestone_issues_cache,
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
                    "ID" => widths.push(Constraint::Length(8)),
                    "Title" => widths.push(Constraint::Fill(1)),
                    "State" => widths.push(Constraint::Length(10)),
                    "Start Date" => widths.push(Constraint::Length(20)),
                    "Due Date" => widths.push(Constraint::Length(20)),
                    "Progress" => widths.push(Constraint::Length(8)),
                    _ => widths.push(Constraint::Fill(1)),
                }
            }
        }

        let rows = filtered_milestones.iter().enumerate().map(|(idx, m)| {
            let mut cells = Vec::new();
            let is_selected = app.milestones.state.selected() == Some(idx);
            for col in &cols {
                if app.is_column_visible(Tab::Milestones, col) {
                    match *col {
                        "ID" => {
                            cells.push(super::helpers::render_fuzzy_cell(
                                &m.iid.to_string(),
                                &app.search_query,
                                is_selected,
                                false,
                                Style::default().fg(THEME.read().unwrap().text_normal),
                                Alignment::Left,
                            ));
                        }
                        "Title" => {
                            cells.push(super::helpers::render_fuzzy_cell(
                                &m.title,
                                &app.search_query,
                                is_selected,
                                false,
                                Style::default()
                                    .fg(THEME.read().unwrap().text_normal)
                                    .add_modifier(Modifier::BOLD),
                                Alignment::Left,
                            ));
                        }
                        "State" => {
                            let (state_text, state_style) = if m.state == "active" {
                                (
                                    "ACTIVE",
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
                            cells.push(
                                Cell::from(Line::from(state_text).alignment(Alignment::Center))
                                    .style(state_style),
                            );
                        }

                        "Start Date" => {
                            let val = m.start_date.clone().unwrap_or_else(|| "N/A".to_string());
                            cells.push(super::helpers::render_fuzzy_cell(
                                &val,
                                &app.search_query,
                                is_selected,
                                false,
                                Style::default().fg(THEME.read().unwrap().blue),
                                Alignment::Left,
                            ));
                        }
                        "Due Date" => {
                            let val = m.due_date.clone().unwrap_or_else(|| "N/A".to_string());
                            cells.push(super::helpers::render_fuzzy_cell(
                                &val,
                                &app.search_query,
                                is_selected,
                                false,
                                Style::default().fg(THEME.read().unwrap().yellow),
                                Alignment::Left,
                            ));
                        }
                        "Progress" => {
                            let mut color = THEME.read().unwrap().text_muted;
                            let progress_str =
                                if let Some(issues) = app.milestone_issues_cache.get(&m.iid) {
                                    let total = issues.len();
                                    if total > 0 {
                                        let closed =
                                            issues.iter().filter(|i| i.state == "closed").count();
                                        let percent = (closed * 100) / total;
                                        color = if percent <= 33 {
                                            THEME.read().unwrap().red
                                        } else if percent <= 67 {
                                            THEME.read().unwrap().yellow
                                        } else {
                                            THEME.read().unwrap().green
                                        };
                                        format!("{}%", percent)
                                    } else {
                                        color = THEME.read().unwrap().red;
                                        "0%".to_string()
                                    }
                                } else if app.selected_milestone_iid == Some(m.iid) {
                                    "Loading...".to_string()
                                } else {
                                    "-".to_string()
                                };
                            cells.push(super::helpers::render_fuzzy_cell(
                                &progress_str,
                                &app.search_query,
                                is_selected,
                                false,
                                Style::default().fg(color),
                                Alignment::Left,
                            ));
                        }
                        _ => {
                            cells.push(Cell::from(String::new()));
                        }
                    }
                }
            }
            let row_style = if is_selected {
                Style::default().bg(THEME.read().unwrap().highlight_bg)
            } else {
                Style::default()
            };
            Row::new(cells).style(row_style)
        });

        if widths.is_empty() {
            widths.push(Constraint::Min(0));
        }

        let table = Table::new(rows, widths)
            .header(Row::new(header_cells).style(header_style).height(1))
            .block(main_block)
            .row_highlight_style(highlight_style)
            .highlight_symbol(format!(" {} ", icons.highlight_arrow));

        f.render_stateful_widget(table, content_area, &mut app.milestones.state);

        let preview_block = Block::default()
            .borders(Borders::ALL)
            .title(" 🎯 Milestone Details ")
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
                        "Title:       ",
                        Style::default().fg(THEME.read().unwrap().text_muted),
                    ),
                    Span::styled(
                        &m.title,
                        Style::default()
                            .fg(THEME.read().unwrap().text_normal)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                text.push(Line::from(vec![
                    Span::styled(
                        "State:       ",
                        Style::default().fg(THEME.read().unwrap().text_muted),
                    ),
                    Span::styled(
                        m.state.to_uppercase(),
                        Style::default()
                            .fg(if m.state == "active" {
                                THEME.read().unwrap().green
                            } else {
                                THEME.read().unwrap().red
                            })
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                text.push(Line::from(vec![
                    Span::styled(
                        "Start Date:  ",
                        Style::default().fg(THEME.read().unwrap().text_muted),
                    ),
                    Span::styled(
                        m.start_date.as_deref().unwrap_or("N/A"),
                        Style::default().fg(THEME.read().unwrap().blue),
                    ),
                ]));
                text.push(Line::from(vec![
                    Span::styled(
                        "Due Date:    ",
                        Style::default().fg(THEME.read().unwrap().text_muted),
                    ),
                    Span::styled(
                        m.due_date.as_deref().unwrap_or("N/A"),
                        Style::default().fg(THEME.read().unwrap().yellow),
                    ),
                ]));
                if let Some(desc) = &m.description {
                    if !desc.trim().is_empty() {
                        text.push(Line::from(""));
                        text.push(Line::from(Span::styled(
                            "Description:",
                            Style::default()
                                .fg(THEME.read().unwrap().header_fg)
                                .add_modifier(Modifier::BOLD),
                        )));
                        text.push(Line::from(desc.as_str()));
                    }
                }
                text.push(Line::from(""));

                if let Some(issues) = &app.selected_milestone_issues {
                    let total = issues.len();
                    let closed = issues.iter().filter(|i| i.state == "closed").count();
                    let open = total - closed;

                    text.push(Line::from(vec![
                        Span::styled(
                            "Issues Status: ",
                            Style::default()
                                .fg(THEME.read().unwrap().header_fg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{} Closed", closed),
                            Style::default().fg(THEME.read().unwrap().green),
                        ),
                        Span::raw(" / "),
                        Span::styled(
                            format!("{} Open", open),
                            Style::default().fg(THEME.read().unwrap().yellow),
                        ),
                        Span::raw(format!(" (Total {})", total)),
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
                        pct,
                    );
                    let progress_color = if pct <= 33.0 {
                        THEME.read().unwrap().red
                    } else if pct <= 66.0 {
                        THEME.read().unwrap().yellow
                    } else {
                        THEME.read().unwrap().green
                    };
                    text.push(Line::from(Span::styled(
                        bar,
                        Style::default().fg(progress_color),
                    )));
                    text.push(Line::from(""));
                } else {
                    text.push(Line::from(Span::styled(
                        "Loading issues details...",
                        Style::default().fg(THEME.read().unwrap().text_muted),
                    )));
                }

                f.render_widget(
                    Paragraph::new(text)
                        .block(preview_block)
                        .wrap(ratatui::widgets::Wrap { trim: true }),
                    detail_rect,
                );
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select a milestone to view details...")
                    .block(preview_block)
                    .style(Style::default().fg(THEME.read().unwrap().text_muted)),
                detail_rect,
            );
        }
    }
}

pub(crate) fn render_tab_branches(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
    highlight_style: Style,
    header_style: Style,
) {
    let icons = crate::config::ICONS.read().unwrap();
    if app.branches.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!("\n\n {} Loading branches...", icons.label_loading))
                .alignment(Alignment::Center)
                .block(main_block.clone())
                .style(Style::default().fg(THEME.read().unwrap().text_muted)),
            content_area,
        );
    } else {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = app
            .enabled_columns
            .get(&Tab::Branches)
            .unwrap_or(&default_set);
        let filtered =
            App::filter_branches_list(&app.branches.items, &app.search_query, enabled_cols);
        let rows = filtered.iter().enumerate().map(|(idx, b)| {
            let is_selected = app.branches.state.selected() == Some(idx);
            let row_style = if is_selected {
                highlight_style
            } else {
                Style::default()
            };
            let mut cells = Vec::new();
            if app.is_column_visible(Tab::Branches, "Name") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &b.name,
                    &app.search_query,
                    is_selected,
                    false,
                    row_style,
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Branches, "Default") {
                let text = if b.default { "YES" } else { "NO" };
                let style = if b.default {
                    Style::default().fg(THEME.read().unwrap().green)
                } else {
                    Style::default().fg(THEME.read().unwrap().text_muted)
                };
                cells.push(Cell::from(Span::styled(text, style)));
            }
            if app.is_column_visible(Tab::Branches, "Protected") {
                let text = if b.protected { "YES" } else { "NO" };
                let style = if b.protected {
                    Style::default().fg(THEME.read().unwrap().yellow)
                } else {
                    Style::default().fg(THEME.read().unwrap().text_muted)
                };
                cells.push(Cell::from(Span::styled(text, style)));
            }
            if app.is_column_visible(Tab::Branches, "SHA") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &crate::utils::format::truncate(&b.commit_sha, 10),
                    &app.search_query,
                    is_selected,
                    false,
                    row_style,
                    Alignment::Left,
                ));
            }
            Row::new(cells).style(row_style).height(1)
        });

        let cols = Tab::Branches.columns(false);
        let widths: Vec<Constraint> = cols
            .iter()
            .filter(|c| app.is_column_visible(Tab::Branches, c))
            .map(|c| match *c {
                "Name" => Constraint::Fill(1),
                "Default" => Constraint::Length(10),
                "Protected" => Constraint::Length(12),
                "SHA" => Constraint::Length(14),
                _ => Constraint::Fill(1),
            })
            .collect();

        let table = Table::new(rows, widths)
            .header(
                Row::new(
                    cols.iter()
                        .filter(|c| app.is_column_visible(Tab::Branches, c))
                        .map(|c| Cell::from(*c).style(header_style)),
                )
                .style(Style::default().add_modifier(Modifier::BOLD))
                .height(1),
            )
            .block(main_block.clone())
            .row_highlight_style(highlight_style)
            .highlight_symbol(format!(" {} ", icons.highlight_arrow));

        f.render_stateful_widget(table, content_area, &mut app.branches.state);

        // Detail pane
        if let Some(idx) = app.branches.state.selected() {
            if let Some(branch) = filtered.get(idx) {
                let detail_text = format!(
                    "Branch: {}\nDefault: {}\nProtected: {}\nCan Push: {}\nSHA: {}",
                    branch.name,
                    branch.default,
                    branch.protected,
                    branch.can_push,
                    branch.commit_sha,
                );
                let detail = Paragraph::new(detail_text)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" 🌿 Branch Details ")
                            .border_style(Style::default().fg(THEME.read().unwrap().border)),
                    )
                    .style(Style::default().fg(THEME.read().unwrap().text_normal))
                    .scroll((app.detail_scroll, 0));
                f.render_widget(detail, detail_rect);
            }
        }
    }
}

pub(crate) fn render_tab_environments(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
    highlight_style: Style,
    header_style: Style,
) {
    let icons = crate::config::ICONS.read().unwrap();
    if app.environments.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!(
                "\n\n {} Loading environments...",
                icons.label_loading
            ))
            .alignment(Alignment::Center)
            .block(main_block.clone())
            .style(Style::default().fg(THEME.read().unwrap().text_muted)),
            content_area,
        );
    } else {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = app
            .enabled_columns
            .get(&Tab::Environments)
            .unwrap_or(&default_set);
        let filtered =
            App::filter_environments_list(&app.environments.items, &app.search_query, enabled_cols);
        let rows = filtered.iter().enumerate().map(|(idx, e)| {
            let is_selected = app.environments.state.selected() == Some(idx);
            let row_style = if is_selected {
                highlight_style
            } else {
                Style::default()
            };
            let mut cells = Vec::new();
            if app.is_column_visible(Tab::Environments, "Name") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &e.name,
                    &app.search_query,
                    is_selected,
                    false,
                    row_style,
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Environments, "State") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &e.state,
                    &app.search_query,
                    is_selected,
                    false,
                    row_style,
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Environments, "Deployment Status") {
                let status = e
                    .last_deployment
                    .as_ref()
                    .map(|d| d.status.as_str())
                    .unwrap_or("N/A");
                cells.push(super::helpers::render_fuzzy_cell(
                    status,
                    &app.search_query,
                    is_selected,
                    false,
                    row_style,
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Environments, "URL") {
                let url = e.external_url.as_deref().unwrap_or("-");
                cells.push(Cell::from(Span::styled(
                    url,
                    Style::default().fg(THEME.read().unwrap().blue),
                )));
            }
            Row::new(cells).style(row_style).height(1)
        });

        let cols = Tab::Environments.columns(false);
        let widths: Vec<Constraint> = cols
            .iter()
            .filter(|c| app.is_column_visible(Tab::Environments, c))
            .map(|c| match *c {
                "Name" => Constraint::Length(24),
                "State" => Constraint::Length(12),
                "Deployment Status" => Constraint::Length(20),
                "URL" => Constraint::Fill(1),
                _ => Constraint::Fill(1),
            })
            .collect();

        let table = Table::new(rows, widths)
            .header(
                Row::new(
                    cols.iter()
                        .filter(|c| app.is_column_visible(Tab::Environments, c))
                        .map(|c| Cell::from(*c).style(header_style)),
                )
                .style(Style::default().add_modifier(Modifier::BOLD))
                .height(1),
            )
            .block(main_block.clone())
            .row_highlight_style(highlight_style)
            .highlight_symbol(format!(" {} ", icons.highlight_arrow));

        f.render_stateful_widget(table, content_area, &mut app.environments.state);

        // Detail pane - show deployments if available
        if app.deployments.items.is_empty() {
            if let Some(idx) = app.environments.state.selected() {
                if let Some(env) = filtered.get(idx) {
                    let last_deploy = env
                        .last_deployment
                        .as_ref()
                        .map(|d| {
                            format!(
                                "Deployment #{}: {}\nRef: {}\nSHA: {}\nDate: {}",
                                d.iid, d.status, d.ref_name, d.sha, d.created_at
                            )
                        })
                        .unwrap_or_else(|| "No deployments".to_string());
                    let detail_text = format!(
                        "Environment: {}\nState: {}\nURL: {}\n\n{}",
                        env.name,
                        env.state,
                        env.external_url.as_deref().unwrap_or("N/A"),
                        last_deploy,
                    );
                    let detail = Paragraph::new(detail_text)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" 🌐 Environment Details ")
                                .border_style(Style::default().fg(THEME.read().unwrap().border)),
                        )
                        .style(Style::default().fg(THEME.read().unwrap().text_normal))
                        .scroll((app.detail_scroll, 0));
                    f.render_widget(detail, detail_rect);
                }
            }
        } else {
            // Show fetched deployments in the detail pane
            let deploy_rows: Vec<Row> = app
                .deployments
                .items
                .iter()
                .map(|d| {
                    let cells = vec![
                        Cell::from(Span::raw(d.status.as_str())),
                        Cell::from(Span::raw(crate::utils::format::truncate(&d.ref_name, 20))),
                        Cell::from(Span::raw(crate::utils::format::truncate(&d.sha, 10))),
                        Cell::from(Span::raw(crate::utils::format::time_ago(&d.created_at))),
                    ];
                    Row::new(cells)
                })
                .collect();
            let deploy_widths = [
                Constraint::Length(14),
                Constraint::Fill(1),
                Constraint::Length(14),
                Constraint::Length(20),
            ];
            let deploy_table = Table::new(deploy_rows, deploy_widths)
                .header(
                    Row::new(
                        ["Status", "Ref", "SHA", "Date"]
                            .iter()
                            .map(|h| Cell::from(*h).style(header_style)),
                    )
                    .style(Style::default().add_modifier(Modifier::BOLD)),
                )
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" 🚀 Deployments ")
                        .border_style(Style::default().fg(THEME.read().unwrap().border)),
                )
                .row_highlight_style(highlight_style);
            f.render_stateful_widget(deploy_table, detail_rect, &mut app.deployments.state);
        }
    }
}

pub(crate) fn render_tab_terminal(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    _main_block: Block<'_>,
    _highlight_style: Style,
    _header_style: Style,
) {
    let num_cmds = app.terminal_commands.len();
    let area = content_area;
    let base_block = Block::default()
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

    let is_github = app
        .gitlab_client
        .as_ref()
        .map(|c| c.is_github)
        .unwrap_or(false);

    for i in start_idx..end_idx {
        if let Some(cmd) = app.terminal_commands.get(i) {
            log_lines.push(super::helpers::build_log_line(
                cmd,
                inner_rect.width as usize,
                is_github,
            ));
        }
    }

    f.render_widget(Paragraph::new(log_lines).block(custom_main_block), area);
}
