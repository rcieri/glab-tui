//! Draws a [`Form`]: one frame for every overlay, its groups stacked in the
//! body, then the overlay's notice, the button row and a one-line key footer
//! pinned to the bottom. The bottom rows are laid out before the body, so a
//! short terminal shrinks the body and never hides the buttons or the keys.

use super::modal::{button_style, clear_area, modal_block};
use crate::form::{ButtonRole, Focus, Form, FormLayout, Group, Rows};
use crate::notice::{Notice, NoticeLevel};
use crate::ui::render_ctx::RenderCtx;
use crate::utils::format::{sanitize_untrusted, wrap_text};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// One axis of a form's outer size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Extent {
    Fixed(u16),
    /// This share of the screen, never below `min`.
    Percent {
        percent: u16,
        min: u16,
    },
    /// Height only: what the groups need.
    Content,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FormSize {
    pub width: Extent,
    pub height: Extent,
}

pub(crate) struct FormFrame<'a> {
    pub title: &'a str,
    pub size: FormSize,
    /// The key footer; `None` takes the focused control's from the form.
    pub hints: Option<&'a str>,
    /// A notice about the overlay's own request, drawn above the buttons.
    pub notice: Option<&'a Notice>,
}

/// Columns between the border and the content on each side.
const PADDING_COLUMNS: u16 = 1;
/// Width of a form whose frame asks for no particular width.
const DEFAULT_WIDTH: u16 = 64;

pub(crate) fn render_form<F>(
    f: &mut Frame,
    ctx: &RenderCtx<'_>,
    screen: Rect,
    frame: &FormFrame<'_>,
    form: &mut Form<F>,
) {
    let width = outer_extent(frame.size.width, screen.width, DEFAULT_WIDTH);
    let text_width = usize::from(width.saturating_sub(2 + 2 * PADDING_COLUMNS));
    let natural: Vec<u16> = form
        .groups
        .iter()
        .map(|group| natural_rows(group, text_width))
        .collect();
    let top_padding = u16::from(!matches!(
        form.groups.first(),
        Some(Group::Custom(_)) | None
    ));
    let notice_rows = u16::from(frame.notice.is_some());
    let button_rows = if form.buttons.is_empty() { 0 } else { 2 };
    let footer_rows = 1;
    let gaps = gap_rows(&natural);
    let content_height = 2
        + top_padding
        + natural.iter().sum::<u16>()
        + gaps
        + notice_rows
        + button_rows
        + footer_rows;
    let height = outer_extent(frame.size.height, screen.height, content_height);
    let area = centered(width, height, screen);

    clear_area(f, ctx, area);
    let block = modal_block(frame.title, &ctx.theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let content = Rect {
        x: inner.x.saturating_add(PADDING_COLUMNS),
        width: inner.width.saturating_sub(2 * PADDING_COLUMNS),
        ..inner
    };

    let [_, body, notice_area, separator, buttons_area, footer] = Layout::vertical([
        Constraint::Length(top_padding),
        Constraint::Min(0),
        Constraint::Length(notice_rows),
        Constraint::Length(button_rows.min(1)),
        Constraint::Length(button_rows.saturating_sub(1)),
        Constraint::Length(footer_rows),
    ])
    .areas(content);

    let group_rects = allocate(&form.groups, &natural, body);
    let mut layout = FormLayout {
        area,
        groups: group_rects.clone(),
        items: Vec::new(),
        buttons: Vec::new(),
    };
    let focus = form.focus();
    for (index, (group, rect)) in form.groups.iter().zip(&group_rects).enumerate() {
        draw_group(f, ctx, group, index, *rect, focus, &mut layout);
    }

    let theme = &ctx.theme;
    let muted = Style::default().fg(theme.text_muted).bg(theme.bg);
    if let Some(notice) = frame.notice {
        f.render_widget(
            Paragraph::new(notice_line(ctx, notice)).style(Style::default().bg(theme.bg)),
            notice_area,
        );
    }
    if !form.buttons.is_empty() {
        f.render_widget(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme.border)),
            separator,
        );
        layout.buttons = draw_buttons(f, ctx, form, buttons_area);
    }
    let hints = frame.hints.unwrap_or_else(|| form.key_hints());
    f.render_widget(Paragraph::new(hints).style(muted), footer);
    form.layout = Some(layout);
}

