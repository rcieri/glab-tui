use super::form::{Extent, FormFrame, FormSize, render_form};
use super::helpers::highlight_fuzzy_match;
use crate::app::SaveMenu;
use crate::app::{App, Tab};
use crate::ui::render_ctx::RenderCtx;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table},
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
        let mut form: crate::form::Form<()> = crate::form::Form::new(
            vec![crate::form::Group::Custom(crate::form::Rows::Fixed(
                list_rows,
            ))],
            Vec::new(),
        );
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
const CONFIGURE_HINTS: &str =
    "j/k move · J/K section · Space toggle · Enter filter / pick · Esc close";
const CONFIGURE_PAGE_SIZE_HINTS: &str = "Type digits · Enter save · Esc cancel";
const SAVE_MENU_WIDTH: u16 = 32;
const SAVE_MENU_HINTS: &str = "j/k move · Enter pick · Esc back";
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
    let mut form: crate::form::Form<()> = crate::form::Form::new(
        vec![crate::form::Group::Custom(crate::form::Rows::Fixed(
            SAVE_MENU_CHOICES.len() as u16,
        ))],
        Vec::new(),
    );
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

const TEXT_INPUT_HINTS: &str = "Type to edit · Enter submit · Ctrl+E $EDITOR · Esc cancel";
const DATE_PICKER_HINTS: &str = "h/l month · j/k day · Enter pick · Esc cancel";
/// Month line, weekday header and six weeks.
const CALENDAR_ROWS: u16 = 8;
/// Seven 4-column day cells with one column between them, plus the frame.
const DATE_PICKER_WIDTH: u16 = 7 * 4 + 6 + 4;

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
    use chrono::Datelike;
    let Some(date_picker) = app.date_picker_mut() else {
        return;
    };
    let mut form: crate::form::Form<()> = crate::form::Form::new(
        vec![crate::form::Group::Custom(crate::form::Rows::Fixed(
            CALENDAR_ROWS,
        ))],
        Vec::new(),
    );
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

    let [month_area, grid_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(calendar);
    f.render_widget(
        Paragraph::new(format!(
            "{}  {} {}  {}",
            ctx.icons.nav_prev,
            month_name(date_picker.month),
            date_picker.year,
            ctx.icons.nav_next
        ))
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(ctx.theme.header_fg)
                .add_modifier(Modifier::BOLD),
        ),
        month_area,
    );

    let header = Row::new(
        ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"]
            .into_iter()
            .map(|day| Cell::from(Line::from(day).alignment(Alignment::Center))),
    )
    .style(Style::default().fg(ctx.theme.text_muted));
    let start_weekday = chrono::NaiveDate::from_ymd_opt(date_picker.year, date_picker.month, 1)
        .map_or(0, |first| first.weekday().num_days_from_sunday());
    let total_days = crate::app::days_in_month(date_picker.year, date_picker.month);
    let rows = (0..6u32).map(|week| {
        Row::new((0..7u32).map(|weekday| {
            let day = (week * 7 + weekday + 1).checked_sub(start_weekday);
            match day.filter(|&day| (1..=total_days).contains(&day)) {
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
            }
        }))
    });
    f.render_widget(
        Table::new(rows, [Constraint::Length(4); 7])
            .header(header)
            .column_spacing(1),
        grid_area,
    );
    date_picker.layout = Some(layout);
    app.overlay_stack
        .push((crate::app::OverlayKind::DatePicker, area));
}

