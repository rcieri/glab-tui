use std::collections::HashMap;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use crate::app::{EditMenu, EntityDocument, Field, FieldType, InspectorContent};
use crate::config::{ICONS, THEME};
use crate::ui::helpers::get_label_color;
use crate::utils::format::{parse_ansi_trace, render_markdown};

pub enum InspectorMode<'a> {
    ReadOnly { scroll: u16, title_suffix: &'a str },
    Interactive { menu: &'a mut EditMenu },
}

pub(crate) fn render_entity_inspector(
    f: &mut Frame,
    doc: &EntityDocument,
    area: Rect,
    mode: InspectorMode<'_>,
    label_colors: &HashMap<String, Color>,
) {
    let icons = ICONS.read().unwrap();
    let theme = THEME.read().unwrap();

    match mode {
        InspectorMode::Interactive { menu } => {
            let is_new_entity = menu.is_new();
            let submit_idx = menu.fields.len() + 1;

            let is_desc_selected = menu.selected_idx < menu.fields.len()
                && menu.fields[menu.selected_idx].label == "Description"
                && menu.fields[menu.selected_idx].kind == FieldType::Text;

            // Layout: main content + submit button
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(3)])
                .split(area);

            let main_area = layout[0];
            let main_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(main_area);

            // Left pane: properties / fields
            let fields_block = Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(theme.border));
            let fields_inner = fields_block.inner(main_chunks[0]);
            f.render_widget(fields_block, main_chunks[0]);

            let field_items = build_field_list_items(
                &menu.fields,
                Some(menu.selected_idx),
                menu.editing,
                menu.cursor_pos,
                main_chunks[0].width,
                label_colors,
                true,
            );

            let list = List::new(field_items).style(Style::default().bg(theme.bg));
            let mut state = menu.state.clone();
            f.render_stateful_widget(list, fields_inner, &mut state);
            menu.state = state;

            // Right pane: content / description
            let desc_value = menu.get_description_value();
            let desc_lines = if desc_value.is_empty() {
                vec![Line::from(Span::styled(
                    "Empty — press Enter to edit, Ctrl+E for editor",
                    Style::default()
                        .fg(theme.text_muted)
                        .add_modifier(Modifier::ITALIC),
                ))]
            } else {
                render_markdown(&desc_value)
            };

            let desc_lines = if is_desc_selected && menu.editing && !desc_value.is_empty() {
                let mut lines = desc_lines;
                if let Some(first) = lines.first_mut() {
                    let mut spans: Vec<Span> = first.spans.clone();
                    spans.push(Span::styled(
                        " \u{258c}",
                        Style::default()
                            .fg(theme.text_normal)
                            .add_modifier(Modifier::SLOW_BLINK),
                    ));
                    *first = Line::from(spans);
                }
                lines
            } else {
                desc_lines
            };

            let desc_border_color = if is_desc_selected {
                theme.border_focused
            } else {
                theme.border
            };
            let desc_block = Block::default()
                .borders(Borders::LEFT)
                .border_style(Style::default().fg(desc_border_color))
                .title(format!(" {} Description ", icons.label_details))
                .title_style(
                    Style::default()
                        .fg(if is_desc_selected {
                            theme.header_fg
                        } else {
                            theme.text_muted
                        })
                        .add_modifier(Modifier::BOLD),
                );

            f.render_widget(
                Paragraph::new(desc_lines)
                    .block(desc_block)
                    .scroll((menu.desc_scroll, 0))
                    .wrap(ratatui::widgets::Wrap { trim: true }),
                main_chunks[1],
            );

            // Submit button
            let submit_chunk = layout[1];
            let btn_text = if is_new_entity {
                format!(" {} Submit ", icons.check_on)
            } else {
                format!(" {} Save ", icons.check_on)
            };
            let is_submit_selected = menu.selected_idx == submit_idx;
            let submit_fg = if is_submit_selected {
                theme.bg
            } else {
                theme.green
            };
            let submit_bg = if is_submit_selected {
                theme.green
            } else {
                theme.bg
            };
            let submit_block = Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme.text_muted));
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
        InspectorMode::ReadOnly {
            scroll,
            title_suffix,
        } => {
            let title = if title_suffix.is_empty() {
                format!(" {} Details ", icons.label_details)
            } else {
                format!(" {} Details{} ", icons.label_details, title_suffix)
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.border))
                .title(title)
                .title_style(
                    Style::default()
                        .fg(theme.text_muted)
                        .add_modifier(Modifier::BOLD),
                );

            let inner = block.inner(area);
            f.render_widget(block, area);

            if inner.width == 0 || inner.height == 0 {
                return;
            }

            let has_content = match &doc.content {
                InspectorContent::Empty(_) => false,
                InspectorContent::Markdown(m) => !m.trim().is_empty(),
                InspectorContent::AnsiTrace { trace, .. } => !trace.trim().is_empty(),
                InspectorContent::PipelineStages(jobs) => !jobs.is_empty(),
                InspectorContent::Custom(lines) => !lines.is_empty(),
            };

            // If we have both fields and rich content, split into two panes if width allows
            if has_content && inner.width >= 70 {
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
                    .split(inner);

                // Left: fields list
                let field_items = build_field_list_items(
                    &doc.fields,
                    None,
                    false,
                    0,
                    chunks[0].width,
                    label_colors,
                    false,
                );
                let list = List::new(field_items).style(Style::default().bg(theme.bg));
                let mut state = ListState::default();
                f.render_stateful_widget(list, chunks[0], &mut state);

                // Separator / Right Content
                let content_block = Block::default()
                    .borders(Borders::LEFT)
                    .border_style(Style::default().fg(theme.border));
                let content_inner = content_block.inner(chunks[1]);
                f.render_widget(content_block, chunks[1]);

                render_inspector_content(f, &doc.content, content_inner, scroll);
            } else {
                // Single pane: if only fields, render fields. If only content, render content. If both on small screen, stack.
                if !has_content {
                    let field_items = build_field_list_items(
                        &doc.fields,
                        None,
                        false,
                        0,
                        inner.width,
                        label_colors,
                        false,
                    );
                    let list = List::new(field_items).style(Style::default().bg(theme.bg));
                    let mut state = ListState::default();
                    f.render_stateful_widget(list, inner, &mut state);
                } else if doc.fields.is_empty() {
                    render_inspector_content(f, &doc.content, inner, scroll);
                } else {
                    // Small screen stacked layout
                    let field_count = doc
                        .fields
                        .iter()
                        .filter(|f| {
                            !(f.kind == FieldType::Section
                                && f.label.to_uppercase() == "DESCRIPTION")
                        })
                        .count() as u16;
                    let split_height = (field_count + 1).min(inner.height.saturating_sub(4)).max(3);

                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(split_height), Constraint::Min(3)])
                        .split(inner);

                    let field_items = build_field_list_items(
                        &doc.fields,
                        None,
                        false,
                        0,
                        chunks[0].width,
                        label_colors,
                        false,
                    );
                    let list = List::new(field_items).style(Style::default().bg(theme.bg));
                    let mut state = ListState::default();
                    f.render_stateful_widget(list, chunks[0], &mut state);

                    let content_block = Block::default()
                        .borders(Borders::TOP)
                        .border_style(Style::default().fg(theme.border));
                    let content_inner = content_block.inner(chunks[1]);
                    f.render_widget(content_block, chunks[1]);

                    render_inspector_content(f, &doc.content, content_inner, scroll);
                }
            }
        }
    }
}

