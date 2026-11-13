use crate::app::{App, SaveMenu};
use crate::effect::Effects;
use crate::event::Event;
use crate::handlers::overlays::handle_submit_dialog;
use tokio::sync::mpsc::UnboundedSender;

fn rect_contains(rect: ratatui::layout::Rect, row: u16, col: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

/// Inner area of a bordered block (Borders::ALL = 1 char per side).
fn border_inner(outer: ratatui::layout::Rect) -> ratatui::layout::Rect {
    ratatui::layout::Rect::new(
        outer.x + 1,
        outer.y + 1,
        outer.width.saturating_sub(2),
        outer.height.saturating_sub(2),
    )
}

pub fn handle_mouse_event(
    app: &mut App,
    mouse_event: &crossterm::event::MouseEvent,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    use crate::app::OverlayKind;
    use crossterm::event::MouseEventKind;

    let row = mouse_event.row;
    let col = mouse_event.column;

    match mouse_event.kind {
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            let scroll_down = mouse_event.kind == MouseEventKind::ScrollDown;

            // Check overlays in z-order (topmost first)
            for (kind, rect) in app.overlay_stack.iter().rev() {
                if !rect_contains(*rect, row, col) {
                    continue;
                }
                match kind {
                    OverlayKind::Selector | OverlayKind::ColumnFilter => {
                        if let Some(sel) = app.selector_mut() {
                            let delta: i32 = if scroll_down { 1 } else { -1 };
                            let new_idx = (sel.cursor_idx as i32 + delta).max(0) as usize;
                            if new_idx < sel.get_filtered_items().len() {
                                sel.cursor_idx = new_idx;
                                sel.state.select(Some(new_idx));
                            }
                        }
                        return;
                    }
                    OverlayKind::EditMenu => {
                        if let Some(menu) = app.edit_menu_mut() {
                            let is_new = menu.is_new();
                            let max = if is_new {
                                menu.fields.len()
                            } else {
                                menu.fields.len().saturating_sub(1)
                            };
                            let mut new = if scroll_down {
                                (menu.selected_idx + 1).min(max)
                            } else {
                                menu.selected_idx.saturating_sub(1)
                            };
                            // Skip section headers
                            while new < menu.fields.len()
                                && menu.fields[new].kind == crate::app::FieldType::Section
                            {
                                if scroll_down {
                                    new = (new + 1).min(max);
                                } else {
                                    if new == 0 {
                                        break;
                                    }
                                    new = new.saturating_sub(1);
                                }
                            }
                            menu.selected_idx = new;
                            menu.state.select(Some(new));
                        }
                        return;
                    }
                    OverlayKind::Help => {
                        crate::handlers::overlays::move_help_selection(app, scroll_down, 1);
                        return;
                    }
                    OverlayKind::Configure => {
                        let last = crate::configure_view::configure_rows(app)
                            .len()
                            .saturating_sub(1);
                        if let Some(c) = app.configure_mut() {
                            c.column_checklist_idx = if scroll_down {
                                c.column_checklist_idx.saturating_add(1).min(last)
                            } else {
                                c.column_checklist_idx.saturating_sub(1)
                            };
                        }
                        return;
                    }
                    OverlayKind::DatePicker => {
                        if let Some(dp) = app.date_picker_mut() {
                            if scroll_down {
                                dp.day =
                                    (dp.day + 1).min(crate::app::days_in_month(dp.year, dp.month));
                            } else {
                                dp.day = if dp.day > 1 { dp.day - 1 } else { 1 };
                            }
                        }
                        return;
                    }
                    OverlayKind::ReviewThreads => {
                        if let Some(overview) = app.review_threads_mut() {
                            let over_list = overview
                                .list_rect
                                .is_some_and(|r| rect_contains(r, row, col));
                            match (over_list, scroll_down) {
                                (true, true) => overview.select(overview.cursor_idx + 1),
                                (true, false) => {
                                    overview.select(overview.cursor_idx.saturating_sub(1))
                                }
                                (false, true) => {
                                    overview.preview_scroll =
                                        overview.preview_scroll.saturating_add(1)
                                }
                                (false, false) => {
                                    overview.preview_scroll =
                                        overview.preview_scroll.saturating_sub(1)
                                }
                            }
                        }
                        return;
                    }
                    OverlayKind::TerminalInspector => {
                        if let Some(inspector) = &mut app.terminal_inspector {
                            inspector.scroll = if scroll_down {
                                inspector.scroll.saturating_add(1)
                            } else {
                                inspector.scroll.saturating_sub(1)
                            };
                        }
                        return;
                    }
                    // Consume scroll on non-scrollable modals
                    _ => return,
                }
            }

            // If a modal is open, don't fall through to table scrolling!
            if app.has_modal() {
                return;
            }

            // Fallback: content area scrolling
            let content_rect = app.layout.map(|l| l.content_area).or(app.content_rect);
            if let Some(content_rect) = content_rect {
                if rect_contains(content_rect, row, col) {
                    app.scroll_active_table(scroll_down);
                    return;
                }
            }
            let detail_rect = app
                .layout
                .and_then(|l| l.detail_rect_opt())
                .or(app.detail_rect);
            if let Some(detail_rect) = detail_rect {
                if rect_contains(detail_rect, row, col) {
                    if scroll_down {
                        app.detail_scroll = app.detail_scroll.saturating_add(1);
                    } else {
                        app.detail_scroll = app.detail_scroll.saturating_sub(1);
                    }
                }
            }
        }

        MouseEventKind::Down(_) => {
            // Check overlays in z-order (topmost first)
            for (kind, rect) in app.overlay_stack.iter().rev() {
                if !rect_contains(*rect, row, col) {
                    continue;
                }
                let inner = border_inner(*rect);
                match kind {
                    OverlayKind::ConfirmPopup => {
                        handle_submit_dialog_mouse(app, *rect, row, col, tx, fx);
                        return;
                    }
                    OverlayKind::ColumnFilter => {
                        handle_selector_mouse(app, inner, row, 3, 3);
                        return;
                    }
                    OverlayKind::SaveMenu => {
                        handle_save_menu_mouse(app, *rect, row, col);
                        return;
                    }
                    OverlayKind::Configure => {
                        handle_configure_mouse(app, row, tx, fx);
                        return;
                    }
                    OverlayKind::DatePicker => {
                        handle_date_picker_mouse(app, *rect, row, col);
                        return;
                    }
                    OverlayKind::Selector => {
                        let has_search = app.selector().is_some_and(|s| s.has_filter_bar());
                        let sr = if has_search { 3 } else { 0 };
                        let notice_rows = if app.modal_notice().is_some() {
                            crate::ui::MODAL_NOTICE_ROWS
                        } else {
                            0
                        };
                        handle_selector_mouse(app, inner, row, sr, 1 + notice_rows);
                        return;
                    }
                    OverlayKind::TextInput => {
                        return;
                    }
                    OverlayKind::ReviewSubmit => {
                        crate::handlers::review_submit::click(app, row, col, tx, fx);
                        return;
                    }
                    OverlayKind::EditMenu => {
                        handle_edit_menu_mouse(app, inner, row, col);
                        return;
                    }
                    OverlayKind::ReviewThreads => {
                        if let Some(overview) = app.review_threads_mut() {
                            if let Some(list) =
                                overview.list_rect.filter(|r| rect_contains(*r, row, col))
                            {
                                let clicked = overview.state.offset() + (row - list.y) as usize;
                                overview.select(clicked);
                            }
                        }
                        return;
                    }
                    OverlayKind::Help => {
                        // Click in search area → focus search
                        if row >= inner.y && row < inner.y + 3 {
                            app.help_search_query.clear();
                        }
                        return;
                    }
                    OverlayKind::TerminalInspector => return,
                }
            }

            // Fallback: don't click through to underlying UI if a modal is open
            if app.has_modal() {
                return;
            }

            // Fallback: sidebar / content clicks
            let sidebar_rect = app
                .layout
                .and_then(|l| l.sidebar_rect_opt())
                .or(app.sidebar_rect);
            if let Some(sidebar_rect) = sidebar_rect {
                if rect_contains(sidebar_rect, row, col) {
                    let tabs = app.available_tabs();
                    let clicked = (row - sidebar_rect.y)
                        .checked_sub(1)
                        .and_then(|idx| tabs.get(idx as usize).copied());
                    if let Some(tab) = clicked.filter(|tab| *tab != app.active_tab) {
                        app.switch_to_tab(tab);
                        crate::handlers::tabs::load_entered_tab(app, tx, fx);
                    }
                    return;
                }
            }

            let content_rect = app.layout.map(|l| l.content_area).or(app.content_rect);
            if let Some(content_rect) = content_rect {
                if rect_contains(content_rect, row, col) && row >= content_rect.y + 3 {
                    let row_idx = (row - content_rect.y).saturating_sub(3) as usize;
                    app.click_active_table_row(row_idx);
                }
            }
        }
        _ => {}
    }
}

