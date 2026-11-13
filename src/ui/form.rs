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
    ItemList, LineInput, ListEntry, NoteLevel, Preview, Progress, Radio, Toggles,
};
use crate::ui::render_ctx::RenderCtx;
use crate::utils::format::{displayable, sanitize_untrusted, wrap_ranges};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::ops::Range;

const FORM_WIDTH: u16 = 64;
const BORDER: u16 = 1;
const PADDING_COLUMNS: u16 = 1;
const CONTENT_MARGIN: Margin = Margin::new(BORDER + PADDING_COLUMNS, BORDER);
const BORDER_ROWS: u16 = 2 * CONTENT_MARGIN.vertical;
const SEPARATOR_ROWS: u16 = 1;
const BUTTON_ROWS: u16 = 1;
const FOOTER_ROWS: u16 = 1;
const CHROME_ROWS: u16 = BORDER_ROWS + SEPARATOR_ROWS + BUTTON_ROWS + FOOTER_ROWS;
const SCREEN_MARGIN_ROWS: u16 = 2;
const MIN_SHORT_LIST_ENTRIES: usize = 3;
const RADIO_GAP_COLUMNS: usize = 2;
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
    /// Item lists keep the entries that still fit, at least
    /// `MIN_SHORT_LIST_ENTRIES`, and a `N more` row.
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

/// How far a body is shrunk: the compaction step, and the entries an item
/// list keeps when that step cuts lists.
#[derive(Clone, Copy)]
struct Fit {
    compaction: Compaction,
    list_entries: Option<usize>,
}

impl Fit {
    fn at(compaction: Compaction) -> Self {
        Self {
            compaction,
            list_entries: (compaction >= Compaction::ShortLists).then_some(MIN_SHORT_LIST_ENTRIES),
        }
    }
}

#[derive(Clone, Copy)]
struct RowTarget {
    column: u16,
    width: u16,
    target: ClickTarget,
}

struct BodyRow {
    line: Line<'static>,
    targets: Vec<RowTarget>,
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
        .saturating_sub(SCREEN_MARGIN_ROWS)
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
            .title(format!(" {} ", sanitize_untrusted(title)))
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
        Constraint::Length(SEPARATOR_ROWS),
        Constraint::Length(BUTTON_ROWS),
        Constraint::Length(FOOTER_ROWS),
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
        targets.extend(row.targets.iter().map(
            |&RowTarget {
                 column,
                 width,
                 target,
             }| {
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
            },
        ));
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
        let fit = Fit::at(compaction);
        rows = body_rows(form, ctx, width, fit);
        if rows.len() > capacity {
            continue;
        }
        return with_longest_lists(form, ctx, width, capacity, fit, rows);
    }
    rows
}

/// Grows the item lists of `rows`, the body at `fit` that fits `capacity`,
/// by as many entries as the rows left over take, when `fit` cuts lists.
/// The body only grows with the entries kept, so a binary search finds the
/// most that still fit.
fn with_longest_lists(
    form: &Form,
    ctx: &RenderCtx<'_>,
    width: usize,
    capacity: usize,
    fit: Fit,
    mut rows: Vec<BodyRow>,
) -> Vec<BodyRow> {
    let Some(fewest) = fit.list_entries else {
        return rows;
    };
    let (mut fitting, mut too_many) = (fewest, longest_list(form.groups()) + 1);
    while fitting + 1 < too_many {
        let kept = fitting + (too_many - fitting) / 2;
        let candidate = body_rows(
            form,
            ctx,
            width,
            Fit {
                list_entries: Some(kept),
                ..fit
            },
        );
        if candidate.len() <= capacity {
            (fitting, rows) = (kept, candidate);
        } else {
            too_many = kept;
        }
    }
    rows
}