fn submit_dialog_icon(icons: &crate::config::Icons, action: &crate::app::ConfirmAction) -> String {
    use crate::app::ConfirmAction;
    match action {
        ConfirmAction::DeleteMilestone(_)
        | ConfirmAction::DeleteRelease(_)
        | ConfirmAction::DeleteBranch(_)
        | ConfirmAction::BulkDeleteBranches(_)
        | ConfirmAction::DeleteIssue(_)
        | ConfirmAction::DeleteMr(_) => icons.action_delete.clone(),
        ConfirmAction::CloseIssue(_)
        | ConfirmAction::CloseMr(_)
        | ConfirmAction::CloseMilestone(_)
        | ConfirmAction::StopEnvironment(_) => icons.action_close.clone(),
        ConfirmAction::ReopenIssue(_)
        | ConfirmAction::ReopenMr(_)
        | ConfirmAction::ReopenMilestone(_) => icons.action_reopen.clone(),
        ConfirmAction::MergeMr(_)
        | ConfirmAction::MergeStack(_)
        | ConfirmAction::BulkMergeMrs(_) => icons.action_merge.clone(),
        ConfirmAction::RevokeMr(_) | ConfirmAction::SubmitReview(_) => icons.action_review.clone(),
        ConfirmAction::RebaseMr(_) => icons.merge_rebase.clone(),
        ConfirmAction::MarkAllTodosRead => icons.tab_todo.clone(),
        ConfirmAction::CancelPipeline(_) | ConfirmAction::CancelJobs { .. } => {
            icons.status_canceled.clone()
        }
        ConfirmAction::RerunCommand(_) => icons.tab_terminal.clone(),
        ConfirmAction::RollbackDeployment { .. } => icons.label_deployment.clone(),
    }
}

/// The bordered filter box above a selector's list.
const SELECTOR_FILTER_ROWS: u16 = 3;

fn selector_hints(selector: &crate::app::Selector) -> &'static str {
    if selector.is_filtering {
        return "Type to filter · Enter / Esc done";
    }
    match (selector.multi_select, selector.purpose.has_filter_bar()) {
        (true, true) => "j/k move · Space select · / filter · Enter confirm · Esc close",
        (true, false) => "j/k move · Space select · Enter confirm · Esc close",
        (false, true) => "j/k move · / filter · Enter pick · Esc close",
        (false, false) => "j/k move · Enter pick · Esc close",
    }
}

/// Every selector, the column-filter picker and the repo switcher included,
/// is drawn here: what differs between them is read from its purpose.
fn render_selector(f: &mut Frame, ctx: &RenderCtx<'_>, app: &mut App, size: Rect) {
    let notice = app.modal_notice().cloned();
    let App {
        modal_stack,
        overlay_stack,
        switch_repo_groups,
        switch_repo_paths,
        selector_layout,
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
        groups.push(crate::form::Group::Custom(crate::form::Rows::Fixed(
            SELECTOR_FILTER_ROWS,
        )));
    }
    groups.push(crate::form::Group::Custom(crate::form::Rows::Fill));
    let mut form: crate::form::Form<()> = crate::form::Form::new(groups, Vec::new());
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
        hints: Some(selector_hints(selector)),
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
                format!("{}▋", selector.search_query),
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
                match indices {
                    Some(indices) => {
                        spans.extend(highlight_fuzzy_match(item, indices, style, match_style))
                    }
                    None => spans.push(Span::styled(item.clone(), style)),
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
        submit_dialog_icon(ctx.icons, &dialog.action),
        dialog.title
    );
    let frame = FormFrame {
        title: &title,
        size: FormSize {
            width: Extent::Fixed(crate::app::SubmitDialog::DIALOG_WIDTH),
            height: Extent::Content,
        },
        hints: None,
        notice: None,
    };
    render_form(f, ctx, size, &frame, &mut dialog.form);
    if let Some(area) = dialog.form.layout.as_ref().map(|layout| layout.area) {
        app.overlay_stack
            .push((crate::app::OverlayKind::ConfirmPopup, area));
    }
}

const CUSTOM_COMMANDS_CATEGORY: &str = "Custom Commands";
const START_MANUAL_JOB_ACTION: &str = "Start manual job";
/// GitHub deployments carry no job, so they have no log to open.
const VIEW_DEPLOYMENT_LOG_ACTION: &str = "Open the selected deployment's job log in $EDITOR";