/// Mouse click on SubmitDialog buttons and option rows.
///
/// The dialog button band sits at the bottom of the popup area; the
/// options (if any) sit between the body and the button band. Clicks
/// outside both zones are ignored — keyboard navigation still drives
/// the dialog otherwise.
fn handle_submit_dialog_mouse(
    app: &mut App,
    rect: ratatui::layout::Rect,
    row: u16,
    col: u16,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    let Some(dialog) = app.submit_dialog() else {
        return;
    };

    let button_height: u16 = 3;
    let band_top = rect.y + rect.height.saturating_sub(button_height);

    // Click anywhere in the button band.
    if row >= band_top && row < rect.y + rect.height {
        let buttons: Vec<usize> = dialog.button_cursors().collect();
        let inner_x = rect.x + 1;
        let inner_width = usize::from(rect.width.saturating_sub(2).max(1));
        let offset = usize::from(col.saturating_sub(inner_x));
        let button = (offset * buttons.len() / inner_width).min(buttons.len() - 1);
        if let Some(d) = app.submit_dialog_mut() {
            d.cursor_idx = buttons[button];
        }
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        handle_submit_dialog(app, &key, tx, fx);
        return;
    }

    // Option rows sit just above the separator, after the body.
    let body_lines =
        crate::utils::format::wrap_text(&dialog.body, crate::app::SubmitDialog::BODY_INNER_WIDTH)
            .len()
            .max(1);
    let option_rows = dialog.options.len();
    if option_rows > 0 {
        let options_y_start = rect.y + 2 + body_lines as u16;
        if row >= options_y_start && row < options_y_start + option_rows as u16 {
            let option_idx = (row - options_y_start) as usize;
            if let Some(d) = app.submit_dialog_mut() {
                d.cursor_idx = option_idx + 1;
                d.toggle_focused_option();
            }
        }
    }
}