fn longest_list(groups: &[Group]) -> usize {
    groups
        .iter()
        .map(|group| match &group.kind {
            GroupKind::ItemList(list) => list.entries.len(),
            GroupKind::Collapsible(section) => longest_list(&section.groups),
            _ => 0,
        })
        .max()
        .unwrap_or(0)
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

fn body_rows(form: &Form, ctx: &RenderCtx<'_>, width: usize, fit: Fit) -> Vec<BodyRow> {
    let compaction = fit.compaction;
    let styles = Styles::new(&ctx.theme);
    let mut rows = Vec::new();
    for (index, group) in form.groups().iter().enumerate() {
        if compaction == Compaction::Roomy && !rows.is_empty() {
            rows.push(BodyRow::blank());
        }
        let path = GroupPath::top(index);
        let GroupKind::Collapsible(section) = &group.kind else {
            rows.extend(group_rows(form, path, group, ctx, &styles, width, fit));
            continue;
        };
        let has_focus = matches!(
            form.focus(),
            Focus::Group(focused) | Focus::Editing(focused) if focused.index == index
        );
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
                fit,
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
        target.column += SECTION_INDENT as u16;
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
        targets: vec![RowTarget {
            column: 0,
            width,
            target: ClickTarget::Row { path, row: 0 },
        }],
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
    fit: Fit,
) -> Vec<BodyRow> {
    let label = group_label(group, ctx.icons);
    let compaction = fit.compaction;
    let is_inline = compaction > Compaction::Roomy && label.is_some();
    let prefix = match (&label, is_inline) {
        (Some(label), true) => format!("{label} "),
        _ => String::new(),
    };
    let prefix_width = Span::raw(prefix.as_str()).width();
    let content_width = width.saturating_sub(prefix_width);
    let focus_row = match form.focus() {
        Focus::Group(focused) | Focus::Editing(focused) if focused == path => {
            Some(group.focus_row())
        }
        _ => None,
    };

    let mut rows = match &group.kind {
        GroupKind::Radio(radio) => radio_rows(radio, path, focus_row, ctx, styles, content_width),
        GroupKind::Toggles(toggles) => toggle_rows(toggles, path, focus_row, ctx.icons, styles),
        GroupKind::LineInput(input) => vec![line_input_row(
            input,
            path,
            form.focus(),
            &ctx.theme,
            content_width,
        )],
        GroupKind::Preview(preview) => {
            preview_rows(preview, compaction >= Compaction::FoldedPreviews, styles)
        }
        GroupKind::ItemList(list) => {
            let kept = fit.list_entries.unwrap_or(list.entries.len());
            item_rows(list, kept, ctx, styles, content_width)
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
                    target.column += prefix_width as u16;
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
/// word too long for a row.
fn wrapped_rows(text: &str, width: usize) -> impl Iterator<Item = &str> {
    wrapped_ranges(text, width).map(|range| &text[range])
}

/// The byte ranges of `wrapped_rows`, trimmed. A break at a space would
/// otherwise leave that space as a row of its own, so whitespace-only rows
/// are dropped while the blank rows between paragraphs stay.
fn wrapped_ranges(text: &str, width: usize) -> impl Iterator<Item = Range<usize>> + '_ {
    wrap_ranges(text, width).into_iter().filter_map(|range| {
        let row = &text[range.clone()];
        let start = range.start + (row.len() - row.trim_start().len());
        let end = range.start + row.trim_end().len();
        (range.is_empty() || start < end).then_some(start..end.max(start))
    })
}

fn radio_rows(
    radio: &Radio,
    path: GroupPath,
    focus_row: Option<usize>,
    ctx: &RenderCtx<'_>,
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
            let mark = if row == radio.selected {
                &ctx.icons.radio_on
            } else {
                &ctx.icons.radio_off
            };
            let is_focused = focus_row == Some(row);
            let style = if is_focused {
                styles.focused
            } else if row == radio.selected {
                styles.text.fg(ctx.theme.accent)
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
        + RADIO_GAP_COLUMNS * choices.len().saturating_sub(1);

    if one_row_width > width {
        return choices
            .into_iter()
            .enumerate()
            .map(|(row, (span, is_focused))| BodyRow {
                targets: vec![RowTarget {
                    column: 0,
                    width: span.width() as u16,
                    target: ClickTarget::Row { path, row },
                }],
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
            spans.push(Span::raw(" ".repeat(RADIO_GAP_COLUMNS)));
            column += RADIO_GAP_COLUMNS;
        }
        let span_width = span.width();
        targets.push(RowTarget {
            column: column as u16,
            width: span_width as u16,
            target: ClickTarget::Row { path, row },
        });
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
    icons: &Icons,
    styles: &Styles,
) -> Vec<BodyRow> {
    toggles
        .items
        .iter()
        .enumerate()
        .map(|(row, toggle)| {
            let mark = if toggle.is_on {
                &icons.check_on
            } else {
                &icons.check_off
            };
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
                targets: vec![RowTarget {
                    column: 0,
                    width,
                    target: ClickTarget::Row { path, row },
                }],
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
    focus: Focus,
    theme: &Theme,
    width: usize,
) -> BodyRow {
    let is_editing = focus == Focus::Editing(path);
    let is_focused = is_editing || focus == Focus::Group(path);
    let counter = input
        .soft_char_limit
        .map(|limit| format!(" {}/{limit}", input.char_count()))
        .unwrap_or_default();
    let counter_style = Style::default()
        .bg(theme.bg)
        .fg(if input.is_over_char_limit() {
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

    let value = input.buffer.value();
    let shown = displayable(value);
    let cursor = displayable(&value[..input.buffer.cursor_byte_offset()]).len();
    let (head, tail) = shown.split_at_checked(cursor).unwrap_or((&shown, ""));
    let (under_cursor, rest) = tail.split_at(tail.chars().next().map_or(0, char::len_utf8));
    let cursor_columns = Span::raw(under_cursor).width().max(1);
    let head = suffix_within(head, field_width.saturating_sub(cursor_columns));
    let rest = prefix_within(
        rest,
        field_width.saturating_sub(Span::raw(head).width() + cursor_columns),
    );

    let mut spans = if is_editing {
        let under_cursor = if under_cursor.is_empty() {
            " "
        } else {
            under_cursor
        };
        vec![
            Span::styled(head.to_string(), field),
            Span::styled(
                under_cursor.to_string(),
                field.add_modifier(Modifier::REVERSED),
            ),
            Span::styled(rest.to_string(), field),
        ]
    } else {
        vec![Span::styled(format!("{head}{under_cursor}{rest}"), field)]
    };
    let used: usize = spans.iter().map(Span::width).sum();
    spans.push(Span::styled(
        " ".repeat(field_width.saturating_sub(used)),
        field,
    ));
    spans.push(Span::styled(counter, counter_style));
    BodyRow {
        line: Line::from(spans),
        targets: vec![RowTarget {
            column: 0,
            width: field_width as u16,
            target: ClickTarget::Row { path, row: 0 },
        }],
        is_focused,
    }
}

/// The longest end of `text` that fits in `columns` display columns.
fn suffix_within(text: &str, columns: usize) -> &str {
    let mut used = 0;
    let mut start = text.len();
    for (offset, c) in text.char_indices().rev() {
        used += Span::raw(&text[offset..offset + c.len_utf8()]).width();
        if used > columns {
            break;
        }
        start = offset;
    }
    &text[start..]
}

/// The longest start of `text` that fits in `columns` display columns.
fn prefix_within(text: &str, columns: usize) -> &str {
    let mut used = 0;
    let mut end = 0;
    for (offset, c) in text.char_indices() {
        used += Span::raw(&text[offset..offset + c.len_utf8()]).width();
        if used > columns {
            break;
        }
        end = offset + c.len_utf8();
    }
    &text[..end]
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
    let noun = if hidden == 1 { "line" } else { "lines" };
    let first = lines.into_iter().next().unwrap_or_default();
    vec![BodyRow::plain(Line::from(vec![
        Span::styled(first, styles.text),
        Span::styled(format!("  +{hidden} {noun}"), styles.muted),
    ]))]
}

fn item_rows(
    list: &ItemList,
    kept_entries: usize,
    ctx: &RenderCtx<'_>,
    styles: &Styles,
    width: usize,
) -> Vec<BodyRow> {
    let shown = if list.entries.len() > kept_entries.saturating_add(1) {
        kept_entries
    } else {
        list.entries.len()
    };
    let has_started = list.has_started();
    let mut rows: Vec<BodyRow> = list
        .entries
        .iter()
        .take(shown)
        .flat_map(|entry| entry_rows(entry, has_started, ctx, styles, width))
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

/// One list entry wrapped to `width` columns. Once the list has started, the
/// first row carries the progress marker, the rows below are indented under
/// it, and a failure reason follows the label muted.
fn entry_rows(
    entry: &ListEntry,
    has_started: bool,
    ctx: &RenderCtx<'_>,
    styles: &Styles,
    width: usize,
) -> Vec<BodyRow> {
    let label = sanitize_untrusted(&entry.label);
    if !has_started {
        return wrapped_rows(&label, width)
            .map(|row| BodyRow::plain(Line::from(Span::styled(row.to_string(), styles.text))))
            .collect();
    }
    let (icon, color) = match &entry.progress {
        Progress::Queued => (&ctx.icons.status_pending, ctx.theme.text_muted),
        Progress::Running => (&ctx.icons.status_running, ctx.theme.blue),
        Progress::Done => (&ctx.icons.status_success, ctx.theme.green),
        Progress::Failed(_) => (&ctx.icons.status_failed, ctx.theme.red),
    };
    let marker = format!("{icon} ");
    let marker_width = Span::raw(marker.as_str()).width();
    let reason = match &entry.progress {
        Progress::Failed(reason) => format!(": {}", sanitize_untrusted(reason)),
        _ => String::new(),
    };
    let text = format!("{label}{reason}");
    let mut rows: Vec<BodyRow> = wrapped_ranges(&text, width.saturating_sub(marker_width))
        .enumerate()
        .map(|(position, range)| {
            let lead = if position == 0 {
                Span::styled(marker.clone(), styles.text.fg(color))
            } else {
                Span::raw(" ".repeat(marker_width))
            };
            let split = label.len().clamp(range.start, range.end);
            BodyRow::plain(Line::from(vec![
                lead,
                Span::styled(text[range.start..split].to_string(), styles.text),
                Span::styled(text[split..range.end].to_string(), styles.muted),
            ]))
        })
        .collect();
    if rows.is_empty() {
        rows.push(BodyRow::plain(Line::from(Span::styled(
            marker,
            styles.text.fg(color),
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
    use crate::form::{Button, ButtonRow, Collapsible, Group, ListEntry, Note, Toggle};
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
            ButtonRow::new(
                Button::new("Merge", ButtonTone::Positive),
                [],
                Button::new("Cancel", ButtonTone::Neutral),
            ),
        )
    }

    #[test]
    fn each_compaction_step_shrinks_the_body_in_the_documented_order() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let form = crowded_form();
        let text = |compaction| rows_text(&body_rows(&form, &ctx, 60, Fit::at(compaction)));

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
                "  - Bypass rules",
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
                "  - Bypass rules",
            ]
        );
        assert_eq!(text(Compaction::FoldedPreviews)[5], "Subject  +2 lines");
        assert_eq!(
            text(Compaction::ShortLists),
            [
                "Merge requests !1",
                "               !2",
                "               !3",
                "               v 2 more",
                "Subject  +2 lines",
                "v Advanced  a",
                "  - Bypass rules",
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
        let rows = body_rows(&form, &ctx, 60, Fit::at(Compaction::FoldedSections));
        assert_eq!(rows_text(&rows)[5..], ["v Advanced  a", "  - Bypass rules"]);
        form.handle_key(&tab);
        let rows = body_rows(&form, &ctx, 60, Fit::at(Compaction::FoldedSections));
        assert_eq!(
            rows.iter().position(|row| row.is_focused),
            Some(rows.len() - 1)
        );
    }

    #[test]
    fn the_section_key_opens_a_section_the_short_form_folded() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let mut form = crowded_form();
        form.handle_key(&crossterm::event::KeyEvent::from(
            crossterm::event::KeyCode::Char('a'),
        ));
        let rows = body_rows(&form, &ctx, 60, Fit::at(Compaction::FoldedSections));
        assert_eq!(rows_text(&rows)[5..], ["v Advanced  a", "  - Bypass rules"]);
    }

    #[test]
    fn a_click_on_the_header_opens_a_section_the_short_form_folded() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let mut form = crowded_form();
        form.click(ClickTarget::Row {
            path: GroupPath::top(2),
            row: 0,
        });
        let rows = body_rows(&form, &ctx, 60, Fit::at(Compaction::FoldedSections));
        assert_eq!(rows_text(&rows)[5..], ["v Advanced  a", "  - Bypass rules"]);
    }

    #[test]
    fn a_folded_preview_counts_its_hidden_lines_in_the_singular_too() {
        let styles = Styles::new(&Theme::default());
        let preview = Preview {
            lines: vec!["Subject".into(), "Body".into()],
            is_collapsed: true,
        };
        assert_eq!(
            rows_text(&preview_rows(&preview, false, &styles)),
            ["Subject  +1 line"]
        );
    }

    #[test]
    fn a_short_list_is_cut_only_when_that_saves_a_row() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let styles = Styles::new(&ctx.theme);
        let list = |count: usize| ItemList {
            entries: (1..=count)
                .map(|i| ListEntry::queued(format!("!{i}")))
                .collect(),
        };
        assert_eq!(
            rows_text(&item_rows(
                &list(4),
                MIN_SHORT_LIST_ENTRIES,
                &ctx,
                &styles,
                60
            )),
            ["!1", "!2", "!3", "!4"]
        );
        assert_eq!(
            rows_text(&item_rows(
                &list(5),
                MIN_SHORT_LIST_ENTRIES,
                &ctx,
                &styles,
                60
            )),
            ["!1", "!2", "!3", "v 2 more"]
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
    fn a_short_list_keeps_as_many_entries_as_the_rows_left_over_hold() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let form = Form::new(
            "Merge 10 merge requests",
            vec![
                Group::note(NoteLevel::Info, "Merging 10 MRs"),
                Group::new(GroupKind::ItemList(ItemList {
                    entries: (1..=10)
                        .map(|i| ListEntry::queued(format!("!{i}")))
                        .collect(),
                })),
            ],
            ButtonRow::single(Button::new("Cancel", ButtonTone::Neutral)),
        );
        assert_eq!(
            rows_text(&fitting_body(&form, &ctx, 60, 8)),
            [
                "Merging 10 MRs",
                "!1",
                "!2",
                "!3",
                "!4",
                "!5",
                "!6",
                "v 4 more"
            ]
        );
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
            ButtonRow::single(Button::new("Close", ButtonTone::Neutral)),
        );
        assert_eq!(
            rows_text(&body_rows(&form, &ctx, 60, Fit::at(Compaction::Roomy))),
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
    fn list_entries_wrap_with_a_failure_reason_indented_under_the_marker() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let styles = Styles::new(&ctx.theme);
        let queued = ItemList {
            entries: vec![ListEntry::queued("#12: aaaa bbbb cccc dddd eeee [draft]")],
        };
        assert_eq!(
            rows_text(&item_rows(&queued, 1, &ctx, &styles, 15)),
            ["#12: aaaa bbbb", "cccc dddd eeee", "[draft]"]
        );
        let failed = ItemList {
            entries: vec![ListEntry {
                label: "!2: aa".into(),
                progress: Progress::Failed("merge conflict in src".into()),
            }],
        };
        assert_eq!(
            rows_text(&item_rows(&failed, 1, &ctx, &styles, 15)),
            ["x !2: aa: merge", "  conflict in", "  src"]
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
            ButtonRow::single(Button::new("Cancel", ButtonTone::Neutral)),
        );
        assert_eq!(
            rows_text(&body_rows(&form, &ctx, 60, Fit::at(Compaction::Roomy))),
            [
                "Delete branch".to_string(),
                "b".repeat(60),
                format!("{}?", "b".repeat(10)),
            ]
        );
    }

    #[test]
    fn a_radio_with_no_choices_draws_an_empty_row() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let rows = radio_rows(
            &Radio::new(Vec::new(), 0),
            GroupPath::top(0),
            None,
            &ctx,
            &Styles::new(&ctx.theme),
            60,
        );
        assert_eq!(rows_text(&rows), [""]);
    }

    #[test]
    fn a_wide_text_scrolls_by_display_columns_so_the_counter_stays_in_the_row() {
        let theme = Theme::default();
        let input = LineInput::new("漢".repeat(30), Some(72));
        let row = line_input_row(&input, GroupPath::top(0), Focus::Button(0), &theme, 22);
        assert_eq!(
            rows_text(std::slice::from_ref(&row)),
            [format!("{}   30/72", "漢".repeat(7))]
        );
        assert_eq!(row.line.width(), 22);
    }

    #[test]
    fn the_cursor_is_drawn_where_the_next_key_lands_past_an_escape_sequence() {
        let theme = Theme::default();
        let mut input = LineInput::new("ab\u{1b}[31mcd".into(), None);
        input.buffer.apply_edit_key(crossterm::event::KeyCode::Left);
        input.buffer.apply_edit_key(crossterm::event::KeyCode::Left);
        let editing = Focus::Editing(GroupPath::top(0));
        let row = line_input_row(&input, GroupPath::top(0), editing, &theme, 20);
        let spans: Vec<&str> = row.line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(spans[..3], ["ab", "c", "d"]);
    }

    #[test]
    fn the_cursor_inside_an_escape_sequence_does_not_reveal_its_tail() {
        let theme = Theme::default();
        let mut input = LineInput::new("ab\u{1b}[31mcd".into(), None);
        for _ in 0..5 {
            input.buffer.apply_edit_key(crossterm::event::KeyCode::Left);
        }
        let editing = Focus::Editing(GroupPath::top(0));
        let row = line_input_row(&input, GroupPath::top(0), editing, &theme, 20);
        let spans: Vec<&str> = row.line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(spans[..3], ["ab", "c", "d"]);
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
            ButtonRow::single(Button::new("Merge", ButtonTone::Positive)),
        );
        assert_eq!(
            rows_text(&body_rows(&form, &ctx, 30, Fit::at(Compaction::NoSpacers))),
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

    #[test]
    fn control_sequences_in_the_title_are_stripped() {
        let icons = Icons::ascii();
        let ctx = ctx(&icons);
        let mut form = crowded_form();
        let mut terminal = Terminal::new(TestBackend::new(60, 9)).unwrap();
        terminal
            .draw(|f| {
                render_form(f, &ctx, &mut form, "Delete Branch 'a\u{9b}31mb'", f.area());
            })
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains(" Delete Branch 'ab' "), "{screen}");
    }
}
