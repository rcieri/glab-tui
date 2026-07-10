#![allow(dead_code)]

mod diff;
mod helpers;
mod overlays;
mod tabs;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use self::diff::{centered_rect_min, format_comment_with_suggestions};
use self::helpers::build_log_line;
use self::overlays::render_overlays;
use crate::app::{App, Tab};
use crate::config::THEME;

pub fn render(f: &mut Frame, app: &mut App) {
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

    // Top: Title & Context
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
    let can_zoom = app.active_tab != Tab::Pipelines || !app.jobs.items.is_empty();

    let sidebar_width = if size.width >= 80 {
        Constraint::Length(22)
    } else {
        Constraint::Length(0)
    };

    let details_width = if !app.detail_visible || size.width < 90 {
        Constraint::Max(0)
    } else if size.width > 150 {
        Constraint::Percentage(35)
    } else if size.width > 100 {
        Constraint::Length(45)
    } else {
        Constraint::Length(30)
    };

    let middle_chunks_raw = if app.details_zoomed && can_zoom {
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
    let [sidebar_rect, content_rect, mut detail_rect] = [
        middle_chunks_raw[0],
        middle_chunks_raw[1],
        middle_chunks_raw[2],
    ];
    if !app.detail_visible {
        detail_rect = Rect::default();
    }

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
            .split(content_rect);
        (tc[0], tc[1])
    } else {
        (content_rect, Rect::default())
    };

    // Sidebar: Tabs
    let is_github = app
        .gitlab_client
        .as_ref()
        .map(|c| c.is_github)
        .unwrap_or(false);
    let sidebar_items: Vec<ListItem> = app
        .available_tabs()
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
    f.render_widget(sidebar, sidebar_rect);

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
        Tab::Issues => tabs::render_tab_issues(
            f,
            app,
            content_area,
            detail_rect,
            main_block.clone(),
            highlight_style,
            header_style,
        ),
        Tab::MergeRequests => tabs::render_tab_merge_requests(
            f,
            app,
            content_area,
            detail_rect,
            main_block.clone(),
            highlight_style,
            header_style,
        ),
        Tab::Pipelines => tabs::render_tab_pipelines(
            f,
            app,
            content_area,
            detail_rect,
            main_block.clone(),
            highlight_style,
            header_style,
        ),
        Tab::Jobs => tabs::render_tab_jobs(
            f,
            app,
            content_area,
            detail_rect,
            main_block.clone(),
            highlight_style,
            header_style,
        ),
        Tab::Runners => tabs::render_tab_runners(
            f,
            app,
            content_area,
            detail_rect,
            main_block.clone(),
            highlight_style,
            header_style,
        ),
        Tab::Releases => tabs::render_tab_releases(
            f,
            app,
            content_area,
            detail_rect,
            main_block.clone(),
            highlight_style,
            header_style,
        ),
        Tab::Todos => tabs::render_tab_todos(
            f,
            app,
            content_area,
            detail_rect,
            main_block.clone(),
            highlight_style,
            header_style,
        ),
        Tab::Milestones => tabs::render_tab_milestones(
            f,
            app,
            content_area,
            detail_rect,
            main_block.clone(),
            highlight_style,
            header_style,
        ),
        Tab::Branches => tabs::render_tab_branches(
            f,
            app,
            content_area,
            detail_rect,
            main_block.clone(),
            highlight_style,
            header_style,
        ),
        Tab::Environments => tabs::render_tab_environments(
            f,
            app,
            content_area,
            detail_rect,
            main_block.clone(),
            highlight_style,
            header_style,
        ),
        Tab::Terminal => tabs::render_tab_terminal(
            f,
            app,
            content_area,
            detail_rect,
            main_block,
            highlight_style,
            header_style,
        ),
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
                    log_lines.push(build_log_line(cmd, bottom_inner.width as usize, is_github));
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

                let gutter_bg = THEME.read().unwrap().gutter_bg;
                let marker_style = Style::default()
                    .fg(THEME.read().unwrap().yellow)
                    .add_modifier(Modifier::BOLD)
                    .bg(gutter_bg);
                let num_style = Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .bg(gutter_bg);
                let sep_style = Style::default()
                    .fg(THEME.read().unwrap().separator_fg)
                    .bg(gutter_bg);

                let sel_bg = if in_selection {
                    Some(THEME.read().unwrap().selection_bg)
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
                            let theme = THEME.read().unwrap();
                            let code_fg = theme.diff_del_fg;
                            let code_bg = theme.diff_del_bg;
                            let prefix_fg = theme.diff_del_prefix;
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
                            let theme = THEME.read().unwrap();
                            let code_fg = theme.diff_add_fg;
                            let code_bg = theme.diff_add_bg;
                            let prefix_fg = theme.diff_add_prefix;
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
                        .bg(THEME.read().unwrap().draft_comment_bg);

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
                        .bg(THEME.read().unwrap().comment_bg);

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

                let gutter_bg = THEME.read().unwrap().gutter_bg;

                let marker_style = Style::default()
                    .fg(THEME.read().unwrap().yellow)
                    .add_modifier(Modifier::BOLD)
                    .bg(gutter_bg);

                let num_style = Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .bg(gutter_bg);

                let sep_style = Style::default()
                    .fg(THEME.read().unwrap().separator_fg)
                    .bg(gutter_bg);

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
                    Some(THEME.read().unwrap().selection_bg)
                } else {
                    None
                };

                match line.line_type {
                    crate::app::DiffLineType::Addition | crate::app::DiffLineType::Deletion => {
                        let theme = THEME.read().unwrap();
                        let is_add = line.line_type == crate::app::DiffLineType::Addition;
                        let code_fg = if is_add {
                            theme.diff_add_fg
                        } else {
                            theme.diff_del_fg
                        };
                        let code_bg = if is_add {
                            theme.diff_add_bg
                        } else {
                            theme.diff_del_bg
                        };
                        let prefix_fg = if is_add {
                            theme.diff_add_prefix
                        } else {
                            theme.diff_del_prefix
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

                // COMMENTS OVERLAY (unified)
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
                        .bg(THEME.read().unwrap().draft_comment_bg);

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
                        .bg(THEME.read().unwrap().comment_bg);

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
                        Style::default().fg(THEME.read().unwrap().separator_fg),
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

    render_overlays(f, app, size);
}
