#![allow(dead_code)]

mod diff;
mod helpers;
pub(crate) mod inspector;
pub(crate) mod modal;
mod overlays;
mod tabs;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use self::diff::{centered_rect_min, format_comment_with_suggestions};
use self::helpers::{build_log_line, highlight_fuzzy_match};
use self::modal::clear_area;
use self::overlays::render_overlays;
use crate::app::{App, DiffLine, Tab};
use crate::config::{ICONS, THEME};
use crate::utils::format::truncate;
use std::collections::HashSet;

/// Render the active edit/create menu as an interactive inspector into the
/// (full-zoom) detail pane — the same pane the read-only preview uses. Returns
/// true if a menu was rendered, in which case the caller should skip its normal
/// preview rendering.
pub(crate) fn render_edit_menu_if_active(f: &mut Frame, app: &mut App, detail_rect: Rect) -> bool {
    let Some(mut menu) = app.edit_menu.take() else {
        return false;
    };
    let label_colors = app.label_colors.clone();

    let is_bulk_edit = matches!(
        menu.entity_kind,
        crate::app::EditEntityKind::BulkEditIssues | crate::app::EditEntityKind::BulkEditMrs
    );
    let content = if is_bulk_edit {
        crate::app::InspectorContent::Custom(bulk_selection_lines(app))
    } else {
        crate::app::InspectorContent::Markdown(menu.get_description_value())
    };

    let doc = crate::app::EntityDocument {
        title: menu.title.clone(),
        fields: menu.fields.clone(),
        content,
    };

    inspector::render_entity_inspector(
        f,
        &doc,
        detail_rect,
        inspector::InspectorMode::Interactive { menu: &mut menu },
        &label_colors,
    );

    app.edit_menu = Some(menu);
    true
}

/// Descriptor-pane lines for a bulk-edit menu: one entry per selected entity,
/// `#iid` accented and the title in normal text, so the user can confirm the
/// exact mutation set before submitting.
fn bulk_selection_lines(app: &App) -> Vec<Line<'static>> {
    let theme = THEME.read().unwrap();
    app.bulk_selection_summary()
        .into_iter()
        .map(|(iid, title)| {
            Line::from(vec![
                Span::styled(
                    format!("#{iid} "),
                    Style::default()
                        .fg(theme.purple)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(title, Style::default().fg(theme.text_normal)),
            ])
        })
        .collect()
}

/// Render a vim/helix-style mode indicator in the right side of the top banner.
/// Shows `[NORMAL]` / `[PREVIEW]` / `[EDIT]` / `[CREATE]` with a colored
/// background.
fn render_mode_indicator(f: &mut Frame, app: &App, area: Rect) {
    let theme = THEME.read().unwrap();

    let (label, bg, fg) = if let Some(menu) = app.edit_menu.as_ref() {
        if menu.entity_kind.is_create() {
            // CREATE — distinct palette so the user knows they're filling
            // out a new entity, not editing an existing one.
            (" CREATE ", theme.green, theme.bg)
        } else {
            // EDIT — reuse the GROUPED badge palette (blue).
            (" EDIT ", theme.blue, theme.bg)
        }
    } else if app.select_mode {
        // SELECT — distinct palette (purple) for yazi-style select mode.
        (" SELECT ", theme.purple, theme.bg)
    } else if app.details_zoomed {
        // PREVIEW — reuse the SEARCHING badge palette (yellow).
        (" PREVIEW ", theme.yellow, theme.bg)
    } else {
        // NORMAL — reuse the GLAB-TUI header palette (border_focused).
        (" NORMAL ", theme.border_focused, theme.bg)
    };

    let line = Line::from(vec![Span::styled(
        label,
        Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD),
    )]);

    let indicator = Paragraph::new(line)
        .style(Style::default().bg(theme.bg))
        .alignment(Alignment::Right);
    f.render_widget(indicator, area);
}

