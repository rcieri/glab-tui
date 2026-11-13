use crate::app::App;
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
                    OverlayKind::Selector => {
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
                        if let Some(c) = app.configure_mut() {
                            let new = if scroll_down {
                                c.column_checklist_idx.saturating_add(1)
                            } else {
                                c.column_checklist_idx.saturating_sub(1)
                            };
                            c.column_checklist_idx = new;
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
            if let Some(content_rect) = app.content_rect {
                if rect_contains(content_rect, row, col) {
                    app.scroll_active_table(scroll_down);
                    return;
                }
            }
            if let Some(detail_rect) = app.detail_rect {
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
                        handle_submit_dialog_mouse(app, row, col, tx, fx);
                        return;
                    }
                    OverlayKind::SaveMenu => {
                        handle_save_menu_mouse(app, row, col);
                        return;
                    }
                    OverlayKind::Configure => {
                        handle_configure_mouse(app, row, col, tx, fx);
                        return;
                    }
                    OverlayKind::DatePicker => {
                        handle_date_picker_mouse(app, row, col);
                        return;
                    }
                    OverlayKind::Selector => {
                        handle_selector_mouse(app, row, col);
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
                        let over_search = app
                            .help_layout
                            .as_ref()
                            .is_some_and(|layout| rect_contains(layout.groups[0], row, col));
                        if over_search {
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
            if let Some(sidebar_rect) = app.sidebar_rect {
                if rect_contains(sidebar_rect, row, col) {
                    let tab_idx = (row - sidebar_rect.y - 1) as usize;
                    let tabs = app.available_tabs();
                    if tab_idx < tabs.len() {
                        app.active_tab = tabs[tab_idx];
                    }
                    return;
                }
            }

            if let Some(content_rect) = app.content_rect {
                if rect_contains(content_rect, row, col) && row >= content_rect.y + 3 {
                    let row_idx = (row - content_rect.y).saturating_sub(3) as usize;
                    app.click_active_table_row(row_idx);
                }
            }
        }
        _ => {}
    }
}

/// A click on a control picks or flips it; a click on a button presses it.
fn handle_submit_dialog_mouse(
    app: &mut App,
    row: u16,
    col: u16,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    let Some(dialog) = app.submit_dialog_mut() else {
        return;
    };
    let Some(focus) = dialog
        .form
        .layout
        .as_ref()
        .and_then(|layout| layout.focus_at(col, row))
    else {
        return;
    };
    dialog.form.set_focus(focus);
    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    );
    handle_submit_dialog(app, &key, tx, fx);
}

/// A click in the filter box starts filtering; a click on a row moves the
/// cursor there and selects it.
fn handle_selector_mouse(app: &mut App, row: u16, col: u16) {
    let Some(layout) = app.selector_layout.clone() else {
        return;
    };
    let Some(sel) = app.selector_mut() else {
        return;
    };
    let Some(&list) = layout.groups.last() else {
        return;
    };
    if layout.groups.len() > 1 && rect_contains(layout.groups[0], row, col) {
        sel.is_filtering = true;
        return;
    }
    if !rect_contains(list, row, col) {
        return;
    }
    let list_y = list.y;

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

/// A click on a save submenu choice selects it.
fn handle_save_menu_mouse(app: &mut App, row: u16, col: u16) {
    let Some(c) = app.configure_mut() else {
        return;
    };
    let Some(list) = c.save_menu_layout.as_ref().map(|layout| layout.groups[0]) else {
        return;
    };
    if !c.save_menu_open || !rect_contains(list, row, col) {
        return;
    }
    if let Some(&(choice, _)) = crate::ui::SAVE_MENU_CHOICES.get(usize::from(row - list.y)) {
        c.save_menu_selection = Some(choice);
    }
}

/// The arrows beside the month step it; a day in the grid is picked.
fn handle_date_picker_mouse(app: &mut App, row: u16, col: u16) {
    use chrono::Datelike;
    // Columns of a day cell plus the gap after it.
    const DAY_CELL_COLUMNS: u16 = 5;
    let Some(dp) = app.date_picker_mut() else {
        return;
    };
    let Some(calendar) = dp.layout.as_ref().map(|layout| layout.groups[0]) else {
        return;
    };
    if !rect_contains(calendar, row, col) {
        return;
    }
    let offset = col - calendar.x;
    if row == calendar.y {
        let [prev, _, next] = crate::ui::date_picker_month_cells(ratatui::layout::Rect {
            height: 1,
            ..calendar
        });
        if rect_contains(prev, row, col) {
            dp.move_month(-1);
        } else if rect_contains(next, row, col) {
            dp.move_month(1);
        }
        return;
    }
    // The weekday header sits between the month line and the first week.
    let Some(week) = row.checked_sub(calendar.y + 2) else {
        return;
    };
    let weekday = offset / DAY_CELL_COLUMNS;
    if weekday >= 7 {
        return;
    }
    let Some(first) = chrono::NaiveDate::from_ymd_opt(dp.year, dp.month, 1) else {
        return;
    };
    let cell = u32::from(week) * 7 + u32::from(weekday) + 1;
    let day = cell.checked_sub(first.weekday().num_days_from_sunday());
    if let Some(day) =
        day.filter(|&d| (1..=crate::app::days_in_month(dp.year, dp.month)).contains(&d))
    {
        dp.day = day;
    }
}

/// A click focuses the setting under it and acts on it like the keyboard:
/// a column is toggled, every other setting runs its `Enter` action.
fn handle_configure_mouse(
    app: &mut App,
    row: u16,
    col: u16,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    let column_count = app
        .active_tab
        .columns(app.kind(), app.scope.is_group())
        .len();
    let Some(c) = app.configure_mut() else {
        return;
    };
    let Some(list) = c.layout.as_ref().map(|layout| layout.groups[0]) else {
        return;
    };
    if c.save_menu_open || c.editing_page_size || !rect_contains(list, row, col) {
        return;
    }
    let Some(Some(setting)) = c.visible_rows.get(usize::from(row - list.y)).copied() else {
        return;
    };
    c.column_checklist_idx = setting;
    let code = if setting < column_count {
        crossterm::event::KeyCode::Char(' ')
    } else {
        crossterm::event::KeyCode::Enter
    };
    let key = crate::keybinding::KeyPress::from(crossterm::event::KeyEvent::new(
        code,
        crossterm::event::KeyModifiers::NONE,
    ));
    crate::handlers::overlays::configure::handle_key(app, &key, tx, fx);
}
