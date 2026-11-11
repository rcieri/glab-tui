//! Draws a [`Form`]: title, body, button row and key footer inside one frame.
//!
//! The title, the buttons and the footer are laid out first and always keep
//! their rows. The body gets what is left, and the compaction steps of
//! [`Compaction`] are applied in order until it fits. When even the most
//! compact body is taller than its rows, it scrolls to keep the focus visible.

use super::diff::centered_rect_fixed;
use super::modal::{button_style, clear_area};
use crate::config::{Icons, Theme};
use crate::form::{
    ButtonTone, ClickTarget, Focus, Form, FormLayout, FormTone, Group, GroupKind, GroupPath,
    ItemList, LineInput, NoteLevel, Preview, Progress, Radio, Toggles,
};
use crate::ui::render_ctx::RenderCtx;
use crate::utils::format::{sanitize_untrusted, wrap_ranges};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

const FORM_WIDTH: u16 = 64;
/// The border plus one padding column on each side, and the border alone
/// above and below.
const CONTENT_MARGIN: Margin = Margin::new(2, 1);
/// Border rows, the separator above the buttons, the buttons and the footer.
const CHROME_ROWS: u16 = 2 + 3;
/// Rows a bulk item list keeps once the form is short of room.
const SHORT_LIST_ROWS: usize = 3;
/// Columns between two radios drawn on one row.
const RADIO_GAP: usize = 2;
const SECTION_INDENT: usize = 2;

/// The steps that shrink a body too tall for the screen, in the order they
/// are taken. Each step keeps the ones before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Compaction {
    /// Labels on their own row and a blank row between groups.
    Roomy,
    /// No blank rows, and each label starts the first row of its group.
    NoSpacers,
    /// Previews fold to their first line and a `+N lines` hint.
    FoldedPreviews,
    /// Item lists keep their first rows and a `N more` row.
    ShortLists,
    /// Collapsible sections fold.
    FoldedSections,
}

impl Compaction {
    const ALL: [Self; 5] = [
        Self::Roomy,
        Self::NoSpacers,
        Self::FoldedPreviews,
        Self::ShortLists,
        Self::FoldedSections,
    ];
}

/// One body row, with the clickable spans on it as `(column offset, width,
/// target)`.
struct BodyRow {
    line: Line<'static>,
    targets: Vec<(u16, u16, ClickTarget)>,
    is_focused: bool,
}

impl BodyRow {
    fn plain(line: Line<'static>) -> Self {
        Self {
            line,
            targets: Vec::new(),
            is_focused: false,
        }
    }

    fn blank() -> Self {
        Self::plain(Line::default())
    }
}

struct Styles {
    text: Style,
    muted: Style,
    label: Style,
    focused: Style,
}

impl Styles {
    fn new(theme: &Theme) -> Self {
        let text = Style::default().fg(theme.text_normal).bg(theme.bg);
        let muted = Style::default().fg(theme.text_muted).bg(theme.bg);
        Self {
            text,
            muted,
            label: muted.add_modifier(Modifier::BOLD),
            focused: Style::default()
                .fg(theme.highlight_bg)
                .bg(theme.border_focused)
                .add_modifier(Modifier::BOLD),
        }
    }
}

