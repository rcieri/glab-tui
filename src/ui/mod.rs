pub mod compaction;
pub mod dashboard;
mod diff;
mod form;
mod helpers;
pub(crate) mod inspector;
pub mod job_trace;
pub mod layout;
pub mod list_tab;
pub(crate) mod modal;
mod overlays;
pub(crate) mod render_ctx;
mod review_submit;
mod review_threads;
mod tabs;
mod terminal_inspector;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use self::diff::{CommentAnchor, centered_rect_min, format_comment_with_suggestions};
use self::helpers::{build_log_line, highlight_fuzzy_match};
use self::modal::clear_area;
use self::overlays::render_overlays;
use crate::app::{App, DiffLine, Tab};
use crate::config::Theme;
use crate::ui::render_ctx::RenderCtx;
use crate::utils::format::{sanitize_untrusted, truncate};
use std::collections::HashSet;
use std::sync::Arc;

/// Rows a modal gives to a notice about its own request, below its content.
pub const MODAL_NOTICE_ROWS: u16 = 1;

/// Render the active edit/create menu as an interactive inspector into the
/// (full-zoom) detail pane — the same pane the read-only preview uses. Returns
/// true if a menu was rendered, in which case the caller should skip its normal
/// preview rendering.
pub(crate) fn render_edit_menu_if_active(
    f: &mut Frame,
    ctx: &RenderCtx<'_>,
    app: &mut App,
    detail_rect: Rect,
) -> bool {
    if app.edit_menu().is_none() {
        return false;
    }
    let label_colors = app.label_colors.clone();

    let is_bulk_edit = app.edit_menu().is_some_and(|m| {
        matches!(
            m.entity_kind,
            crate::app::EditEntityKind::BulkEditIssues | crate::app::EditEntityKind::BulkEditMrs
        )
    });
    let content = if is_bulk_edit {
        crate::app::InspectorContent::Custom(bulk_selection_lines(app, &ctx.theme))
    } else {
        crate::app::InspectorContent::Markdown(
            app.edit_menu()
                .map_or(String::new(), |m| m.get_description_value()),
        )
    };

    let Some(menu) = app.edit_menu_mut() else {
        return false;
    };

    let doc = crate::app::EntityDocument {
        title: menu.title.clone(),
        fields: menu.fields.clone(),
        content,
    };

    inspector::render_entity_inspector(
        f,
        ctx,
        &doc,
        detail_rect,
        inspector::InspectorMode::Interactive { menu },
        &label_colors,
    );

    true
}

/// Descriptor-pane lines for a bulk-edit menu: one entry per selected entity,
/// `#iid` accented and the title in normal text, so the user can confirm the
/// exact mutation set before submitting.
fn bulk_selection_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
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
fn render_mode_indicator(f: &mut Frame, ctx: &RenderCtx<'_>, app: &App, area: Rect) {
    let theme = &ctx.theme;

    let (label, bg, fg) = if let Some(menu) = app.edit_menu() {
        if menu.entity_kind.is_create() {
            // CREATE — distinct palette so the user knows they're filling
            // out a new entity, not editing an existing one.
            (" CREATE ", theme.green, theme.highlight_bg)
        } else {
            // EDIT — reuse the GROUPED badge palette (blue).
            (" EDIT ", theme.blue, theme.highlight_bg)
        }
    } else if app.select_mode {
        // SELECT — distinct palette (purple) for yazi-style select mode.
        (" SELECT ", theme.purple, theme.highlight_bg)
    } else if app.details_zoomed {
        // PREVIEW — reuse the SEARCHING badge palette (yellow).
        (" PREVIEW ", theme.yellow, theme.highlight_bg)
    } else {
        // NORMAL — reuse the GLAB-TUI header palette (border_focused).
        (" NORMAL ", theme.border_focused, theme.highlight_bg)
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

/// What sets a diff row apart from the rows around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowHighlight {
    Plain,
    Selected,
    SearchMatch,
}

impl RowHighlight {
    fn row_bg(self, theme: &Theme) -> Option<Color> {
        match self {
            RowHighlight::Plain => None,
            RowHighlight::Selected => Some(theme.highlight_bg),
            RowHighlight::SearchMatch => Some(theme.diff_search_bg),
        }
    }

    /// The foreground of every token on the row. Diff and syntax colors are
    /// picked for the row's own background, and on a search background
    /// (often a bright palette yellow) they can be unreadable.
    fn row_fg(self, theme: &Theme) -> Option<Color> {
        (self == RowHighlight::SearchMatch).then_some(theme.diff_search_fg)
    }
}

/// Render a diff line's content: syntax colors (not on a search match), fuzzy
/// search matches and, on an unhighlighted row, the emphasis background on the
/// words that differ from its paired line.
fn render_diff_line_content(
    line: &DiffLine,
    base_style: Style,
    theme: &Theme,
    highlight: RowHighlight,
) -> Vec<Span<'static>> {
    let spans = render_highlighted_content(line, base_style, theme, highlight);
    if highlight != RowHighlight::Plain {
        return spans;
    }
    let emphasis_bg = match line.line_type {
        crate::app::DiffLineType::Addition => theme.diff_addition_emph_bg,
        crate::app::DiffLineType::Deletion => theme.diff_deletion_emph_bg,
        _ => return spans,
    };
    // A palette color or the terminal's own background cannot be blended into
    // a stronger one, so the emphasis background can equal the row's and would
    // mark nothing.
    if base_style.bg == Some(emphasis_bg) {
        emphasize_ranges(spans, &line.changed_ranges, |style| {
            style.add_modifier(Modifier::REVERSED)
        })
    } else {
        emphasize_ranges(spans, &line.changed_ranges, |style| style.bg(emphasis_bg))
    }
}

/// Splits `spans`, which together hold one line, at the edges of `ranges`
/// (byte ranges of that line) and restyles the parts inside them with
/// `emphasize`.
fn emphasize_ranges(
    spans: Vec<Span<'static>>,
    ranges: &[std::ops::Range<usize>],
    emphasize: impl Fn(Style) -> Style,
) -> Vec<Span<'static>> {
    if ranges.is_empty() {
        return spans;
    }
    let mut out = Vec::with_capacity(spans.len() + 2 * ranges.len());
    let mut span_start = 0;
    for span in spans {
        let text = span.content.as_ref();
        let span_end = span_start + text.len();
        let mut cut = 0;
        for range in ranges
            .iter()
            .filter(|r| r.start < span_end && r.end > span_start)
        {
            let start = range.start.max(span_start) - span_start;
            let end = range.end.min(span_end) - span_start;
            push_piece(&mut out, text, cut..start, span.style);
            push_piece(&mut out, text, start..end, emphasize(span.style));
            cut = end;
        }
        push_piece(&mut out, text, cut..text.len(), span.style);
        span_start = span_end;
    }
    out
}

fn push_piece(
    out: &mut Vec<Span<'static>>,
    text: &str,
    range: std::ops::Range<usize>,
    style: Style,
) {
    if let Some(piece) = text.get(range).filter(|piece| !piece.is_empty()) {
        out.push(Span::styled(piece.to_string(), style));
    }
}

