use super::diff::{centered_rect_fixed, centered_rect_min};
use super::helpers::highlight_fuzzy_match;
use super::modal::{clear_area, modal_area};
use crate::app::SaveMenu;
use crate::app::{App, Tab};
use crate::config::{ICONS, THEME};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table},
};

/// Word-wrap text to a target width, returning a multi-line `Text` suitable for
/// a table cell. ratatui's `Table` expands the row height to fit multi-line
/// cells, so wrapping the action text here prevents it from being clipped.
fn wrap_cell_text(s: &str, width: u16) -> (Text<'static>, u16) {
    let width = width.max(8);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for para in s.split('\n') {
        if para.is_empty() {
            lines.push(Line::from(String::new()));
            continue;
        }
        let words: Vec<&str> = para.split(' ').collect();
        let mut current = String::new();
        for word in words {
            if current.is_empty() {
                current = word.to_string();
            } else if current.chars().count() + 1 + word.chars().count() <= width as usize {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(Line::from(std::mem::take(&mut current)));
                current = word.to_string();
            }
        }
        lines.push(Line::from(current));
    }
    let count = lines.len().max(1) as u16;
    (Text::from(lines), count)
}

pub(crate) fn render_overlays(f: &mut Frame, app: &mut App, size: Rect) {
    app.overlay_stack.clear();
    let icons = ICONS.read().unwrap();
    let label_colors = app.label_colors.clone();
    // EditMenu is rendered as a full-zoom interactive inspector in the detail
    // pane (ui/mod.rs::render_edit_menu_if_active); register its area for mouse
    // scroll/click handling so it participates in overlay z-ordering.
    if app.edit_menu.is_some() {
        if let Some(rect) = app.detail_rect {
            app.overlay_stack
                .push((crate::app::OverlayKind::EditMenu, rect));
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
                .border_style(
                    Style::default()
                        .fg(border_color_search)
                        .bg(THEME.read().unwrap().bg),
                )
                .title(" Filter ");

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
                    .bg(THEME.read().unwrap().bg)
                    .add_modifier(Modifier::ITALIC)
            } else {
                Style::default()
                    .fg(THEME.read().unwrap().text_normal)
                    .bg(THEME.read().unwrap().bg)
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
                            .bg(THEME.read().unwrap().bg)
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
                                .bg(THEME.read().unwrap().bg)
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
                                THEME.read().unwrap().bg
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

                    let list =
                        List::new(items).style(Style::default().bg(THEME.read().unwrap().bg));
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
            .style(Style::default().bg(THEME.read().unwrap().bg));

        let area = centered_rect_min(60, 60, 36, 6, size);
        clear_area(f, area);
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
                    .bg(THEME.read().unwrap().bg),
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
            .style(Style::default().bg(THEME.read().unwrap().bg));

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

        clear_area(f, area);
        f.render_widget(block, area);
        f.render_widget(header_p, chunks[0]);
        f.render_widget(table, chunks[1]);
    }

    if app.focus_column_checklist && app.selector.is_none() {
        let tab = app.active_tab;
        let kind = app.kind();
        let is_github = kind.is_github();
        let cols = tab.columns(kind);
        let active_idx = app.column_checklist_idx;

        let group_cols: Vec<&str> = cols.iter().copied().collect();

        let cols_end = cols.len();
        let group_end = cols_end + group_cols.len();
        let order_end = group_end + 2;
        let page_size_idx = order_end;
        let theme_idx = page_size_idx + 1;
        let save_end = theme_idx + 1;

        // Build the entire Configure view as one flat, scrollable list so the
        // section headers scroll together with their items.
        let mut lines: Vec<(Option<usize>, ListItem)> = Vec::new();
        let mut active_line: Option<usize> = None;

        let t = THEME.read().unwrap();

        // COLUMNS header
        lines.push((
            None,
            ListItem::new(format!("  {} COLUMNS", icons.label_columns)).style(
                Style::default()
                    .fg(t.header_fg)
                    .add_modifier(Modifier::BOLD),
            ),
        ));

        for (i, col) in cols.iter().enumerate() {
            let logical = i;
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
            let is_active = logical == active_idx;
            if is_active {
                active_line = Some(lines.len());
            }
            let style = if is_active {
                Style::default()
                    .fg(t.highlight_bg)
                    .bg(t.border_focused)
                    .add_modifier(Modifier::BOLD)
            } else if checked {
                Style::default().fg(t.text_normal)
            } else {
                Style::default().fg(t.text_muted)
            };
            lines.push((Some(logical), ListItem::new(text).style(style)));
        }

        // spacer
        lines.push((None, ListItem::new("")));

        // GROUP BY header
        lines.push((
            None,
            ListItem::new(format!("  {} GROUP BY", icons.label_group))
                .style(Style::default().fg(t.green).add_modifier(Modifier::BOLD)),
        ));

        for (j, col) in group_cols.iter().enumerate() {
            let logical = cols_end + j;
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
            let is_active = logical == active_idx;
            if is_active {
                active_line = Some(lines.len());
            }
            let style = if is_active {
                Style::default()
                    .fg(t.highlight_bg)
                    .bg(t.border_focused)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(t.green)
            } else {
                Style::default().fg(t.text_normal)
            };
            lines.push((Some(logical), ListItem::new(text).style(style)));
        }

        // spacer
        lines.push((None, ListItem::new("")));

        // ORDER header
        lines.push((
            None,
            ListItem::new(format!("  {} ORDER", icons.label_order))
                .style(Style::default().fg(t.yellow).add_modifier(Modifier::BOLD)),
        ));

        for (i, label) in ["Ascending", "Descending"].iter().enumerate() {
            let logical = group_end + i;
            let is_selected = app.group_ascending.get(&tab).copied().unwrap_or(true) == (i == 0);
            let text = format!(
                " {} {}",
                if is_selected {
                    &icons.radio_on
                } else {
                    &icons.radio_off
                },
                label
            );
            let is_active = logical == active_idx;
            if is_active {
                active_line = Some(lines.len());
            }
            let style = if is_active {
                Style::default()
                    .fg(t.highlight_bg)
                    .bg(t.border_focused)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(t.yellow)
            } else {
                Style::default().fg(t.text_normal)
            };
            lines.push((Some(logical), ListItem::new(text).style(style)));
        }

        // spacer
        lines.push((None, ListItem::new("")));

        // Page Size — inline row (icon + label in header_fg, value in text_normal)
        let is_page_size_active = active_idx == page_size_idx;
        let page_size_value = if app.editing_page_size {
            format!("[ {}| ]", app.page_size_input)
        } else {
            format!("[ {} ]", app.page_size)
        };
        let page_size_style = if app.editing_page_size {
            Style::default()
                .fg(t.highlight_bg)
                .bg(t.green)
                .add_modifier(Modifier::BOLD)
        } else if is_page_size_active {
            Style::default()
                .fg(t.highlight_bg)
                .bg(t.border_focused)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text_normal)
        };
        let page_size_line = if is_page_size_active || app.editing_page_size {
            Line::from(Span::styled(
                format!(
                    " {} Page Size   {} ",
                    icons.label_page_size, page_size_value
                ),
                page_size_style,
            ))
        } else {
            Line::from(vec![
                Span::styled(
                    format!(" {} Page Size ", icons.label_page_size),
                    Style::default()
                        .fg(t.header_fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(page_size_value, Style::default().fg(t.text_normal)),
            ])
        };
        if is_page_size_active {
            active_line = Some(lines.len());
        }
        lines.push((
            Some(page_size_idx),
            ListItem::new(page_size_line).style(page_size_style),
        ));

        // Theme — inline row (icon + label in purple, value aligned with Page Size)
        let current_theme_name = app.config.theme_preset.as_deref().unwrap_or("default");
        let is_theme_active = active_idx == theme_idx;
        let theme_value = format!("[ {} ]", current_theme_name);
        let theme_style = if is_theme_active {
            Style::default()
                .fg(t.highlight_bg)
                .bg(t.border_focused)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text_normal)
        };
        let theme_line = if is_theme_active {
            Line::from(Span::styled(
                format!(" {} Theme     {} ", icons.label_theme, theme_value),
                theme_style,
            ))
        } else {
            Line::from(vec![
                Span::styled(
                    format!(" {} Theme     ", icons.label_theme),
                    Style::default().fg(t.purple).add_modifier(Modifier::BOLD),
                ),
                Span::styled(theme_value, Style::default().fg(t.text_normal)),
            ])
        };
        if is_theme_active {
            active_line = Some(lines.len());
        }
        lines.push((
            Some(theme_idx),
            ListItem::new(theme_line).style(theme_style),
        ));

        // spacer
        lines.push((None, ListItem::new("")));

        // Save button — no header, centered in the inner area
        let is_save_selected = active_idx == save_end;
        let width: u16 = 64;
        let inner_w = width.saturating_sub(2) as usize; // -2 for borders
        let save_label = format!("{} Save View", icons.label_save);
        let save_visible_width = save_label.chars().count();
        let save_left_pad = (inner_w.saturating_sub(save_visible_width)) / 2;
        let save_button_text = format!("{:pad$}{}", "", save_label, pad = save_left_pad);
        let save_button_style = if is_save_selected {
            Style::default()
                .fg(t.highlight_bg)
                .bg(t.border_focused)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text_muted)
        };
        if is_save_selected {
            active_line = Some(lines.len());
        }
        lines.push((
            Some(save_end),
            ListItem::new(save_button_text).style(save_button_style),
        ));

        drop(t);

        // Grow the popup with content, but cap it so the whole list scrolls
        // within the available terminal height (headers included).
        let content_len = lines.len() as u16;
        // +2 accounts for the block's top/bottom border so inner_area has room
        // for every item without clipping the save button.
        let height = content_len
            .saturating_add(2)
            .max(18)
            .min(size.height.saturating_sub(2));
        let area = centered_rect_fixed(width, height, size);
        app.overlay_stack
            .push((crate::app::OverlayKind::Configure, area));

        let checklist_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
            .style(Style::default().bg(THEME.read().unwrap().bg))
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

        clear_area(f, area);
        f.render_widget(checklist_block.clone(), area);

        let inner_area = checklist_block.inner(area);

        let items: Vec<ListItem> = lines.into_iter().map(|(_, li)| li).collect();
        let mut state = ListState::default();
        state.select(active_line);
        f.render_stateful_widget(List::new(items), inner_area, &mut state);

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
            clear_area(f, submenu_area);
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
                            .fg(THEME.read().unwrap().highlight_bg)
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
            let (body, selector_area) = modal_area(f, &selector.title, 50, 60, 34, 6, size);
            app.overlay_stack
                .push((crate::app::OverlayKind::ColumnFilter, selector_area));

            let constraints = vec![
                Constraint::Length(3), // Search/Filter
                Constraint::Min(0),    // List of items
            ];

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(constraints)
                .split(body);

            let (search_chunk, list_chunk) = (chunks[0], chunks[1]);

            let border_color_search = if selector.is_filtering {
                THEME.read().unwrap().border_focused
            } else {
                THEME.read().unwrap().border
            };
            let search_block = Block::default()
                .borders(Borders::ALL)
                .border_style(
                    Style::default()
                        .fg(border_color_search)
                        .bg(THEME.read().unwrap().bg),
                )
                .title(" Filter ");

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

            f.render_widget(search_p, search_chunk);

            let filtered_items = selector.get_filtered_items_with_indices();
            if filtered_items.is_empty() {
                let p = Paragraph::new("\n  No matching options found.")
                    .style(
                        Style::default()
                            .fg(THEME.read().unwrap().text_muted)
                            .bg(THEME.read().unwrap().bg)
                            .add_modifier(Modifier::ITALIC),
                    )
                    .wrap(ratatui::widgets::Wrap { trim: true });
                f.render_widget(p, list_chunk);
            } else {
                let items: Vec<ListItem> = filtered_items
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
                            THEME.read().unwrap().bg
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

                let list = List::new(items).style(Style::default().bg(THEME.read().unwrap().bg));
                let mut state = selector.state.clone();
                f.render_stateful_widget(list, list_chunk, &mut state);
                selector.state = state;
            }
        }
    }

    if let Some(dialog) = &app.submit_dialog {
        let theme = THEME.read().unwrap();
        let icon =
            match dialog.action {
                crate::app::ConfirmAction::DeleteMilestone(_)
                | crate::app::ConfirmAction::DeleteRelease(_)
                | crate::app::ConfirmAction::DeleteBranch(_)
                | crate::app::ConfirmAction::DeleteIssue(_)
                | crate::app::ConfirmAction::DeleteMr(_) => icons.action_delete.clone(),
                crate::app::ConfirmAction::CloseIssue(_)
                | crate::app::ConfirmAction::CloseMr(_)
                | crate::app::ConfirmAction::CloseMilestone(_) => icons.action_close.clone(),
                crate::app::ConfirmAction::ReopenIssue(_)
                | crate::app::ConfirmAction::ReopenMr(_)
                | crate::app::ConfirmAction::ReopenMilestone(_) => icons.action_reopen.clone(),
                crate::app::ConfirmAction::MergeMr(_)
                | crate::app::ConfirmAction::BulkMergeMrs(_) => icons.action_merge.clone(),
                crate::app::ConfirmAction::RevokeMr(_)
                | crate::app::ConfirmAction::SubmitReview(_) => icons.action_review.clone(),
                crate::app::ConfirmAction::RebaseMr(_) => icons.merge_rebase.clone(),
            };
        let option_rows = dialog.options.len();

        let title = format!(" {} {} ", icon, dialog.title);
        let mut body_lines = if dialog.body.is_empty() {
            0
        } else {
            textwrap(&dialog.body, 56).len() + 1 // text + leading blank line
        };
        if option_rows > 0 {
            body_lines += 1; // +1 gap before options
        }
        let option_rows = dialog.options.len();
        let button_height: u16 = 1; // label only (no top/bottom borders)
        // [border top] + [pad] + body + options + [separator] + [buttons]
        let mut dialog_height =
            (2u16 + 1 + body_lines as u16 + option_rows as u16 + 1 + button_height) as u16;
        let max_h = size.height.saturating_sub(2).max(11);
        dialog_height = dialog_height.clamp(11, max_h);

        let area = centered_rect_fixed(crate::app::SubmitDialog::DIALOG_WIDTH, dialog_height, size);
        app.overlay_stack
            .push((crate::app::OverlayKind::ConfirmPopup, area));

        let block = Block::default()
            .title(title)
            .title_style(
                Style::default()
                    .fg(theme.header_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_focused))
            .style(Style::default().bg(theme.bg));

        // Draw the backdrop + border first so the content below renders
        // on top of it (the block fills the whole area with its bg).
        clear_area(f, area);
        f.render_widget(block, area);

        let v = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(1),             // pad below title
                Constraint::Min(0),                // body + options (flexes)
                Constraint::Length(1),             // separator
                Constraint::Length(button_height), // button row
            ])
            .split(area);

        let content = v[1];
        let sep = v[2];
        let buttons = v[3];

        let content_chunks = if option_rows > 0 {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(option_rows as u16)])
                .split(content)
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0)])
                .split(content)
        };

        let mut body_lines = if dialog.body.is_empty() {
            vec![]
        } else {
            textwrap(&dialog.body, 56)
        };

        if !dialog.body.is_empty() {
            body_lines.insert(0, Line::from(""));
            if option_rows > 0 {
                body_lines.push(Line::from(""));
            }
        } else if option_rows > 0 {
            body_lines.push(Line::from(""));
        }
        let body_p = Paragraph::new(body_lines)
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text_normal))
            .wrap(ratatui::widgets::Wrap { trim: true });
        f.render_widget(body_p, content_chunks[0]);

        if option_rows > 0 {
            let mut state = ListState::default();
            state.select(dialog.option_idx());
            let items: Vec<Line<'static>> = dialog
                .options
                .iter()
                .map(|o| {
                    let is_radio = o.label.starts_with("Strategy: ");
                    let display_label = if is_radio {
                        o.label.trim_start_matches("Strategy: ")
                    } else {
                        &o.label
                    };
                    let mark = if is_radio {
                        if o.checked {
                            "(x) ".to_string()
                        } else {
                            "( ) ".to_string()
                        }
                    } else {
                        if o.checked {
                            "[x] ".to_string()
                        } else {
                            "[ ] ".to_string()
                        }
                    };
                    Line::from(format!("{mark}{}", display_label))
                })
                .collect();
            let list = List::new(items)
                .style(Style::default().fg(theme.text_normal))
                .highlight_style(
                    Style::default()
                        .bg(theme.border_focused)
                        .fg(theme.highlight_bg)
                        .add_modifier(Modifier::BOLD),
                );

            // Center the options list to match the centered body text
            let max_option_width = dialog
                .options
                .iter()
                .map(|o| {
                    let text = o.label.trim_start_matches("Strategy: ");
                    text.chars().count() + 4 // "[x] "
                })
                .max()
                .unwrap_or(20) as u16;
            let pad = content_chunks[1].width.saturating_sub(max_option_width) / 2;
            let list_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(pad),
                    Constraint::Length(max_option_width),
                    Constraint::Min(0),
                ])
                .split(content_chunks[1]);

            f.render_stateful_widget(list, list_layout[1], &mut state);
        }

        f.render_widget(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme.border)),
            sep,
        );

        let halves = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 2),
                Constraint::Length(1),
                Constraint::Ratio(1, 2),
            ])
            .split(buttons);

        let submit_selected = dialog.is_on_submit();
        let cancel_selected = dialog.is_on_cancel();

        // Cancel button (right half)
        f.render_widget(
            Paragraph::new(format!("{} Cancel", icons.check_off))
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .fg(if cancel_selected {
                            theme.bg
                        } else {
                            theme.text_normal
                        })
                        .bg(if cancel_selected {
                            theme.border_focused
                        } else {
                            theme.bg
                        })
                        .add_modifier(if cancel_selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
            halves[2],
        );

        // Submit button (left half)
        f.render_widget(
            Paragraph::new(format!("{} {}", icons.check_on, dialog.submit_label))
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .fg(if submit_selected {
                            theme.bg
                        } else {
                            if dialog.action.is_destructive() {
                                theme.red
                            } else {
                                theme.green
                            }
                        })
                        .bg(if submit_selected {
                            if dialog.action.is_destructive() {
                                theme.red
                            } else {
                                theme.green
                            }
                        } else {
                            theme.bg
                        })
                        .add_modifier(if submit_selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
            halves[0],
        );
    }

    render_help(f, app, size);
}