/// Draws `form` centred in `size` over the dimmed screen, records where its
/// rows and buttons went for clicks and returns its frame.
pub(crate) fn render_form(
    f: &mut Frame,
    ctx: &RenderCtx<'_>,
    form: &mut Form,
    title: &str,
    size: Rect,
) -> Rect {
    let width = FORM_WIDTH.min(size.width);
    let text_width = usize::from(width.saturating_sub(2 * CONTENT_MARGIN.horizontal));
    let max_height = size
        .height
        .saturating_sub(2)
        .max(CHROME_ROWS)
        .min(size.height);
    let body_capacity = usize::from(max_height.saturating_sub(CHROME_ROWS));

    let rows = fitting_body(form, ctx, text_width, body_capacity);
    let body_height = rows.len().min(body_capacity) as u16;
    let area = centered_rect_fixed(width, CHROME_ROWS + body_height, size);

    dim_backdrop(f.buffer_mut(), size, area, ctx.theme.backdrop_dim);
    clear_area(f, ctx, area);
    let border = match form.tone() {
        FormTone::Normal => ctx.theme.border_focused,
        FormTone::Danger => ctx.theme.danger_border,
    };
    f.render_widget(
        Block::default()
            .title(format!(" {title} "))
            .title_style(
                Style::default()
                    .fg(ctx.theme.header_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border))
            .style(Style::default().bg(ctx.theme.bg)),
        area,
    );

    let [body_area, separator, buttons_area, footer_area] = Layout::vertical([
        Constraint::Length(body_height),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area.inner(CONTENT_MARGIN));

    let mut targets = Vec::new();
    form.scroll = scroll_to_focus(form.scroll, &rows, usize::from(body_height));
    for (offset, row) in rows
        .into_iter()
        .skip(form.scroll)
        .take(usize::from(body_height))
        .enumerate()
    {
        let row_area = Rect {
            y: body_area.y + offset as u16,
            height: 1,
            ..body_area
        };
        targets.extend(row.targets.iter().map(|&(column, width, target)| {
            let x = row_area.x + column.min(row_area.width);
            let width = width.min(row_area.right().saturating_sub(x));
            (
                Rect {
                    x,
                    width,
                    ..row_area
                },
                target,
            )
        }));
        f.render_widget(Paragraph::new(row.line), row_area);
    }

    f.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(ctx.theme.border)),
        separator,
    );
    render_buttons(f, ctx, form, buttons_area, &mut targets);
    f.render_widget(
        Paragraph::new(footer_line(
            form,
            &ctx.theme,
            usize::from(footer_area.width),
        )),
        footer_area,
    );

    form.layout = Some(FormLayout { targets });
    area
}

/// The body at the first compaction step that fits `capacity` rows, else
/// at the last one, which then scrolls.
fn fitting_body(form: &Form, ctx: &RenderCtx<'_>, width: usize, capacity: usize) -> Vec<BodyRow> {
    let mut rows = Vec::new();
    for compaction in Compaction::ALL {
        rows = body_rows(form, ctx, width, compaction);
        if rows.len() <= capacity {
            break;
        }
    }
    rows
}

/// The first row to draw so the focused row stays in a body of `height`
/// rows, moving `scroll` as little as possible.
fn scroll_to_focus(scroll: usize, rows: &[BodyRow], height: usize) -> usize {
    let max_scroll = rows.len().saturating_sub(height);
    let Some(focused) = rows.iter().position(|row| row.is_focused) else {
        return scroll.min(max_scroll);
    };
    let scroll = if focused < scroll {
        focused
    } else if focused >= scroll + height {
        focused + 1 - height
    } else {
        scroll
    };
    scroll.min(max_scroll)
}

/// Repaints the text outside `form_area` in `color`, so the screen behind
/// stays readable but recedes.
fn dim_backdrop(buffer: &mut Buffer, screen: Rect, form_area: Rect, color: Color) {
    for y in screen.top()..screen.bottom() {
        for x in screen.left()..screen.right() {
            let is_inside = form_area.contains((x, y).into());
            if let Some(cell) = buffer.cell_mut((x, y)).filter(|_| !is_inside) {
                cell.set_fg(color);
            }
        }
    }
}

