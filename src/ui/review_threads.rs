use super::helpers::rendered_line_count;
use super::modal::modal_area;
use crate::app::{App, OverlayKind, ReviewThreadsOverview};
use crate::config::{ICONS, THEME, Theme};
use crate::domain::review_threads::{ReviewThread, ThreadAnchor};
use crate::utils::format::{time_ago, truncate};
use crate::utils::markdown::render_markdown;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

const EXCERPT_CHARS: usize = 80;
const KEY_HINTS: &str = " j/k move · Enter jump to line · a actions · u unresolved only · J/K scroll thread · Esc close";

pub(crate) fn render_review_threads(f: &mut Frame, app: &mut App, size: Rect) {
    if app.diff_view.is_none() {
        return;
    }
    let Some(overview) = app.review_threads.as_mut() else {
        return;
    };
    let theme = *THEME.read().unwrap();

    let (body, area) = modal_area(f, &modal_title(overview), 85, 85, 70, 16, size);
    app.overlay_stack.push((OverlayKind::ReviewThreads, area));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(body);
    let (list_area, preview_area, hints_area) = (chunks[0], chunks[1], chunks[2]);

    overview.list_rect = Some(list_area);
    render_thread_list(f, overview, list_area, &theme);
    render_thread_preview(f, overview, preview_area, &theme);
    f.render_widget(
        Paragraph::new(KEY_HINTS).style(Style::default().fg(theme.text_muted).bg(theme.bg)),
        hints_area,
    );
}

fn modal_title(overview: &ReviewThreadsOverview) -> String {
    let filter = if overview.unresolved_only {
        " · unresolved only"
    } else {
        ""
    };
    format!(
        "Review Threads — {} unresolved / {} total{}",
        overview.unresolved_count(),
        overview.threads.len(),
        filter
    )
}

fn render_thread_list(
    f: &mut Frame,
    overview: &mut ReviewThreadsOverview,
    area: Rect,
    theme: &Theme,
) {
    let items: Vec<ListItem> = overview
        .visible_threads()
        .map(|thread| ListItem::new(thread_row(thread, theme)))
        .collect();

    if items.is_empty() {
        let message = if overview.threads.is_empty() {
            "  No comments on this merge request."
        } else {
            "  No unresolved threads. Press u to show all."
        };
        f.render_widget(
            Paragraph::new(message).style(
                Style::default()
                    .fg(theme.text_muted)
                    .bg(theme.bg)
                    .add_modifier(Modifier::ITALIC),
            ),
            area,
        );
        return;
    }

    let list = List::new(items)
        .style(Style::default().bg(theme.bg))
        .highlight_style(
            Style::default()
                .bg(theme.highlight_bg)
                .add_modifier(Modifier::BOLD),
        );
    f.render_stateful_widget(list, area, &mut overview.state);
}

fn thread_row(thread: &ReviewThread, theme: &Theme) -> Line<'static> {
    let root = thread.root();
    let mut spans = vec![status_badge(thread, theme), Span::raw(" ")];
    spans.extend(anchor_spans(&thread.anchor, theme));
    spans.push(Span::styled(
        format!("  @{}: ", root.author.username),
        Style::default().fg(theme.blue),
    ));
    spans.push(Span::styled(
        truncate(&root.body.replace('\n', " "), EXCERPT_CHARS),
        Style::default().fg(theme.text_normal),
    ));
    if thread.reply_count() > 0 {
        spans.push(Span::styled(
            format!(
                "  +{} {}",
                thread.reply_count(),
                pluralize_reply(thread.reply_count())
            ),
            Style::default().fg(theme.text_muted),
        ));
    }
    Line::from(spans)
}

fn pluralize_reply(count: usize) -> &'static str {
    if count == 1 { "reply" } else { "replies" }
}

fn status_badge(thread: &ReviewThread, theme: &Theme) -> Span<'static> {
    let (label, fg, bg) = if thread.is_unresolved() {
        (" UNRESOLVED ", theme.yellow, theme.yellow_bg)
    } else if thread.is_resolvable() {
        (" RESOLVED   ", theme.green, theme.green_bg)
    } else {
        (" COMMENT    ", theme.text_muted, theme.bg)
    };
    Span::styled(
        label,
        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
    )
}

fn anchor_spans(anchor: &ThreadAnchor, theme: &Theme) -> Vec<Span<'static>> {
    match anchor {
        ThreadAnchor::General => vec![Span::styled(
            " GENERAL ",
            Style::default()
                .fg(theme.purple)
                .bg(theme.purple_bg)
                .add_modifier(Modifier::BOLD),
        )],
        ThreadAnchor::InDiff { file_path, line } => vec![Span::styled(
            location(file_path, *line),
            Style::default().fg(theme.text_normal),
        )],
        ThreadAnchor::Outdated { file_path, line } => vec![
            Span::styled(
                " OUTDATED ",
                Style::default()
                    .fg(theme.red)
                    .bg(theme.red_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}", location(file_path, *line)),
                Style::default().fg(theme.text_muted),
            ),
        ],
    }
}

fn location(file_path: &str, line: Option<u64>) -> String {
    match line {
        Some(line) => format!("{file_path}:{line}"),
        None => file_path.to_string(),
    }
}

fn render_thread_preview(
    f: &mut Frame,
    overview: &mut ReviewThreadsOverview,
    area: Rect,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border))
        .title(" Thread ")
        .style(Style::default().bg(theme.bg));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(thread) = overview.selected_thread() else {
        return;
    };
    let lines = thread_preview_lines(thread, theme, inner.width);
    let rendered_rows = rendered_line_count(&lines, inner.width as usize, true);
    let max_scroll =
        u16::try_from(rendered_rows.saturating_sub(inner.height as usize)).unwrap_or(u16::MAX);
    overview.preview_scroll = overview.preview_scroll.min(max_scroll);

    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((overview.preview_scroll, 0))
            .style(Style::default().bg(theme.bg)),
        inner,
    );
}

fn thread_preview_lines(thread: &ReviewThread, theme: &Theme, width: u16) -> Vec<Line<'static>> {
    let comment_icon = ICONS.read().unwrap().comment.clone();
    let mut header = vec![status_badge(thread, theme), Span::raw(" ")];
    header.extend(anchor_spans(&thread.anchor, theme));
    let mut lines = vec![Line::from(header), Line::default()];

    for note in &thread.notes {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{comment_icon} @{}", note.author.username),
                Style::default().fg(theme.blue).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" · {}", time_ago(&note.created_at)),
                Style::default().fg(theme.text_muted),
            ),
        ]));
        lines.extend(render_markdown(&note.body, theme, width));
        lines.push(Line::default());
    }
    lines
}