pub(crate) fn render_help(f: &mut Frame, app: &mut App, size: Rect) {
    if !app.show_help {
        return;
    }

    let icons = ICONS.read().unwrap();

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
            action: "Toggle columns config popup (filter / group / sort)",
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
                app.config.keybindings.global.scroll_down, app.config.keybindings.global.scroll_up
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
            key: d(app.config.keybindings.global.quit.clone()),
            action: "Quit program",
        },
        Shortcut {
            category: "Global & Nav",
            key: s("Esc"),
            action: "Close active overlay",
        },
        Shortcut {
            category: "Global & Nav",
            key: s("Ctrl+C"),
            action: "Quit program",
        },
        // ── Issues ──
        Shortcut {
            category: "Issues",
            key: d(app.config.keybindings.issues.create_issue.clone()),
            action: "Create new Issue",
        },
        Shortcut {
            category: "Issues",
            key: d(app.config.keybindings.issues.select_issue.clone()),
            action: "Toggle issue selection (bulk edit with e)",
        },
        Shortcut {
            category: "Issues",
            key: d(app.config.keybindings.issues.selection_toggle.clone()),
            action: "Toggle select mode (paint selection while navigating)",
        },
        Shortcut {
            category: "Issues",
            key: d(app.config.keybindings.issues.edit_entity.clone()),
            action: "Open parameter edit menu",
        },
        Shortcut {
            category: "Issues",
            key: d(app.config.keybindings.issues.close_entity.clone()),
            action: "Close selected Issue",
        },
        Shortcut {
            category: "Issues",
            key: d(app.config.keybindings.issues.reopen_entity.clone()),
            action: "Reopen selected Issue",
        },
        Shortcut {
            category: "Issues",
            key: d(app.config.keybindings.issues.delete_entity.clone()),
            action: "Delete selected Issue",
        },
        Shortcut {
            category: "Issues",
            key: d(app.config.keybindings.issues.copy_reference.clone()),
            action: "Copy selected Issue as Markdown link",
        },
        Shortcut {
            category: "Issues",
            key: d(app.config.keybindings.issues.open_in_browser.clone()),
            action: "Open selected Issue in browser",
        },
        Shortcut {
            category: "Issues",
            key: d(app.config.keybindings.issues.create_mr.clone()),
            action: "Create Merge Request from selected Issue",
        },
        // ── Merge Requests ──
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.create_mr.clone()),
            action: "Create new Merge Request",
        },
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.select_mr.clone()),
            action: "Toggle MR/PR selection (bulk edit/merge with e/m)",
        },
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.selection_toggle.clone()),
            action: "Toggle select mode (paint selection while navigating)",
        },
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.edit_entity.clone()),
            action: "Open parameter edit menu",
        },
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.approve_mr.clone()),
            action: "Approve selected MR",
        },
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.revoke_mr.clone()),
            action: "Revoke your approval (GitLab only)",
        },
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.rebase_mr.clone()),
            action: "Rebase source branch onto target",
        },
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.merge_mr.clone()),
            action: "Merge selected MR (configure squash/delete)",
        },
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.toggle_draft.clone()),
            action: "Toggle Draft / Ready status",
        },
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.view_diff.clone()),
            action: "View Merge Request diff changes",
        },
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.view_related_pipelines.clone()),
            action: "View related pipelines for selected MR",
        },
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.close_entity.clone()),
            action: "Close selected MR",
        },
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.reopen_entity.clone()),
            action: "Reopen selected MR",
        },
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.delete_entity.clone()),
            action: "Delete selected MR",
        },
        Shortcut {
            category: "Merge Requests",
            key: d(app.config.keybindings.mrs.open_in_browser.clone()),
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
            key: d(app.config.keybindings.pipelines.run_new.clone()),
            action: "Create pipeline with interactive form",
        },
        Shortcut {
            category: "Pipelines",
            key: d(app.config.keybindings.pipelines.trigger_pipeline.clone()),
            action: "Trigger new pipeline from MR",
        },
        Shortcut {
            category: "Pipelines",
            key: d(app.config.keybindings.pipelines.retry.clone()),
            action: "Retry selected pipeline(s)",
        },
        Shortcut {
            category: "Pipelines",
            key: d(app.config.keybindings.pipelines.cancel.clone()),
            action: "Cancel pipeline execution",
        },
        Shortcut {
            category: "Pipelines",
            key: d(app.config.keybindings.pipelines.open_workflow.clone()),
            action: "Open pipeline workflow in browser",
        },
        Shortcut {
            category: "Pipelines",
            key: s("Space"),
            action: "Check / uncheck pipeline for bulk retry",
        },
        Shortcut {
            category: "Pipelines",
            key: d(app.config.keybindings.pipelines.open_in_browser.clone()),
            action: "Open pipeline in browser",
        },
        // ── Jobs ──
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.view_trace.clone()),
            action: "View job trace (toggle zoom)",
        },
        Shortcut {
            category: "Jobs",
            key: s("Esc / Backspc"),
            action: "Go back to Pipelines list",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.enter_pipeline.clone()),
            action: "Switch to pipeline selector",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.retry.clone()),
            action: "Retry selected job(s)",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.start_job.clone()),
            action: "Start manual job (GitLab only)",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.cancel.clone()),
            action: "Cancel selected job(s)",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.select_job.clone()),
            action: "Check / uncheck job for bulk retry/cancel",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.select_stage.clone()),
            action: "Select all jobs in stage",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.download_artifact.clone()),
            action: "Download job artifact",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.view_trace_editor.clone()),
            action: "Open job trace in external $EDITOR",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.open_in_browser.clone()),
            action: "Open selected job in browser",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.toggle_trace_wrap.clone()),
            action: "Toggle trace word wrap / clipped view",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.trace_search.clone()),
            action: "Search within job trace",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.toggle_trace_follow.clone()),
            action: "Toggle trace auto-follow mode",
        },
        Shortcut {
            category: "Jobs",
            key: s("m"),
            action: "Collapse / expand matrix jobs",
        },
        // ── Milestones ──
        Shortcut {
            category: "Milestones",
            key: d(app.config.keybindings.milestones.create_milestone.clone()),
            action: "Create new milestone",
        },
        Shortcut {
            category: "Milestones",
            key: d(app.config.keybindings.milestones.edit_milestone.clone()),
            action: "Edit selected milestone",
        },
        Shortcut {
            category: "Milestones",
            key: d(app.config.keybindings.milestones.close_milestone.clone()),
            action: "Close selected milestone",
        },
        Shortcut {
            category: "Milestones",
            key: d(app.config.keybindings.milestones.reopen_milestone.clone()),
            action: "Reopen selected milestone",
        },
        Shortcut {
            category: "Milestones",
            key: d(app.config.keybindings.milestones.delete_milestone.clone()),
            action: "Delete selected milestone",
        },
        Shortcut {
            category: "Milestones",
            key: d(app.config.keybindings.milestones.open_in_browser.clone()),
            action: "Open milestone in browser",
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
            key: d(app.config.keybindings.runners.edit_description.clone()),
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
            key: d(app.config.keybindings.releases.create_release.clone()),
            action: "Create new release tag & changelog",
        },
        Shortcut {
            category: "Releases",
            key: d(app.config.keybindings.releases.edit_release.clone()),
            action: "Edit selected release",
        },
        Shortcut {
            category: "Releases",
            key: d(app.config.keybindings.releases.delete_release.clone()),
            action: "Delete selected release",
        },
        Shortcut {
            category: "Releases",
            key: d(app.config.keybindings.releases.open_in_browser.clone()),
            action: "Open release in browser",
        },
        // ── TODOs ──
        Shortcut {
            category: "TODOs",
            key: d(app.config.keybindings.todos.mark_as_read.clone()),
            action: "Open todo target & mark read",
        },
        Shortcut {
            category: "TODOs",
            key: d(app.config.keybindings.todos.open_in_browser.clone()),
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
            key: d(app.config.keybindings.terminal.toggle_wrap.clone()),
            action: "Toggle terminal line wrapping",
        },
        // ── Branches ──
        Shortcut {
            category: "Branches",
            key: d(app.config.keybindings.branches.create_branch.clone()),
            action: "Create new branch",
        },
        Shortcut {
            category: "Branches",
            key: d(app.config.keybindings.branches.delete_branch.clone()),
            action: "Delete selected branch",
        },
        // ── Environments ──
        Shortcut {
            category: "Environments",
            key: d(app.config.keybindings.environments.view_deployments.clone()),
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
            key: s("m"),
            action: "Mark / unmark file (or directory) as reviewed",
        },
        Shortcut {
            category: "Diff View",
            key: s("M"),
            action: "Hide / show reviewed files in the tree",
        },
        Shortcut {
            category: "Diff View",
            key: s("Enter / Space"),
            action: "Expand file tree / Toggle zoom",
        },
        Shortcut {
            category: "Diff View",
            key: d(format!("{} / F1", app.config.keybindings.global.help)),
            action: "Show this help modal",
        },
        // ── Inspector / Editor ──
        Shortcut {
            category: "Inspector / Editor",
            key: s("j / k / ↓ / ↑ / Tab / Shift+Tab"),
            action: "Navigate fields & buttons",
        },
        Shortcut {
            category: "Inspector / Editor",
            key: s("Enter / Space"),
            action: "Edit field / Select option / Submit",
        },
        Shortcut {
            category: "Inspector / Editor",
            key: s("Ctrl+E"),
            action: "Edit description / notes in $EDITOR",
        },
        Shortcut {
            category: "Inspector / Editor",
            key: s("J / K"),
            action: "Scroll description / notes pane",
        },
        Shortcut {
            category: "Inspector / Editor",
            key: s("Esc"),
            action: "Exit inspector / Close editor form",
        },
        Shortcut {
            category: "Inspector / Editor",
            key: d(format!("{} / F1", app.config.keybindings.global.help)),
            action: "Show this help modal",
        },
        // ── Selector / Filter ──
        Shortcut {
            category: "Selector / Filter",
            key: s("j / k / ↓ / ↑"),
            action: "Navigate options",
        },
        Shortcut {
            category: "Selector / Filter",
            key: s("Enter"),
            action: "Confirm selection",
        },
        Shortcut {
            category: "Selector / Filter",
            key: s("Space"),
            action: "Toggle item (multi-select)",
        },
        Shortcut {
            category: "Selector / Filter",
            key: s("Esc"),
            action: "Close selector overlay",
        },
        Shortcut {
            category: "Selector / Filter",
            key: s("Type to search"),
            action: "Fuzzy filter available items",
        },
        Shortcut {
            category: "Selector / Filter",
            key: d(format!("{} / F1", app.config.keybindings.global.help)),
            action: "Show this help modal",
        },
        // ── Column Config ──
        Shortcut {
            category: "Column Config",
            key: s("j / k / ↓ / ↑"),
            action: "Navigate columns & group-by options",
        },
        Shortcut {
            category: "Column Config",
            key: s("Space"),
            action: "Toggle column visibility checkbox",
        },
        Shortcut {
            category: "Column Config",
            key: s("Enter"),
            action: "Open value filter selector for column",
        },
        Shortcut {
            category: "Column Config",
            key: d(app.config.keybindings.global.save_view.clone()),
            action: "Save layout to config",
        },
        Shortcut {
            category: "Column Config",
            key: s("Esc"),
            action: "Close columns config popup",
        },
        Shortcut {
            category: "Column Config",
            key: d(format!("{} / F1", app.config.keybindings.global.help)),
            action: "Show this help modal",
        },
        // ── Date Picker ──
        Shortcut {
            category: "Date Picker",
            key: s("h / l / ← / →"),
            action: "Previous / Next month",
        },
        Shortcut {
            category: "Date Picker",
            key: s("j / k / ↓ / ↑"),
            action: "Previous / Next day",
        },
        Shortcut {
            category: "Date Picker",
            key: s("Enter"),
            action: "Confirm date selection",
        },
        Shortcut {
            category: "Date Picker",
            key: s("Esc"),
            action: "Cancel date selection",
        },
        Shortcut {
            category: "Date Picker",
            key: d(format!("{} / F1", app.config.keybindings.global.help)),
            action: "Show this help modal",
        },
    ];

    let active_categories: &[&str] = if app.diff_view.is_some() {
        &["Diff View"]
    } else if app.edit_menu.is_some() {
        &["Global & Nav", "Inspector / Editor"]
    } else if app.focus_column_checklist {
        &["Global & Nav", "Column Config"]
    } else if app.date_picker.is_some() {
        &["Global & Nav", "Date Picker"]
    } else if app.selector.is_some() {
        &["Global & Nav", "Selector / Filter"]
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
        .style(Style::default().bg(THEME.read().unwrap().bg));

    let area = centered_rect_min(90, 85, 95, 38, size);
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

    // Action column width: inner table width minus the two fixed columns
    // (20 + 24) and the inter-column spacing (2 gaps of 2).
    let action_width = help_chunks[1].width.saturating_sub(20 + 24 + 4).max(8);

    let border_color = if app.help_search_query.is_empty() {
        THEME.read().unwrap().border
    } else {
        THEME.read().unwrap().border_focused
    };
    let search_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(" Filter Shortcuts ")
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

    let rows: Vec<Row> =
        if app.help_search_query.is_empty() {
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
                    let (action_text, action_lines) = wrap_cell_text(s.action, action_width);
                    result_rows.push(
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
                            Cell::from(action_text.patch_style(
                                Style::default().fg(THEME.read().unwrap().text_normal),
                            )),
                        ])
                        .height(action_lines),
                    );
                    last_category = s.category;
                } else {
                    let (action_text, action_lines) = wrap_cell_text(s.action, action_width);
                    result_rows.push(
                        Row::new(vec![
                            Cell::from(""),
                            Cell::from(Span::styled(
                                s.key.clone(),
                                Style::default()
                                    .fg(THEME.read().unwrap().text_normal)
                                    .add_modifier(Modifier::BOLD),
                            )),
                            Cell::from(action_text.patch_style(
                                Style::default().fg(THEME.read().unwrap().text_normal),
                            )),
                        ])
                        .height(action_lines),
                    );
                }
            }
            result_rows
        } else {
            let query = app.help_search_query.to_lowercase();
            shortcuts
                .iter()
                .filter(|s| {
                    s.category.to_lowercase().contains(&query)
                        || s.key.to_lowercase().contains(&query)
                        || s.action.to_lowercase().contains(&query)
                })
                .map(|s| {
                    let (action_text, action_lines) = wrap_cell_text(s.action, action_width);
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
                        Cell::from(
                            action_text.patch_style(
                                Style::default().fg(THEME.read().unwrap().text_normal),
                            ),
                        ),
                    ])
                    .height(action_lines)
                })
                .collect()
        };

    let widths = [
        Constraint::Length(20),
        Constraint::Length(24),
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

    clear_area(f, area);
    f.render_widget(search_p, help_chunks[0]);
    f.render_widget(table, help_chunks[1]);
}

/// Wrap a string into `width`-character lines at word boundaries,
/// returning one `Line` per wrapped line. Hard-break overlong words
/// instead of panicking on zero width.
fn textwrap(text: &str, width: usize) -> Vec<Line<'static>> {
    crate::utils::format::wrap_text(text, width)
        .into_iter()
        .map(Line::from)
        .collect()
}