fn body_rows(
    form: &Form,
    ctx: &RenderCtx<'_>,
    width: usize,
    compaction: Compaction,
) -> Vec<BodyRow> {
    let styles = Styles::new(&ctx.theme);
    let mut rows = Vec::new();
    for (index, group) in form.groups().iter().enumerate() {
        if compaction == Compaction::Roomy && !rows.is_empty() {
            rows.push(BodyRow::blank());
        }
        let path = GroupPath::top(index);
        let GroupKind::Collapsible(section) = &group.kind else {
            rows.extend(group_rows(
                form, path, group, ctx, &styles, width, compaction,
            ));
            continue;
        };
        let has_focus = matches!(form.focus(), Focus::Group(focused) if focused.index == index);
        let is_expanded =
            section.is_expanded && (compaction < Compaction::FoldedSections || has_focus);
        rows.push(section_header(
            form,
            path,
            section,
            is_expanded,
            ctx.icons,
            &styles,
        ));
        if !is_expanded {
            continue;
        }
        for (child, group) in section.groups.iter().enumerate() {
            let path = GroupPath {
                index,
                child: Some(child),
            };
            let child_rows = group_rows(
                form,
                path,
                group,
                ctx,
                &styles,
                width.saturating_sub(SECTION_INDENT),
                compaction,
            );
            rows.extend(child_rows.into_iter().map(indent));
        }
    }
    rows
}

fn indent(mut row: BodyRow) -> BodyRow {
    row.line
        .spans
        .insert(0, Span::raw(" ".repeat(SECTION_INDENT)));
    for target in &mut row.targets {
        target.0 += SECTION_INDENT as u16;
    }
    row
}

fn section_header(
    form: &Form,
    path: GroupPath,
    section: &crate::form::Collapsible,
    is_expanded: bool,
    icons: &Icons,
    styles: &Styles,
) -> BodyRow {
    let is_focused = form.focus() == Focus::Group(path);
    let icon = if is_expanded {
        &icons.folder_expanded
    } else {
        &icons.folder_collapsed
    };
    let header = format!("{icon} {}", sanitize_untrusted(&section.label));
    let width = Span::raw(header.as_str()).width() as u16;
    let style = if is_focused {
        styles.focused
    } else {
        styles.label
    };
    BodyRow {
        line: Line::from(vec![
            Span::styled(header, style),
            Span::styled(format!("  {}", section.key), styles.muted),
        ]),
        targets: vec![(0, width, ClickTarget::Row { path, row: 0 })],
        is_focused,
    }
}

/// The rows of one group, its label first: on a row of its own when there
/// is room, else at the start of its first row.
fn group_rows(
    form: &Form,
    path: GroupPath,
    group: &Group,
    ctx: &RenderCtx<'_>,
    styles: &Styles,
    width: usize,
    compaction: Compaction,
) -> Vec<BodyRow> {
    let label = group_label(group, ctx.icons);
    let is_inline = compaction > Compaction::Roomy && label.is_some();
    let prefix = match (&label, is_inline) {
        (Some(label), true) => format!("{label} "),
        _ => String::new(),
    };
    let prefix_width = Span::raw(prefix.as_str()).width();
    let content_width = width.saturating_sub(prefix_width);
    let focus_row = match form.focus() {
        Focus::Group(focused) if focused == path => Some(group.focus_row()),
        _ => None,
    };

    let mut rows = match &group.kind {
        GroupKind::Radio(radio) => {
            radio_rows(radio, path, focus_row, &ctx.theme, styles, content_width)
        }
        GroupKind::Toggles(toggles) => toggle_rows(toggles, path, focus_row, styles),
        GroupKind::LineInput(input) => vec![line_input_row(
            input,
            path,
            focus_row.is_some(),
            form.is_editing(),
            &ctx.theme,
            content_width,
        )],
        GroupKind::Preview(preview) => {
            preview_rows(preview, compaction >= Compaction::FoldedPreviews, styles)
        }
        GroupKind::ItemList(list) => {
            item_rows(list, compaction >= Compaction::ShortLists, ctx, styles)
        }
        GroupKind::Note(note) => {
            let (icon, style) = match note.level {
                NoteLevel::Info => (None, styles.text),
                NoteLevel::Warning => (
                    Some(&ctx.icons.flag_unresolved),
                    styles.text.fg(ctx.theme.yellow),
                ),
                NoteLevel::Error => (
                    Some(&ctx.icons.status_failed),
                    styles.text.fg(ctx.theme.red),
                ),
            };
            let text = match icon {
                Some(icon) => format!("{icon} {}", sanitize_untrusted(&note.text)),
                None => sanitize_untrusted(&note.text).into_owned(),
            };
            wrapped_rows(&text, content_width)
                .map(|row| BodyRow::plain(Line::from(Span::styled(row.to_string(), style))))
                .collect()
        }
        GroupKind::Collapsible(_) => Vec::new(),
    };
    if group.is_loading() && label.is_none() {
        rows.push(BodyRow::plain(Line::from(Span::styled(
            format!("{} Loading…", ctx.icons.label_loading),
            styles.muted,
        ))));
    }

    match label {
        Some(label) if !is_inline => {
            rows.insert(
                0,
                BodyRow::plain(Line::from(Span::styled(label, styles.label))),
            );
        }
        Some(_) => {
            for (position, row) in rows.iter_mut().enumerate() {
                let lead = if position == 0 {
                    Span::styled(prefix.clone(), styles.label)
                } else {
                    Span::raw(" ".repeat(prefix_width))
                };
                row.line.spans.insert(0, lead);
                for target in &mut row.targets {
                    target.0 += prefix_width as u16;
                }
            }
        }
        None => {}
    }
    rows
}