/// Render a diff line's content with per-character fuzzy search highlighting.
fn render_diff_line_content(
    line: &DiffLine,
    base_style: Style,
    code_fg: Color,
    match_fg: Color,
) -> Vec<Span<'static>> {
    let code = if line.content.len() > 1 {
        &line.content[1..]
    } else {
        ""
    };

    let Some(ref indices) = line.fuzzy_indices else {
        return render_content_normal(line, base_style, code_fg, code);
    };

    let code_indices: HashSet<usize> = indices.iter().filter(|&&i| i > 0).map(|&i| i - 1).collect();

    if code_indices.is_empty() {
        return render_content_normal(line, base_style, code_fg, code);
    }

    let mut match_style = base_style;
    match_style = match_style
        .fg(match_fg)
        .add_modifier(Modifier::BOLD)
        .add_modifier(Modifier::UNDERLINED);

    if let Some(ref highlighted) = line.syntax_highlighted {
        merge_syntax_with_fuzzy(highlighted, &code_indices, base_style, match_style, code_fg)
    } else {
        let mut indices_vec: Vec<usize> = code_indices.into_iter().collect();
        indices_vec.sort_unstable();
        highlight_fuzzy_match(code, &indices_vec, base_style, match_style)
    }
}

fn render_content_normal(
    line: &DiffLine,
    base_style: Style,
    code_fg: Color,
    code: &str,
) -> Vec<Span<'static>> {
    if let Some(ref highlighted) = line.syntax_highlighted {
        highlighted
            .iter()
            .map(|(s, t)| {
                Span::styled(
                    t.clone(),
                    base_style
                        .fg(s.fg.unwrap_or(code_fg))
                        .add_modifier(s.add_modifier),
                )
            })
            .collect()
    } else {
        vec![Span::styled(code.to_string(), base_style)]
    }
}

fn merge_syntax_with_fuzzy(
    highlighted: &[(Style, String)],
    code_indices: &HashSet<usize>,
    base_style: Style,
    match_style: Style,
    code_fg: Color,
) -> Vec<Span<'static>> {
    let mut result = Vec::new();
    let mut pos: usize = 0;
    for (syn_style, text) in highlighted {
        let chars: Vec<char> = text.chars().collect();
        let mut buf = String::new();
        let mut in_match = false;
        for (j, &c) in chars.iter().enumerate() {
            let char_pos = pos + j;
            let is_match = code_indices.contains(&char_pos);
            if is_match != in_match && !buf.is_empty() {
                let s = if in_match {
                    // Keep the fuzzy match color (yellow) so matches stay visible
                    // over syntax coloring.
                    match_style.add_modifier(syn_style.add_modifier)
                } else {
                    base_style
                        .fg(syn_style.fg.unwrap_or(code_fg))
                        .add_modifier(syn_style.add_modifier)
                };
                result.push(Span::styled(buf.clone(), s));
                buf.clear();
            }
            in_match = is_match;
            buf.push(c);
        }
        if !buf.is_empty() {
            let s = if in_match {
                match_style.add_modifier(syn_style.add_modifier)
            } else {
                base_style
                    .fg(syn_style.fg.unwrap_or(code_fg))
                    .add_modifier(syn_style.add_modifier)
            };
            result.push(Span::styled(buf, s));
        }
        pos += chars.len();
    }
    result
}

