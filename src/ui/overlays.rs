use super::form::{Extent, FormFrame, FormSize, render_form};
use super::helpers::highlight_fuzzy_match;
use crate::app::SaveMenu;
use crate::app::{App, Tab};
use crate::config::Icons;
use crate::keymap::{Action, KeyContext, Keymap};
use crate::ui::render_ctx::RenderCtx;
use crate::utils::format::sanitize_untrusted;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table},
};
use std::borrow::Cow;

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

pub(crate) fn render_overlays(f: &mut Frame, ctx: &RenderCtx<'_>, app: &mut App, size: Rect) {
    app.overlay_stack.clear();
    let icons = ctx.icons;
    // EditMenu is rendered as a full-zoom interactive inspector in the detail
    // pane (ui/mod.rs::render_edit_menu_if_active); register its area for mouse
    // scroll/click handling so it participates in overlay z-ordering.
    if app.edit_menu().is_some() {
        if let Some(rect) = app.detail_rect {
            app.overlay_stack
                .push((crate::app::OverlayKind::EditMenu, rect));
        }
    }

    // Below every other overlay: its comment actions and reply prompts open on top.
    super::review_threads::render_review_threads(f, ctx, app, size);
    // Below the re-run confirmation it opens.
    super::terminal_inspector::render_terminal_inspector(f, ctx, app, size);

    render_selector(f, ctx, app, size);

    render_text_input(f, ctx, app, size);
    render_date_picker(f, ctx, app, size);

    if app.focus_column_checklist() && app.selector().is_none() {
        let (active_idx, editing_page_size, save_menu_open, save_menu_selection) =
            match app.configure() {
                Some(cfg) => (
                    cfg.column_checklist_idx,
                    cfg.editing_page_size,
                    cfg.save_menu_open,
                    cfg.save_menu_selection,
                ),
                None => (0, false, false, None),
            };
        let tab = app.active_tab;
        let kind = ctx.kind;
        let cols = tab.columns(kind, app.scope.is_group());

        let group_cols: Vec<&str> = cols.to_vec();

        let cols_end = cols.len();
        let group_end = cols_end + group_cols.len();
        let order_end = group_end + 2;
        let page_size_idx = order_end;
        let prefetch_idx = page_size_idx + 1;
        let theme_idx = prefetch_idx + 1;
        let save_end = theme_idx + 1;

        // Build the entire Configure view as one flat, scrollable list so the
        // section headers scroll together with their items.
        let mut lines: Vec<(Option<usize>, ListItem)> = Vec::new();
        let mut active_line: Option<usize> = None;

        let t = &ctx.theme;

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
        let page_size_value = if editing_page_size {
            format!("[ {}| ]", app.page_size_input)
        } else {
            format!("[ {} ]", app.page_size)
        };
        let page_size_style = if editing_page_size {
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
        let page_size_line = if is_page_size_active || editing_page_size {
            Line::from(Span::styled(
                format!(
                    " {} Page Size      {} ",
                    icons.label_page_size, page_size_value
                ),
                page_size_style,
            ))
        } else {
            Line::from(vec![
                Span::styled(
                    format!(" {} Page Size      ", icons.label_page_size),
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

        // Prefetch Tabs — inline row (icon + label in header_fg, value in text_normal)
        let is_prefetch_active = active_idx == prefetch_idx;
        let prefetch_value = format!("[ {} ]", app.config.prefetch_tabs);
        let prefetch_style = if is_prefetch_active {
            Style::default()
                .fg(t.highlight_bg)
                .bg(t.border_focused)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text_normal)
        };
        let prefetch_line = if is_prefetch_active {
            Line::from(Span::styled(
                format!(
                    " {} Prefetch Tabs  {} ",
                    icons.label_fetching, prefetch_value
                ),
                prefetch_style,
            ))
        } else {
            Line::from(vec![
                Span::styled(
                    format!(" {} Prefetch Tabs  ", icons.label_fetching),
                    Style::default()
                        .fg(t.header_fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(prefetch_value, Style::default().fg(t.text_normal)),
            ])
        };
        if is_prefetch_active {
            active_line = Some(lines.len());
        }
        lines.push((
            Some(prefetch_idx),
            ListItem::new(prefetch_line).style(prefetch_style),
        ));

        // Theme — inline row (icon + label in purple, value aligned with Page Size & Prefetch Tabs)
        let current_theme_name = app.config.theme_preset.as_deref().unwrap_or("default");
        let is_theme_active = active_idx == theme_idx;
        let theme_value = format!("[ {current_theme_name} ]");
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
                format!(" {} Theme          {} ", icons.label_theme, theme_value),
                theme_style,
            ))
        } else {
            Line::from(vec![
                Span::styled(
                    format!(" {} Theme          ", icons.label_theme),
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

        let is_save_selected = active_idx == save_end;
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
            ListItem::new(
                Line::from(format!("{} Save View", icons.label_save)).alignment(Alignment::Center),
            )
            .style(save_button_style),
        ));

        let title = format!(
            "{} Configure View: {}",
            icons.label_configure,
            tab.title(kind, ctx.icons)
        );
        let list_rows = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        let mut form = crate::form::Form::panel(vec![crate::form::Group::region(
            crate::form::Rows::Fixed(list_rows),
        )]);
        let hints = if editing_page_size {
            CONFIGURE_PAGE_SIZE_HINTS
        } else {
            CONFIGURE_HINTS
        };
        let frame = FormFrame {
            title: &title,
            size: FormSize {
                width: Extent::Fixed(CONFIGURE_WIDTH),
                height: Extent::Content,
            },
            hints: Some(hints),
            notice: None,
        };
        render_form(f, ctx, size, &frame, &mut form);
        if let Some(layout) = form.layout {
            let targets: Vec<Option<usize>> = lines.iter().map(|(target, _)| *target).collect();
            let items: Vec<ListItem> = lines.into_iter().map(|(_, li)| li).collect();
            let mut state = ListState::default();
            state.select(active_line);
            f.render_stateful_widget(List::new(items), layout.groups[0], &mut state);
            app.overlay_stack
                .push((crate::app::OverlayKind::Configure, layout.area));
            if let Some(configure) = app.configure_mut() {
                configure.visible_rows = targets.into_iter().skip(state.offset()).collect();
                configure.layout = Some(layout);
            }
        }

        if save_menu_open {
            render_save_menu(f, ctx, app, size, save_menu_selection);
        }
    }

    render_submit_dialog(f, ctx, app, size);

    super::review_submit::render_review_submit(f, ctx, app, size);
    render_help(f, ctx, app, size);
}

const CONFIGURE_WIDTH: u16 = 64;
const CONFIGURE_HINTS: &str = "j/k · J/K section · Space toggle · Enter pick · Esc close";
const CONFIGURE_PAGE_SIZE_HINTS: &str = "Type digits · Enter save · Esc cancel";
const SAVE_MENU_WIDTH: u16 = 32;
const SAVE_MENU_HINTS: &str = "j/k · Enter pick · Esc back";
/// The save submenu's choices, top to bottom.
pub(crate) const SAVE_MENU_CHOICES: [(SaveMenu, &str); 3] = [
    (SaveMenu::Local, "Local Repo"),
    (SaveMenu::Global, "Global"),
    (SaveMenu::Cancel, "Cancel"),
];

fn render_save_menu(
    f: &mut Frame,
    ctx: &RenderCtx<'_>,
    app: &mut App,
    size: Rect,
    selection: Option<SaveMenu>,
) {
    let title = format!("{} Save to Config", ctx.icons.label_save);
    let mut form = crate::form::Form::panel(vec![crate::form::Group::region(
        crate::form::Rows::Fixed(SAVE_MENU_CHOICES.len() as u16),
    )]);
    let frame = FormFrame {
        title: &title,
        size: FormSize {
            width: Extent::Fixed(SAVE_MENU_WIDTH),
            height: Extent::Content,
        },
        hints: Some(SAVE_MENU_HINTS),
        notice: None,
    };
    render_form(f, ctx, size, &frame, &mut form);
    let Some(layout) = form.layout else {
        return;
    };
    let items: Vec<ListItem> = SAVE_MENU_CHOICES
        .iter()
        .map(|&(choice, label)| {
            let style = if selection == Some(choice) {
                Style::default()
                    .fg(ctx.theme.highlight_bg)
                    .bg(ctx.theme.border_focused)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ctx.theme.text_normal)
            };
            ListItem::new(label).style(style)
        })
        .collect();
    f.render_widget(List::new(items), layout.groups[0]);
    app.overlay_stack
        .push((crate::app::OverlayKind::SaveMenu, layout.area));
    if let Some(configure) = app.configure_mut() {
        configure.save_menu_layout = Some(layout);
    }
}

const TEXT_INPUT_HINTS: &str = "Enter submit · Ctrl+E editor · Esc cancel";
const DATE_PICKER_HINTS: &str = "hjkl day · [ ] month · Enter · Esc";
/// Month line, weekday header and six weeks.
const CALENDAR_ROWS: u16 = 8;
/// Columns of one day in the date picker's grid.
const DAY_COLUMNS: u16 = 4;
/// Columns between two neighbouring days in the date picker's grid.
const DAY_GAP_COLUMNS: u16 = 1;
/// Seven days with a gap between each two, plus the frame.
const DATE_PICKER_WIDTH: u16 = 7 * DAY_COLUMNS + 6 * DAY_GAP_COLUMNS + 4;
/// Columns of the previous and next month arrows at each end of the month line.
const MONTH_ARROW_COLUMNS: u16 = 4;

/// The previous-month arrow, the month and the next-month arrow on the
/// date picker's month line; clicks resolve against the same cells.
pub(crate) fn date_picker_month_cells(calendar: Rect) -> [Rect; 3] {
    let [month_line, _] = date_picker_calendar_rows(calendar);
    Layout::horizontal([
        Constraint::Length(MONTH_ARROW_COLUMNS),
        Constraint::Fill(1),
        Constraint::Length(MONTH_ARROW_COLUMNS),
    ])
    .areas(month_line)
}

/// The month line, then the day grid: the weekday header row and the weeks.
fn date_picker_calendar_rows(calendar: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(calendar)
}

/// The weekday (0 is Sunday) and the week of the day cell drawn at a point
/// of the calendar, a gap counting as the day before it.
pub(crate) fn date_picker_day_at(calendar: Rect, column: u16, row: u16) -> Option<(u32, u32)> {
    let [_, grid] = date_picker_calendar_rows(calendar);
    if !grid.contains(ratatui::layout::Position::new(column, row)) {
        return None;
    }
    let week = (row - grid.y).checked_sub(1)?;
    let weekday = (column - grid.x) / (DAY_COLUMNS + DAY_GAP_COLUMNS);
    (weekday < 7).then_some((u32::from(weekday), u32::from(week)))
}

fn render_text_input(f: &mut Frame, ctx: &RenderCtx<'_>, app: &mut App, size: Rect) {
    let Some(text_input) = app.text_input_mut() else {
        return;
    };
    let frame = FormFrame {
        title: &text_input.title,
        size: FormSize {
            width: Extent::Percent {
                percent: 60,
                min: 36,
            },
            height: Extent::Content,
        },
        hints: Some(TEXT_INPUT_HINTS),
        notice: None,
    };
    render_form(f, ctx, size, &frame, &mut text_input.form);
    if let Some(area) = text_input.form.layout.as_ref().map(|layout| layout.area) {
        app.overlay_stack
            .push((crate::app::OverlayKind::TextInput, area));
    }
}

fn month_name(month: u32) -> &'static str {
    match month {
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
    }
}

fn render_date_picker(f: &mut Frame, ctx: &RenderCtx<'_>, app: &mut App, size: Rect) {
    let Some(date_picker) = app.date_picker_mut() else {
        return;
    };
    let mut form = crate::form::Form::panel(vec![crate::form::Group::region(
        crate::form::Rows::Fixed(CALENDAR_ROWS),
    )]);
    let frame = FormFrame {
        title: &date_picker.title,
        size: FormSize {
            width: Extent::Fixed(DATE_PICKER_WIDTH),
            height: Extent::Content,
        },
        hints: Some(DATE_PICKER_HINTS),
        notice: None,
    };
    render_form(f, ctx, size, &frame, &mut form);
    let Some(layout) = form.layout else {
        return;
    };
    let calendar = layout.groups[0];
    let area = layout.area;

    let [_, grid_area] = date_picker_calendar_rows(calendar);
    let [prev_area, title_area, next_area] = date_picker_month_cells(calendar);
    let month_style = Style::default()
        .fg(ctx.theme.header_fg)
        .add_modifier(Modifier::BOLD);
    for (text, area) in [
        (ctx.icons.nav_prev.clone(), prev_area),
        (
            format!("{} {}", month_name(date_picker.month), date_picker.year),
            title_area,
        ),
        (ctx.icons.nav_next.clone(), next_area),
    ] {
        f.render_widget(
            Paragraph::new(text)
                .alignment(Alignment::Center)
                .style(month_style),
            area,
        );
    }

    let header = Row::new(
        ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"]
            .into_iter()
            .map(|day| Cell::from(Line::from(day).alignment(Alignment::Center))),
    )
    .style(Style::default().fg(ctx.theme.text_muted));
    let rows = (0..6u32).map(|week| {
        Row::new(
            (0..7u32).map(|weekday| match date_picker.day_in_cell(weekday, week) {
                Some(day) => {
                    let style = if day == date_picker.day {
                        Style::default()
                            .bg(ctx.theme.highlight_bg)
                            .fg(ctx.theme.header_fg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(ctx.theme.text_normal)
                    };
                    Cell::from(
                        Line::from(day.to_string())
                            .alignment(Alignment::Center)
                            .style(style),
                    )
                }
                None => Cell::from(""),
            }),
        )
    });
    f.render_widget(
        Table::new(rows, [Constraint::Length(DAY_COLUMNS); 7])
            .header(header)
            .column_spacing(DAY_GAP_COLUMNS),
        grid_area,
    );
    date_picker.layout = Some(layout);
    app.overlay_stack
        .push((crate::app::OverlayKind::DatePicker, area));
}

/// The bordered filter box above a selector's list.
const SELECTOR_FILTER_ROWS: u16 = 3;

/// The selector's key footer, naming the first key bound to each action so
/// it stays true after a remap. Filtering reads raw keys, so its hint is fixed.
fn selector_hints(selector: &crate::app::Selector, keys: &Keymap) -> String {
    if selector.is_filtering {
        return "Type to filter · Enter / Esc done".to_string();
    }
    let first = |action| {
        keys.bindings(action)
            .iter()
            .map(|key| key.trim())
            .find(|key| !key.is_empty())
    };
    let movement = match (first(Action::SelectorNext), first(Action::SelectorPrevious)) {
        (Some(next), Some(previous)) => Some(format!("{next}/{previous}")),
        (next, previous) => next.or(previous).map(str::to_string),
    };
    let confirm = if selector.multi_select {
        "confirm"
    } else {
        "pick"
    };
    let toggle = first(Action::SelectorToggle).filter(|_| selector.multi_select);
    let filter = first(Action::SelectorFilter).filter(|_| selector.purpose.has_filter_bar());
    [
        first(Action::SelectorConfirm).map(|key| format!("{key} {confirm}")),
        first(Action::SelectorClose).map(|key| format!("{key} close")),
        movement.map(|keys| format!("{keys} move")),
        toggle.map(|key| format!("{key} select")),
        filter.map(|key| format!("{key} filter")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// Draws the open selector. What differs between selectors is read from its purpose.
fn render_selector(f: &mut Frame, ctx: &RenderCtx<'_>, app: &mut App, size: Rect) {
    let notice = app.modal_notice().cloned();
    let App {
        modal_stack,
        overlay_stack,
        switch_repo_groups,
        switch_repo_paths,
        selector_layout,
        config,
        ..
    } = app;
    let Some(crate::app::Modal::Selector(selector)) = modal_stack
        .iter_mut()
        .rev()
        .find(|m| matches!(m, crate::app::Modal::Selector(_)))
    else {
        return;
    };
    let has_filter = selector.purpose.has_filter_bar();
    let mut groups = Vec::with_capacity(2);
    if has_filter {
        groups.push(crate::form::Group::region(crate::form::Rows::Fixed(
            SELECTOR_FILTER_ROWS,
        )));
    }
    groups.push(crate::form::Group::region(crate::form::Rows::Fill));
    let mut form = crate::form::Form::panel(groups);
    let hints = selector_hints(selector, &config.keybindings);
    let frame = FormFrame {
        title: selector.title.trim(),
        size: FormSize {
            width: Extent::Percent {
                percent: 70,
                min: 60,
            },
            height: Extent::Percent {
                percent: 70,
                min: 8,
            },
        },
        hints: Some(&hints),
        notice: notice.as_ref(),
    };
    render_form(f, ctx, size, &frame, &mut form);
    let Some(layout) = form.layout else {
        return;
    };
    let theme = &ctx.theme;
    let icons = ctx.icons;

    if has_filter {
        let search_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if selector.is_filtering {
                theme.border_focused
            } else {
                theme.border
            }))
            .title(" Filter ");
        let (search_text, search_style) = if selector.is_filtering {
            (
                format!("{}{}", selector.search_query, icons.input_cursor),
                Style::default().fg(theme.text_normal),
            )
        } else if selector.search_query.is_empty() {
            (
                "Type to filter...".to_string(),
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::ITALIC),
            )
        } else {
            (
                selector.search_query.clone(),
                Style::default().fg(theme.text_normal),
            )
        };
        f.render_widget(
            Paragraph::new(Span::styled(search_text, search_style)).block(search_block),
            layout.groups[0],
        );
    }

    let list_area = layout.groups[layout.groups.len() - 1];
    let filtered_items = selector.get_filtered_items_with_indices();
    let placeholder = if selector.is_loading {
        Some("Loading options…")
    } else if filtered_items.is_empty() {
        Some("No matching options found.")
    } else {
        None
    };
    if let Some(placeholder) = placeholder {
        f.render_widget(
            Paragraph::new(placeholder).style(
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ),
            list_area,
        );
    } else {
        let is_switch_repo = selector.purpose == crate::app::SelectorPurpose::SwitchRepo;
        let items: Vec<ListItem> = filtered_items
            .iter()
            .enumerate()
            .map(|(i, (item, indices))| {
                let is_selected = if item.starts_with("+ Create \"") {
                    selector
                        .selected_items
                        .contains(selector.search_query.trim())
                } else {
                    selector.selected_items.contains(item)
                };
                let is_cursor = i == selector.cursor_idx;
                let item_bg = if is_cursor {
                    theme.highlight_bg
                } else {
                    theme.bg
                };
                let text_style = Style::default().fg(theme.text_normal).bg(item_bg);
                let style = if is_cursor {
                    text_style.add_modifier(Modifier::BOLD)
                } else {
                    text_style
                };
                let match_style = Style::default()
                    .fg(theme.yellow)
                    .bg(item_bg)
                    .add_modifier(Modifier::BOLD);
                let (marker, marker_color) = if is_selected {
                    (&icons.check_on, theme.green)
                } else {
                    (&icons.check_off, theme.text_muted)
                };
                let mut spans = vec![Span::styled(
                    format!(" {marker} "),
                    Style::default()
                        .fg(marker_color)
                        .bg(item_bg)
                        .add_modifier(Modifier::BOLD),
                )];
                let is_group = is_switch_repo && switch_repo_groups.contains(item);
                if is_group {
                    spans.push(Span::styled(
                        format!("{} ", icons.label_group),
                        Style::default().fg(theme.text_muted).bg(item_bg),
                    ));
                }
                // The fuzzy indices count characters of the raw item, so the
                // highlight is only kept when sanitizing changed nothing.
                match (indices, sanitize_untrusted(item)) {
                    (Some(indices), Cow::Borrowed(_)) => {
                        spans.extend(highlight_fuzzy_match(item, indices, style, match_style))
                    }
                    (_, clean) => spans.push(Span::styled(clean.into_owned(), style)),
                }
                // Repos sharing a basename are told apart by their path.
                if is_switch_repo && !is_group {
                    if let Some(path) = switch_repo_paths
                        .get(item)
                        .filter(|path| !path.is_empty() && *path != item)
                    {
                        spans.push(Span::styled(
                            format!(" {path}"),
                            Style::default().fg(theme.text_muted).bg(item_bg),
                        ));
                    }
                }
                ListItem::new(Line::from(spans)).style(Style::default().bg(item_bg))
            })
            .collect();
        let mut state = selector.state;
        f.render_stateful_widget(
            List::new(items).style(Style::default().bg(theme.bg)),
            list_area,
            &mut state,
        );
        selector.state = state;
    }
    overlay_stack.push((crate::app::OverlayKind::Selector, layout.area));
    *selector_layout = Some(layout);
}

fn render_submit_dialog(f: &mut Frame, ctx: &RenderCtx<'_>, app: &mut App, size: Rect) {
    let Some(dialog) = app.submit_dialog_mut() else {
        return;
    };
    let title = format!(
        "{} {}",
        submit_dialog_icon(&dialog.action, ctx.icons),
        dialog.form.title
    );
    let area = render_form(f, ctx, size, &FormFrame::dialog(&title), &mut dialog.form);
    app.overlay_stack
        .push((crate::app::OverlayKind::ConfirmPopup, area));
}

fn submit_dialog_icon<'a>(action: &crate::app::ConfirmAction, icons: &'a Icons) -> &'a str {
    use crate::app::ConfirmAction;
    match action {
        ConfirmAction::DeleteMilestone(_)
        | ConfirmAction::DeleteRelease(_)
        | ConfirmAction::DeleteBranch(_)
        | ConfirmAction::BulkDeleteBranches(_)
        | ConfirmAction::DeleteIssue(_)
        | ConfirmAction::DeleteMr(_) => &icons.action_delete,
        ConfirmAction::CloseIssue(_)
        | ConfirmAction::CloseMr(_)
        | ConfirmAction::CloseMilestone(_)
        | ConfirmAction::StopEnvironment(_) => &icons.action_close,
        ConfirmAction::ReopenIssue(_)
        | ConfirmAction::ReopenMr(_)
        | ConfirmAction::ReopenMilestone(_) => &icons.action_reopen,
        ConfirmAction::MergeMr(_)
        | ConfirmAction::MergeStack(_)
        | ConfirmAction::BulkMergeMrs(_) => &icons.action_merge,
        ConfirmAction::RevokeMr(_) | ConfirmAction::SubmitReview(_) => &icons.action_review,
        ConfirmAction::RebaseMr(_) => &icons.merge_rebase,
        ConfirmAction::MarkAllTodosRead => &icons.tab_todo,
        ConfirmAction::CancelPipeline(_) | ConfirmAction::CancelJobs { .. } => {
            &icons.status_canceled
        }
        ConfirmAction::RerunCommand(_) => &icons.tab_terminal,
        ConfirmAction::RollbackDeployment { .. } => &icons.label_deployment,
    }
}

const CUSTOM_COMMANDS_CATEGORY: &str = "Custom Commands";

fn help_category(context: KeyContext, is_github: bool) -> &'static str {
    match context {
        KeyContext::Global | KeyContext::ListView => "Global & Nav",
        KeyContext::EditMenu => "Inspector / Editor",
        KeyContext::DiffView => "Diff View",
        KeyContext::Selector => "Selector / Filter",
        KeyContext::Dialog => "Confirm Dialog",
        KeyContext::Tab(tab) => match tab {
            Tab::Issues => "Issues",
            Tab::MergeRequests if is_github => "Pull Requests",
            Tab::MergeRequests => "Merge Requests",
            Tab::Pipelines => "Pipelines",
            Tab::Jobs => "Jobs",
            Tab::Runners => "Runners",
            Tab::Releases => "Releases",
            Tab::Todos => "TODOs",
            Tab::Milestones => "Milestones",
            Tab::Branches => "Branches",
            Tab::Environments => "Environments",
            Tab::Terminal => "Terminal",
        },
    }
}

pub(crate) fn render_help(f: &mut Frame, ctx: &RenderCtx<'_>, app: &mut App, size: Rect) {
    if !app.show_help() {
        return;
    }

    let icons = ctx.icons;
    let is_github = ctx.kind.is_github();

    struct Shortcut {
        category: &'static str,
        key: std::borrow::Cow<'static, str>,
        action: String,
    }

    let s = |k: &'static str| std::borrow::Cow::Borrowed(k);
    let d = |k: String| std::borrow::Cow::Owned(k);

    let custom_commands: Vec<crate::custom_commands::CustomCommand> = if app.diff_view().is_some() {
        app.custom_commands.in_diff().cloned().collect()
    } else {
        app.custom_commands
            .reachable_on(app.active_tab)
            .cloned()
            .collect()
    };

    let mut shortcuts: Vec<Shortcut> = Action::ALL
        .iter()
        .filter(|action| action.spec().support.includes(ctx.kind))
        .map(|&action| {
            let spec = action.spec();
            Shortcut {
                category: help_category(spec.context, is_github),
                key: d(app.config.keybindings.display(action)),
                action: spec.label_for(ctx.kind),
            }
        })
        .collect();

    shortcuts.extend([
        Shortcut {
            category: "Inspector / Editor",
            key: s("j / k / ↓ / ↑ / Tab / Shift+Tab"),
            action: "Navigate fields & buttons".to_string(),
        },
        Shortcut {
            category: "Inspector / Editor",
            key: s("Enter / Space"),
            action: "Edit field / Select option / Submit".to_string(),
        },
        Shortcut {
            category: "Inspector / Editor",
            key: s("Ctrl+E"),
            action: "Edit description / notes in $EDITOR".to_string(),
        },
        Shortcut {
            category: "Inspector / Editor",
            key: s("J / K"),
            action: "Scroll description / notes pane".to_string(),
        },
        Shortcut {
            category: "Inspector / Editor",
            key: s("Esc"),
            action: "Exit inspector / Close editor form".to_string(),
        },
        Shortcut {
            category: "Global & Nav",
            key: s("Ctrl+W / Ctrl+U"),
            action: "Search: delete previous word / clear line".to_string(),
        },
        Shortcut {
            category: "Global & Nav",
            key: s("Esc"),
            action: "Exit input / clear the active filter".to_string(),
        },
        Shortcut {
            category: "Column Config",
            key: s("j / k / ↓ / ↑"),
            action: "Navigate columns & group-by options".to_string(),
        },
        Shortcut {
            category: "Column Config",
            key: s("Space"),
            action: "Toggle column visibility checkbox".to_string(),
        },
        Shortcut {
            category: "Column Config",
            key: s("Enter"),
            action: "Open value filter selector for column".to_string(),
        },
        Shortcut {
            category: "Column Config",
            key: s("Esc"),
            action: "Close columns config popup".to_string(),
        },
        Shortcut {
            category: "Date Picker",
            key: s("h / l / ← / →"),
            action: "Previous / Next month".to_string(),
        },
        Shortcut {
            category: "Date Picker",
            key: s("j / k / ↓ / ↑"),
            action: "Previous / Next day".to_string(),
        },
        Shortcut {
            category: "Date Picker",
            key: s("Enter"),
            action: "Confirm date selection".to_string(),
        },
        Shortcut {
            category: "Date Picker",
            key: s("Esc"),
            action: "Cancel date selection".to_string(),
        },
    ]);

    // The few user-defined commands go first, above the long built-in lists.
    shortcuts.splice(
        0..0,
        custom_commands.iter().map(|command| Shortcut {
            category: CUSTOM_COMMANDS_CATEGORY,
            key: d(command.key.clone()),
            action: command.label().to_string(),
        }),
    );

    let active_categories: &[&str] = if app.submit_dialog().is_some() {
        &["Confirm Dialog"]
    } else if app.diff_view().is_some() {
        &["Diff View", CUSTOM_COMMANDS_CATEGORY]
    } else if app.edit_menu().is_some() {
        &["Global & Nav", "Inspector / Editor"]
    } else if app.focus_column_checklist() {
        &["Global & Nav", "Column Config"]
    } else if app.date_picker().is_some() {
        &["Global & Nav", "Date Picker"]
    } else if app.selector().is_some() {
        &["Global & Nav", "Selector / Filter"]
    } else {
        match app.active_tab {
            Tab::Issues => &["Global & Nav", "Issues", CUSTOM_COMMANDS_CATEGORY],
            Tab::MergeRequests => {
                if is_github {
                    &["Global & Nav", "Pull Requests", CUSTOM_COMMANDS_CATEGORY]
                } else {
                    &["Global & Nav", "Merge Requests", CUSTOM_COMMANDS_CATEGORY]
                }
            }
            Tab::Pipelines => &["Global & Nav", "Pipelines", CUSTOM_COMMANDS_CATEGORY],
            Tab::Jobs => &["Global & Nav", "Jobs", CUSTOM_COMMANDS_CATEGORY],
            Tab::Milestones => &["Global & Nav", "Milestones", CUSTOM_COMMANDS_CATEGORY],
            Tab::Runners => &["Global & Nav", "Runners", CUSTOM_COMMANDS_CATEGORY],
            Tab::Releases => &["Global & Nav", "Releases", CUSTOM_COMMANDS_CATEGORY],
            Tab::Todos => &["Global & Nav", "TODOs", CUSTOM_COMMANDS_CATEGORY],
            Tab::Branches => &["Global & Nav", "Branches", CUSTOM_COMMANDS_CATEGORY],
            Tab::Environments => &["Global & Nav", "Environments", CUSTOM_COMMANDS_CATEGORY],
            Tab::Terminal => &["Global & Nav", "Terminal", CUSTOM_COMMANDS_CATEGORY],
        }
    };

    // The help key itself stays reachable from every context, even one
    // (Diff View, Confirm Dialog) whose active categories do not include
    // "Global & Nav".
    let help_label = Action::GlobalHelp.spec().label;
    let mut filtered_shortcuts: Vec<&Shortcut> = shortcuts
        .iter()
        .filter(|s| {
            (active_categories.contains(&s.category) || s.action == help_label)
                && !s.key.trim().is_empty()
        })
        .collect();
    // The registry lists the global keys first, but the keys of the open
    // view are what the modal is opened for.
    filtered_shortcuts.sort_by_key(|s| match s.category {
        CUSTOM_COMMANDS_CATEGORY => 0,
        "Global & Nav" => 2,
        _ => 1,
    });

    let query = app.help_search_query.to_lowercase();
    let is_grouped = query.is_empty();
    let listed: Vec<&Shortcut> = if is_grouped {
        filtered_shortcuts
    } else {
        shortcuts
            .iter()
            .filter(|s| {
                s.category.to_lowercase().contains(&query)
                    || s.key.to_lowercase().contains(&query)
                    || s.action.to_lowercase().contains(&query)
            })
            .collect()
    };
    let shortcut_count = listed.len();
    app.help_selected = app.help_selected.min(shortcut_count.saturating_sub(1));
    let position = if shortcut_count == 0 {
        "0/0 · type to filter · Esc close".to_string()
    } else {
        format!(
            "{}/{shortcut_count} · ↑↓ PgUp/PgDn · type to filter · Esc close",
            app.help_selected + 1
        )
    };

    let title = format!("{} Keyboard Shortcuts", icons.label_keyboard);
    let mut form = crate::form::Form::panel(vec![
        crate::form::Group::region(crate::form::Rows::Fixed(HELP_SEARCH_ROWS)),
        crate::form::Group::region(crate::form::Rows::Fill),
    ]);
    let frame = FormFrame {
        title: &title,
        size: FormSize {
            width: Extent::Percent {
                percent: 90,
                min: 95,
            },
            height: Extent::Percent {
                percent: 85,
                min: 38,
            },
        },
        hints: Some(&position),
        notice: None,
    };
    render_form(f, ctx, size, &frame, &mut form);
    let Some(layout) = form.layout else {
        return;
    };
    let (search_area, list_area) = (layout.groups[0], layout.groups[1]);
    app.overlay_stack
        .push((crate::app::OverlayKind::Help, layout.area));

    // The scrollbar is drawn on the region's last column, so the table stops
    // one column short of it and a full Action line keeps its last character.
    let table_area = Rect {
        width: list_area.width.saturating_sub(1),
        ..list_area
    };
    // Action column width: the table width minus the two fixed columns
    // (20 + 24) and the inter-column spacing (2 gaps of 2).
    let action_width = table_area.width.saturating_sub(20 + 24 + 4).max(8);

    let border_color = if app.help_search_query.is_empty() {
        ctx.theme.border
    } else {
        ctx.theme.border_focused
    };
    let search_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(" Filter Shortcuts ")
        .title_style(
            Style::default()
                .fg(ctx.theme.text_muted)
                .add_modifier(Modifier::BOLD),
        );

    let search_text = if app.help_search_query.is_empty() {
        format!("Type to search commands...{}", icons.input_cursor)
    } else {
        format!("{}{}", app.help_search_query, icons.input_cursor)
    };

    let search_style = if app.help_search_query.is_empty() {
        Style::default()
            .fg(ctx.theme.text_muted)
            .add_modifier(Modifier::ITALIC)
    } else {
        Style::default().fg(ctx.theme.text_normal)
    };

    f.render_widget(
        Paragraph::new(search_text)
            .style(search_style)
            .block(search_block)
            .wrap(ratatui::widgets::Wrap { trim: true }),
        search_area,
    );

    let theme = &ctx.theme;
    let mut rows: Vec<Row> = Vec::new();
    // Table row of each listed shortcut; spacer rows between categories are
    // never selectable.
    let mut shortcut_rows: Vec<usize> = Vec::with_capacity(listed.len());
    let mut last_category = "";
    for s in &listed {
        let starts_category = s.category != last_category;
        if is_grouped && starts_category && !last_category.is_empty() {
            rows.push(Row::new(vec![
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
            ]));
        }
        let category = if !is_grouped || starts_category {
            s.category
        } else {
            ""
        };
        let (action_text, action_lines) = wrap_cell_text(&s.action, action_width);
        shortcut_rows.push(rows.len());
        rows.push(
            Row::new(vec![
                Cell::from(Span::styled(
                    category,
                    Style::default()
                        .fg(theme.purple)
                        .add_modifier(Modifier::BOLD),
                )),
                Cell::from(Span::styled(
                    s.key.clone(),
                    Style::default()
                        .fg(theme.text_normal)
                        .add_modifier(Modifier::BOLD),
                )),
                Cell::from(action_text.patch_style(Style::default().fg(theme.text_normal))),
            ])
            .height(action_lines),
        );
        last_category = s.category;
    }
    app.help_table_state
        .select(shortcut_rows.get(app.help_selected).copied());

    let widths = [
        Constraint::Length(20),
        Constraint::Length(24),
        Constraint::Min(0),
    ];

    let header_style = Style::default()
        .fg(theme.header_fg)
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
        .row_highlight_style(Style::default().bg(theme.highlight_bg))
        .column_spacing(2);
    f.render_stateful_widget(table, table_area, &mut app.help_table_state);
    let mut scrollbar_state =
        ratatui::widgets::ScrollbarState::new(shortcut_count).position(app.help_selected);
    f.render_stateful_widget(
        ratatui::widgets::Scrollbar::new(ratatui::widgets::ScrollbarOrientation::VerticalRight),
        Rect {
            y: list_area.y.saturating_add(1),
            height: list_area.height.saturating_sub(1),
            ..list_area
        },
        &mut scrollbar_state,
    );
    app.help_layout = Some(layout);
}

/// The help overlay's filter box, a bordered single line.
const HELP_SEARCH_ROWS: u16 = 3;
#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::create_backend;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect()
    }

    #[test]
    fn render_help_labels_merge_requests_on_gitlab() {
        let backend = TestBackend::new(160, 80);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.open_help();
        app.active_tab = Tab::MergeRequests;

        let icons = std::sync::Arc::clone(&app.icons);
        let ctx = &crate::ui::render_ctx::RenderCtx::new(&app, &icons);
        terminal
            .draw(|f| {
                render_help(f, ctx, &mut app, f.area());
            })
            .unwrap();

        let text = buffer_text(&terminal);
        assert!(
            text.contains("Merge Requests"),
            "expected MR label on gitlab, got: {text:?}",
        );
        assert!(
            !text.contains("Pull Requests"),
            "PR label leaked into gitlab help: {text:?}",
        );
        assert!(text.contains("Approve selected MR"));
        assert!(text.contains("Open selected MR in browser"));
        assert!(text.contains("View related Pipelines for selected MR"));
        assert!(text.contains("Merge selected MR (configure squash/delete)"));
        assert!(!text.contains("stacked PR"), "{text:?}");
        assert!(text.contains("Delete selected MR"));
    }

    #[test]
    fn render_help_labels_pull_requests_on_github() {
        let backend = TestBackend::new(160, 80);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.gitlab_client = Some(crate::domain::client::GitlabClient {
            is_github: true,
            backend: create_backend(true),
            tx: None,
            page_size: 50,
            api_per_page: 50,
            show_read_notifications: false,
            opened_pipeline: None,
        });
        app.open_help();
        app.active_tab = Tab::MergeRequests;

        let icons = std::sync::Arc::clone(&app.icons);
        let ctx = &crate::ui::render_ctx::RenderCtx::new(&app, &icons);
        terminal
            .draw(|f| {
                render_help(f, ctx, &mut app, f.area());
            })
            .unwrap();

        let text = buffer_text(&terminal);
        assert!(
            text.contains("Pull Requests"),
            "expected PR label on github, got: {text:?}",
        );
        assert!(
            !text.contains("Merge Requests"),
            "MR label leaked into github help: {text:?}",
        );
        assert!(text.contains("Approve selected PR"));
        assert!(text.contains("Open selected PR in browser"));
        assert!(text.contains("View related Actions for selected PR"));
        assert!(
            text.contains("Merge selected PR (a stacked PR merges with the open PRs below it)")
        );
        assert!(text.contains("Create new Pull Request"));
        assert!(!text.contains("Delete selected PR"), "{text:?}");
    }

    fn render_jobs_help(is_github: bool) -> String {
        let mut terminal = Terminal::new(TestBackend::new(160, 80)).unwrap();
        let mut app = App::default();
        app.gitlab_client = Some(crate::domain::client::GitlabClient {
            is_github,
            backend: create_backend(is_github),
            tx: None,
            page_size: 50,
            api_per_page: 50,
            show_read_notifications: false,
            opened_pipeline: None,
        });
        app.open_help();
        app.active_tab = Tab::Jobs;

        let icons = std::sync::Arc::clone(&app.icons);
        let ctx = &crate::ui::render_ctx::RenderCtx::new(&app, &icons);
        terminal
            .draw(|f| render_help(f, ctx, &mut app, f.area()))
            .unwrap();
        buffer_text(&terminal)
    }

    #[test]
    fn render_help_offers_trace_follow_on_gitlab() {
        let text = render_jobs_help(false);
        assert!(text.contains("Toggle trace auto-follow mode"), "{text:?}");
    }

    #[test]
    fn render_help_does_not_offer_trace_follow_on_github() {
        let text = render_jobs_help(true);
        assert!(text.contains("Toggle trace word wrap"), "{text:?}");
        assert!(
            !text.contains("Toggle trace auto-follow mode"),
            "trace follow is not offered on GitHub: {text:?}"
        );
    }

    #[test]
    fn render_help_lists_custom_commands_only_on_their_tab() {
        let backend = TestBackend::new(160, 80);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.config.custom_keybindings = toml::from_str(
            r#"
[[mrs]]
key = "w"
name = "worktree + devbox shell"
command = "true"
"#,
        )
        .unwrap();
        app.custom_commands = crate::custom_commands::CustomCommands::load(&app.config).0;
        app.open_help();

        app.active_tab = Tab::MergeRequests;
        let icons = std::sync::Arc::clone(&app.icons);
        let ctx = &crate::ui::render_ctx::RenderCtx::new(&app, &icons);
        terminal
            .draw(|f| render_help(f, ctx, &mut app, f.area()))
            .unwrap();
        let on_mrs = buffer_text(&terminal);
        assert!(on_mrs.contains("Custom Commands"), "{on_mrs:?}");
        assert!(on_mrs.contains("worktree + devbox shell"), "{on_mrs:?}");

        app.active_tab = Tab::Issues;
        terminal
            .draw(|f| render_help(f, ctx, &mut app, f.area()))
            .unwrap();
        let on_issues = buffer_text(&terminal);
        assert!(
            !on_issues.contains("worktree + devbox shell"),
            "{on_issues:?}"
        );
    }

    /// The scrollbar sits right of the table, so an action line that fills
    /// the whole help region keeps its last character.
    #[test]
    fn the_help_scrollbar_hides_no_action_character() {
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        let mut app = App::default();
        app.open_help();
        app.active_tab = Tab::MergeRequests;
        let icons = std::sync::Arc::clone(&app.icons);
        let ctx = &crate::ui::render_ctx::RenderCtx::new(&app, &icons);
        terminal
            .draw(|f| render_help(f, ctx, &mut app, f.area()))
            .unwrap();
        let list = app.help_layout.as_ref().unwrap().groups[1];
        // A line exactly as wide as the Action column would be if the table
        // took the whole region, ending in a marker.
        let region_action_columns = usize::from(list.width) - (20 + 24 + 4);
        let name = format!("{} §", "x".repeat(region_action_columns - 2));
        app.config.custom_keybindings = toml::from_str(&format!(
            "[[mrs]]\nkey = \"w\"\nname = \"{name}\"\ncommand = \"true\"\n"
        ))
        .unwrap();
        app.custom_commands = crate::custom_commands::CustomCommands::load(&app.config).0;
        terminal
            .draw(|f| render_help(f, ctx, &mut app, f.area()))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let marker_drawn = (list.y..list.bottom())
            .any(|y| (list.x..list.right()).any(|x| buffer[(x, y)].symbol() == "§"));
        assert!(marker_drawn, "{:?}", buffer_text(&terminal));
    }

    #[test]
    fn help_list_scrolls_to_entries_below_the_visible_area() {
        let backend = TestBackend::new(160, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.open_help();
        app.active_tab = Tab::Issues;

        let icons = std::sync::Arc::clone(&app.icons);
        let ctx = &crate::ui::render_ctx::RenderCtx::new(&app, &icons);
        terminal
            .draw(|f| render_help(f, ctx, &mut app, f.area()))
            .unwrap();
        let top = buffer_text(&terminal);
        assert!(top.contains("Create new Issue"), "{top:?}");
        let listed = top
            .split(" 1/")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|count| count.parse::<usize>().ok())
            .expect("position counter 1/N");

        app.help_selected = usize::MAX;
        terminal
            .draw(|f| render_help(f, ctx, &mut app, f.area()))
            .unwrap();
        let bottom = buffer_text(&terminal);
        assert_eq!(
            app.help_selected,
            listed - 1,
            "End clamps to the last entry"
        );
        assert!(
            bottom.contains(&format!(" {listed}/{listed} ")),
            "{bottom:?}"
        );
        assert!(
            !bottom.contains("Create new Issue"),
            "the list scrolled past the first entries"
        );
    }

    #[test]
    fn render_help_filters_out_empty_keybindings() {
        let backend = TestBackend::new(160, 80);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.open_help();
        app.active_tab = Tab::Issues;

        let icons = std::sync::Arc::clone(&app.icons);
        let ctx = &crate::ui::render_ctx::RenderCtx::new(&app, &icons);
        terminal
            .draw(|f| {
                render_help(f, ctx, &mut app, f.area());
            })
            .unwrap();

        let text = buffer_text(&terminal);
        assert!(
            !text.contains("Save view layout to config"),
            "save_view should not be listed in help: {text:?}",
        );

        // If a keybinding is set to no keys, it is filtered out from help
        app.config.keybindings.set(Action::IssuesCreateIssue, &[]);
        terminal
            .draw(|f| {
                render_help(f, ctx, &mut app, f.area());
            })
            .unwrap();

        let text_empty = buffer_text(&terminal);
        assert!(
            !text_empty.contains("Create new Issue"),
            "empty keybinding shortcut should be filtered from help: {text_empty:?}",
        );
    }

    #[test]
    fn render_help_lists_actions_bound_only_to_slash() {
        let backend = TestBackend::new(160, 80);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.open_help();
        app.active_tab = Tab::Jobs;

        let icons = std::sync::Arc::clone(&app.icons);
        let ctx = &crate::ui::render_ctx::RenderCtx::new(&app, &icons);
        terminal
            .draw(|f| render_help(f, ctx, &mut app, f.area()))
            .unwrap();

        let text = buffer_text(&terminal);
        assert!(text.contains(Action::GlobalSearch.spec().label), "{text:?}");
        assert!(
            text.contains(Action::JobsTraceSearch.spec().label),
            "{text:?}"
        );
    }

    #[test]
    fn render_help_shows_a_remapped_key_instead_of_the_default() {
        let backend = TestBackend::new(160, 80);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();
        app.open_help();
        app.active_tab = Tab::Issues;
        app.config
            .keybindings
            .set(Action::IssuesCreateIssue, &["F9"]);

        let icons = std::sync::Arc::clone(&app.icons);
        let ctx = &crate::ui::render_ctx::RenderCtx::new(&app, &icons);
        terminal
            .draw(|f| render_help(f, ctx, &mut app, f.area()))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row: String = buffer
            .content
            .chunks(buffer.area.width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .find(|row| row.contains("Create new Issue"))
            .expect("Create new Issue row rendered");
        let before_label = row.split("Create new Issue").next().unwrap_or_default();
        let key_cell = before_label
            .trim_end()
            .rsplit("  ")
            .next()
            .unwrap_or_default();
        assert_eq!(key_cell.trim(), "F9", "{row:?}");
    }

    /// The merge dialog for the fixture MR.
    fn render_merge_dialog(width: u16, height: u16) -> Terminal<TestBackend> {
        render_dialog(crate::app::ConfirmAction::MergeMr(2), width, height)
    }

    /// A bulk merge of twenty MRs, too tall for 24 rows even without blank
    /// rows, so the form shortens the list.
    fn render_bulk_merge_dialog(width: u16, height: u16) -> Terminal<TestBackend> {
        let items = (1..=20).map(|iid| (String::new(), iid)).collect();
        render_dialog(
            crate::app::ConfirmAction::BulkMergeMrs(items),
            width,
            height,
        )
    }

    /// Drawn with the ASCII icon set, the one the snapshots are written in,
    /// for a project allowing merge commits and squash.
    fn render_dialog(
        action: crate::app::ConfirmAction,
        width: u16,
        height: u16,
    ) -> Terminal<TestBackend> {
        use crate::domain::mr::{MergeMethod, MergeMethods};
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut app = App::default();
        app.mrs.items =
            serde_json::from_str(include_str!("../../tests/fixtures/mrs.json")).unwrap();
        app.merge_methods.insert(
            app.scope.as_str().to_string(),
            MergeMethods::new(&[MergeMethod::Merge, MergeMethod::Squash], None).unwrap(),
        );
        app.open_submit_dialog(crate::app::SubmitDialog::build(action, &app));
        let ascii = crate::config::Icons::ascii();
        let ctx = &crate::ui::render_ctx::RenderCtx {
            theme: crate::config::Theme::default(),
            icons: &ascii,
            kind: crate::backend::BackendKind::GitLab,
        };
        terminal
            .draw(|f| render_overlays(f, ctx, &mut app, f.area()))
            .unwrap();
        terminal
    }

    /// One line per buffer row, trailing blanks trimmed.
    fn snapshot_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for row in buffer.content.chunks(buffer.area.width as usize) {
            let line: String = row.iter().map(|cell| cell.symbol()).collect();
            text.push_str(line.trim_end());
            text.push('\n');
        }
        text
    }

    /// Compares against `src/ui/snapshots/<name>.txt`. Set
    /// `GLAB_TUI_UPDATE_SNAPSHOTS=1` to rewrite the file after an intended
    /// layout change.
    fn assert_snapshot(name: &str, actual: &str) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/ui/snapshots")
            .join(format!("{name}.txt"));
        if std::env::var_os("GLAB_TUI_UPDATE_SNAPSHOTS").is_some() {
            std::fs::write(&path, actual).unwrap();
            return;
        }
        let expected = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}\nactual:\n{actual}", path.display()))
            .replace("\r\n", "\n");
        assert_eq!(
            actual, expected,
            "{name} changed; rerun with GLAB_TUI_UPDATE_SNAPSHOTS=1 if intended"
        );
    }

    #[test]
    fn merge_dialog_at_60_columns_matches_snapshot() {
        let terminal = render_merge_dialog(60, 24);
        assert_snapshot("submit_dialog_merge_60x24", &snapshot_text(&terminal));
    }

    #[test]
    fn merge_dialog_at_80_columns_matches_snapshot() {
        let terminal = render_merge_dialog(80, 24);
        assert_snapshot("submit_dialog_merge_80x24", &snapshot_text(&terminal));
    }

    #[test]
    fn merge_dialog_at_120_columns_matches_snapshot() {
        let terminal = render_merge_dialog(120, 24);
        assert_snapshot("submit_dialog_merge_120x24", &snapshot_text(&terminal));
    }

    fn click(app: &mut App, column: u16, row: u16) {
        let mut fx = crate::effect::Effects::default();
        let tx = app.tx.clone();
        crate::handlers::mouse::handle_mouse_event(
            app,
            &crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column,
                row,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            &tx,
            &mut fx,
        );
    }

    /// The column-filter picker and a field picker share one render, and a
    /// click lands on the row drawn under it, also while a notice is shown.
    #[test]
    fn a_click_on_a_selector_row_selects_that_row_whatever_the_purpose() {
        use crate::app::{Selector, SelectorField, SelectorPurpose};
        for (purpose, notice) in [
            (
                SelectorPurpose::ColumnFilter {
                    tab: Tab::Issues,
                    column: "State".to_string(),
                },
                false,
            ),
            (SelectorPurpose::Field(SelectorField::Labels), true),
        ] {
            let mut app = App::default();
            let request = notice.then(|| app.next_request_tag());
            app.open_selector(Selector {
                title: "Pick".to_string(),
                all_items: vec!["alpha".to_string(), "beta".to_string()],
                selected_items: std::collections::HashSet::new(),
                cursor_idx: 0,
                search_query: String::new(),
                is_filtering: false,
                is_loading: false,
                entity_iid: 0,
                entity_type: String::new(),
                purpose: purpose.clone(),
                multi_select: true,
                state: ListState::default(),
                request,
            });
            if let Some(request) = request {
                app.push_notice(crate::notice::Notice::error("offline").from_request(request));
            }
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
            let icons = crate::config::Icons::ascii();
            let ctx = &crate::ui::render_ctx::RenderCtx {
                theme: crate::config::Theme::default(),
                icons: &icons,
                kind: crate::backend::BackendKind::GitLab,
            };
            terminal
                .draw(|f| render_overlays(f, ctx, &mut app, f.area()))
                .unwrap();
            let text = snapshot_text(&terminal);
            let (beta_row, beta_line) = text
                .lines()
                .enumerate()
                .find(|(_, line)| line.contains("beta"))
                .expect("beta is drawn");
            let beta_column = beta_line.find("beta").unwrap();

            click(&mut app, beta_column as u16, beta_row as u16);

            let selector = app.selector().unwrap();
            assert_eq!(selector.cursor_idx, 1, "{purpose:?}");
            assert_eq!(
                selector.selected_items,
                std::collections::HashSet::from(["beta".to_string()]),
                "{purpose:?}"
            );
        }
    }

    /// Selector rows carry third-party text (comment bodies, titles): an
    /// escape sequence in one never reaches the terminal, highlighted or not.
    #[test]
    fn a_selector_row_draws_no_escape_sequence() {
        use crate::app::{Selector, SelectorField, SelectorPurpose};
        for query in ["", "evil"] {
            let mut app = App::default();
            app.open_selector(Selector {
                title: "Pick".to_string(),
                all_items: vec!["evil\x1b]0;pwned\x07 body".to_string()],
                selected_items: std::collections::HashSet::new(),
                cursor_idx: 0,
                search_query: query.to_string(),
                is_filtering: false,
                is_loading: false,
                entity_iid: 0,
                entity_type: String::new(),
                purpose: SelectorPurpose::Field(SelectorField::Labels),
                multi_select: true,
                state: ListState::default(),
                request: None,
            });
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
            let icons = crate::config::Icons::ascii();
            let ctx = &crate::ui::render_ctx::RenderCtx {
                theme: crate::config::Theme::default(),
                icons: &icons,
                kind: crate::backend::BackendKind::GitLab,
            };
            terminal
                .draw(|f| render_overlays(f, ctx, &mut app, f.area()))
                .unwrap();
            let text = snapshot_text(&terminal);
            assert!(text.contains("evil body"), "query {query:?}:\n{text}");
            assert!(!text.contains("pwned"), "query {query:?}:\n{text}");
            assert!(!text.contains('\x1b'), "query {query:?}:\n{text}");
        }
    }

    /// The footer names the keys that work, also after a remap.
    #[test]
    fn the_selector_footer_names_the_remapped_keys() {
        use crate::app::{Selector, SelectorField, SelectorPurpose};
        let selector = Selector {
            title: "Pick".to_string(),
            all_items: vec!["alpha".to_string()],
            selected_items: std::collections::HashSet::new(),
            cursor_idx: 0,
            search_query: String::new(),
            is_filtering: false,
            is_loading: false,
            entity_iid: 0,
            entity_type: String::new(),
            purpose: SelectorPurpose::Field(SelectorField::Labels),
            multi_select: true,
            state: ListState::default(),
            request: None,
        };
        let mut keys = App::default().config.keybindings;
        assert_eq!(
            selector_hints(&selector, &keys),
            "Enter confirm · Esc close · j/k move · Space select · f filter"
        );

        keys.set(Action::SelectorFilter, &["s"]);
        keys.set(Action::SelectorClose, &["q"]);
        keys.set(Action::SelectorToggle, &[]);
        assert_eq!(
            selector_hints(&selector, &keys),
            "Enter confirm · q close · j/k move · s filter"
        );
    }

    /// The labels picker's footer is wider than the selector at 80 columns,
    /// and what it loses must not be the way out.
    #[test]
    fn the_labels_picker_footer_keeps_its_close_key_at_80_columns() {
        use crate::app::{Selector, SelectorField, SelectorPurpose};
        let mut app = App::default();
        app.open_selector(Selector {
            title: "Labels".to_string(),
            all_items: vec!["bug".to_string()],
            selected_items: std::collections::HashSet::new(),
            cursor_idx: 0,
            search_query: String::new(),
            is_filtering: false,
            is_loading: false,
            entity_iid: 0,
            entity_type: String::new(),
            purpose: SelectorPurpose::Field(SelectorField::Labels),
            multi_select: true,
            state: ListState::default(),
            request: None,
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let ascii = crate::config::Icons::ascii();
        let ctx = &crate::ui::render_ctx::RenderCtx {
            theme: crate::config::Theme::default(),
            icons: &ascii,
            kind: crate::backend::BackendKind::GitLab,
        };
        terminal
            .draw(|f| render_overlays(f, ctx, &mut app, f.area()))
            .unwrap();
        let text = snapshot_text(&terminal);
        assert!(
            text.lines().any(|line| line.contains("Esc close")),
            "close key clipped:\n{text}"
        );
    }

    /// While the save menu or the page size field owns the keys, a click on
    /// a Configure row outside the menu neither saves nor commits.
    #[test]
    fn a_click_on_a_configure_row_is_ignored_while_a_sub_state_owns_the_keys() {
        for open_save_menu in [true, false] {
            let mut app = App::default();
            app.open_configure();
            if let Some(c) = app.configure_mut() {
                c.save_menu_open = open_save_menu;
                c.save_menu_selection = open_save_menu.then_some(crate::app::SaveMenu::Cancel);
                c.editing_page_size = !open_save_menu;
            }
            let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
            let icons = crate::config::Icons::ascii();
            let ctx = &crate::ui::render_ctx::RenderCtx {
                theme: crate::config::Theme::default(),
                icons: &icons,
                kind: crate::backend::BackendKind::GitLab,
            };
            terminal
                .draw(|f| render_overlays(f, ctx, &mut app, f.area()))
                .unwrap();
            let configure = app.configure().unwrap();
            let list = configure.layout.as_ref().unwrap().groups[0];
            let before = configure.column_checklist_idx;
            let enter_row = configure
                .visible_rows
                .iter()
                .rposition(Option::is_some)
                .expect("a selectable row is drawn");

            click(&mut app, list.x + 1, list.y + enter_row as u16);

            let c = app.configure().expect("Configure stays open");
            assert_eq!(
                (
                    c.save_menu_open,
                    c.editing_page_size,
                    c.column_checklist_idx
                ),
                (open_save_menu, !open_save_menu, before)
            );
        }
    }

    /// A click on a Configure setting acts on the setting drawn under it.
    #[test]
    fn a_click_on_a_configure_setting_acts_on_the_setting_drawn_there() {
        let mut app = App::default();
        app.open_configure();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        let icons = crate::config::Icons::ascii();
        let ctx = &crate::ui::render_ctx::RenderCtx {
            theme: crate::config::Theme::default(),
            icons: &icons,
            kind: crate::backend::BackendKind::GitLab,
        };
        terminal
            .draw(|f| render_overlays(f, ctx, &mut app, f.area()))
            .unwrap();
        let text = snapshot_text(&terminal);
        let (row, column) = text
            .lines()
            .enumerate()
            .find_map(|(row, line)| {
                let byte = line.find("Theme")?;
                Some((row as u16, line[..byte].chars().count() as u16))
            })
            .expect("the Theme setting is drawn");

        click(&mut app, column, row);

        assert_eq!(
            app.selector().map(|selector| &selector.purpose),
            Some(&crate::app::SelectorPurpose::Theme),
            "{text}"
        );
    }

    /// The footer is drawn unwrapped on one row, and it is the visible way
    /// out of the overlay, so a fixed-width overlay's footer fits inside it.
    #[test]
    fn every_fixed_width_footer_fits_inside_its_frame() {
        for (hints, width) in [
            (CONFIGURE_HINTS, CONFIGURE_WIDTH),
            (CONFIGURE_PAGE_SIZE_HINTS, CONFIGURE_WIDTH),
            (SAVE_MENU_HINTS, SAVE_MENU_WIDTH),
            (DATE_PICKER_HINTS, DATE_PICKER_WIDTH),
        ] {
            let columns = usize::from(crate::ui::form::text_columns(width));
            assert!(
                Span::raw(hints).width() <= columns,
                "{hints:?} is wider than {columns} columns"
            );
        }
    }

    #[test]
    fn the_text_input_footer_fits_its_percent_width_at_80_columns() {
        let mut app = App::default();
        app.open_text_input(crate::app::TextInput::new(
            "Title",
            String::new(),
            crate::app::TextInputAction::EditField {
                entity_iid: 1,
                entity_type: "issue".to_string(),
                field_type: "title".to_string(),
            },
        ));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let ascii = crate::config::Icons::ascii();
        let ctx = &crate::ui::render_ctx::RenderCtx {
            theme: crate::config::Theme::default(),
            icons: &ascii,
            kind: crate::backend::BackendKind::GitLab,
        };
        terminal
            .draw(|f| render_overlays(f, ctx, &mut app, f.area()))
            .unwrap();
        let text = snapshot_text(&terminal);
        assert!(
            text.lines().any(|line| line.contains(TEXT_INPUT_HINTS)),
            "footer clipped:\n{text}"
        );
    }

    /// A click on a day drawn in the grid picks that day, and a click on the
    /// weekday header above it picks nothing.
    #[test]
    fn a_click_on_a_date_picker_day_picks_the_day_drawn_there() {
        let mut app = App::default();
        app.open_date_picker(crate::app::DatePicker::new(
            "Pick".to_string(),
            "2026-08-23",
            crate::app::DatePickerAction::EditNewField { field_idx: 0 },
        ));
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let ascii = crate::config::Icons::ascii();
        let ctx = &crate::ui::render_ctx::RenderCtx {
            theme: crate::config::Theme::default(),
            icons: &ascii,
            kind: crate::backend::BackendKind::GitLab,
        };
        terminal
            .draw(|f| render_overlays(f, ctx, &mut app, f.area()))
            .unwrap();
        let text = snapshot_text(&terminal);
        let (row, column) = text
            .lines()
            .enumerate()
            .find_map(|(row, line)| {
                let byte = line.find(" 13 ")? + 1;
                Some((row as u16, line[..byte].chars().count() as u16))
            })
            .expect("13 is drawn");

        click(&mut app, column, row);
        assert_eq!(app.date_picker().unwrap().day, 13);

        let lines: Vec<&str> = text.lines().collect();
        let header_row = lines
            .iter()
            .position(|line| line.contains("Su"))
            .expect("the weekday header is drawn");
        let first_week = lines[header_row + 1];
        let first_day_column = first_week
            .chars()
            .position(|c| c == '1')
            .expect("August 2026 starts on the last weekday of the first week");
        click(&mut app, first_day_column as u16, header_row as u16);
        assert_eq!(app.date_picker().unwrap().day, 13);
    }

    /// A click on the arrows drawn beside the month steps the month.
    #[test]
    fn a_click_on_a_date_picker_arrow_steps_the_month() {
        let mut app = App::default();
        app.open_date_picker(crate::app::DatePicker::new(
            "Pick".to_string(),
            "2026-08-23",
            crate::app::DatePickerAction::EditNewField { field_idx: 0 },
        ));
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let ascii = crate::config::Icons::ascii();
        let ctx = &crate::ui::render_ctx::RenderCtx {
            theme: crate::config::Theme::default(),
            icons: &ascii,
            kind: crate::backend::BackendKind::GitLab,
        };
        for (month_line, arrow, month) in [
            ("August 2026", &ascii.nav_prev, 7),
            ("July 2026", &ascii.nav_next, 8),
            ("August 2026", &ascii.nav_next, 9),
        ] {
            terminal
                .draw(|f| render_overlays(f, ctx, &mut app, f.area()))
                .unwrap();
            let text = snapshot_text(&terminal);
            let (row, column) = text
                .lines()
                .enumerate()
                .find(|(_, line)| line.contains(month_line))
                .and_then(|(row, line)| {
                    let byte = line.find(arrow.as_str())?;
                    Some((row as u16, line[..byte].chars().count() as u16))
                })
                .unwrap_or_else(|| panic!("{arrow} beside {month_line} is drawn:\n{text}"));

            click(&mut app, column, row);

            assert_eq!(app.date_picker().unwrap().month, month, "{text}");
        }
    }

    #[test]
    fn bulk_merge_form_at_60_columns_matches_snapshot() {
        let terminal = render_bulk_merge_dialog(60, 24);
        assert_snapshot("form_bulk_merge_60x24", &snapshot_text(&terminal));
    }

    #[test]
    fn bulk_merge_form_at_80_columns_matches_snapshot() {
        let terminal = render_bulk_merge_dialog(80, 24);
        assert_snapshot("form_bulk_merge_80x24", &snapshot_text(&terminal));
    }

    #[test]
    fn a_long_bulk_branch_delete_lists_the_branches_that_fit_and_counts_the_rest() {
        const BRANCHES: usize = 60;
        let names = (1..=BRANCHES).map(|n| format!("branch-{n}")).collect();
        let terminal = render_dialog(crate::app::ConfirmAction::BulkDeleteBranches(names), 80, 24);
        let text = snapshot_text(&terminal);
        let rows: Vec<&str> = text
            .lines()
            .map(|line| line.trim_matches(|c: char| c == '│' || c.is_whitespace()))
            .collect();
        assert!(rows.contains(&"Are you sure you want to delete 60 selected branches?"));
        let shown: Vec<&str> = rows
            .iter()
            .copied()
            .filter(|row| row.starts_with("branch-"))
            .collect();
        let expected: Vec<String> = (1..=shown.len()).map(|n| format!("branch-{n}")).collect();
        assert_eq!(shown, expected);
        assert!(shown.len() >= 3, "only {} branches shown", shown.len());
        let more = format!("v {} more", BRANCHES - shown.len());
        assert!(rows.contains(&more.as_str()), "no `{more}` row in\n{text}");
    }
}