/// Mouse click on selector list area.
fn handle_selector_mouse(
    app: &mut App,
    inner: ratatui::layout::Rect,
    row: u16,
    search_rows: u16,
    footer_rows: u16,
) {
    let sel = match app.selector_mut() {
        Some(s) => s,
        None => return,
    };

    // Search/filter bar click area
    if search_rows > 0 {
        let search_bot = inner.y + search_rows;
        if row >= inner.y && row < search_bot {
            sel.is_filtering = true;
            return;
        }
    }

    // List area: after search, before footer
    let list_y = inner.y + search_rows;
    let list_h = inner.height.saturating_sub(search_rows + footer_rows);
    let list_bot = list_y + list_h;

    if row < list_y || row >= list_bot {
        return;
    }

    let offset = sel.state.offset();
    let item_idx = (row - list_y) as usize + offset;
    let items = sel.get_filtered_items();
    if item_idx >= items.len() {
        return;
    }

    sel.cursor_idx = item_idx;
    sel.state.select(Some(item_idx));

    // Toggle or select
    if sel.multi_select {
        let item = items[item_idx].to_string();
        if sel.selected_items.contains(&item) {
            sel.selected_items.remove(&item);
        } else {
            sel.selected_items.insert(item);
        }
    } else {
        // For single-select: select + confirm
        let item = items[item_idx].to_string();
        sel.selected_items.clear();
        sel.selected_items.insert(item);
    }
}

