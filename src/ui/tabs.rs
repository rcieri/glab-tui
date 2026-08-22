use crate::app::{App, Tab};
use crate::config::THEME;
use crate::utils::format::{format_ref, time_ago, truncate};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

/// Return a responsive column width: uses `base` normally but shrinks on narrow terminals.
fn col_w(content_width: u16, base: u16) -> Constraint {
    if content_width >= 120 {
        Constraint::Length(base)
    } else if content_width >= 90 {
        Constraint::Length(base.min(14))
    } else {
        Constraint::Length(base.min(10))
    }
}

pub(crate) fn render_tab_issues(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
    highlight_style: Style,
    header_style: Style,
) {
    let theme = THEME.read().unwrap();
    if super::render_edit_menu_if_active(f, app, detail_rect) {
        return;
    }
    let icons = crate::config::ICONS.read().unwrap();
    if app.issues.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!("\n\n {} Loading issues...", icons.label_loading))
                .alignment(Alignment::Center)
                .block(main_block.clone())
                .style(Style::default().fg(theme.text_muted)),
            content_area,
        );
        f.render_widget(
            Paragraph::new("Select an item to view preview...")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} Preview ", icons.label_details))
                        .border_style(Style::default().fg(theme.border)),
                )
                .style(Style::default().fg(theme.text_muted)),
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
            let is_checked = app.selected_issues.contains(&i.iid);
            let (state_text, state_style) = if i.state == "opened" {
                (
                    format!("{} OPEN", icons.state_open),
                    Style::default()
                        .fg(theme.green)
                        .bg(if is_selected {
                            theme.highlight_bg
                        } else {
                            theme.green_bg
                        })
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                (
                    format!("{} CLOSED", icons.state_closed),
                    Style::default()
                        .fg(theme.red)
                        .bg(if is_selected {
                            theme.highlight_bg
                        } else {
                            theme.red_bg
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
                    Style::default().fg(theme.text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Issues, "State") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &state_text,
                    &app.search_query,
                    is_selected,
                    is_checked,
                    state_style,
                    Alignment::Center,
                ));
            }
            if app.is_column_visible(Tab::Issues, "Title") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&i.title, 100),
                    &app.search_query,
                    is_selected,
                    is_checked,
                    Style::default().fg(theme.text_normal),
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
                    is_checked,
                    Style::default().fg(theme.blue),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Issues, "Labels") {
                cells.push(super::helpers::render_labels_cell(
                    &i.labels,
                    &app.label_colors,
                    &app.search_query,
                    is_selected,
                    is_checked,
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
                    is_checked,
                    Style::default().fg(theme.yellow),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Issues, "Due Date") {
                let due_str = i.due_date.as_deref().unwrap_or("—");
                cells.push(super::helpers::render_fuzzy_cell(
                    due_str,
                    &app.search_query,
                    is_selected,
                    is_checked,
                    Style::default().fg(theme.yellow),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Issues, "Author") {
                let author_str = format!("@{}", i.author.username);
                cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&author_str, 15),
                    &app.search_query,
                    is_selected,
                    is_checked,
                    Style::default().fg(theme.blue),
                    Alignment::Left,
                ));
            }
            let row_style = if is_selected {
                Style::default().bg(THEME.read().unwrap().highlight_bg)
            } else {
                Style::default()
            };
            // yazi-style leftmost selection bar: a 1-wide colored stripe on
            // selected rows instead of highlighting the whole row.
            let bar_cell = Cell::from(" ").style(if is_checked {
                Style::default().bg(THEME.read().unwrap().checked_bg)
            } else {
                Style::default()
            });
            cells.insert(0, bar_cell);
            Row::new(cells).style(row_style).height(1)
        });

        let mut header_cells = Vec::new();
        let mut widths = Vec::new();

        header_cells.push(Cell::from(""));
        widths.push(Constraint::Length(1));

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
            widths.push(col_w(content_area.width, 22));
        }
        if app.is_column_visible(Tab::Issues, "Labels") {
            header_cells.push(Cell::from("Labels"));
            widths.push(col_w(content_area.width, 26));
        }
        if app.is_column_visible(Tab::Issues, "Milestone") {
            header_cells.push(Cell::from("Milestone"));
            widths.push(col_w(content_area.width, 18));
        }
        if app.is_column_visible(Tab::Issues, "Due Date") {
            header_cells.push(Cell::from("Due Date"));
            widths.push(col_w(content_area.width, 20));
        }
        if app.is_column_visible(Tab::Issues, "Author") {
            header_cells.push(Cell::from("Author"));
            widths.push(col_w(content_area.width, 18));
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
            .border_style(Style::default().fg(theme.border))
            .title(format!(" {} Preview ", icons.label_details))
            .title_style(
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::BOLD),
            );
        let selected_issue_idx = app.issues.state.selected();
        if let Some(selected) = selected_issue_idx {
            if let Some(issue) = filtered_issues.get(selected) {
                let is_github = app.is_github();
                let doc = crate::entity_editor::build_issue_document(issue, is_github);
                super::inspector::render_entity_inspector(
                    f,
                    &doc,
                    detail_rect,
                    super::inspector::InspectorMode::ReadOnly {
                        scroll: app.detail_scroll,
                        title_suffix: "",
                    },
                    &app.label_colors,
                );
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select an item to view preview...")
                    .block(preview_block)
                    .style(Style::default().fg(theme.text_muted)),
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
    let theme = THEME.read().unwrap();
    if super::render_edit_menu_if_active(f, app, detail_rect) {
        return;
    }
    let icons = crate::config::ICONS.read().unwrap();
    if app.mrs.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!(
                "\n\n {} Loading merge requests...",
                icons.label_loading
            ))
            .alignment(Alignment::Center)
            .block(main_block.clone())
            .style(Style::default().fg(theme.text_muted)),
            content_area,
        );
        f.render_widget(
            Paragraph::new("Select an item to view preview...")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} Preview ", icons.label_details))
                        .border_style(Style::default().fg(theme.border)),
                )
                .style(Style::default().fg(theme.text_muted)),
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
            let is_checked = app.selected_mrs.contains(&m.iid);
            let (prefix, clean_title) = crate::utils::format::parse_mr_title_prefix(&m.title);

            let (state_text, state_style) = if m.state == "opened" {
                (
                    format!("{} OPEN", icons.state_open),
                    Style::default()
                        .fg(theme.green)
                        .bg(if is_selected {
                            theme.highlight_bg
                        } else {
                            theme.green_bg
                        })
                        .add_modifier(Modifier::BOLD),
                )
            } else if m.state == "merged" {
                (
                    format!("{} MERGED", icons.state_merged),
                    Style::default()
                        .fg(theme.purple)
                        .bg(if is_selected {
                            theme.highlight_bg
                        } else {
                            theme.purple_bg
                        })
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                (
                    format!("{} CLOSED", icons.state_closed),
                    Style::default()
                        .fg(theme.red)
                        .bg(if is_selected {
                            theme.highlight_bg
                        } else {
                            theme.red_bg
                        })
                        .add_modifier(Modifier::BOLD),
                )
            };

            let (status_styled, status_style) = if m.draft {
                (
                    format!("{} DRAFT", icons.status_draft),
                    Style::default()
                        .fg(theme.yellow)
                        .bg(if is_selected {
                            theme.highlight_bg
                        } else {
                            theme.yellow_bg
                        })
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                let upper = prefix.to_uppercase();
                if upper == "WIP" || upper == "DRAFT" {
                    (
                        format!("{} DRAFT", icons.status_draft),
                        Style::default()
                            .fg(theme.yellow)
                            .bg(if is_selected {
                                theme.highlight_bg
                            } else {
                                theme.yellow_bg
                            })
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    (
                        format!("{} READY", icons.approval_approved),
                        Style::default()
                            .fg(theme.green)
                            .bg(if is_selected {
                                theme.highlight_bg
                            } else {
                                theme.green_bg
                            })
                            .add_modifier(Modifier::BOLD),
                    )
                }
            };

            let status_styled = format!(
                "{}{}",
                status_styled,
                crate::domain::mr_state::status_flags(m.blocking_discussions_resolved)
            );

            let mut cells = Vec::new();
            if app.is_column_visible(Tab::MergeRequests, "ID") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &format!("!{}", m.iid),
                    &app.search_query,
                    is_selected,
                    false,
                    Style::default().fg(theme.text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::MergeRequests, "State") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &state_text,
                    &app.search_query,
                    is_selected,
                    is_checked,
                    state_style,
                    Alignment::Center,
                ));
            }
            if app.is_column_visible(Tab::MergeRequests, "Status") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &status_styled,
                    &app.search_query,
                    is_selected,
                    is_checked,
                    status_style,
                    Alignment::Center,
                ));
            }
            if app.is_column_visible(Tab::MergeRequests, "Mergeable") {
                let (text, tone) = crate::domain::mr_state::mergeable_cell(m.mergeability.as_ref());
                let style = {
                    let t = &theme;
                    match tone {
                        crate::domain::mr_state::MergeTone::Conflict => Style::default()
                            .fg(t.red)
                            .bg(if is_selected {
                                t.highlight_bg
                            } else {
                                t.red_bg
                            })
                            .add_modifier(Modifier::BOLD),
                        crate::domain::mr_state::MergeTone::Rebase => Style::default()
                            .fg(t.yellow)
                            .bg(if is_selected {
                                t.highlight_bg
                            } else {
                                t.yellow_bg
                            })
                            .add_modifier(Modifier::BOLD),
                        crate::domain::mr_state::MergeTone::Clean => Style::default()
                            .fg(t.green)
                            .bg(if is_selected {
                                t.highlight_bg
                            } else {
                                t.green_bg
                            })
                            .add_modifier(Modifier::BOLD),
                        crate::domain::mr_state::MergeTone::Computing => Style::default()
                            .fg(t.blue)
                            .bg(if is_selected {
                                t.highlight_bg
                            } else {
                                t.blue_bg
                            })
                            .add_modifier(Modifier::BOLD),
                        _ => Style::default().fg(t.text_muted),
                    }
                };
                cells.push(super::helpers::render_fuzzy_cell(
                    &text,
                    &app.search_query,
                    is_selected,
                    is_checked,
                    style,
                    Alignment::Center,
                ));
            }
            if app.is_column_visible(Tab::MergeRequests, "Approval") {
                let (text, tone) =
                    crate::domain::mr_state::approval_cell(m.approval.as_ref(), app.is_github());
                let style = {
                    let t = &theme;
                    match tone {
                        crate::domain::mr_state::ApprovalTone::ChangesRequested => Style::default()
                            .fg(t.red)
                            .bg(if is_selected {
                                t.highlight_bg
                            } else {
                                t.red_bg
                            })
                            .add_modifier(Modifier::BOLD),
                        crate::domain::mr_state::ApprovalTone::AwaitingYou => Style::default()
                            .fg(t.yellow)
                            .bg(if is_selected {
                                t.highlight_bg
                            } else {
                                t.yellow_bg
                            })
                            .add_modifier(Modifier::BOLD),
                        crate::domain::mr_state::ApprovalTone::Approved => Style::default()
                            .fg(t.green)
                            .bg(if is_selected {
                                t.highlight_bg
                            } else {
                                t.green_bg
                            })
                            .add_modifier(Modifier::BOLD),
                        crate::domain::mr_state::ApprovalTone::Pending => Style::default()
                            .fg(t.yellow)
                            .bg(if is_selected {
                                t.highlight_bg
                            } else {
                                t.yellow_bg
                            })
                            .add_modifier(Modifier::BOLD),
                        _ => Style::default().fg(t.text_muted),
                    }
                };
                cells.push(super::helpers::render_fuzzy_cell(
                    &text,
                    &app.search_query,
                    is_selected,
                    is_checked,
                    style,
                    Alignment::Center,
                ));
            }
            if app.is_column_visible(Tab::MergeRequests, "Title") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&clean_title, 100),
                    &app.search_query,
                    is_selected,
                    is_checked,
                    Style::default().fg(theme.text_normal),
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
                    is_checked,
                    Style::default().fg(theme.blue),
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
                    is_checked,
                    Style::default().fg(theme.blue),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::MergeRequests, "Workflow") {
                let text = crate::domain::mr_state::workflow_cell(m.workflow);
                let (color, bg_color) = {
                    let t = &theme;
                    match m.workflow {
                        Some(crate::domain::mr_state::WorkflowStatus::ReturnedToYou) => {
                            (t.red, Some(t.red_bg))
                        }
                        Some(crate::domain::mr_state::WorkflowStatus::ReviewRequested) => {
                            (t.yellow, Some(t.yellow_bg))
                        }
                        Some(crate::domain::mr_state::WorkflowStatus::YourMergeRequest) => {
                            (t.blue, Some(t.blue_bg))
                        }
                        Some(crate::domain::mr_state::WorkflowStatus::ApprovedByYou) => {
                            (t.green, Some(t.green_bg))
                        }
                        _ => (t.text_muted, None),
                    }
                };
                let mut wf_style = Style::default().fg(color);
                if let Some(bg) = bg_color {
                    let t = &theme;
                    wf_style = wf_style
                        .bg(if is_selected { t.highlight_bg } else { bg })
                        .add_modifier(Modifier::BOLD);
                }
                cells.push(super::helpers::render_fuzzy_cell(
                    &text,
                    &app.search_query,
                    is_selected,
                    is_checked,
                    wf_style,
                    Alignment::Center,
                ));
            }
            if app.is_column_visible(Tab::MergeRequests, "Labels") {
                cells.push(super::helpers::render_labels_cell(
                    &m.labels,
                    &app.label_colors,
                    &app.search_query,
                    is_selected,
                    is_checked,
                    24,
                ));
            }
            let is_github = app.is_github();
            if app.is_column_visible(
                Tab::MergeRequests,
                if is_github { "Action" } else { "Pipeline" },
            ) {
                let resolved_pipe = m.head_pipeline.as_ref().or_else(|| {
                    if is_github {
                        app.pipelines
                            .items
                            .iter()
                            .find(|p| p.ref_branch() == m.source_branch)
                    } else {
                        None
                    }
                });
                if let Some(pipe) = resolved_pipe {
                    let stages_dots = if let Some(jobs) = app.pipeline_jobs.get(&pipe.id()) {
                        super::helpers::get_stages_dots(jobs)
                    } else {
                        icons.label_loading.clone()
                    };

                    if stages_dots.is_empty() {
                        let (pipe_text, pipe_color, pipe_bg) = match pipe.status() {
                            "success" => (
                                format!("{} SUCCESS", icons.status_success),
                                theme.green,
                                theme.green_bg,
                            ),
                            "failed" => (
                                format!("{} FAILED", icons.status_failed),
                                theme.red,
                                theme.red_bg,
                            ),
                            "running" => (
                                format!("{} RUNNING", icons.status_running),
                                theme.blue,
                                theme.blue_bg,
                            ),
                            "canceled" => (
                                format!("{} CANCEL", icons.status_canceled),
                                theme.text_muted,
                                theme.inactive_bg,
                            ),
                            "pending" => (
                                format!("{} PENDING", icons.status_pending),
                                theme.yellow,
                                theme.yellow_bg,
                            ),
                            "skipped" => (
                                format!("{} SKIP", icons.status_skipped),
                                theme.text_muted,
                                theme.inactive_bg,
                            ),
                            _ => (
                                format!("{} UNKNOWN", icons.status_unknown),
                                theme.text_muted,
                                theme.inactive_bg,
                            ),
                        };
                        let bg = if is_selected {
                            theme.highlight_bg
                        } else {
                            pipe_bg
                        };
                        cells.push(super::helpers::render_fuzzy_cell(
                            &pipe_text,
                            &app.search_query,
                            is_selected,
                            is_checked,
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
                            is_checked,
                            Style::default().fg(theme.text_normal),
                            Alignment::Left,
                        ));
                    }
                } else {
                    cells.push(super::helpers::render_fuzzy_cell(
                        "—",
                        &app.search_query,
                        is_selected,
                        is_checked,
                        Style::default().fg(theme.text_muted),
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
                    is_checked,
                    Style::default().fg(theme.yellow),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::MergeRequests, "Author") {
                let author_str = format!("@{}", m.author.username);
                cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&author_str, 15),
                    &app.search_query,
                    is_selected,
                    is_checked,
                    Style::default().fg(theme.blue),
                    Alignment::Left,
                ));
            }
            let row_style = if is_selected {
                Style::default().bg(theme.highlight_bg)
            } else {
                Style::default()
            };
            // yazi-style leftmost selection bar: a 1-wide colored stripe on
            // selected rows instead of highlighting the whole row.
            let bar_cell = Cell::from(" ").style(if is_checked {
                Style::default().bg(theme.checked_bg)
            } else {
                Style::default()
            });
            cells.insert(0, bar_cell);
            Row::new(cells).style(row_style).height(1)
        });

        let mut header_cells = Vec::new();
        let mut widths = Vec::new();

        header_cells.push(Cell::from(""));
        widths.push(Constraint::Length(1));

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
        if app.is_column_visible(Tab::MergeRequests, "Mergeable") {
            header_cells.push(Cell::from(
                Line::from("Mergeable").alignment(Alignment::Center),
            ));
            widths.push(col_w(content_area.width, 13));
        }
        if app.is_column_visible(Tab::MergeRequests, "Approval") {
            header_cells.push(Cell::from(
                Line::from("Approval").alignment(Alignment::Center),
            ));
            widths.push(col_w(content_area.width, 12));
        }
        if app.is_column_visible(Tab::MergeRequests, "Title") {
            header_cells.push(Cell::from("Title"));
            widths.push(Constraint::Fill(1));
        }
        if app.is_column_visible(Tab::MergeRequests, "Assignees") {
            header_cells.push(Cell::from("Assignees"));
            widths.push(col_w(content_area.width, 22));
        }
        if app.is_column_visible(Tab::MergeRequests, "Reviewers") {
            header_cells.push(Cell::from("Reviewers"));
            widths.push(col_w(content_area.width, 22));
        }
        if app.is_column_visible(Tab::MergeRequests, "Workflow") {
            header_cells.push(Cell::from(
                Line::from("Workflow").alignment(Alignment::Center),
            ));
            widths.push(col_w(content_area.width, 13));
        }
        if app.is_column_visible(Tab::MergeRequests, "Labels") {
            header_cells.push(Cell::from("Labels"));
            widths.push(col_w(content_area.width, 26));
        }
        let is_github = app.is_github();
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
            widths.push(col_w(content_area.width, 18));
        }
        if app.is_column_visible(Tab::MergeRequests, "Author") {
            header_cells.push(Cell::from("Author"));
            widths.push(col_w(content_area.width, 18));
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
            .border_style(Style::default().fg(theme.border))
            .title(format!(" {} Preview ", icons.label_details))
            .title_style(
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::BOLD),
            );
        if let Some(selected) = app.mrs.state.selected() {
            if let Some(mr) = filtered_mrs.get(selected) {
                let is_github = app.is_github();
                let unresolved_threads = if is_github {
                    None
                } else if app.diff_view.as_ref().map(|d| d.mr_iid) == Some(mr.iid) {
                    Some(
                        app.current_comments
                            .iter()
                            .filter(|c| {
                                c.resolvable.unwrap_or(false) && !c.resolved.unwrap_or(false)
                            })
                            .count(),
                    )
                } else {
                    None
                };

                let doc =
                    crate::entity_editor::build_mr_document(mr, is_github, unresolved_threads);
                super::inspector::render_entity_inspector(
                    f,
                    &doc,
                    detail_rect,
                    super::inspector::InspectorMode::ReadOnly {
                        scroll: app.detail_scroll,
                        title_suffix: "",
                    },
                    &app.label_colors,
                );
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select an item to view preview...")
                    .block(preview_block)
                    .style(Style::default().fg(theme.text_muted)),
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
    let theme = THEME.read().unwrap();
    if super::render_edit_menu_if_active(f, app, detail_rect) {
        return;
    }
    let icons = crate::config::ICONS.read().unwrap();
    let is_github = app.is_github();
    if app.pipelines.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(if is_github {
                format!("\n\n {} Loading actions...", icons.label_loading)
            } else {
                format!("\n\n {} Loading pipelines...", icons.label_loading)
            })
            .alignment(Alignment::Center)
            .block(main_block.clone())
            .style(Style::default().fg(theme.text_muted)),
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
                    .title(format!(" {} Preview ", icons.label_details))
                    .border_style(Style::default().fg(theme.border)),
            )
            .style(Style::default().fg(theme.text_muted)),
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
            App::pipeline_filter_values,
        );

        let rows = filtered_pipelines.iter().enumerate().map(|(idx, p)| {
            let is_row_highlighted = app.pipelines.state.selected() == Some(idx);
            let (status_text, status_color, bg_color) = match p.status() {
                "success" => (
                    format!("{} SUCCESS", icons.status_success),
                    theme.green,
                    theme.green_bg,
                ),
                "failed" => (
                    format!("{} FAILED", icons.status_failed),
                    theme.red,
                    theme.red_bg,
                ),
                "running" => (
                    format!("{} RUNNING", icons.status_running),
                    theme.blue,
                    theme.blue_bg,
                ),
                "canceled" => (
                    format!("{} CANCEL", icons.status_canceled),
                    theme.text_muted,
                    theme.inactive_bg,
                ),
                "pending" => (
                    format!("{} PENDING", icons.status_pending),
                    theme.yellow,
                    theme.yellow_bg,
                ),
                "skipped" => (
                    format!("{} SKIP", icons.status_skipped),
                    theme.text_muted,
                    theme.inactive_bg,
                ),
                "manual" => (
                    format!("{} MANUAL", icons.status_manual),
                    theme.text_muted,
                    theme.inactive_bg,
                ),
                _ => (
                    format!("{} UNKNOWN", icons.status_unknown),
                    theme.text_muted,
                    theme.inactive_bg,
                ),
            };
            let stages_dots = if let Some(jobs) = app.pipeline_jobs.get(&p.id()) {
                super::helpers::get_stages_dots(jobs)
            } else {
                icons.label_loading.clone()
            };
            let is_checked = app.selected_pipelines.contains(&p.id());
            let status_bg = if is_row_highlighted {
                theme.highlight_bg
            } else if is_checked {
                theme.checked_bg
            } else {
                bg_color
            };
            let mut row_cells = Vec::new();
            if app.is_column_visible(Tab::Pipelines, "ID") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &format!("#{}", p.id()),
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
                    Style::default().fg(theme.text_normal),
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
                    Style::default().fg(theme.text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Pipelines, "Source") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(p.source().unwrap_or_default(), 18),
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
                    Style::default().fg(theme.text_muted),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Pipelines, "Name") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(p.name(), 30),
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
                    Style::default().fg(theme.text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Pipelines, "Event") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(p.event(), 15),
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
                    Style::default().fg(theme.text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Pipelines, "SHA") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(p.head_sha(), 8),
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
                    Style::default().fg(theme.text_muted),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Pipelines, "Actor") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(p.actor_login(), 20),
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
                    Style::default().fg(theme.text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Pipelines, "Created") {
                let created_str = p.created_at().map(|c| time_ago(c)).unwrap_or_default();
                row_cells.push(Cell::from(Span::styled(
                    truncate(&created_str, 15),
                    Style::default().fg(theme.yellow),
                )));
            }
            if app.is_column_visible(Tab::Pipelines, "Duration") {
                let duration = p
                    .duration_seconds()
                    .map(|d| format!("{}m {}s", d / 60, d % 60))
                    .unwrap_or_else(|| "-".to_string());
                row_cells.push(Cell::from(Span::styled(
                    duration,
                    Style::default().fg(theme.text_normal),
                )));
            }
            if app.is_column_visible(Tab::Pipelines, "Ref") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&format_ref(p.ref_branch()), 100),
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
                    Style::default().fg(theme.purple),
                    Alignment::Left,
                ));
            }
            let row_style = if is_row_highlighted {
                Style::default().bg(theme.highlight_bg)
            } else if is_checked {
                Style::default().bg(theme.checked_bg)
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
        if app.is_column_visible(Tab::Pipelines, "Source") {
            header_cells.push(Cell::from("Source"));
            widths.push(Constraint::Length(16));
        }
        if app.is_column_visible(Tab::Pipelines, "Name") {
            header_cells.push(Cell::from("Name"));
            widths.push(Constraint::Length(22));
        }
        if app.is_column_visible(Tab::Pipelines, "Event") {
            header_cells.push(Cell::from("Event"));
            widths.push(Constraint::Length(14));
        }
        if app.is_column_visible(Tab::Pipelines, "SHA") {
            header_cells.push(Cell::from("SHA"));
            widths.push(Constraint::Length(8));
        }
        if app.is_column_visible(Tab::Pipelines, "Actor") {
            header_cells.push(Cell::from("Actor"));
            widths.push(Constraint::Length(18));
        }
        if app.is_column_visible(Tab::Pipelines, "Created") {
            header_cells.push(Cell::from("Created"));
            widths.push(Constraint::Length(15));
        }
        if app.is_column_visible(Tab::Pipelines, "Duration") {
            header_cells.push(Cell::from("Duration"));
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
            .title(format!(" {} Preview ", icons.label_details))
            .title_style(
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(Style::default().fg(theme.border));
        if let Some(selected) = app.pipelines.state.selected() {
            if let Some(p) = filtered_pipelines.get(selected) {
                let jobs = app.pipeline_jobs.get(&p.id()).cloned().unwrap_or_default();
                let doc = crate::entity_editor::build_pipeline_document(p, &jobs);
                super::inspector::render_entity_inspector(
                    f,
                    &doc,
                    detail_rect,
                    super::inspector::InspectorMode::ReadOnly {
                        scroll: app.detail_scroll,
                        title_suffix: "",
                    },
                    &app.label_colors,
                );
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select an item to view preview...")
                    .block(preview_block)
                    .style(Style::default().fg(theme.text_muted)),
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
    let theme = THEME.read().unwrap();
    let icons = crate::config::ICONS.read().unwrap();
    if app.jobs.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!("\n\n {} Loading jobs...", icons.label_loading))
                .alignment(Alignment::Center)
                .block(main_block.clone())
                .style(Style::default().fg(theme.text_muted)),
            content_area,
        );
        f.render_widget(
            Paragraph::new("Select a job to view preview...")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} Preview ", icons.label_details))
                        .border_style(Style::default().fg(theme.border)),
                )
                .style(Style::default().fg(theme.text_muted)),
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
                "ID" => vec![item.id().to_string()],
                "Stage" => vec![item.stage().to_string()],
                "Status" => vec![item.status().to_string()],
                "Name" => vec![item.name().to_string()],
                "Matrix" => vec![item.matrix().map(|m| m.to_string()).unwrap_or_default()],
                "Runner" => vec![item.runner().unwrap_or("-").to_string()],
                "Needs" => item.needs().to_vec(),
                "Duration" => vec![
                    item.duration_seconds()
                        .map(|d| d.to_string())
                        .unwrap_or_default(),
                ],
                _ => vec![],
            },
        );

        let rows = filtered_jobs.iter().enumerate().map(|(i, j)| {
            let (matrix_display, status_text_display, status_color_display, status_bg_display) = {
                let (status_text, status_color, bg_color) = match j.status() {
                    "success" => (
                        format!("{} SUCCESS", icons.status_success),
                        theme.green,
                        theme.green_bg,
                    ),
                    "failed" => (
                        format!("{} FAILED", icons.status_failed),
                        theme.red,
                        theme.red_bg,
                    ),
                    "running" => (
                        format!("{} RUNNING", icons.status_running),
                        theme.blue,
                        theme.blue_bg,
                    ),
                    "canceled" => (
                        format!("{} CANCEL", icons.status_canceled),
                        theme.text_muted,
                        theme.inactive_bg,
                    ),
                    "pending" => (
                        format!("{} PENDING", icons.status_pending),
                        theme.yellow,
                        theme.yellow_bg,
                    ),
                    "skipped" => (
                        format!("{} SKIP", icons.status_skipped),
                        theme.text_muted,
                        theme.inactive_bg,
                    ),
                    "manual" => (
                        format!("{} MANUAL", icons.status_manual),
                        theme.text_muted,
                        theme.inactive_bg,
                    ),
                    "preparing" => (
                        format!("{} PREPARE", icons.status_pending),
                        theme.yellow,
                        theme.yellow_bg,
                    ),
                    _ => (
                        format!("{} UNKNOWN", icons.status_unknown),
                        theme.text_muted,
                        theme.inactive_bg,
                    ),
                };
                let m_str = if let Some(m) = j.matrix() {
                    format!("{} [{}]", icons.matrix_variant, m)
                } else {
                    String::new()
                };
                (m_str, status_text, status_color, bg_color)
            };

            let is_job_selected = Some(i) == app.jobs.state.selected();
            let is_checked = app.selected_jobs.contains(&j.id());
            let status_bg = if is_job_selected {
                theme.highlight_bg
            } else if is_checked {
                theme.checked_bg
            } else {
                status_bg_display
            };

            let matrix_str = matrix_display;
            let status_text = status_text_display;
            let status_color = status_color_display;
            let mut row_cells = Vec::new();
            if app.is_column_visible(Tab::Jobs, "ID") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &format!("#{}", j.id()),
                    &app.search_query,
                    is_job_selected,
                    is_checked,
                    Style::default().fg(theme.text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Jobs, "Stage") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &j.stage(),
                    &app.search_query,
                    is_job_selected,
                    is_checked,
                    Style::default().fg(theme.purple),
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
                    &j.name(),
                    &app.search_query,
                    is_job_selected,
                    is_checked,
                    Style::default().fg(theme.text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Jobs, "Matrix") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &matrix_str,
                    &app.search_query,
                    is_job_selected,
                    is_checked,
                    Style::default().fg(theme.text_muted),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Jobs, "Runner") {
                let runner_str = j.runner().unwrap_or("-");
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(runner_str, 20),
                    &app.search_query,
                    is_job_selected,
                    is_checked,
                    Style::default().fg(theme.text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Jobs, "Needs") {
                let needs_str = if j.needs().is_empty() {
                    "-".to_string()
                } else {
                    j.needs().join(", ")
                };
                row_cells.push(Cell::from(Span::styled(
                    truncate(&needs_str, 25),
                    Style::default().fg(theme.text_muted),
                )));
            }
            if app.is_column_visible(Tab::Jobs, "Duration") {
                let dur_str = match j.duration_seconds() {
                    Some(d) => format!("{}m {}s", d / 60, d % 60),
                    None => "-".to_string(),
                };
                row_cells.push(Cell::from(Span::styled(
                    dur_str,
                    Style::default().fg(theme.text_normal),
                )));
            }
            let row_style = if is_job_selected {
                Style::default().bg(theme.highlight_bg)
            } else if is_checked {
                Style::default().bg(theme.checked_bg)
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
        if app.is_column_visible(Tab::Jobs, "Runner") {
            header_cells.push(Cell::from("Runner"));
            widths.push(Constraint::Length(18));
        }
        if app.is_column_visible(Tab::Jobs, "Needs") {
            header_cells.push(Cell::from("Needs"));
            widths.push(Constraint::Length(14));
        }
        if app.is_column_visible(Tab::Jobs, "Duration") {
            header_cells.push(Cell::from("Duration"));
            widths.push(Constraint::Length(14));
        }

        if widths.is_empty() {
            widths.push(Constraint::Min(0));
        }

        let kind = app.kind();
        let is_github = kind.is_github();
        let jobs_title = Tab::Jobs.title(kind);
        let table = Table::new(rows, widths)
            .header(Row::new(header_cells).style(header_style).height(1))
            .block(main_block.clone().title(format!(" {} ", jobs_title)))
            .row_highlight_style(highlight_style)
            .highlight_symbol(format!(" {} ", icons.highlight_arrow));

        let mut state = app.jobs.state.clone();
        let filtered_count = filtered_jobs.len();
        if filtered_count > 0 {
            if let Some(sel) = state.selected() {
                if sel >= filtered_count {
                    state.select(Some(filtered_count.saturating_sub(1)));
                }
            }
        } else {
            state.select(None);
        }
        f.render_stateful_widget(table, content_area, &mut state);
        app.jobs.state = state;

        if app.job_trace_loading {
            let preview_block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} Preview ", icons.label_details))
                .title_style(
                    Style::default()
                        .fg(theme.text_muted)
                        .add_modifier(Modifier::BOLD),
                )
                .border_style(Style::default().fg(theme.border));

            f.render_widget(
                Paragraph::new("\n\n  Loading job trace... (Press Esc to cancel)")
                    .alignment(Alignment::Center)
                    .block(preview_block)
                    .style(Style::default().fg(theme.text_muted)),
                detail_rect,
            );
        } else if let Some(trace) = &app.job_trace {
            let width = detail_rect.width.saturating_sub(2) as usize;
            let height = detail_rect.height.saturating_sub(2) as usize;

            let formatted_lines = crate::utils::format::parse_ansi_trace(trace, &theme);

            let total_lines = if app.job_trace_wrap {
                let stripped = crate::utils::format::strip_ansi_escapes(trace);
                super::diff::count_wrapped_lines(&stripped, width)
            } else {
                formatted_lines.len()
            };

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

            let help_text = if app.job_trace_wrap {
                " Esc: Back | Enter: Zoom | j/k: Scroll | /: Search | f: Follow | w: No-wrap "
            } else {
                " Esc: Back | Enter: Zoom | j/k: Scroll | /: Search | f: Follow | w: Wrap "
            };
            let search_suffix = if app.job_trace_search_query.is_empty() {
                String::new()
            } else {
                format!(" [Search: {}]", app.job_trace_search_query)
            };
            let follow_suffix = if app.job_trace_follow {
                " [FOLLOW]"
            } else {
                ""
            };

            let preview_block = Block::default()
                .borders(Borders::ALL)
                .title(format!(
                    " Preview{}{}{} ",
                    title_suffix, search_suffix, follow_suffix
                ))
                .title_style(
                    Style::default()
                        .fg(theme.text_muted)
                        .add_modifier(Modifier::BOLD),
                )
                .title_bottom(
                    ratatui::text::Line::from(vec![Span::styled(
                        help_text,
                        Style::default().fg(theme.text_muted),
                    )])
                    .alignment(Alignment::Right),
                )
                .border_style(Style::default().fg(theme.border));

            let mut paragraph = Paragraph::new(formatted_lines)
                .block(preview_block)
                .scroll((app.detail_scroll, 0));

            if app.job_trace_wrap {
                paragraph = paragraph.wrap(ratatui::widgets::Wrap { trim: false });
            }

            f.render_widget(paragraph, detail_rect);
        } else {
            let preview_block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} Preview ", icons.label_details))
                .title_style(
                    Style::default()
                        .fg(theme.text_muted)
                        .add_modifier(Modifier::BOLD),
                )
                .border_style(Style::default().fg(theme.border));
            let mut text = Vec::new();

            // Show selected job metadata first
            if let Some(selected) = app.jobs.state.selected() {
                if let Some(j) = filtered_jobs.get(selected) {
                    let (status_text, status_color) = match j.status() {
                        "success" => ("success", theme.green),
                        "failed" => ("failed", theme.red),
                        "running" => ("running", theme.blue),
                        "canceled" => ("canceled", theme.text_muted),
                        "pending" => ("pending", theme.yellow),
                        "skipped" => ("skipped", theme.text_muted),
                        _ => ("unknown", theme.text_muted),
                    };
                    text.push(Line::from(vec![
                        Span::styled("Name:     ", Style::default().fg(theme.text_muted)),
                        Span::styled(
                            j.name().to_string(),
                            Style::default()
                                .fg(theme.text_normal)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                    text.push(Line::from(vec![
                        Span::styled("Status:   ", Style::default().fg(theme.text_muted)),
                        Span::styled(
                            status_text,
                            Style::default()
                                .fg(status_color)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                    if !is_github {
                        text.push(Line::from(vec![
                            Span::styled("Stage:    ", Style::default().fg(theme.text_muted)),
                            Span::styled(j.stage().to_string(), Style::default().fg(theme.purple)),
                        ]));
                    }
                    if let Some(matrix) = j.matrix() {
                        text.push(Line::from(vec![
                            Span::styled("Matrix:   ", Style::default().fg(theme.text_muted)),
                            Span::styled(matrix.to_string(), Style::default().fg(theme.text_muted)),
                        ]));
                    }
                    if let Some(runner) = j.runner() {
                        text.push(Line::from(vec![
                            Span::styled("Runner:   ", Style::default().fg(theme.text_muted)),
                            Span::styled(
                                runner.to_string(),
                                Style::default().fg(theme.text_normal),
                            ),
                        ]));
                    }
                    if let Some(dur) = j.duration_seconds() {
                        let mins = dur / 60;
                        let secs = dur % 60;
                        text.push(Line::from(vec![
                            Span::styled("Duration: ", Style::default().fg(theme.text_muted)),
                            Span::styled(
                                format!("{}m {}s", mins, secs),
                                Style::default().fg(theme.text_normal),
                            ),
                        ]));
                    }
                    if !j.needs().is_empty() {
                        text.push(Line::from(vec![
                            Span::styled("Needs:    ", Style::default().fg(theme.text_muted)),
                            Span::styled(j.needs().join(", "), Style::default().fg(theme.yellow)),
                        ]));
                    }
                    text.push(Line::from(""));
                }
            }

            let stage_label = if is_github {
                "Jobs Status:"
            } else {
                "Stages Success Rate:"
            };
            text.push(Line::from(vec![Span::styled(
                stage_label,
                Style::default()
                    .fg(theme.header_fg)
                    .add_modifier(Modifier::BOLD),
            )]));
            text.push(Line::from(""));
            if is_github {
                super::helpers::append_job_summaries(&mut text, &app.jobs.items);
            } else {
                super::helpers::append_stage_summaries(&mut text, &app.jobs.items);
            }
            f.render_widget(
                Paragraph::new(text)
                    .block(preview_block)
                    .scroll((app.detail_scroll, 0)),
                detail_rect,
            );
        }
    } else {
        f.render_widget(Paragraph::new("\n\n No jobs loaded.\n Press 'p' to manually enter a pipeline ID to fetch jobs for,\n or view a pipeline in Pipelines tab and press Enter.").alignment(Alignment::Center).block(main_block).style(Style::default().fg(theme.text_muted)), content_area);
        f.render_widget(
            Paragraph::new("Select a job to view preview...")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} Preview ", icons.label_details))
                        .border_style(Style::default().fg(theme.border)),
                )
                .style(Style::default().fg(theme.text_muted)),
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
    let theme = THEME.read().unwrap();
    let icons = crate::config::ICONS.read().unwrap();
    if app.runners.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!("\n\n {} Loading runners...", icons.label_loading))
                .alignment(Alignment::Center)
                .block(main_block.clone())
                .style(Style::default().fg(theme.text_muted)),
            content_area,
        );
        f.render_widget(
            Paragraph::new("Select a runner to view details...")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} Preview ", icons.label_details))
                        .border_style(Style::default().fg(theme.border)),
                )
                .style(Style::default().fg(theme.text_muted)),
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
                    theme.green,
                    theme.green_bg,
                ),
                "paused" => (
                    format!("{} PAUSED", icons.runner_paused),
                    theme.yellow,
                    theme.yellow_bg,
                ),
                "offline" => (
                    format!("{} OFFLINE", icons.runner_offline),
                    theme.red,
                    theme.red_bg,
                ),
                _ => (
                    format!("{} UNKNOWN", icons.status_unknown),
                    theme.text_muted,
                    theme.inactive_bg,
                ),
            };
            let desc = r.description.as_deref().unwrap_or("No description");
            let mut row_cells = Vec::new();
            if app.is_column_visible(Tab::Runners, "ID") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &format!("#{}", r.id),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(theme.text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Runners, "Description") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(desc, 100),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(theme.text_normal),
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
                            theme.highlight_bg
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
                    Style::default().fg(if r.active { theme.green } else { theme.red }),
                    Alignment::Left,
                ));
            }
            let row_style = if is_row_highlighted {
                Style::default().bg(theme.highlight_bg)
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
            .title(format!(" {} Preview ", icons.label_details))
            .title_style(
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(Style::default().fg(theme.border));
        if let Some(selected) = app.runners.state.selected() {
            if let Some(r) = filtered_runners.get(selected) {
                let doc = crate::entity_editor::build_runner_document(r);
                super::inspector::render_entity_inspector(
                    f,
                    &doc,
                    detail_rect,
                    super::inspector::InspectorMode::ReadOnly {
                        scroll: app.detail_scroll,
                        title_suffix: "",
                    },
                    &app.label_colors,
                );
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select an item to view preview...")
                    .block(preview_block)
                    .style(Style::default().fg(theme.text_muted)),
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
    let theme = THEME.read().unwrap();
    if super::render_edit_menu_if_active(f, app, detail_rect) {
        return;
    }
    let icons = crate::config::ICONS.read().unwrap();
    if app.releases.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!("\n\n {} Loading releases...", icons.label_loading))
                .alignment(Alignment::Center)
                .block(main_block.clone())
                .style(Style::default().fg(theme.text_muted)),
            content_area,
        );
        f.render_widget(
            Paragraph::new("Select a release to view details...")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} Preview ", icons.label_details))
                        .border_style(Style::default().fg(theme.border)),
                )
                .style(Style::default().fg(theme.text_muted)),
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
                        .fg(theme.green)
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
                    Style::default().fg(theme.text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Releases, "Date") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&r.released_at, 15),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(theme.yellow),
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
                    Style::default().fg(theme.blue),
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
                    Style::default().fg(theme.blue),
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
                    Style::default().fg(theme.text_muted),
                    Alignment::Left,
                ));
            }
            let row_style = if is_row_highlighted {
                Style::default().bg(theme.highlight_bg)
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
            .title(format!(" {} Preview ", icons.label_details))
            .title_style(
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(Style::default().fg(theme.border));
        if let Some(selected) = app.releases.state.selected() {
            if let Some(r) = filtered_releases.get(selected) {
                let doc = crate::entity_editor::build_release_document(r);
                super::inspector::render_entity_inspector(
                    f,
                    &doc,
                    detail_rect,
                    super::inspector::InspectorMode::ReadOnly {
                        scroll: app.detail_scroll,
                        title_suffix: "",
                    },
                    &app.label_colors,
                );
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select an item to view preview...")
                    .block(preview_block)
                    .style(Style::default().fg(theme.text_muted)),
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
    let theme = THEME.read().unwrap();
    let icons = crate::config::ICONS.read().unwrap();
    if app.todos.items.is_empty() {
        let entity_label = if app.kind().is_github() {
            "notifications"
        } else {
            "todos"
        };
        if app.loading_tabs.contains(&app.active_tab) {
            f.render_widget(
                Paragraph::new(format!(
                    "\n\n {} Loading {}...",
                    icons.label_loading, entity_label
                ))
                .alignment(Alignment::Center)
                .block(main_block.clone())
                .style(Style::default().fg(theme.text_muted)),
                content_area,
            );
        } else {
            f.render_widget(
                Paragraph::new(format!("\n\n {} No {}", icons.label_details, entity_label))
                    .alignment(Alignment::Center)
                    .block(main_block.clone())
                    .style(Style::default().fg(theme.text_muted)),
                content_area,
            );
        }
        f.render_widget(
            Paragraph::new("Select a todo...")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} Preview ", icons.label_details))
                        .border_style(Style::default().fg(theme.border)),
                )
                .style(Style::default().fg(theme.text_muted)),
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
                "Updated" => vec![crate::utils::format::time_ago(&item.updated_at)],
                _ => vec![],
            },
        );

        let rows = filtered_todos.iter().enumerate().map(|(idx, n)| {
            let is_row_highlighted = app.todos.state.selected() == Some(idx);

            let (state_str, state_style) = if n.state == "unread" || n.state == "pending" {
                (
                    " NEW",
                    Style::default()
                        .fg(theme.green)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                (" READ", Style::default().fg(theme.text_muted))
            };

            let type_style = if n.target_type == "MergeRequest" {
                Style::default()
                    .fg(theme.purple)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.blue).add_modifier(Modifier::BOLD)
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
                    Style::default().fg(theme.text_muted),
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
                let prefix = if n.target_type == "MergeRequest" {
                    "!"
                } else {
                    "#"
                };
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &format!("{}{}", prefix, n.target_iid),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(theme.blue),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Todos, "Title") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&n.title, 80),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(theme.text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Todos, "Updated") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &time_ago(&n.updated_at),
                    &app.search_query,
                    is_row_highlighted,
                    false,
                    Style::default().fg(theme.yellow),
                    Alignment::Left,
                ));
            }
            let row_style = if is_row_highlighted {
                Style::default().bg(theme.highlight_bg)
            } else {
                Style::default()
            };
            Row::new(row_cells).style(row_style).height(1)
        });

        let mut header_cells = Vec::new();
        let mut widths = Vec::new();

        if app.is_column_visible(Tab::Todos, "State") {
            header_cells.push(Cell::from(Line::from("State").alignment(Alignment::Center)));
            widths.push(Constraint::Length(6));
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
        if app.is_column_visible(Tab::Todos, "Updated") {
            header_cells.push(Cell::from("Updated"));
            widths.push(Constraint::Length(16));
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
            .title(format!(" {} Preview ", icons.label_details))
            .title_style(
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(Style::default().fg(theme.border));
        if let Some(selected) = app.todos.state.selected() {
            if let Some(n) = filtered_todos.get(selected) {
                let doc = crate::entity_editor::build_todo_document(n);
                super::inspector::render_entity_inspector(
                    f,
                    &doc,
                    detail_rect,
                    super::inspector::InspectorMode::ReadOnly {
                        scroll: app.detail_scroll,
                        title_suffix: "",
                    },
                    &app.label_colors,
                );
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select an item to view preview...")
                    .block(preview_block)
                    .style(Style::default().fg(theme.text_muted)),
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
    let theme = THEME.read().unwrap();
    if super::render_edit_menu_if_active(f, app, detail_rect) {
        return;
    }
    let icons = crate::config::ICONS.read().unwrap();
    if app.milestones.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!(
                "\n\n {} Loading milestones...",
                icons.label_loading
            ))
            .alignment(Alignment::Center)
            .block(main_block.clone())
            .style(Style::default().fg(theme.text_muted)),
            content_area,
        );
        f.render_widget(
            Paragraph::new("Select a milestone...")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} Preview ", icons.label_details))
                        .border_style(Style::default().fg(theme.border)),
                )
                .style(Style::default().fg(theme.text_muted)),
            detail_rect,
        );
    } else {
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
        let cols = Tab::Milestones.columns(app.kind());
        for col in &cols {
            if app.is_column_visible(Tab::Milestones, col) {
                header_cells.push(Cell::from(*col));
                match *col {
                    "ID" => widths.push(Constraint::Length(8)),
                    "Title" => widths.push(Constraint::Fill(1)),
                    "State" => widths.push(Constraint::Length(10)),
                    "Start Date" => widths.push(Constraint::Length(20)),
                    "Due Date" => widths.push(Constraint::Length(20)),
                    "Progress" => widths.push(Constraint::Length(18)),
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
                                &format!("%{}", m.iid),
                                &app.search_query,
                                is_selected,
                                false,
                                Style::default().fg(theme.text_normal),
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
                                    .fg(theme.text_normal)
                                    .add_modifier(Modifier::BOLD),
                                Alignment::Left,
                            ));
                        }
                        "State" => {
                            let (state_text, state_style) = if m.state == "active" {
                                (
                                    "ACTIVE",
                                    Style::default()
                                        .fg(theme.green)
                                        .bg(if is_selected {
                                            theme.highlight_bg
                                        } else {
                                            theme.green_bg
                                        })
                                        .add_modifier(Modifier::BOLD),
                                )
                            } else {
                                (
                                    "CLOSED",
                                    Style::default()
                                        .fg(theme.red)
                                        .bg(if is_selected {
                                            theme.highlight_bg
                                        } else {
                                            theme.red_bg
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
                                Style::default().fg(theme.blue),
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
                                Style::default().fg(theme.yellow),
                                Alignment::Left,
                            ));
                        }
                        "Progress" => {
                            let (bar_text, color) =
                                if let Some(issues) = app.milestone_issues_cache.get(&m.iid) {
                                    let total = issues.len();
                                    if total > 0 {
                                        let closed =
                                            issues.iter().filter(|i| i.state == "closed").count();
                                        let pct = (closed as f32 / total as f32) * 100.0;
                                        let color = if pct <= 33.0 {
                                            theme.red
                                        } else if pct <= 66.0 {
                                            theme.yellow
                                        } else {
                                            theme.green
                                        };
                                        let bar_segments = 10;
                                        let filled_len = (closed * bar_segments) / total;
                                        (
                                            format!(
                                                "[{}{}] {:.0}%",
                                                "█".repeat(filled_len),
                                                "░".repeat(bar_segments - filled_len),
                                                pct,
                                            ),
                                            color,
                                        )
                                    } else {
                                        ("[░░░░░░░░░░] 0%".to_string(), theme.red)
                                    }
                                } else if app.selected_milestone_iid == Some(m.iid) {
                                    ("Loading...".to_string(), theme.text_muted)
                                } else {
                                    ("-".to_string(), theme.text_muted)
                                };
                            cells.push(Cell::from(bar_text).style(Style::default().fg(color)));
                        }
                        _ => {
                            cells.push(Cell::from(String::new()));
                        }
                    }
                }
            }
            let row_style = if is_selected {
                Style::default().bg(theme.highlight_bg)
            } else {
                Style::default()
            };
            Row::new(cells).style(row_style).height(1)
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
            .title(format!(" {} Preview ", icons.label_milestone))
            .title_style(
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(Style::default().fg(theme.border));

        if let Some(selected_idx) = app.milestones.state.selected() {
            if let Some(m) = filtered_milestones.get(selected_idx) {
                let is_github = app.is_github();
                let issues = app
                    .selected_milestone_issues
                    .as_deref()
                    .or_else(|| app.milestone_issues_cache.get(&m.iid).map(|v| v.as_slice()));
                let doc = crate::entity_editor::build_milestone_document(m, issues, is_github);
                super::inspector::render_entity_inspector(
                    f,
                    &doc,
                    detail_rect,
                    super::inspector::InspectorMode::ReadOnly {
                        scroll: app.detail_scroll,
                        title_suffix: "",
                    },
                    &app.label_colors,
                );
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select an item to view preview...")
                    .block(preview_block)
                    .style(Style::default().fg(theme.text_muted)),
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
    let theme = THEME.read().unwrap();
    if super::render_edit_menu_if_active(f, app, detail_rect) {
        return;
    }
    let icons = crate::config::ICONS.read().unwrap();
    if app.branches.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!("\n\n {} Loading branches...", icons.label_loading))
                .alignment(Alignment::Center)
                .block(main_block.clone())
                .style(Style::default().fg(theme.text_muted)),
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
                    &format!("{} {}", icons.label_branch, b.name),
                    &app.search_query,
                    is_selected,
                    false,
                    row_style,
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Branches, "Default") {
                let cell = if b.default {
                    Cell::from(Span::styled(
                        format!(" {} YES ", icons.radio_on),
                        Style::default()
                            .fg(theme.green)
                            .bg(if is_selected {
                                theme.highlight_bg
                            } else {
                                theme.green_bg
                            })
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Cell::from(Span::styled(" NO ", Style::default().fg(theme.text_muted)))
                };
                cells.push(cell);
            }
            if app.is_column_visible(Tab::Branches, "Protected") {
                let cell = if b.protected {
                    Cell::from(Span::styled(
                        format!(" \u{f023} YES "),
                        Style::default()
                            .fg(theme.yellow)
                            .bg(if is_selected {
                                theme.highlight_bg
                            } else {
                                theme.yellow_bg
                            })
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Cell::from(Span::styled(" NO ", Style::default().fg(theme.text_muted)))
                };
                cells.push(cell);
            }
            if app.is_column_visible(Tab::Branches, "SHA") {
                let sha_text = if b.commit_sha.is_empty() {
                    "--".to_string()
                } else {
                    crate::utils::format::truncate(&b.commit_sha, 8)
                };
                cells.push(super::helpers::render_fuzzy_cell(
                    &sha_text,
                    &app.search_query,
                    is_selected,
                    false,
                    row_style,
                    Alignment::Left,
                ));
            }
            Row::new(cells).style(row_style).height(1)
        });

        let cols = Tab::Branches.columns(app.kind());
        let widths: Vec<Constraint> = cols
            .iter()
            .filter(|c| app.is_column_visible(Tab::Branches, c))
            .map(|c| match *c {
                "Name" => Constraint::Fill(1),
                "Default" => Constraint::Length(12),
                "Protected" => Constraint::Length(14),
                "SHA" => Constraint::Length(12),
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
                .height(1),
            )
            .block(main_block.clone())
            .row_highlight_style(highlight_style)
            .highlight_symbol(format!(" {} ", icons.highlight_arrow));

        f.render_stateful_widget(table, content_area, &mut app.branches.state);

        // Detail pane
        let preview_block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} Preview ", icons.label_branch))
            .title_style(
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(Style::default().fg(theme.border));
        if let Some(idx) = app.branches.state.selected() {
            if let Some(branch) = filtered.get(idx) {
                let doc = crate::entity_editor::build_branch_document(branch);
                super::inspector::render_entity_inspector(
                    f,
                    &doc,
                    detail_rect,
                    super::inspector::InspectorMode::ReadOnly {
                        scroll: app.detail_scroll,
                        title_suffix: "",
                    },
                    &app.label_colors,
                );
            } else {
                f.render_widget(Paragraph::new("").block(preview_block), detail_rect);
            }
        } else {
            f.render_widget(
                Paragraph::new("Select an item to view preview...")
                    .block(preview_block)
                    .style(Style::default().fg(theme.text_muted)),
                detail_rect,
            );
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
    let theme = THEME.read().unwrap();
    let icons = crate::config::ICONS.read().unwrap();
    if app.environments.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!(
                "\n\n {} Loading environments...",
                icons.label_loading
            ))
            .alignment(Alignment::Center)
            .block(main_block.clone())
            .style(Style::default().fg(theme.text_muted)),
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
                    Style::default().fg(theme.blue),
                )));
            }
            Row::new(cells).style(row_style).height(1)
        });

        let cols = Tab::Environments.columns(app.kind());
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
                .height(1),
            )
            .block(main_block.clone())
            .row_highlight_style(highlight_style)
            .highlight_symbol(format!(" {} ", icons.highlight_arrow));

        f.render_stateful_widget(table, content_area, &mut app.environments.state);

        // Detail pane - show deployments if available
        let preview_block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} Preview ", icons.label_environment))
            .title_style(
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(Style::default().fg(theme.border));
        if app.deployments.items.is_empty() {
            if let Some(idx) = app.environments.state.selected() {
                if let Some(env) = filtered.get(idx) {
                    let doc = crate::entity_editor::build_environment_document(env);
                    super::inspector::render_entity_inspector(
                        f,
                        &doc,
                        detail_rect,
                        super::inspector::InspectorMode::ReadOnly {
                            scroll: app.detail_scroll,
                            title_suffix: "",
                        },
                        &app.label_colors,
                    );
                }
            } else {
                f.render_widget(
                    Paragraph::new("Select an environment to view details...")
                        .block(preview_block)
                        .style(Style::default().fg(theme.text_muted)),
                    detail_rect,
                );
            }
        } else {
            // Show fetched deployments in the detail pane
            let deploy_rows: Vec<Row> = app
                .deployments
                .items
                .iter()
                .map(|d| {
                    let status_color = match d.status.as_str() {
                        "success" => theme.green,
                        "failed" => theme.red,
                        "running" => theme.blue,
                        "canceled" => theme.text_muted,
                        _ => theme.text_muted,
                    };
                    let cells = vec![
                        Cell::from(Span::styled(
                            d.status.as_str(),
                            Style::default()
                                .fg(status_color)
                                .add_modifier(Modifier::BOLD),
                        )),
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
                        .title(format!(" {} Deployments ", icons.label_deployment))
                        .border_style(Style::default().fg(theme.border)),
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
    let theme = THEME.read().unwrap();
    let num_cmds = app.terminal_commands.len();
    let area = content_area;
    let base_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.focus_column_checklist {
            theme.border
        } else {
            theme.border_focused
        }));
    let inner_rect = base_block.inner(area);
    let log_height = inner_rect.height as usize;
    let width = inner_rect.width as usize;

    let total_lines = if app.terminal_wrap {
        let full_text: String = app
            .terminal_commands
            .iter()
            .map(|cmd| {
                let line = super::helpers::build_log_line(cmd, usize::MAX);
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<Vec<&str>>()
                    .join("")
            })
            .collect::<Vec<String>>()
            .join("\n");
        super::diff::count_wrapped_lines(&full_text, width)
    } else {
        num_cmds
    };

    let max_scroll = total_lines.saturating_sub(log_height);
    app.terminal_scroll = app.terminal_scroll.min(max_scroll);

    let block_title = if app.terminal_wrap {
        format!(" Terminal (Wrap) [{} lines] ", total_lines)
    } else if app.terminal_scroll > 0 {
        format!(
            " Terminal (Scroll: {}/{}) ",
            app.terminal_scroll, max_scroll,
        )
    } else {
        " Terminal ".to_string()
    };
    let custom_main_block = base_block.clone().title(block_title).title_style(
        Style::default()
            .fg(theme.header_fg)
            .add_modifier(Modifier::BOLD),
    );

    if app.terminal_wrap {
        let all_lines: Vec<Line> = app
            .terminal_commands
            .iter()
            .map(|cmd| super::helpers::build_log_line(cmd, usize::MAX))
            .collect();

        let paragraph = Paragraph::new(all_lines)
            .block(custom_main_block)
            .scroll((app.terminal_scroll as u16, 0))
            .wrap(ratatui::widgets::Wrap { trim: false });

        f.render_widget(paragraph, area);
    } else {
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
                log_lines.push(super::helpers::build_log_line(
                    cmd,
                    inner_rect.width as usize,
                ));
            }
        }

        f.render_widget(Paragraph::new(log_lines).block(custom_main_block), area);
    }
}