pub(crate) fn build_field_list_items(
    fields: &[Field],
    selected_idx: Option<usize>,
    editing: bool,
    cursor_pos: usize,
    pane_width: u16,
    label_colors: &HashMap<String, Color>,
    skip_description: bool,
) -> Vec<ListItem<'static>> {
    let icons = ICONS.read().unwrap();
    let theme = THEME.read().unwrap();

    let label_width = fields
        .iter()
        .map(|f| f.label.len())
        .max()
        .unwrap_or(16)
        .max(12);

    fields
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            if skip_description {
                !(f.kind == FieldType::Section && f.label.to_uppercase() == "DESCRIPTION")
                    && !(f.kind == FieldType::Text && f.label == "Description")
            } else {
                true
            }
        })
        .map(|(i, f)| {
            let label = &f.label;
            let val = &f.value;
            let is_selected = selected_idx == Some(i);

            if f.kind == FieldType::Section {
                let available = (pane_width.saturating_sub(4) as usize) / 2;
                let label_text = format!(" {} ", label.to_uppercase());
                let pad = (available.saturating_sub(label_text.len() + 2)) / 2;
                let line = format!(
                    "{:\u{2500}>pad$} {} {:\u{2500}<pad$}",
                    "",
                    label_text,
                    "",
                    pad = pad.max(1)
                );
                return ListItem::new(Line::from(Span::styled(
                    line,
                    Style::default()
                        .fg(theme.text_muted)
                        .add_modifier(Modifier::BOLD),
                )));
            }

            let item_bg = if is_selected {
                theme.highlight_bg
            } else {
                theme.bg
            };

            let label_style = if is_selected {
                Style::default()
                    .fg(theme.text_normal)
                    .bg(item_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_muted).bg(item_bg)
            };

            let sep_style = if is_selected {
                Style::default()
                    .fg(theme.text_normal)
                    .bg(item_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_muted).bg(item_bg)
            };

            let mut val_spans = Vec::new();
            if val.is_empty() && f.kind != FieldType::Text && f.kind != FieldType::ReadOnly {
                val_spans.push(Span::styled(
                    " None",
                    Style::default()
                        .fg(theme.text_muted)
                        .bg(item_bg)
                        .add_modifier(Modifier::ITALIC),
                ));
            } else if !val.is_empty() {
                let truncated = val.clone();
                match f.kind {
                    FieldType::Section => {}
                    FieldType::MultiSelect => {
                        if val == "None" {
                            val_spans.push(Span::styled(
                                " None",
                                Style::default()
                                    .fg(theme.text_muted)
                                    .bg(item_bg)
                                    .add_modifier(Modifier::ITALIC),
                            ));
                        } else {
                            let parts: Vec<&str> = truncated.split(',').collect();
                            for (idx, part) in parts.iter().enumerate() {
                                if idx > 0 {
                                    val_spans.push(Span::styled(
                                        ", ",
                                        Style::default().fg(theme.text_normal).bg(item_bg),
                                    ));
                                }
                                let trimmed = part.trim();
                                let color = if label == "Labels" {
                                    get_label_color(trimmed, label_colors)
                                } else {
                                    theme.blue
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
                    FieldType::Toggle => {
                        let (display, fg, bg) = match val.to_lowercase().as_str() {
                            "yes" | "true" | "confidential" => (
                                format!(" {} YES ", icons.check_on),
                                theme.green,
                                if is_selected {
                                    theme.highlight_bg
                                } else {
                                    theme.green_bg
                                },
                            ),
                            "no" | "false" | "public" => (
                                format!(" {} NO ", icons.check_off),
                                theme.text_muted,
                                item_bg,
                            ),
                            "draft" => (
                                format!(" {} DRAFT ", icons.status_draft),
                                theme.yellow,
                                if is_selected {
                                    theme.highlight_bg
                                } else {
                                    theme.yellow_bg
                                },
                            ),
                            "ready" => (
                                format!(" {} READY ", icons.status_ready),
                                theme.green,
                                if is_selected {
                                    theme.highlight_bg
                                } else {
                                    theme.green_bg
                                },
                            ),
                            _ => (format!(" {} ", val), theme.text_muted, item_bg),
                        };
                        val_spans.push(Span::styled(
                            display,
                            Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
                        ));
                    }
                    FieldType::Date => {
                        let display = if val == "Set" {
                            "Not set".to_string()
                        } else {
                            val.clone()
                        };
                        val_spans.push(Span::styled(
                            format!(" {}", display),
                            Style::default()
                                .fg(theme.yellow)
                                .bg(item_bg)
                                .add_modifier(Modifier::BOLD),
                        ));
                    }
                    FieldType::Ref => {
                        val_spans.push(Span::styled(
                            format!(" {}", truncated),
                            Style::default()
                                .fg(theme.purple)
                                .bg(item_bg)
                                .add_modifier(Modifier::BOLD),
                        ));
                    }
                    FieldType::Text | FieldType::ReadOnly => {
                        let mut badge_bg: Option<Color> = None;
                        let mut formatted_val: Option<String> = None;

                        let (val_fg, is_bold) = match label.as_str() {
                            "State" => match val.to_lowercase().as_str() {
                                "opened" | "open" | "active" => {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.green_bg
                                    });
                                    formatted_val = Some(format!(" {} OPEN ", icons.state_open));
                                    (theme.green, true)
                                }
                                "closed" | "close" => {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.red_bg
                                    });
                                    formatted_val =
                                        Some(format!(" {} CLOSED ", icons.state_closed));
                                    (theme.red, true)
                                }
                                "merged" => {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.purple_bg
                                    });
                                    formatted_val =
                                        Some(format!(" {} MERGED ", icons.state_merged));
                                    (theme.purple, true)
                                }
                                _ => (theme.text_normal, true),
                            },
                            "Status" | "Deploy Status" => match val.to_lowercase().as_str() {
                                "success" | "online" | "passed" => {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.green_bg
                                    });
                                    formatted_val = Some(format!(
                                        " {} {} ",
                                        icons.status_success,
                                        val.to_uppercase()
                                    ));
                                    (theme.green, true)
                                }
                                "ready" => {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.green_bg
                                    });
                                    formatted_val = Some(format!(" {} READY ", icons.status_ready));
                                    (theme.green, true)
                                }
                                "failed" | "offline" => {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.red_bg
                                    });
                                    formatted_val = Some(format!(
                                        " {} {} ",
                                        icons.status_failed,
                                        val.to_uppercase()
                                    ));
                                    (theme.red, true)
                                }
                                "running" => {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.blue_bg
                                    });
                                    formatted_val = Some(format!(
                                        " {} {} ",
                                        icons.status_running,
                                        val.to_uppercase()
                                    ));
                                    (theme.blue, true)
                                }
                                "pending" | "waiting" => {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.yellow_bg
                                    });
                                    formatted_val = Some(format!(
                                        " {} {} ",
                                        icons.status_pending,
                                        val.to_uppercase()
                                    ));
                                    (theme.yellow, true)
                                }
                                "draft" => {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.yellow_bg
                                    });
                                    formatted_val = Some(format!(" {} DRAFT ", icons.status_draft));
                                    (theme.yellow, true)
                                }
                                "paused" => {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.yellow_bg
                                    });
                                    formatted_val =
                                        Some(format!(" {} PAUSED ", icons.runner_paused));
                                    (theme.yellow, true)
                                }
                                "canceled" | "cancelled" => {
                                    badge_bg = Some(item_bg);
                                    formatted_val =
                                        Some(format!(" {} CANCELED ", icons.status_canceled));
                                    (theme.text_muted, true)
                                }
                                "unread" | "new" => {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.green_bg
                                    });
                                    formatted_val = Some(" NEW ".to_string());
                                    (theme.green, true)
                                }
                                _ => (theme.text_normal, true),
                            },
                            "Approval" => {
                                if val.contains("APPROVED") {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.green_bg
                                    });
                                    formatted_val = Some(format!(" {} ", val));
                                    (theme.green, true)
                                } else if val.contains("CHANGES") || val.contains("CHG") {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.red_bg
                                    });
                                    formatted_val = Some(format!(" {} ", val));
                                    (theme.red, true)
                                } else if val.contains("AWAITING") || val.contains("REVIEW REQ") {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.yellow_bg
                                    });
                                    formatted_val = Some(format!(" {} ", val));
                                    (theme.yellow, true)
                                } else {
                                    (theme.text_normal, false)
                                }
                            }
                            "Mergeable" => {
                                if val.contains("CLEAN") {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.green_bg
                                    });
                                    formatted_val = Some(format!(" {} ", val));
                                    (theme.green, true)
                                } else if val.contains("CONFLICT") {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.red_bg
                                    });
                                    formatted_val = Some(format!(" {} ", val));
                                    (theme.red, true)
                                } else if val.contains("REBASE") || val.contains("CHECKING") {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.yellow_bg
                                    });
                                    formatted_val = Some(format!(" {} ", val));
                                    (theme.yellow, true)
                                } else {
                                    (theme.text_normal, false)
                                }
                            }
                            "Workflow" => {
                                if val.contains("Approved") {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.green_bg
                                    });
                                    formatted_val = Some(format!(" {} ", val));
                                    (theme.green, true)
                                } else if val.contains("Returned") {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.red_bg
                                    });
                                    formatted_val = Some(format!(" {} ", val));
                                    (theme.red, true)
                                } else if val.contains("Review") {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.yellow_bg
                                    });
                                    formatted_val = Some(format!(" {} ", val));
                                    (theme.yellow, true)
                                } else if val.contains("Yours") {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.blue_bg
                                    });
                                    formatted_val = Some(format!(" {} ", val));
                                    (theme.blue, true)
                                } else {
                                    (theme.text_muted, false)
                                }
                            }
                            "Threads" => {
                                if val.contains("0") || val.contains("all resolved") {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.green_bg
                                    });
                                    formatted_val = Some(format!(" {} ", val));
                                    (theme.green, true)
                                } else {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.red_bg
                                    });
                                    formatted_val = Some(format!(" {} ", val));
                                    (theme.red, true)
                                }
                            }
                            "Default" | "Protected" | "Can Push" | "Active" | "Confidential" => {
                                if val == "YES" || val == "Yes" || val == "true" {
                                    badge_bg = Some(if is_selected {
                                        theme.highlight_bg
                                    } else {
                                        theme.green_bg
                                    });
                                    formatted_val = Some(format!(" {} YES ", icons.check_on));
                                    (theme.green, true)
                                } else {
                                    badge_bg = Some(item_bg);
                                    formatted_val = Some(format!(" {} NO ", icons.check_off));
                                    (theme.text_muted, false)
                                }
                            }
                            "Milestone" | "Branch" | "Ref" | "Deploy Ref" | "Stage" => {
                                (theme.purple, false)
                            }
                            "Author" | "Assignees" | "Reviewers" | "Deployer" | "Target"
                            | "Project" => (theme.blue, false),
                            "Updated" | "Created" | "Duration" | "Released" | "Deployed"
                            | "Date" | "Due Date" | "Start Date" | "Avg Wait" => {
                                (theme.yellow, false)
                            }
                            "ID" | "SHA" | "Commit" | "Runner" | "Tag" | "Deploy SHA"
                            | "Deploy ID" => (theme.blue, false),
                            _ => (theme.text_normal, false),
                        };

                        let current_bg = badge_bg.unwrap_or(item_bg);
                        let mut style = Style::default().fg(val_fg).bg(current_bg);
                        if is_selected || is_bold {
                            style = style.add_modifier(Modifier::BOLD);
                        }

                        if label == "Title" && is_selected && editing {
                            let cursor = cursor_pos.min(val.len());
                            let before = if cursor > 0 && cursor <= val.len() {
                                val[..cursor].to_string()
                            } else {
                                String::new()
                            };
                            let at_cursor = val
                                .chars()
                                .nth(cursor)
                                .map(|ch| ch.to_string())
                                .unwrap_or_else(|| " ".to_string());
                            let after = if cursor < val.len() {
                                val[cursor + at_cursor.len()..].to_string()
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
                                Style::default()
                                    .fg(theme.bg)
                                    .bg(val_fg)
                                    .add_modifier(Modifier::SLOW_BLINK),
                            ));
                            val_spans.push(Span::styled(after, style));
                        } else {
                            let display_text =
                                formatted_val.unwrap_or_else(|| format!(" {}", truncated));
                            val_spans.push(Span::styled(display_text, style));
                        }
                    }
                }
            }

            let icon = match f.kind {
                FieldType::Section => "",
                FieldType::MultiSelect => {
                    if label == "Labels" {
                        "\u{f02b}"
                    } else if label == "Assignees" || label == "Reviewers" || label == "Author" {
                        "\u{f007}"
                    } else {
                        icons.check_on.as_str()
                    }
                }
                FieldType::Toggle => icons.radio_on.as_str(),
                FieldType::Date => "\u{f073}",
                FieldType::Ref => icons.label_branch.as_str(),
                FieldType::Text | FieldType::ReadOnly => match label.as_str() {
                    "Title" | "Description" | "Name" => icons.label_details.as_str(),
                    "State" => match val.to_lowercase().as_str() {
                        "opened" | "open" | "active" => icons.state_open.as_str(),
                        "closed" | "close" => icons.state_closed.as_str(),
                        "merged" => icons.state_merged.as_str(),
                        _ => icons.label_details.as_str(),
                    },
                    "Status" | "Deploy Status" => match val.to_lowercase().as_str() {
                        "success" | "online" | "ready" => icons.status_success.as_str(),
                        "failed" | "offline" => icons.status_failed.as_str(),
                        "running" => icons.status_running.as_str(),
                        "pending" | "waiting" | "draft" => icons.status_pending.as_str(),
                        "canceled" | "cancelled" => icons.status_canceled.as_str(),
                        "paused" => icons.runner_paused.as_str(),
                        _ => icons.label_details.as_str(),
                    },
                    "Author" | "Assignees" | "Reviewers" | "Deployer" => "\u{f007}",
                    "Milestone" => icons.label_milestone.as_str(),
                    "Branch" | "Ref" | "Deploy Ref" => icons.label_branch.as_str(),
                    "Environment" => icons.label_environment.as_str(),
                    "Approval" => icons.approval_approved.as_str(),
                    "Mergeable" => icons.merge_clean.as_str(),
                    "Workflow" => icons.workflow_review.as_str(),
                    "Threads" => icons.thread_unresolved.as_str(),
                    "Created" | "Updated" | "Date" | "Due Date" | "Start Date" | "Released"
                    | "Deployed" => "\u{f073}",
                    "Duration" | "Avg Wait" => "\u{f017}",
                    "ID" | "SHA" | "Commit" | "Deploy SHA" | "Deploy ID" | "Runner" | "Tag" => {
                        "\u{f029}"
                    }
                    "Metrics" | "Utilization" | "Queue Depth" | "Active Jobs" | "Progress" => {
                        "\u{f080}"
                    }
                    _ => icons.label_details.as_str(),
                },
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
            ListItem::new(Line::from(line_spans)).style(Style::default().bg(item_bg))
        })
        .collect()
}

