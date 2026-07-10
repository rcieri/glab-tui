use super::diff::{centered_rect_fixed, centered_rect_min};
use super::helpers::{get_label_color, highlight_fuzzy_match};
use crate::app::SaveMenu;
use crate::app::{App, Tab};
use crate::config::THEME;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table,
    },
};

pub(crate) fn render_overlays(f: &mut Frame, app: &mut App, size: Rect) {
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
        f.render_widget(ratatui::widgets::Clear, area);
        f.render_widget(block, area);

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
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(ratatui::layout::Alignment::Center);

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
                key: d(format!("{}", app.config.keybindings.issues.reply_comment)),
                action: "Reply to issue (open $EDITOR)",
            },
            Shortcut {
                category: "Issues",
                key: d(format!("{}", app.config.keybindings.issues.resolve_comment)),
                action: "Toggle resolve discussion",
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
                key: d(format!("{}", app.config.keybindings.mrs.reply_comment)),
                action: "Reply to MR (open $EDITOR)",
            },
            Shortcut {
                category: "Merge Requests",
                key: d(format!("{}", app.config.keybindings.mrs.resolve_comment)),
                action: "Toggle resolve discussion",
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
                key: s("m"),
                action: "Collapse / expand matrix jobs",
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
                category: "Branches",
                key: d(format!("{}", app.config.keybindings.branches.create_branch)),
                action: "Create new branch",
            },
            Shortcut {
                category: "Branches",
                key: d(format!("{}", app.config.keybindings.branches.delete_branch)),
                action: "Delete selected branch",
            },
            Shortcut {
                category: "Environments",
                key: d(format!(
                    "{}",
                    app.config.keybindings.environments.view_deployments
                )),
                action: "View deployments list for environment",
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
                Tab::Branches => &["Global & Nav", "Branches"],
                Tab::Environments => &["Global & Nav", "Environments"],
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
        let page_size_idx = order_end;
        let theme_start = page_size_idx + 1;
        let theme_end = theme_start + themes.len();
        let save_end = theme_end;

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
        constraints.push(Constraint::Length(1)); // PAGE SIZE header
        constraints.push(Constraint::Length(1)); // PAGE SIZE value
        constraints.push(Constraint::Length(1)); // spacer
        constraints.push(Constraint::Length(1)); // THEME header
        constraints.push(Constraint::Length(themes.len() as u16));
        constraints.push(Constraint::Length(1)); // spacer
        constraints.push(Constraint::Length(1)); // SAVE header
        constraints.push(Constraint::Length(1)); // SAVE button
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
                let is_selected =
                    app.group_by_column.get(&tab).cloned().flatten().as_deref() == Some(col);
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

        let order_header = Paragraph::new(" ORDER").style(
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
                let is_selected =
                    app.group_ascending.get(&tab).copied().unwrap_or(true) == (i == 0);
                let text = format!(" {} {}", if is_selected { "◉" } else { "○" }, label);
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

        // Page Size
        let page_size_header = Paragraph::new(" PAGE SIZE").style(
            Style::default()
                .fg(THEME.read().unwrap().header_fg)
                .add_modifier(Modifier::BOLD),
        );
        f.render_widget(page_size_header, popup_layout[chunk_idx]);
        chunk_idx += 1;

        let is_page_size_active = active_idx == page_size_idx;
        let page_size_text = if app.editing_page_size {
            format!("   [ {}| ]", app.page_size_input)
        } else if is_page_size_active {
            format!("   [ {} ]", app.page_size)
        } else {
            format!("   {}", app.page_size)
        };
        let page_size_style = if app.editing_page_size {
            Style::default()
                .fg(THEME.read().unwrap().bg)
                .bg(THEME.read().unwrap().green)
                .add_modifier(Modifier::BOLD)
        } else if is_page_size_active {
            Style::default()
                .fg(THEME.read().unwrap().bg)
                .bg(THEME.read().unwrap().border_focused)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(THEME.read().unwrap().text_normal)
        };
        let page_size_paragraph = Paragraph::new(page_size_text)
            .style(page_size_style)
            .alignment(Alignment::Center);
        f.render_widget(page_size_paragraph, popup_layout[chunk_idx]);
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
                let flat_idx = theme_start + i;
                let is_selected = app.config.theme_preset.as_deref().unwrap_or("default") == *name;
                let text = format!(" {} {}", if is_selected { "◉" } else { "○" }, name);
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

        chunk_idx += 1; // spacer

        // Save button
        let save_header = Paragraph::new(" SAVE").style(
            Style::default()
                .fg(THEME.read().unwrap().header_fg)
                .add_modifier(Modifier::BOLD),
        );
        f.render_widget(save_header, popup_layout[chunk_idx]);
        chunk_idx += 1;

        let is_save_selected = active_idx == save_end;
        let save_button_text = if is_save_selected {
            " > Save View <"
        } else {
            "   Save View"
        };
        let save_button_style = if is_save_selected {
            Style::default()
                .fg(THEME.read().unwrap().bg)
                .bg(THEME.read().unwrap().border_focused)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(THEME.read().unwrap().text_normal)
        };
        let save_button = Paragraph::new(save_button_text)
            .style(save_button_style)
            .alignment(Alignment::Center);
        f.render_widget(save_button, popup_layout[chunk_idx]);
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

        // Save submenu
        if app.save_menu_open {
            let submenu_height = 7;
            let submenu_width = 30;
            let submenu_area = centered_rect_fixed(submenu_width, submenu_height, size);
            let submenu_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
                .title(" Save to Config ")
                .title_style(
                    Style::default()
                        .fg(THEME.read().unwrap().border_focused)
                        .add_modifier(Modifier::BOLD),
                );
            f.render_widget(Clear, submenu_area);
            f.render_widget(submenu_block.clone(), submenu_area);
            let submenu_inner = submenu_block.inner(submenu_area);

            let options = ["Local Repo", "Global", "Cancel"];
            let submenu_items: Vec<ListItem> = options
                .iter()
                .enumerate()
                .map(|(i, &label)| {
                    let is_active = match app.save_menu_selection {
                        Some(SaveMenu::Local) => i == 0,
                        Some(SaveMenu::Global) => i == 1,
                        Some(SaveMenu::Cancel) => i == 2,
                        None => false,
                    };
                    let style = if is_active {
                        Style::default()
                            .fg(THEME.read().unwrap().bg)
                            .bg(THEME.read().unwrap().border_focused)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(THEME.read().unwrap().text_normal)
                    };
                    ListItem::new(label).style(style)
                })
                .collect();

            let mut submenu_state = ListState::default();
            submenu_state.select(Some(match app.save_menu_selection {
                Some(SaveMenu::Local) => 0,
                Some(SaveMenu::Global) => 1,
                Some(SaveMenu::Cancel) => 2,
                None => 0,
            }));

            f.render_stateful_widget(
                List::new(submenu_items)
                    .highlight_style(Style::default().add_modifier(Modifier::BOLD)),
                submenu_inner,
                &mut submenu_state,
            );
        }
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
            crate::app::ConfirmAction::DeleteBranch(branch_name) => (
                " Delete Branch? ",
                format!("Are you sure you want to delete branch '{}'?", branch_name),
            ),
            crate::app::ConfirmAction::CloseIssue(iid) => (
                " Close Issue? ",
                format!("Are you sure you want to close issue #{}?", iid),
            ),
            crate::app::ConfirmAction::CloseMr(iid) => (
                " Close Merge Request? ",
                format!("Are you sure you want to close MR/PR #{}?", iid),
            ),
            crate::app::ConfirmAction::MergeMr(iid) => (
                " Merge Request? ",
                format!("Are you sure you want to merge MR/PR #{}?", iid),
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

        let footer_p = Paragraph::new(Line::from(vec![
            Span::styled(
                "     [ YES ]     ",
                Style::default()
                    .fg(if app.confirm_popup_selected_yes {
                        THEME.read().unwrap().bg
                    } else {
                        THEME.read().unwrap().border_focused
                    })
                    .bg(if app.confirm_popup_selected_yes {
                        THEME.read().unwrap().border_focused
                    } else {
                        Color::Reset
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(
                "     [ NO ]     ",
                Style::default()
                    .fg(if !app.confirm_popup_selected_yes {
                        THEME.read().unwrap().bg
                    } else {
                        THEME.read().unwrap().border_focused
                    })
                    .bg(if !app.confirm_popup_selected_yes {
                        THEME.read().unwrap().border_focused
                    } else {
                        Color::Reset
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .alignment(Alignment::Center);

        f.render_widget(Clear, area);
        f.render_widget(block, area);
        f.render_widget(message_p, chunks[0]);
        f.render_widget(footer_p, chunks[1]);
    }
}