/// Render a diff line's content with per-character fuzzy search highlighting.
fn render_highlighted_content(
    line: &DiffLine,
    base_style: Style,
    theme: &Theme,
    highlight: RowHighlight,
) -> Vec<Span<'static>> {
    let code = line.code();
    let syntax = match highlight {
        RowHighlight::SearchMatch => None,
        RowHighlight::Plain | RowHighlight::Selected => line.syntax_spans(theme),
    };

    let code_indices: HashSet<usize> = line
        .fuzzy_indices
        .iter()
        .flatten()
        .filter(|&&i| i > 0)
        .map(|&i| i - 1)
        .collect();

    if code_indices.is_empty() {
        return render_content_normal(syntax, base_style, code);
    }

    let match_style = base_style
        .fg(theme.diff_search_fg)
        .add_modifier(Modifier::BOLD)
        .add_modifier(Modifier::UNDERLINED);

    if let Some(highlighted) = syntax {
        merge_syntax_with_fuzzy(highlighted, &code_indices, base_style, match_style)
    } else {
        let mut indices_vec: Vec<usize> = code_indices.into_iter().collect();
        indices_vec.sort_unstable();
        highlight_fuzzy_match(code, &indices_vec, base_style, match_style)
    }
}

fn render_content_normal<'a>(
    syntax: Option<impl Iterator<Item = (Style, &'a str)>>,
    base_style: Style,
    code: &str,
) -> Vec<Span<'static>> {
    if let Some(highlighted) = syntax {
        highlighted
            .map(|(s, t)| Span::styled(t.to_string(), syntax_style_on_row(base_style, s)))
            .collect()
    } else {
        vec![Span::styled(code.to_string(), base_style)]
    }
}

/// A syntax color equal to the row background would hide the token, so the
/// token keeps the row's foreground instead.
fn syntax_style_on_row(row_style: Style, syntax: Style) -> Style {
    let syntax = if syntax.fg == row_style.bg {
        Style { fg: None, ..syntax }
    } else {
        syntax
    };
    row_style.patch(syntax)
}

fn merge_syntax_with_fuzzy<'a>(
    highlighted: impl Iterator<Item = (Style, &'a str)>,
    code_indices: &HashSet<usize>,
    base_style: Style,
    match_style: Style,
) -> Vec<Span<'static>> {
    let mut result = Vec::new();
    let mut pos: usize = 0;
    for (syn_style, text) in highlighted {
        // Keep the search match color so matches stay visible over syntax
        // coloring.
        let style_for = |in_match: bool| {
            if in_match {
                match_style.add_modifier(syn_style.add_modifier)
            } else {
                syntax_style_on_row(base_style, syn_style)
            }
        };
        let mut buf = String::new();
        let mut in_match = false;
        for (j, c) in text.chars().enumerate() {
            let is_match = code_indices.contains(&(pos + j));
            if is_match != in_match && !buf.is_empty() {
                result.push(Span::styled(std::mem::take(&mut buf), style_for(in_match)));
            }
            in_match = is_match;
            buf.push(c);
        }
        if !buf.is_empty() {
            result.push(Span::styled(buf, style_for(in_match)));
        }
        pos += text.chars().count();
    }
    result
}