pub(crate) fn render_inspector_content(
    f: &mut Frame,
    content: &InspectorContent,
    area: Rect,
    scroll: u16,
) {
    let theme = THEME.read().unwrap();

    match content {
        InspectorContent::Markdown(md) => {
            let lines = if md.trim().is_empty() {
                vec![Line::from(Span::styled(
                    "No description provided.",
                    Style::default()
                        .fg(theme.text_muted)
                        .add_modifier(Modifier::ITALIC),
                ))]
            } else {
                render_markdown(md)
            };
            f.render_widget(
                Paragraph::new(lines)
                    .scroll((scroll, 0))
                    .wrap(ratatui::widgets::Wrap { trim: true }),
                area,
            );
        }
        InspectorContent::AnsiTrace { trace, wrap } => {
            let formatted_lines = parse_ansi_trace(trace, &theme);
            let mut paragraph = Paragraph::new(formatted_lines).scroll((scroll, 0));
            if *wrap {
                paragraph = paragraph.wrap(ratatui::widgets::Wrap { trim: false });
            }
            f.render_widget(paragraph, area);
        }
        InspectorContent::PipelineStages(jobs) => {
            let mut lines = Vec::new();
            lines.push(Line::from(vec![Span::styled(
                "Pipeline Jobs & Stages:",
                Style::default()
                    .fg(theme.header_fg)
                    .add_modifier(Modifier::BOLD),
            )]));
            lines.push(Line::from(""));
            for job in jobs {
                let status_style = match job.status.as_str() {
                    "success" => Style::default()
                        .fg(theme.green)
                        .add_modifier(Modifier::BOLD),
                    "failed" => Style::default().fg(theme.red).add_modifier(Modifier::BOLD),
                    "running" => Style::default().fg(theme.blue).add_modifier(Modifier::BOLD),
                    _ => Style::default().fg(theme.text_muted),
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("[{}] ", job.stage),
                        Style::default().fg(theme.purple),
                    ),
                    Span::styled(
                        format!("{:<24} ", job.name),
                        Style::default().fg(theme.text_normal),
                    ),
                    Span::styled(job.status.to_uppercase(), status_style),
                ]));
            }
            f.render_widget(
                Paragraph::new(lines)
                    .scroll((scroll, 0))
                    .wrap(ratatui::widgets::Wrap { trim: true }),
                area,
            );
        }
        InspectorContent::Custom(lines) => {
            f.render_widget(
                Paragraph::new(lines.clone())
                    .scroll((scroll, 0))
                    .wrap(ratatui::widgets::Wrap { trim: true }),
                area,
            );
        }
        InspectorContent::Empty(msg) => {
            f.render_widget(
                Paragraph::new(Span::styled(
                    *msg,
                    Style::default()
                        .fg(theme.text_muted)
                        .add_modifier(Modifier::ITALIC),
                ))
                .alignment(Alignment::Center),
                area,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{EntityDocument, Field, InspectorContent};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::collections::HashMap;

    #[test]
    fn test_build_field_list_items_read_only() {
        let fields = vec![
            Field::read_only("Title", "Fix bug in parser".to_string()),
            Field::read_only("State", "OPEN".to_string()),
            Field::section("Details"),
            Field::multi_select("Labels", "bug, urgent".to_string()),
        ];
        let label_colors = HashMap::new();
        let items = build_field_list_items(&fields, None, false, 0, 80, &label_colors, true);
        assert_eq!(items.len(), 4);
    }

    #[test]
    fn test_render_entity_inspector_read_only() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();

        let doc = EntityDocument {
            title: "Issue #42".to_string(),
            fields: vec![
                Field::read_only("Title", "Test Issue".to_string()),
                Field::read_only("State", "OPEN".to_string()),
            ],
            content: InspectorContent::Markdown("This is test markdown content.".to_string()),
        };
        let label_colors = HashMap::new();

        terminal
            .draw(|f| {
                render_entity_inspector(
                    f,
                    &doc,
                    f.area(),
                    InspectorMode::ReadOnly {
                        scroll: 0,
                        title_suffix: "",
                    },
                    &label_colors,
                );
            })
            .unwrap();
    }

    #[test]
    fn test_render_entity_inspector_empty_content() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        let doc = EntityDocument {
            title: "Runner #1".to_string(),
            fields: vec![
                Field::read_only("ID", "#1".to_string()),
                Field::read_only("Status", "ONLINE".to_string()),
            ],
            content: InspectorContent::Empty("Runner metrics"),
        };
        let label_colors = HashMap::new();

        terminal
            .draw(|f| {
                render_entity_inspector(
                    f,
                    &doc,
                    f.area(),
                    InspectorMode::ReadOnly {
                        scroll: 0,
                        title_suffix: "",
                    },
                    &label_colors,
                );
            })
            .unwrap();
    }
}