/// The label of `group` with what it is waiting for or how far it got.
fn group_label(group: &Group, icons: &Icons) -> Option<String> {
    let label = sanitize_untrusted(group.label.as_deref()?).into_owned();
    if group.is_loading() {
        return Some(format!("{label} {} loading…", icons.label_loading));
    }
    match &group.kind {
        GroupKind::ItemList(list) if list.has_started() => Some(format!(
            "{label} {}/{}",
            list.finished_count(),
            list.entries.len()
        )),
        _ => Some(label),
    }
}

/// `text` split into rows of at most `width` display columns, breaking a
/// word too long for a row. A break at a space would otherwise leave that
/// space as a row of its own, so whitespace-only rows are dropped while the
/// blank rows between paragraphs stay.
fn wrapped_rows(text: &str, width: usize) -> impl Iterator<Item = &str> {
    wrap_ranges(text, width).into_iter().filter_map(|range| {
        let row = text[range.clone()].trim();
        (range.is_empty() || !row.is_empty()).then_some(row)
    })
}

fn radio_rows(
    radio: &Radio,
    path: GroupPath,
    focus_row: Option<usize>,
    theme: &Theme,
    styles: &Styles,
    width: usize,
) -> Vec<BodyRow> {
    if let [only] = radio.choices.as_slice() {
        return vec![BodyRow::plain(Line::from(Span::styled(
            sanitize_untrusted(only).into_owned(),
            styles.text,
        )))];
    }
    let choices: Vec<(Span<'static>, bool)> = radio
        .choices
        .iter()
        .enumerate()
        .map(|(row, choice)| {
            let mark = if row == radio.selected { "(x)" } else { "( )" };
            let is_focused = focus_row == Some(row);
            let style = if is_focused {
                styles.focused
            } else if row == radio.selected {
                styles.text.fg(theme.accent)
            } else {
                styles.text
            };
            (
                Span::styled(format!("{mark} {}", sanitize_untrusted(choice)), style),
                is_focused,
            )
        })
        .collect();
    let one_row_width = choices.iter().map(|(span, _)| span.width()).sum::<usize>()
        + RADIO_GAP * (choices.len() - 1);

    if one_row_width > width {
        return choices
            .into_iter()
            .enumerate()
            .map(|(row, (span, is_focused))| BodyRow {
                targets: vec![(0, span.width() as u16, ClickTarget::Row { path, row })],
                line: Line::from(span),
                is_focused,
            })
            .collect();
    }
    let mut spans = Vec::new();
    let mut targets = Vec::new();
    let mut column = 0;
    let mut is_focused = false;
    for (row, (span, is_choice_focused)) in choices.into_iter().enumerate() {
        if row > 0 {
            spans.push(Span::raw(" ".repeat(RADIO_GAP)));
            column += RADIO_GAP;
        }
        let span_width = span.width();
        targets.push((
            column as u16,
            span_width as u16,
            ClickTarget::Row { path, row },
        ));
        column += span_width;
        is_focused |= is_choice_focused;
        spans.push(span);
    }
    vec![BodyRow {
        line: Line::from(spans),
        targets,
        is_focused,
    }]
}

fn toggle_rows(
    toggles: &Toggles,
    path: GroupPath,
    focus_row: Option<usize>,
    styles: &Styles,
) -> Vec<BodyRow> {
    toggles
        .items
        .iter()
        .enumerate()
        .map(|(row, toggle)| {
            let mark = if toggle.is_on { "[x]" } else { "[ ]" };
            let is_focused = focus_row == Some(row);
            let text = format!("{mark} {}", sanitize_untrusted(&toggle.label));
            let width = Span::raw(text.as_str()).width() as u16;
            BodyRow {
                line: Line::from(Span::styled(
                    text,
                    if is_focused {
                        styles.focused
                    } else {
                        styles.text
                    },
                )),
                targets: vec![(0, width, ClickTarget::Row { path, row })],
                is_focused,
            }
        })
        .collect()
}

/// The input across the row with its counter at the end. A text too long
/// for the row shows the part that ends at the cursor.
fn line_input_row(
    input: &LineInput,
    path: GroupPath,
    is_focused: bool,
    is_editing: bool,
    theme: &Theme,
    width: usize,
) -> BodyRow {
    let counter = input
        .soft_limit
        .map(|limit| format!(" {}/{limit}", input.char_count()))
        .unwrap_or_default();
    let counter_style = Style::default().bg(theme.bg).fg(if input.is_over_limit() {
        theme.yellow
    } else {
        theme.accent
    });
    let field_width = width
        .saturating_sub(Span::raw(counter.as_str()).width())
        .max(1);
    let field = Style::default()
        .fg(theme.text_normal)
        .bg(theme.highlight_bg);
    let field = if is_focused {
        field.add_modifier(Modifier::BOLD)
    } else {
        field
    };

    let value = sanitize_untrusted(input.buffer.value()).replace(['\t', '\r'], " ");
    let chars: Vec<char> = value.chars().collect();
    let cursor = input.buffer.value()[..input.buffer.cursor_byte_offset()]
        .chars()
        .count()
        .min(chars.len());
    let first = (cursor + 1).saturating_sub(field_width);
    let shown: String = chars.iter().skip(first).take(field_width).collect();

    let mut spans = if is_editing && is_focused {
        let (before, after) = shown.split_at(
            shown
                .char_indices()
                .nth(cursor - first)
                .map_or(shown.len(), |(offset, _)| offset),
        );
        let mut after = after.chars();
        let under_cursor = after.next().map_or_else(|| " ".to_string(), String::from);
        vec![
            Span::styled(before.to_string(), field),
            Span::styled(under_cursor, field.add_modifier(Modifier::REVERSED)),
            Span::styled(after.as_str().to_string(), field),
        ]
    } else {
        vec![Span::styled(shown, field)]
    };
    let used: usize = spans.iter().map(Span::width).sum();
    spans.push(Span::styled(
        " ".repeat(field_width.saturating_sub(used)),
        field,
    ));
    spans.push(Span::styled(counter, counter_style));
    BodyRow {
        line: Line::from(spans),
        targets: vec![(0, field_width as u16, ClickTarget::Row { path, row: 0 })],
        is_focused,
    }
}

fn preview_rows(preview: &Preview, is_short_of_room: bool, styles: &Styles) -> Vec<BodyRow> {
    let lines: Vec<String> = preview
        .lines
        .iter()
        .map(|line| sanitize_untrusted(line).into_owned())
        .collect();
    let is_folded = (preview.is_collapsed || is_short_of_room) && lines.len() > 1;
    if !is_folded {
        return lines
            .into_iter()
            .map(|line| BodyRow::plain(Line::from(Span::styled(line, styles.muted))))
            .collect();
    }
    let hidden = lines.len() - 1;
    let first = lines.into_iter().next().unwrap_or_default();
    vec![BodyRow::plain(Line::from(vec![
        Span::styled(first, styles.text),
        Span::styled(format!("  +{hidden} lines · e"), styles.muted),
    ]))]
}

fn item_rows(
    list: &ItemList,
    is_short_of_room: bool,
    ctx: &RenderCtx<'_>,
    styles: &Styles,
) -> Vec<BodyRow> {
    let shown = if is_short_of_room {
        SHORT_LIST_ROWS
    } else {
        list.entries.len()
    };
    let has_started = list.has_started();
    let mut rows: Vec<BodyRow> = list
        .entries
        .iter()
        .take(shown)
        .map(|entry| {
            let label = sanitize_untrusted(&entry.label).into_owned();
            if !has_started {
                return BodyRow::plain(Line::from(Span::styled(label, styles.text)));
            }
            let (icon, color) = match &entry.progress {
                Progress::Queued => (&ctx.icons.status_pending, ctx.theme.text_muted),
                Progress::Running => (&ctx.icons.status_running, ctx.theme.blue),
                Progress::Done => (&ctx.icons.status_success, ctx.theme.green),
                Progress::Failed(_) => (&ctx.icons.status_failed, ctx.theme.red),
            };
            let mut spans = vec![
                Span::styled(format!("{icon} "), styles.text.fg(color)),
                Span::styled(label, styles.text),
            ];
            if let Progress::Failed(reason) = &entry.progress {
                spans.push(Span::styled(
                    format!(": {}", sanitize_untrusted(reason)),
                    styles.muted,
                ));
            }
            BodyRow::plain(Line::from(spans))
        })
        .collect();
    let more = list.entries.len().saturating_sub(shown);
    if more > 0 {
        rows.push(BodyRow::plain(Line::from(Span::styled(
            format!("{} {more} more", ctx.icons.sort_desc),
            styles.muted,
        ))));
    }
    rows
}

fn render_buttons(
    f: &mut Frame,
    ctx: &RenderCtx<'_>,
    form: &Form,
    area: Rect,
    targets: &mut Vec<(Rect, ClickTarget)>,
) {
    let buttons = form.buttons();
    let count = buttons.len() as u32;
    let constraints: Vec<Constraint> = (0..buttons.len())
        .flat_map(|i| {
            let spacer = (i > 0).then_some(Constraint::Length(1));
            spacer
                .into_iter()
                .chain(std::iter::once(Constraint::Ratio(1, count)))
        })
        .collect();
    let cells = Layout::horizontal(constraints).split(area);
    for (index, button) in buttons.iter().enumerate() {
        let cell = cells[index * 2];
        let accent = match button.tone {
            ButtonTone::Neutral => None,
            ButtonTone::Positive => Some(ctx.theme.green),
            ButtonTone::Destructive => Some(ctx.theme.red),
        };
        f.render_widget(
            Paragraph::new(button_text(&button.label, usize::from(cell.width)))
                .alignment(Alignment::Center)
                .style(button_style(
                    &ctx.theme,
                    form.focused_button() == Some(index),
                    accent,
                )),
            cell,
        );
        targets.push((cell, ClickTarget::Button(index)));
    }
}

/// `[ label ]`, or tighter brackets, or the bare label, whichever fits in
/// `width` columns first.
fn button_text(label: &str, width: usize) -> String {
    let label_width = Span::raw(label).width();
    if label_width + 4 <= width {
        format!("[ {label} ]")
    } else if label_width + 2 <= width {
        format!("[{label}]")
    } else {
        label.to_string()
    }
}

/// The focused group's keys, as many as fit in `width` columns.
fn footer_line(form: &Form, theme: &Theme, width: usize) -> Line<'static> {
    let key = Style::default()
        .fg(theme.key_hint)
        .bg(theme.bg)
        .add_modifier(Modifier::BOLD);
    let action = Style::default().fg(theme.text_muted).bg(theme.bg);
    let mut spans = Vec::new();
    let mut used = 0;
    for hint in form.key_hints() {
        let gap = if spans.is_empty() { "" } else { "  " };
        let hint_width = gap.len() + hint.key.len() + 1 + Span::raw(hint.action.as_str()).width();
        if used + hint_width > width {
            break;
        }
        used += hint_width;
        spans.push(Span::styled(gap, action));
        spans.push(Span::styled(hint.key, key));
        spans.push(Span::styled(format!(" {}", hint.action), action));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::{Button, Collapsible, Group, ListEntry, Note, Toggle};
    use ratatui::{Terminal, backend::TestBackend};

    fn ctx(icons: &Icons) -> RenderCtx<'_> {
        RenderCtx {
            theme: Theme::default(),
            icons,
            kind: crate::backend::BackendKind::GitLab,
        }
    }

    fn rows_text(rows: &[BodyRow]) -> Vec<String> {
        rows.iter()
            .map(|row| {
                row.line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    fn crowded_form() -> Form {
        Form::new(
            "Merge 5 merge requests",
            vec![
                Group::new(GroupKind::ItemList(ItemList {
                    entries: (1..=5)
                        .map(|i| ListEntry::queued(format!("!{i}")))
                        .collect(),
                }))
                .with_label("Merge requests"),
                Group::new(GroupKind::Preview(Preview {
                    lines: vec!["Subject".into(), "Body one".into(), "Body two".into()],
                    is_collapsed: false,
                })),
                Group::new(GroupKind::Collapsible(Collapsible {
                    label: "Advanced".into(),
                    key: 'a',
                    is_expanded: true,
                    groups: vec![Group::new(GroupKind::Toggles(Toggles::new(vec![
                        Toggle::new("Bypass rules", false),
                    ])))],
                })),
            ],
            vec![
                Button::new("Merge", ButtonTone::Positive),
                Button::new("Cancel", ButtonTone::Neutral),
            ],
            0,
            1,
        )
    }

    #[test]
    fn each_compaction_step_shrinks_the_body_in_the_documented_order() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let form = crowded_form();
        let text = |compaction| rows_text(&body_rows(&form, &ctx, 60, compaction));

        assert_eq!(
            text(Compaction::Roomy),
            [
                "Merge requests",
                "!1",
                "!2",
                "!3",
                "!4",
                "!5",
                "",
                "Subject",
                "Body one",
                "Body two",
                "",
                "v Advanced  a",
                "  [ ] Bypass rules",
            ]
        );
        assert_eq!(
            text(Compaction::NoSpacers),
            [
                "Merge requests !1",
                "               !2",
                "               !3",
                "               !4",
                "               !5",
                "Subject",
                "Body one",
                "Body two",
                "v Advanced  a",
                "  [ ] Bypass rules",
            ]
        );
        assert_eq!(text(Compaction::FoldedPreviews)[5], "Subject  +2 lines · e");
        assert_eq!(
            text(Compaction::ShortLists),
            [
                "Merge requests !1",
                "               !2",
                "               !3",
                "               v 2 more",
                "Subject  +2 lines · e",
                "v Advanced  a",
                "  [ ] Bypass rules",
            ]
        );
        assert_eq!(
            text(Compaction::FoldedSections)[5..],
            ["> Advanced  a".to_string()]
        );
    }

    #[test]
    fn a_folded_section_stays_open_while_the_focus_is_in_it() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let mut form = crowded_form();
        let tab = crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Tab);
        form.handle_key(&tab);
        let rows = body_rows(&form, &ctx, 60, Compaction::FoldedSections);
        assert_eq!(rows_text(&rows)[5..], ["v Advanced  a", "  [ ] Bypass rules"]);
        form.handle_key(&tab);
        let rows = body_rows(&form, &ctx, 60, Compaction::FoldedSections);
        assert_eq!(
            rows.iter().position(|row| row.is_focused),
            Some(rows.len() - 1)
        );
    }

    #[test]
    fn the_first_step_that_fits_is_taken_and_the_last_one_scrolls() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let form = crowded_form();
        assert_eq!(fitting_body(&form, &ctx, 60, 10).len(), 10);
        assert_eq!(fitting_body(&form, &ctx, 60, 7).len(), 7);
        assert_eq!(fitting_body(&form, &ctx, 60, 3).len(), 6);
    }

    #[test]
    fn scrolling_moves_only_as_far_as_the_focused_row_needs() {
        let rows: Vec<BodyRow> = (0..10)
            .map(|i| BodyRow {
                is_focused: i == 7,
                ..BodyRow::blank()
            })
            .collect();
        assert_eq!(scroll_to_focus(0, &rows, 4), 4);
        assert_eq!(scroll_to_focus(5, &rows, 4), 5);
        assert_eq!(scroll_to_focus(9, &rows, 4), 6);
    }

    #[test]
    fn a_progress_list_marks_each_entry_and_counts_the_finished_ones() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let form = Form::new(
            "Merging",
            vec![
                Group::new(GroupKind::ItemList(ItemList {
                    entries: vec![
                        ListEntry {
                            label: "!1".into(),
                            progress: Progress::Done,
                        },
                        ListEntry {
                            label: "!2".into(),
                            progress: Progress::Failed("conflict".into()),
                        },
                        ListEntry {
                            label: "!3".into(),
                            progress: Progress::Running,
                        },
                    ],
                }))
                .with_label("Progress"),
                Group::new(GroupKind::Note(Note {
                    level: NoteLevel::Error,
                    text: "1 failed".into(),
                })),
            ],
            vec![Button::new("Close", ButtonTone::Neutral)],
            0,
            0,
        );
        assert_eq!(
            rows_text(&body_rows(&form, &ctx, 60, Compaction::Roomy)),
            [
                "Progress 2/3",
                "+ !1",
                "x !2: conflict",
                "> !3",
                "",
                "x 1 failed"
            ]
        );
    }

    #[test]
    fn a_note_breaks_a_word_too_long_for_its_row() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let branch = "b".repeat(70);
        let form = Form::new(
            "Delete Branch",
            vec![Group::note(
                NoteLevel::Info,
                format!("Delete branch {branch}?"),
            )],
            vec![Button::new("Cancel", ButtonTone::Neutral)],
            0,
            0,
        );
        assert_eq!(
            rows_text(&body_rows(&form, &ctx, 60, Compaction::Roomy)),
            [
                "Delete branch".to_string(),
                "b".repeat(60),
                format!("{}?", "b".repeat(10)),
            ]
        );
    }

    #[test]
    fn the_counter_sits_at_the_end_of_the_input_row() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let form = Form::new(
            "Merge",
            vec![
                Group::new(GroupKind::LineInput(LineInput::new("Fix".into(), Some(72))))
                    .with_label("Subject"),
            ],
            vec![Button::new("Merge", ButtonTone::Positive)],
            0,
            0,
        );
        assert_eq!(
            rows_text(&body_rows(&form, &ctx, 30, Compaction::NoSpacers)),
            ["Subject Fix               3/72"]
        );
    }

    #[test]
    fn the_title_buttons_and_footer_stay_on_screen_when_the_body_does_not_fit() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let mut form = crowded_form();
        let mut terminal = Terminal::new(TestBackend::new(60, 9)).unwrap();
        terminal
            .draw(|f| {
                render_form(f, &ctx, &mut form, "Merge", f.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = buffer
            .content
            .chunks(usize::from(buffer.area.width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect();
        assert!(rows[1].contains(" Merge "), "{rows:#?}");
        assert!(
            rows[5].contains("[ Merge ]") && rows[5].contains("[ Cancel ]"),
            "{rows:#?}"
        );
        assert!(rows[6].contains("Enter merge"), "{rows:#?}");
    }
}