const RUNNER_PAUSE_RESUME_ACTION: &str = "Pause / Resume runner";
const RUNNER_EDIT_DESCRIPTION_ACTION: &str = "Edit runner description text";

pub(crate) fn render_help(f: &mut Frame, ctx: &RenderCtx<'_>, app: &mut App, size: Rect) {
    if !app.show_help() {
        return;
    }

    let icons = ctx.icons;
    let is_github = ctx.kind.is_github();
    let mr_label: &'static str = if is_github {
        "Pull Requests"
    } else {
        "Merge Requests"
    };

    struct Shortcut<'a> {
        category: &'static str,
        key: std::borrow::Cow<'static, str>,
        action: &'a str,
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

    let mut shortcuts: Vec<Shortcut> = vec![
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
            key: d(app.config.keybindings.global.configure.to_string()),
            action: "Toggle columns config popup (filter / group / sort)",
        },
        Shortcut {
            category: "Global & Nav",
            key: s("j / k / ↓ / ↑ / Home / End"),
            action: "Select item / Scroll page / Jump to top or bottom",
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
            key: d(format!(
                "{} / {}",
                app.config.keybindings.global.scroll_page_down,
                app.config.keybindings.global.scroll_page_up
            )),
            action: "Scroll description pane by one page",
        },
        Shortcut {
            category: "Global & Nav",
            key: d(format!(
                "{} / {}",
                app.config.keybindings.global.scroll_half_page_down,
                app.config.keybindings.global.scroll_half_page_up
            )),
            action: "Scroll description pane (or the list when it is hidden) by half a page",
        },
        Shortcut {
            category: "Global & Nav",
            key: d(app.config.keybindings.global.scroll_to_end.to_string()),
            action: "Jump to the last line of the description pane",
        },
        Shortcut {
            category: "Global & Nav",
            key: d(app.config.keybindings.global.scroll_top.to_string()),
            action: "Jump to the top of the description pane",
        },
        Shortcut {
            category: "Global & Nav",
            key: d(format!("{} / f", app.config.keybindings.global.search)),
            action: "Open fuzzy search / filter bar",
        },
        Shortcut {
            category: "Global & Nav",
            key: s("Ctrl+W / Ctrl+U"),
            action: "Search: delete previous word / clear line",
        },
        Shortcut {
            category: "Global & Nav",
            key: s("Esc"),
            action: "Exit input / clear the active filter",
        },
        Shortcut {
            category: "Global & Nav",
            key: d(app.config.keybindings.global.jump_to_id.to_string()),
            action: "Jump to issue/MR by ID; fetch if not cached",
        },
        Shortcut {
            category: "Global & Nav",
            key: d(format!(
                "{} / {}",
                app.config.keybindings.global.sort_next, app.config.keybindings.global.sort_prev
            )),
            action: "Sort list by next / previous column",
        },
        Shortcut {
            category: "Global & Nav",
            key: d(app.config.keybindings.global.sort_reverse.to_string()),
            action: "Reverse sort order",
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
            key: d(app.config.keybindings.global.submit_edit.clone()),
            action: "Submit active edit form",
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
            key: d(app.config.keybindings.issues.select_all.clone()),
            action: "Select all filtered issues (in select mode)",
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
            action: if is_github {
                "Create Pull Request from selected Issue"
            } else {
                "Create Merge Request from selected Issue"
            },
        },
        Shortcut {
            category: "Issues",
            key: d(app.config.keybindings.issues.jump_related_mrs.clone()),
            action: "Jump to related Merge Requests / Pull Requests",
        },
        // ── Merge Requests / Pull Requests ──
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.create_mr.clone()),
            action: if is_github {
                "Create new Pull Request"
            } else {
                "Create new Merge Request"
            },
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.select_mr.clone()),
            action: "Toggle MR/PR selection (bulk edit/merge with e/m)",
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.selection_toggle.clone()),
            action: "Toggle select mode (paint selection while navigating)",
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.select_all.clone()),
            action: "Select all filtered MRs / PRs (in select mode)",
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.edit_entity.clone()),
            action: "Open parameter edit menu",
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.approve_mr.clone()),
            action: if is_github {
                "Approve selected PR"
            } else {
                "Approve selected MR"
            },
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.revoke_mr.clone()),
            action: "Revoke your approval (GitLab only)",
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.rebase_mr.clone()),
            action: "Rebase source branch onto target",
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.merge_mr.clone()),
            action: if is_github {
                "Merge selected PR (a stacked PR merges with the open PRs below it)"
            } else {
                "Merge selected MR (configure squash/delete)"
            },
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.toggle_draft.clone()),
            action: "Toggle Draft / Ready status",
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.view_diff.clone()),
            action: if is_github {
                "View Pull Request diff changes"
            } else {
                "View Merge Request diff changes"
            },
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.view_related_pipelines.clone()),
            action: if is_github {
                "View related Actions for selected PR"
            } else {
                "View related pipelines for selected MR"
            },
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.view_stack.clone()),
            action: if is_github {
                "Browse the stack of the selected stacked PR"
            } else {
                "Browse the stack of the selected MR (GitHub-only)"
            },
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.close_entity.clone()),
            action: if is_github {
                "Close selected PR"
            } else {
                "Close selected MR"
            },
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.reopen_entity.clone()),
            action: if is_github {
                "Reopen selected PR"
            } else {
                "Reopen selected MR"
            },
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.delete_entity.clone()),
            action: if is_github {
                "Delete selected PR"
            } else {
                "Delete selected MR"
            },
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.copy_reference.clone()),
            action: if is_github {
                "Copy selected PR as Markdown link"
            } else {
                "Copy selected MR as Markdown link"
            },
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.jump_linked_issues.clone()),
            action: "Jump to linked Issues",
        },
        Shortcut {
            category: mr_label,
            key: d(app.config.keybindings.mrs.open_in_browser.clone()),
            action: if is_github {
                "Open selected PR in browser"
            } else {
                "Open selected MR in browser"
            },
        },
        // ── Pipelines ──
        Shortcut {
            category: "Pipelines",
            key: s("Enter"),
            action: "Descend into downstream pipelines, or the pipeline's jobs",
        },
        Shortcut {
            category: "Pipelines",
            key: d(app.config.keybindings.pipelines.enter_jobs.clone()),
            action: "View the selected pipeline's own jobs",
        },
        Shortcut {
            category: "Pipelines",
            key: d(app.config.keybindings.pipelines.toggle_preview.clone()),
            action: "Show / hide the pipeline inspector (commit, failed jobs)",
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
            key: d(app.config.keybindings.pipelines.retry_failed.clone()),
            action: "Retry only the failed jobs of the selected pipeline(s)",
        },
        Shortcut {
            category: "Pipelines",
            key: d(app.config.keybindings.pipelines.cycle_status_filter.clone()),
            action: "Filter by status: All / Running / Failed / Success",
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
            key: d(app.config.keybindings.pipelines.select_pipeline.clone()),
            action: "Check / uncheck pipeline for bulk retry",
        },
        Shortcut {
            category: "Pipelines",
            key: d(app.config.keybindings.pipelines.selection_toggle.clone()),
            action: "Toggle select mode (paint selection while navigating)",
        },
        Shortcut {
            category: "Pipelines",
            key: d(app.config.keybindings.pipelines.select_all.clone()),
            action: "Select all filtered pipelines (in select mode)",
        },
        Shortcut {
            category: "Pipelines",
            key: d(app.config.keybindings.pipelines.open_in_browser.clone()),
            action: "Open pipeline in browser",
        },
        Shortcut {
            category: "Pipelines",
            key: d(app.config.keybindings.pipelines.copy_url.clone()),
            action: "Copy selected pipeline URL",
        },
        Shortcut {
            category: "Pipelines",
            key: d(app.config.keybindings.pipelines.copy_id.clone()),
            action: "Copy selected pipeline ID (#123)",
        },
        Shortcut {
            category: "Pipelines",
            key: d(app.config.keybindings.pipelines.copy_sha.clone()),
            action: "Copy selected pipeline commit SHA",
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
            action: START_MANUAL_JOB_ACTION,
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
            key: d(app.config.keybindings.jobs.selection_toggle.clone()),
            action: "Toggle select mode (paint selection while navigating)",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.select_all.clone()),
            action: "Select all filtered jobs (in select mode)",
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
            key: d(app.config.keybindings.jobs.jump_to_failure.clone()),
            action: "Jump to the next failure line in the job trace",
        },
        Shortcut {
            category: "Jobs",
            key: s("m"),
            action: "Collapse / expand matrix jobs",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.copy_sha.clone()),
            action: "Copy pipeline commit SHA",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.copy_url.clone()),
            action: "Copy job web URL",
        },
        Shortcut {
            category: "Jobs",
            key: d(app.config.keybindings.jobs.copy_id.clone()),
            action: "Copy job ID",
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
            action: RUNNER_PAUSE_RESUME_ACTION,
        },
        Shortcut {
            category: "Runners",
            key: d(app.config.keybindings.runners.edit_description.clone()),
            action: RUNNER_EDIT_DESCRIPTION_ACTION,
        },
        Shortcut {
            category: "Runners",
            key: d(app.config.keybindings.runners.open_in_browser.clone()),
            action: "Open runner in browser",
        },
        Shortcut {
            category: "Runners",
            key: d(app.config.keybindings.runners.cycle_status.clone()),
            action: if ctx.kind.supports_runner_management() {
                "Cycle status filter (Online, Paused, Offline, All)"
            } else {
                "Cycle status filter (Online, Offline, All)"
            },
        },
        Shortcut {
            category: "Runners",
            key: d(app.config.keybindings.runners.copy_url.clone()),
            action: "Copy runner URL",
        },
        Shortcut {
            category: "Runners",
            key: d(app.config.keybindings.runners.copy_id.clone()),
            action: "Copy runner ID",
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
            action: "Create new release",
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
            action: "Open todo target & mark read (selection: mark all selected read)",
        },
        Shortcut {
            category: "TODOs",
            key: d(app.config.keybindings.todos.open_in_browser.clone()),
            action: "Open todo in browser",
        },
        Shortcut {
            category: "TODOs",
            key: d(app.config.keybindings.todos.mark_as_done.clone()),
            action: "Mark todo (or all selected) done",
        },
        Shortcut {
            category: "TODOs",
            key: d(app.config.keybindings.todos.mark_all_as_read.clone()),
            action: "Mark all todos read",
        },
        Shortcut {
            category: "TODOs",
            key: d(app.config.keybindings.todos.unsubscribe.clone()),
            action: "Unsubscribe from todo target",
        },
        Shortcut {
            category: "TODOs",
            key: d(app.config.keybindings.todos.toggle_show_read.clone()),
            action: "Toggle showing read todos",
        },
        Shortcut {
            category: "TODOs",
            key: d(app.config.keybindings.todos.select_todo.clone()),
            action: "Toggle selection of todo",
        },
        Shortcut {
            category: "TODOs",
            key: d(app.config.keybindings.todos.selection_toggle.clone()),
            action: "Toggle select mode (paint selection while navigating)",
        },
        Shortcut {
            category: "TODOs",
            key: d(app.config.keybindings.todos.select_all.clone()),
            action: "Select all filtered todos",
        },
        Shortcut {
            category: "TODOs",
            key: d(app.config.keybindings.todos.copy_url.clone()),
            action: "Copy todo target URL",
        },
        Shortcut {
            category: "TODOs",
            key: d(app.config.keybindings.todos.copy_reference.clone()),
            action: "Copy todo target reference (#123 / !456)",
        },
        Shortcut {
            category: "TODOs",
            key: d(app.config.keybindings.todos.toggle_preview.clone()),
            action: "Show / hide the todo preview",
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
        Shortcut {
            category: "Terminal",
            key: d(format!(
                "{} / P",
                app.config.keybindings.terminal.inspect_command
            )),
            action: "Inspect the highlighted command (full command, timing, stdout, stderr)",
        },
        Shortcut {
            category: "Terminal",
            key: d(app.config.keybindings.terminal.cycle_status_filter.clone()),
            action: "Cycle the status filter: All, Errors, Success",
        },
        Shortcut {
            category: "Terminal",
            key: d(app.config.keybindings.terminal.rerun_command.clone()),
            action: "Re-run the highlighted glab, gh or git call",
        },
        Shortcut {
            category: "Terminal",
            key: d(app.config.keybindings.terminal.copy_command.clone()),
            action: "Copy the highlighted command line",
        },
        Shortcut {
            category: "Terminal",
            key: d(app.config.keybindings.terminal.copy_output.clone()),
            action: "Copy the highlighted command's output or error",
        },
        Shortcut {
            category: "Terminal",
            key: d(app.config.keybindings.terminal.clear_log.clone()),
            action: "Clear the terminal log",
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
        Shortcut {
            category: "Branches",
            key: d(app.config.keybindings.branches.checkout_branch.clone()),
            action: "Checkout selected branch",
        },
        Shortcut {
            category: "Branches",
            key: d(app.config.keybindings.branches.create_mr.clone()),
            action: if is_github {
                "Create Pull Request from selected branch"
            } else {
                "Create Merge Request from selected branch"
            },
        },
        Shortcut {
            category: "Branches",
            key: d(app.config.keybindings.branches.select_branch.clone()),
            action: "Toggle branch selection",
        },
        Shortcut {
            category: "Branches",
            key: d(app.config.keybindings.branches.selection_toggle.clone()),
            action: "Toggle visual select mode",
        },
        Shortcut {
            category: "Branches",
            key: d(app.config.keybindings.branches.select_all.clone()),
            action: "Select all visible branches",
        },
        Shortcut {
            category: "Branches",
            key: d(app.config.keybindings.branches.open_in_browser.clone()),
            action: "Open branch in browser",
        },
        Shortcut {
            category: "Branches",
            key: d(app.config.keybindings.branches.copy_branch.clone()),
            action: "Copy selected branch name",
        },
        // ── Environments ──
        Shortcut {
            category: "Environments",
            key: d(app.config.keybindings.environments.view_deployments.clone()),
            action: "List and focus the environment's deployments",
        },
        Shortcut {
            category: "Environments",
            key: d(app
                .config
                .keybindings
                .environments
                .focus_deployments
                .clone()),
            action: "Move focus between environments and deployments",
        },
        Shortcut {
            category: "Environments",
            key: s("j / k (focused)"),
            action: "Select a deployment to inspect",
        },
        Shortcut {
            category: "Environments",
            key: d(app
                .config
                .keybindings
                .environments
                .rollback_deployment
                .clone()),
            action: "Roll back to the selected deployment",
        },
        Shortcut {
            category: "Environments",
            key: d(app
                .config
                .keybindings
                .environments
                .view_deployment_log
                .clone()),
            action: VIEW_DEPLOYMENT_LOG_ACTION,
        },
        Shortcut {
            category: "Environments",
            key: d(app.config.keybindings.environments.stop_environment.clone()),
            action: "Stop environment",
        },
        Shortcut {
            category: "Environments",
            key: d(app.config.keybindings.environments.copy_url.clone()),
            action: "Copy environment URL",
        },
        Shortcut {
            category: "Environments",
            key: d(app.config.keybindings.environments.open_in_browser.clone()),
            action: "Open environment in browser",
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
            action: "Submit Review (comment/approve/request changes/keep as draft)",
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
            action: "Interact with comments on current line (focus the diff with `l`/`Tab` first; pushes and drafts both)",
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
            key: s("T"),
            action: "Review threads overview (general, outdated, unresolved)",
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
    if !ctx.kind.supports_runner_management() {
        shortcuts.retain(|shortcut| {
            ![RUNNER_PAUSE_RESUME_ACTION, RUNNER_EDIT_DESCRIPTION_ACTION].contains(&shortcut.action)
        });
    }
    // The few user-defined commands go first, above the long built-in lists.
    shortcuts.splice(
        0..0,
        custom_commands.iter().map(|command| Shortcut {
            category: CUSTOM_COMMANDS_CATEGORY,
            key: d(command.key.clone()),
            action: command.label(),
        }),
    );
    if is_github {
        shortcuts.retain(|shortcut| {
            !matches!(
                (shortcut.category, shortcut.action),
                ("Jobs", START_MANUAL_JOB_ACTION) | ("Environments", VIEW_DEPLOYMENT_LOG_ACTION)
            )
        });
    }

    let active_categories: &[&str] = if app.diff_view().is_some() {
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

    let filtered_shortcuts: Vec<&Shortcut> = shortcuts
        .iter()
        .filter(|s| active_categories.contains(&s.category) && !s.key.trim().is_empty())
        .filter(|s| s.key.split('/').any(|k| !k.trim().is_empty()))
        .collect();

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
    let mut form: crate::form::Form<()> = crate::form::Form::new(
        vec![
            crate::form::Group::Custom(crate::form::Rows::Fixed(HELP_SEARCH_ROWS)),
            crate::form::Group::Custom(crate::form::Rows::Fill),
        ],
        Vec::new(),
    );
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
    let (search_area, table_area) = (layout.groups[0], layout.groups[1]);
    app.overlay_stack
        .push((crate::app::OverlayKind::Help, layout.area));

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
        "Type to search commands...▋".to_string()
    } else {
        format!("{}▋", app.help_search_query)
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
        let (action_text, action_lines) = wrap_cell_text(s.action, action_width);
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
            y: table_area.y.saturating_add(1),
            height: table_area.height.saturating_sub(1),
            ..table_area
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
        assert!(text.contains("View related pipelines for selected MR"));
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
        assert!(text.contains("Create new Pull Request"));
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
        assert!(top.contains("Next tab"), "{top:?}");
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
            !bottom.contains("Next tab"),
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

        // If a keybinding is set to empty string, it is filtered out from help
        app.config.keybindings.issues.create_issue = "".to_string();
        terminal
            .draw(|f| {
                render_help(f, ctx, &mut app, f.area());
            })
            .unwrap();

        let text_empty = buffer_text(&terminal);
        assert!(
            !text_empty.contains("Create new issue"),
            "empty keybinding shortcut should be filtered from help: {text_empty:?}",
        );
    }

    /// The merge dialog for the fixture MR, the most crowded `SubmitDialog`:
    /// a body line, the methods a project allowing merge commits and squash
    /// offers, two toggles and both buttons. Drawn with the ASCII icon set,
    /// the one the snapshots are written in.
    fn render_merge_dialog(width: u16, height: u16) -> Terminal<TestBackend> {
        use crate::domain::mr::{MergeMethod, MergeMethods};
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut app = App::default();
        app.mrs.items =
            serde_json::from_str(include_str!("../../tests/fixtures/mrs.json")).unwrap();
        let project = app.scope.as_str().to_string();
        app.merge_methods.insert(
            project,
            MergeMethods::new(&[MergeMethod::Merge, MergeMethod::Squash], None).unwrap(),
        );
        app.open_submit_dialog(crate::app::SubmitDialog::build(
            crate::app::ConfirmAction::MergeMr(2),
            &app,
        ));
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
}
