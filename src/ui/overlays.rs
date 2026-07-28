use super::diff::{centered_rect_fixed, centered_rect_min};
use super::helpers::{get_label_color, highlight_fuzzy_match};
use super::modal::modal_area;
use crate::app::SaveMenu;
use crate::app::{App, Tab};
use crate::config::{ICONS, THEME};
use crate::utils::format::render_markdown;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table,
    },
};
use textwrap;

pub(crate) fn render_overlays(f: &mut Frame, app: &mut App, size: Rect) {
    app.overlay_stack.clear();
    let icons = ICONS.read().unwrap();
    if let Some(menu) = &mut app.edit_menu {
        let is_new_entity = menu.is_new();
        let (body, edit_menu_area) = modal_area(f, &menu.title, 80, 85, 52, 16, size);
        app.overlay_stack
            .push((crate::app::OverlayKind::EditMenu, edit_menu_area));

        let label_width = menu
            .fields
            .iter()
            .map(|f| f.label.len())
            .max()
            .unwrap_or(18)
            .max(14);

        // Build left-pane items (fields — skip Description and its section header)
        let desc_value = menu
            .fields
            .iter()
            .find(|f| f.label == "Description" && f.kind == crate::app::FieldType::Text)
            .map(|f| f.value.clone())
            .unwrap_or_default();

        let is_desc_selected = menu.selected_idx < menu.fields.len()
            && menu.fields[menu.selected_idx].label == "Description"
            && menu.fields[menu.selected_idx].kind == crate::app::FieldType::Text;

        let field_items: Vec<ListItem> = menu
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                !(f.kind == crate::app::FieldType::Section
                    && f.label.to_uppercase() == "DESCRIPTION")
                    && !(f.kind == crate::app::FieldType::Text && f.label == "Description")
            })
            .map(|(i, f)| {
                let label = &f.label;
                let val = &f.value;
                let is_selected = i == menu.selected_idx;

                if f.kind == crate::app::FieldType::Section {
                    let available = body.width.saturating_sub(4) as usize / 2;
                    let label_text = format!(" {} ", label.to_uppercase());
                    let pad = (available.saturating_sub(label_text.len() + 2)) / 2;
                    let line = format!(
                        "{:\u{2500}>pad$} {} {:\u{2500}<pad$}",
                        "",
                        label_text,
                        "",
                        pad = pad
                    );
                    return ListItem::new(Line::from(Span::styled(
                        line,
                        Style::default()
                            .fg(THEME.read().unwrap().text_muted)
                            .add_modifier(Modifier::BOLD),
                    )));
                }

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
                if val.is_empty() && f.kind != crate::app::FieldType::Text {
                    val_spans.push(Span::styled(
                        " None",
                        Style::default()
                            .fg(THEME.read().unwrap().text_muted)
                            .bg(item_bg)
                            .add_modifier(Modifier::ITALIC),
                    ));
                } else if !val.is_empty() {
                    let max_val_width = (body.width.saturating_sub(6) / 2)
                        .saturating_sub(label_width as u16)
                        as usize;
                    let truncated = if val.len() > max_val_width.max(20) {
                        let cutoff = (max_val_width.max(20)).saturating_sub(3);
                        format!("{}...", &val[..cutoff.min(val.len())])
                    } else {
                        val.clone()
                    };
                    match f.kind {
                        crate::app::FieldType::Section => {}
                        crate::app::FieldType::MultiSelect => {
                            if val == "None" {
                                val_spans.push(Span::styled(
                                    " None",
                                    Style::default()
                                        .fg(THEME.read().unwrap().text_muted)
                                        .bg(item_bg)
                                        .add_modifier(Modifier::ITALIC),
                                ));
                            } else {
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
                                    let color = if label == "Labels" {
                                        get_label_color(trimmed)
                                    } else {
                                        THEME.read().unwrap().blue
                                    };
                                    let mut style = Style::default()
                                        .fg(color)
                                        .bg(item_bg)
                                        .add_modifier(Modifier::BOLD);
                                    if is_selected {
                                        style = style.add_modifier(Modifier::UNDERLINED);
                                    }
                                    val_spans.push(Span::styled(trimmed.to_string(), style));
                                }
                            }
                        }
                        crate::app::FieldType::Toggle => {
                            let (display, fg, bg) = match val.to_lowercase().as_str() {
                                "yes" | "true" | "confidential" => {
                                    (" YES ", THEME.read().unwrap().bg, THEME.read().unwrap().red)
                                }
                                "no" | "false" | "public" => (
                                    "  NO ",
                                    THEME.read().unwrap().bg,
                                    THEME.read().unwrap().green,
                                ),
                                "draft" => (
                                    " DRAFT ",
                                    THEME.read().unwrap().bg,
                                    THEME.read().unwrap().yellow,
                                ),
                                "ready" => (
                                    " READY ",
                                    THEME.read().unwrap().bg,
                                    THEME.read().unwrap().green,
                                ),
                                _ => (&val[..], THEME.read().unwrap().text_muted, Color::Reset),
                            };
                            val_spans.push(Span::styled(
                                display,
                                Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
                            ));
                        }
                        crate::app::FieldType::Date => {
                            let display = if val == "Set" {
                                "Not set".to_string()
                            } else {
                                val.clone()
                            };
                            val_spans.push(Span::styled(
                                format!(" {}", display),
                                Style::default()
                                    .fg(THEME.read().unwrap().yellow)
                                    .bg(item_bg)
                                    .add_modifier(Modifier::BOLD),
                            ));
                        }
                        crate::app::FieldType::Ref => {
                            val_spans.push(Span::styled(
                                format!(" {}", truncated),
                                Style::default()
                                    .fg(THEME.read().unwrap().purple)
                                    .bg(item_bg)
                                    .add_modifier(Modifier::BOLD),
                            ));
                        }
                        crate::app::FieldType::Text => {
                            let val_fg = match label.as_str() {
                                "Milestone" => THEME.read().unwrap().purple,
                                _ => THEME.read().unwrap().text_normal,
                            };
                            let mut style = Style::default().fg(val_fg).bg(item_bg);
                            if is_selected {
                                style = style.add_modifier(Modifier::BOLD);
                            }
                            if label == "Title" && is_selected && menu.editing {
                                let cursor_pos = menu.cursor_pos.min(val.len());
                                let before = val[..cursor_pos].to_string();
                                let at_cursor = val
                                    .chars()
                                    .nth(cursor_pos)
                                    .map(|ch| ch.to_string())
                                    .unwrap_or_else(|| " ".to_string());
                                let after = if cursor_pos < val.len() {
                                    val[cursor_pos + at_cursor.len()..].to_string()
                                } else {
                                    String::new()
                                };
                                val_spans.push(Span::styled(before, style));
                                val_spans.push(Span::styled(
                                    if at_cursor.is_empty() {
                                        " ".to_string()
                                    } else {
                                        at_cursor
                                    },
                                    Style::default().fg(THEME.read().unwrap().bg).bg(val_fg),
                                ));
                                val_spans.push(Span::styled(after, style));
                            } else {
                                val_spans.push(Span::styled(truncated, style));
                            }
                        }
                    }
                }

                let icon = match f.kind {
                    crate::app::FieldType::Section => "",
                    crate::app::FieldType::MultiSelect => icons.check_on.as_str(),
                    crate::app::FieldType::Toggle => icons.radio_on.as_str(),
                    crate::app::FieldType::Date => "\u{f073}",
                    crate::app::FieldType::Ref => icons.label_branch.as_str(),
                    crate::app::FieldType::Text => icons.label_details.as_str(),
                };
                let mut line_spans = vec![
                    Span::styled(
                        format!(
                            " {} {:label_width$} ",
                            icon,
                            label,
                            label_width = label_width
                        ),
                        label_style,
                    ),
                    Span::styled(format!(" {} ", icons.separator), sep_style),
                ];
                line_spans.extend(val_spans);
                let line = Line::from(line_spans);
                ListItem::new(line).style(Style::default().bg(item_bg))
            })
            .collect();

        // Build description preview lines for right pane
        let desc_lines: Vec<Line> = if desc_value.is_empty() {
            vec![Line::from(Span::styled(
                "Empty — press Enter to edit, Ctrl+E for editor",
                Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .add_modifier(Modifier::ITALIC),
            ))]
        } else {
            let md_lines = render_markdown(&desc_value);
            let wrap_width = (body.width.saturating_sub(8) / 2).saturating_sub(2) as usize;
            md_lines
                .into_iter()
                .flat_map(|l| {
                    let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                    let style = l.spans.first().map(|s| s.style).unwrap_or_default();
                    textwrap::wrap(&text, wrap_width.max(20))
                        .into_iter()
                        .map(|c| Line::from(Span::styled(c.into_owned(), style)))
                        .collect::<Vec<_>>()
                })
                .collect()
        };

        let submit_idx = menu.fields.len() + 1;

        // Layout: main content + submit
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(3)])
            .split(body);

        let main_area = layout[0];
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_area);

        // Left pane: fields list
        let fields_block = Block::default()
            .borders(Borders::RIGHT)
            .border_style(Style::default().fg(THEME.read().unwrap().border));
        let fields_inner = fields_block.inner(main_chunks[0]);
        f.render_widget(fields_block, main_chunks[0]);
        let list = List::new(field_items).style(Style::default().bg(Color::Reset));
        let mut state = menu.state.clone();
        f.render_stateful_widget(list, fields_inner, &mut state);
        menu.state = state;

        // Right pane: description preview
        let desc_border_color = if is_desc_selected {
            THEME.read().unwrap().border_focused
        } else {
            THEME.read().unwrap().border
        };
        let desc_block = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(desc_border_color))
            .title(format!(" {} Description ", icons.label_details))
            .title_style(
                Style::default()
                    .fg(if is_desc_selected {
                        THEME.read().unwrap().header_fg
                    } else {
                        THEME.read().unwrap().text_muted
                    })
                    .add_modifier(Modifier::BOLD),
            );
        f.render_widget(
            Paragraph::new(desc_lines)
                .block(desc_block)
                .wrap(ratatui::widgets::Wrap { trim: true }),
            main_chunks[1],
        );

        // Submit button
        {
            let submit_chunk = layout[1];
            let btn_text = if is_new_entity {
                format!(" {} Submit ", icons.check_on)
            } else {
                format!(" {} Save ", icons.check_on)
            };
            let is_submit_selected = menu.selected_idx == submit_idx;
            let submit_fg = if is_submit_selected {
                THEME.read().unwrap().bg
            } else {
                THEME.read().unwrap().green
            };
            let submit_bg = if is_submit_selected {
                THEME.read().unwrap().green
            } else {
                Color::Reset
            };
            let submit_block = Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(THEME.read().unwrap().text_muted));
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    btn_text,
                    Style::default()
                        .fg(submit_fg)
                        .bg(submit_bg)
                        .add_modifier(Modifier::BOLD),
                )]))
                .block(submit_block)
                .alignment(Alignment::Center),
                submit_chunk,
            );
        }
    }

    if app.column_filter_context.is_none() {
        if let Some(selector) = &mut app.selector {
            let (body, selector_area) = modal_area(f, &selector.title, 50, 60, 34, 6, size);
            app.overlay_stack
                .push((crate::app::OverlayKind::Selector, selector_area));

            let has_filter = selector.field_type != "comment_action_select"
                && selector.field_type != "review_submit_status"
                && selector.field_type != "merge_options";

            let constraints = if has_filter {
                vec![
                    Constraint::Length(3), // Search/Filter
                    Constraint::Min(0),    // List of items
                ]
            } else {
                vec![
                    Constraint::Min(0), // List of items
                ]
            };

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(constraints)
                .split(body);

            let (search_chunk, list_chunk) = if has_filter {
                (Some(chunks[0]), chunks[1])
            } else {
                (None, chunks[0])
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

                            let marker = if is_selected {
                                format!(" {} ", icons.check_on)
                            } else {
                                format!(" {} ", icons.check_off)
                            };
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
                    Constraint::Min(0), // Value input line
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

        f.render_widget(value_p, chunks[0]);
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
        app.overlay_stack
            .push((crate::app::OverlayKind::DatePicker, area));
        let inner_area = block.inner(area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(1), // Month/Year line
                    Constraint::Min(0),    // Grid of days
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

        f.render_widget(Clear, area);
        f.render_widget(block, area);
        f.render_widget(header_p, chunks[0]);
        f.render_widget(table, chunks[1]);
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
            // ── Global & Nav ──
            Shortcut {
                category: "Global & Nav",
                key: d(format!("{} / →", app.config.keybindings.global.next_tab)),
                action: "Next tab",
            },
            Shortcut {
                category: "Global & Nav",
                key: d(format!("{} / ←", app.config.keybindings.global.prev_tab)),
                action: "Previous tab",
            },
            Shortcut {
                category: "Global & Nav",
                key: d(format!("{}", app.config.keybindings.global.configure)),
                action: "Toggle columns config popup",
            },
            Shortcut {
                category: "Global & Nav",
                key: s("j / k / ↓ / ↑"),
                action: "Select item / Scroll page",
            },
            Shortcut {
                category: "Global & Nav",
                key: d(format!(
                    "{} / {}",
                    app.config.keybindings.global.scroll_down,
                    app.config.keybindings.global.scroll_up
                )),
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
                key: d(format!("{}", app.config.keybindings.global.global_search)),
                action: "Global search across all tabs",
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
                key: d(format!("{}", app.config.keybindings.global.save_view)),
                action: "Save view layout to config",
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
            // ── Issues ──
            Shortcut {
                category: "Issues",
                key: d(format!("{}", app.config.keybindings.issues.create_issue)),
                action: "Create new Issue",
            },
            Shortcut {
                category: "Issues",
                key: d(format!("{}", app.config.keybindings.issues.select_issue)),
                action: "Toggle issue selection (bulk edit with e)",
            },
            Shortcut {
                category: "Issues",
                key: d(format!("{}", app.config.keybindings.issues.edit_entity)),
                action: "Open parameter edit menu",
            },
            Shortcut {
                category: "Issues",
                key: d(format!("{}", app.config.keybindings.issues.close_entity)),
                action: "Close selected Issue",
            },
            Shortcut {
                category: "Issues",
                key: d(format!("{}", app.config.keybindings.issues.reopen_entity)),
                action: "Reopen selected Issue",
            },
            Shortcut {
                category: "Issues",
                key: d(format!("{}", app.config.keybindings.issues.delete_entity)),
                action: "Delete selected Issue",
            },
            Shortcut {
                category: "Issues",
                key: s("o"),
                action: "Open selected Issue in browser",
            },
            Shortcut {
                category: "Issues",
                key: d(format!("{}", app.config.keybindings.issues.create_mr)),
                action: "Create Merge Request from selected Issue",
            },
            // ── Merge Requests ──
            Shortcut {
                category: "Merge Requests",
                key: d(format!("{}", app.config.keybindings.mrs.create_mr)),
                action: "Create new Merge Request",
            },
            Shortcut {
                category: "Merge Requests",
                key: d(format!("{}", app.config.keybindings.mrs.select_mr)),
                action: "Toggle MR/PR selection (bulk edit with e)",
            },
            Shortcut {
                category: "Merge Requests",
                key: d(format!("{}", app.config.keybindings.mrs.edit_entity)),
                action: "Open parameter edit menu",
            },
            Shortcut {
                category: "Merge Requests",
                key: d(format!("{}", app.config.keybindings.mrs.approve_mr)),
                action: "Approve selected MR",
            },
            Shortcut {
                category: "Merge Requests",
                key: d(format!("{}", app.config.keybindings.mrs.merge_mr)),
                action: "Merge selected MR (configure squash/delete)",
            },
            Shortcut {
                category: "Merge Requests",
                key: d(format!("{}", app.config.keybindings.mrs.toggle_draft)),
                action: "Toggle Draft / Ready status",
            },
            Shortcut {
                category: "Merge Requests",
                key: d(format!("{}", app.config.keybindings.mrs.view_diff)),
                action: "View Merge Request diff changes",
            },
            Shortcut {
                category: "Merge Requests",
                key: d(format!(
                    "{}",
                    app.config.keybindings.mrs.view_related_pipelines
                )),
                action: "View related pipelines for selected MR",
            },
            Shortcut {
                category: "Merge Requests",
                key: d(format!("{}", app.config.keybindings.mrs.close_entity)),
                action: "Close selected MR",
            },
            Shortcut {
                category: "Merge Requests",
                key: d(format!("{}", app.config.keybindings.mrs.reopen_entity)),
                action: "Reopen selected MR",
            },
            Shortcut {
                category: "Merge Requests",
                key: d(format!("{}", app.config.keybindings.mrs.delete_entity)),
                action: "Delete selected MR",
            },
            Shortcut {
                category: "Merge Requests",
                key: s("o"),
                action: "Open selected MR in browser",
            },
            // ── Pipelines ──
            Shortcut {
                category: "Pipelines",
                key: s("Enter"),
                action: "View pipeline jobs list",
            },
            Shortcut {
                category: "Pipelines",
                key: s("n"),
                action: "Create pipeline with interactive form",
            },
            Shortcut {
                category: "Pipelines",
                key: d(format!(
                    "{}",
                    app.config.keybindings.pipelines.trigger_pipeline
                )),
                action: "Trigger new pipeline from MR",
            },
            Shortcut {
                category: "Pipelines",
                key: d(format!("{}", app.config.keybindings.pipelines.retry)),
                action: "Retry selected pipeline(s)",
            },
            Shortcut {
                category: "Pipelines",
                key: d(format!("{}", app.config.keybindings.pipelines.cancel)),
                action: "Cancel pipeline execution",
            },
            Shortcut {
                category: "Pipelines",
                key: d(format!(
                    "{}",
                    app.config.keybindings.pipelines.download_artifact
                )),
                action: "Download pipeline artifact",
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
            // ── Jobs ──
            Shortcut {
                category: "Jobs",
                key: d(format!("{}", app.config.keybindings.jobs.view_trace)),
                action: "View job trace (toggle zoom)",
            },
            Shortcut {
                category: "Jobs",
                key: s("Esc / Backspc"),
                action: "Go back to Pipelines list",
            },
            Shortcut {
                category: "Jobs",
                key: d(format!("{}", app.config.keybindings.jobs.enter_pipeline)),
                action: "Switch to pipeline selector",
            },
            Shortcut {
                category: "Jobs",
                key: d(format!("{}", app.config.keybindings.jobs.retry)),
                action: "Retry selected job(s)",
            },
            Shortcut {
                category: "Jobs",
                key: d(format!("{}", app.config.keybindings.jobs.start_job)),
                action: "Start manual job (GitLab only)",
            },
            Shortcut {
                category: "Jobs",
                key: d(format!("{}", app.config.keybindings.jobs.cancel)),
                action: "Cancel selected job(s)",
            },
            Shortcut {
                category: "Jobs",
                key: d(format!("{}", app.config.keybindings.jobs.select_job)),
                action: "Check / uncheck job for bulk retry/cancel",
            },
            Shortcut {
                category: "Jobs",
                key: d(format!("{}", app.config.keybindings.jobs.select_stage)),
                action: "Select all jobs in stage",
            },
            Shortcut {
                category: "Jobs",
                key: d(format!("{}", app.config.keybindings.jobs.download_artifact)),
                action: "Download job artifact",
            },
            Shortcut {
                category: "Jobs",
                key: d(format!("{}", app.config.keybindings.jobs.view_trace_editor)),
                action: "Open job trace in external $EDITOR",
            },
            Shortcut {
                category: "Jobs",
                key: d(format!("{}", app.config.keybindings.jobs.open_in_browser)),
                action: "Open selected job in browser",
            },
            Shortcut {
                category: "Jobs",
                key: d(format!("{}", app.config.keybindings.jobs.toggle_trace_wrap)),
                action: "Toggle trace word wrap / clipped view",
            },
            Shortcut {
                category: "Jobs",
                key: s("m"),
                action: "Collapse / expand matrix jobs",
            },
            // ── Milestones ──
            Shortcut {
                category: "Milestones",
                key: d(format!(
                    "{}",
                    app.config.keybindings.milestones.create_milestone
                )),
                action: "Create new milestone",
            },
            Shortcut {
                category: "Milestones",
                key: d(format!(
                    "{}",
                    app.config.keybindings.milestones.edit_milestone
                )),
                action: "Edit selected milestone",
            },
            Shortcut {
                category: "Milestones",
                key: d(format!(
                    "{}",
                    app.config.keybindings.milestones.close_milestone
                )),
                action: "Close selected milestone",
            },
            Shortcut {
                category: "Milestones",
                key: d(format!(
                    "{}",
                    app.config.keybindings.milestones.reopen_milestone
                )),
                action: "Reopen selected milestone",
            },
            Shortcut {
                category: "Milestones",
                key: d(format!(
                    "{}",
                    app.config.keybindings.milestones.delete_milestone
                )),
                action: "Delete selected milestone",
            },
            Shortcut {
                category: "Milestones",
                key: d(format!(
                    "{}",
                    app.config.keybindings.milestones.open_in_browser
                )),
                action: "Open milestone in browser",
            },
            Shortcut {
                category: "Milestones",
                key: s("J / K"),
                action: "Scroll milestone issues list",
            },
            // ── Runners ──
            Shortcut {
                category: "Runners",
                key: d(format!(
                    "{} / {}",
                    app.config.keybindings.runners.pause, app.config.keybindings.runners.resume,
                )),
                action: "Pause / Resume runner",
            },
            Shortcut {
                category: "Runners",
                key: d(format!(
                    "{}",
                    app.config.keybindings.runners.edit_description
                )),
                action: "Edit runner description text",
            },
            // ── Releases ──
            Shortcut {
                category: "Releases",
                key: s("Enter"),
                action: "View release notes (toggle zoom)",
            },
            Shortcut {
                category: "Releases",
                key: d(format!(
                    "{}",
                    app.config.keybindings.releases.create_release
                )),
                action: "Create new release tag & changelog",
            },
            Shortcut {
                category: "Releases",
                key: d(format!("{}", app.config.keybindings.releases.edit_release)),
                action: "Edit selected release",
            },
            Shortcut {
                category: "Releases",
                key: d(format!(
                    "{}",
                    app.config.keybindings.releases.delete_release
                )),
                action: "Delete selected release",
            },
            Shortcut {
                category: "Releases",
                key: d(format!(
                    "{}",
                    app.config.keybindings.releases.open_in_browser
                )),
                action: "Open release in browser",
            },
            // ── TODOs ──
            Shortcut {
                category: "TODOs",
                key: d(format!("{}", app.config.keybindings.todos.mark_as_read)),
                action: "Open todo target & mark read",
            },
            Shortcut {
                category: "TODOs",
                key: d(format!("{}", app.config.keybindings.todos.open_in_browser)),
                action: "Open todo in browser",
            },
            // ── Terminal ──
            Shortcut {
                category: "Terminal",
                key: s("j / k / ↑ / ↓"),
                action: "Scroll terminal log",
            },
            Shortcut {
                category: "Terminal",
                key: d(format!("{}", app.config.keybindings.terminal.toggle_wrap)),
                action: "Toggle terminal line wrapping",
            },
            // ── Branches ──
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
            // ── Environments ──
            Shortcut {
                category: "Environments",
                key: d(format!(
                    "{}",
                    app.config.keybindings.environments.view_deployments
                )),
                action: "View deployments list for environment",
            },
            // ── Diff View ──
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
                action: "Page down / Page up",
            },
            Shortcut {
                category: "Diff View",
                key: s("[ / ]"),
                action: "Previous / Next Hunk",
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
                key: s("d"),
                action: "Toggle unified / side-by-side layout",
            },
            Shortcut {
                category: "Diff View",
                key: s("v / V"),
                action: "Start / Stop line selection for comment",
            },
            Shortcut {
                category: "Diff View",
                key: s("a"),
                action: "Interact with comments on current line",
            },
            Shortcut {
                category: "Diff View",
                key: s("C"),
                action: "Add comment via external $EDITOR",
            },
            Shortcut {
                category: "Diff View",
                key: s("e"),
                action: "Add code suggestion via $EDITOR",
            },
            Shortcut {
                category: "Diff View",
                key: s("/ f"),
                action: "Search within diff",
            },
            Shortcut {
                category: "Diff View",
                key: s("Ctrl+n / Ctrl+N"),
                action: "Search next / previous match",
            },
            Shortcut {
                category: "Diff View",
                key: s("z / Z"),
                action: "Collapse / Expand all files",
            },
            Shortcut {
                category: "Diff View",
                key: s("Enter / Space"),
                action: "Expand file tree / Toggle zoom",
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
            .title(format!(" {} Keyboard Shortcuts ", icons.label_keyboard))
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
        app.overlay_stack
            .push((crate::app::OverlayKind::Help, area));

        let help_chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints(
                [
                    Constraint::Length(3), // Search / Filter
                    Constraint::Min(0),    // Table
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

        f.render_widget(Clear, area);
        f.render_widget(search_p, help_chunks[0]);
        f.render_widget(table, help_chunks[1]);
    }

    if app.focus_column_checklist {
        let tab = app.active_tab;
        let kind = app.kind();
        let is_github = kind.is_github();
        let cols = tab.columns(kind);
        let active_idx = app.column_checklist_idx;

        let group_cols: Vec<&str> = cols.iter().copied().collect();

        let columns_list: Vec<(usize, &str)> = cols.iter().copied().enumerate().collect();

        let cols_end = cols.len();
        let group_end = cols_end + group_cols.len();
        let themes = crate::config::all_theme_presets();
        let theme_list_len = themes.len();
        let width = 48;
        let height =
            (columns_list.len() + group_cols.len() + theme_list_len + 4 + 2 + 2 + 6 + 6) as u16;
        let area = centered_rect_fixed(width, height, size);
        app.overlay_stack
            .push((crate::app::OverlayKind::Configure, area));

        let checklist_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
            .title(format!(
                " {} Configure View: {} ",
                icons.label_configure,
                tab.title(kind)
            ))
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().border_focused)
                    .add_modifier(Modifier::BOLD),
            );

        f.render_widget(Clear, area);
        f.render_widget(checklist_block.clone(), area);

        let inner_area = checklist_block.inner(area);

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

        let popup_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner_area);

        let mut chunk_idx = 0;

        let columns_header = Paragraph::new(format!("  {} COLUMNS", icons.label_columns)).style(
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

        let group_header = Paragraph::new(format!("  {} GROUP BY", icons.label_group)).style(
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
                let text = format!(
                    "  {} {}",
                    if is_selected {
                        &icons.radio_on
                    } else {
                        &icons.radio_off
                    },
                    col
                );
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

        let order_header = Paragraph::new(format!("  {} ORDER", icons.label_order)).style(
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
                let text = format!(
                    " {} {}",
                    if is_selected {
                        &icons.radio_on
                    } else {
                        &icons.radio_off
                    },
                    label
                );
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
        let page_size_header = Paragraph::new(format!(" {} PAGE SIZE", icons.label_page_size))
            .style(
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

        let theme_header = Paragraph::new(format!("  {} THEME", icons.label_theme)).style(
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
                let text = format!(
                    " {} {}",
                    if is_selected {
                        &icons.radio_on
                    } else {
                        &icons.radio_off
                    },
                    name
                );
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
            format!(" › {} Save View ‹", icons.label_save)
        } else {
            format!("   {} Save View", icons.label_save)
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

        // Save submenu
        if app.save_menu_open {
            let submenu_height = 7;
            let submenu_width = 30;
            let submenu_area = centered_rect_fixed(submenu_width, submenu_height, size);
            app.overlay_stack
                .push((crate::app::OverlayKind::SaveMenu, submenu_area));
            let submenu_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
                .title(format!(" {} Save to Config ", icons.label_save))
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
            app.overlay_stack
                .push((crate::app::OverlayKind::ColumnFilter, area));

            let constraints = vec![
                Constraint::Length(3), // Search/Filter
                Constraint::Min(0),    // List of items
            ];

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(constraints)
                .split(area);

            let (search_chunk, list_chunk) = (chunks[0], chunks[1]);

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

                    let marker = if is_selected {
                        format!(" {} ", icons.check_on)
                    } else {
                        format!(" {} ", icons.check_off)
                    };
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
        }
    }

    if let Some(confirm) = &app.confirm_popup {
        let kind = app.kind();
        let (title, message) = match confirm {
            crate::app::ConfirmAction::DeleteMilestone(iid) => (
                format!(" {} Delete Milestone? ", icons.action_delete),
                format!("Are you sure you want to delete milestone #{}?", iid),
            ),
            crate::app::ConfirmAction::DeleteRelease(tag_name) => (
                format!(" {} Delete Release? ", icons.action_delete),
                format!("Are you sure you want to delete release {}?", tag_name),
            ),
            crate::app::ConfirmAction::DeleteBranch(branch_name) => (
                format!(" {} Delete Branch? ", icons.action_delete),
                format!("Are you sure you want to delete branch '{}'?", branch_name),
            ),
            crate::app::ConfirmAction::CloseIssue(iid) => (
                format!(" {} Close Issue? ", icons.action_close),
                format!("Are you sure you want to close issue #{}?", iid),
            ),
            crate::app::ConfirmAction::DeleteIssue(iid) => (
                format!(" {} Delete Issue? ", icons.action_delete),
                format!(
                    "Are you sure you want to delete issue #{}? This action is permanent.",
                    iid
                ),
            ),
            crate::app::ConfirmAction::CloseMr(iid) => {
                let mr = kind.term("mr");
                let mr_short = kind.term("mr_short");
                (
                    format!(" {} Close {mr}? ", icons.action_close),
                    format!("Are you sure you want to close {mr_short} #{}?", iid),
                )
            }
            crate::app::ConfirmAction::DeleteMr(iid) => {
                let mr = kind.term("mr");
                let mr_short = kind.term("mr_short");
                (
                    format!(" {} Delete {mr}? ", icons.action_delete),
                    format!(
                        "Are you sure you want to delete {mr_short} #{}? This action is permanent.",
                        iid
                    ),
                )
            }
            crate::app::ConfirmAction::MergeMr(iid) => {
                let mr = kind.term("mr");
                let mr_short = kind.term("mr_short");
                (
                    format!(" {} Merge {mr}? ", icons.action_merge),
                    format!("Are you sure you want to merge {mr_short} #{}?", iid),
                )
            }
            crate::app::ConfirmAction::SubmitReview(_) => (
                format!(" {} Submit Review? ", icons.action_review),
                "You have pending draft comments. Would you like to submit your review now?"
                    .to_string(),
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
        app.overlay_stack
            .push((crate::app::OverlayKind::ConfirmPopup, area));

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Min(0),    // Message
                Constraint::Length(1), // YES/NO buttons
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
