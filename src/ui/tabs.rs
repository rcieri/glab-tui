use crate::app::{App, EntityDocument, InspectorContent, Tab};
use crate::command_log::StatusFilter;
use crate::config::THEME;
use crate::ui::helpers::sort_header_label;
use crate::ui::list_tab::TableContext;
use crate::utils::format::{format_ref, sanitize_untrusted, time_ago, truncate};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

/// Resolve `app.detail_scroll` against the pane's `max`, which only the
/// render pass knows: honour a pending jump to the bottom, otherwise cap the
/// scroll so a held-down scroll key can't push the preview content past its
/// last line.
///
/// The jump flag is consumed here, in the frame that reads it — left standing
/// it would send the next pane the user opens to its bottom as well.
fn settle_detail_scroll(app: &mut App, max: u16) {
    if app.detail_scroll_to_bottom {
        app.detail_scroll = max;
        app.detail_scroll_to_bottom = false;
    } else {
        app.detail_scroll = app.detail_scroll.min(max);
    }
}

/// Draw `doc` read-only in a list tab's preview pane, or a placeholder when
/// no row is selected.
fn render_list_preview(
    f: &mut Frame,
    app: &mut App,
    detail_rect: Rect,
    doc: Option<crate::app::EntityDocument>,
    theme: &crate::config::Theme,
    title_icon: &str,
) {
    let Some(doc) = doc else {
        f.render_widget(
            Paragraph::new("Select an item to view preview...")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {title_icon} Preview "))
                        .title_style(
                            Style::default()
                                .fg(theme.text_muted)
                                .add_modifier(Modifier::BOLD),
                        )
                        .border_style(Style::default().fg(theme.border)),
                )
                .style(Style::default().fg(theme.text_muted)),
            detail_rect,
        );
        return;
    };
    let max_detail_scroll = super::inspector::render_entity_inspector(
        f,
        &doc,
        detail_rect,
        super::inspector::InspectorMode::ReadOnly {
            scroll: app.detail_scroll,
            title_suffix: "",
        },
        &app.label_colors,
    );
    settle_detail_scroll(app, max_detail_scroll);
}

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

/// The list and preview placeholders for a repository-only tab opened in
/// group scope. Names the configured switch-repo key, since that is how the
/// user gets out.
fn group_scope_unavailable<'a>(
    app: &App,
    theme: &crate::config::Theme,
    icons: &crate::config::Icons,
    tab_name: &str,
    main_block: Block<'a>,
) -> [Paragraph<'a>; 2] {
    let list = Paragraph::new(format!(
        "\n\n {} {} tab is not available in group scope ({}).\n Press `{}` to switch to a repository scope, or `Esc` to return.",
        icons.label_details,
        tab_name,
        app.scope.as_str(),
        app.config.keybindings.global.switch_repo,
    ))
    .alignment(Alignment::Center)
    .block(main_block)
    .style(Style::default().fg(theme.text_muted));
    let preview = Paragraph::new("Select a repository scope to view details...")
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} Preview ", icons.label_details))
                .border_style(Style::default().fg(theme.border)),
        )
        .style(Style::default().fg(theme.text_muted));
    [list, preview]
}