fn outer_extent(extent: Extent, available: u16, content: u16) -> u16 {
    let wanted = match extent {
        Extent::Fixed(size) => size,
        Extent::Percent { percent, min } => {
            let share = u32::from(available) * u32::from(percent) / 100;
            u16::try_from(share).unwrap_or(u16::MAX).max(min)
        }
        Extent::Content => content,
    };
    wanted.min(available)
}

fn centered(width: u16, height: u16, screen: Rect) -> Rect {
    Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    )
}

fn natural_rows<F>(group: &Group<F>, width: usize) -> u16 {
    let rows = match group {
        Group::Info(text) | Group::Warning(text) if text.is_empty() => 0,
        Group::Info(text) | Group::Warning(text) => wrap_text(text, width).len(),
        Group::Radio(radio) if radio.choices.len() > 1 => {
            usize::from(!radio.label.is_empty()) + radio.choices.len()
        }
        Group::Radio(_) => 1,
        Group::Toggles(toggles) => toggles.len(),
        Group::LineInput(input) => wrap_text(&input.buffer.value_with_cursor('▋'), width)
            .len()
            .max(1),
        Group::Custom(Rows::Fixed(rows)) => usize::from(*rows),
        Group::Custom(Rows::Percent(_) | Rows::Fill) => 0,
    };
    u16::try_from(rows).unwrap_or(u16::MAX)
}

/// One blank row between two groups that both take rows.
fn gap_rows(natural: &[u16]) -> u16 {
    let shown = natural.iter().filter(|&&rows| rows > 0).count() as u16;
    shown.saturating_sub(1)
}

/// Top to bottom; a later group gets what the earlier ones leave, so on a
/// short screen the last groups shrink first. Custom regions sized by share
/// or fill take their rows once the rest is placed.
fn allocate<F>(groups: &[Group<F>], natural: &[u16], body: Rect) -> Vec<Rect> {
    let flexible: u16 = groups
        .iter()
        .map(|group| match group {
            Group::Custom(Rows::Percent(percent)) => {
                u16::try_from(u32::from(body.height) * u32::from(*percent) / 100)
                    .unwrap_or(u16::MAX)
            }
            _ => 0,
        })
        .sum();
    let fixed: u16 = natural.iter().sum::<u16>() + gap_rows(natural);
    let fill = body.height.saturating_sub(fixed.saturating_add(flexible));
    let mut fill_left = fill;

    let mut y = body.y;
    let bottom = body.y + body.height;
    let mut previous_shown = false;
    groups
        .iter()
        .zip(natural)
        .map(|(group, &rows)| {
            let wanted = match group {
                Group::Custom(Rows::Percent(percent)) => {
                    u16::try_from(u32::from(body.height) * u32::from(*percent) / 100)
                        .unwrap_or(u16::MAX)
                }
                Group::Custom(Rows::Fill) => std::mem::take(&mut fill_left),
                _ => rows,
            };
            if wanted > 0 && previous_shown {
                y = (y + 1).min(bottom);
            }
            let height = wanted.min(bottom - y);
            previous_shown |= height > 0;
            let rect = Rect::new(body.x, y, body.width, height);
            y += height;
            rect
        })
        .collect()
}

fn focused_style(ctx: &RenderCtx<'_>) -> Style {
    Style::default()
        .fg(ctx.theme.highlight_bg)
        .bg(ctx.theme.border_focused)
        .add_modifier(Modifier::BOLD)
}