/// Mouse click on edit menu field list area.
fn handle_edit_menu_mouse(app: &mut App, inner: ratatui::layout::Rect, row: u16, _col: u16) {
    let menu = match app.edit_menu_mut() {
        Some(m) => m,
        None => return,
    };
    let list_bot = inner.y + inner.height.saturating_sub(1);
    if row < inner.y || row >= list_bot {
        return;
    }
    let offset = menu.state.offset();
    let item_idx = (row - inner.y) as usize + offset;
    if item_idx >= menu.fields.len() {
        return;
    }
    if matches!(
        menu.fields[item_idx].kind,
        crate::app::FieldType::ReadOnly | crate::app::FieldType::Section
    ) {
        return;
    }
    menu.selected_idx = item_idx;
    menu.state.select(Some(item_idx));
}

/// Mouse click on save submenu.
fn handle_save_menu_mouse(app: &mut App, rect: ratatui::layout::Rect, row: u16, _col: u16) {
    let Some(c) = app.configure_mut() else {
        return;
    };
    if !c.save_menu_open {
        return;
    }
    let inner = border_inner(rect);
    if row < inner.y || row >= inner.y + 3 {
        return;
    }
    let idx = (row - inner.y) as usize;
    match idx {
        0 => c.save_menu_selection = Some(SaveMenu::Local),
        1 => c.save_menu_selection = Some(SaveMenu::Global),
        2 => c.save_menu_selection = Some(SaveMenu::Cancel),
        _ => {}
    }
}

/// Mouse click on date picker.
fn handle_date_picker_mouse(app: &mut App, rect: ratatui::layout::Rect, row: u16, col: u16) {
    use chrono::Datelike;
    let dp = match app.date_picker_mut() {
        Some(d) => d,
        None => return,
    };
    let inner = border_inner(rect);

    // Header row: ◀ (col 0-2), Month Year (center), ▶ (cols 33-35 for 36-wide)
    let header_y = inner.y;
    if row == header_y {
        let left_arrow = col >= rect.x && col <= rect.x + 3;
        let right_arrow = col >= rect.x + rect.width - 4 && col <= rect.x + rect.width;
        if left_arrow {
            if dp.month == 1 {
                dp.month = 12;
                dp.year = dp.year.saturating_sub(1);
            } else {
                dp.month -= 1;
            }
            dp.day = dp.day.min(crate::app::days_in_month(dp.year, dp.month));
            return;
        }
        if right_arrow {
            if dp.month == 12 {
                dp.month = 1;
                dp.year += 1;
            } else {
                dp.month += 1;
            }
            dp.day = dp.day.min(crate::app::days_in_month(dp.year, dp.month));
            return;
        }
        return;
    }

    // Day grid: rows inner.y+1 through inner.y+7, 7 columns
    let day_y = row.saturating_sub(inner.y + 1);
    if day_y >= 7 {
        return;
    }
    // Columns in the grid: each cell ~5 chars wide (36-2 border = 34, 34/7 ≈ 4.8)
    let grid_start = inner.x;
    let grid_width = inner.width;
    let col_width = grid_width / 7;
    let day_col = col.saturating_sub(grid_start) / col_width;
    if day_col >= 7 {
        return;
    }
    let Some(first_date) = chrono::NaiveDate::from_ymd_opt(dp.year, dp.month, 1) else {
        return;
    };
    let start_weekday = first_date.weekday().num_days_from_sunday();
    let total_days = crate::app::days_in_month(dp.year, dp.month);
    let cell_idx = day_y * 7 + day_col;
    let day_num = (cell_idx as i32) - (start_weekday as i32) + 1;
    if day_num >= 1 && day_num <= total_days as i32 {
        dp.day = day_num as u32;
    }
}

/// Mouse click on a Configure View row: focus it and act on it like `Space`.
/// Ignored while the save menu is open or the page size is being typed, as
/// keys are.
fn handle_configure_mouse(app: &mut App, row: u16, tx: &UnboundedSender<Event>, fx: &mut Effects) {
    if app
        .configure()
        .is_none_or(|c| c.save_menu_open || c.editing_page_size)
    {
        return;
    }
    let Some(idx) = app.configure().and_then(|c| {
        c.row_hits
            .iter()
            .find(|(y, _)| *y == row)
            .map(|(_, idx)| *idx)
    }) else {
        return;
    };
    let Some(clicked) = crate::configure_view::configure_rows(app).get(idx).copied() else {
        return;
    };
    crate::handlers::overlays::configure::focus(app, idx);
    crate::handlers::overlays::configure::activate_row(app, clicked, tx, fx);
}