pub(crate) fn render_tab_issues(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
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
            App::issue_filter_values,
        );

        let rows = filtered_issues.iter().enumerate().map(|(idx, i)| {
            let is_selected = app.issues.state.selected() == Some(idx);
            let is_checked = app
                .selected_issues
                .contains(&(i.project_path.clone(), i.iid));
            let (state_text, state_style) = if i.state.eq_ignore_ascii_case("opened")
                || i.state.eq_ignore_ascii_case("open")
                || i.state.eq_ignore_ascii_case("OPEN")
            {
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
            if app.scope.is_group() && app.is_column_visible(Tab::Issues, "Project") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&i.project_path, 35),
                    &app.search_query,
                    is_selected,
                    is_checked,
                    Style::default().fg(theme.badge_group_bg),
                    Alignment::Left,
                ));
            }
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
            let show_related_mrs =
                app.is_column_visible(Tab::Issues, if app.is_github() { "PRs" } else { "MRs" });
            if show_related_mrs {
                let related_str = match &i.related_mrs {
                    None if app.fetching_related_mrs.contains(&i.iid) => "…".to_string(),
                    None => "--".to_string(),
                    Some(crate::domain::issues::RelatedMrsState::Empty) => "--".to_string(),
                    Some(crate::domain::issues::RelatedMrsState::Failed(_)) => "!".to_string(),
                    Some(crate::domain::issues::RelatedMrsState::Items(items)) => {
                        let prefix = "!";
                        if items.is_empty() {
                            "--".to_string()
                        } else {
                            let mut parts = Vec::new();
                            for r in items.iter().take(2) {
                                let st = if !r.state.is_empty() {
                                    r.state.as_str()
                                } else {
                                    app.mrs
                                        .items
                                        .iter()
                                        .find(|m| m.iid == r.iid)
                                        .map(|m| m.state.as_str())
                                        .unwrap_or("")
                                };
                                let sym = match st {
                                    "opened" | "open" => &icons.state_open,
                                    "closed" | "close" => &icons.state_closed,
                                    "merged" => &icons.state_merged,
                                    _ => "",
                                };
                                if sym.is_empty() {
                                    parts.push(format!("{}{}", prefix, r.iid));
                                } else {
                                    parts.push(format!("{}{} {}", prefix, r.iid, sym));
                                }
                            }
                            if items.len() > 2 {
                                parts.push(format!("+{}", items.len() - 2));
                            }
                            parts.join(", ")
                        }
                    }
                };
                cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&related_str, 16),
                    &app.search_query,
                    is_selected,
                    is_checked,
                    Style::default().fg(theme.purple),
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
            let bar_cell = Cell::from(ratatui::text::Span::styled(
                " ",
                if is_checked {
                    Style::default().bg(THEME.read().unwrap().checked_bg)
                } else {
                    Style::default()
                },
            ));
            cells.insert(0, bar_cell);
            Row::new(cells).style(row_style).height(1)
        });

        let mut header_cells = Vec::new();
        let mut widths = Vec::new();

        header_cells.push(Cell::from(""));
        widths.push(Constraint::Length(1));

        let sort = app.sort_for(Tab::Issues);
        let header_line = |name: &str| Line::from(sort_header_label(name, sort, &icons));
        let header = |name: &str| Cell::from(header_line(name));

        if app.scope.is_group() && app.is_column_visible(Tab::Issues, "Project") {
            header_cells.push(header("Project"));
            widths.push(col_w(content_area.width, 30));
        }

        if app.is_column_visible(Tab::Issues, "ID") {
            header_cells.push(header("ID"));
            widths.push(Constraint::Length(8));
        }
        if app.is_column_visible(Tab::Issues, "State") {
            header_cells.push(Cell::from(
                header_line("State").alignment(Alignment::Center),
            ));
            widths.push(Constraint::Length(10));
        }
        if app.is_column_visible(Tab::Issues, "Title") {
            header_cells.push(header("Title"));
            widths.push(Constraint::Fill(1));
        }
        if app.is_column_visible(Tab::Issues, "Assignees") {
            header_cells.push(header("Assignees"));
            widths.push(col_w(content_area.width, 22));
        }
        if app.is_column_visible(Tab::Issues, "Labels") {
            header_cells.push(header("Labels"));
            widths.push(col_w(content_area.width, 26));
        }
        if app.is_column_visible(Tab::Issues, "Milestone") {
            header_cells.push(header("Milestone"));
            widths.push(col_w(content_area.width, 18));
        }
        let show_related_mrs =
            app.is_column_visible(Tab::Issues, if app.is_github() { "PRs" } else { "MRs" });
        if show_related_mrs {
            header_cells.push(header(if app.is_github() { "PRs" } else { "MRs" }));
            widths.push(col_w(content_area.width, 16));
        }
        if app.is_column_visible(Tab::Issues, "Due Date") {
            header_cells.push(header("Due Date"));
            widths.push(col_w(content_area.width, 20));
        }
        if app.is_column_visible(Tab::Issues, "Author") {
            header_cells.push(header("Author"));
            widths.push(col_w(content_area.width, 18));
        }

        if widths.is_empty() {
            widths.push(Constraint::Min(0));
        }

        let table = Table::new(rows, widths)
            .header(Row::new(header_cells).style(header_style).height(1))
            .block(main_block)
            .row_highlight_style(Style::default())
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
                let doc = crate::entity_editor::build_issue_document(
                    issue,
                    is_github,
                    app.fetching_related_mrs.contains(&issue.iid),
                );
                let max_detail_scroll = super::inspector::render_entity_inspector(
                    f,
                    &doc,
                    detail_rect,
                    super::inspector::InspectorMode::ReadOnly {
                        scroll: app.detail_scroll,
                        title_suffix: "",
                    },
                    &app.label_colors,
                );
                settle_detail_scroll(app, max_detail_scroll);
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
                "\n\n {} Loading {}...",
                icons.label_loading,
                app.kind().term("mr_plural").to_lowercase()
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
            App::mr_filter_values,
        );

        let rows = filtered_mrs.iter().enumerate().map(|(idx, m)| {
            let is_selected = app.mrs.state.selected() == Some(idx);
            let is_checked = app.selected_mrs.contains(&(m.project_path.clone(), m.iid));
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
            if app.scope.is_group() && app.is_column_visible(Tab::MergeRequests, "Project") {
                cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&m.project_path, 35),
                    &app.search_query,
                    is_selected,
                    is_checked,
                    Style::default().fg(theme.badge_group_bg),
                    Alignment::Left,
                ));
            }
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
            if app.is_column_visible(Tab::MergeRequests, "Stack") {
                let stack_str = if let Some(ref s) = m.stack {
                    format!("#{} {}/{}", s.number, s.position, s.size)
                } else {
                    "—".to_string()
                };
                cells.push(super::helpers::render_fuzzy_cell(
                    &stack_str,
                    &app.search_query,
                    is_selected,
                    is_checked,
                    if m.stack.is_some() {
                        Style::default().fg(theme.purple)
                    } else {
                        Style::default().fg(theme.text_muted)
                    },
                    Alignment::Center,
                ));
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
            let show_closes = app.is_column_visible(Tab::MergeRequests, "Closes");
            if show_closes {
                let linked_str = match &m.related_issues {
                    None if app.fetching_mr_related_issues.contains(&m.iid) => "…".to_string(),
                    None => "--".to_string(),
                    Some(crate::domain::mr::RelatedIssuesState::Empty) => "--".to_string(),
                    Some(crate::domain::mr::RelatedIssuesState::Failed(_)) => "!".to_string(),
                    Some(crate::domain::mr::RelatedIssuesState::Items(items)) => {
                        if items.is_empty() {
                            "--".to_string()
                        } else {
                            let mut parts = Vec::new();
                            for r in items.iter().take(2) {
                                let st = if !r.state.is_empty() {
                                    r.state.as_str()
                                } else {
                                    app.issues
                                        .items
                                        .iter()
                                        .find(|i| i.iid == r.iid)
                                        .map(|i| i.state.as_str())
                                        .unwrap_or("")
                                };
                                let sym = match st {
                                    "opened" | "open" => &icons.state_open,
                                    "closed" | "close" => &icons.state_closed,
                                    _ => "",
                                };
                                if sym.is_empty() {
                                    parts.push(format!("#{}", r.iid));
                                } else {
                                    parts.push(format!("#{} {}", r.iid, sym));
                                }
                            }
                            if items.len() > 2 {
                                parts.push(format!("+{}", items.len() - 2));
                            }
                            parts.join(", ")
                        }
                    }
                };
                cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&linked_str, 16),
                    &app.search_query,
                    is_selected,
                    is_checked,
                    Style::default().fg(theme.blue),
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
            let bar_cell = Cell::from(ratatui::text::Span::styled(
                " ",
                if is_checked {
                    Style::default().bg(theme.checked_bg)
                } else {
                    Style::default()
                },
            ));
            cells.insert(0, bar_cell);
            Row::new(cells).style(row_style).height(1)
        });

        let mut header_cells = Vec::new();
        let mut widths = Vec::new();

        header_cells.push(Cell::from(""));
        widths.push(Constraint::Length(1));

        let sort = app.sort_for(Tab::MergeRequests);
        let header_line = |name: &str| Line::from(sort_header_label(name, sort, &icons));
        let header = |name: &str| Cell::from(header_line(name));

        if app.scope.is_group() && app.is_column_visible(Tab::MergeRequests, "Project") {
            header_cells.push(header("Project"));
            widths.push(col_w(content_area.width, 30));
        }

        if app.is_column_visible(Tab::MergeRequests, "ID") {
            header_cells.push(header("ID"));
            widths.push(Constraint::Length(8));
        }
        if app.is_column_visible(Tab::MergeRequests, "State") {
            header_cells.push(Cell::from(
                header_line("State").alignment(Alignment::Center),
            ));
            widths.push(Constraint::Length(10));
        }
        if app.is_column_visible(Tab::MergeRequests, "Status") {
            header_cells.push(Cell::from(
                header_line("Status").alignment(Alignment::Center),
            ));
            widths.push(Constraint::Length(12));
        }
        if app.is_column_visible(Tab::MergeRequests, "Mergeable") {
            header_cells.push(Cell::from(
                header_line("Mergeable").alignment(Alignment::Center),
            ));
            widths.push(col_w(content_area.width, 13));
        }
        if app.is_column_visible(Tab::MergeRequests, "Approval") {
            header_cells.push(Cell::from(
                header_line("Approval").alignment(Alignment::Center),
            ));
            widths.push(col_w(content_area.width, 12));
        }
        if app.is_column_visible(Tab::MergeRequests, "Title") {
            header_cells.push(header("Title"));
            widths.push(Constraint::Fill(1));
        }
        if app.is_column_visible(Tab::MergeRequests, "Assignees") {
            header_cells.push(header("Assignees"));
            widths.push(col_w(content_area.width, 22));
        }
        if app.is_column_visible(Tab::MergeRequests, "Reviewers") {
            header_cells.push(header("Reviewers"));
            widths.push(col_w(content_area.width, 22));
        }
        if app.is_column_visible(Tab::MergeRequests, "Workflow") {
            header_cells.push(Cell::from(
                header_line("Workflow").alignment(Alignment::Center),
            ));
            widths.push(col_w(content_area.width, 13));
        }
        if app.is_column_visible(Tab::MergeRequests, "Labels") {
            header_cells.push(header("Labels"));
            widths.push(col_w(content_area.width, 26));
        }
        let is_github = app.is_github();
        if app.is_column_visible(
            Tab::MergeRequests,
            if is_github { "Action" } else { "Pipeline" },
        ) {
            header_cells.push(Cell::from(
                header_line(if is_github { "Action" } else { "Pipeline" })
                    .alignment(Alignment::Center),
            ));
            widths.push(Constraint::Length(12));
        }
        if app.is_column_visible(Tab::MergeRequests, "Stack") {
            header_cells.push(Cell::from(
                header_line("Stack").alignment(Alignment::Center),
            ));
            widths.push(col_w(content_area.width, 14));
        }
        if app.is_column_visible(Tab::MergeRequests, "Milestone") {
            header_cells.push(header("Milestone"));
            widths.push(col_w(content_area.width, 18));
        }
        if app.is_column_visible(Tab::MergeRequests, "Closes") {
            header_cells.push(header("Closes"));
            widths.push(col_w(content_area.width, 16));
        }
        if app.is_column_visible(Tab::MergeRequests, "Author") {
            header_cells.push(header("Author"));
            widths.push(col_w(content_area.width, 18));
        }

        if widths.is_empty() {
            widths.push(Constraint::Min(0));
        }

        let table = Table::new(rows, widths)
            .header(Row::new(header_cells).style(header_style).height(1))
            .block(main_block)
            .row_highlight_style(Style::default())
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

                let fetching_issues = app.fetching_mr_related_issues.contains(&mr.iid);
                let doc = crate::entity_editor::build_mr_document(
                    mr,
                    is_github,
                    unresolved_threads,
                    fetching_issues,
                );
                let max_detail_scroll = super::inspector::render_entity_inspector(
                    f,
                    &doc,
                    detail_rect,
                    super::inspector::InspectorMode::ReadOnly {
                        scroll: app.detail_scroll,
                        title_suffix: "",
                    },
                    &app.label_colors,
                );
                settle_detail_scroll(app, max_detail_scroll);
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
        let default_set = std::collections::HashSet::new();
        let enabled_cols = app
            .enabled_columns
            .get(&Tab::Pipelines)
            .unwrap_or(&default_set);
        let mut filtered_pipelines = App::filter_pipelines_list(
            &app.pipelines.items,
            &app.search_query,
            &app.pipeline_jobs,
            enabled_cols,
        );
        App::apply_column_filters(
            &mut filtered_pipelines,
            &app.column_filters,
            Tab::Pipelines,
            App::pipeline_filter_values,
        );

        // Triggers that have not spawned a pipeline are appended as render-only
        // rows. They never enter `pipelines.items`, so nothing keyed by
        // pipeline id can see them, and the cursor — bounded by `items.len()` —
        // cannot land on them.
        let pending_rows: Vec<crate::domain::pipelines::Pipeline> = app
            .pending_triggers
            .iter()
            .map(|trigger| crate::domain::pipelines::Pipeline {
                id: 0,
                status: trigger.status.clone(),
                r#ref: String::new(),
                updated_at: String::new(),
                name: trigger.name.clone(),
                display_title: String::new(),
                event: String::new(),
                head_sha: String::new(),
                actor_login: String::new(),
                duration_seconds: None,
                created_at: None,
                source: Some("parent_pipeline".to_string()),
                project_path: String::new(),
                web_url: None,
                commit: None,
            })
            .collect();
        filtered_pipelines.extend(pending_rows.iter());

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
            } else {
                bg_color
            };
            let mut row_cells = Vec::new();
            if app.scope.is_group() && app.is_column_visible(Tab::Pipelines, "Project") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&p.project_path, 35),
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
                    Style::default().fg(theme.badge_group_bg),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Pipelines, "ID") {
                // A trigger that has not spawned has no pipeline to identify.
                let id_text = if p.id() == 0 {
                    "—".to_string()
                } else {
                    format!("#{}", p.id())
                };
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &id_text,
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
                let created_str = p.created_at().map(time_ago).unwrap_or_default();
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
            } else {
                Style::default()
            };
            let bar_cell = Cell::from(ratatui::text::Span::styled(
                " ",
                if is_checked {
                    Style::default().bg(theme.checked_bg)
                } else {
                    Style::default()
                },
            ));
            row_cells.insert(0, bar_cell);
            Row::new(row_cells).style(row_style).height(1)
        });

        let mut header_cells = Vec::new();
        let mut widths = Vec::new();

        header_cells.push(Cell::from(""));
        widths.push(Constraint::Length(1));

        let sort = app.sort_for(Tab::Pipelines);
        let header_line = |name: &str| Line::from(sort_header_label(name, sort, &icons));
        let header = |name: &str| Cell::from(header_line(name));

        if app.scope.is_group() && app.is_column_visible(Tab::Pipelines, "Project") {
            header_cells.push(header("Project"));
            widths.push(col_w(content_area.width, 30));
        }

        if app.is_column_visible(Tab::Pipelines, "ID") {
            header_cells.push(header("ID"));
            widths.push(Constraint::Length(8));
        }
        if app.is_column_visible(Tab::Pipelines, "Status") {
            header_cells.push(Cell::from(
                header_line("Status").alignment(Alignment::Center),
            ));
            widths.push(Constraint::Length(12));
        }
        if app.is_column_visible(Tab::Pipelines, "Stages") {
            header_cells.push(header("Stages"));
            widths.push(Constraint::Length(14));
        }
        if app.is_column_visible(Tab::Pipelines, "Source") {
            header_cells.push(header("Source"));
            widths.push(Constraint::Length(16));
        }
        if app.is_column_visible(Tab::Pipelines, "Name") {
            header_cells.push(header("Name"));
            widths.push(Constraint::Length(22));
        }
        if app.is_column_visible(Tab::Pipelines, "Event") {
            header_cells.push(header("Event"));
            widths.push(Constraint::Length(14));
        }
        if app.is_column_visible(Tab::Pipelines, "SHA") {
            header_cells.push(header("SHA"));
            widths.push(Constraint::Length(8));
        }
        if app.is_column_visible(Tab::Pipelines, "Actor") {
            header_cells.push(header("Actor"));
            widths.push(Constraint::Length(18));
        }
        if app.is_column_visible(Tab::Pipelines, "Created") {
            header_cells.push(header("Created"));
            widths.push(Constraint::Length(15));
        }
        if app.is_column_visible(Tab::Pipelines, "Duration") {
            header_cells.push(header("Duration"));
            widths.push(Constraint::Length(14));
        }
        if app.is_column_visible(Tab::Pipelines, "Ref") {
            header_cells.push(header("Ref"));
            widths.push(Constraint::Fill(1));
        }

        if widths.is_empty() {
            widths.push(Constraint::Min(0));
        }

        let table = Table::new(rows, widths)
            .header(Row::new(header_cells).style(header_style).height(1))
            .block(main_block)
            .row_highlight_style(Style::default())
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
                let doc = crate::entity_editor::build_pipeline_document(p, &jobs, is_github);
                let max_detail_scroll = super::inspector::render_entity_inspector(
                    f,
                    &doc,
                    detail_rect,
                    super::inspector::InspectorMode::ReadOnly {
                        scroll: app.detail_scroll,
                        title_suffix: "",
                    },
                    &app.label_colors,
                );
                settle_detail_scroll(app, max_detail_scroll);
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
            App::job_filter_values,
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
                    j.stage(),
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
                    j.name(),
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
                    truncate(&sanitize_untrusted(&needs_str), 25),
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
            } else {
                Style::default()
            };
            let bar_cell = Cell::from(ratatui::text::Span::styled(
                " ",
                if is_checked {
                    Style::default().bg(theme.checked_bg)
                } else {
                    Style::default()
                },
            ));
            row_cells.insert(0, bar_cell);
            Row::new(row_cells).style(row_style).height(1)
        });

        let mut header_cells = Vec::new();
        let mut widths = Vec::new();

        header_cells.push(Cell::from(""));
        widths.push(Constraint::Length(1));

        let sort = app.sort_for(Tab::Jobs);
        let header_line = |name: &str| Line::from(sort_header_label(name, sort, &icons));
        let header = |name: &str| Cell::from(header_line(name));

        if app.is_column_visible(Tab::Jobs, "ID") {
            header_cells.push(header("ID"));
            widths.push(Constraint::Length(8));
        }
        if app.is_column_visible(Tab::Jobs, "Stage") {
            header_cells.push(header("Stage"));
            widths.push(Constraint::Length(14));
        }
        if app.is_column_visible(Tab::Jobs, "Status") {
            header_cells.push(Cell::from(
                header_line("Status").alignment(Alignment::Center),
            ));
            widths.push(Constraint::Length(12));
        }
        if app.is_column_visible(Tab::Jobs, "Name") {
            header_cells.push(header("Name"));
            widths.push(Constraint::Fill(1));
        }
        if app.is_column_visible(Tab::Jobs, "Matrix") {
            header_cells.push(header("Matrix"));
            widths.push(Constraint::Length(20));
        }
        if app.is_column_visible(Tab::Jobs, "Runner") {
            header_cells.push(header("Runner"));
            widths.push(Constraint::Length(18));
        }
        if app.is_column_visible(Tab::Jobs, "Needs") {
            header_cells.push(header("Needs"));
            widths.push(Constraint::Length(14));
        }
        if app.is_column_visible(Tab::Jobs, "Duration") {
            header_cells.push(header("Duration"));
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
            .block(main_block.clone().title(format!(" {jobs_title} ")))
            .row_highlight_style(Style::default())
            .highlight_symbol(format!(" {} ", icons.highlight_arrow));

        let mut state = app.jobs.state;
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

        let shows_trace = app.job_trace_loading || app.job_trace.is_some();
        let selected_job = app
            .jobs
            .state
            .selected()
            .and_then(|selected| filtered_jobs.get(selected).copied());
        if shows_trace {
            let header = selected_job.map(|job| {
                crate::entity_editor::build_job_document(job, InspectorContent::Empty(""))
            });
            let trace_rect = match header {
                Some(header) => {
                    let header_height = u16::try_from(super::inspector::read_only_fields_height(
                        &header,
                        detail_rect.width,
                        &app.label_colors,
                    ))
                    .unwrap_or(u16::MAX)
                    .min(detail_rect.height / 2);
                    let [header_rect, trace_rect] =
                        Layout::vertical([Constraint::Length(header_height), Constraint::Fill(1)])
                            .areas(detail_rect);
                    super::inspector::render_entity_inspector(
                        f,
                        &header,
                        header_rect,
                        super::inspector::InspectorMode::ReadOnly {
                            scroll: 0,
                            title_suffix: "",
                        },
                        &app.label_colors,
                    );
                    trace_rect
                }
                None => detail_rect,
            };
            if app.job_trace_loading {
                f.render_widget(
                    Paragraph::new("\n\n  Loading job trace... (Press Esc to cancel)")
                        .alignment(Alignment::Center)
                        .block(job_trace_block("", &theme))
                        .style(Style::default().fg(theme.text_muted)),
                    trace_rect,
                );
            } else {
                render_job_trace(f, app, trace_rect, &theme);
            }
        } else {
            let summary =
                InspectorContent::Custom(job_summary_lines(&app.jobs.items, is_github, &theme));
            let document = match selected_job {
                Some(job) => crate::entity_editor::build_job_document(job, summary),
                None => EntityDocument {
                    title: String::new(),
                    fields: Vec::new(),
                    content: summary,
                },
            };
            let max_detail_scroll = super::inspector::render_entity_inspector(
                f,
                &document,
                detail_rect,
                super::inspector::InspectorMode::ReadOnly {
                    scroll: app.detail_scroll,
                    title_suffix: "",
                },
                &app.label_colors,
            );
            settle_detail_scroll(app, max_detail_scroll);
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

fn job_trace_block(title_suffix: &str, theme: &crate::config::Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(format!(" Trace{title_suffix} "))
        .title_style(
            Style::default()
                .fg(theme.text_muted)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(theme.border))
}

/// Draws the shown job log in `area`, scrolled to `app.detail_scroll` or to
/// the line a jump asked for.
fn render_job_trace(f: &mut Frame, app: &mut App, area: Rect, theme: &crate::config::Theme) {
    let width = area.width.saturating_sub(2) as usize;
    let height = area.height.saturating_sub(2) as usize;
    let wrap_width = app.job_trace_wrap.then_some(width);
    let Some(view) = app.job_trace.as_mut() else {
        return;
    };
    let total_lines = view.rows(theme, wrap_width);
    let is_truncated = view.trace().is_truncated();
    if let Some(line) = app.job_trace_scroll_to_line.take() {
        let row = view.jump_to_line(theme, wrap_width, line);
        app.detail_scroll = u16::try_from(row).unwrap_or(u16::MAX);
    }

    let max_scroll = u16::try_from(total_lines.saturating_sub(height)).unwrap_or(u16::MAX);
    settle_detail_scroll(app, max_scroll);

    let mut title_suffix = String::new();
    if total_lines > height {
        let percent = (app.detail_scroll as usize * 100) / max_scroll.max(1) as usize;
        title_suffix.push_str(&format!(" [j/k | {}%] ", percent.min(100)));
    }
    if is_truncated {
        title_suffix.push_str(&format!(
            " [TRUNCATED: first {} MiB]",
            crate::domain::job_trace::JOB_TRACE_MAX_BYTES / (1024 * 1024)
        ));
    }
    if !app.job_trace_search_query.is_empty() {
        title_suffix.push_str(&format!(" [Search: {}]", app.job_trace_search_query));
    }
    if app.job_trace_follow {
        title_suffix.push_str(" [FOLLOW]");
    }

    let scroll = usize::from(app.detail_scroll);
    let Some(view) = app.job_trace.as_mut() else {
        return;
    };
    let (visible_lines, rows_above) = view.visible_lines(theme, wrap_width, scroll, height);
    let mut paragraph = Paragraph::new(visible_lines)
        .block(job_trace_block(&title_suffix, theme))
        .scroll((u16::try_from(rows_above).unwrap_or(u16::MAX), 0));
    if app.job_trace_wrap {
        paragraph = paragraph.wrap(ratatui::widgets::Wrap { trim: false });
    }
    f.render_widget(paragraph, area);
}

/// Per-stage success rates on GitLab, per-job statuses on GitHub, whose jobs
/// have no stages.
fn job_summary_lines(
    jobs: &[crate::domain::pipelines::Job],
    is_github: bool,
    theme: &crate::config::Theme,
) -> Vec<Line<'static>> {
    let heading = if is_github {
        "Jobs Status:"
    } else {
        "Stages Success Rate:"
    };
    let mut lines = vec![
        Line::from(Span::styled(
            heading,
            Style::default()
                .fg(theme.header_fg)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    if is_github {
        super::helpers::append_job_summaries(&mut lines, jobs);
    } else {
        super::helpers::append_stage_summaries(&mut lines, jobs);
    }
    lines
}

pub(crate) fn render_tab_runners(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
    header_style: Style,
) {
    let theme = THEME.read().unwrap();
    let icons = crate::config::ICONS.read().unwrap();
    if app.scope.is_group() {
        let [list, preview] = group_scope_unavailable(app, &theme, &icons, "Runners", main_block);
        f.render_widget(list, content_area);
        f.render_widget(preview, detail_rect);
        return;
    }
    if app.runners.items().is_empty() && app.loading_tabs.contains(&app.active_tab) {
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
        app.runners.render(
            f,
            content_area,
            TableContext {
                block: main_block,
                header_style,
                highlight_style: Style::default().bg(theme.highlight_bg),
                theme: &theme,
                icons: &icons,
                query: &app.search_query,
            },
        );
        let doc = app
            .runners
            .cursor_row()
            .map(crate::entity_editor::build_runner_document);
        render_list_preview(f, app, detail_rect, doc, &theme, &icons.label_details);
    }
}
pub(crate) fn render_tab_releases(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
    header_style: Style,
) {
    let theme = THEME.read().unwrap();
    if super::render_edit_menu_if_active(f, app, detail_rect) {
        return;
    }
    let icons = crate::config::ICONS.read().unwrap();
    if app.scope.is_group() {
        let [list, preview] = group_scope_unavailable(app, &theme, &icons, "Releases", main_block);
        f.render_widget(list, content_area);
        f.render_widget(preview, detail_rect);
        return;
    }
    if app.releases.items().is_empty() && app.loading_tabs.contains(&app.active_tab) {
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
        app.releases.render(
            f,
            content_area,
            TableContext {
                block: main_block,
                header_style,
                highlight_style: Style::default().bg(theme.highlight_bg),
                theme: &theme,
                icons: &icons,
                query: &app.search_query,
            },
        );
        let doc = app
            .releases
            .cursor_row()
            .map(crate::entity_editor::build_release_document);
        render_list_preview(f, app, detail_rect, doc, &theme, &icons.label_details);
    }
}
pub(crate) fn render_tab_todos(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    detail_rect: Rect,
    main_block: Block<'_>,
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
        let kind = app.kind();
        let mut filtered_todos = App::filtered_todos_list(
            &app.todos.items,
            &app.search_query,
            &app.enabled_columns,
            app.group_ascending
                .get(&Tab::Todos)
                .copied()
                .unwrap_or(true),
            app.group_by_column.get(&Tab::Todos).unwrap_or(&None),
            kind,
        );
        App::apply_column_filters(
            &mut filtered_todos,
            &app.column_filters,
            Tab::Todos,
            |n, col| App::todo_filter_values(n, col, kind),
        );
        let rows = filtered_todos.iter().enumerate().map(|(idx, n)| {
            let is_row_highlighted = app.todos.state.selected() == Some(idx);
            let is_checked = app.selected_todos.contains(&n.id);
            let row_bg = if is_row_highlighted {
                theme.highlight_bg
            } else {
                ratatui::style::Color::Reset
            };
            let badge_cell = |label: &str, value: &str| {
                let (fg, bg, is_bold, text) = super::helpers::badge_style_for(
                    label,
                    value,
                    is_row_highlighted,
                    row_bg,
                    &theme,
                    &icons,
                );
                let mut style = Style::default().fg(fg).bg(bg.unwrap_or(row_bg));
                if is_bold {
                    style = style.add_modifier(Modifier::BOLD);
                }
                super::helpers::render_fuzzy_cell(
                    &text.unwrap_or_else(|| value.to_string()),
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
                    style,
                    Alignment::Center,
                )
            };

            let mut row_cells = vec![Cell::from(ratatui::text::Span::styled(
                " ",
                if is_checked {
                    Style::default().bg(theme.checked_bg)
                } else {
                    Style::default()
                },
            ))];
            if app.is_column_visible(Tab::Todos, "State") {
                row_cells.push(badge_cell("State", &n.state));
            }
            if app.is_column_visible(Tab::Todos, "Project") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &n.project_path,
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
                    Style::default().fg(theme.text_muted),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Todos, "Type") {
                row_cells.push(badge_cell("Type", n.type_label(kind)));
            }
            if app.is_column_visible(Tab::Todos, "ID") {
                let reference = n.reference(kind).unwrap_or_else(|| "—".to_string());
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &reference,
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
                    Style::default().fg(theme.blue),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Todos, "Title") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &truncate(&n.title, 80),
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
                    Style::default().fg(theme.text_normal),
                    Alignment::Left,
                ));
            }
            if app.is_column_visible(Tab::Todos, "Updated") {
                row_cells.push(super::helpers::render_fuzzy_cell(
                    &time_ago(&n.updated_at),
                    &app.search_query,
                    is_row_highlighted,
                    is_checked,
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

        let mut header_cells = vec![Cell::from("")];
        let mut widths = vec![Constraint::Length(1)];

        let sort = app.sort_for(Tab::Todos);
        let header_line = |name: &str| Line::from(sort_header_label(name, sort, &icons));
        let header = |name: &str| Cell::from(header_line(name));

        if app.is_column_visible(Tab::Todos, "State") {
            header_cells.push(Cell::from(
                header_line("State").alignment(Alignment::Center),
            ));
            widths.push(Constraint::Length(8));
        }
        if app.is_column_visible(Tab::Todos, "Project") {
            header_cells.push(header("Project"));
            widths.push(Constraint::Length(24));
        }
        if app.is_column_visible(Tab::Todos, "Type") {
            header_cells.push(Cell::from(header_line("Type").alignment(Alignment::Center)));
            widths.push(Constraint::Length(14));
        }
        if app.is_column_visible(Tab::Todos, "ID") {
            header_cells.push(header("ID"));
            widths.push(Constraint::Length(8));
        }
        if app.is_column_visible(Tab::Todos, "Title") {
            header_cells.push(header("Title"));
            widths.push(Constraint::Fill(1));
        }
        if app.is_column_visible(Tab::Todos, "Updated") {
            header_cells.push(header("Updated"));
            widths.push(Constraint::Length(16));
        }

        let table = Table::new(rows, widths)
            .header(Row::new(header_cells).style(header_style).height(1))
            .block(main_block)
            .row_highlight_style(Style::default())
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
                let doc = crate::entity_editor::build_todo_document(
                    n,
                    kind,
                    app.failed_todo_details.get(&n.id).map(String::as_str),
                );
                let max_detail_scroll = super::inspector::render_entity_inspector(
                    f,
                    &doc,
                    detail_rect,
                    super::inspector::InspectorMode::ReadOnly {
                        scroll: app.detail_scroll,
                        title_suffix: "",
                    },
                    &app.label_colors,
                );
                settle_detail_scroll(app, max_detail_scroll);
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
            &app.milestone_progress_cache,
        );
        App::apply_column_filters(
            &mut filtered_milestones,
            &app.column_filters,
            Tab::Milestones,
            App::milestone_filter_values,
        );

        let mut header_cells = Vec::new();
        let mut widths = Vec::new();
        let cols = Tab::Milestones.columns(app.kind(), app.scope.is_group());
        let sort = app.sort_for(Tab::Milestones);
        for col in &cols {
            if app.is_column_visible(Tab::Milestones, col) {
                header_cells.push(Cell::from(sort_header_label(col, sort, &icons)));
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
                            let (state_text, state_style) =
                                if crate::app::milestone_state_display(&m.state) == "ACTIVE" {
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
                            // Prefer the cheap aggregate derived from
                            // `Issue.milestone` (rebuilt on every IssuesFetched).
                            // Fall back to the per-milestone issue list when the
                            // user has drilled in. Never refetch: the bar must
                            // show whatever we already have, even if stale.
                            //
                            // `milestone_progress_cache` stores `(closed, total)`.
                            // If neither cache has data, render a dash.
                            let cached = app
                                .milestone_progress_cache
                                .get(&m.iid)
                                .copied()
                                .filter(|&(_, total)| total > 0);
                            let (closed, total) = cached.unwrap_or_else(|| {
                                app.milestone_issues_cache
                                    .get(&m.iid)
                                    .filter(|v| !v.is_empty())
                                    .map(|issues| {
                                        let total = issues.len();
                                        let closed =
                                            issues.iter().filter(|i| i.state == "closed").count();
                                        (closed, total)
                                    })
                                    .unwrap_or((0, 0))
                            });
                            let (bar_text, color) = if total > 0 {
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
                                let empty_len = bar_segments - filled_len;
                                (
                                    format!(
                                        "[{}{}] {:.0}%",
                                        "█".repeat(filled_len),
                                        "░".repeat(empty_len),
                                        pct,
                                    ),
                                    color,
                                )
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
            .row_highlight_style(Style::default())
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
                let progress = app.milestone_progress_cache.get(&m.iid).copied();
                let doc =
                    crate::entity_editor::build_milestone_document(m, issues, progress, is_github);
                let max_detail_scroll = super::inspector::render_entity_inspector(
                    f,
                    &doc,
                    detail_rect,
                    super::inspector::InspectorMode::ReadOnly {
                        scroll: app.detail_scroll,
                        title_suffix: "",
                    },
                    &app.label_colors,
                );
                settle_detail_scroll(app, max_detail_scroll);
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
    if app.scope.is_group() {
        let [list, preview] = group_scope_unavailable(app, &theme, &icons, "Branches", main_block);
        f.render_widget(list, content_area);
        f.render_widget(preview, detail_rect);
        return;
    }
    if app.branches.items().is_empty() && app.loading_tabs.contains(&app.active_tab) {
        f.render_widget(
            Paragraph::new(format!("\n\n {} Loading branches...", icons.label_loading))
                .alignment(Alignment::Center)
                .block(main_block.clone())
                .style(Style::default().fg(theme.text_muted)),
            content_area,
        );
    } else {
        app.branches.render(
            f,
            content_area,
            TableContext {
                block: main_block,
                header_style,
                highlight_style,
                theme: &theme,
                icons: &icons,
                query: &app.search_query,
            },
        );
        let doc = app
            .branches
            .cursor_row()
            .map(crate::entity_editor::build_branch_document);
        render_list_preview(f, app, detail_rect, doc, &theme, &icons.label_branch);
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
    if app.scope.is_group() {
        let [list, preview] =
            group_scope_unavailable(app, &theme, &icons, "Environments", main_block);
        f.render_widget(list, content_area);
        f.render_widget(preview, detail_rect);
        return;
    }
    if app.environments.items().is_empty() && app.loading_tabs.contains(&app.active_tab) {
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
        app.environments.render(
            f,
            content_area,
            TableContext {
                block: main_block,
                header_style,
                highlight_style,
                theme: &theme,
                icons: &icons,
                query: &app.search_query,
            },
        );
        let has_deployments = app
            .environment_deployments
            .as_ref()
            .is_some_and(|shown| !shown.table.items.is_empty());
        if has_deployments {
            render_environment_deployments(
                f,
                app,
                detail_rect,
                highlight_style,
                header_style,
                &theme,
                &icons,
            );
        } else {
            let keys = &app.config.keybindings.environments;
            let is_github = app.is_github();
            let doc = app
                .environments
                .cursor_row()
                .map(|env| crate::entity_editor::build_environment_document(env, keys, is_github));
            render_list_preview(f, app, detail_rect, doc, &theme, &icons.label_environment);
        }
    }
}

/// Rows the deployments table takes in the detail pane: all of them when
/// they fit in half the pane, so the selected deployment's details always
/// keep the other half.
fn deployments_table_height(deployment_count: usize, pane_height: u16) -> u16 {
    const BORDERS_AND_HEADER: u16 = 3;
    let wanted = u16::try_from(deployment_count)
        .unwrap_or(u16::MAX)
        .saturating_add(BORDERS_AND_HEADER);
    wanted.min(pane_height / 2).max(BORDERS_AND_HEADER + 1)
}

/// A column of the deployments table in the Environments detail pane.
#[derive(Clone, Copy)]
enum DeploymentColumn {
    Status,
    Id,
    Sha,
    Ref,
    Deployer,
    Created,
}

impl DeploymentColumn {
    /// Narrower panes drop the ID and deployer, which the details below
    /// the table show anyway, so the ref keeps room to be read.
    const WIDE: &'static [Self] = &[
        Self::Status,
        Self::Id,
        Self::Sha,
        Self::Ref,
        Self::Deployer,
        Self::Created,
    ];
    const NARROW: &'static [Self] = &[Self::Status, Self::Sha, Self::Created, Self::Ref];
    const WIDE_PANE_MIN_WIDTH: u16 = 80;

    fn fitting(pane_width: u16) -> &'static [Self] {
        if pane_width >= Self::WIDE_PANE_MIN_WIDTH {
            Self::WIDE
        } else {
            Self::NARROW
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Id => "ID",
            Self::Sha => "SHA",
            Self::Ref => "Ref",
            Self::Deployer => "Deployer",
            Self::Created => "Created",
        }
    }

    fn width(self) -> Constraint {
        match self {
            Self::Status => Constraint::Length(12),
            Self::Id => Constraint::Length(5),
            Self::Sha => Constraint::Length(8),
            Self::Ref => Constraint::Fill(1),
            Self::Deployer => Constraint::Length(16),
            Self::Created => Constraint::Length(13),
        }
    }

    fn cell(
        self,
        deployment: &crate::domain::deployments::Deployment,
        is_selected: bool,
        theme: &crate::config::Theme,
        icons: &crate::config::Icons,
    ) -> Cell<'static> {
        match self {
            Self::Status => {
                let (fg, bg, is_bold, text) = super::helpers::badge_style_for(
                    "Deploy Status",
                    &deployment.status,
                    is_selected,
                    theme.bg,
                    theme,
                    icons,
                );
                let mut style = Style::default().fg(fg);
                if let Some(bg) = bg {
                    style = style.bg(bg);
                }
                if is_bold {
                    style = style.add_modifier(Modifier::BOLD);
                }
                let text =
                    text.unwrap_or_else(|| sanitize_untrusted(&deployment.status).into_owned());
                Cell::from(Span::styled(text, style))
            }
            Self::Id => Cell::from(format!("#{}", deployment.iid)),
            Self::Sha => Cell::from(deployment.short_sha().to_string()),
            Self::Ref => Cell::from(sanitize_untrusted(&deployment.ref_name).into_owned()),
            Self::Deployer => Cell::from(
                deployment
                    .user
                    .as_ref()
                    .map(|user| format!("@{}", sanitize_untrusted(&user.username)))
                    .unwrap_or_default(),
            ),
            Self::Created => Cell::from(time_ago(&deployment.created_at)),
        }
    }
}

/// The shown deployments as a table over the selected deployment's details.
fn render_environment_deployments(
    f: &mut Frame,
    app: &mut App,
    detail_rect: Rect,
    highlight_style: Style,
    header_style: Style,
    theme: &crate::config::Theme,
    icons: &crate::config::Icons,
) {
    let Some(shown) = app.environment_deployments.as_mut() else {
        return;
    };
    let [table_rect, inspector_rect] = ratatui::layout::Layout::vertical([
        Constraint::Length(deployments_table_height(
            shown.table.items.len(),
            detail_rect.height,
        )),
        Constraint::Fill(1),
    ])
    .areas(detail_rect);

    let columns = DeploymentColumn::fitting(table_rect.width);
    let selected = shown.table.state.selected();
    let rows: Vec<Row> = shown
        .table
        .items
        .iter()
        .enumerate()
        .map(|(idx, deployment)| {
            let is_selected = selected == Some(idx) && shown.is_focused;
            Row::new(
                columns
                    .iter()
                    .map(|column| column.cell(deployment, is_selected, theme, icons)),
            )
        })
        .collect();
    let widths: Vec<Constraint> = columns.iter().map(|column| column.width()).collect();
    let (border_color, focus_hint) = if shown.is_focused {
        (theme.border_focused, "")
    } else {
        (theme.border, " (unfocused)")
    };
    let title = format!(
        " {} Deployments of {}{focus_hint} ",
        icons.label_deployment,
        sanitize_untrusted(&shown.environment)
    );
    let table = Table::new(rows, widths)
        .header(
            Row::new(
                columns
                    .iter()
                    .map(|column| Cell::from(column.label()).style(header_style)),
            )
            .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(border_color)),
        )
        .row_highlight_style(if shown.is_focused {
            highlight_style
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        })
        .highlight_symbol(format!(" {} ", icons.highlight_arrow));
    f.render_stateful_widget(table, table_rect, &mut shown.table.state);

    let doc = shown
        .selected()
        .map(crate::entity_editor::build_deployment_document);
    render_list_preview(f, app, inspector_rect, doc, theme, &icons.label_deployment);
}

pub(crate) fn render_tab_terminal(
    f: &mut Frame,
    app: &mut App,
    content_area: Rect,
    highlight_style: Style,
) {
    let theme = THEME.read().unwrap();
    let entries = app.filtered_terminal_commands();
    let num_cmds = entries.len();
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
        let full_text: String = entries
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

    let max_scroll = if app.terminal_wrap {
        total_lines.saturating_sub(log_height)
    } else {
        num_cmds.saturating_sub(1)
    };
    let scroll = app.terminal_scroll.min(max_scroll);

    let status_filter = app.terminal_status_filter;
    let filter_label = if status_filter == StatusFilter::All {
        String::new()
    } else {
        format!(" · {}", status_filter.label())
    };
    let filter_count = if app.search_query.trim().is_empty() && status_filter == StatusFilter::All {
        String::new()
    } else {
        format!(" [{}/{}]", num_cmds, app.terminal_commands.len())
    };
    let block_title = if app.terminal_wrap {
        format!(" Terminal (Wrap){filter_label}{filter_count} [{total_lines} lines] ")
    } else if scroll > 0 {
        format!(" Terminal (Scroll: {scroll}/{max_scroll}){filter_label}{filter_count} ")
    } else {
        format!(" Terminal{filter_label}{filter_count} ")
    };
    let custom_main_block = base_block.clone().title(block_title).title_style(
        Style::default()
            .fg(theme.header_fg)
            .add_modifier(Modifier::BOLD),
    );

    let paragraph = if app.terminal_wrap {
        let all_lines: Vec<Line> = entries
            .iter()
            .map(|cmd| super::helpers::build_log_line(cmd, usize::MAX))
            .collect();

        Paragraph::new(all_lines)
            .block(custom_main_block)
            .scroll((scroll as u16, 0))
            .wrap(ratatui::widgets::Wrap { trim: false })
    } else {
        let end_idx = num_cmds.saturating_sub(scroll);
        let start_idx = end_idx.saturating_sub(log_height);

        let mut log_lines = Vec::new();
        let visible_count = end_idx - start_idx;
        if visible_count < log_height {
            for _ in 0..(log_height - visible_count) {
                log_lines.push(Line::from(""));
            }
        }

        let cursor = end_idx.checked_sub(1);
        for (index, cmd) in entries.iter().enumerate().take(end_idx).skip(start_idx) {
            let line = super::helpers::build_log_line(cmd, width);
            log_lines.push(if Some(index) == cursor {
                line.patch_style(highlight_style)
            } else {
                line
            });
        }

        Paragraph::new(log_lines).block(custom_main_block)
    };
    app.terminal_scroll = scroll;
    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issues::{Author, Issue};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn render_tab_issues_clamps_detail_scroll_to_content_max() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.issues.items = vec![Issue {
            iid: 1,
            title: "Issue 1".to_string(),
            state: "opened".to_string(),
            labels: vec![],
            updated_at: String::new(),
            created_at: None,
            closed_at: None,
            author: Author {
                username: "u1".to_string(),
            },
            milestone: None,
            assignees: vec![],
            description: Some("line\n\n".repeat(100)),
            due_date: None,
            web_url: String::new(),
            project_path: String::new(),
            related_mrs: None,
        }];
        app.issues.state.select(Some(0));
        app.detail_scroll = 999;

        terminal
            .draw(|f| {
                let area = f.area();
                let content_area = Rect::new(0, 0, area.width, 10);
                let detail_rect = Rect::new(0, 10, area.width, 10);
                render_tab_issues(
                    f,
                    &mut app,
                    content_area,
                    detail_rect,
                    Block::default(),
                    Style::default(),
                );
            })
            .unwrap();

        // Pinned exact value for this fixture:
        //
        //   detail_rect      = Rect::new(0, 10, 100, 10)
        //   description      = Some("line\n\n".repeat(100))
        //   app.detail_scroll= 999  (input, before clamp)
        //
        // render_entity_inspector returns max_detail_scroll = total_lines -
        // visible_content_height for the description pane (see
        // src/ui/inspector.rs:1156). The exact integer pins the math that
        // converts rendered markdown line count + pane height into max_scroll —
        // so an off-by-one regression in rendered_line_count
        // (src/ui/helpers.rs:937) is caught here, not just in the pure-helper
        // unit tests. Update this number if the inspector layout or markdown
        // renderer changes.
        const EXPECTED_MAX_DETAIL_SCROLL: u16 = 196;

        assert_eq!(
            app.detail_scroll, EXPECTED_MAX_DETAIL_SCROLL,
            "detail_scroll should have been clamped to the content's exact max \
             for this fixture (100x10 detail_rect, description = \"line\\n\\n\".repeat(100))"
        );
        // Sanity floor: if the exact assertion ever breaks because the
        // inspector layout shifts, the old "< 999" check still tells us
        // clamping happened at all.
        assert!(app.detail_scroll < 999);
    }

    #[test]
    fn settle_detail_scroll_caps_scroll_to_max() {
        let mut app = App::default();
        app.detail_scroll = 50;

        settle_detail_scroll(&mut app, 10);

        assert_eq!(app.detail_scroll, 10);
    }

    #[test]
    fn settle_detail_scroll_leaves_scroll_below_max_unchanged() {
        let mut app = App::default();
        app.detail_scroll = 3;

        settle_detail_scroll(&mut app, 10);

        assert_eq!(app.detail_scroll, 3);
    }

    /// When the handler raises `detail_scroll_to_bottom`, the render pass
    /// snaps the scroll to `max` and clears the flag — leaving it standing
    /// would jump the next pane the user opens to its bottom as well.
    #[test]
    fn settle_detail_scroll_honours_the_jump_to_bottom_flag() {
        let mut app = App::default();
        app.detail_scroll = 5;
        app.detail_scroll_to_bottom = true;

        settle_detail_scroll(&mut app, 42);

        assert_eq!(app.detail_scroll, 42);
        assert!(!app.detail_scroll_to_bottom);
    }

    /// Regression test for the slice::repeat capacity overflow: when
    /// `milestone_progress_cache` had data, the renderer used to destructure
    /// `(total, closed)` from a tuple that actually held `(closed, total)`,
    /// so any milestone with open issues produced `filled_len > bar_segments`
    /// and `bar_segments - filled_len` underflowed into `usize::MAX`. Calling
    /// `"░".repeat(usize::MAX)` panicked with "capacity overflow".
    #[test]
    fn render_milestone_progress_handles_open_issues_without_panic() {
        use crate::domain::milestones::Milestone;
        let backend = TestBackend::new(160, 80);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.milestones.items = vec![Milestone {
            id: 0,
            iid: 7,
            title: "v1.0".to_string(),
            description: None,
            state: "active".to_string(),
            start_date: None,
            due_date: None,
            created_at: String::new(),
            project_path: String::new(),
        }];
        // 3 closed of 5 total — the case that triggered the panic.
        app.milestone_progress_cache.insert(7, (3, 5));
        app.milestones.state.select(Some(0));
        app.active_tab = Tab::Milestones;

        terminal
            .draw(|f| {
                let area = f.area();
                let content_area = Rect::new(0, 0, area.width, 40);
                let detail_rect = Rect::new(0, 40, area.width, 30);
                render_tab_milestones(
                    f,
                    &mut app,
                    content_area,
                    detail_rect,
                    Block::default(),
                    Style::default(),
                );
            })
            .unwrap();
    }

    /// Same regression for the all-closed case: filled_len == bar_segments,
    /// empty_len == 0 — must not panic on `String::repeat(0)`.
    #[test]
    fn render_milestone_progress_handles_all_closed_without_panic() {
        use crate::domain::milestones::Milestone;
        let backend = TestBackend::new(160, 80);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.milestones.items = vec![Milestone {
            id: 0,
            iid: 9,
            title: "v2.0".to_string(),
            description: None,
            state: "closed".to_string(),
            start_date: None,
            due_date: None,
            created_at: String::new(),
            project_path: String::new(),
        }];
        app.milestone_progress_cache.insert(9, (4, 4));
        app.milestones.state.select(Some(0));
        app.active_tab = Tab::Milestones;

        terminal
            .draw(|f| {
                let area = f.area();
                let content_area = Rect::new(0, 0, area.width, 40);
                let detail_rect = Rect::new(0, 40, area.width, 30);
                render_tab_milestones(
                    f,
                    &mut app,
                    content_area,
                    detail_rect,
                    Block::default(),
                    Style::default(),
                );
            })
            .unwrap();
    }

    #[test]
    fn render_tab_mrs_loading_text_is_host_aware() {
        use crate::backend::gh::GhBackend;
        use crate::backend::glab::GlabBackend;
        use crate::domain::client::GitlabClient;
        let backend = TestBackend::new(160, 80);
        let mut terminal = Terminal::new(backend).unwrap();

        // GitHub mode -> "Loading pull requests..."
        let mut app = App::default();
        app.gitlab_client = Some(GitlabClient {
            is_github: true,
            backend: Box::new(GhBackend::new()),
            tx: None,
            page_size: 30,
            api_per_page: 30,
            show_read_notifications: false,
        });
        app.active_tab = Tab::MergeRequests;
        app.loading_tabs.insert(Tab::MergeRequests);

        terminal
            .draw(|f| {
                let area = f.area();
                let content_area = Rect::new(0, 0, area.width, 40);
                let detail_rect = Rect::new(0, 40, area.width, 30);
                render_tab_merge_requests(
                    f,
                    &mut app,
                    content_area,
                    detail_rect,
                    Block::default(),
                    Style::default(),
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("Loading pull requests..."));

        // GitLab mode -> "Loading merge requests..."
        let mut app = App::default();
        app.gitlab_client = Some(GitlabClient {
            is_github: false,
            backend: Box::new(GlabBackend::new()),
            tx: None,
            page_size: 30,
            api_per_page: 30,
            show_read_notifications: false,
        });
        app.active_tab = Tab::MergeRequests;
        app.loading_tabs.insert(Tab::MergeRequests);

        terminal
            .draw(|f| {
                let area = f.area();
                let content_area = Rect::new(0, 0, area.width, 40);
                let detail_rect = Rect::new(0, 40, area.width, 30);
                render_tab_merge_requests(
                    f,
                    &mut app,
                    content_area,
                    detail_rect,
                    Block::default(),
                    Style::default(),
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("Loading merge requests..."));
    }

    #[test]
    fn test_selected_and_checked_row_buffer() {
        let backend = TestBackend::new(50, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.issues.items = vec![Issue {
            iid: 1,
            title: "Issue 1".to_string(),
            state: "opened".to_string(),
            labels: vec![],
            updated_at: String::new(),
            created_at: None,
            closed_at: None,
            author: Author {
                username: "alice".to_string(),
            },
            milestone: None,
            assignees: vec![],
            description: None,
            due_date: None,
            web_url: "".to_string(),
            project_path: "owner/repo".to_string(),
            related_mrs: None,
        }];
        app.issues.state.select(Some(0));
        app.selected_issues.insert(("owner/repo".to_string(), 1));

        terminal
            .draw(|f| {
                let area = f.area();
                let content_area = Rect::new(0, 0, area.width, 5);
                let detail_rect = Rect::new(0, 5, area.width, 5);
                let theme = THEME.read().unwrap();
                let header_style = Style::default().fg(theme.text_normal);
                render_tab_issues(
                    f,
                    &mut app,
                    content_area,
                    detail_rect,
                    Block::default(),
                    header_style,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let theme = THEME.read().unwrap();
        // Row 1 is the first data row.
        // x=3 is the leftmost selection bar cell (after 3-char highlight arrow gutter).
        assert_eq!(buffer[(3, 1)].bg, theme.checked_bg);
        // x=5 is the ID "#1" cell, which should have the cursor highlight background.
        assert_eq!(buffer[(5, 1)].bg, theme.highlight_bg);
    }

    fn jobs_app_showing_trace(trace: crate::domain::job_trace::JobTrace) -> App {
        let mut app = App::default();
        app.jobs.items = vec![crate::domain::pipelines::Job {
            id: 1,
            status: "success".to_string(),
            stage: "test".to_string(),
            name: "job1".to_string(),
            matrix: None,
            duration_seconds: None,
            runner: None,
            ..Default::default()
        }];
        app.job_trace = Some(crate::ui::job_trace::JobTraceView::new(trace));
        app
    }

    /// Renders the Jobs tab with the trace pane on rows 10-19.
    fn render_jobs_with_trace_pane(app: &mut App) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                let content_area = Rect::new(0, 0, area.width, 10);
                let detail_rect = Rect::new(0, 10, area.width, 10);
                render_tab_jobs(
                    f,
                    app,
                    content_area,
                    detail_rect,
                    Block::default(),
                    Style::default(),
                );
            })
            .unwrap();
        terminal
    }

    fn buffer_row(terminal: &Terminal<TestBackend>, y: u16) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn render_tab_jobs_clamps_page_scroll_past_trace_end() {
        let trace =
            crate::domain::job_trace::JobTrace::from_read("line\n".repeat(200).as_bytes(), false);
        let mut app = jobs_app_showing_trace(trace);
        app.detail_scroll = 999;

        let terminal = render_jobs_with_trace_pane(&mut app);

        assert_eq!(
            app.detail_scroll, 192,
            "detail_scroll should clamp to the trace's max_scroll, not keep counting past it"
        );
        assert_eq!(buffer_row(&terminal, 18), format!("│{:<98}│", "line"));
    }

    #[test]
    fn render_tab_jobs_marks_a_truncated_trace_in_the_pane_title() {
        let trace = crate::domain::job_trace::JobTrace::from_read(b"first part\n", true);
        let mut app = jobs_app_showing_trace(trace);

        let terminal = render_jobs_with_trace_pane(&mut app);

        let title = " Trace [TRUNCATED: first 16 MiB] ";
        assert_eq!(
            buffer_row(&terminal, 10),
            format!("┌{title}{}┐", "─".repeat(98 - title.len()))
        );
        assert_eq!(buffer_row(&terminal, 11), format!("│{:<98}│", "first part"));
    }

    #[test]
    fn render_tab_jobs_shows_the_selected_jobs_metadata_above_its_trace() {
        let trace = crate::domain::job_trace::JobTrace::from_read(b"first part\n", false);
        let mut app = jobs_app_showing_trace(trace);
        app.jobs.items[0].runner = Some("gitlab-runner-7".to_string());
        app.jobs.items[0].web_url = Some("https://gitlab.com/o/r/-/jobs/1".to_string());
        app.jobs.state.select(Some(0));
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                render_tab_jobs(
                    f,
                    &mut app,
                    Rect::new(0, 0, area.width, 10),
                    Rect::new(0, 10, area.width, 30),
                    Block::default(),
                    Style::default(),
                );
            })
            .unwrap();

        let rows = buffer_rows(&terminal);
        assert!(rows[10].contains("Preview: Job #1"), "{:?}", rows[10]);
        let header: Vec<(&str, &str)> = rows[11..17]
            .iter()
            .map(|row| {
                let cells: Vec<&str> = row
                    .split(['│', ' '])
                    .filter(|cell| !cell.is_empty())
                    .collect();
                (cells[1], cells[cells.len() - 1])
            })
            .collect();
        assert_eq!(
            header,
            [
                ("ID", "#1"),
                ("Name", "job1"),
                ("Status", "SUCCESS"),
                ("Stage", "test"),
                ("Runner", "gitlab-runner-7"),
                ("URL", "https://gitlab.com/o/r/-/jobs/1"),
            ]
        );
        assert_eq!(rows[18], format!("┌ Trace {}┐", "─".repeat(91)));
        assert_eq!(rows[19], format!("│{:<98}│", "first part"));
        // 30 rows less the 8-row header and the trace pane's two borders.
        assert_eq!(app.job_trace.as_ref().unwrap().viewport_height(), 20);
    }

    #[test]
    fn render_tab_jobs_header_keeps_a_wrapped_url_in_the_side_pane() {
        let trace = crate::domain::job_trace::JobTrace::from_read(b"first part\n", false);
        let mut app = jobs_app_showing_trace(trace);
        app.jobs.items[0].runner = Some("gitlab-runner-7".to_string());
        app.jobs.items[0].web_url = Some("https://gitlab.com/o/r/-/jobs/1".to_string());
        app.jobs.state.select(Some(0));
        let mut terminal = Terminal::new(TestBackend::new(45, 40)).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                render_tab_jobs(
                    f,
                    &mut app,
                    Rect::new(0, 0, area.width, 10),
                    Rect::new(0, 10, area.width, 30),
                    Block::default(),
                    Style::default(),
                );
            })
            .unwrap();

        // The 45-column side pane wraps the URL onto a second row, which the
        // header must leave room for.
        let rows = buffer_rows(&terminal);
        let url: Vec<&str> = rows[16]
            .split(['│', ' '])
            .filter(|cell| !cell.is_empty())
            .collect();
        assert_eq!(
            (url[1], url[url.len() - 1]),
            ("URL", "https://gitlab.com/o/r/-/jobs")
        );
        assert_eq!(rows[17], format!("│{:<43}│", format!("{:11}/1", "")));
        assert_eq!(rows[18], format!("└{}┘", "─".repeat(43)));
        assert_eq!(rows[19], format!("┌ Trace {}┐", "─".repeat(36)));
        assert_eq!(rows[20], format!("│{:<43}│", "first part"));
    }

    #[test]
    fn jump_to_trace_failure_scrolls_a_wrapped_trace_to_the_failure_row() {
        let mut log = format!("{}\nok\nERROR: Job failed: exit code 1\n", "x".repeat(250));
        log.push_str(&"after\n".repeat(50));
        let trace = crate::domain::job_trace::JobTrace::from_read(log.as_bytes(), false);
        let mut app = jobs_app_showing_trace(trace);
        app.job_trace_wrap = true;

        app.jump_to_trace_failure(None);
        let terminal = render_jobs_with_trace_pane(&mut app);

        // At 98 cells the first line takes rows 0-2 and "ok" row 3.
        assert_eq!(app.detail_scroll, 4);
        assert_eq!(
            buffer_row(&terminal, 11),
            format!("│{:<98}│", "ERROR: Job failed: exit code 1")
        );
    }

    #[test]
    fn jump_to_trace_failure_moves_past_failures_on_the_last_page_and_wraps() {
        let mut log = "fatal: early\n".to_string();
        log.push_str(&"ok\n".repeat(50));
        log.push_str("error: late\nok\nERROR: Job failed: exit code 1\n");
        let trace = crate::domain::job_trace::JobTrace::from_read(log.as_bytes(), false);
        let mut app = jobs_app_showing_trace(trace);

        // 54 lines in an 8-row pane: the last page starts at line 46, below
        // both late failures (lines 51 and 53).
        let mut scrolls = Vec::new();
        let mut search_starts = Vec::new();
        for _ in 0..3 {
            let after = app.job_trace.as_ref().unwrap().failure_search_start();
            app.jump_to_trace_failure(Some(after));
            render_jobs_with_trace_pane(&mut app);
            scrolls.push(app.detail_scroll);
            search_starts.push(app.job_trace.as_ref().unwrap().failure_search_start());
        }

        assert_eq!(scrolls, vec![46, 46, 0]);
        assert_eq!(search_starts, vec![51, 53, 0]);
    }

    fn buffer_rows(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        let width = buffer.area.width as usize;
        buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|c| c.symbol()).collect())
            .collect()
    }

    #[test]
    fn render_tab_jobs_strips_escape_sequences_from_needs() {
        let backend = TestBackend::new(160, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.jobs.items = vec![crate::domain::pipelines::Job {
            id: 1,
            status: "success".to_string(),
            stage: "test".to_string(),
            name: "deploy".to_string(),
            matrix: None,
            duration_seconds: None,
            runner: None,
            needs: vec![
                "\u{1b}[31mbuild\u{1b}[0m".to_string(),
                "\u{9b}2Jlint".to_string(),
            ],
            ..Default::default()
        }];
        app.enabled_columns
            .entry(Tab::Jobs)
            .or_default()
            .insert("Needs".to_string());

        terminal
            .draw(|f| {
                let area = f.area();
                render_tab_jobs(
                    f,
                    &mut app,
                    Rect::new(0, 0, area.width, 10),
                    Rect::new(0, 10, area.width, 10),
                    Block::default(),
                    Style::default(),
                );
            })
            .unwrap();

        let rows = buffer_rows(&terminal);
        let job_row = rows
            .iter()
            .find(|row| row.contains("deploy"))
            .expect("job row rendered");
        assert!(
            job_row.trim_end().ends_with(" build, lint"),
            "row: {job_row:?}"
        );
    }

    #[test]
    fn sorted_column_header_carries_the_sort_arrow() {
        let sort_desc = crate::config::ICONS.read().unwrap().sort_desc.clone();
        let backend = TestBackend::new(160, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.issues.items = vec![Issue {
            iid: 1,
            title: "Issue 1".to_string(),
            state: "opened".to_string(),
            labels: vec![],
            updated_at: String::new(),
            created_at: None,
            closed_at: None,
            author: Author {
                username: "u1".to_string(),
            },
            milestone: None,
            assignees: vec![],
            description: None,
            due_date: None,
            web_url: String::new(),
            project_path: String::new(),
            related_mrs: None,
        }];
        app.group_by_column
            .insert(Tab::Issues, Some("Title".to_string()));
        app.group_ascending.insert(Tab::Issues, false);
        app.environments
            .set_items(vec![crate::domain::deployments::Environment {
                id: 1,
                name: "production".to_string(),
                state: "available".to_string(),
                external_url: None,
                last_deployment: None,
            }]);
        app.group_by_column
            .insert(Tab::Environments, Some("Name".to_string()));
        app.group_ascending.insert(Tab::Environments, false);
        app.active_tab = Tab::Environments;
        app.sync_list_views();

        let headers = |terminal: &Terminal<TestBackend>| buffer_rows(terminal)[0].clone();
        terminal
            .draw(|f| {
                let area = f.area();
                render_tab_issues(
                    f,
                    &mut app,
                    Rect::new(0, 0, area.width, 10),
                    Rect::new(0, 10, area.width, 10),
                    Block::default(),
                    Style::default(),
                );
            })
            .unwrap();
        let issue_header = headers(&terminal);
        assert!(
            issue_header.contains(&format!("Title{sort_desc}")),
            "{issue_header}"
        );
        assert!(
            !issue_header.contains(&format!("ID{sort_desc}")),
            "{issue_header}"
        );

        terminal
            .draw(|f| {
                let area = f.area();
                render_tab_environments(
                    f,
                    &mut app,
                    Rect::new(0, 0, area.width, 10),
                    Rect::new(0, 10, area.width, 10),
                    Block::default(),
                    Style::default(),
                    Style::default(),
                );
            })
            .unwrap();
        let env_header = headers(&terminal);
        assert!(
            env_header.contains(&format!("Name{sort_desc}")),
            "{env_header}"
        );
    }

    #[test]
    fn render_tab_environments_strips_escape_sequences_from_url() {
        let backend = TestBackend::new(160, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.active_tab = Tab::Environments;
        app.environments
            .set_items(vec![crate::domain::deployments::Environment {
            id: 1,
            name: "production".to_string(),
            state: "available".to_string(),
            external_url: Some(
                "\u{1b}]8;;https://evil.example\u{7}https://prod.example\u{1b}]8;;\u{7}\u{1b}[2J"
                    .to_string(),
            ),
            last_deployment: None,
        }]);
        app.enabled_columns
            .entry(Tab::Environments)
            .or_default()
            .insert("URL".to_string());
        app.sync_list_views();

        terminal
            .draw(|f| {
                let area = f.area();
                render_tab_environments(
                    f,
                    &mut app,
                    Rect::new(0, 0, area.width, 10),
                    Rect::new(0, 10, area.width, 10),
                    Block::default(),
                    Style::default(),
                    Style::default(),
                );
            })
            .unwrap();

        let rows = buffer_rows(&terminal);
        let env_row = rows
            .iter()
            .find(|row| row.contains("production"))
            .expect("environment row rendered");
        let cells: Vec<_> = env_row.split_whitespace().collect();
        let icons = crate::config::ICONS.read().unwrap();
        assert_eq!(
            cells,
            [
                icons.highlight_arrow.as_str(),
                "production",
                icons.state_open.as_str(),
                "AVAILABLE",
                "N/A",
                "https://prod.example"
            ],
        );
    }

    #[test]
    fn unwrapped_terminal_scrolls_until_the_oldest_entry_is_the_copied_row() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        for command in ["Fetch issues: glab issue list", "Fetch MRs: glab mr list"] {
            app.terminal_commands
                .push(crate::command_log::TerminalCommand {
                    timestamp: String::new(),
                    command: command.to_string(),
                    status: "Success".to_string(),
                    ..Default::default()
                });
        }
        app.terminal_scroll = usize::MAX;

        terminal
            .draw(|f| {
                let area = f.area();
                render_tab_terminal(f, &mut app, area, Style::default());
            })
            .unwrap();

        assert_eq!(app.terminal_scroll, 1);
        assert_eq!(app.terminal_cursor(app.terminal_commands.len()), Some(0));
    }

    #[test]
    fn render_tab_jobs_saturates_follow_scroll_past_u16_rows() {
        let text: String = (0..70_000).map(|line| format!("{line}\n")).collect();
        let trace = crate::domain::job_trace::JobTrace::from_read(text.as_bytes(), false);
        let mut app = jobs_app_showing_trace(trace);
        app.detail_scroll_to_bottom = true;

        let terminal = render_jobs_with_trace_pane(&mut app);

        assert_eq!(app.detail_scroll, u16::MAX);
        assert_eq!(buffer_row(&terminal, 11), format!("│{:<98}│", "65535"));
    }
}