fn draw_group<F>(
    f: &mut Frame,
    ctx: &RenderCtx<'_>,
    group: &Group<F>,
    index: usize,
    rect: Rect,
    focus: Option<Focus>,
    layout: &mut FormLayout,
) {
    if rect.height == 0 {
        return;
    }
    let theme = &ctx.theme;
    let icons = ctx.icons;
    let text = Style::default().fg(theme.text_normal).bg(theme.bg);
    let is_focused = |item: usize| focus == Some(Focus::Item { group: index, item });
    let row = |offset: u16| Rect {
        y: rect.y + offset,
        height: 1,
        ..rect
    };
    match group {
        Group::Info(body) => {
            let lines: Vec<Line> = wrap_text(body, usize::from(rect.width))
                .into_iter()
                .map(Line::from)
                .collect();
            f.render_widget(Paragraph::new(lines).style(text), rect);
        }
        Group::Warning(body) => {
            let lines: Vec<Line> = wrap_text(body, usize::from(rect.width))
                .into_iter()
                .map(Line::from)
                .collect();
            f.render_widget(
                Paragraph::new(lines).style(Style::default().fg(theme.red).bg(theme.bg)),
                rect,
            );
        }
        Group::Radio(radio) if radio.choices.len() > 1 => {
            let mut offset = 0;
            if !radio.label.is_empty() {
                f.render_widget(
                    Paragraph::new(radio.label.as_str()).style(
                        Style::default()
                            .fg(theme.text_muted)
                            .bg(theme.bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    row(0),
                );
                offset = 1;
            }
            for (item, choice) in radio.choices.iter().enumerate() {
                let item_offset = offset + item as u16;
                if item_offset >= rect.height {
                    break;
                }
                let mark = if item == radio.selected {
                    &icons.radio_on
                } else {
                    &icons.radio_off
                };
                let style = if is_focused(item) {
                    focused_style(ctx)
                } else {
                    text
                };
                let item_rect = row(item_offset);
                f.render_widget(
                    Paragraph::new(format!("{mark} {}", choice.label)).style(style),
                    item_rect,
                );
                layout
                    .items
                    .push((Focus::Item { group: index, item }, item_rect));
            }
        }
        Group::Radio(radio) => {
            let choice = radio
                .choices
                .first()
                .map(|choice| choice.label.as_str())
                .unwrap_or_default();
            let line = if radio.label.is_empty() {
                Line::from(Span::styled(choice.to_string(), text))
            } else {
                Line::from(vec![
                    Span::styled(
                        format!("{}: ", radio.label),
                        Style::default().fg(theme.text_muted).bg(theme.bg),
                    ),
                    Span::styled(choice.to_string(), text),
                ])
            };
            f.render_widget(Paragraph::new(line), row(0));
        }
        Group::Toggles(toggles) => {
            for (item, toggle) in toggles.iter().enumerate() {
                if item as u16 >= rect.height {
                    break;
                }
                let mark = if toggle.checked {
                    &icons.check_on
                } else {
                    &icons.check_off
                };
                let style = if is_focused(item) {
                    focused_style(ctx)
                } else {
                    text
                };
                let item_rect = row(item as u16);
                f.render_widget(
                    Paragraph::new(format!("{mark} {}", toggle.label)).style(style),
                    item_rect,
                );
                layout
                    .items
                    .push((Focus::Item { group: index, item }, item_rect));
            }
        }
        Group::LineInput(input) => {
            let value = sanitize_untrusted(input.buffer.value()).into_owned();
            let shown = if is_focused(0) {
                let mut with_cursor = value;
                let at = input.buffer.cursor_byte_offset().min(with_cursor.len());
                if with_cursor.is_char_boundary(at) {
                    with_cursor.insert(at, '▋');
                }
                with_cursor
            } else {
                value
            };
            let lines: Vec<Line> = wrap_text(&shown, usize::from(rect.width))
                .into_iter()
                .map(Line::from)
                .collect();
            f.render_widget(
                Paragraph::new(lines).style(text).wrap(Wrap { trim: false }),
                rect,
            );
            layout.items.push((
                Focus::Item {
                    group: index,
                    item: 0,
                },
                rect,
            ));
        }
        Group::Custom(_) => {}
    }
}

fn draw_buttons<F>(f: &mut Frame, ctx: &RenderCtx<'_>, form: &Form<F>, area: Rect) -> Vec<Rect> {
    let theme = &ctx.theme;
    let icons = ctx.icons;
    let count = form.buttons.len() as u32;
    let constraints: Vec<Constraint> = (0..form.buttons.len())
        .flat_map(|i| {
            let spacer = (i > 0).then_some(Constraint::Length(1));
            spacer
                .into_iter()
                .chain(std::iter::once(Constraint::Ratio(1, count)))
        })
        .collect();
    let cells = Layout::horizontal(constraints).split(area);
    let focus = form.focus();
    form.buttons
        .iter()
        .enumerate()
        .map(|(index, button)| {
            let (label, accent): (String, Option<Color>) = match button.role {
                ButtonRole::Submit { is_destructive } => (
                    format!("{} {}", icons.check_on, button.label),
                    Some(if is_destructive {
                        theme.red
                    } else {
                        theme.green
                    }),
                ),
                ButtonRole::Extra { is_destructive } => {
                    (button.label.clone(), is_destructive.then_some(theme.red))
                }
                ButtonRole::Cancel => (format!("{} {}", icons.check_off, button.label), None),
            };
            let cell = cells[index * 2];
            f.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(button_style(
                        theme,
                        focus == Some(Focus::Button(index)),
                        accent,
                    )),
                cell,
            );
            cell
        })
        .collect()
}

/// One row inside a form for a notice about the overlay's own request.
fn notice_line(ctx: &RenderCtx<'_>, notice: &Notice) -> Line<'static> {
    let theme = &ctx.theme;
    let mut spans = match notice.level() {
        NoticeLevel::Error => vec![Span::styled(
            format!(
                "{} {}",
                ctx.icons.status_failed,
                sanitize_untrusted(notice.text())
            ),
            Style::default().fg(theme.red).add_modifier(Modifier::BOLD),
        )],
        NoticeLevel::Info => vec![Span::styled(
            sanitize_untrusted(notice.text()).into_owned(),
            Style::default().fg(theme.yellow),
        )],
    };
    if let Some(detail) = notice.detail() {
        spans.push(Span::styled(
            format!(" {}", sanitize_untrusted(detail)),
            Style::default().fg(theme.text_muted),
        ));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::{Button, Choice, Radio, Toggle};
    use ratatui::{Terminal, backend::TestBackend};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Id {
        A,
        B,
        Flag,
        Go,
        Cancel,
    }

    fn sample_form() -> Form<Id> {
        Form::new(
            vec![
                Group::Info("A body long enough to wrap onto a second row here.".to_string()),
                Group::Radio(Radio {
                    label: "Method".to_string(),
                    choices: vec![
                        Choice {
                            id: Id::A,
                            label: "Alpha".to_string(),
                        },
                        Choice {
                            id: Id::B,
                            label: "Beta".to_string(),
                        },
                    ],
                    selected: 0,
                }),
                Group::Toggles(vec![Toggle {
                    id: Id::Flag,
                    label: "Flag".to_string(),
                    checked: true,
                }]),
            ],
            vec![
                Button {
                    id: Id::Go,
                    label: "Go".to_string(),
                    role: ButtonRole::Submit {
                        is_destructive: false,
                    },
                },
                Button {
                    id: Id::Cancel,
                    label: "Cancel".to_string(),
                    role: ButtonRole::Cancel,
                },
            ],
        )
    }

    fn draw(form: &mut Form<Id>, width: u16, height: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let icons = crate::config::Icons::ascii();
        let ctx = RenderCtx {
            theme: crate::config::Theme::default(),
            icons: &icons,
            kind: crate::backend::BackendKind::GitLab,
        };
        terminal
            .draw(|f| {
                let frame = FormFrame {
                    title: "Sample",
                    size: FormSize {
                        width: Extent::Fixed(40),
                        height: Extent::Content,
                    },
                    hints: None,
                    notice: None,
                };
                render_form(f, &ctx, f.area(), &frame, form);
            })
            .unwrap();
        terminal
    }

    fn row_text(terminal: &Terminal<TestBackend>, y: u16) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn a_screen_too_short_for_the_body_still_shows_the_buttons_and_the_footer() {
        let mut form = sample_form();
        let terminal = draw(&mut form, 40, 7);
        let layout = form.layout.clone().unwrap();
        assert_eq!(layout.area.height, 7);
        assert_eq!(layout.buttons.len(), 2);
        assert_eq!(layout.buttons[0].y, 4);
        assert!(row_text(&terminal, 4).contains("+ Go"));
        assert!(row_text(&terminal, 5).contains("j/k move"));
    }

    #[test]
    fn clicks_resolve_to_the_control_or_button_drawn_there() {
        let mut form = sample_form();
        draw(&mut form, 60, 24);
        let layout = form.layout.clone().unwrap();
        let (beta_focus, beta_rect) = layout.items[1];
        assert_eq!(beta_focus, Focus::Item { group: 1, item: 1 });
        assert_eq!(layout.focus_at(beta_rect.x, beta_rect.y), Some(beta_focus));
        let cancel = layout.buttons[1];
        assert_eq!(
            layout.focus_at(cancel.x + cancel.width - 1, cancel.y),
            Some(Focus::Button(1))
        );
        assert_eq!(layout.focus_at(layout.area.x, layout.area.y), None);
    }

    #[test]
    fn content_height_fits_every_group_with_one_row_between_them() {
        let mut form = sample_form();
        draw(&mut form, 60, 30);
        let layout = form.layout.unwrap();
        // border, padding, 2 info rows, gap, 3 radio rows, gap, 1 toggle,
        // separator, buttons, footer, border
        assert_eq!(
            layout.area.height,
            1 + 1 + 2 + 1 + 3 + 1 + 1 + 1 + 1 + 1 + 1
        );
        assert_eq!(layout.groups[1].y, layout.groups[0].bottom() + 1);
    }
}