pub fn render(f: &mut Frame, app: &mut App) {
    let size = f.area();

    // Paint full canvas with theme background so theme renders consistently regardless of terminal emulator defaults
    f.render_widget(
        Block::default().style(Style::default().bg(THEME.read().unwrap().bg)),
        size,
    );

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
    let icons = crate::config::ICONS.read().unwrap();
    let header_icon = if app.is_github() {
        &icons.header_github
    } else {
        &icons.header_gitlab
    };
    let mut title_spans = vec![
        Span::styled(
            format!(" {} GLAB-TUI ", header_icon),
            Style::default()
                .bg(THEME.read().unwrap().border_focused)
                .fg(THEME.read().unwrap().bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " {} {} ",
                &crate::config::ICONS.read().unwrap().separator,
                app.project_context
            ),
            Style::default()
                .fg(THEME.read().unwrap().text_normal)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(Some(group_col)) = app.group_by_column.get(&app.active_tab) {
        title_spans.push(Span::styled(
            format!(" {} GROUPED: {} ", icons.label_filtered, group_col),
            Style::default()
                .bg(THEME.read().unwrap().blue)
                .fg(THEME.read().unwrap().bg)
                .add_modifier(Modifier::BOLD),
        ));
        if app.is_typing_search || !app.search_query.is_empty() {
            title_spans.push(Span::raw(" "));
        }
    }
    if app.is_typing_search {
        title_spans.push(Span::styled(
            format!(" {} SEARCHING ", icons.label_searching),
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
            format!(" {} FILTERED ", icons.label_filtered),
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

    // Top banner split into left content and a right-aligned mode indicator.
    let banner = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(12)])
        .split(title_area);

    let title = Paragraph::new(Line::from(title_spans))
        .style(Style::default().bg(THEME.read().unwrap().bg))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(THEME.read().unwrap().border)),
        );
    f.render_widget(title, banner[0]);
    render_mode_indicator(f, app, banner[1]);

    // Middle: Sidebar | Main Area | Preview Area
    let can_zoom = app.active_tab != Tab::Pipelines || !app.jobs.items.is_empty();

    let sidebar_width = if size.width >= 80 && app.config.ui.sidebar_visible {
        Constraint::Length(app.config.ui.sidebar_width)
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

    // When an edit/create menu is active, force the detail pane to full-zoom
    // (exactly like the preview's zoomed state) so the form fills the workspace.
    let edit_zoomed = app.edit_menu.is_some();

    let middle_chunks_raw = if (app.details_zoomed && can_zoom) || edit_zoomed {
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
    if !app.detail_visible && !app.details_zoomed && !edit_zoomed {
        detail_rect = Rect::default();
    }

    // Split middle column vertically: main content area + compact terminal pane
    let term_height = if app.active_tab != Tab::Terminal
        && size.height >= 18
        && app.config.ui.terminal_pane_visible
    {
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

    app.sidebar_rect = Some(sidebar_rect);
    app.content_rect = Some(content_area);
    app.detail_rect = if detail_rect.width > 0 {
        Some(detail_rect)
    } else {
        None
    };

    // Sidebar: Tabs
    let kind = app.kind();
    let sidebar_items: Vec<ListItem> = app
        .available_tabs()
        .iter()
        .map(|t| {
            let title = format!(" {} ", t.title(kind).to_uppercase());
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
            .title(format!(" {} Navigation ", icons.label_navigation))
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().text_muted)
                    .add_modifier(Modifier::BOLD),
            ),
    );
    f.render_widget(sidebar, sidebar_rect);

    // Main Area Title
    let tab_title = format!(" {} ", app.active_tab.title(kind));
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
            .title(format!(" {} Terminal ", icons.label_terminal))
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
            .title(format!(" {} Fetching Diff ", icons.label_fetching))
            .title_style(
                Style::default()
                    .fg(THEME.read().unwrap().header_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.read().unwrap().border_focused))
            .style(Style::default().bg(THEME.read().unwrap().bg));

        let pr_label = if app.is_github() {
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

        clear_area(f, area);
        f.render_widget(paragraph, area);
    }

    if app.edit_menu.is_none() {
        if let Some(mut diff_view) = app.diff_view.take() {
            let area = centered_rect_min(95, 95, 30, 6, size);

            let unresolved_count = app.unresolved_threads_count();
            let unresolved_suffix = if unresolved_count > 0 {
                format!(
                    " [{} Unresolved Threads: {}] ",
                    &ICONS.read().unwrap().thread_unresolved,
                    unresolved_count
                )
            } else {
                String::new()
            };

            let title_suffix = if app.in_review_mode {
                format!(" [REVIEW MODE: ON ({} pending)] ", app.draft_comments.len())
            } else {
                String::new()
            };

            let pr_label = if app.is_github() {
                "Pull Request"
            } else {
                "Merge Request"
            };

            let mut title_spans = vec![
                Span::styled(
                    format!(" {} Diff #{} ", pr_label, diff_view.mr_iid),
                    Style::default()
                        .fg(THEME.read().unwrap().header_fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    unresolved_suffix,
                    Style::default().fg(THEME.read().unwrap().header_fg),
                ),
                Span::styled(
                    title_suffix,
                    Style::default().fg(THEME.read().unwrap().header_fg),
                ),
            ];
            if diff_view.search_active {
                title_spans.push(Span::styled(
                    format!(" {} SEARCHING ", &ICONS.read().unwrap().label_searching),
                    Style::default()
                        .bg(THEME.read().unwrap().yellow)
                        .fg(THEME.read().unwrap().bg)
                        .add_modifier(Modifier::BOLD),
                ));
                title_spans.push(Span::styled(
                    format!(" {}_ ", diff_view.search_query),
                    Style::default().fg(THEME.read().unwrap().yellow),
                ));
            } else if !diff_view.search_query.is_empty() {
                let match_info = format!(
                    " ({}/{})",
                    if diff_view.search_matches.is_empty() {
                        0
                    } else {
                        diff_view.search_cursor + 1
                    },
                    diff_view.search_matches.len()
                );
                title_spans.push(Span::styled(
                    format!(" {} FILTERED ", &ICONS.read().unwrap().label_filtered),
                    Style::default()
                        .bg(THEME.read().unwrap().yellow)
                        .fg(THEME.read().unwrap().bg)
                        .add_modifier(Modifier::BOLD),
                ));
                title_spans.push(Span::styled(
                    format!(" {}{} ", diff_view.search_query, match_info),
                    Style::default().fg(THEME.read().unwrap().yellow),
                ));
            }

            let outer_block = Block::default()
                .title(Line::from(title_spans))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.read().unwrap().border))
                .style(Style::default().bg(THEME.read().unwrap().bg));

            let inner_area = outer_block.inner(area);

            let main_chunks = {
                let file_tree_constraint = if diff_view.file_tree_visible {
                    Constraint::Percentage(25)
                } else {
                    Constraint::Length(0)
                };
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([file_tree_constraint, Constraint::Percentage(100)].as_ref())
                    .split(inner_area)
            };

            // 1. Render Files list on the left (only if visible)
            let files_rect = main_chunks[0];
            let files_list_height = (files_rect.height as usize).saturating_sub(2);
            let file_tree_visible = diff_view.file_tree_visible;

            // Adjust file tree scroll offset
            if file_tree_visible {
                if diff_view.selected_visible_idx < diff_view.file_tree_scroll_offset {
                    diff_view.file_tree_scroll_offset = diff_view.selected_visible_idx;
                } else if diff_view.selected_visible_idx
                    >= diff_view.file_tree_scroll_offset + files_list_height
                {
                    diff_view.file_tree_scroll_offset =
                        (diff_view.selected_visible_idx + 1).saturating_sub(files_list_height);
                }
            }

            let panel_inner_width = if file_tree_visible {
                (files_rect.width as usize).saturating_sub(2)
            } else {
                80
            };

            let files_list = if file_tree_visible {
                let (reviewed_count, total_files) = diff_view.review_progress();
                let files_title = if reviewed_count > 0 {
                    format!(
                        " {} {} {}/{}{} ",
                        icons.label_files,
                        &ICONS.read().unwrap().file_reviewed,
                        reviewed_count,
                        total_files,
                        if diff_view.hide_reviewed {
                            " (hidden)"
                        } else {
                            ""
                        }
                    )
                } else {
                    format!(" {} ", icons.label_files)
                };
                let files_block = Block::default()
                    .title(files_title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(if diff_view.focus_on_files {
                        THEME.read().unwrap().border_focused
                    } else {
                        THEME.read().unwrap().border
                    }));

                let mut file_items = Vec::new();
                let scroll_start = diff_view.file_tree_scroll_offset;
                let scroll_end =
                    (scroll_start + files_list_height).min(diff_view.visible_nodes.len());
                for i in scroll_start..scroll_end {
                    let node = &diff_view.visible_nodes[i];
                    let is_selected = i == diff_view.selected_visible_idx;

                    let indent = "  ".repeat(node.depth);
                    let indicator = if node.is_dir {
                        if node.is_expanded {
                            format!("{} ", &ICONS.read().unwrap().folder_expanded)
                        } else {
                            format!("{} ", &ICONS.read().unwrap().folder_collapsed)
                        }
                    } else if node.is_reviewed {
                        format!("{} ", &ICONS.read().unwrap().file_reviewed)
                    } else {
                        "  ".to_string()
                    };

                    let mut name_display = node.name.clone();
                    if node.is_dir {
                        name_display.push('/');
                    }

                    // Rename indicator: show old → new
                    if let Some(ref old_path) = node.old_file_path {
                        if let Some(ref new_path) = node.file_path {
                            let full = format!("{} → {}", old_path, new_path);
                            name_display = if full.chars().count() > panel_inner_width / 2 {
                                let old_base = old_path
                                    .rsplit_once('/')
                                    .map_or(old_path.as_str(), |(_, n)| n);
                                let new_base = new_path
                                    .rsplit_once('/')
                                    .map_or(new_path.as_str(), |(_, n)| n);
                                format!("{} → {}", old_base, new_base)
                            } else {
                                full
                            };
                        }
                    }

                    // Build stats (right-aligned, colored): only for files, not directories
                    let stats_str = if !node.is_dir {
                        let mut s = String::new();
                        if node.additions > 0 {
                            s.push_str(&format!(" +{}", node.additions));
                        }
                        if node.deletions > 0 {
                            s.push_str(&format!(" -{}", node.deletions));
                        }
                        if !s.is_empty() { Some(s) } else { None }
                    } else {
                        None
                    };

                    let unresolved_count = app.unresolved_threads_count_for_path(&node.path_id);
                    let count_suffix = if unresolved_count > 0 {
                        format!(
                            " ({} {})",
                            &ICONS.read().unwrap().thread_unresolved,
                            unresolved_count
                        )
                    } else {
                        String::new()
                    };

                    let stats_total_len = stats_str.as_ref().map_or(0, |s| s.chars().count())
                        + count_suffix.chars().count();

                    let prefix = format!(" {}{}", indent, indicator);
                    let (name_display_padded, padding) = crate::ui::helpers::diff_tree_row_layout(
                        panel_inner_width,
                        &prefix,
                        &name_display,
                        stats_total_len,
                    );
                    name_display = name_display_padded;

                    // Determine per-item style
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
                    } else if node.is_reviewed {
                        // Reviewed files (and fully reviewed directories) fade
                        // into the background so only pending work stands out.
                        Style::default()
                            .fg(THEME.read().unwrap().text_muted)
                            .add_modifier(Modifier::DIM)
                    } else if node.is_dir {
                        Style::default()
                            .fg(THEME.read().unwrap().blue)
                            .add_modifier(Modifier::BOLD)
                    } else if node.is_new_file {
                        Style::default()
                            .fg(THEME.read().unwrap().green)
                            .add_modifier(Modifier::BOLD)
                    } else if node.is_deleted_file {
                        Style::default()
                            .fg(THEME.read().unwrap().red)
                            .add_modifier(Modifier::BOLD)
                    } else if node.old_file_path.is_some() {
                        Style::default()
                            .fg(THEME.read().unwrap().yellow)
                            .add_modifier(Modifier::ITALIC)
                    } else {
                        Style::default().fg(THEME.read().unwrap().text_normal)
                    };

                    // Build colored line spans
                    let mut line_spans: Vec<Span> = Vec::new();
                    line_spans.push(Span::styled(prefix, item_style));
                    line_spans.push(Span::styled(name_display, item_style));
                    line_spans.push(Span::styled(padding, item_style));

                    // Render stats with separate colors
                    if let Some(ref s) = stats_str {
                        // Parse the stats string into colored spans
                        // Format: " +N -M" (additions first, then deletions, space-separated)
                        let parts: Vec<&str> = s.split_whitespace().collect();
                        let mut i = 0;
                        while i < parts.len() {
                            let part = parts[i];
                            if part.starts_with('+') {
                                line_spans.push(Span::styled(
                                    format!(" {}", part),
                                    Style::default()
                                        .fg(THEME.read().unwrap().diff_addition_fg)
                                        .add_modifier(Modifier::BOLD),
                                ));
                            } else if part.starts_with('-') {
                                line_spans.push(Span::styled(
                                    format!(" {}", part),
                                    Style::default()
                                        .fg(THEME.read().unwrap().diff_deletion_fg)
                                        .add_modifier(Modifier::BOLD),
                                ));
                            }
                            i += 1;
                        }
                    }

                    if !count_suffix.is_empty() {
                        line_spans.push(Span::styled(
                            count_suffix,
                            Style::default().fg(THEME.read().unwrap().text_muted),
                        ));
                    }

                    file_items.push(ListItem::new(Line::from(line_spans)));
                }
                List::new(file_items).block(files_block)
            } else {
                // Dummy empty list when tree is hidden
                List::new(Vec::<ListItem>::new())
            };

            // 2. Render Diff content on the right
            let diff_block = Block::default()
                .title(format!(" {} ", icons.label_diff))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if !diff_view.focus_on_files {
                    THEME.read().unwrap().border_focused
                } else {
                    THEME.read().unwrap().border
                }));

            let list_height = (main_chunks[1].height as usize).saturating_sub(2);

            let mut updated_diff_view = diff_view;
            // When tree is hidden, force focus to diff content
            if !file_tree_visible {
                updated_diff_view.focus_on_files = false;
            }
            updated_diff_view.viewport_height = list_height;
            let total_lines = if updated_diff_view.side_by_side {
                updated_diff_view.side_by_side_lines.len()
            } else {
                updated_diff_view.lines.len()
            };

            if updated_diff_view.cursor_idx < updated_diff_view.scroll_offset {
                updated_diff_view.scroll_offset = updated_diff_view.cursor_idx;
            } else if updated_diff_view.cursor_idx >= updated_diff_view.scroll_offset + list_height
            {
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

                    let num_width = updated_diff_view.line_number_width;
                    let gutter_bg = THEME.read().unwrap().diff_gutter_bg;
                    let marker_style = Style::default()
                        .fg(THEME.read().unwrap().yellow)
                        .add_modifier(Modifier::BOLD)
                        .bg(gutter_bg);
                    let num_style = Style::default()
                        .fg(THEME.read().unwrap().text_muted)
                        .bg(gutter_bg);
                    let sep_style = Style::default()
                        .fg(THEME.read().unwrap().diff_sep)
                        .bg(gutter_bg);

                    let sel_bg = if in_selection {
                        Some(THEME.read().unwrap().highlight_bg)
                    } else if updated_diff_view.search_matches.contains(&idx) {
                        Some(THEME.read().unwrap().yellow_bg)
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
                            Span::styled(
                                format!("{:>width$} ", old_str, width = num_width),
                                num_style,
                            ),
                            Span::styled("│ ", sep_style),
                        ]);

                        match line.line_type {
                            crate::app::DiffLineType::Deletion => {
                                let theme = THEME.read().unwrap();
                                let code_fg = theme.diff_deletion_fg;
                                let code_bg = theme.diff_deletion_bg;
                                let prefix_fg = theme.diff_deletion_fg;
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

                                let mut content_spans = render_diff_line_content(
                                    line,
                                    final_style,
                                    code_fg,
                                    THEME.read().unwrap().yellow,
                                );
                                left_spans.append(&mut content_spans);
                            }
                            crate::app::DiffLineType::Normal => {
                                let actual_bg = sel_bg.unwrap_or(THEME.read().unwrap().bg);
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

                                let mut content_spans = render_diff_line_content(
                                    line,
                                    final_style,
                                    THEME.read().unwrap().text_normal,
                                    THEME.read().unwrap().yellow,
                                );
                                left_spans.append(&mut content_spans);
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
                                            .fg(span_style
                                                .fg
                                                .unwrap_or(THEME.read().unwrap().purple))
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
                        left_spans.extend(crate::ui::helpers::empty_side_gutter_spans(
                            if is_cursor {
                                " ❯ "
                            } else if in_selection {
                                " ▐ "
                            } else {
                                "   "
                            },
                            marker_style,
                            num_style,
                            sep_style,
                            num_width,
                            sel_bg,
                        ));
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
                            Span::styled(
                                format!("{:>width$} ", new_str, width = num_width),
                                num_style,
                            ),
                            Span::styled("│ ", sep_style),
                        ]);

                        match line.line_type {
                            crate::app::DiffLineType::Addition => {
                                let theme = THEME.read().unwrap();
                                let code_fg = theme.diff_addition_fg;
                                let code_bg = theme.diff_addition_bg;
                                let prefix_fg = theme.diff_addition_fg;
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

                                let mut content_spans = render_diff_line_content(
                                    line,
                                    final_style,
                                    code_fg,
                                    THEME.read().unwrap().yellow,
                                );
                                right_spans.append(&mut content_spans);
                            }
                            crate::app::DiffLineType::Normal => {
                                let actual_bg = sel_bg.unwrap_or(THEME.read().unwrap().bg);
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

                                let mut content_spans = render_diff_line_content(
                                    line,
                                    final_style,
                                    THEME.read().unwrap().text_normal,
                                    THEME.read().unwrap().yellow,
                                );
                                right_spans.append(&mut content_spans);
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
                                            .fg(span_style
                                                .fg
                                                .unwrap_or(THEME.read().unwrap().purple))
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
                        right_spans.extend(crate::ui::helpers::empty_side_gutter_spans(
                            if is_cursor {
                                " ❯ "
                            } else if in_selection {
                                " ▐ "
                            } else {
                                "   "
                            },
                            marker_style,
                            num_style,
                            sep_style,
                            num_width,
                            sel_bg,
                        ));
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
                                    && sline.right.as_ref().and_then(|r| r.new_line_num)
                                        == c.line_num)
                                    || (c.old_line_num.is_some()
                                        && sline.left.as_ref().and_then(|l| l.old_line_num)
                                            == c.old_line_num))
                        })
                        .collect();

                    for comment in matching_comments {
                        let comment_style = Style::default()
                            .fg(THEME.read().unwrap().yellow)
                            .bg(THEME.read().unwrap().comment_draft_bg);

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
                                    let path_matches =
                                        sline.left.as_ref().map_or(false, |l| {
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
                                                && sline.left.as_ref().and_then(|l| {
                                                    l.old_line_num.map(|n| n as u64)
                                                }) == pos.old_line))
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

                    let num_width = updated_diff_view.line_number_width;
                    let gutter_bg = THEME.read().unwrap().diff_gutter_bg;

                    let marker_style = Style::default()
                        .fg(THEME.read().unwrap().yellow)
                        .add_modifier(Modifier::BOLD)
                        .bg(gutter_bg);

                    let num_style = Style::default()
                        .fg(THEME.read().unwrap().text_muted)
                        .bg(gutter_bg);

                    let sep_style = Style::default()
                        .fg(THEME.read().unwrap().diff_sep)
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
                        Span::styled(
                            format!("{:>width$} ", old_str, width = num_width),
                            num_style,
                        ),
                        Span::styled(
                            format!("{:>width$} ", new_str, width = num_width),
                            num_style,
                        ),
                        Span::styled("│ ", sep_style),
                    ];

                    let sel_bg = if in_selection {
                        Some(THEME.read().unwrap().highlight_bg)
                    } else if updated_diff_view.search_matches.contains(&idx) {
                        Some(THEME.read().unwrap().yellow_bg)
                    } else {
                        None
                    };

                    match line.line_type {
                        crate::app::DiffLineType::Addition | crate::app::DiffLineType::Deletion => {
                            let theme = THEME.read().unwrap();
                            let is_add = line.line_type == crate::app::DiffLineType::Addition;
                            let code_fg = if is_add {
                                theme.diff_addition_fg
                            } else {
                                theme.diff_deletion_fg
                            };
                            let code_bg = if is_add {
                                theme.diff_addition_bg
                            } else {
                                theme.diff_deletion_bg
                            };
                            let prefix_fg = if is_add {
                                theme.diff_addition_fg
                            } else {
                                theme.diff_deletion_fg
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

                            let mut content_spans = render_diff_line_content(
                                line,
                                final_style,
                                code_fg,
                                THEME.read().unwrap().yellow,
                            );
                            line_spans.append(&mut content_spans);
                        }
                        crate::app::DiffLineType::Normal => {
                            let actual_bg = sel_bg.unwrap_or(THEME.read().unwrap().bg);
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

                            let mut content_spans = render_diff_line_content(
                                line,
                                final_style,
                                THEME.read().unwrap().text_normal,
                                THEME.read().unwrap().yellow,
                            );
                            line_spans.append(&mut content_spans);
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
                            .bg(THEME.read().unwrap().comment_draft_bg);

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

            clear_area(f, area);
            f.render_widget(outer_block, area);
            if file_tree_visible {
                f.render_widget(files_list, main_chunks[0]);
            }

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
                            Style::default().fg(THEME.read().unwrap().diff_sep),
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
    }

    render_overlays(f, app, size);

    if let Some(msg) = &app.error_message {
        if app.error_message_at.is_none() {
            app.error_message_at = Some(std::time::Instant::now());
        }
        if let Some(at) = app.error_message_at {
            if at.elapsed() > std::time::Duration::from_secs(5) {
                app.error_message = None;
                app.error_message_at = None;
            }
        }
    }
    if let Some(ref msg) = app.error_message {
        let theme = THEME.read().unwrap();
        let icons = ICONS.read().unwrap();

        // Inner content: "  <icon> <msg>  "
        let label = format!("  {} {}  ", icons.status_failed, msg);
        // Box width: content + 2 border chars, capped to terminal width
        let inner_w = (label.chars().count() as u16).min(size.width.saturating_sub(4));
        let box_w = inner_w + 2;
        // Anchor near the bottom but above the last row so the border is never clipped
        let box_h = 3u16;
        let box_x = size.x + (size.width.saturating_sub(box_w)) / 2;
        let box_y = size.height.saturating_sub(box_h + 1);
        let toast_area = Rect::new(box_x, box_y, box_w, box_h);

        // Clear + paint background so it sits cleanly over whatever is below
        use ratatui::widgets::{BorderType, Wrap};
        f.render_widget(ratatui::widgets::Clear, toast_area);
        f.render_widget(
            Block::default().style(Style::default().bg(theme.red_bg)),
            toast_area,
        );

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.red))
            .style(Style::default().bg(theme.red_bg));
        let inner = block.inner(toast_area);
        f.render_widget(block, toast_area);

        // Truncate label to inner width so it never overflows the border
        let display = truncate(&label, inner.width as usize);
        let toast = Paragraph::new(display.as_str())
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.red).add_modifier(Modifier::BOLD))
            .wrap(Wrap { trim: false });
        f.render_widget(toast, inner);
    }
}