pub fn render(f: &mut Frame, app: &mut App) {
    let icons = Arc::clone(&app.icons);
    let ctx = &RenderCtx::new(app, &icons);
    let size = f.area();
    let cursor_marker = format!(" {} ", ctx.icons.pointer);

    // Paint full canvas with theme background so theme renders consistently regardless of terminal emulator defaults
    f.render_widget(
        Block::default().style(Style::default().bg(ctx.theme.bg)),
        size,
    );

    let plan = app.compute_layout(size);
    app.layout = Some(plan);
    app.content_rect = Some(plan.content_area);
    app.detail_rect = plan.detail_rect_opt();

    // Minimum terminal size guard
    if plan.is_too_small {
        let msg = format!(
            "Terminal too small — resize to at least {}×{}",
            layout::MIN_TERMINAL_WIDTH,
            layout::MIN_TERMINAL_HEIGHT
        );
        f.render_widget(
            Paragraph::new(msg)
                .alignment(Alignment::Center)
                .style(Style::default().fg(ctx.theme.red)),
            size,
        );
        return;
    }

    // Top: Title & Context
    let icons = ctx.icons;
    let header_icon = if ctx.kind.is_github() {
        &icons.header_github
    } else {
        &icons.header_gitlab
    };
    let mut title_spans = vec![
        Span::styled(
            format!(" {header_icon} GLAB-TUI "),
            Style::default()
                .bg(ctx.theme.border_focused)
                .fg(ctx.theme.highlight_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} {} ", ctx.icons.separator, app.scope_label()),
            Style::default()
                .fg(ctx.theme.text_normal)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some((group_col, ascending)) = app.sort_for(app.active_tab) {
        title_spans.push(Span::styled(
            format!(
                " {} GROUPED: {} {} ",
                icons.label_filtered,
                group_col,
                if ascending { "asc" } else { "desc" }
            ),
            Style::default()
                .bg(ctx.theme.blue)
                .fg(ctx.theme.highlight_bg)
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
                .bg(ctx.theme.yellow)
                .fg(ctx.theme.highlight_bg)
                .add_modifier(Modifier::BOLD),
        ));
        title_spans.push(Span::styled(
            format!(" {}_ ", app.search_query),
            Style::default().fg(ctx.theme.yellow),
        ));
    } else if !app.search_query.is_empty() {
        title_spans.push(Span::styled(
            format!(" {} FILTERED ", icons.label_filtered),
            Style::default()
                .bg(ctx.theme.yellow)
                .fg(ctx.theme.highlight_bg)
                .add_modifier(Modifier::BOLD),
        ));
        title_spans.push(Span::styled(
            format!(" {} ", app.search_query),
            Style::default().fg(ctx.theme.yellow),
        ));
    }
    if let Some(stale) = app.stale_tabs.get(&app.active_tab) {
        let theme = &ctx.theme;
        title_spans.push(Span::raw(" "));
        title_spans.push(Span::styled(
            format!(" {} ", stale.label()),
            Style::default()
                .bg(theme.red)
                .fg(theme.highlight_bg)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(status) = app.header_notice() {
        let text = crate::utils::format::sanitize_untrusted(status.text());
        title_spans.push(Span::raw(" "));
        title_spans.push(Span::styled(
            format!(" {text} "),
            Style::default().fg(ctx.theme.text_muted),
        ));
    }

    let title = Paragraph::new(Line::from(title_spans))
        .style(Style::default().bg(ctx.theme.bg))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(ctx.theme.border)),
        );
    f.render_widget(title, plan.banner_title);
    render_mode_indicator(f, ctx, app, plan.banner_mode);

    let sidebar_rect = plan.sidebar_rect;
    let content_area = plan.content_area;
    let detail_rect = plan.detail_rect;

    // Sidebar: Tabs
    let kind = ctx.kind;
    let sidebar_items: Vec<ListItem> = app
        .available_tabs()
        .iter()
        .map(|t| {
            let title = format!(" {} ", t.title(kind, ctx.icons).to_uppercase());
            if *t == app.shown_tab() {
                ListItem::new(title).style(
                    Style::default()
                        .bg(ctx.theme.border_focused)
                        .fg(ctx.theme.highlight_bg)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ListItem::new(title).style(Style::default().fg(ctx.theme.text_muted))
            }
        })
        .collect();

    let sidebar = List::new(sidebar_items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ctx.theme.border))
            .title(format!(" {} Navigation ", icons.label_navigation))
            .title_style(
                Style::default()
                    .fg(ctx.theme.text_muted)
                    .add_modifier(Modifier::BOLD),
            ),
    );
    f.render_widget(sidebar, sidebar_rect);

    // Main Area Title
    // Inside a pipeline descent the tab name alone no longer says where you
    // are, so the path down is appended. Jobs shows the opened pipeline's
    // path. The Todos tab shows which items its read filter lets through.
    let shown_tab = app.shown_tab();
    let breadcrumb = match shown_tab {
        Tab::Dashboard => app.scope.display(),
        Tab::Pipelines => app.nav_breadcrumb(),
        Tab::Jobs => app.jobs_breadcrumb(),
        Tab::Todos if app.shows_read_todos() => "All".to_string(),
        Tab::Todos => "Unread".to_string(),
        _ => String::new(),
    };
    let tab_title = if breadcrumb.is_empty() {
        format!(" {} ", shown_tab.title(kind, ctx.icons))
    } else {
        format!(" {} › {} ", shown_tab.title(kind, ctx.icons), breadcrumb)
    };
    let main_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.focus_column_checklist() {
            ctx.theme.border
        } else {
            ctx.theme.border_focused
        }))
        .title(tab_title)
        .title_style(
            Style::default()
                .fg(ctx.theme.header_fg)
                .add_modifier(Modifier::BOLD),
        );

    if let Some(table) = app.dashboard.queue_table {
        dashboard::render_queue_table(
            f,
            ctx,
            app,
            table,
            [content_area, detail_rect],
            main_block.clone(),
        );
    } else {
        match app.active_tab {
            Tab::Issues => {
                tabs::render_tab_issues(f, ctx, app, content_area, detail_rect, main_block.clone())
            }
            Tab::MergeRequests => tabs::render_tab_merge_requests(
                f,
                ctx,
                app,
                content_area,
                detail_rect,
                main_block.clone(),
            ),
            Tab::Pipelines => tabs::render_tab_pipelines(
                f,
                ctx,
                app,
                content_area,
                detail_rect,
                main_block.clone(),
            ),
            Tab::Jobs => {
                tabs::render_tab_jobs(f, ctx, app, content_area, detail_rect, main_block.clone())
            }
            Tab::Runners => {
                tabs::render_tab_runners(f, ctx, app, content_area, detail_rect, main_block.clone())
            }
            Tab::Releases => tabs::render_tab_releases(
                f,
                ctx,
                app,
                content_area,
                detail_rect,
                main_block.clone(),
            ),
            Tab::Todos => {
                tabs::render_tab_todos(f, ctx, app, content_area, detail_rect, main_block.clone())
            }
            Tab::Milestones => tabs::render_tab_milestones(
                f,
                ctx,
                app,
                content_area,
                detail_rect,
                main_block.clone(),
            ),
            Tab::Branches => tabs::render_tab_branches(
                f,
                ctx,
                app,
                content_area,
                detail_rect,
                main_block.clone(),
            ),
            Tab::Environments => tabs::render_tab_environments(
                f,
                ctx,
                app,
                content_area,
                detail_rect,
                main_block.clone(),
            ),
            Tab::Terminal => tabs::render_tab_terminal(f, ctx, app, content_area),
            Tab::Dashboard => {
                dashboard::render_tab_dashboard(f, ctx, app, content_area, main_block.clone());
            }
        }
    }

    // Compact terminal pane at bottom of the middle column
    if let Some(term_area) = plan.terminal_area_opt() {
        let bottom_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ctx.theme.border))
            .title(format!(" {} Terminal ", icons.label_terminal))
            .title_style(
                Style::default()
                    .fg(ctx.theme.purple)
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

            for cmd in app.terminal_commands.iter().skip(start_idx) {
                log_lines.push(build_log_line(ctx, cmd, bottom_inner.width as usize));
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
                    .fg(ctx.theme.header_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ctx.theme.border_focused))
            .style(Style::default().bg(ctx.theme.bg));

        let pr_label = if ctx.kind.is_github() {
            "Pull Request"
        } else {
            "Merge Request"
        };
        let text = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("   Fetching {pr_label} Diff..."),
                Style::default().fg(ctx.theme.text_normal),
            )),
            Line::from(Span::styled(
                "   Please wait, running CLI tool in background...",
                Style::default().fg(ctx.theme.text_muted),
            )),
            Line::from(""),
        ];

        let paragraph = Paragraph::new(text)
            .block(block)
            .alignment(Alignment::Left)
            .wrap(ratatui::widgets::Wrap { trim: true });

        clear_area(f, ctx, area);
        f.render_widget(paragraph, area);
    }

    if app.edit_menu().is_none() && app.diff_view().is_some() {
        let diff_layout = app.diff_layout;
        let unresolved_count = app.unresolved_threads_count();
        let unresolved_suffix = if unresolved_count > 0 {
            format!(
                " [{} Unresolved Threads: {}] ",
                ctx.icons.thread_unresolved, unresolved_count
            )
        } else {
            String::new()
        };

        let in_review_mode = app.in_review_mode;
        let draft_comments_len = app.draft_comments.len();
        let title_suffix = if in_review_mode {
            format!(" [REVIEW MODE: ON ({draft_comments_len} pending)] ")
        } else {
            String::new()
        };

        let App {
            modal_stack,
            draft_comments,
            current_comments,
            ..
        } = app;

        if let Some(crate::app::Modal::Diff(diff_view)) = modal_stack
            .iter_mut()
            .rev()
            .find(|m| matches!(m, crate::app::Modal::Diff(_)))
        {
            let diff_view = &mut **diff_view;
            let area = centered_rect_min(95, 95, 30, 6, size);

            let outer_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ctx.theme.border))
                .style(Style::default().bg(ctx.theme.bg));

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

            // Auto picks the layout from the pane width, which only the draw
            // knows, and before the title, whose search count depends on the
            // layout's rows. A selection holds the layout: switching would drop
            // it, and the layout catches up once it ends.
            let diff_columns = main_chunks[1].width.saturating_sub(2);
            if diff_view.selection_start.is_none() {
                diff_view.set_side_by_side(diff_layout.is_side_by_side(diff_columns));
            }
            let pr_label = if ctx.kind.is_github() {
                "Pull Request"
            } else {
                "Merge Request"
            };

            let mut title_spans = vec![
                Span::styled(
                    format!(" {} Diff #{} ", pr_label, diff_view.mr_iid),
                    Style::default()
                        .fg(ctx.theme.header_fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(unresolved_suffix, Style::default().fg(ctx.theme.header_fg)),
                Span::styled(title_suffix, Style::default().fg(ctx.theme.header_fg)),
            ];
            if diff_view.search_active {
                title_spans.push(Span::styled(
                    format!(" {} SEARCHING ", ctx.icons.label_searching),
                    Style::default()
                        .bg(ctx.theme.yellow)
                        .fg(ctx.theme.highlight_bg)
                        .add_modifier(Modifier::BOLD),
                ));
                title_spans.push(Span::styled(
                    format!(" {}_ ", diff_view.search_query),
                    Style::default().fg(ctx.theme.yellow),
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
                    format!(" {} FILTERED ", ctx.icons.label_filtered),
                    Style::default()
                        .bg(ctx.theme.yellow)
                        .fg(ctx.theme.highlight_bg)
                        .add_modifier(Modifier::BOLD),
                ));
                title_spans.push(Span::styled(
                    format!(" {}{} ", diff_view.search_query, match_info),
                    Style::default().fg(ctx.theme.yellow),
                ));
            }

            let outer_block = outer_block.title(Line::from(title_spans));

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
                        ctx.icons.file_reviewed,
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
                        ctx.theme.border_focused
                    } else {
                        ctx.theme.border
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
                            format!("{} ", ctx.icons.folder_expanded)
                        } else {
                            format!("{} ", ctx.icons.folder_collapsed)
                        }
                    } else if node.is_reviewed {
                        format!("{} ", ctx.icons.file_reviewed)
                    } else {
                        "  ".to_string()
                    };

                    let mut name_display = node.name.clone();
                    if node.is_dir {
                        name_display.push('/');
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

                    let unresolved_count = App::unresolved_threads_count_for_path_slice(
                        current_comments,
                        &node.path_id,
                    );
                    let count_suffix = if unresolved_count > 0 {
                        format!(" ({} {})", ctx.icons.thread_unresolved, unresolved_count)
                    } else {
                        String::new()
                    };

                    let stats_total_len = stats_str.as_ref().map_or(0, |s| s.chars().count())
                        + count_suffix.chars().count();

                    let prefix = format!(" {indent}{indicator}");
                    if let (Some(old_path), Some(new_path)) = (&node.old_file_path, &node.file_path)
                    {
                        name_display = crate::ui::helpers::rename_label(
                            old_path,
                            new_path,
                            panel_inner_width
                                .saturating_sub(prefix.chars().count() + stats_total_len),
                        );
                    }
                    let (name_display_padded, padding) = crate::ui::helpers::diff_tree_row_layout(
                        panel_inner_width,
                        &prefix,
                        &name_display,
                        stats_total_len,
                    );
                    name_display = name_display_padded;

                    // Determine base foreground color and style based on node type
                    let fg_color = if node.is_reviewed {
                        ctx.theme.text_muted
                    } else if node.is_dir {
                        ctx.theme.blue
                    } else if node.is_new_file {
                        ctx.theme.green
                    } else if node.is_deleted_file {
                        ctx.theme.red
                    } else if node.old_file_path.is_some() {
                        ctx.theme.yellow
                    } else {
                        ctx.theme.text_normal
                    };

                    let mut item_style = Style::default().fg(fg_color);
                    let sel_bg = if is_selected {
                        let bg = if diff_view.focus_on_files {
                            ctx.theme.highlight_bg
                        } else {
                            ctx.theme.border
                        };
                        item_style = item_style.bg(bg).add_modifier(Modifier::BOLD);
                        Some(bg)
                    } else {
                        if node.is_reviewed {
                            item_style = item_style.add_modifier(Modifier::DIM);
                        } else if node.is_dir || node.is_new_file || node.is_deleted_file {
                            item_style = item_style.add_modifier(Modifier::BOLD);
                        } else if node.old_file_path.is_some() {
                            item_style = item_style.add_modifier(Modifier::ITALIC);
                        }
                        None
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
                                let mut stat_style = Style::default()
                                    .fg(ctx.theme.diff_addition_fg)
                                    .add_modifier(Modifier::BOLD);
                                if let Some(bg) = sel_bg {
                                    stat_style = stat_style.bg(bg);
                                }
                                line_spans.push(Span::styled(format!(" {part}"), stat_style));
                            } else if part.starts_with('-') {
                                let mut stat_style = Style::default()
                                    .fg(ctx.theme.diff_deletion_fg)
                                    .add_modifier(Modifier::BOLD);
                                if let Some(bg) = sel_bg {
                                    stat_style = stat_style.bg(bg);
                                }
                                line_spans.push(Span::styled(format!(" {part}"), stat_style));
                            }
                            i += 1;
                        }
                    }

                    if !count_suffix.is_empty() {
                        let mut suffix_style = Style::default().fg(ctx.theme.text_muted);
                        if let Some(bg) = sel_bg {
                            suffix_style = suffix_style.bg(bg);
                        }
                        line_spans.push(Span::styled(count_suffix, suffix_style));
                    }

                    file_items.push(ListItem::new(Line::from(line_spans)));
                }
                List::new(file_items).block(files_block)
            } else {
                // Dummy empty list when tree is hidden
                List::new(Vec::<ListItem>::new())
            };

            // 2. Render Diff content on the right
            let layout_name = if diff_view.side_by_side {
                "side-by-side"
            } else {
                "unified"
            };
            let layout_choice = if app.diff_layout == crate::app::DiffLayout::Auto {
                "auto"
            } else {
                "pinned"
            };
            let diff_block = Block::default()
                .title(format!(
                    " {} ({layout_name}, {layout_choice}) ",
                    icons.label_diff
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if !diff_view.focus_on_files {
                    ctx.theme.border_focused
                } else {
                    ctx.theme.border
                }));

            let list_height = (main_chunks[1].height as usize).saturating_sub(2);

            let updated_diff_view = diff_view;
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
                        .is_some_and(|(s, e)| idx >= s && idx <= e);

                    let num_width = updated_diff_view.line_number_width;
                    let gutter_bg = ctx.theme.diff_gutter_bg;
                    let marker_style = Style::default()
                        .fg(ctx.theme.yellow)
                        .add_modifier(Modifier::BOLD)
                        .bg(gutter_bg);
                    let num_style = Style::default().fg(ctx.theme.text_muted).bg(gutter_bg);
                    let sep_style = Style::default().fg(ctx.theme.diff_sep).bg(gutter_bg);

                    let highlight = if in_selection {
                        RowHighlight::Selected
                    } else if updated_diff_view.search_matches.contains(&idx) {
                        RowHighlight::SearchMatch
                    } else {
                        RowHighlight::Plain
                    };
                    let sel_bg = highlight.row_bg(&ctx.theme);
                    let search_fg = highlight.row_fg(&ctx.theme);

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
                                    cursor_marker.as_str()
                                } else if in_selection {
                                    " ▐ "
                                } else {
                                    "   "
                                },
                                marker_style,
                            ),
                            Span::styled(format!("{old_str:>num_width$} "), num_style),
                            Span::styled("│ ", sep_style),
                        ]);

                        match line.line_type {
                            crate::app::DiffLineType::Deletion => {
                                let theme = &ctx.theme;
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
                                        .fg(search_fg.unwrap_or(prefix_fg))
                                        .add_modifier(Modifier::BOLD)
                                        .bg(actual_bg),
                                ));

                                let content_base = Style::default()
                                    .fg(search_fg.unwrap_or(code_fg))
                                    .bg(actual_bg);
                                let final_style = if is_cursor {
                                    content_base
                                        .add_modifier(Modifier::UNDERLINED)
                                        .add_modifier(Modifier::BOLD)
                                } else {
                                    content_base
                                };

                                let mut content_spans =
                                    render_diff_line_content(line, final_style, theme, highlight);
                                left_spans.append(&mut content_spans);
                            }
                            crate::app::DiffLineType::Normal => {
                                let actual_bg = sel_bg.unwrap_or(ctx.theme.bg);
                                let prefix = line
                                    .content
                                    .chars()
                                    .next()
                                    .map(|c| c.to_string())
                                    .unwrap_or_else(|| " ".to_string());
                                left_spans.push(Span::styled(
                                    prefix,
                                    Style::default().fg(ctx.theme.text_muted).bg(actual_bg),
                                ));

                                let content_base = Style::default()
                                    .fg(search_fg.unwrap_or(ctx.theme.text_normal))
                                    .bg(actual_bg);
                                let final_style = if is_cursor {
                                    content_base
                                        .add_modifier(Modifier::UNDERLINED)
                                        .add_modifier(Modifier::BOLD)
                                } else {
                                    content_base
                                };

                                let theme = &ctx.theme;
                                let mut content_spans =
                                    render_diff_line_content(line, final_style, theme, highlight);
                                left_spans.append(&mut content_spans);
                            }
                            crate::app::DiffLineType::Meta => {
                                let mut s = Style::default()
                                    .fg(search_fg.unwrap_or(ctx.theme.blue))
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
                                left_spans.push(Span::styled(&line.content, final_style));
                            }
                            crate::app::DiffLineType::HunkHeader => {
                                let mut s = Style::default()
                                    .fg(search_fg.unwrap_or(ctx.theme.purple))
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
                                left_spans.push(Span::styled(&line.content, final_style));
                            }
                            _ => {}
                        }
                    } else {
                        left_spans.extend(crate::ui::helpers::empty_side_gutter_spans(
                            if is_cursor {
                                cursor_marker.as_str()
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
                                    cursor_marker.as_str()
                                } else if in_selection {
                                    " ▐ "
                                } else {
                                    "   "
                                },
                                marker_style,
                            ),
                            Span::styled(format!("{new_str:>num_width$} "), num_style),
                            Span::styled("│ ", sep_style),
                        ]);

                        match line.line_type {
                            crate::app::DiffLineType::Addition => {
                                let theme = &ctx.theme;
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
                                        .fg(search_fg.unwrap_or(prefix_fg))
                                        .add_modifier(Modifier::BOLD)
                                        .bg(actual_bg),
                                ));

                                let content_base = Style::default()
                                    .fg(search_fg.unwrap_or(code_fg))
                                    .bg(actual_bg);
                                let final_style = if is_cursor {
                                    content_base
                                        .add_modifier(Modifier::UNDERLINED)
                                        .add_modifier(Modifier::BOLD)
                                } else {
                                    content_base
                                };

                                let mut content_spans =
                                    render_diff_line_content(line, final_style, theme, highlight);
                                right_spans.append(&mut content_spans);
                            }
                            crate::app::DiffLineType::Normal => {
                                let actual_bg = sel_bg.unwrap_or(ctx.theme.bg);
                                let prefix = line
                                    .content
                                    .chars()
                                    .next()
                                    .map(|c| c.to_string())
                                    .unwrap_or_else(|| " ".to_string());
                                right_spans.push(Span::styled(
                                    prefix,
                                    Style::default().fg(ctx.theme.text_muted).bg(actual_bg),
                                ));

                                let content_base = Style::default()
                                    .fg(search_fg.unwrap_or(ctx.theme.text_normal))
                                    .bg(actual_bg);
                                let final_style = if is_cursor {
                                    content_base
                                        .add_modifier(Modifier::UNDERLINED)
                                        .add_modifier(Modifier::BOLD)
                                } else {
                                    content_base
                                };

                                let theme = &ctx.theme;
                                let mut content_spans =
                                    render_diff_line_content(line, final_style, theme, highlight);
                                right_spans.append(&mut content_spans);
                            }
                            crate::app::DiffLineType::Meta => {
                                let mut s = Style::default()
                                    .fg(search_fg.unwrap_or(ctx.theme.blue))
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
                                right_spans.push(Span::styled(&line.content, final_style));
                            }
                            crate::app::DiffLineType::HunkHeader => {
                                let mut s = Style::default()
                                    .fg(search_fg.unwrap_or(ctx.theme.purple))
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
                                right_spans.push(Span::styled(&line.content, final_style));
                            }
                            _ => {}
                        }
                    } else {
                        right_spans.extend(crate::ui::helpers::empty_side_gutter_spans(
                            if is_cursor {
                                cursor_marker.as_str()
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
                    let matching_comments: Vec<_> = draft_comments
                        .iter()
                        .filter(|c| {
                            let path_matches = sline
                                .left
                                .as_ref()
                                .is_some_and(|l| l.file_path == c.file_path)
                                || sline
                                    .right
                                    .as_ref()
                                    .is_some_and(|r| r.file_path == c.file_path);

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
                            .fg(ctx.theme.yellow)
                            .bg(ctx.theme.comment_draft_bg);

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
                            .fg(ctx.theme.yellow)
                            .add_modifier(Modifier::BOLD);

                        let bubble = &ctx.icons.comment_bubble;
                        let right_prefix_first = format!(" {bubble} Draft Note{range_info}: ");
                        let left_prefix_first = format!(" {bubble} Draft ");
                        let left_prefix_rest = " ".repeat(Span::raw(&left_prefix_first).width());

                        let formatted_lines = format_comment_with_suggestions(
                            ctx,
                            &comment.body,
                            &CommentAnchor {
                                file_path: &comment.file_path,
                                start_new: comment.line_num.map(|n| n as u64),
                                end_new: comment.end_line_num.map(|n| n as u64),
                                start_old: comment.old_line_num.map(|n| n as u64),
                                end_old: comment.end_old_line_num.map(|n| n as u64),
                            },
                            &updated_diff_view.all_lines,
                            &right_prefix_first,
                            prefix_style,
                        );

                        for (i, (right_prefix, prefix_style, content_spans)) in
                            formatted_lines.into_iter().enumerate()
                        {
                            let left_prefix = if i == 0 {
                                &left_prefix_first
                            } else {
                                &left_prefix_rest
                            };

                            left_list_lines.push(
                                Line::from(vec![
                                    Span::styled("         ", Style::default()),
                                    Span::styled(left_prefix.clone(), prefix_style),
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

                    let matching_current: Vec<_> = current_comments
                        .iter()
                        .filter(|c| {
                            !c.system
                                && c.position.as_ref().is_some_and(|pos| {
                                    crate::app::note_position_anchors(
                                        pos,
                                        sline.left.as_ref(),
                                        sline.right.as_ref(),
                                    )
                                })
                        })
                        .collect();

                    for comment in matching_current {
                        let comment_style =
                            Style::default().fg(ctx.theme.blue).bg(ctx.theme.comment_bg);

                        let prefix_style = Style::default()
                            .fg(ctx.theme.blue)
                            .add_modifier(Modifier::BOLD);

                        let bubble = &ctx.icons.comment_bubble;
                        let right_prefix_first =
                            format!(" {bubble} @{}: ", comment.author.username);
                        let left_prefix_first = format!(" {bubble} Comment ");
                        let left_prefix_rest = " ".repeat(Span::raw(&left_prefix_first).width());

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
                            ctx,
                            &comment.body,
                            &CommentAnchor {
                                file_path: &file_path,
                                start_new,
                                end_new,
                                start_old,
                                end_old,
                            },
                            &updated_diff_view.all_lines,
                            &right_prefix_first,
                            prefix_style,
                        );

                        for (i, (right_prefix, prefix_style, content_spans)) in
                            formatted_lines.into_iter().enumerate()
                        {
                            let left_prefix = if i == 0 {
                                &left_prefix_first
                            } else {
                                &left_prefix_rest
                            };

                            left_list_lines.push(
                                Line::from(vec![
                                    Span::styled("         ", Style::default()),
                                    Span::styled(left_prefix.clone(), prefix_style),
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
                        .is_some_and(|(s, e)| idx >= s && idx <= e);

                    let old_str = line
                        .old_line_num
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| " ".to_string());
                    let new_str = line
                        .new_line_num
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| " ".to_string());

                    let num_width = updated_diff_view.line_number_width;
                    let gutter_bg = ctx.theme.diff_gutter_bg;

                    let marker_style = Style::default()
                        .fg(ctx.theme.yellow)
                        .add_modifier(Modifier::BOLD)
                        .bg(gutter_bg);

                    let num_style = Style::default().fg(ctx.theme.text_muted).bg(gutter_bg);

                    let sep_style = Style::default().fg(ctx.theme.diff_sep).bg(gutter_bg);

                    let mut line_spans = vec![
                        Span::styled(
                            if is_cursor {
                                cursor_marker.as_str()
                            } else if in_selection {
                                " ▐ "
                            } else {
                                "   "
                            },
                            marker_style,
                        ),
                        Span::styled(format!("{old_str:>num_width$} "), num_style),
                        Span::styled(format!("{new_str:>num_width$} "), num_style),
                        Span::styled("│ ", sep_style),
                    ];

                    let highlight = if in_selection {
                        RowHighlight::Selected
                    } else if updated_diff_view.search_matches.contains(&idx) {
                        RowHighlight::SearchMatch
                    } else {
                        RowHighlight::Plain
                    };
                    let sel_bg = highlight.row_bg(&ctx.theme);
                    let search_fg = highlight.row_fg(&ctx.theme);

                    match line.line_type {
                        crate::app::DiffLineType::Addition | crate::app::DiffLineType::Deletion => {
                            let theme = &ctx.theme;
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
                                    .fg(search_fg.unwrap_or(prefix_fg))
                                    .add_modifier(Modifier::BOLD)
                                    .bg(actual_bg),
                            ));

                            let content_base = Style::default()
                                .fg(search_fg.unwrap_or(code_fg))
                                .bg(actual_bg);
                            let final_style = if is_cursor {
                                content_base
                                    .add_modifier(Modifier::UNDERLINED)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                content_base
                            };

                            let mut content_spans =
                                render_diff_line_content(line, final_style, theme, highlight);
                            line_spans.append(&mut content_spans);
                        }
                        crate::app::DiffLineType::Normal => {
                            let actual_bg = sel_bg.unwrap_or(ctx.theme.bg);
                            let prefix = line
                                .content
                                .chars()
                                .next()
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| " ".to_string());
                            line_spans.push(Span::styled(
                                prefix,
                                Style::default().fg(ctx.theme.text_muted).bg(actual_bg),
                            ));

                            let content_base = Style::default()
                                .fg(search_fg.unwrap_or(ctx.theme.text_normal))
                                .bg(actual_bg);
                            let final_style = if is_cursor {
                                content_base
                                    .add_modifier(Modifier::UNDERLINED)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                content_base
                            };

                            let theme = &ctx.theme;
                            let mut content_spans =
                                render_diff_line_content(line, final_style, theme, highlight);
                            line_spans.append(&mut content_spans);
                        }
                        crate::app::DiffLineType::Meta => {
                            let mut s = Style::default()
                                .fg(search_fg.unwrap_or(ctx.theme.blue))
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
                            line_spans.push(Span::styled(&line.content, final_style));
                        }
                        crate::app::DiffLineType::HunkHeader => {
                            let mut s = Style::default()
                                .fg(search_fg.unwrap_or(ctx.theme.purple))
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
                            line_spans.push(Span::styled(&line.content, final_style));
                        }
                    }
                    list_lines.push(Line::from(line_spans));

                    // COMMENTS OVERLAY (unified)
                    let matching_comments: Vec<_> = draft_comments
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
                            .fg(ctx.theme.yellow)
                            .bg(ctx.theme.comment_draft_bg);

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
                            .fg(ctx.theme.yellow)
                            .add_modifier(Modifier::BOLD);

                        let right_prefix_first =
                            format!(" {} Draft Note{}: ", ctx.icons.comment_bubble, range_info);

                        let formatted_lines = format_comment_with_suggestions(
                            ctx,
                            &comment.body,
                            &CommentAnchor {
                                file_path: &comment.file_path,
                                start_new: comment.line_num.map(|n| n as u64),
                                end_new: comment.end_line_num.map(|n| n as u64),
                                start_old: comment.old_line_num.map(|n| n as u64),
                                end_old: comment.end_old_line_num.map(|n| n as u64),
                            },
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

                    let matching_current: Vec<_> = current_comments
                        .iter()
                        .filter(|c| {
                            !c.system
                                && c.position.as_ref().is_some_and(|pos| {
                                    crate::app::note_position_anchors(pos, Some(line), Some(line))
                                })
                        })
                        .collect();

                    for comment in matching_current {
                        let comment_style =
                            Style::default().fg(ctx.theme.blue).bg(ctx.theme.comment_bg);

                        let prefix_style = Style::default()
                            .fg(ctx.theme.blue)
                            .add_modifier(Modifier::BOLD);

                        let right_prefix_first = format!(
                            " {} @{}: ",
                            ctx.icons.comment_bubble, comment.author.username
                        );

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
                            ctx,
                            &comment.body,
                            &CommentAnchor {
                                file_path: &file_path,
                                start_new,
                                end_new,
                                start_old,
                                end_old,
                            },
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

            clear_area(f, ctx, area);
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
                    .map(|_| Line::from(Span::styled("│", Style::default().fg(ctx.theme.diff_sep))))
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
        }
    }

    render_overlays(f, ctx, app, size);

    if let Some(notice) = app.toast_notice() {
        let theme = &ctx.theme;
        let icons = ctx.icons;

        // First content line: "  <icon> <msg>  "
        let label = format!(
            "  {} {}  ",
            icons.status_failed,
            sanitize_untrusted(notice.text())
        );
        let hint = notice
            .detail()
            .map(|detail| format!("  {}  ", sanitize_untrusted(detail)));

        // Box width: content line width (plus hint width when shown), capped to terminal width
        let content_w = hint.as_ref().map_or(label.chars().count(), |hint| {
            label.chars().count().max(hint.chars().count())
        }) as u16;
        let inner_w = content_w.min(size.width.saturating_sub(4));
        let box_w = inner_w + 2;
        // 4 rows when hint is shown, 3 rows otherwise
        let box_h = if hint.is_some() { 4u16 } else { 3u16 };
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

        if let Some(hint) = &hint {
            // Split inner into two rows: error message + hint
            use ratatui::layout::Layout;
            let rows = Layout::default()
                .direction(ratatui::layout::Direction::Vertical)
                .constraints([
                    ratatui::layout::Constraint::Length(1),
                    ratatui::layout::Constraint::Length(1),
                ])
                .split(inner);

            // Row 0 — error message (truncated, bold red)
            let display = truncate(&label, rows[0].width as usize);
            let toast = Paragraph::new(display.as_str())
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.red).add_modifier(Modifier::BOLD))
                .wrap(Wrap { trim: false });
            f.render_widget(toast, rows[0]);

            // Row 1 — muted hint pointing to the terminal log
            let hint_display = truncate(hint, rows[1].width as usize);
            let hint_widget = Paragraph::new(hint_display.as_str())
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_muted));
            f.render_widget(hint_widget, rows[1]);
        } else {
            // No hint — single content row, truncated to inner width
            let display = truncate(&label, inner.width as usize);
            let toast = Paragraph::new(display.as_str())
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.red).add_modifier(Modifier::BOLD))
                .wrap(Wrap { trim: false });
            f.render_widget(toast, inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn error_toast_strips_escape_sequences_from_cli_stderr() {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.raise_error_toast(
            "push rejected: \u{1b}]8;;https://evil.example\u{7}feature\u{1b}]8;;\u{7}\u{1b}[5m!"
                .to_string(),
        );

        terminal.draw(|f| render(f, &mut app)).unwrap();

        let buffer = terminal.backend().buffer();
        let width = buffer.area.width as usize;
        let toast_row: String = buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .find(|row| row.contains("push rejected"))
            .expect("toast rendered");
        let icons = std::sync::Arc::clone(&app.icons);
        let ctx = &crate::ui::render_ctx::RenderCtx::new(&app, &icons);
        let icon = ctx.icons.status_failed.clone();
        let toast_start = toast_row.find(&icon).expect("toast starts with its icon");
        let toast_text = toast_row[toast_start..]
            .split('│')
            .next()
            .map(str::trim_end);
        assert_eq!(
            toast_text,
            Some(format!("{icon} push rejected: feature!").as_str())
        );
    }

    #[test]
    fn changed_words_keep_their_syntax_colors_on_the_emphasis_background() {
        let diff = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1 +1 @@
-let total = 1 + 2;
+let total = 3 * 4;
";
        let view = crate::app::DiffView::new(1, "o/r".to_string(), diff.to_string());
        let added = view
            .all_lines
            .iter()
            .find(|l| l.content == "+let total = 3 * 4;")
            .expect("added line parsed");
        let theme = Theme::default();
        let base = Style::default().bg(theme.diff_addition_bg);
        let spans = render_diff_line_content(added, base, &theme, RowHighlight::Plain);

        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, added.code());
        let emphasized: String = spans
            .iter()
            .filter(|s| s.style.bg == Some(theme.diff_addition_emph_bg))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(emphasized, "3 * 4");
        let operator = spans
            .iter()
            .find(|s| s.content == "*")
            .expect("operator is its own span");
        assert_eq!(operator.style.fg, Some(theme.purple));
    }

    #[test]
    fn every_bundled_preset_marks_the_changed_words() {
        let diff = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1 +1 @@
-let total = 1 + 2;
+let total = 3 * 4;
";
        let view = crate::app::DiffView::new(1, "o/r".to_string(), diff.to_string());
        let line = |content: &str| {
            view.all_lines
                .iter()
                .find(|l| l.content == content)
                .expect("diff line parsed")
        };
        let (removed, added) = (line("-let total = 1 + 2;"), line("+let total = 3 * 4;"));

        for name in crate::config::all_theme_presets() {
            let Some(theme) = Theme::preset(&name) else {
                continue;
            };
            for (row, changed_word, fg, bg) in [
                (removed, "1", theme.diff_deletion_fg, theme.diff_deletion_bg),
                (added, "3", theme.diff_addition_fg, theme.diff_addition_bg),
            ] {
                let base = Style::default().fg(fg).bg(bg);
                let spans = render_diff_line_content(row, base, &theme, RowHighlight::Plain);
                let marked = spans
                    .iter()
                    .find(|s| s.content == changed_word)
                    .expect("changed word is its own span")
                    .style;

                assert!(
                    marked.bg != Some(bg) || marked.add_modifier.contains(Modifier::REVERSED),
                    "preset {name}: {changed_word:?} in {:?} is not marked",
                    row.content
                );
            }
        }
    }

    #[test]
    fn a_syntax_color_matching_the_row_background_falls_back_to_the_row_foreground() {
        let diff = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -0,0 +1 @@
+fn total() {}
";
        let view = crate::app::DiffView::new(1, "o/r".to_string(), diff.to_string());
        let added = view
            .all_lines
            .iter()
            .find(|l| l.content == "+fn total() {}")
            .expect("added line parsed");
        let theme = Theme::default();
        let function_fg = |row_bg: Color| {
            let base = Style::default().fg(theme.text_normal).bg(row_bg);
            render_diff_line_content(added, base, &theme, RowHighlight::Plain)
                .into_iter()
                .find(|s| s.content == "total")
                .expect("function name is its own span")
                .style
                .fg
        };

        assert_eq!(function_fg(theme.bg), Some(theme.yellow));
        assert_eq!(function_fg(theme.yellow), Some(theme.text_normal));
    }

    #[test]
    fn a_search_matched_row_draws_every_token_in_the_search_foreground() {
        let diff = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -0,0 +1 @@
+fn total() { let n = 3 * 4; }
";
        let mut view = crate::app::DiffView::new(1, "o/r".to_string(), diff.to_string());
        let added = view
            .all_lines
            .iter_mut()
            .find(|l| l.content == "+fn total() { let n = 3 * 4; }")
            .expect("added line parsed");
        added.fuzzy_indices = Some(vec![4, 5, 6, 7, 8]);
        let theme = Theme::preset("terminal").expect("terminal preset parses");
        let base = Style::default()
            .fg(theme.diff_search_fg)
            .bg(theme.diff_search_bg);

        let spans = render_diff_line_content(added, base, &theme, RowHighlight::SearchMatch);

        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, added.code());
        assert!(
            spans
                .iter()
                .all(|s| s.style.fg == Some(theme.diff_search_fg)),
            "{spans:?}"
        );
        let matched: String = spans
            .iter()
            .filter(|s| {
                s.style
                    .add_modifier
                    .contains(Modifier::BOLD | Modifier::UNDERLINED)
            })
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(matched, "total");
    }

    fn rendered_rows(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        buffer
            .content()
            .chunks(buffer.area.width as usize)
            .map(|row| row.iter().map(|c| c.symbol()).collect())
            .collect()
    }

    fn labels_selector(request: crate::notice::RequestTag) -> crate::app::Selector {
        crate::app::Selector {
            title: "Select Labels".to_string(),
            all_items: vec![],
            selected_items: HashSet::new(),
            cursor_idx: 0,
            search_query: String::new(),
            is_filtering: false,
            is_loading: false,
            entity_iid: 7,
            entity_type: "issue".to_string(),
            field_type: "labels".to_string(),
            multi_select: true,
            state: ratatui::widgets::ListState::default(),
            request: Some(request),
        }
    }

    #[test]
    fn error_about_the_open_selector_request_renders_inside_the_selector() {
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let mut app = App::default();
        let request = app.next_request_tag();
        app.open_selector(labels_selector(request));
        app.push_notice(
            crate::notice::Notice::error("Failed to load options: HTTP 502").from_request(request),
        );

        terminal.draw(|f| render(f, &mut app)).unwrap();

        let selector_area = app
            .overlay_stack
            .iter()
            .find(|(kind, _)| *kind == crate::app::OverlayKind::Selector)
            .map(|(_, area)| *area)
            .expect("selector drawn");
        let rows = rendered_rows(&terminal);
        let notice_rows: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.contains("Failed to load options: HTTP 502"))
            .map(|(y, _)| y)
            .collect();
        assert_eq!(notice_rows.len(), 1, "drawn once, not also as a toast");
        let y = notice_rows[0] as u16;
        assert!(
            (selector_area.y..selector_area.y + selector_area.height).contains(&y),
            "row {y} is outside the selector {selector_area:?}"
        );
        assert_eq!(app.toast_notice(), None);
    }

    #[test]
    fn the_pipelines_tab_does_not_zoom_its_preview() {
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let mut app = App::default();
        app.jobs.items = vec![
            serde_json::from_value(serde_json::json!({
                "id": 70, "status": "running", "stage": "test", "name": "job-70"
            }))
            .expect("job fixture"),
        ];
        app.active_tab = Tab::Pipelines;
        app.detail_visible = true;
        app.details_zoomed = true;

        terminal.draw(|f| render(f, &mut app)).unwrap();

        assert!(app.content_rect.is_some_and(|rect| rect.width > 0));
    }

    #[test]
    fn error_about_a_closed_selector_request_renders_as_the_toast() {
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let mut app = App::default();
        let request = app.next_request_tag();
        app.open_selector(labels_selector(request));
        app.push_notice(
            crate::notice::Notice::error("Failed to load options: HTTP 502").from_request(request),
        );
        app.close_selector();

        terminal.draw(|f| render(f, &mut app)).unwrap();

        let icon = app.icons.status_failed.clone();
        let toast_row = rendered_rows(&terminal)
            .into_iter()
            .find(|row| row.contains("Failed to load options: HTTP 502"))
            .expect("toast rendered");
        assert!(toast_row.contains(&format!("{icon} Failed to load options: HTTP 502")));
    }

    fn workspace_panes_at(width: u16, height: u16) -> (Option<Rect>, Option<Rect>) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut app = App::default();
        app.detail_visible = true;
        terminal.draw(|f| render(f, &mut app)).unwrap();
        (app.content_rect, app.detail_rect)
    }

    #[test]
    fn preview_sits_beside_the_list_on_a_wide_terminal() {
        let (Some(list), Some(preview)) = workspace_panes_at(120, 30) else {
            panic!("list and preview drawn");
        };
        assert_eq!((preview.x, preview.y), (list.right(), list.y));
    }

    #[test]
    fn preview_narrows_beside_the_list_where_its_preferred_width_would_squeeze_the_list() {
        for width in 101..=104 {
            let (Some(list), Some(preview)) = workspace_panes_at(width, 30) else {
                panic!("list and preview drawn at {width} columns");
            };
            assert_eq!(
                (preview.x, preview.y, preview.width),
                (list.right(), list.y, compaction::MIN_PREVIEW_COLUMNS),
                "at {width} columns"
            );
        }
    }

    #[test]
    fn preview_moves_below_the_list_on_a_narrow_terminal() {
        for width in [60, 68, 79, 80, 89] {
            let (Some(list), Some(preview)) = workspace_panes_at(width, 30) else {
                panic!("list and preview drawn at {width} columns");
            };
            assert_eq!(
                (preview.x, preview.y, preview.width),
                (list.x, list.bottom(), list.width),
                "at {width} columns"
            );
        }
    }

    #[test]
    fn terminal_log_gives_way_before_the_stacked_preview_is_hidden() {
        for height in [18, 19] {
            let (Some(list), Some(preview)) = workspace_panes_at(80, height) else {
                panic!("list and preview drawn at {height} rows");
            };
            assert_eq!(
                (preview.x, preview.y, preview.bottom()),
                (list.x, list.bottom(), height),
                "at {height} rows"
            );
        }
    }

    #[test]
    fn preview_is_hidden_when_the_terminal_is_too_short_to_stack_it() {
        let (list, preview) = workspace_panes_at(80, 12);
        assert_eq!(list.map(|list| list.height), Some(10));
        assert_eq!(preview, None);
    }

    const ONE_LINE_DIFF: &str =
        "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-x\n+y\n";

    fn diff_is_side_by_side_at(width: u16, layout: crate::app::DiffLayout) -> bool {
        let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
        let mut app = App::default();
        app.diff_layout = layout;
        app.open_diff_view(crate::app::DiffView::new(
            1,
            "o/r".to_string(),
            ONE_LINE_DIFF.to_string(),
        ));
        terminal.draw(|f| render(f, &mut app)).unwrap();
        app.diff_view().is_some_and(|view| view.side_by_side)
    }

    #[test]
    fn auto_diff_layout_is_unified_when_narrow_and_side_by_side_when_wide() {
        use crate::app::DiffLayout;
        assert!(!diff_is_side_by_side_at(120, DiffLayout::Auto));
        assert!(diff_is_side_by_side_at(220, DiffLayout::Auto));
    }

    #[test]
    fn a_pinned_diff_layout_ignores_the_terminal_width() {
        use crate::app::DiffLayout;
        assert!(!diff_is_side_by_side_at(220, DiffLayout::Unified));
        assert!(diff_is_side_by_side_at(60, DiffLayout::SideBySide));
    }

    #[test]
    fn auto_diff_layout_waits_for_a_line_selection_to_end() {
        let mut terminal = Terminal::new(TestBackend::new(220, 30)).unwrap();
        let mut app = App::default();
        let mut view = crate::app::DiffView::new(1, "o/r".to_string(), ONE_LINE_DIFF.to_string());
        view.selection_start = Some(4);
        view.selection_end = Some(5);
        app.open_diff_view(view);

        terminal.draw(|f| render(f, &mut app)).unwrap();
        let view = app.diff_view_mut().expect("diff view open");
        assert_eq!(
            (view.side_by_side, view.selection_start, view.selection_end),
            (false, Some(4), Some(5))
        );

        view.selection_start = None;
        view.selection_end = None;
        terminal.draw(|f| render(f, &mut app)).unwrap();
        assert!(app.diff_view().is_some_and(|view| view.side_by_side));
    }

    #[test]
    fn diff_search_count_follows_an_auto_layout_switch_in_the_same_frame() {
        let mut terminal = Terminal::new(TestBackend::new(220, 30)).unwrap();
        let mut app = App::default();
        let mut view = crate::app::DiffView::new(
            1,
            "o/r".to_string(),
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-foo\n+foo bar\n"
                .to_string(),
        );
        view.search("foo");
        assert_eq!(view.search_matches.len(), 2, "unified counts both lines");
        app.open_diff_view(view);

        terminal.draw(|f| render(f, &mut app)).unwrap();
        let title = rendered_rows(&terminal)
            .into_iter()
            .find(|row| row.contains("FILTERED"))
            .expect("filtered title drawn");
        assert!(title.contains("foo (1/1)"), "{title}");
    }
}
