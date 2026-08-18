#![allow(clippy::all)]
#![allow(unused_variables)]
#![allow(unused_assignments)]

mod app;
mod backend;
mod cli;
mod config;
mod domain;
mod editor;
mod entity_editor;
mod event;
mod fetch;
mod git_helpers;
pub mod handlers;
mod keybinding;
mod templates;
mod ui;
pub mod utils;

use anyhow::Result;
use app::{App, SaveMenu};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use event::{Event, EventHandler};
use ratatui::{Terminal, backend::CrosstermBackend, widgets::ListState};
use std::io;

type AppTerminal = Terminal<CrosstermBackend<std::io::Stdout>>;

pub use editor::*;
pub use entity_editor::*;

fn parse_key_value_pairs(input: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut current = String::new();
    let mut in_parens: i32 = 0;
    for c in input.chars() {
        match c {
            '(' => {
                in_parens += 1;
                current.push(c);
            }
            ')' => {
                in_parens = in_parens.saturating_sub(1);
                current.push(c);
            }
            ',' if in_parens == 0 => {
                if !current.trim().is_empty() {
                    if let Some(pos) = current.find(':').or_else(|| current.find('=')) {
                        let k = current[..pos].trim().to_string();
                        let v = current[pos + 1..].trim().to_string();
                        if !k.is_empty() {
                            pairs.push((k, v));
                        }
                    }
                }
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }
    if !current.trim().is_empty() {
        if let Some(pos) = current.find(':').or_else(|| current.find('=')) {
            let k = current[..pos].trim().to_string();
            let v = current[pos + 1..].trim().to_string();
            if !k.is_empty() {
                pairs.push((k, v));
            }
        }
    }
    pairs
}

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

fn handle_mouse_event(app: &mut App, mouse_event: &crossterm::event::MouseEvent) {
    use app::OverlayKind;
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
                let inner = border_inner(*rect);
                match kind {
                    OverlayKind::Selector | OverlayKind::ColumnFilter => {
                        if let Some(ref mut sel) = app.selector {
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
                        if let Some(ref mut menu) = app.edit_menu {
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
                                    new = new.saturating_sub(1);
                                }
                            }
                            menu.selected_idx = new;
                            menu.state.select(Some(new));
                        }
                        return;
                    }
                    OverlayKind::Help => {
                        return;
                    }
                    OverlayKind::Configure => {
                        let new = if scroll_down {
                            app.column_checklist_idx.saturating_add(1)
                        } else {
                            app.column_checklist_idx.saturating_sub(1)
                        };
                        app.column_checklist_idx = new;
                        return;
                    }
                    OverlayKind::DatePicker => {
                        if let Some(ref mut dp) = app.date_picker {
                            if scroll_down {
                                dp.day =
                                    (dp.day + 1).min(crate::app::days_in_month(dp.year, dp.month));
                            } else {
                                dp.day = if dp.day > 1 { dp.day - 1 } else { 1 };
                            }
                        }
                        return;
                    }
                    // Consume scroll on non-scrollable modals
                    _ => return,
                }
            }

            // Fallback: content area scrolling
            if let Some(content_rect) = app.content_rect {
                if rect_contains(content_rect, row, col) {
                    if let Some(s) = app.active_table_state_mut() {
                        let selected = s.selected().unwrap_or(0);
                        let new = if scroll_down {
                            selected.saturating_add(1)
                        } else {
                            selected.saturating_sub(1)
                        };
                        s.select(Some(new));
                    }
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
                    return;
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
                        handle_confirm_popup_mouse(app, *rect, row, col);
                        return;
                    }
                    OverlayKind::ColumnFilter => {
                        handle_selector_mouse(app, inner, row, col, 3, 3);
                        return;
                    }
                    OverlayKind::SaveMenu => {
                        handle_save_menu_mouse(app, *rect, row, col);
                        return;
                    }
                    OverlayKind::Configure => {
                        handle_configure_mouse(app, *rect, row, col);
                        return;
                    }
                    OverlayKind::DatePicker => {
                        handle_date_picker_mouse(app, *rect, row, col);
                        return;
                    }
                    OverlayKind::Selector => {
                        let has_search = app.selector.as_ref().map_or(false, |s| {
                            s.field_type != "comment_action_select"
                                && s.field_type != "review_submit_status"
                                && s.field_type != "merge_options"
                        });
                        let sr = if has_search { 3 } else { 0 };
                        handle_selector_mouse(app, inner, row, col, sr, 1);
                        return;
                    }
                    OverlayKind::EditMenu => {
                        handle_edit_menu_mouse(app, inner, row, col);
                        return;
                    }
                    OverlayKind::Help => {
                        // Click in search area → focus search
                        if row >= inner.y && row < inner.y + 3 {
                            app.help_search_query.clear();
                        }
                        return;
                    }
                }
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
                    if let Some(s) = app.active_table_state_mut() {
                        s.select(Some(row_idx));
                    }
                    return;
                }
            }
        }
        _ => {}
    }
}

/// Mouse click on confirm popup YES/NO buttons.
fn handle_confirm_popup_mouse(app: &mut App, rect: ratatui::layout::Rect, row: u16, col: u16) {
    // Footer area: last 3 rows of 60x9 = rows 6-8
    let footer_y = rect.y + 6;
    if row < footer_y || row >= rect.y + rect.height {
        return;
    }
    let inner = border_inner(rect);
    let mid = inner.x + inner.width / 2;
    if col < mid {
        app.confirm_popup_selected_yes = true;
    } else {
        app.confirm_popup_selected_yes = false;
    }
    // Execute or cancel
    if let Some(confirm_action) = app.confirm_popup.take() {
        if app.confirm_popup_selected_yes {
            let client = app.gitlab_client.clone();
            let project_path = app.project_context.clone();
            let tx = client.as_ref().and_then(|c| c.tx.clone());
            if let (Some(client), Some(tx)) = (client, tx) {
                match confirm_action {
                    crate::app::ConfirmAction::DeleteMilestone(iid) => {
                        tokio::spawn(async move {
                            let res = crate::domain::milestones::delete_milestone(
                                &client,
                                &project_path,
                                iid,
                            )
                            .await;
                            match res {
                                Ok(_) => {
                                    let _ = tx.send(crate::event::Event::CommandCompleted(
                                        crate::app::Tab::Milestones,
                                        Ok(()),
                                    ));
                                    let _ = tx.send(crate::event::Event::MilestoneDeleted);
                                }
                                Err(e) => {
                                    let _ = tx.send(crate::event::Event::CommandCompleted(
                                        crate::app::Tab::Milestones,
                                        Err(e.to_string()),
                                    ));
                                }
                            }
                        });
                    }
                    crate::app::ConfirmAction::DeleteRelease(tag_name) => {
                        tokio::spawn(async move {
                            let res = crate::domain::releases::delete_release(
                                &client,
                                &project_path,
                                &tag_name,
                            )
                            .await;
                            match res {
                                Ok(_) => {
                                    let _ = tx.send(crate::event::Event::CommandCompleted(
                                        crate::app::Tab::Releases,
                                        Ok(()),
                                    ));
                                    let _ = tx.send(crate::event::Event::ReleaseDeleted);
                                }
                                Err(e) => {
                                    let _ = tx.send(crate::event::Event::CommandCompleted(
                                        crate::app::Tab::Releases,
                                        Err(e.to_string()),
                                    ));
                                }
                            }
                        });
                    }
                    crate::app::ConfirmAction::DeleteBranch(branch_name) => {
                        tokio::spawn(async move {
                            let res = crate::domain::branches::delete_branch(
                                &client,
                                &project_path,
                                &branch_name,
                            )
                            .await;
                            let _ = tx.send(crate::event::Event::CommandCompleted(
                                crate::app::Tab::Branches,
                                res.map(|_| ())
                                    .map_err(|e| format!("Failed to delete branch: {}", e)),
                            ));
                        });
                    }
                    crate::app::ConfirmAction::CloseIssue(iid) => {
                        if let Some(pos) = app.issues.items.iter().position(|i| i.iid == iid) {
                            app.issues.items.remove(pos);
                        }
                        app.update_filter_selection();
                        let tx2 = tx.clone();
                        tokio::spawn(async move {
                            let result = client.close_issue(&project_path, iid).await;
                            let _ = tx2.send(crate::event::Event::CommandCompleted(
                                crate::app::Tab::Issues,
                                result.map_err(|e| e.to_string()),
                            ));
                        });
                    }
                    crate::app::ConfirmAction::DeleteIssue(iid) => {
                        tokio::spawn(async move {
                            let res = client.delete_issue(&project_path, iid).await;
                            match res {
                                Ok(_) => {
                                    let _ = tx.send(crate::event::Event::CommandCompleted(
                                        crate::app::Tab::Issues,
                                        Ok(()),
                                    ));
                                    let _ = tx.send(crate::event::Event::IssueDeleted);
                                }
                                Err(e) => {
                                    let _ = tx.send(crate::event::Event::CommandCompleted(
                                        crate::app::Tab::Issues,
                                        Err(format!("Failed to delete issue: {}", e)),
                                    ));
                                }
                            }
                        });
                    }
                    crate::app::ConfirmAction::CloseMr(iid) => {
                        if let Some(pos) = app.mrs.items.iter().position(|m| m.iid == iid) {
                            app.mrs.items.remove(pos);
                        }
                        app.update_filter_selection();
                        let tx2 = tx.clone();
                        tokio::spawn(async move {
                            let result = client.close_mr(&project_path, iid).await;
                            let _ = tx2.send(crate::event::Event::CommandCompleted(
                                crate::app::Tab::MergeRequests,
                                result.map_err(|e| e.to_string()),
                            ));
                        });
                    }
                    crate::app::ConfirmAction::DeleteMr(iid) => {
                        tokio::spawn(async move {
                            let res = client.delete_mr(&project_path, iid).await;
                            match res {
                                Ok(_) => {
                                    let _ = tx.send(crate::event::Event::CommandCompleted(
                                        crate::app::Tab::MergeRequests,
                                        Ok(()),
                                    ));
                                    let _ = tx.send(crate::event::Event::MrDeleted);
                                }
                                Err(e) => {
                                    let _ = tx.send(crate::event::Event::CommandCompleted(
                                        crate::app::Tab::MergeRequests,
                                        Err(format!("Failed to delete merge request: {}", e)),
                                    ));
                                }
                            }
                        });
                    }
                    crate::app::ConfirmAction::MergeMr(iid) => {
                        if let Some(pos) = app.mrs.items.iter().position(|m| m.iid == iid) {
                            app.mrs.items.remove(pos);
                        }
                        app.update_filter_selection();
                        let tx2 = tx.clone();
                        tokio::spawn(async move {
                            let result =
                                client.merge_mr(&project_path, iid, true, true, None).await;
                            let _ = tx2.send(crate::event::Event::CommandCompleted(
                                crate::app::Tab::MergeRequests,
                                result.map_err(|e| e.to_string()),
                            ));
                        });
                    }
                    crate::app::ConfirmAction::RevokeMr(iid) => {
                        let tx2 = tx.clone();
                        tokio::spawn(async move {
                            let result = client.revoke_mr(&project_path, iid).await;
                            let _ = tx2.send(crate::event::Event::CommandCompleted(
                                crate::app::Tab::MergeRequests,
                                result.map_err(|e| e.to_string()),
                            ));
                        });
                    }
                    crate::app::ConfirmAction::RebaseMr(iid) => {
                        let tx2 = tx.clone();
                        tokio::spawn(async move {
                            let result = client.rebase_mr(&project_path, iid).await;
                            let _ = tx2.send(crate::event::Event::CommandCompleted(
                                crate::app::Tab::MergeRequests,
                                result.map_err(|e| e.to_string()),
                            ));
                        });
                    }
                    crate::app::ConfirmAction::SubmitReview(mr_iid) => {
                        app.selector = Some(crate::app::Selector {
                            title: " Submit Pull Request Review ".to_string(),
                            all_items: vec![
                                "Approve".to_string(),
                                "Request Changes".to_string(),
                                "Comment".to_string(),
                            ],
                            selected_items: std::collections::HashSet::new(),
                            cursor_idx: 0,
                            search_query: String::new(),
                            is_filtering: false,
                            is_loading: false,
                            entity_iid: mr_iid,
                            entity_type: "mr".to_string(),
                            field_type: "review_submit_status".to_string(),
                            multi_select: false,
                            state: ratatui::widgets::ListState::default(),
                        });
                    }
                }
            }
        } else {
            // NO clicked — if the confirm_action was SubmitReview, clean up
            if matches!(confirm_action, crate::app::ConfirmAction::SubmitReview(_)) {
                app.draft_comments.clear();
                app.in_review_mode = false;
                app.diff_view = None;
            }
        }
    }
}

/// Mouse click on selector list area.
fn handle_selector_mouse(
    app: &mut App,
    inner: ratatui::layout::Rect,
    row: u16,
    col: u16,
    search_rows: u16,
    footer_rows: u16,
) {
    let sel = match &mut app.selector {
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
    let menu = match &mut app.edit_menu {
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
    menu.selected_idx = item_idx;
    menu.state.select(Some(item_idx));
}

/// Mouse click on save submenu.
fn handle_save_menu_mouse(app: &mut App, rect: ratatui::layout::Rect, row: u16, _col: u16) {
    if !app.save_menu_open {
        return;
    }
    let inner = border_inner(rect);
    if row < inner.y || row >= inner.y + 3 {
        return;
    }
    let idx = (row - inner.y) as usize;
    match idx {
        0 => app.save_menu_selection = Some(SaveMenu::Local),
        1 => app.save_menu_selection = Some(SaveMenu::Global),
        2 => app.save_menu_selection = Some(SaveMenu::Cancel),
        _ => {}
    }
}

/// Mouse click on date picker.
fn handle_date_picker_mouse(app: &mut App, rect: ratatui::layout::Rect, row: u16, col: u16) {
    use chrono::Datelike;
    let dp = match &mut app.date_picker {
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

/// Mouse click on column configure overlay.
fn handle_configure_mouse(app: &mut App, rect: ratatui::layout::Rect, row: u16, col: u16) {
    let tab = app.active_tab;
    let kind = app.kind();
    let cols = tab.columns(kind);
    let group_cols: Vec<&str> = cols.iter().copied().collect();
    let columns_list: Vec<(usize, &str)> = cols.iter().copied().enumerate().collect();
    let themes = crate::config::all_theme_presets();

    // Build layout same as rendering
    let inner = border_inner(rect);
    let mut y_off = inner.y;

    // Skip COLUMNS header
    y_off += 1;
    // Each column item
    let col_start = y_off;
    let col_end = col_start + columns_list.len() as u16;
    y_off = col_end;
    y_off += 1; // spacer
    // GROUP BY header
    y_off += 1;
    let group_start = y_off;
    let group_end = group_start + group_cols.len() as u16;
    y_off = group_end;
    y_off += 1; // spacer
    // ORDER header
    y_off += 1;
    let order_start = y_off;
    let order_end = order_start + 2;
    y_off = order_end;
    y_off += 1; // spacer
    // PAGE SIZE
    y_off += 1; // header
    let page_size_start = y_off;
    y_off += 1; // value
    y_off += 1; // spacer
    // THEME header
    y_off += 1;
    let theme_start = y_off;
    let theme_end = theme_start + themes.len() as u16;
    y_off = theme_end;
    y_off += 1; // spacer
    // SAVE
    y_off += 1;
    let save_y = y_off;

    if row >= col_start && row < col_end {
        let idx = (row - col_start) as usize;
        if idx < columns_list.len() {
            let (orig_idx, col_name) = columns_list[idx];
            let col_str = col_name.to_string();
            if let Some(set) = app.enabled_columns.get_mut(&tab) {
                if set.contains(&col_str) {
                    set.remove(&col_str);
                } else {
                    set.insert(col_str);
                }
                app.update_filter_selection();
            }
            app.column_checklist_idx = orig_idx;
        }
    } else if row >= group_start && row < group_end {
        let idx = (row - group_start) as usize;
        if idx < group_cols.len() {
            let col = group_cols[idx];
            app.group_by_column.insert(tab, Some(col.to_string()));
            app.column_checklist_idx = cols.len() + idx;
        }
    } else if row >= order_start && row < order_end {
        let idx = (row - order_start) as usize;
        if idx == 0 {
            app.group_ascending.insert(tab, true);
        } else {
            app.group_ascending.insert(tab, false);
        }
        app.column_checklist_idx = group_end as usize + idx;
    } else if row == page_size_start {
        app.editing_page_size = true;
    } else if row >= theme_start && row < theme_end {
        let idx = (row - theme_start) as usize;
        if idx < themes.len() {
            app.config.theme_preset = Some(themes[idx].to_string());
            app.apply_config();
        }
    } else if row == save_y {
        app.save_menu_open = true;
    }
}

pub use git_helpers::*;
pub use keybinding::keybinding_matches;
pub use templates::*;

pub use fetch::spawn_fetch_repo_attributes;
pub use fetch::spawn_refresh_active_tab;
use handlers::overlays::*;

#[tokio::main]
async fn main() -> Result<()> {
    use clap::Parser;

    // ── Subcommand dispatch ──
    let cli = cli::Cli::parse();

    if let Some(cmd) = cli.command {
        match cmd {
            cli::Commands::Doctor => {
                cli::run_doctor().await;
                return Ok(());
            }
            cli::Commands::CleanCache { dry_run } => {
                cli::run_clean_cache(dry_run);
                return Ok(());
            }
            cli::Commands::Cache => {
                cli::run_cache_list();
                return Ok(());
            }
            cli::Commands::Open { entity, id } => {
                cli::run_open_in_browser(&entity, &id);
                return Ok(());
            }
            cli::Commands::Repos => {
                cli::run_repos_list();
                return Ok(());
            }
        }
    }

    if cli.update {
        cli::run_update().await;
        return Ok(());
    }

    let custom_repo = cli.repo;
    let custom_dir = cli.dir;

    if let Some(ref dir) = custom_dir {
        if let Err(e) = std::env::set_current_dir(dir) {
            eprintln!("Error changing directory to '{}': {}", dir, e);
            std::process::exit(1);
        }
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and event handler
    let mut app = App::new();
    let mut events = EventHandler::new(250);
    app.tx = Some(events.sender());

    // Initialize gitlab context
    if let Some(repo) = custom_repo {
        app.project_context = repo;
    } else if let Ok(context) = domain::client::get_project_context().await {
        app.project_context = context;
    }

    // Add current directory to recent repositories list
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(cwd_str) = cwd.to_str() {
            crate::utils::cache::add_recent_repo(cwd_str);
        }
    }

    // Load offline cache
    let cache = crate::utils::cache::load_cache(&app.project_context);
    app.project_cache = cache.clone();
    app.issues.items = cache.issues;
    app.mrs.items = cache.mrs;
    // workflow is #[serde(skip)] — cached rows arrive with it unset even
    // though the approval state it derives from was persisted and just
    // loaded above.
    crate::fetch::derive_workflow(&mut app.mrs.items);
    app.pipelines.items = cache.pipelines;
    app.runners.items = cache.runners;
    app.releases.items = cache.releases;
    app.todos.items = cache.todos;
    app.milestones.items = cache.milestones;
    app.pipeline_jobs = cache.pipeline_jobs;
    app.branches.items = cache.branches;
    app.environments.items = cache.environments;
    app.milestone_issues_cache = cache.milestone_issues;
    app.cached_labels = cache.labels;
    if app.config.fetch_label_colors {
        app.label_colors = cache
            .label_colors
            .iter()
            .filter_map(|(name, hex)| crate::config::hex_to_color(hex).map(|c| (name.clone(), c)))
            .collect();
    }
    app.cached_members = cache.members;

    let has_any_cached = !app.issues.items.is_empty()
        || !app.mrs.items.is_empty()
        || !app.pipelines.items.is_empty()
        || !app.runners.items.is_empty()
        || !app.releases.items.is_empty()
        || !app.todos.items.is_empty()
        || !app.milestones.items.is_empty();
    if has_any_cached {
        app.status_message = Some("Loaded from offline cache".to_string());
    }

    if !app.issues.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Issues);
    }
    if !app.mrs.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::MergeRequests);
    }
    if !app.pipelines.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Pipelines);
    }
    if !app.runners.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Runners);
    }
    if !app.releases.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Releases);
    }
    if !app.todos.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Todos);
    }
    if !app.milestones.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Milestones);
    }
    if !app.branches.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Branches);
    }
    if !app.environments.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Environments);
    }
    app.update_filter_selection();

    if let Ok(mut client) = domain::client::GitlabClient::new(&app.config).await {
        client.page_size = app.config.page_size;
        client.api_per_page = app.config.api_per_page_clamped();
        client.tx = Some(events.sender());
        app.gitlab_client = Some(client.clone());
        let tx = events.sender();
        if app.issues.items.is_empty() {
            app.start_loading_tab(app.active_tab);
        }
        spawn_refresh_active_tab(&client, &app.project_context, app.active_tab, tx.clone());
        spawn_fetch_repo_attributes(&client.muted(), &app.project_context, tx);
    } else {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        app.terminal_commands.push(crate::app::TerminalCommand {
            timestamp,
            command: "Initialization: gitlab client".to_string(),
            status: "Failed: Failed to initialize GitLab client".to_string(),
        });
        app.error_message = Some("Failed to initialize GitLab client".to_string());
    }

    // If we couldn't detect a valid project, prompt to select a cached repo
    if app.project_context == "unknown/unknown" || app.project_context == "group/repository" {
        let switchable = crate::utils::cache::get_switchable_repos();
        if !switchable.is_empty() {
            let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
            app.terminal_commands.push(crate::app::TerminalCommand {
                timestamp,
                command: "Startup: Not in a git repository".to_string(),
                status: "Failed: No repo detected — select one below or press Esc".to_string(),
            });
            app.selector = Some(crate::app::Selector {
                title: " No Repo Detected — Select a Repository ".to_string(),
                all_items: switchable,
                selected_items: std::collections::HashSet::new(),
                cursor_idx: 0,
                search_query: String::new(),
                is_filtering: false,
                is_loading: false,
                entity_iid: 0,
                entity_type: "app".to_string(),
                field_type: "switch_repo".to_string(),
                multi_select: false,
                state: {
                    let mut s = ListState::default();
                    s.select(Some(0));
                    s
                },
            });
        }
    }

    let mut last_refresh = std::time::Instant::now();
    let mut last_active_tab = app.active_tab;

    // Run app
    while app.running {
        if app.active_tab == app::Tab::Pipelines {
            if let Some(client) = &app.gitlab_client {
                if let Some(idx) = app.pipelines.state.selected() {
                    let pipe_id = app.filtered_pipelines().get(idx).map(|p| p.id());
                    if let Some(pipe_id) = pipe_id {
                        if !app.pipeline_jobs.contains_key(&pipe_id)
                            && !app.fetching_pipelines.contains(&pipe_id)
                        {
                            app.fetching_pipelines.insert(pipe_id);
                            let client_clone = client.clone();
                            let project_context = app.project_context.clone();
                            let tx = events.sender();
                            tokio::spawn(async move {
                                if let Ok(jobs) = domain::pipelines::list_pipeline_jobs(
                                    &client_clone,
                                    &project_context,
                                    pipe_id,
                                )
                                .await
                                {
                                    let _ = tx.send(Event::PipelineJobs(pipe_id, jobs));
                                } else {
                                    let _ = tx.send(Event::PipelineJobs(pipe_id, vec![]));
                                }
                            });
                        }
                    }
                }
            }
        }

        if app.active_tab == app::Tab::MergeRequests {
            if let Some(client) = &app.gitlab_client {
                if let Some(idx) = app.mrs.state.selected() {
                    let m = app.filtered_mrs().get(idx).cloned();
                    if let Some(m) = m {
                        let is_github = client.is_github;
                        let resolved_pipe =
                            m.head_pipeline.as_ref().map(|p| p.id()).or_else(|| {
                                if is_github {
                                    app.pipelines
                                        .items
                                        .iter()
                                        .find(|p| p.ref_branch() == m.source_branch)
                                        .map(|p| p.id())
                                } else {
                                    None
                                }
                            });
                        if let Some(pipe_id) = resolved_pipe {
                            if !app.pipeline_jobs.contains_key(&pipe_id)
                                && !app.fetching_pipelines.contains(&pipe_id)
                            {
                                app.fetching_pipelines.insert(pipe_id);
                                let client_clone = client.clone();
                                let project_context = app.project_context.clone();
                                let tx = events.sender();
                                tokio::spawn(async move {
                                    if let Ok(jobs) = domain::pipelines::list_pipeline_jobs(
                                        &client_clone,
                                        &project_context,
                                        pipe_id,
                                    )
                                    .await
                                    {
                                        let _ = tx.send(Event::PipelineJobs(pipe_id, jobs));
                                    } else {
                                        let _ = tx.send(Event::PipelineJobs(pipe_id, vec![]));
                                    }
                                });
                            }
                        }
                    }
                }
            }
        }

        if app.active_tab == app::Tab::Milestones {
            if let Some(client) = &app.gitlab_client {
                if let Some(idx) = app.milestones.state.selected() {
                    let milestone = app
                        .filtered_milestones()
                        .get(idx)
                        .map(|m| (m.iid, m.title.clone()));
                    if let Some((milestone_iid, milestone_title)) = milestone {
                        if app.selected_milestone_iid != Some(milestone_iid) {
                            app.selected_milestone_iid = Some(milestone_iid);
                            // Use cached data if available; only fetch if not yet cached.
                            // An empty cached list is NOT treated as data: it is what a
                            // failed/buggy fetch (e.g. `--milestone <iid>` matching nothing)
                            // leaves behind, and trusting it would freeze progress at 0%.
                            let cached = app
                                .milestone_issues_cache
                                .get(&milestone_iid)
                                .filter(|c| !c.is_empty());
                            if let Some(cached) = cached {
                                app.selected_milestone_issues = Some(cached.clone());
                            } else {
                                app.selected_milestone_issues = None;
                                let client_clone = client.clone();
                                let project_context = app.project_context.clone();
                                // `glab issue list --milestone` filters by milestone
                                // title, not iid — passing the title here is required
                                // for the glab backend to return any issues.
                                let tx = events.sender();
                                tokio::spawn(async move {
                                    match domain::milestones::list_milestone_issues(
                                        &client_clone,
                                        &project_context,
                                        milestone_iid,
                                        &milestone_title,
                                    )
                                    .await
                                    {
                                        Ok(issues) => {
                                            let _ = tx.send(Event::MilestoneIssuesFetched(
                                                milestone_iid,
                                                issues,
                                            ));
                                        }
                                        Err(e) => {
                                            // Don't silently substitute an empty list: that
                                            // renders as 0% progress with no explanation.
                                            let _ = tx.send(Event::FetchFailed(
                                                app::Tab::Milestones,
                                                format!("Failed to fetch milestone issues: {e}"),
                                            ));
                                        }
                                    }
                                });
                            }
                        }
                    }
                }
            }
        }

        terminal.draw(|f| ui::render(f, &mut app))?;

        if let Some(event) = events.next().await {
            match event {
                Event::Tick => {
                    app.tick();
                    if app.active_tab == app::Tab::Jobs
                        && app.job_trace_follow
                        && app.job_trace.is_some()
                        && !app.job_trace_loading
                        && app.job_trace_last_refresh.elapsed() >= std::time::Duration::from_secs(5)
                    {
                        if let Some(selected) = app.jobs.state.selected() {
                            if let Some(job) = app.filtered_jobs().get(selected) {
                                if let Some(client) = app.gitlab_client.clone() {
                                    let project = app.project_context.clone();
                                    let tx = events.sender();
                                    let job_id = job.id();
                                    app.job_trace_loading = true;
                                    app.job_trace_last_refresh = std::time::Instant::now();
                                    tokio::spawn(async move {
                                        let result = domain::pipelines::get_job_trace(
                                            &client, &project, job_id,
                                        )
                                        .await;
                                        let _ = tx.send(Event::JobTraceFetched(
                                            job_id,
                                            result.map_err(|e| e.to_string()),
                                        ));
                                    });
                                }
                            }
                        }
                    }
                    if app.active_tab != last_active_tab {
                        last_active_tab = app.active_tab;
                        last_refresh = std::time::Instant::now();
                        app.last_attr_refresh = std::time::Instant::now();
                    } else {
                        if last_refresh.elapsed() >= std::time::Duration::from_secs(60)
                            && app.active_tab.is_high_churn()
                        {
                            if app.text_input.is_none()
                                && app.edit_menu.is_none()
                                && app.selector.is_none()
                                && !app.loading_tabs.contains(&app.active_tab)
                            {
                                if let Some(client) = app.gitlab_client.clone() {
                                    app.start_loading_tab(app.active_tab);
                                    spawn_refresh_active_tab(
                                        &client.muted(),
                                        &app.project_context,
                                        app.active_tab,
                                        events.sender(),
                                    );
                                }
                            }
                            last_refresh = std::time::Instant::now();
                        }
                        if app.last_attr_refresh.elapsed() >= std::time::Duration::from_secs(300)
                            && app.text_input.is_none()
                            && app.edit_menu.is_none()
                            && app.selector.is_none()
                        {
                            if let Some(client) = app.gitlab_client.clone() {
                                spawn_fetch_repo_attributes(
                                    &client.muted(),
                                    &app.project_context,
                                    events.sender(),
                                );
                            }
                            app.last_attr_refresh = std::time::Instant::now();
                        }
                    }
                }
                Event::PipelineJobs(id, jobs) => {
                    app.fetching_pipelines.remove(&id);
                    app.pipeline_jobs.insert(id, jobs.clone());

                    let mut is_active = false;
                    if app.active_tab == app::Tab::Jobs && app.active_pipeline_id == Some(id) {
                        is_active = true;
                    } else if app.active_tab == app::Tab::Pipelines {
                        if let Some(idx) = app.pipelines.state.selected() {
                            if app.filtered_pipelines().get(idx).map(|p| p.id()) == Some(id) {
                                is_active = true;
                            }
                        }
                    }

                    if is_active {
                        app.jobs.items = jobs;
                        app.jobs.state.select(app.jobs.state.selected().or(Some(0)));
                    }

                    app.project_cache.pipeline_jobs = app.pipeline_jobs.clone();
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::JobsTabFetched(pipeline_id, jobs) => {
                    app.complete_loading_tab(app::Tab::Jobs, "Success");
                    app.loaded_tabs.insert(app::Tab::Jobs);
                    app.jobs.items = jobs;
                    app.active_pipeline_id = Some(pipeline_id);
                    app.jobs.state.select(Some(0));
                    app.detail_scroll = 0;
                    app.job_trace = None;
                }
                Event::JobTraceFetched(job_id, result) => {
                    app.job_trace_loading = false;
                    let current_selected_job_id = match app.active_tab {
                        app::Tab::Jobs => {
                            if let Some(idx) = app.jobs.state.selected() {
                                app.filtered_jobs().get(idx).map(|j| j.id())
                            } else {
                                None
                            }
                        }
                        app::Tab::Pipelines => {
                            if let Some(idx) = app.jobs.state.selected() {
                                app.jobs.items.get(idx).map(|j| j.id())
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    if current_selected_job_id == Some(job_id) {
                        match result {
                            Ok(trace) => {
                                app.job_trace = Some(trace);
                                app.job_trace_needs_scroll_to_bottom = app.job_trace_follow;
                                app.job_trace_last_refresh = std::time::Instant::now();
                                app.details_zoomed = true;
                                app.detail_visible = true;
                            }
                            Err(e) => {
                                app.error_message = Some(e);
                            }
                        }
                    }
                }
                Event::IssuesFetched(issues) => {
                    app.complete_loading_tab(app::Tab::Issues, "Success");
                    app.loaded_tabs.insert(app::Tab::Issues);
                    app.refreshed_tabs.insert(app::Tab::Issues);
                    app.status_message = None;
                    app.issues.items = issues;
                    app.update_filter_selection();
                    app.project_cache.issues = app.issues.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::MrsFetched(mrs) => {
                    app.complete_loading_tab(app::Tab::MergeRequests, "Success");
                    app.loaded_tabs.insert(app::Tab::MergeRequests);
                    app.refreshed_tabs.insert(app::Tab::MergeRequests);
                    app.status_message = None;
                    app.mrs.items = mrs;
                    app.update_filter_selection();
                    app.project_cache.mrs = app.mrs.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::PipelinesFetched(pipelines) => {
                    app.complete_loading_tab(app::Tab::Pipelines, "Success");
                    app.loaded_tabs.insert(app::Tab::Pipelines);
                    app.refreshed_tabs.insert(app::Tab::Pipelines);
                    app.status_message = None;
                    app.pipelines.items = pipelines;
                    if let Some(pipe_id) = app.pending_pipeline_select.take() {
                        if let Some(idx) =
                            app.pipelines.items.iter().position(|p| p.id() == pipe_id)
                        {
                            app.pipelines.state.select(Some(idx));
                        }
                    }
                    app.update_filter_selection();
                    let new_ids: std::collections::HashSet<u64> =
                        app.pipelines.items.iter().map(|p| p.id()).collect();
                    app.pipeline_jobs.retain(|id, _| new_ids.contains(id));
                    app.fetching_pipelines.clear();
                    app.project_cache.pipelines = app.pipelines.items.clone();
                    app.project_cache.pipeline_jobs = app.pipeline_jobs.clone();
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::TodosFetched(notifs) => {
                    app.complete_loading_tab(app::Tab::Todos, "Success");
                    app.loaded_tabs.insert(app::Tab::Todos);
                    app.refreshed_tabs.insert(app::Tab::Todos);
                    app.status_message = None;
                    app.todos.items = notifs;
                    app.update_filter_selection();
                    app.project_cache.todos = app.todos.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::RunnersFetched(runners) => {
                    app.complete_loading_tab(app::Tab::Runners, "Success");
                    app.loaded_tabs.insert(app::Tab::Runners);
                    app.refreshed_tabs.insert(app::Tab::Runners);
                    app.status_message = None;
                    app.runners.items = runners;
                    app.update_filter_selection();
                    app.project_cache.runners = app.runners.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::ReleasesFetched(releases) => {
                    app.complete_loading_tab(app::Tab::Releases, "Success");
                    app.loaded_tabs.insert(app::Tab::Releases);
                    app.refreshed_tabs.insert(app::Tab::Releases);
                    app.status_message = None;
                    app.releases.items = releases;
                    app.update_filter_selection();
                    app.project_cache.releases = app.releases.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::MilestonesFetched(milestones) => {
                    app.complete_loading_tab(app::Tab::Milestones, "Success");
                    app.loaded_tabs.insert(app::Tab::Milestones);
                    app.refreshed_tabs.insert(app::Tab::Milestones);
                    app.status_message = None;
                    app.milestones.items = milestones;
                    app.update_filter_selection();
                    app.project_cache.milestones = app.milestones.items.clone();
                    app.milestone_issues_cache.clear();
                    app.selected_milestone_issues = None;
                    app.selected_milestone_iid = None;
                    app.project_cache.milestone_issues.clear();
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::MilestoneIssuesFetched(iid, issues) => {
                    let mut fallback_success = false;
                    if issues.is_empty() {
                        // Only fall back to a cached list that actually has data;
                        // a cached empty list is indistinguishable from a failed
                        // fetch and must not mask a real 0% progress display.
                        if let Some(cached) = app
                            .project_cache
                            .milestone_issues
                            .get(&iid)
                            .filter(|c| !c.is_empty())
                        {
                            app.milestone_issues_cache.insert(iid, cached.clone());
                            if app.selected_milestone_iid == Some(iid) {
                                app.selected_milestone_issues = Some(cached.clone());
                                app.status_message = Some(
                                    "Offline fallback: loaded cached milestone issues.".to_string(),
                                );
                            }
                            fallback_success = true;
                        }
                    }
                    if !fallback_success {
                        app.milestone_issues_cache.insert(iid, issues.clone());
                        if app.selected_milestone_iid == Some(iid) {
                            app.selected_milestone_issues = Some(issues.clone());
                        }
                        app.project_cache.milestone_issues.insert(iid, issues);
                        crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                    }
                }
                Event::MilestoneUpdated | Event::MilestoneClosed | Event::MilestoneReopened => {
                    app.status_message = None;
                    app.project_cache.milestones = app.milestones.items.clone();
                    app.milestone_issues_cache.clear();
                    app.selected_milestone_issues = None;
                    app.selected_milestone_iid = None;
                    app.project_cache.milestone_issues.clear();
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::MilestoneDeleted => {
                    app.complete_loading_tab(app::Tab::Milestones, "Success");
                    app.status_message = None;
                    if let Some(iid) = app.pending_delete_milestone_iid.take() {
                        app.milestones.items.retain(|m| m.iid != iid);
                    }
                    app.project_cache.milestones = app.milestones.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::ReleaseUpdated => {
                    app.status_message = None;
                    app.project_cache.releases = app.releases.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::ReleaseDeleted => {
                    app.complete_loading_tab(app::Tab::Releases, "Success");
                    app.status_message = None;
                    if let Some(tag) = app.pending_delete_release_tag.take() {
                        app.releases.items.retain(|r| r.tag_name != tag);
                    }
                    app.project_cache.releases = app.releases.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::IssueDeleted => {
                    app.complete_loading_tab(app::Tab::Issues, "Success");
                    app.status_message = None;
                    app.issues.items.clear();
                    if let Some(client) = app.gitlab_client.clone() {
                        if !app.loading_tabs.contains(&app::Tab::Issues) {
                            app.start_loading_tab(app::Tab::Issues);
                        }
                        spawn_refresh_active_tab(
                            &client,
                            &app.project_context,
                            app::Tab::Issues,
                            events.sender(),
                        );
                    }
                }
                Event::MrDeleted => {
                    app.complete_loading_tab(app::Tab::MergeRequests, "Success");
                    app.status_message = None;
                    app.mrs.items.clear();
                    if let Some(client) = app.gitlab_client.clone() {
                        if !app.loading_tabs.contains(&app::Tab::MergeRequests) {
                            app.start_loading_tab(app::Tab::MergeRequests);
                        }
                        spawn_refresh_active_tab(
                            &client,
                            &app.project_context,
                            app::Tab::MergeRequests,
                            events.sender(),
                        );
                    }
                }
                Event::BranchesFetched(branches) => {
                    app.complete_loading_tab(app::Tab::Branches, "Success");
                    app.loaded_tabs.insert(app::Tab::Branches);
                    app.refreshed_tabs.insert(app::Tab::Branches);
                    app.status_message = None;
                    app.branches.items = branches;
                    app.update_filter_selection();
                    app.project_cache.branches = app.branches.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::EnvironmentsFetched(envs) => {
                    app.complete_loading_tab(app::Tab::Environments, "Success");
                    app.loaded_tabs.insert(app::Tab::Environments);
                    app.refreshed_tabs.insert(app::Tab::Environments);
                    app.status_message = None;
                    app.environments.items = envs;
                    app.update_filter_selection();
                    app.project_cache.environments = app.environments.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::SelectorItemsFetched(items) => {
                    let mut applied_from_cache = false;
                    if items.is_empty() {
                        // Determine which typed cache to fall back on
                        if let Some(ref selector) = app.selector {
                            let fallback = match selector.field_type.as_str() {
                                "labels" => {
                                    if !app.cached_labels.is_empty() {
                                        Some(app.cached_labels.clone())
                                    } else {
                                        None
                                    }
                                }
                                "assignees" | "reviewers" => {
                                    if !app.cached_members.is_empty() {
                                        Some(app.cached_members.clone())
                                    } else {
                                        None
                                    }
                                }
                                "milestone" => Some(
                                    app.milestones
                                        .items
                                        .iter()
                                        .map(|m| m.title.clone())
                                        .collect(),
                                ),
                                "source_branch" | "target_branch" | "pipeline_branch" => Some(
                                    app.branches.items.iter().map(|b| b.name.clone()).collect(),
                                ),
                                _ => None,
                            };
                            if let Some(cached) = fallback {
                                if !cached.is_empty() {
                                    app.status_message = Some(
                                        "Offline fallback: cached selector items.".to_string(),
                                    );
                                    if let Some(mut selector) = app.selector.take() {
                                        selector.all_items = cached;
                                        selector.is_loading = false;
                                        app.selector = Some(selector);
                                    }
                                    applied_from_cache = true;
                                }
                            }
                        }
                    }
                    if !applied_from_cache {
                        // Update typed cache based on field type
                        if let Some(ref selector) = app.selector {
                            match selector.field_type.as_str() {
                                "labels" => {
                                    app.cached_labels = items.clone();
                                    app.project_cache.labels = items.clone();
                                }
                                "assignees" | "reviewers" => {
                                    app.cached_members = items.clone();
                                    app.project_cache.members = items.clone();
                                }
                                _ => {}
                            }
                        }
                        crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                        if let Some(mut selector) = app.selector.take() {
                            if selector.field_type == "milestone" {
                                let mut ms_items = vec!["None".to_string()];
                                ms_items.extend(items.into_iter().filter(|i| i != "None"));
                                selector.all_items = ms_items;
                            } else {
                                selector.all_items = items;
                            }
                            selector.is_loading = false;
                            app.selector = Some(selector);
                        }
                    }
                }
                Event::RepoAttributesFetched { labels, members } => {
                    if !labels.is_empty() {
                        let names: Vec<String> = labels.iter().map(|l| l.name.clone()).collect();
                        app.cached_labels = names.clone();
                        app.project_cache.labels = names;
                        if app.config.fetch_label_colors {
                            let colors: std::collections::HashMap<String, String> = labels
                                .iter()
                                .filter_map(|l| {
                                    l.color.as_ref().map(|c| (l.name.clone(), c.clone()))
                                })
                                .collect();
                            app.project_cache.label_colors = colors.clone();
                            app.label_colors = colors
                                .iter()
                                .filter_map(|(name, hex)| {
                                    crate::config::hex_to_color(hex).map(|c| (name.clone(), c))
                                })
                                .collect();
                        }
                    }
                    if !members.is_empty() {
                        app.cached_members = members.clone();
                        app.project_cache.members = members;
                    }
                    crate::utils::cache::save_cache(&app.project_context, &app.project_cache);
                }
                Event::DeploymentsFetched(deployments) => {
                    app.deployments.items = deployments;
                    app.deployments.state.select(Some(0));
                    app.status_message = None;
                    app.update_filter_selection();
                }
                Event::FetchFailed(tab, err_msg) => {
                    app.complete_loading_tab(tab, &format!("Failed: {}", err_msg));
                    let has_cached_items = match tab {
                        app::Tab::Issues => !app.issues.items.is_empty(),
                        app::Tab::MergeRequests => !app.mrs.items.is_empty(),
                        app::Tab::Pipelines => !app.pipelines.items.is_empty(),
                        app::Tab::Runners => !app.runners.items.is_empty(),
                        app::Tab::Releases => !app.releases.items.is_empty(),
                        app::Tab::Todos => !app.todos.items.is_empty(),
                        app::Tab::Milestones => !app.milestones.items.is_empty(),
                        app::Tab::Branches => !app.branches.items.is_empty(),
                        app::Tab::Environments => !app.environments.items.is_empty(),
                        _ => false,
                    };
                    if has_cached_items {
                        app.status_message = Some("Offline / Connection failed".to_string());
                    } else {
                        app.error_message = Some(err_msg);
                    }
                }
                Event::DiffFetched {
                    mr_iid,
                    raw_diff,
                    comments,
                } => {
                    app.diff_loading = false;
                    app.diff_view = Some(crate::app::DiffView::new(mr_iid, raw_diff));
                    app.current_comments = comments;
                    app.last_fetched_mr_iid = Some(mr_iid);
                    app.in_review_mode = true;
                    if let Some(pos) = app
                        .terminal_commands
                        .iter()
                        .rposition(|cmd| cmd.command.contains("diff") && cmd.status == "Running")
                    {
                        app.terminal_commands[pos].status = "Success".to_string();
                    }
                }
                Event::DiffFetchFailed(err_msg) => {
                    app.diff_loading = false;
                    app.error_message = Some(err_msg.clone());
                    if let Some(pos) = app
                        .terminal_commands
                        .iter()
                        .rposition(|cmd| cmd.command.contains("diff") && cmd.status == "Running")
                    {
                        app.terminal_commands[pos].status = format!("Failed: {}", err_msg);
                    }
                }
                Event::TerminalCommandLogged {
                    timestamp,
                    command,
                    status,
                } => {
                    if status == "Running" {
                        app.terminal_commands.push(crate::app::TerminalCommand {
                            timestamp,
                            command,
                            status,
                        });
                    } else if let Some(pos) = app
                        .terminal_commands
                        .iter()
                        .rposition(|cmd| cmd.command == command && cmd.status == "Running")
                    {
                        app.terminal_commands[pos].status = status;
                    } else if let Some(pos) = app
                        .terminal_commands
                        .iter()
                        .rposition(|cmd| cmd.status == "Running")
                    {
                        // fallback: update most recent Running entry when
                        // command strings differ (e.g. CommandStarted vs backend log)
                        app.terminal_commands[pos].status = status;
                    } else {
                        app.terminal_commands.push(crate::app::TerminalCommand {
                            timestamp,
                            command,
                            status,
                        });
                    }
                }
                Event::CommandStarted(msg) => {
                    app.status_message = Some(msg.clone());
                    let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
                    app.terminal_commands.push(crate::app::TerminalCommand {
                        timestamp,
                        command: msg,
                        status: "Running".to_string(),
                    });
                    // Force an immediate render so the "Running..." banner is visible
                    // even if CommandCompleted arrives in the very next event.
                    terminal.draw(|f| ui::render(f, &mut app))?;
                }
                Event::CommandCompleted(tab, res) => {
                    match &res {
                        Ok(_) => {
                            if let Some(pos) = app
                                .terminal_commands
                                .iter()
                                .rposition(|cmd| cmd.status == "Running")
                            {
                                app.terminal_commands[pos].status = "Success".to_string();
                            }
                        }
                        Err(_) => {}
                    }
                    match res {
                        Ok(_) => {
                            if let Some(client) = app.gitlab_client.clone() {
                                if !app.loading_tabs.contains(&tab) {
                                    app.start_loading_tab(tab);
                                }
                                spawn_refresh_active_tab(
                                    &client.muted(),
                                    &app.project_context,
                                    tab,
                                    events.sender(),
                                );
                            }
                            if let Some(diff_view) = &app.diff_view {
                                let client = app.gitlab_client.clone();
                                let project_context = app.project_context.clone();
                                let tx = events.sender();
                                let mr_iid = diff_view.mr_iid;
                                let mr_iid_str = mr_iid.to_string();
                                tokio::spawn(async move {
                                    let is_github = client
                                        .as_ref()
                                        .is_some_and(|client| client.kind().is_github());

                                    let program = if is_github { "gh" } else { "glab" };
                                    let (entity, sub) = if is_github {
                                        ("pr", "diff")
                                    } else {
                                        ("mr", "diff")
                                    };
                                    let cmd_args = vec![
                                        entity.to_string(),
                                        sub.to_string(),
                                        mr_iid_str.clone(),
                                    ];
                                    let status_msg = format!(
                                        "Fetching Diff: {} {}",
                                        program,
                                        cmd_args.join(" ")
                                    );
                                    let _ = tx.send(Event::CommandStarted(status_msg));

                                    let mut cmd = tokio::process::Command::new(program);
                                    cmd.args(&cmd_args);

                                    let diff_res = cmd.output().await;

                                    let comments = if let Some(ref c) = client {
                                        crate::domain::mr::list_mr_notes(
                                            c,
                                            &project_context,
                                            mr_iid,
                                        )
                                        .await
                                        .unwrap_or_default()
                                    } else {
                                        vec![]
                                    };

                                    if let Ok(output) = diff_res {
                                        if output.status.success() {
                                            let raw_diff = String::from_utf8_lossy(&output.stdout)
                                                .into_owned();
                                            let _ = tx.send(Event::DiffFetched {
                                                mr_iid,
                                                raw_diff,
                                                comments,
                                            });
                                        }
                                    }
                                });
                            }
                        }
                        Err(err) => {
                            app.show_error(err);
                        }
                    }
                }
                Event::Mouse(mouse_event) => {
                    handle_mouse_event(&mut app, &mouse_event);
                }
                Event::Key(key_event) => {
                    if key_event.code == KeyCode::Char('c')
                        && key_event
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                    {
                        app.quit();
                        continue;
                    }

                    if keybinding_matches(&app.config.keybindings.global.quit, &key_event)
                        && app.text_input.is_none()
                        && app.edit_menu.is_none()
                        && app.selector.is_none()
                        && !app.show_help
                        && !app.focus_column_checklist
                    {
                        app.quit();
                        continue;
                    }

                    if handle_confirm_popup(&mut app, &key_event, &mut terminal, events.sender())
                        || handle_help_keybinding(&mut app, &key_event)
                        || handle_help_overlay(&mut app, &key_event)
                        || handle_switch_repo(&mut app, &key_event)
                        || handle_refresh(&mut app, &key_event, &mut last_refresh, events.sender())
                        || handle_date_picker(&mut app, &key_event, &mut terminal, events.sender())
                    {
                        continue;
                    }

                    if let Some(mut text_input) = app.text_input.take() {
                        if key_event
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                            && key_event.code == KeyCode::Char('e')
                        {
                            if let Some(new_val) = edit_in_editor(&text_input.value, &mut terminal)
                            {
                                text_input.value = new_val.clone();
                                text_input.cursor_idx = new_val.len();
                            }
                            app.text_input = Some(text_input);
                            continue;
                        }
                        match key_event.code {
                            KeyCode::Esc => {
                                // Cancel
                            }
                            KeyCode::Backspace => {
                                if text_input.cursor_idx > 0 {
                                    text_input.value.remove(text_input.cursor_idx - 1);
                                    text_input.cursor_idx -= 1;
                                }
                                app.text_input = Some(text_input);
                            }
                            KeyCode::Delete => {
                                if text_input.cursor_idx < text_input.value.len() {
                                    text_input.value.remove(text_input.cursor_idx);
                                }
                                app.text_input = Some(text_input);
                            }
                            KeyCode::Left => {
                                if text_input.cursor_idx > 0 {
                                    text_input.cursor_idx -= 1;
                                }
                                app.text_input = Some(text_input);
                            }
                            KeyCode::Right => {
                                if text_input.cursor_idx < text_input.value.len() {
                                    text_input.cursor_idx += 1;
                                }
                                app.text_input = Some(text_input);
                            }
                            KeyCode::Char(c) => {
                                text_input.value.insert(text_input.cursor_idx, c);
                                text_input.cursor_idx += 1;
                                app.text_input = Some(text_input);
                            }
                            KeyCode::Enter => {
                                let value = text_input.value.clone();
                                match text_input.action {
                                    crate::app::TextInputAction::EditPageSize => {
                                        if let Ok(new_size) = value.trim().parse::<usize>() {
                                            if new_size > 0 {
                                                app.config.page_size = new_size;
                                                app.page_size = new_size;
                                                if let Some(ref mut client) = app.gitlab_client {
                                                    client.page_size = new_size;
                                                }
                                                if let Some(client) = app.gitlab_client.clone() {
                                                    app.start_loading_tab(app.active_tab);
                                                    spawn_refresh_active_tab(
                                                        &client,
                                                        &app.project_context,
                                                        app.active_tab,
                                                        events.sender(),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    crate::app::TextInputAction::EditField {
                                        entity_iid,
                                        entity_type,
                                        field_type,
                                    } => {
                                        let active_tab = app.active_tab;
                                        apply_field_text_change(
                                            &mut app,
                                            &entity_type,
                                            entity_iid,
                                            &field_type,
                                            value,
                                            &mut terminal,
                                            events.sender(),
                                            active_tab,
                                        );
                                        rebuild_edit_menu(&mut app, &entity_type, entity_iid);
                                    }
                                    crate::app::TextInputAction::CreateIssue => {
                                        if !value.trim().is_empty() {
                                            let client = app.gitlab_client.clone().unwrap();
                                            let project = app.project_context.clone();
                                            let title = value.clone();
                                            let tx = events.sender();
                                            let tab = app.active_tab;
                                            tokio::spawn(async move {
                                                match client
                                                    .create_issue(
                                                        &project, &title, "", "", "", "", "", "",
                                                    )
                                                    .await
                                                {
                                                    Ok(_) => {
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            tab,
                                                            Ok(()),
                                                        ));
                                                    }
                                                    Err(e) => {
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            tab,
                                                            Err(e.to_string()),
                                                        ));
                                                    }
                                                }
                                            });
                                        }
                                    }
                                    crate::app::TextInputAction::AddReviewComment {
                                        mr_iid,
                                        file_path,
                                        line_num,
                                        old_line_num,
                                        end_line_num,
                                        end_old_line_num,
                                    } => {
                                        if !value.trim().is_empty() {
                                            if app.in_review_mode {
                                                app.draft_comments.push(crate::app::DraftComment {
                                                    file_path,
                                                    line_num,
                                                    old_line_num,
                                                    end_line_num,
                                                    end_old_line_num,
                                                    body: value,
                                                });
                                                app.status_message = Some(format!(
                                                    "Added draft comment. ({} pending)",
                                                    app.draft_comments.len()
                                                ));
                                            } else {
                                                let client = app.gitlab_client.clone().unwrap();
                                                let project = app.project_context.clone();
                                                let body = value;
                                                let tx = events.sender();
                                                let tab = app.active_tab;
                                                tokio::spawn(async move {
                                                    match client
                                                        .add_mr_comment(
                                                            &project,
                                                            mr_iid,
                                                            &body,
                                                            Some(&file_path),
                                                            line_num.map(|v| v as u64),
                                                            old_line_num.map(|v| v as u64),
                                                        )
                                                        .await
                                                    {
                                                        Ok(_) => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    tab,
                                                                    Ok(()),
                                                                ));
                                                        }
                                                        Err(e) => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    tab,
                                                                    Err(e.to_string()),
                                                                ));
                                                        }
                                                    }
                                                });
                                            }
                                        }
                                    }
                                    crate::app::TextInputAction::EnterPipelineId => {
                                        if let Ok(pipeline_id) = value.trim().parse::<u64>() {
                                            if let Some(client) = &app.gitlab_client {
                                                app.loading_tabs.insert(app::Tab::Jobs);
                                                let client_clone = client.clone();
                                                let project_context = app.project_context.clone();
                                                let tx = events.sender();
                                                tokio::spawn(async move {
                                                    match domain::pipelines::list_pipeline_jobs(
                                                        &client_clone,
                                                        &project_context,
                                                        pipeline_id,
                                                    )
                                                    .await
                                                    {
                                                        Ok(jobs) => {
                                                            let _ = tx.send(Event::JobsTabFetched(
                                                                pipeline_id,
                                                                jobs,
                                                            ));
                                                        }
                                                        Err(e) => {
                                                            let _ = tx.send(Event::FetchFailed(app::Tab::Jobs, format!("Failed to fetch jobs for pipeline {}: {}", pipeline_id, e)));
                                                        }
                                                    }
                                                });
                                            }
                                        } else {
                                            app.error_message =
                                                Some("Invalid pipeline ID".to_string());
                                        }
                                    }
                                    crate::app::TextInputAction::CreateRelease => {
                                        if !value.trim().is_empty() {
                                            let tag_name = value.trim().to_string();
                                            let tx = events.sender();
                                            let is_github = app.is_github();
                                            let program = if is_github { "gh" } else { "glab" };
                                            let _ = tx.send(Event::CommandStarted(format!(
                                                "Creating Release: {} release create {}",
                                                program, tag_name
                                            )));
                                            let active_tab = app.active_tab;
                                            tokio::spawn(async move {
                                                let last_tag = if let Ok(output) =
                                                    tokio::process::Command::new("git")
                                                        .args(["describe", "--tags", "--abbrev=0"])
                                                        .output()
                                                        .await
                                                {
                                                    let t = String::from_utf8_lossy(&output.stdout)
                                                        .trim()
                                                        .to_string();
                                                    if t.is_empty() { None } else { Some(t) }
                                                } else {
                                                    None
                                                };

                                                let log_args = if let Some(ref tag) = last_tag {
                                                    vec![
                                                        "log".to_string(),
                                                        format!("{}..HEAD", tag),
                                                        "--oneline".to_string(),
                                                    ]
                                                } else {
                                                    vec!["log".to_string(), "--oneline".to_string()]
                                                };

                                                let commits = if let Ok(output) =
                                                    tokio::process::Command::new("git")
                                                        .args(&log_args)
                                                        .output()
                                                        .await
                                                {
                                                    String::from_utf8_lossy(&output.stdout)
                                                        .lines()
                                                        .map(|line| {
                                                            let parts: Vec<&str> =
                                                                line.splitn(2, ' ').collect();
                                                            if parts.len() == 2 {
                                                                format!(
                                                                    "- {} ({})",
                                                                    parts[1], parts[0]
                                                                )
                                                            } else {
                                                                format!("- {}", line)
                                                            }
                                                        })
                                                        .collect::<Vec<_>>()
                                                        .join("\n")
                                                } else {
                                                    "".to_string()
                                                };

                                                let title_range = if let Some(ref tag) = last_tag {
                                                    format!("Changes since {}", tag)
                                                } else {
                                                    "All Changes".to_string()
                                                };

                                                let changelog = format!(
                                                    "## Release Notes\n\n### {}\n\n{}\n",
                                                    title_range,
                                                    if commits.is_empty() {
                                                        "- No changes found".to_string()
                                                    } else {
                                                        commits
                                                    }
                                                );

                                                let temp_path = std::env::temp_dir().join(format!(
                                                    "glab-tui-release-{}.md",
                                                    tag_name
                                                ));
                                                if let Ok(_) =
                                                    std::fs::write(&temp_path, &changelog)
                                                {
                                                    let temp_str =
                                                        temp_path.to_string_lossy().to_string();

                                                    let program =
                                                        if is_github { "gh" } else { "glab" };
                                                    let args = [
                                                        "release", "create", &tag_name, "-F",
                                                        &temp_str,
                                                    ];

                                                    let mut cmd =
                                                        tokio::process::Command::new(program);
                                                    cmd.args(&args);

                                                    match cmd.output().await {
                                                        Ok(output) => {
                                                            let _ =
                                                                std::fs::remove_file(&temp_path);
                                                            if output.status.success() {
                                                                let _ = tx.send(
                                                                    Event::CommandCompleted(
                                                                        active_tab,
                                                                        Ok(()),
                                                                    ),
                                                                );
                                                            } else {
                                                                let err_msg =
                                                                    String::from_utf8_lossy(
                                                                        &output.stderr,
                                                                    )
                                                                    .trim()
                                                                    .to_string();
                                                                let _ = tx.send(
                                                                    Event::CommandCompleted(
                                                                        active_tab,
                                                                        Err(format!(
                                                                            "Command failed: {}",
                                                                            err_msg
                                                                        )),
                                                                    ),
                                                                );
                                                            }
                                                        }
                                                        Err(e) => {
                                                            let _ =
                                                                std::fs::remove_file(&temp_path);
                                                            let _ = tx.send(Event::CommandCompleted(
                                                                active_tab,
                                                                Err(format!("Failed to execute command: {}", e)),
                                                            ));
                                                        }
                                                    }
                                                } else {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        active_tab,
                                                        Err("Failed to write temporary changelog file".to_string()),
                                                    ));
                                                }
                                            });
                                        }
                                    }
                                    crate::app::TextInputAction::CreateBranch(ref ref_branch) => {
                                        if !value.trim().is_empty() {
                                            let branch_name = value.trim().to_string();
                                            let client = app.gitlab_client.clone();
                                            let project_context = app.project_context.clone();
                                            let ref_branch = ref_branch.clone();
                                            let tx = events.sender();
                                            let _ = tx.send(Event::CommandStarted(format!(
                                                "Creating branch: {} from {}",
                                                branch_name, ref_branch
                                            )));
                                            tokio::spawn(async move {
                                                if let Some(client) = client {
                                                    match crate::domain::branches::create_branch(
                                                        &client,
                                                        &project_context,
                                                        &branch_name,
                                                        &ref_branch,
                                                    )
                                                    .await
                                                    {
                                                        Ok(_) => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::Branches,
                                                                    Ok(()),
                                                                ));
                                                        }
                                                        Err(e) => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::Branches,
                                                                    Err(format!("Failed: {}", e)),
                                                                ));
                                                        }
                                                    }
                                                }
                                            });
                                        }
                                    }
                                    crate::app::TextInputAction::CreateMilestone => {
                                        if !value.trim().is_empty() {
                                            let title = value.trim().to_string();
                                            let client = app.gitlab_client.clone().unwrap();
                                            let project = app.project_context.clone();
                                            let tx = events.sender();
                                            let tab = app.active_tab;
                                            tokio::spawn(async move {
                                                match client
                                                    .create_milestone(
                                                        &project, &title, "", None, None,
                                                    )
                                                    .await
                                                {
                                                    Ok(_) => {
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            tab,
                                                            Ok(()),
                                                        ));
                                                    }
                                                    Err(e) => {
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            tab,
                                                            Err(e.to_string()),
                                                        ));
                                                    }
                                                }
                                            });
                                        }
                                    }
                                    crate::app::TextInputAction::ReplyToComment {
                                        mr_iid,
                                        comment_id,
                                        ref discussion_id,
                                    } => {
                                        if !value.trim().is_empty() {
                                            let client = app.gitlab_client.clone();
                                            let project_context = app.project_context.clone();
                                            let tx = events.sender();
                                            let is_github =
                                                client.as_ref().map_or(false, |c| c.is_github);
                                            let discussion_id_clone = discussion_id.clone();
                                            let value_clone = value.clone();

                                            let _ = tx.send(Event::CommandStarted(format!(
                                                "Replying to comment ID {} in MR #{}",
                                                comment_id, mr_iid
                                            )));

                                            tokio::spawn(async move {
                                                if let Some(client) = client {
                                                    let output = if is_github {
                                                        let payload = serde_json::json!({
                                                            "body": value_clone,
                                                            "in_reply_to": comment_id,
                                                        });
                                                        let temp_path =
                                                            std::env::temp_dir().join(format!(
                                                                "glab-tui-reply-{}.json",
                                                                comment_id
                                                            ));
                                                        let _ = std::fs::write(
                                                            &temp_path,
                                                            serde_json::to_string(&payload)
                                                                .unwrap(),
                                                        );
                                                        let temp_str =
                                                            temp_path.to_string_lossy().to_string();

                                                        let res = tokio::process::Command::new(
                                                            "gh",
                                                        )
                                                        .args([
                                                            "api",
                                                            &format!(
                                                                "repos/{}/pulls/{}/comments",
                                                                project_context, mr_iid
                                                            ),
                                                            "--input",
                                                            &temp_str,
                                                            "-X",
                                                            "POST",
                                                        ])
                                                        .output()
                                                        .await;
                                                        let _ = std::fs::remove_file(&temp_path);
                                                        res
                                                    } else {
                                                        let encoded_path =
                                                            project_context.replace("/", "%2F");
                                                        let payload = serde_json::json!({
                                                            "body": value_clone,
                                                        });
                                                        let temp_path =
                                                            std::env::temp_dir().join(format!(
                                                                "glab-tui-reply-{}.json",
                                                                comment_id
                                                            ));
                                                        let _ = std::fs::write(
                                                            &temp_path,
                                                            serde_json::to_string(&payload)
                                                                .unwrap(),
                                                        );
                                                        let temp_str =
                                                            temp_path.to_string_lossy().to_string();

                                                        let res = tokio::process::Command::new("glab")
                                                            .args([
                                                                "api",
                                                                &format!(
                                                                    "projects/{}/merge_requests/{}/discussions/{}/notes",
                                                                    encoded_path, mr_iid, discussion_id_clone
                                                                ),
                                                                "--input",
                                                                &temp_str,
                                                                "-X",
                                                                "POST"
                                                            ])
                                                            .output()
                                                            .await;
                                                        let _ = std::fs::remove_file(&temp_path);
                                                        res
                                                    };

                                                    match output {
                                                        Ok(out) if out.status.success() => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::MergeRequests,
                                                                    Ok(()),
                                                                ));
                                                        }
                                                        Ok(out) => {
                                                            let err = String::from_utf8_lossy(
                                                                &out.stderr,
                                                            )
                                                            .trim()
                                                            .to_string();
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::MergeRequests,
                                                                    Err(err),
                                                                ));
                                                        }
                                                        Err(e) => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::MergeRequests,
                                                                    Err(e.to_string()),
                                                                ));
                                                        }
                                                    }
                                                }
                                            });
                                        }
                                    }
                                    crate::app::TextInputAction::SubmitReviewFinal {
                                        mr_iid,
                                        status,
                                    } => {
                                        let is_github = app.is_github();
                                        let tx = events.sender();
                                        let comments = app.draft_comments.clone();
                                        app.draft_comments.clear();
                                        app.in_review_mode = false;

                                        let project_context = app.project_context.clone();
                                        let status_clone = status.clone();
                                        let value_clone = value.clone();

                                        tokio::spawn(async move {
                                            if is_github {
                                                let github_event = match status_clone.as_str() {
                                                    "Approve" => "APPROVE",
                                                    "Request Changes" => "REQUEST_CHANGES",
                                                    _ => "COMMENT",
                                                };
                                                let mut json_comments = serde_json::json!([]);
                                                if let Some(arr) = json_comments.as_array_mut() {
                                                    for comment in &comments {
                                                        let line = comment
                                                            .line_num
                                                            .or(comment.old_line_num)
                                                            .unwrap_or(1);
                                                        let side = if comment.old_line_num.is_some()
                                                        {
                                                            "LEFT"
                                                        } else {
                                                            "RIGHT"
                                                        };
                                                        let mut obj = serde_json::json!({
                                                            "path": comment.file_path,
                                                            "line": line,
                                                            "side": side,
                                                            "body": comment.body,
                                                        });
                                                        // Add multi-line range if applicable
                                                        if let Some(end_l) = comment.end_line_num {
                                                            if let Some(start_l) = comment.line_num
                                                            {
                                                                if end_l != start_l {
                                                                    if let Some(obj_map) =
                                                                        obj.as_object_mut()
                                                                    {
                                                                        obj_map.insert(
                                                                            "start_line"
                                                                                .to_string(),
                                                                            serde_json::json!(
                                                                                start_l.min(end_l)
                                                                            ),
                                                                        );
                                                                        obj_map.insert(
                                                                            "start_side"
                                                                                .to_string(),
                                                                            serde_json::json!(
                                                                                "RIGHT"
                                                                            ),
                                                                        );
                                                                        obj_map.insert(
                                                                            "line".to_string(),
                                                                            serde_json::json!(
                                                                                start_l.max(end_l)
                                                                            ),
                                                                        );
                                                                        obj_map.insert(
                                                                            "side".to_string(),
                                                                            serde_json::json!(
                                                                                "RIGHT"
                                                                            ),
                                                                        );
                                                                    }
                                                                }
                                                            }
                                                        } else if let Some(end_o) =
                                                            comment.end_old_line_num
                                                        {
                                                            if let Some(oln) = comment.old_line_num
                                                            {
                                                                if end_o != oln {
                                                                    if let Some(obj_map) =
                                                                        obj.as_object_mut()
                                                                    {
                                                                        obj_map.insert(
                                                                            "start_line"
                                                                                .to_string(),
                                                                            serde_json::json!(
                                                                                oln.min(end_o)
                                                                            ),
                                                                        );
                                                                        obj_map.insert(
                                                                            "start_side"
                                                                                .to_string(),
                                                                            serde_json::json!(
                                                                                "LEFT"
                                                                            ),
                                                                        );
                                                                        obj_map.insert(
                                                                            "line".to_string(),
                                                                            serde_json::json!(
                                                                                oln.max(end_o)
                                                                            ),
                                                                        );
                                                                        obj_map.insert(
                                                                            "side".to_string(),
                                                                            serde_json::json!(
                                                                                "LEFT"
                                                                            ),
                                                                        );
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        arr.push(obj);
                                                    }
                                                }
                                                let payload = serde_json::json!({
                                                    "body": value_clone,
                                                    "event": github_event,
                                                    "comments": json_comments,
                                                });
                                                let temp_path = std::env::temp_dir().join(format!(
                                                    "glab-tui-review-{}.json",
                                                    mr_iid
                                                ));
                                                if let Ok(_) = std::fs::write(
                                                    &temp_path,
                                                    serde_json::to_string(&payload).unwrap(),
                                                ) {
                                                    let temp_str =
                                                        temp_path.to_string_lossy().to_string();
                                                    let _ =
                                                        tx.send(Event::CommandStarted(format!(
                                                            "SUBMITTING REVIEW: gh api repos/{}/pulls/{}/reviews",
                                                            project_context, mr_iid
                                                        )));
                                                    let output = tokio::process::Command::new("gh")
                                                        .args([
                                                            "api",
                                                            &format!(
                                                                "repos/{}/pulls/{}/reviews",
                                                                project_context, mr_iid
                                                            ),
                                                            "--input",
                                                            &temp_str,
                                                        ])
                                                        .output()
                                                        .await;
                                                    let _ = std::fs::remove_file(&temp_path);
                                                    match output {
                                                        Ok(out) if out.status.success() => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::MergeRequests,
                                                                    Ok(()),
                                                                ));
                                                        }
                                                        Ok(out) => {
                                                            let err = String::from_utf8_lossy(
                                                                &out.stderr,
                                                            )
                                                            .trim()
                                                            .to_string();
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::MergeRequests,
                                                                    Err(format!(
                                                                        "Submit review failed: {}",
                                                                        err
                                                                    )),
                                                                ));
                                                        }
                                                        Err(e) => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::MergeRequests,
                                                                    Err(format!(
                                                                        "Failed to run gh: {}",
                                                                        e
                                                                    )),
                                                                ));
                                                        }
                                                    }
                                                }
                                            } else {
                                                let _ = tx.send(Event::CommandStarted(format!(
                                                    "SUBMITTING REVIEW: glab mr approve {}; glab mr note create {}",
                                                    mr_iid, mr_iid
                                                )));
                                                let encoded_path =
                                                    project_context.replace("/", "%2F");
                                                let mut success = true;
                                                let mut err_msg = String::new();

                                                // Fetch MR details to get base_sha, start_sha, and head_sha
                                                let mr_output =
                                                    tokio::process::Command::new("glab")
                                                        .args([
                                                            "api",
                                                            &format!(
                                                                "projects/{}/merge_requests/{}",
                                                                encoded_path, mr_iid
                                                            ),
                                                        ])
                                                        .output()
                                                        .await;

                                                let (base_sha, start_sha, head_sha) =
                                                    if let Ok(out) = mr_output {
                                                        if out.status.success() {
                                                            if let Ok(v) = serde_json::from_slice::<
                                                                serde_json::Value,
                                                            >(
                                                                &out.stdout
                                                            ) {
                                                                let base =
                                                                    v["diff_refs"]["base_sha"]
                                                                        .as_str()
                                                                        .map(|s| s.to_string());
                                                                let start =
                                                                    v["diff_refs"]["start_sha"]
                                                                        .as_str()
                                                                        .map(|s| s.to_string());
                                                                let head =
                                                                    v["diff_refs"]["head_sha"]
                                                                        .as_str()
                                                                        .map(|s| s.to_string());
                                                                (base, start, head)
                                                            } else {
                                                                (None, None, None)
                                                            }
                                                        } else {
                                                            (None, None, None)
                                                        }
                                                    } else {
                                                        (None, None, None)
                                                    };

                                                for comment in &comments {
                                                    let mut position = serde_json::json!({
                                                        "position_type": "text",
                                                        "new_path": comment.file_path,
                                                    });
                                                    if let Some(ref base) = base_sha {
                                                        position["base_sha"] =
                                                            serde_json::json!(base);
                                                    }
                                                    if let Some(ref start) = start_sha {
                                                        position["start_sha"] =
                                                            serde_json::json!(start);
                                                    }
                                                    if let Some(ref head) = head_sha {
                                                        position["head_sha"] =
                                                            serde_json::json!(head);
                                                    }
                                                    if let Some(line_num) = comment.line_num {
                                                        position["new_line"] =
                                                            serde_json::json!(line_num);
                                                    }
                                                    if let Some(old_line_num) = comment.old_line_num
                                                    {
                                                        position["old_line"] =
                                                            serde_json::json!(old_line_num);
                                                        position["old_path"] =
                                                            serde_json::json!(comment.file_path);
                                                    }

                                                    // Multi-line range for GitLab
                                                    if let Some(end_l) = comment.end_line_num {
                                                        if let Some(start_l) = comment.line_num {
                                                            if end_l != start_l {
                                                                let line_range = serde_json::json!({
                                                                    "start": {"line_code": "", "type": "new_line"},
                                                                    "end": {"line_code": "", "type": "new_line"},
                                                                });
                                                                if let Some(lr) =
                                                                    line_range.as_object()
                                                                {
                                                                    position["line_range"] = serde_json::json!({
                                                                        "start": {
                                                                            "line_code": "",
                                                                            "type": "new_line",
                                                                            "new_line": start_l.min(end_l),
                                                                        },
                                                                        "end": {
                                                                            "line_code": "",
                                                                            "type": "new_line",
                                                                            "new_line": start_l.max(end_l),
                                                                        },
                                                                    });
                                                                }
                                                            }
                                                        }
                                                    } else if let Some(end_o) =
                                                        comment.end_old_line_num
                                                    {
                                                        if let Some(start_o) = comment.old_line_num
                                                        {
                                                            if end_o != start_o {
                                                                let line_range = serde_json::json!({
                                                                    "start": {"line_code": "", "type": "old_line"},
                                                                    "end": {"line_code": "", "type": "old_line"},
                                                                });
                                                                if let Some(lr) =
                                                                    line_range.as_object()
                                                                {
                                                                    position["line_range"] = serde_json::json!({
                                                                        "start": {
                                                                            "line_code": "",
                                                                            "type": "old_line",
                                                                            "old_line": start_o.min(end_o),
                                                                        },
                                                                        "end": {
                                                                            "line_code": "",
                                                                            "type": "old_line",
                                                                            "old_line": start_o.max(end_o),
                                                                        },
                                                                    });
                                                                }
                                                            }
                                                        }
                                                    }

                                                    let draft_payload = serde_json::json!({
                                                        "note": comment.body,
                                                        "position": position,
                                                    });
                                                    let temp_path = std::env::temp_dir().join(
                                                        format!("glab-tui-draft-{}.json", mr_iid),
                                                    );
                                                    if let Ok(_) = std::fs::write(
                                                        &temp_path,
                                                        serde_json::to_string(&draft_payload)
                                                            .unwrap(),
                                                    ) {
                                                        let temp_str =
                                                            temp_path.to_string_lossy().to_string();
                                                        let output = tokio::process::Command::new("glab")
                                                            .args([
                                                                "api",
                                                                &format!("projects/{}/merge_requests/{}/draft_notes", encoded_path, mr_iid),
                                                                "--input",
                                                                &temp_str,
                                                                "-X",
                                                                "POST",
                                                            ])
                                                            .output()
                                                            .await;
                                                        let _ = std::fs::remove_file(&temp_path);
                                                        if let Ok(out) = output {
                                                            if !out.status.success() {
                                                                success = false;
                                                                err_msg = String::from_utf8_lossy(
                                                                    &out.stderr,
                                                                )
                                                                .trim()
                                                                .to_string();
                                                                break;
                                                            }
                                                        } else {
                                                            success = false;
                                                            err_msg = "Failed to run glab api"
                                                                .to_string();
                                                            break;
                                                        }
                                                    }
                                                }

                                                if success {
                                                    let publish_success = if !comments.is_empty() {
                                                        let publish_output = tokio::process::Command::new("glab")
                                                            .args([
                                                                "api",
                                                                &format!("projects/{}/merge_requests/{}/draft_notes/bulk_publish", encoded_path, mr_iid),
                                                                "-X",
                                                                "POST",
                                                            ])
                                                            .output()
                                                            .await;
                                                        match publish_output {
                                                            Ok(out) if out.status.success() => true,
                                                            Ok(out) => {
                                                                err_msg = String::from_utf8_lossy(
                                                                    &out.stderr,
                                                                )
                                                                .trim()
                                                                .to_string();
                                                                false
                                                            }
                                                            Err(e) => {
                                                                err_msg = format!(
                                                                    "Failed to publish draft notes: {}",
                                                                    e
                                                                );
                                                                false
                                                            }
                                                        }
                                                    } else {
                                                        true
                                                    };

                                                    if publish_success {
                                                        if status_clone == "Approve" {
                                                            let approve_output =
                                                                tokio::process::Command::new(
                                                                    "glab",
                                                                )
                                                                .args([
                                                                    "mr",
                                                                    "approve",
                                                                    &mr_iid.to_string(),
                                                                ])
                                                                .output()
                                                                .await;
                                                            if let Ok(out) = approve_output {
                                                                if !out.status.success() {
                                                                    let approval_err =
                                                                        String::from_utf8_lossy(
                                                                            &out.stderr,
                                                                        )
                                                                        .trim()
                                                                        .to_string();
                                                                    let _ = tx.send(Event::FetchFailed(
                                                                        app::Tab::MergeRequests,
                                                                        format!("MR approval failed: {}", approval_err),
                                                                    ));
                                                                }
                                                            }
                                                        }

                                                        if !value_clone.trim().is_empty() {
                                                            let _ = tokio::process::Command::new(
                                                                "glab",
                                                            )
                                                            .args([
                                                                "mr",
                                                                "note",
                                                                "create",
                                                                &mr_iid.to_string(),
                                                                "-m",
                                                                &value_clone,
                                                            ])
                                                            .output()
                                                            .await;
                                                        }

                                                        let _ = tx.send(Event::CommandCompleted(
                                                            app::Tab::MergeRequests,
                                                            Ok(()),
                                                        ));
                                                    } else {
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            app::Tab::MergeRequests,
                                                            Err(format!(
                                                                "Bulk publish failed: {}",
                                                                err_msg
                                                            )),
                                                        ));
                                                    }
                                                } else {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        app::Tab::MergeRequests,
                                                        Err(format!(
                                                            "Draft notes creation failed: {}",
                                                            err_msg
                                                        )),
                                                    ));
                                                }
                                            }
                                        });
                                    }
                                    crate::app::TextInputAction::EditNewField { field_idx } => {
                                        // Write the value directly into the edit_menu fields
                                        // (no CLI call — iid==0 means this entity is not yet created)
                                        if let Some(ref mut menu) = app.edit_menu {
                                            if let Some(field) = menu.fields.get_mut(field_idx) {
                                                field.value = value.clone();
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {
                                app.text_input = Some(text_input);
                            }
                        }
                        continue;
                    }

                    if let Some(mut selector) = app.selector.take() {
                        if selector.is_filtering {
                            match key_event.code {
                                KeyCode::Enter | KeyCode::Esc => {
                                    selector.is_filtering = false;
                                    app.selector = Some(selector);
                                }
                                KeyCode::Backspace => {
                                    selector.search_query.pop();
                                    selector.cursor_idx = 0;
                                    selector.state.select(Some(0));
                                    app.selector = Some(selector);
                                }
                                KeyCode::Char(c) => {
                                    selector.search_query.push(c);
                                    selector.cursor_idx = 0;
                                    selector.state.select(Some(0));
                                    app.selector = Some(selector);
                                }
                                _ => {
                                    app.selector = Some(selector);
                                }
                            }
                        } else {
                            let filtered_items = selector.get_filtered_items();
                            match key_event.code {
                                KeyCode::Esc => {
                                    // Close selector, go back to EditMenu (it is already in app.edit_menu)
                                }
                                KeyCode::Char('f') | KeyCode::Char('/') | KeyCode::Char('i') => {
                                    let has_filter = selector.field_type != "comment_action_select"
                                        && selector.field_type != "review_submit_status"
                                        && selector.field_type != "merge_options";
                                    if has_filter {
                                        selector.is_filtering = true;
                                    }
                                    app.selector = Some(selector);
                                }
                                KeyCode::Char('j') | KeyCode::Down => {
                                    if !filtered_items.is_empty() {
                                        selector.cursor_idx =
                                            (selector.cursor_idx + 1) % filtered_items.len();
                                        selector.state.select(Some(selector.cursor_idx));
                                    }
                                    app.selector = Some(selector);
                                }
                                KeyCode::Char('k') | KeyCode::Up => {
                                    if !filtered_items.is_empty() {
                                        if selector.cursor_idx == 0 {
                                            selector.cursor_idx = filtered_items.len() - 1;
                                        } else {
                                            selector.cursor_idx -= 1;
                                        }
                                        selector.state.select(Some(selector.cursor_idx));
                                    }
                                    app.selector = Some(selector);
                                }
                                KeyCode::Char(' ') => {
                                    if !filtered_items.is_empty() {
                                        let item = &filtered_items[selector.cursor_idx];
                                        if item.starts_with("+ Create \"") {
                                            let clean_val =
                                                selector.search_query.trim().to_string();
                                            if !clean_val.is_empty() {
                                                if selector.multi_select {
                                                    if selector.selected_items.contains(&clean_val)
                                                    {
                                                        selector.selected_items.remove(&clean_val);
                                                    } else {
                                                        selector.selected_items.insert(clean_val);
                                                    }
                                                } else {
                                                    selector.selected_items.clear();
                                                    selector.selected_items.insert(clean_val);
                                                }
                                            }
                                        } else {
                                            if selector.multi_select {
                                                if selector.selected_items.contains(item) {
                                                    selector.selected_items.remove(item);
                                                } else {
                                                    selector.selected_items.insert(item.clone());
                                                }
                                            } else {
                                                if selector.selected_items.contains(item) {
                                                    selector.selected_items.remove(item);
                                                } else {
                                                    selector.selected_items.clear();
                                                    selector.selected_items.insert(item.clone());
                                                }
                                            }
                                        }
                                    }
                                    app.selector = Some(selector);
                                }
                                KeyCode::Enter => {
                                    let field_type = selector.field_type.clone();
                                    if field_type == "global_search" {
                                        let filtered_items = selector.get_filtered_items();
                                        let selected_val = if !selector.selected_items.is_empty() {
                                            selector.selected_items.iter().next().cloned()
                                        } else if !filtered_items.is_empty() {
                                            Some(filtered_items[selector.cursor_idx].clone())
                                        } else {
                                            None
                                        };
                                        if let Some(val) = selected_val {
                                            if val.starts_with("Issue #") {
                                                if let Some(iid_str) = val
                                                    .strip_prefix("Issue #")
                                                    .and_then(|s| s.split(':').next())
                                                {
                                                    if let Ok(iid) = iid_str.parse::<u64>() {
                                                        app.active_tab = crate::app::Tab::Issues;
                                                        if let Some(idx) = app
                                                            .issues
                                                            .items
                                                            .iter()
                                                            .position(|i| i.iid == iid)
                                                        {
                                                            app.issues.state.select(Some(idx));
                                                        }
                                                    }
                                                }
                                            } else if val.starts_with("MR !") {
                                                if let Some(iid_str) = val
                                                    .strip_prefix("MR !")
                                                    .and_then(|s| s.split(':').next())
                                                {
                                                    if let Ok(iid) = iid_str.parse::<u64>() {
                                                        app.active_tab =
                                                            crate::app::Tab::MergeRequests;
                                                        if let Some(idx) = app
                                                            .mrs
                                                            .items
                                                            .iter()
                                                            .position(|m| m.iid == iid)
                                                        {
                                                            app.mrs.state.select(Some(idx));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        continue;
                                    }
                                    if field_type == "column_filter" {
                                        if let Some((tab, col)) = app.column_filter_context.take() {
                                            app.set_column_filter(
                                                tab,
                                                &col,
                                                selector.selected_items.clone(),
                                            );
                                            app.update_filter_selection();
                                        }
                                        continue;
                                    }
                                    if field_type == "switch_repo" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val =
                                            selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val =
                                                Some(filtered_items[selector.cursor_idx].clone());
                                        }

                                        if let Some(mut path) = selected_val {
                                            if path.starts_with("+ Create \"") {
                                                path = selector.search_query.trim().to_string();
                                            }

                                            let repos_dir = crate::utils::cache::get_repos_dir();
                                            let target_path =
                                                if std::path::Path::new(&path).is_absolute() {
                                                    std::path::PathBuf::from(&path)
                                                } else {
                                                    repos_dir.join(&path)
                                                };

                                            let target_path_str =
                                                target_path.to_string_lossy().into_owned();
                                            if crate::utils::cache::is_git_repo(&target_path_str) {
                                                if std::env::set_current_dir(&target_path).is_ok() {
                                                    crate::utils::cache::add_recent_repo(
                                                        &target_path_str,
                                                    );
                                                    app.config = crate::config::Config::load();
                                                    app.apply_config();
                                                    crate::config::reload_theme();

                                                    if let Ok(context) =
                                                        domain::client::get_project_context().await
                                                    {
                                                        app.project_context = context;
                                                    }
                                                    if let Ok(mut client) =
                                                        domain::client::GitlabClient::new(
                                                            &app.config,
                                                        )
                                                        .await
                                                    {
                                                        client.page_size = app.config.page_size;
                                                        client.api_per_page =
                                                            app.config.api_per_page_clamped();
                                                        client.tx = Some(events.sender());
                                                        client.backend.set_tx(events.sender());
                                                        app.gitlab_client = Some(client.clone());
                                                    } else {
                                                        app.gitlab_client = None;
                                                    }

                                                    app.loaded_tabs.clear();
                                                    app.loading_tabs.clear();
                                                    app.refreshed_tabs.clear();
                                                    app.status_message = None;
                                                    app.issues.items.clear();
                                                    app.mrs.items.clear();
                                                    app.pipelines.items.clear();
                                                    app.runners.items.clear();
                                                    app.releases.items.clear();
                                                    app.todos.items.clear();
                                                    app.milestones.items.clear();
                                                    app.pipeline_jobs.clear();
                                                    app.fetching_pipelines.clear();
                                                    app.edit_menu = None;
                                                    app.selected_milestone_issues = None;
                                                    app.selected_milestone_iid = None;

                                                    let cache = crate::utils::cache::load_cache(
                                                        &app.project_context,
                                                    );
                                                    app.project_cache = cache.clone();
                                                    app.issues.items = cache.issues;
                                                    app.mrs.items = cache.mrs;
                                                    // workflow is #[serde(skip)] — see the
                                                    // comment at the startup cache load.
                                                    crate::fetch::derive_workflow(
                                                        &mut app.mrs.items,
                                                    );
                                                    app.pipelines.items = cache.pipelines;
                                                    app.runners.items = cache.runners;
                                                    app.releases.items = cache.releases;
                                                    app.todos.items = cache.todos;
                                                    app.milestones.items = cache.milestones;
                                                    app.pipeline_jobs = cache.pipeline_jobs;
                                                    app.branches.items = cache.branches;
                                                    app.environments.items = cache.environments;
                                                    app.milestone_issues_cache =
                                                        cache.milestone_issues;
                                                    app.cached_labels = cache.labels;
                                                    app.cached_members = cache.members;

                                                    let has_any_cached =
                                                        !app.issues.items.is_empty()
                                                            || !app.mrs.items.is_empty()
                                                            || !app.pipelines.items.is_empty()
                                                            || !app.runners.items.is_empty()
                                                            || !app.releases.items.is_empty()
                                                            || !app.todos.items.is_empty()
                                                            || !app.milestones.items.is_empty();
                                                    if has_any_cached {
                                                        app.status_message = Some(
                                                            "Loaded from offline cache".to_string(),
                                                        );
                                                    }

                                                    if !app.issues.items.is_empty() {
                                                        app.loaded_tabs.insert(app::Tab::Issues);
                                                    }
                                                    if !app.mrs.items.is_empty() {
                                                        app.loaded_tabs
                                                            .insert(app::Tab::MergeRequests);
                                                    }
                                                    if !app.pipelines.items.is_empty() {
                                                        app.loaded_tabs.insert(app::Tab::Pipelines);
                                                    }
                                                    if !app.runners.items.is_empty() {
                                                        app.loaded_tabs.insert(app::Tab::Runners);
                                                    }
                                                    if !app.releases.items.is_empty() {
                                                        app.loaded_tabs.insert(app::Tab::Releases);
                                                    }
                                                    if !app.todos.items.is_empty() {
                                                        app.loaded_tabs.insert(app::Tab::Todos);
                                                    }
                                                    if !app.milestones.items.is_empty() {
                                                        app.loaded_tabs
                                                            .insert(app::Tab::Milestones);
                                                    }

                                                    app.issues.state.select(
                                                        if app.issues.items.is_empty() {
                                                            None
                                                        } else {
                                                            Some(0)
                                                        },
                                                    );
                                                    app.mrs.state.select(
                                                        if app.mrs.items.is_empty() {
                                                            None
                                                        } else {
                                                            Some(0)
                                                        },
                                                    );
                                                    app.pipelines.state.select(
                                                        if app.pipelines.items.is_empty() {
                                                            None
                                                        } else {
                                                            Some(0)
                                                        },
                                                    );
                                                    app.update_filter_selection();

                                                    if let Some(client) = &app.gitlab_client {
                                                        let has_cached = match app.active_tab {
                                                            app::Tab::Issues => {
                                                                !app.issues.items.is_empty()
                                                            }
                                                            app::Tab::MergeRequests => {
                                                                !app.mrs.items.is_empty()
                                                            }
                                                            app::Tab::Pipelines => {
                                                                !app.pipelines.items.is_empty()
                                                            }
                                                            app::Tab::Runners => {
                                                                !app.runners.items.is_empty()
                                                            }
                                                            app::Tab::Releases => {
                                                                !app.releases.items.is_empty()
                                                            }
                                                            app::Tab::Todos => {
                                                                !app.todos.items.is_empty()
                                                            }
                                                            app::Tab::Milestones => {
                                                                !app.milestones.items.is_empty()
                                                            }
                                                            _ => false,
                                                        };
                                                        if !has_cached {
                                                            app.loading_tabs.insert(app.active_tab);
                                                        }
                                                        spawn_refresh_active_tab(
                                                            client,
                                                            &app.project_context,
                                                            app.active_tab,
                                                            events.sender(),
                                                        );
                                                        spawn_fetch_repo_attributes(
                                                            &client.clone().muted(),
                                                            &app.project_context,
                                                            events.sender(),
                                                        );
                                                    }
                                                } else {
                                                    app.error_message = Some(format!(
                                                        "Could not change directory to: {}",
                                                        path
                                                    ));
                                                }
                                            } else {
                                                app.error_message = Some(format!(
                                                    "Not a valid git repository: {}",
                                                    path
                                                ));
                                            }
                                        }
                                        continue;
                                    }

                                    if field_type == "issue_template_selector" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val =
                                            selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val =
                                                Some(filtered_items[selector.cursor_idx].clone());
                                        }
                                        let choice = selected_val.unwrap_or_default();
                                        let mut desc_val = String::new();
                                        if choice != "None (blank)" {
                                            let templates = list_templates("issue");
                                            if let Some(content) = templates
                                                .iter()
                                                .find(|(n, _)| n == &choice)
                                                .map(|(_, c)| c)
                                            {
                                                desc_val = content.clone();
                                            }
                                        }
                                        if let Some(ref mut menu) = app.edit_menu {
                                            if let Some(f) = menu.fields.iter_mut().find(|f| {
                                                f.label == "Description"
                                                    && f.kind == crate::app::FieldType::Text
                                            }) {
                                                f.value = desc_val.clone();
                                            }
                                        } else {
                                            let is_github = app.is_github();
                                            let fields = crate::entity_editor::issue_fields(
                                                String::new(),
                                                String::new(),
                                                String::new(),
                                                String::new(),
                                                "No".to_string(),
                                                String::new(),
                                                "0".to_string(),
                                                desc_val,
                                                is_github,
                                            );
                                            app.open_edit_menu(crate::app::EditMenu {
                                                title: "Create Issue".to_string(),
                                                fields,
                                                selected_idx: 0,
                                                entity_iid: 0,
                                                entity_kind:
                                                    crate::app::EditEntityKind::CreateIssue,
                                                state: {
                                                    let mut s = ListState::default();
                                                    s.select(Some(0));
                                                    s
                                                },
                                                workflow_inputs: vec![],
                                                cursor_pos: 0,
                                                editing: false,
                                                desc_scroll: 0,
                                            });
                                        }
                                        continue;
                                    }

                                    if field_type == "mr_template_selector" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val =
                                            selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val =
                                                Some(filtered_items[selector.cursor_idx].clone());
                                        }
                                        let choice = selected_val.unwrap_or_default();
                                        let mut desc_val = String::new();
                                        if choice != "None (blank)" {
                                            let templates = list_templates("mr");
                                            if let Some(content) = templates
                                                .iter()
                                                .find(|(n, _)| n == &choice)
                                                .map(|(_, c)| c)
                                            {
                                                desc_val = content.clone();
                                            }
                                        }
                                        if let Some(ref mut menu) = app.edit_menu {
                                            if let Some(f) = menu.fields.iter_mut().find(|f| {
                                                f.label == "Description"
                                                    && f.kind == crate::app::FieldType::Text
                                            }) {
                                                f.value = desc_val.clone();
                                            }
                                        } else {
                                            let is_github = app.is_github();
                                            let target_branch = get_default_branch()
                                                .unwrap_or_else(|| "main".to_string());
                                            let fields = crate::entity_editor::mr_fields(
                                                String::new(),
                                                String::new(),
                                                String::new(),
                                                String::new(),
                                                String::new(),
                                                target_branch,
                                                "Ready".to_string(),
                                                desc_val,
                                                is_github,
                                            );
                                            app.open_edit_menu(crate::app::EditMenu {
                                                title: "Create Merge Request".to_string(),
                                                fields,
                                                selected_idx: 0,
                                                entity_iid: 0,
                                                entity_kind: crate::app::EditEntityKind::CreateMr,
                                                state: {
                                                    let mut s = ListState::default();
                                                    s.select(Some(0));
                                                    s
                                                },
                                                workflow_inputs: vec![],
                                                cursor_pos: 0,
                                                editing: false,
                                                desc_scroll: 0,
                                            });
                                        }
                                        continue;
                                    }

                                    if field_type == "description_action" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val =
                                            selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val =
                                                Some(filtered_items[selector.cursor_idx].clone());
                                        }
                                        let choice = selected_val.unwrap_or_default();
                                        let entity_iid = selector.entity_iid;
                                        let entity_type = selector.entity_type.clone();

                                        if choice == "Open in Editor" {
                                            // Get current description from edit menu or entity data
                                            let current_desc = if entity_iid == 0
                                                || entity_type.starts_with("new_")
                                            {
                                                app.edit_menu
                                                    .as_ref()
                                                    .and_then(|m| {
                                                        m.fields
                                                            .iter()
                                                            .find(|f| f.label == "Description" && f.kind == crate::app::FieldType::Text)
                                                            .map(|f| f.value.clone())
                                                    })
                                                    .unwrap_or_default()
                                            } else if entity_type == "issue"
                                                || entity_type == "edit_issue"
                                            {
                                                app.issues
                                                    .items
                                                    .iter()
                                                    .find(|i| i.iid == entity_iid)
                                                    .and_then(|i| i.description.clone())
                                                    .unwrap_or_default()
                                            } else if entity_type == "milestone"
                                                || entity_type == "edit_milestone"
                                            {
                                                app.milestones
                                                    .items
                                                    .iter()
                                                    .find(|m| m.iid == entity_iid)
                                                    .and_then(|m| m.description.clone())
                                                    .unwrap_or_default()
                                            } else {
                                                app.mrs
                                                    .items
                                                    .iter()
                                                    .find(|m| m.iid == entity_iid)
                                                    .and_then(|m| m.description.clone())
                                                    .unwrap_or_default()
                                            };
                                            if let Some(new_desc) = crate::editor::edit_in_editor(
                                                &current_desc,
                                                &mut terminal,
                                            ) {
                                                if let Some(ref mut menu) = app.edit_menu {
                                                    if let Some(f) =
                                                        menu.fields.iter_mut().find(|f| {
                                                            f.label == "Description"
                                                                && f.kind
                                                                    == crate::app::FieldType::Text
                                                        })
                                                    {
                                                        f.value = new_desc.clone();
                                                    }
                                                }
                                                // Apply the change to the backend
                                                let Some(client) = app.gitlab_client.clone() else {
                                                    continue;
                                                };
                                                let project_path = app.project_context.clone();
                                                let result = if entity_type == "issue"
                                                    || entity_type == "edit_issue"
                                                {
                                                    client
                                                        .update_issue_description(
                                                            &project_path,
                                                            entity_iid,
                                                            &new_desc,
                                                        )
                                                        .await
                                                } else if entity_type == "milestone"
                                                    || entity_type == "edit_milestone"
                                                {
                                                    crate::domain::milestones::update_milestone(
                                                        &client,
                                                        &project_path,
                                                        entity_iid,
                                                        &app.milestones
                                                            .items
                                                            .iter()
                                                            .find(|m| m.iid == entity_iid)
                                                            .map(|m| m.title.clone())
                                                            .unwrap_or_default(),
                                                        &new_desc,
                                                        app.milestones
                                                            .items
                                                            .iter()
                                                            .find(|m| m.iid == entity_iid)
                                                            .and_then(|m| m.start_date.clone())
                                                            .as_deref(),
                                                        app.milestones
                                                            .items
                                                            .iter()
                                                            .find(|m| m.iid == entity_iid)
                                                            .and_then(|m| m.due_date.clone())
                                                            .as_deref(),
                                                    )
                                                    .await
                                                } else {
                                                    client
                                                        .update_mr_description(
                                                            &project_path,
                                                            entity_iid,
                                                            &new_desc,
                                                        )
                                                        .await
                                                };
                                                if let Err(e) = result {
                                                    app.error_message = Some(format!(
                                                        "Failed to update description: {}",
                                                        e
                                                    ));
                                                }
                                            }
                                        } else {
                                            // Edit Inline — open TextInput
                                            let current_val = if entity_iid == 0
                                                || entity_type.starts_with("new_")
                                            {
                                                app.edit_menu
                                                    .as_ref()
                                                    .and_then(|m| {
                                                        m.fields
                                                            .iter()
                                                            .find(|f| f.label == "Description" && f.kind == crate::app::FieldType::Text)
                                                            .map(|f| f.value.clone())
                                                    })
                                                    .unwrap_or_default()
                                            } else if entity_type == "issue"
                                                || entity_type == "edit_issue"
                                            {
                                                app.issues
                                                    .items
                                                    .iter()
                                                    .find(|i| i.iid == entity_iid)
                                                    .and_then(|i| i.description.clone())
                                                    .unwrap_or_default()
                                            } else if entity_type == "milestone"
                                                || entity_type == "edit_milestone"
                                            {
                                                app.milestones
                                                    .items
                                                    .iter()
                                                    .find(|m| m.iid == entity_iid)
                                                    .and_then(|m| m.description.clone())
                                                    .unwrap_or_default()
                                            } else {
                                                app.mrs
                                                    .items
                                                    .iter()
                                                    .find(|m| m.iid == entity_iid)
                                                    .and_then(|m| m.description.clone())
                                                    .unwrap_or_default()
                                            };
                                            let action = if entity_iid == 0
                                                || entity_type.starts_with("new_")
                                            {
                                                crate::app::TextInputAction::EditNewField {
                                                    field_idx: app
                                                        .edit_menu
                                                        .as_ref()
                                                        .and_then(|m| {
                                                            m.fields.iter().position(|f| {
                                                                f.label == "Description"
                                                            })
                                                        })
                                                        .unwrap_or(0),
                                                }
                                            } else {
                                                crate::app::TextInputAction::EditField {
                                                    entity_iid,
                                                    entity_type: entity_type.clone(),
                                                    field_type: "description".to_string(),
                                                }
                                            };
                                            app.text_input = Some(crate::app::TextInput {
                                                title: " Edit Description ".to_string(),
                                                value: current_val.clone(),
                                                cursor_idx: current_val.len(),
                                                action,
                                            });
                                        }
                                        continue;
                                    }

                                    if field_type == "create_mr" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val =
                                            selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val =
                                                Some(filtered_items[selector.cursor_idx].clone());
                                        }

                                        app.selector = None;

                                        let is_github = app.is_github();
                                        let pr_suffix = if is_github {
                                            "Pull Request"
                                        } else {
                                            "Merge Request"
                                        };

                                        let mut title_val = String::new();
                                        let mut labels_val = String::new();
                                        let mut assignees_val = String::new();
                                        let mut milestone_val = String::new();
                                        let mut source_branch_val =
                                            get_current_branch().unwrap_or_default();
                                        let mut description_val = String::new();
                                        let mut issue_iid = 0;

                                        if let Some(item) = selected_val {
                                            if item == "Create blank (No issue)" {
                                                let mr_tmpl =
                                                    get_default_template("mr").unwrap_or_default();
                                                description_val = mr_tmpl;
                                            } else {
                                                let id_val = item.clone();
                                                let parsed_iid = if id_val.starts_with('#') {
                                                    id_val
                                                        .strip_prefix('#')
                                                        .and_then(|s| {
                                                            s.split(|c: char| !c.is_numeric())
                                                                .next()
                                                        })
                                                        .and_then(|s| s.parse::<u64>().ok())
                                                } else {
                                                    id_val.trim().parse::<u64>().ok()
                                                };

                                                if let Some(iid) = parsed_iid {
                                                    if let Some(issue) = app
                                                        .issues
                                                        .items
                                                        .iter()
                                                        .find(|i| i.iid == iid)
                                                    {
                                                        issue_iid = issue.iid;
                                                        title_val = issue.title.clone();
                                                        source_branch_val = format!(
                                                            "{}-{}",
                                                            issue.iid,
                                                            slugify(&issue.title)
                                                        );
                                                        if !issue.labels.is_empty() {
                                                            labels_val = issue.labels.join(", ");
                                                        }
                                                        if !issue.assignees.is_empty() {
                                                            assignees_val = issue
                                                                .assignees
                                                                .iter()
                                                                .map(|a| format!("@{}", a.username))
                                                                .collect::<Vec<_>>()
                                                                .join(", ");
                                                        }
                                                        if let Some(ref m) = issue.milestone {
                                                            milestone_val = m.title.clone();
                                                        }
                                                        if let Some(ref d) = issue.description {
                                                            description_val = format!(
                                                                "Closes #{}\n\n{}",
                                                                issue.iid, d
                                                            );
                                                        } else {
                                                            let mr_tmpl =
                                                                get_default_template("mr")
                                                                    .unwrap_or_default();
                                                            if mr_tmpl.is_empty() {
                                                                description_val = format!(
                                                                    "Closes #{}",
                                                                    issue.iid
                                                                );
                                                            } else {
                                                                description_val = format!(
                                                                    "Closes #{}\n\n{}",
                                                                    issue.iid, mr_tmpl
                                                                );
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        app.open_edit_menu(crate::app::EditMenu {
                                            title: format!("Create {}", pr_suffix),
                                            fields: vec![
                                                crate::app::Field::text("Title", title_val),
                                                crate::app::Field::toggle(
                                                    "Status (Draft/Ready)",
                                                    "Draft".to_string(),
                                                ),
                                                crate::app::Field::ref_field(
                                                    "Source Branch",
                                                    source_branch_val,
                                                ),
                                                crate::app::Field::ref_field(
                                                    "Target Branch",
                                                    get_default_branch()
                                                        .unwrap_or_else(|| "main".to_string()),
                                                ),
                                                crate::app::Field::multi_select(
                                                    "Assignees",
                                                    assignees_val,
                                                ),
                                                crate::app::Field::multi_select(
                                                    "Reviewers",
                                                    String::new(),
                                                ),
                                                crate::app::Field::multi_select(
                                                    "Milestone",
                                                    milestone_val,
                                                ),
                                                crate::app::Field::multi_select(
                                                    "Labels", labels_val,
                                                ),
                                                crate::app::Field::text(
                                                    "Description",
                                                    description_val,
                                                ),
                                            ],
                                            selected_idx: 0,
                                            entity_iid: issue_iid,
                                            entity_kind: crate::app::EditEntityKind::CreateMr,
                                            state: {
                                                let mut s = ListState::default();
                                                s.select(Some(0));
                                                s
                                            },
                                            workflow_inputs: vec![],
                                            cursor_pos: 0,
                                            editing: false,
                                            desc_scroll: 0,
                                        });
                                        continue;
                                    }

                                    if field_type == "review_submit_status" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val =
                                            selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val =
                                                Some(filtered_items[selector.cursor_idx].clone());
                                        }

                                        let status =
                                            selected_val.unwrap_or_else(|| "Comment".to_string());
                                        app.selector = None;
                                        app.text_input = Some(crate::app::TextInput {
                                            title: format!(
                                                " Submit Review ({}) - Summary/Description ",
                                                status
                                            ),
                                            value: String::new(),
                                            cursor_idx: 0,
                                            action:
                                                crate::app::TextInputAction::SubmitReviewFinal {
                                                    mr_iid: selector.entity_iid,
                                                    status,
                                                },
                                        });
                                        continue;
                                    }

                                    if field_type == "merge_options" {
                                        let is_bulk_merge =
                                            selector.entity_type == "bulk_merge_mrs";
                                        let merge_iids = if is_bulk_merge {
                                            let mut iids: Vec<u64> =
                                                app.selected_mrs.iter().copied().collect();
                                            iids.sort_unstable();
                                            iids
                                        } else {
                                            vec![selector.entity_iid]
                                        };
                                        let mut squash = false;
                                        let mut delete_branch = false;
                                        let mut merge_strategy: Option<&str> = None;
                                        let selected: Vec<String> =
                                            selector.selected_items.iter().cloned().collect();
                                        for opt in &selected {
                                            match opt.as_str() {
                                                "Squash" => squash = true,
                                                "Delete source branch" => delete_branch = true,
                                                "Create merge commit" => {
                                                    merge_strategy = Some("merge");
                                                }
                                                "Rebase and merge" => {
                                                    merge_strategy = Some("rebase");
                                                }
                                                _ => {}
                                            }
                                        }
                                        app.selector = None;
                                        let Some(client) = app.gitlab_client.clone() else {
                                            app.error_message = Some(
                                                "No backend is available for merging".to_string(),
                                            );
                                            continue;
                                        };
                                        let project = app.project_context.clone();
                                        let tx = events.sender();
                                        let tab = app.active_tab;
                                        tokio::spawn(async move {
                                            let mut failures = Vec::new();
                                            for mr_iid in merge_iids {
                                                if let Err(e) = client
                                                    .merge_mr(
                                                        &project,
                                                        mr_iid,
                                                        squash,
                                                        delete_branch,
                                                        merge_strategy,
                                                    )
                                                    .await
                                                {
                                                    failures.push(format!("#{}: {}", mr_iid, e));
                                                }
                                            }
                                            let result = if failures.is_empty() {
                                                Ok(())
                                            } else {
                                                Err(format!(
                                                    "Merge failures: {}",
                                                    failures.join("; ")
                                                ))
                                            };
                                            let _ = tx.send(Event::CommandCompleted(tab, result));
                                        });
                                        if is_bulk_merge {
                                            app.selected_mrs.clear();
                                        } else {
                                            let mr_iid = selector.entity_iid;
                                            if let Some(pos) =
                                                app.mrs.items.iter().position(|m| m.iid == mr_iid)
                                            {
                                                app.mrs.items.remove(pos);
                                            }
                                            app.update_filter_selection();
                                        }
                                        continue;
                                    }

                                    if field_type == "pipeline_select" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val =
                                            selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val =
                                                Some(filtered_items[selector.cursor_idx].clone());
                                        }
                                        if let Some(val) = selected_val {
                                            if let Some(id_str) = val
                                                .strip_prefix('#')
                                                .and_then(|s| s.split(" — ").next())
                                            {
                                                if let Ok(pipeline_id) = id_str.parse::<u64>() {
                                                    if let Some(client) = &app.gitlab_client {
                                                        app.active_pipeline_id = Some(pipeline_id);
                                                        app.jobs.state.select(Some(0));
                                                        app.jobs.items.clear();
                                                        app.loading_tabs.insert(app::Tab::Jobs);
                                                        let client_clone = client.clone();
                                                        let project_context =
                                                            app.project_context.clone();
                                                        let tx = events.sender();
                                                        tokio::spawn(async move {
                                                            match domain::pipelines::list_pipeline_jobs(
                                                                &client_clone,
                                                                &project_context,
                                                                pipeline_id,
                                                            )
                                                            .await
                                                            {
                                                                Ok(jobs) => {
                                                                    let _ =
                                                                        tx.send(Event::JobsTabFetched(
                                                                            pipeline_id,
                                                                            jobs,
                                                                        ));
                                                                }
                                                                Err(e) => {
                                                                    let _ = tx.send(
                                                                        Event::FetchFailed(
                                                                            app::Tab::Jobs,
                                                                            format!(
                                                                                "Failed to fetch jobs for pipeline {}: {}",
                                                                                pipeline_id, e
                                                                            ),
                                                                        ),
                                                                    );
                                                                }
                                                            }
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                        continue;
                                    }

                                    if field_type == "comment_select" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val =
                                            selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val =
                                                Some(filtered_items[selector.cursor_idx].clone());
                                        }

                                        if let Some(val) = selected_val {
                                            if let Some(id_str) = val
                                                .strip_prefix("ID: ")
                                                .and_then(|s| s.split(" |").next())
                                            {
                                                if let Ok(comment_id) = id_str.parse::<u64>() {
                                                    if let Some(comment) = app
                                                        .current_comments
                                                        .iter()
                                                        .find(|c| c.id == comment_id)
                                                        .cloned()
                                                    {
                                                        let is_github = app.is_github();

                                                        let mut actions =
                                                            vec!["Reply to Thread".to_string()];

                                                        if !is_github {
                                                            let is_resolved =
                                                                comment.resolved.unwrap_or(false);
                                                            if is_resolved {
                                                                actions.push(
                                                                    "Unresolve Thread".to_string(),
                                                                );
                                                            } else {
                                                                actions.push(
                                                                    "Resolve Thread".to_string(),
                                                                );
                                                            }
                                                        }

                                                        actions.push("Edit Comment".to_string());
                                                        actions.push("Delete Comment".to_string());

                                                        app.selector = Some(crate::app::Selector {
                                                            title: format!(
                                                                " Actions for Comment {} ",
                                                                comment_id
                                                            ),
                                                            all_items: actions,
                                                            selected_items:
                                                                std::collections::HashSet::new(),
                                                            cursor_idx: 0,
                                                            search_query: String::new(),
                                                            is_filtering: false,
                                                            is_loading: false,
                                                            entity_iid: comment_id,
                                                            entity_type: selector
                                                                .entity_iid
                                                                .to_string(), // Store MR IID as string
                                                            field_type: "comment_action_select"
                                                                .to_string(),
                                                            multi_select: false,
                                                            state: ListState::default(),
                                                        });
                                                        continue;
                                                    }
                                                    // Edit entity: close menu, writes happen on field-level handlers
                                                    app.edit_menu = None;
                                                    continue;
                                                }
                                            }
                                        }
                                        app.selector = None;
                                        continue;
                                    }

                                    if field_type == "comment_action_select" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val =
                                            selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val =
                                                Some(filtered_items[selector.cursor_idx].clone());
                                        }

                                        app.selector = None;

                                        if let Some(action_str) = selected_val {
                                            let comment_id = selector.entity_iid;
                                            let mr_iid =
                                                selector.entity_type.parse::<u64>().unwrap_or(0);

                                            let comment = app
                                                .current_comments
                                                .iter()
                                                .find(|c| c.id == comment_id)
                                                .cloned();

                                            if let Some(comment) = comment {
                                                match action_str.as_str() {
                                                    "Reply to Thread" => {
                                                        let discussion_id = comment
                                                            .discussion_id
                                                            .clone()
                                                            .unwrap_or_else(|| {
                                                                comment.id.to_string()
                                                            });
                                                        app.text_input = Some(crate::app::TextInput {
                                                            title: format!(" Reply to @{} ", comment.author.username),
                                                            value: String::new(),
                                                            cursor_idx: 0,
                                                            action: crate::app::TextInputAction::ReplyToComment {
                                                                mr_iid,
                                                                comment_id,
                                                                discussion_id,
                                                            },
                                                        });
                                                    }
                                                    "Resolve Thread" | "Unresolve Thread" => {
                                                        let is_resolve =
                                                            action_str == "Resolve Thread";
                                                        let client = app.gitlab_client.clone();
                                                        let project_context =
                                                            app.project_context.clone();
                                                        let tx = events.sender();
                                                        let discussion_id = comment
                                                            .discussion_id
                                                            .clone()
                                                            .unwrap_or_default();

                                                        let status_desc = if is_resolve {
                                                            "Resolving"
                                                        } else {
                                                            "Unresolving"
                                                        };
                                                        let _ = tx.send(Event::CommandStarted(
                                                            format!(
                                                                "{} thread MR #{}",
                                                                status_desc, mr_iid
                                                            ),
                                                        ));

                                                        tokio::spawn(async move {
                                                            if let Some(client) = client {
                                                                let encoded_path = project_context
                                                                    .replace("/", "%2F");
                                                                let res_str = if is_resolve {
                                                                    "true"
                                                                } else {
                                                                    "false"
                                                                };
                                                                let output = tokio::process::Command::new("glab")
                                                                    .args([
                                                                        "api",
                                                                        &format!(
                                                                            "projects/{}/merge_requests/{}/discussions/{}?resolved={}",
                                                                            encoded_path, mr_iid, discussion_id, res_str
                                                                        ),
                                                                        "-X",
                                                                        "PUT",
                                                                    ])
                                                                    .output()
                                                                    .await;

                                                                match output {
                                                                    Ok(out)
                                                                        if out.status.success() =>
                                                                    {
                                                                        let _ = tx.send(Event::CommandCompleted(
                                                                            app::Tab::MergeRequests,
                                                                            Ok(()),
                                                                        ));
                                                                    }
                                                                    Ok(out) => {
                                                                        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                                                                        let _ = tx.send(Event::CommandCompleted(
                                                                            app::Tab::MergeRequests,
                                                                            Err(err),
                                                                        ));
                                                                    }
                                                                    Err(e) => {
                                                                        let _ = tx.send(Event::CommandCompleted(
                                                                            app::Tab::MergeRequests,
                                                                            Err(e.to_string()),
                                                                        ));
                                                                    }
                                                                }
                                                            }
                                                        });
                                                    }
                                                    "Edit Comment" => {
                                                        let client = app.gitlab_client.clone();
                                                        let project_context =
                                                            app.project_context.clone();
                                                        let tx = events.sender();

                                                        app.status_message = Some(
                                                            "Opening editor to edit comment..."
                                                                .to_string(),
                                                        );

                                                        let is_github = client
                                                            .as_ref()
                                                            .map_or(false, |c| c.is_github);
                                                        let ext = std::path::Path::new(
                                                            comment
                                                                .position
                                                                .as_ref()
                                                                .and_then(|p| p.new_path.as_ref())
                                                                .map(|s| s.as_str())
                                                                .unwrap_or("md"),
                                                        )
                                                        .extension()
                                                        .and_then(|s| s.to_str())
                                                        .unwrap_or("md");
                                                        let suffix = format!(".{}", ext);

                                                        let new_body = edit_in_editor_with_suffix(
                                                            &comment.body,
                                                            &suffix,
                                                            &mut terminal,
                                                        );
                                                        if let Some(body) = new_body {
                                                            if body != comment.body
                                                                && !body.trim().is_empty()
                                                            {
                                                                let _ = tx.send(
                                                                    Event::CommandStarted(format!(
                                                                        "Editing comment MR #{}",
                                                                        mr_iid
                                                                    )),
                                                                );

                                                                tokio::spawn(async move {
                                                                    if let Some(client) = client {
                                                                        let output = if is_github {
                                                                            let endpoint =
                                                                                if comment
                                                                                    .position
                                                                                    .is_some()
                                                                                {
                                                                                    format!("repos/{}/pulls/comments/{}", project_context, comment_id)
                                                                                } else {
                                                                                    format!("repos/{}/issues/comments/{}", project_context, comment_id)
                                                                                };
                                                                            let payload = serde_json::json!({ "body": body });
                                                                            let temp_path = std::env::temp_dir().join(format!("glab-tui-edit-{}.json", comment_id));
                                                                            let _ = std::fs::write(&temp_path, serde_json::to_string(&payload).unwrap());
                                                                            let temp_str = temp_path.to_string_lossy().to_string();

                                                                            let res = tokio::process::Command::new("gh")
                                                                                .args(["api", &endpoint, "--input", &temp_str, "-X", "PATCH"])
                                                                                .output()
                                                                                .await;
                                                                            let _ = std::fs::remove_file(&temp_path);
                                                                            res
                                                                        } else {
                                                                            let encoded_path =
                                                                                project_context
                                                                                    .replace(
                                                                                        "/", "%2F",
                                                                                    );
                                                                            let payload = serde_json::json!({ "body": body });
                                                                            let temp_path = std::env::temp_dir().join(format!("glab-tui-edit-{}.json", comment_id));
                                                                            let _ = std::fs::write(&temp_path, serde_json::to_string(&payload).unwrap());
                                                                            let temp_str = temp_path.to_string_lossy().to_string();

                                                                            let res = tokio::process::Command::new("glab")
                                                                                .args([
                                                                                    "api",
                                                                                    &format!("projects/{}/merge_requests/{}/notes/{}", encoded_path, mr_iid, comment_id),
                                                                                    "--input",
                                                                                    &temp_str,
                                                                                    "-X",
                                                                                    "PUT"
                                                                                ])
                                                                                .output()
                                                                                .await;
                                                                            let _ = std::fs::remove_file(&temp_path);
                                                                            res
                                                                        };

                                                                        match output {
                                                                            Ok(out)
                                                                                if out
                                                                                    .status
                                                                                    .success() =>
                                                                            {
                                                                                let _ = tx.send(Event::CommandCompleted(
                                                                                    app::Tab::MergeRequests,
                                                                                    Ok(()),
                                                                                ));
                                                                            }
                                                                            Ok(out) => {
                                                                                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                                                                                let _ = tx.send(Event::CommandCompleted(
                                                                                    app::Tab::MergeRequests,
                                                                                    Err(err),
                                                                                ));
                                                                            }
                                                                            Err(e) => {
                                                                                let _ = tx.send(Event::CommandCompleted(
                                                                                    app::Tab::MergeRequests,
                                                                                    Err(e.to_string()),
                                                                                ));
                                                                            }
                                                                        }
                                                                    }
                                                                });
                                                            }
                                                        }
                                                    }
                                                    "Delete Comment" => {
                                                        let client = app.gitlab_client.clone();
                                                        let project_context =
                                                            app.project_context.clone();
                                                        let tx = events.sender();
                                                        let is_github = client
                                                            .as_ref()
                                                            .map_or(false, |c| c.is_github);

                                                        let _ = tx.send(Event::CommandStarted(
                                                            format!(
                                                                "Deleting comment MR #{}",
                                                                mr_iid
                                                            ),
                                                        ));

                                                        tokio::spawn(async move {
                                                            if let Some(client) = client {
                                                                let output = if is_github {
                                                                    let endpoint = if comment
                                                                        .position
                                                                        .is_some()
                                                                    {
                                                                        format!(
                                                                            "repos/{}/pulls/comments/{}",
                                                                            project_context,
                                                                            comment_id
                                                                        )
                                                                    } else {
                                                                        format!(
                                                                            "repos/{}/issues/comments/{}",
                                                                            project_context,
                                                                            comment_id
                                                                        )
                                                                    };
                                                                    tokio::process::Command::new(
                                                                        "gh",
                                                                    )
                                                                    .args([
                                                                        "api", &endpoint, "-X",
                                                                        "DELETE",
                                                                    ])
                                                                    .output()
                                                                    .await
                                                                } else {
                                                                    let encoded_path =
                                                                        project_context
                                                                            .replace("/", "%2F");
                                                                    tokio::process::Command::new("glab")
                                                                        .args([
                                                                            "api",
                                                                            &format!("projects/{}/merge_requests/{}/notes/{}", encoded_path, mr_iid, comment_id),
                                                                            "-X",
                                                                            "DELETE"
                                                                        ])
                                                                        .output()
                                                                        .await
                                                                };

                                                                match output {
                                                                    Ok(out)
                                                                        if out.status.success() =>
                                                                    {
                                                                        let _ = tx.send(Event::CommandCompleted(
                                                                            app::Tab::MergeRequests,
                                                                            Ok(()),
                                                                        ));
                                                                    }
                                                                    Ok(out) => {
                                                                        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                                                                        let _ = tx.send(Event::CommandCompleted(
                                                                            app::Tab::MergeRequests,
                                                                            Err(err),
                                                                        ));
                                                                    }
                                                                    Err(e) => {
                                                                        let _ = tx.send(Event::CommandCompleted(
                                                                            app::Tab::MergeRequests,
                                                                            Err(e.to_string()),
                                                                        ));
                                                                    }
                                                                }
                                                            }
                                                        });
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                        continue;
                                    }

                                    let entity_type = selector.entity_type.clone();
                                    let entity_iid = selector.entity_iid;
                                    let filtered_items = selector.get_filtered_items();
                                    let mut selected_list: Vec<String> =
                                        selector.selected_items.iter().cloned().collect();

                                    // Include highlighted item in selection if nothing auto-selected
                                    if !filtered_items.is_empty() {
                                        let item = &filtered_items[selector.cursor_idx];
                                        if item.starts_with("+ Create \"") {
                                            let query = selector.search_query.trim().to_string();
                                            if !query.is_empty() {
                                                if selector.multi_select {
                                                    if !selected_list.contains(&query) {
                                                        selected_list.push(query);
                                                    }
                                                } else {
                                                    selected_list = vec![query];
                                                }
                                            }
                                        } else if !selector.multi_select
                                            && selected_list.is_empty()
                                            && selector.field_type != "milestone"
                                        {
                                            selected_list.push(item.clone());
                                        }
                                    }

                                    if app.edit_menu.is_some() {
                                        // Write the values directly to the active field of app.edit_menu
                                        if let Some(ref mut menu) = app.edit_menu {
                                            let target_field_name = match field_type.as_str() {
                                                "labels" => "Labels",
                                                "assignees" => "Assignees",
                                                "reviewers" => "Reviewers",
                                                "milestone" => "Milestone",
                                                "confidential" => "Confidential",
                                                "draft_status" => "Status (Draft/Ready)",
                                                "mr_pipeline" => "Merge Request Pipeline",
                                                "source_branch" => "Source Branch",
                                                "target_branch" => "Target Branch",
                                                "pipeline_branch" => "Branch / Ref",
                                                "workflow_file" => "Workflow File",
                                                "tag" => "Tag",
                                                other if other.starts_with("Input: ") => other,
                                                _ => "",
                                            };
                                            if !target_field_name.is_empty() {
                                                if let Some(f) = menu
                                                    .fields
                                                    .iter_mut()
                                                    .find(|f| f.label == target_field_name)
                                                {
                                                    let display_val = if field_type
                                                        == "confidential"
                                                    {
                                                        selected_list
                                                            .first()
                                                            .cloned()
                                                            .unwrap_or_else(|| "No".to_string())
                                                    } else if field_type == "draft_status" {
                                                        selected_list
                                                            .first()
                                                            .cloned()
                                                            .unwrap_or_else(|| "Ready".to_string())
                                                    } else if field_type == "mr_pipeline" {
                                                        selected_list
                                                            .first()
                                                            .cloned()
                                                            .unwrap_or_else(|| "No".to_string())
                                                    } else {
                                                        selected_list.join(", ")
                                                    };

                                                    let is_workflow_file = field_type
                                                        == "workflow_file"
                                                        && !display_val.is_empty();

                                                    f.value = display_val.clone();

                                                    let _ = f; // release borrow before modifying fields

                                                    // When a workflow file is selected, parse
                                                    // its workflow_dispatch inputs and rebuild
                                                    // the edit menu fields to show per-input fields.
                                                    if is_workflow_file {
                                                        let repo_root =
                                                            std::process::Command::new("git")
                                                                .args([
                                                                    "rev-parse",
                                                                    "--show-toplevel",
                                                                ])
                                                                .output()
                                                                .ok()
                                                                .and_then(|o| {
                                                                    String::from_utf8(o.stdout).ok()
                                                                })
                                                                .map(|s| s.trim().to_string());

                                                        if let Some(root) = repo_root {
                                                            let yaml_path = format!(
                                                                "{}/.github/workflows/{}",
                                                                root, display_val
                                                            );
                                                            if let Some(inputs) =
                                                                crate::domain::workflow_inputs::parse_workflow_inputs(&yaml_path)
                                                            {
                                                                menu.workflow_inputs = inputs.clone();
                                                                menu.fields.retain(|f| f.label != "Inputs");
                                                                let insert_pos = menu
                                                                    .fields
                                                                    .iter()
                                                                    .position(|f| f.label == "Variables")
                                                                    .unwrap_or(menu.fields.len());
                                                                for input in inputs.iter().rev() {
                                                                    let label = format!("Input: {}", input.name);
                                                                    let default_val = input.default.clone().unwrap_or_default();
                                                                    menu.fields.insert(insert_pos, crate::app::Field::text(&label, default_val));
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        let active_tab = app.active_tab;
                                        apply_selector_changes(
                                            &mut app,
                                            &entity_type,
                                            entity_iid,
                                            &field_type,
                                            selected_list,
                                            &mut terminal,
                                            events.sender(),
                                            active_tab,
                                        );

                                        rebuild_edit_menu(&mut app, &entity_type, entity_iid);
                                    }
                                }
                                _ => {
                                    app.selector = Some(selector);
                                }
                            }
                        }
                        continue;
                    }

                    if let Some(mut menu) = app.edit_menu.take() {
                        if menu.editing {
                            match key_event.code {
                                KeyCode::Esc | KeyCode::Enter => {
                                    menu.editing = false;
                                    app.edit_menu = Some(menu);
                                }
                                KeyCode::Left => {
                                    if menu.selected_idx < menu.fields.len()
                                        && (menu.fields[menu.selected_idx].label == "Title"
                                            || menu.fields[menu.selected_idx].label
                                                == "Description")
                                    {
                                        if let Some(f) = menu.fields.get(menu.selected_idx) {
                                            if let Some((byte_idx, _)) = f.value[..menu.cursor_pos]
                                                .char_indices()
                                                .next_back()
                                            {
                                                menu.cursor_pos = byte_idx;
                                            } else {
                                                menu.cursor_pos = 0;
                                            }
                                        }
                                    }
                                    app.edit_menu = Some(menu);
                                }
                                KeyCode::Right => {
                                    if menu.selected_idx < menu.fields.len()
                                        && (menu.fields[menu.selected_idx].label == "Title"
                                            || menu.fields[menu.selected_idx].label
                                                == "Description")
                                    {
                                        if let Some(f) = menu.fields.get(menu.selected_idx) {
                                            let char_len = f.value[menu.cursor_pos..]
                                                .chars()
                                                .next()
                                                .map(|c| c.len_utf8())
                                                .unwrap_or(0);
                                            if char_len > 0
                                                && menu.cursor_pos + char_len <= f.value.len()
                                            {
                                                menu.cursor_pos += char_len;
                                            }
                                        }
                                    }
                                    app.edit_menu = Some(menu);
                                }
                                KeyCode::Home => {
                                    if menu.selected_idx < menu.fields.len()
                                        && (menu.fields[menu.selected_idx].label == "Title"
                                            || menu.fields[menu.selected_idx].label
                                                == "Description")
                                    {
                                        menu.cursor_pos = 0;
                                    }
                                    app.edit_menu = Some(menu);
                                }
                                KeyCode::End => {
                                    if menu.selected_idx < menu.fields.len()
                                        && (menu.fields[menu.selected_idx].label == "Title"
                                            || menu.fields[menu.selected_idx].label
                                                == "Description")
                                    {
                                        menu.cursor_pos =
                                            menu.fields[menu.selected_idx].value.len();
                                    }
                                    app.edit_menu = Some(menu);
                                }
                                KeyCode::Backspace => {
                                    let field_name = if menu.selected_idx < menu.fields.len() {
                                        menu.fields[menu.selected_idx].label.clone()
                                    } else {
                                        String::new()
                                    };
                                    if field_name == "Title"
                                        || field_name == "Branch Name"
                                        || field_name == "Description"
                                    {
                                        if let Some(f) = menu.fields.get_mut(menu.selected_idx) {
                                            if let Some((byte_idx, _)) = f.value[..menu.cursor_pos]
                                                .char_indices()
                                                .next_back()
                                            {
                                                f.value.remove(byte_idx);
                                                menu.cursor_pos = byte_idx;
                                            }
                                        }
                                    }
                                    app.edit_menu = Some(menu);
                                }
                                KeyCode::Delete => {
                                    let field_name = if menu.selected_idx < menu.fields.len() {
                                        menu.fields[menu.selected_idx].label.clone()
                                    } else {
                                        String::new()
                                    };
                                    if field_name == "Title"
                                        || field_name == "Branch Name"
                                        || field_name == "Description"
                                    {
                                        if let Some(f) = menu.fields.get_mut(menu.selected_idx) {
                                            if menu.cursor_pos < f.value.len() {
                                                f.value.remove(menu.cursor_pos);
                                            }
                                        }
                                    }
                                    app.edit_menu = Some(menu);
                                }
                                KeyCode::Char(c)
                                    if key_event.modifiers.contains(KeyModifiers::CONTROL)
                                        && c == 'e' =>
                                {
                                    let field_name = if menu.selected_idx < menu.fields.len() {
                                        menu.fields[menu.selected_idx].label.clone()
                                    } else {
                                        String::new()
                                    };
                                    if field_name == "Description" {
                                        let desc = if let Some(f) = menu.fields.iter().find(|f| {
                                            f.label == "Description"
                                                && f.kind == crate::app::FieldType::Text
                                        }) {
                                            f.value.clone()
                                        } else {
                                            String::new()
                                        };
                                        app.edit_menu = Some(menu);
                                        if let Some(new_desc) =
                                            crate::editor::edit_in_editor(&desc, &mut terminal)
                                        {
                                            if let Some(menu) = &mut app.edit_menu {
                                                if let Some(f) = menu.fields.iter_mut().find(|f| {
                                                    f.label == "Description"
                                                        && f.kind == crate::app::FieldType::Text
                                                }) {
                                                    f.value = new_desc;
                                                }
                                            }
                                        }
                                    } else {
                                        app.edit_menu = Some(menu);
                                    }
                                }
                                KeyCode::Char(c) => {
                                    let field_name = if menu.selected_idx < menu.fields.len() {
                                        menu.fields[menu.selected_idx].label.clone()
                                    } else {
                                        String::new()
                                    };
                                    if field_name == "Title"
                                        || field_name == "Branch Name"
                                        || field_name == "Description"
                                    {
                                        if let Some(f) = menu.fields.get_mut(menu.selected_idx) {
                                            f.value.insert(menu.cursor_pos, c);
                                            menu.cursor_pos += c.len_utf8();
                                        }
                                    }
                                    app.edit_menu = Some(menu);
                                }
                                _ => {
                                    // Disable all navigation buttons in edit mode
                                    app.edit_menu = Some(menu);
                                }
                            }
                            continue;
                        }

                        // Navigation mode (!menu.editing):
                        match key_event.code {
                            KeyCode::Esc => {
                                // Close the edit form and restore the zoom state
                                // captured when the menu opened: back to PREVIEW
                                // if entered via double-Enter, or back to NORMAL
                                // if entered via `e`.
                                app.details_zoomed = app.prev_details_zoomed;
                                // Close menu (drop menu by not reassigning)
                            }
                            KeyCode::Char('j') | KeyCode::Down | KeyCode::Tab => {
                                let is_new =
                                    menu.entity_iid == 0 || menu.entity_kind.needs_submit();
                                let max_idx = if is_new {
                                    menu.fields.len() + 1 // fields + spacer + submit
                                } else {
                                    menu.fields.len() - 1
                                };
                                menu.selected_idx = if menu.selected_idx >= max_idx {
                                    0
                                } else {
                                    menu.selected_idx + 1
                                };
                                // Skip section headers, ReadOnly fields, Description, and spacer row.
                                while menu.selected_idx < menu.fields.len()
                                    && (menu.fields[menu.selected_idx].kind
                                        == crate::app::FieldType::Section
                                        || menu.fields[menu.selected_idx].kind
                                            == crate::app::FieldType::ReadOnly
                                        || menu.fields[menu.selected_idx].label == "Description")
                                {
                                    menu.selected_idx += 1;
                                }
                                if is_new && menu.selected_idx == menu.fields.len() {
                                    menu.selected_idx += 1;
                                }
                                menu.state.select(Some(menu.selected_idx));
                                // Reset cursor for inline-editable fields
                                if menu.selected_idx < menu.fields.len() {
                                    let label = &menu.fields[menu.selected_idx].label;
                                    if label == "Title"
                                        || label == "Branch Name"
                                        || label == "Description"
                                    {
                                        menu.cursor_pos =
                                            menu.fields[menu.selected_idx].value.len();
                                    }
                                }
                                app.edit_menu = Some(menu);
                            }
                            // h: jump to left pane (first non-section field)
                            KeyCode::Char('h') | KeyCode::Left => {
                                // Find Title or first editable (non-section, non-readonly) field
                                let target = menu
                                    .fields
                                    .iter()
                                    .position(|f| {
                                        f.kind != crate::app::FieldType::Section
                                            && f.kind != crate::app::FieldType::ReadOnly
                                    })
                                    .unwrap_or(0);
                                menu.selected_idx = target;
                                menu.state.select(Some(target));
                                menu.cursor_pos = menu.fields[target].value.len();
                                app.edit_menu = Some(menu);
                            }
                            // l: jump to right pane (Description field) if present
                            KeyCode::Char('l') | KeyCode::Right => {
                                if let Some(target) = menu.fields.iter().position(|f| {
                                    f.label == "Description"
                                        && f.kind == crate::app::FieldType::Text
                                }) {
                                    menu.selected_idx = target;
                                    menu.state.select(Some(target));
                                    menu.cursor_pos = menu.fields[target].value.len();
                                }
                                app.edit_menu = Some(menu);
                            }
                            // J/K: scroll description pane
                            KeyCode::Char('J') => {
                                menu.desc_scroll = menu.desc_scroll.saturating_add(1);
                                app.edit_menu = Some(menu);
                            }
                            KeyCode::Char('K') => {
                                menu.desc_scroll = menu.desc_scroll.saturating_sub(1);
                                app.edit_menu = Some(menu);
                            }
                            KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => {
                                let is_new =
                                    menu.entity_iid == 0 || menu.entity_kind.needs_submit();
                                let max_idx = if is_new {
                                    menu.fields.len() + 1
                                } else {
                                    menu.fields.len() - 1
                                };
                                menu.selected_idx = if menu.selected_idx == 0 {
                                    max_idx
                                } else {
                                    menu.selected_idx - 1
                                };
                                // Skip section headers, ReadOnly fields, and Description
                                while menu.selected_idx < menu.fields.len()
                                    && (menu.fields[menu.selected_idx].kind
                                        == crate::app::FieldType::Section
                                        || menu.fields[menu.selected_idx].kind
                                            == crate::app::FieldType::ReadOnly
                                        || menu.fields[menu.selected_idx].label == "Description")
                                    && menu.selected_idx > 0
                                {
                                    menu.selected_idx -= 1;
                                }
                                // Skip spacer row
                                if is_new && menu.selected_idx == menu.fields.len() {
                                    menu.selected_idx = menu.fields.len().saturating_sub(1);
                                }
                                menu.state.select(Some(menu.selected_idx));
                                // Reset cursor for inline-editable fields
                                if menu.selected_idx < menu.fields.len() {
                                    let label = &menu.fields[menu.selected_idx].label;
                                    if label == "Title"
                                        || label == "Branch Name"
                                        || label == "Description"
                                    {
                                        menu.cursor_pos =
                                            menu.fields[menu.selected_idx].value.len();
                                    }
                                }
                                app.edit_menu = Some(menu);
                            }
                            KeyCode::Enter => {
                                let entity_iid = menu.entity_iid;
                                let entity_type = menu.entity_kind.legacy_string();
                                let is_new_entity =
                                    entity_iid == 0 || entity_type.starts_with("new_");
                                let is_on_submit = menu.selected_idx == menu.fields.len() + 1;

                                if is_on_submit {
                                    if entity_type == "new_issue" {
                                        let title = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Title")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let description = menu
                                            .fields
                                            .iter()
                                            .find(|f| {
                                                f.label == "Description"
                                                    && f.kind == crate::app::FieldType::Text
                                            })
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let labels = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Labels")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let assignees = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Assignees")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let milestone = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Milestone")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let due_date = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Due Date")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let weight = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Weight")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();

                                        app.edit_menu = None;
                                        let client = app.gitlab_client.clone().unwrap();
                                        let project = app.project_context.clone();
                                        let tx = events.sender();
                                        let tab = app.active_tab;
                                        tokio::spawn(async move {
                                            match client
                                                .create_issue(
                                                    &project,
                                                    &title,
                                                    &description,
                                                    &labels,
                                                    &assignees,
                                                    &milestone,
                                                    &due_date,
                                                    &weight,
                                                )
                                                .await
                                            {
                                                Ok(_) => {
                                                    let _ = tx
                                                        .send(Event::CommandCompleted(tab, Ok(())));
                                                }
                                                Err(e) => {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        tab,
                                                        Err(e.to_string()),
                                                    ));
                                                }
                                            }
                                        });
                                        continue;
                                    } else if entity_type == "new_mr" {
                                        let title = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Title")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let source = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Source Branch")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let target = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Target Branch")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let labels = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Labels")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let assignees = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Assignees")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let reviewers = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Reviewers")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let milestone = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Milestone")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let description = menu
                                            .fields
                                            .iter()
                                            .find(|f| {
                                                f.label == "Description"
                                                    && f.kind == crate::app::FieldType::Text
                                            })
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();

                                        if !source.is_empty() {
                                            let exists = std::process::Command::new("git")
                                                .args(["rev-parse", "--verify", "--quiet", &source])
                                                .output()
                                                .ok()
                                                .map_or(false, |o| o.status.success());
                                            if !exists {
                                                let _ = std::process::Command::new("git")
                                                    .args(["branch", &source, "HEAD"])
                                                    .output();
                                            }
                                            let _ = std::process::Command::new("git")
                                                .args(["push", "-u", "origin", &source])
                                                .output();
                                        }

                                        let issue_iid = if menu.entity_iid > 0 {
                                            Some(menu.entity_iid)
                                        } else {
                                            None
                                        };

                                        app.edit_menu = None;
                                        let client = app.gitlab_client.clone().unwrap();
                                        let project = app.project_context.clone();
                                        let tx = events.sender();
                                        let tab = app.active_tab;
                                        tokio::spawn(async move {
                                            match client
                                                .create_mr(
                                                    &project,
                                                    &title,
                                                    &description,
                                                    &source,
                                                    &target,
                                                    &labels,
                                                    &assignees,
                                                    &reviewers,
                                                    &milestone,
                                                    issue_iid,
                                                )
                                                .await
                                            {
                                                Ok(_) => {
                                                    let _ = tx
                                                        .send(Event::CommandCompleted(tab, Ok(())));
                                                }
                                                Err(e) => {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        tab,
                                                        Err(e.to_string()),
                                                    ));
                                                }
                                            }
                                        });
                                        continue;
                                    } else if entity_type == "new_bulk_edit_issues" {
                                        let labels = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Labels")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let assignees = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Assignees")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let milestone = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Milestone")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();

                                        if labels.is_empty()
                                            && assignees.is_empty()
                                            && milestone.is_empty()
                                        {
                                            app.edit_menu = None;
                                            continue;
                                        }

                                        let selected: Vec<u64> =
                                            app.selected_issues.iter().copied().collect();
                                        app.edit_menu = None;
                                        app.selected_issues.clear();
                                        let client = app.gitlab_client.clone().unwrap();
                                        let project = app.project_context.clone();
                                        let tx = events.sender();
                                        let tab = app.active_tab;
                                        tokio::spawn(async move {
                                            if !labels.is_empty() {
                                                if let Err(e) = client
                                                    .bulk_update_issues_labels(
                                                        &project, &selected, &labels,
                                                    )
                                                    .await
                                                {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        tab,
                                                        Err(e.to_string()),
                                                    ));
                                                    return;
                                                }
                                            }
                                            if !assignees.is_empty() {
                                                if let Err(e) = client
                                                    .bulk_update_issues_assignees(
                                                        &project, &selected, &assignees,
                                                    )
                                                    .await
                                                {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        tab,
                                                        Err(e.to_string()),
                                                    ));
                                                    return;
                                                }
                                            }
                                            if !milestone.is_empty() {
                                                if let Err(e) = client
                                                    .bulk_update_issues_milestone(
                                                        &project, &selected, &milestone,
                                                    )
                                                    .await
                                                {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        tab,
                                                        Err(e.to_string()),
                                                    ));
                                                    return;
                                                }
                                            }
                                            let _ = tx.send(Event::CommandCompleted(tab, Ok(())));
                                        });
                                        continue;
                                    } else if entity_type == "new_bulk_edit_mrs" {
                                        let labels = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Labels")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let assignees = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Assignees")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let milestone = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Milestone")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();

                                        if labels.is_empty()
                                            && assignees.is_empty()
                                            && milestone.is_empty()
                                        {
                                            app.edit_menu = None;
                                            continue;
                                        }

                                        let selected: Vec<u64> =
                                            app.selected_mrs.iter().copied().collect();
                                        app.edit_menu = None;
                                        app.selected_mrs.clear();
                                        let client = app.gitlab_client.clone().unwrap();
                                        let project = app.project_context.clone();
                                        let tx = events.sender();
                                        let tab = app.active_tab;
                                        tokio::spawn(async move {
                                            if !labels.is_empty() {
                                                if let Err(e) = client
                                                    .bulk_update_mrs_labels(
                                                        &project, &selected, &labels,
                                                    )
                                                    .await
                                                {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        tab,
                                                        Err(e.to_string()),
                                                    ));
                                                    return;
                                                }
                                            }
                                            if !assignees.is_empty() {
                                                if let Err(e) = client
                                                    .bulk_update_mrs_assignees(
                                                        &project, &selected, &assignees,
                                                    )
                                                    .await
                                                {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        tab,
                                                        Err(e.to_string()),
                                                    ));
                                                    return;
                                                }
                                            }
                                            if !milestone.is_empty() {
                                                if let Err(e) = client
                                                    .bulk_update_mrs_milestone(
                                                        &project, &selected, &milestone,
                                                    )
                                                    .await
                                                {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        tab,
                                                        Err(e.to_string()),
                                                    ));
                                                    return;
                                                }
                                            }
                                            let _ = tx.send(Event::CommandCompleted(tab, Ok(())));
                                        });
                                        continue;
                                    } else if entity_type == "new_milestone" {
                                        let title = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Title")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let description = menu
                                            .fields
                                            .iter()
                                            .find(|f| {
                                                f.label == "Description"
                                                    && f.kind == crate::app::FieldType::Text
                                            })
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let start_date = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Start Date")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let due_date = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Due Date")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();

                                        app.edit_menu = None;
                                        let client = app.gitlab_client.clone().unwrap();
                                        let project = app.project_context.clone();
                                        let tx = events.sender();
                                        let tab = app.active_tab;
                                        tokio::spawn(async move {
                                            let sd = if start_date.is_empty()
                                                || start_date == "YYYY-MM-DD"
                                            {
                                                None
                                            } else {
                                                Some(start_date.as_str())
                                            };
                                            let dd = if due_date.is_empty()
                                                || due_date == "YYYY-MM-DD"
                                            {
                                                None
                                            } else {
                                                Some(due_date.as_str())
                                            };
                                            match client
                                                .create_milestone(
                                                    &project,
                                                    &title,
                                                    &description,
                                                    sd,
                                                    dd,
                                                )
                                                .await
                                            {
                                                Ok(_) => {
                                                    let _ = tx
                                                        .send(Event::CommandCompleted(tab, Ok(())));
                                                    let _ = tx.send(Event::MilestoneUpdated);
                                                }
                                                Err(e) => {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        tab,
                                                        Err(e.to_string()),
                                                    ));
                                                }
                                            }
                                        });
                                        continue;
                                    } else if entity_type == "new_pipeline" {
                                        let branch = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Branch / Ref")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let mr = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Merge Request Pipeline")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let variables = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Variables")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let inputs = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Inputs")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let workflow = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Workflow File")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();

                                        let var_pairs = parse_key_value_pairs(&variables);
                                        // Collect per-input fields when workflow_dispatch inputs
                                        // were detected; otherwise fall back to the generic
                                        // "Inputs" field.
                                        let per_input_fields: Vec<&crate::app::Field> = menu
                                            .fields
                                            .iter()
                                            .filter(|f| f.label.starts_with("Input: "))
                                            .collect();
                                        let input_pairs: Vec<(String, String)> =
                                            if !per_input_fields.is_empty() {
                                                per_input_fields
                                                    .iter()
                                                    .map(|f| {
                                                        let name = f
                                                            .label
                                                            .strip_prefix("Input: ")
                                                            .unwrap_or(&f.label);
                                                        (
                                                            name.to_string(),
                                                            f.value.trim().to_string(),
                                                        )
                                                    })
                                                    .filter(|(_, v)| !v.is_empty())
                                                    .collect()
                                            } else {
                                                parse_key_value_pairs(&inputs)
                                            };
                                        let mr_flag = mr.to_lowercase() == "yes";

                                        app.edit_menu = None;
                                        let client = app.gitlab_client.clone().unwrap();
                                        let project = app.project_context.clone();
                                        let tx = events.sender();
                                        let tab = app.active_tab;
                                        tokio::spawn(async move {
                                            match client
                                                .run_pipeline(
                                                    &project,
                                                    &branch,
                                                    mr_flag,
                                                    &var_pairs,
                                                    &input_pairs,
                                                    &workflow,
                                                )
                                                .await
                                            {
                                                Ok(_) => {
                                                    let _ = tx
                                                        .send(Event::CommandCompleted(tab, Ok(())));
                                                }
                                                Err(e) => {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        tab,
                                                        Err(e.to_string()),
                                                    ));
                                                }
                                            }
                                        });
                                    } else if entity_type == "new_release" {
                                        let tag = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Tag")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let name = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Release Name")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let description = menu
                                            .fields
                                            .iter()
                                            .find(|f| {
                                                f.label == "Description"
                                                    && f.kind == crate::app::FieldType::Text
                                            })
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();

                                        if !tag.is_empty() {
                                            app.edit_menu = None;
                                            let client = app.gitlab_client.clone().unwrap();
                                            let project = app.project_context.clone();
                                            let tx = events.sender();
                                            let tab = app.active_tab;
                                            tokio::spawn(async move {
                                                match client
                                                    .create_release(
                                                        &project,
                                                        &tag,
                                                        &name,
                                                        &description,
                                                    )
                                                    .await
                                                {
                                                    Ok(_) => {
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            tab,
                                                            Ok(()),
                                                        ));
                                                    }
                                                    Err(e) => {
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            tab,
                                                            Err(e.to_string()),
                                                        ));
                                                    }
                                                }
                                            });
                                        }
                                        continue;
                                    } else if entity_type == "issue" || entity_type == "edit_issue"
                                    {
                                        let title = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Title")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let description = menu
                                            .fields
                                            .iter()
                                            .find(|f| {
                                                f.label == "Description"
                                                    && f.kind == crate::app::FieldType::Text
                                            })
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let labels = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Labels")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let assignees = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Assignees")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let milestone = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Milestone")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let due_date = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Due Date")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let weight = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Weight")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let confidential = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Confidential")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();

                                        app.edit_menu = None;
                                        if let Some(item) = app
                                            .issues
                                            .items
                                            .iter_mut()
                                            .find(|i| i.iid == entity_iid)
                                        {
                                            if !title.is_empty() {
                                                item.title = title.clone();
                                            }
                                            item.description = Some(description.clone());
                                        }

                                        let client = app.gitlab_client.clone().unwrap();
                                        let project = app.project_context.clone();
                                        let tx = events.sender();
                                        let tab = app.active_tab;
                                        tokio::spawn(async move {
                                            let mut last_err = None;
                                            if !title.is_empty() {
                                                if let Err(e) = client
                                                    .update_issue_title(
                                                        &project, entity_iid, &title,
                                                    )
                                                    .await
                                                {
                                                    last_err = Some(e.to_string());
                                                }
                                            }
                                            if let Err(e) = client
                                                .update_issue_description(
                                                    &project,
                                                    entity_iid,
                                                    &description,
                                                )
                                                .await
                                            {
                                                last_err = Some(e.to_string());
                                            }
                                            if !labels.is_empty() && labels != "None" {
                                                let add: Vec<String> = labels
                                                    .split(',')
                                                    .map(|s| s.trim().to_string())
                                                    .filter(|s| !s.is_empty())
                                                    .collect();
                                                let _ = client
                                                    .update_issue_labels(
                                                        &project,
                                                        entity_iid,
                                                        &add,
                                                        &[],
                                                    )
                                                    .await;
                                            }
                                            if !assignees.is_empty() && assignees != "None" {
                                                let add: Vec<String> = assignees
                                                    .split(',')
                                                    .map(|s| {
                                                        s.trim().trim_start_matches('@').to_string()
                                                    })
                                                    .filter(|s| !s.is_empty())
                                                    .collect();
                                                let _ = client
                                                    .update_issue_assignees(
                                                        &project,
                                                        entity_iid,
                                                        &add,
                                                        &[],
                                                    )
                                                    .await;
                                            }
                                            if !milestone.is_empty() && milestone != "None" {
                                                let _ = client
                                                    .update_issue_milestone(
                                                        &project, entity_iid, &milestone,
                                                    )
                                                    .await;
                                            }
                                            if !due_date.is_empty() && due_date != "YYYY-MM-DD" {
                                                let _ = client
                                                    .update_issue_due_date(
                                                        &project, entity_iid, &due_date,
                                                    )
                                                    .await;
                                            }
                                            if !weight.is_empty() {
                                                let _ = client
                                                    .update_issue_weight(
                                                        &project, entity_iid, &weight,
                                                    )
                                                    .await;
                                            }
                                            if !confidential.is_empty() {
                                                let is_conf = confidential.to_lowercase()
                                                    == "confidential"
                                                    || confidential.to_lowercase() == "yes";
                                                let _ = client
                                                    .update_issue_confidential(
                                                        &project, entity_iid, is_conf,
                                                    )
                                                    .await;
                                            }

                                            if let Some(err) = last_err {
                                                let _ =
                                                    tx.send(Event::CommandCompleted(tab, Err(err)));
                                            } else {
                                                let _ =
                                                    tx.send(Event::CommandCompleted(tab, Ok(())));
                                            }
                                        });
                                        continue;
                                    } else if entity_type == "mr" || entity_type == "edit_mr" {
                                        let title = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Title")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let description = menu
                                            .fields
                                            .iter()
                                            .find(|f| {
                                                f.label == "Description"
                                                    && f.kind == crate::app::FieldType::Text
                                            })
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let labels = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Labels")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let assignees = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Assignees")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let reviewers = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Reviewers")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let milestone = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Milestone")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let target_branch = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Target Branch")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let status_draft = menu
                                            .fields
                                            .iter()
                                            .find(|f| {
                                                f.label == "Status (Draft/Ready)"
                                                    || f.label == "Status"
                                            })
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();

                                        app.edit_menu = None;
                                        if let Some(item) =
                                            app.mrs.items.iter_mut().find(|m| m.iid == entity_iid)
                                        {
                                            if !title.is_empty() {
                                                item.title = title.clone();
                                            }
                                            item.description = Some(description.clone());
                                        }

                                        let client = app.gitlab_client.clone().unwrap();
                                        let project = app.project_context.clone();
                                        let tx = events.sender();
                                        let tab = app.active_tab;
                                        tokio::spawn(async move {
                                            let mut last_err = None;
                                            if !title.is_empty() {
                                                if let Err(e) = client
                                                    .update_mr_title(&project, entity_iid, &title)
                                                    .await
                                                {
                                                    last_err = Some(e.to_string());
                                                }
                                            }
                                            if let Err(e) = client
                                                .update_mr_description(
                                                    &project,
                                                    entity_iid,
                                                    &description,
                                                )
                                                .await
                                            {
                                                last_err = Some(e.to_string());
                                            }
                                            if !labels.is_empty() && labels != "None" {
                                                let add: Vec<String> = labels
                                                    .split(',')
                                                    .map(|s| s.trim().to_string())
                                                    .filter(|s| !s.is_empty())
                                                    .collect();
                                                let _ = client
                                                    .update_mr_labels(
                                                        &project,
                                                        entity_iid,
                                                        &add,
                                                        &[],
                                                    )
                                                    .await;
                                            }
                                            if !assignees.is_empty() && assignees != "None" {
                                                let add: Vec<String> = assignees
                                                    .split(',')
                                                    .map(|s| {
                                                        s.trim().trim_start_matches('@').to_string()
                                                    })
                                                    .filter(|s| !s.is_empty())
                                                    .collect();
                                                let _ = client
                                                    .update_mr_assignees(
                                                        &project,
                                                        entity_iid,
                                                        &add,
                                                        &[],
                                                    )
                                                    .await;
                                            }
                                            if !reviewers.is_empty() && reviewers != "None" {
                                                let add: Vec<String> = reviewers
                                                    .split(',')
                                                    .map(|s| {
                                                        s.trim().trim_start_matches('@').to_string()
                                                    })
                                                    .filter(|s| !s.is_empty())
                                                    .collect();
                                                let _ = client
                                                    .update_mr_reviewers(
                                                        &project,
                                                        entity_iid,
                                                        &add,
                                                        &[],
                                                    )
                                                    .await;
                                            }
                                            if !milestone.is_empty() && milestone != "None" {
                                                let _ = client
                                                    .update_mr_milestone(
                                                        &project, entity_iid, &milestone,
                                                    )
                                                    .await;
                                            }
                                            if !target_branch.is_empty() {
                                                let _ = client
                                                    .update_mr_target_branch(
                                                        &project,
                                                        entity_iid,
                                                        &target_branch,
                                                    )
                                                    .await;
                                            }
                                            if !status_draft.is_empty() {
                                                let is_draft =
                                                    status_draft.to_lowercase() == "draft";
                                                let _ = client
                                                    .toggle_mr_draft(&project, entity_iid, is_draft)
                                                    .await;
                                            }

                                            if let Some(err) = last_err {
                                                let _ =
                                                    tx.send(Event::CommandCompleted(tab, Err(err)));
                                            } else {
                                                let _ =
                                                    tx.send(Event::CommandCompleted(tab, Ok(())));
                                            }
                                        });
                                        continue;
                                    } else if entity_type == "milestone"
                                        || entity_type == "edit_milestone"
                                    {
                                        let title = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Title")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let description = menu
                                            .fields
                                            .iter()
                                            .find(|f| {
                                                f.label == "Description"
                                                    && f.kind == crate::app::FieldType::Text
                                            })
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let start_date = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Start Date")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();
                                        let due_date = menu
                                            .fields
                                            .iter()
                                            .find(|f| f.label == "Due Date")
                                            .map(|f| f.value.trim().to_string())
                                            .unwrap_or_default();

                                        app.edit_menu = None;
                                        let client = app.gitlab_client.clone().unwrap();
                                        let project = app.project_context.clone();
                                        let tx = events.sender();
                                        let tab = app.active_tab;
                                        tokio::spawn(async move {
                                            let sd = if start_date.is_empty()
                                                || start_date == "YYYY-MM-DD"
                                                || start_date == "Set"
                                            {
                                                None
                                            } else {
                                                Some(start_date.as_str())
                                            };
                                            let dd = if due_date.is_empty()
                                                || due_date == "YYYY-MM-DD"
                                                || due_date == "Set"
                                            {
                                                None
                                            } else {
                                                Some(due_date.as_str())
                                            };
                                            match client
                                                .update_milestone(
                                                    &project,
                                                    entity_iid,
                                                    &title,
                                                    &description,
                                                    sd,
                                                    dd,
                                                )
                                                .await
                                            {
                                                Ok(_) => {
                                                    let _ = tx
                                                        .send(Event::CommandCompleted(tab, Ok(())));
                                                    let _ = tx.send(Event::MilestoneUpdated);
                                                }
                                                Err(e) => {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        tab,
                                                        Err(e.to_string()),
                                                    ));
                                                }
                                            }
                                        });
                                        continue;
                                    }
                                }

                                // Not on submit — act on the currently selected field.
                                // Non-editable fields (ReadOnly / Section) and the
                                // spacer row are no-ops: pressing Enter on them must
                                // never crash and must never open an editor.
                                if menu.selected_idx < menu.fields.len() {
                                    let f = &menu.fields[menu.selected_idx];
                                    if f.kind == crate::app::FieldType::ReadOnly
                                        || f.kind == crate::app::FieldType::Section
                                    {
                                        app.edit_menu = Some(menu);
                                        continue;
                                    }
                                } else if menu.selected_idx != menu.fields.len() + 1 {
                                    // Spacer row between fields and submit.
                                    app.edit_menu = Some(menu);
                                    continue;
                                }

                                let field_name = if menu.selected_idx < menu.fields.len() {
                                    menu.fields[menu.selected_idx].label.clone()
                                } else {
                                    String::new()
                                };

                                // Title / Branch Name / Description: Enter toggles inline edit mode
                                if field_name == "Title"
                                    || field_name == "Branch Name"
                                    || field_name == "Description"
                                {
                                    if menu.editing {
                                        menu.editing = false;
                                    } else {
                                        menu.editing = true;
                                        if let Some(f) = menu.fields.get(menu.selected_idx) {
                                            menu.cursor_pos = f.value.len();
                                        }
                                    }
                                    app.edit_menu = Some(menu);
                                    continue;
                                }

                                if field_name == "Labels"
                                    || field_name == "Assignees"
                                    || field_name == "Reviewers"
                                    || field_name == "Milestone"
                                    || field_name == "Confidential"
                                    || field_name == "Status (Draft/Ready)"
                                    || field_name == "Merge Request Pipeline"
                                    || field_name == "Source Branch"
                                    || field_name == "Target Branch"
                                    || field_name == "Branch / Ref"
                                    || field_name == "Workflow File"
                                    || field_name == "Tag"
                                    || field_name.starts_with("Input: ")
                                {
                                    let mut current_set = std::collections::HashSet::new();
                                    let field_type = match field_name.as_str() {
                                        "Labels" => "labels",
                                        "Assignees" => "assignees",
                                        "Reviewers" => "reviewers",
                                        "Milestone" => "milestone",
                                        "Confidential" => "confidential",
                                        "Status (Draft/Ready)" => "draft_status",
                                        "Merge Request Pipeline" => "mr_pipeline",
                                        "Source Branch" => "source_branch",
                                        "Target Branch" => "target_branch",
                                        "Branch / Ref" => "pipeline_branch",
                                        "Create From" => "create_from",
                                        "Workflow File" => "workflow_file",
                                        "Tag" => "tag",
                                        _ => "",
                                    };
                                    let multi_select = match field_type {
                                        "labels" | "assignees" | "reviewers" => true,
                                        _ => false,
                                    };

                                    let mut all_items = Vec::new();
                                    let mut is_loading = true;

                                    if field_type == "confidential" {
                                        all_items =
                                            vec!["Public".to_string(), "Confidential".to_string()];
                                        is_loading = false;
                                    } else if field_type == "draft_status" {
                                        all_items = vec!["Draft".to_string(), "Ready".to_string()];
                                        is_loading = false;
                                        let is_new_entity =
                                            entity_iid == 0 || entity_type.starts_with("new_");
                                        if is_new_entity {
                                            let current_val =
                                                menu.fields[menu.selected_idx].value.clone();
                                            if !current_val.is_empty() {
                                                current_set.insert(current_val);
                                            } else {
                                                current_set.insert("Ready".to_string());
                                            }
                                        } else if let Some(mr) =
                                            app.mrs.items.iter().find(|m| m.iid == entity_iid)
                                        {
                                            current_set.insert(if mr.draft {
                                                "Draft".to_string()
                                            } else {
                                                "Ready".to_string()
                                            });
                                        }
                                    } else if field_type == "mr_pipeline" {
                                        all_items = vec!["Yes".to_string(), "No".to_string()];
                                        is_loading = false;
                                        let is_new_entity =
                                            entity_iid == 0 || entity_type.starts_with("new_");
                                        if is_new_entity {
                                            let current_val =
                                                menu.fields[menu.selected_idx].value.clone();
                                            if !current_val.is_empty() {
                                                current_set.insert(current_val);
                                            } else {
                                                current_set.insert("No".to_string());
                                            }
                                        }
                                    } else if field_type == "labels" {
                                        if !app.cached_labels.is_empty() {
                                            all_items = app.cached_labels.clone();
                                            is_loading = false;
                                        }
                                    } else if field_type == "assignees" || field_type == "reviewers"
                                    {
                                        if !app.cached_members.is_empty() {
                                            all_items = app.cached_members.clone();
                                            is_loading = false;
                                        }
                                    } else if field_type == "milestone" {
                                        let mut ms_items = vec!["None".to_string()];
                                        ms_items.extend(
                                            app.milestones
                                                .items
                                                .iter()
                                                .map(|m| m.title.clone())
                                                .filter(|t| t != "None"),
                                        );
                                        all_items = ms_items;
                                        is_loading = false;
                                    } else if field_type == "source_branch"
                                        || field_type == "target_branch"
                                        || field_type == "pipeline_branch"
                                        || field_type == "create_from"
                                    {
                                        let branch_names: Vec<String> = app
                                            .branches
                                            .items
                                            .iter()
                                            .map(|b| b.name.clone())
                                            .collect();
                                        if field_type == "pipeline_branch" {
                                            let current_val =
                                                menu.fields[menu.selected_idx].value.clone();
                                            if !current_val.is_empty() {
                                                current_set.insert(current_val);
                                            }
                                        }
                                        if !branch_names.is_empty() {
                                            all_items = branch_names;
                                            is_loading = false;
                                        }
                                    } else if field_type == "workflow_file" {
                                        all_items = get_workflow_files(app.is_github());
                                        is_loading = false;
                                        // Pre-select any already-typed value
                                        let current_val =
                                            menu.fields[menu.selected_idx].value.clone();
                                        if !current_val.is_empty() {
                                            current_set.insert(current_val);
                                        }
                                    } else if field_name.starts_with("Input: ") {
                                        let input_name =
                                            field_name.strip_prefix("Input: ").unwrap_or("");
                                        if let Some(input) = menu
                                            .workflow_inputs
                                            .iter()
                                            .find(|i| i.name == input_name)
                                        {
                                            use crate::domain::workflow_inputs::WorkflowInputType;
                                            match input.input_type {
                                                WorkflowInputType::Choice => {
                                                    all_items = input.options.clone();
                                                }
                                                WorkflowInputType::Boolean => {
                                                    all_items = vec![
                                                        "true".to_string(),
                                                        "false".to_string(),
                                                    ];
                                                }
                                                _ => {}
                                            }
                                            is_loading = false;
                                        }
                                        let current_val =
                                            menu.fields[menu.selected_idx].value.clone();
                                        if !current_val.is_empty() {
                                            current_set.insert(current_val);
                                        }
                                    } else if field_type == "tag" {
                                        // Collect existing tags from releases cache + git tag
                                        let mut tags: Vec<String> = app
                                            .releases
                                            .items
                                            .iter()
                                            .map(|r| r.tag_name.clone())
                                            .collect();
                                        if let Ok(output) =
                                            std::process::Command::new("git").args(["tag"]).output()
                                        {
                                            if output.status.success() {
                                                for line in
                                                    String::from_utf8_lossy(&output.stdout).lines()
                                                {
                                                    let t = line.trim().to_string();
                                                    if !t.is_empty() && !tags.contains(&t) {
                                                        tags.push(t);
                                                    }
                                                }
                                            }
                                        }
                                        tags.sort();
                                        all_items = tags;
                                        is_loading = false;
                                        let current_val =
                                            menu.fields[menu.selected_idx].value.clone();
                                        if !current_val.is_empty() {
                                            current_set.insert(current_val);
                                        }
                                    }

                                    if entity_iid == 0 || entity_type.starts_with("new_") {
                                        let current_val =
                                            menu.fields[menu.selected_idx].value.clone();
                                        if !current_val.is_empty()
                                            && field_type != "draft_status"
                                            && field_type != "mr_pipeline"
                                        {
                                            if multi_select {
                                                for item in current_val.split(',') {
                                                    let trimmed = item.trim().to_string();
                                                    if !trimmed.is_empty() {
                                                        current_set.insert(trimmed);
                                                    }
                                                }
                                            } else {
                                                current_set.insert(current_val);
                                            }
                                        }
                                    } else if entity_type == "issue" {
                                        if let Some(issue) =
                                            app.issues.items.iter().find(|i| i.iid == entity_iid)
                                        {
                                            match field_type {
                                                "labels" => {
                                                    for l in &issue.labels {
                                                        current_set.insert(l.clone());
                                                    }
                                                }
                                                "assignees" => {
                                                    for a in &issue.assignees {
                                                        current_set
                                                            .insert(format!("@{}", a.username));
                                                    }
                                                }
                                                "milestone" => {
                                                    if let Some(m) = &issue.milestone {
                                                        current_set.insert(m.title.clone());
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                    } else if entity_type == "mr" {
                                        if let Some(mr) =
                                            app.mrs.items.iter().find(|m| m.iid == entity_iid)
                                        {
                                            match field_type {
                                                "labels" => {
                                                    for l in &mr.labels {
                                                        current_set.insert(l.clone());
                                                    }
                                                }
                                                "assignees" => {
                                                    for a in &mr.assignees {
                                                        current_set
                                                            .insert(format!("@{}", a.username));
                                                    }
                                                }
                                                "reviewers" => {
                                                    for r in &mr.reviewers {
                                                        current_set
                                                            .insert(format!("@{}", r.username));
                                                    }
                                                }
                                                "milestone" => {
                                                    if let Some(m) = &mr.milestone {
                                                        current_set.insert(m.title.clone());
                                                    }
                                                }
                                                "target_branch" => {
                                                    current_set.insert(mr.target_branch.clone());
                                                }
                                                _ => {}
                                            }
                                        }
                                    }

                                    let start_idx = if multi_select {
                                        0
                                    } else {
                                        current_set
                                            .iter()
                                            .next()
                                            .and_then(|sel| all_items.iter().position(|a| a == sel))
                                            .unwrap_or(0)
                                    };

                                    app.selector = Some(crate::app::Selector {
                                        title: format!("Select {}", field_name),
                                        all_items,
                                        selected_items: current_set,
                                        cursor_idx: start_idx,
                                        search_query: String::new(),
                                        is_filtering: false,
                                        is_loading,
                                        entity_iid,
                                        entity_type: entity_type.clone(),
                                        field_type: if field_name.starts_with("Input: ") {
                                            field_name.clone()
                                        } else {
                                            field_type.to_string()
                                        },
                                        multi_select,
                                        state: {
                                            let mut s = ListState::default();
                                            s.select(Some(0));
                                            s
                                        },
                                    });

                                    app.edit_menu = Some(menu);

                                    if is_loading {
                                        if let Some(client) = &app.gitlab_client {
                                            let client = client.clone();
                                            let project_context = app.project_context.clone();
                                            let field_type = field_type.to_string();
                                            let tx = events.sender();
                                            tokio::spawn(async move {
                                                let res = match field_type.as_str() {
                                                    "labels" => client
                                                        .fetch_labels(&project_context)
                                                        .await
                                                        .map(|labels| {
                                                            labels
                                                                .into_iter()
                                                                .map(|l| l.name)
                                                                .collect()
                                                        }),
                                                    "assignees" | "reviewers" => {
                                                        client.fetch_members(&project_context).await
                                                    }
                                                    "milestone" => {
                                                        client
                                                            .fetch_milestones(&project_context)
                                                            .await
                                                    }
                                                    "source_branch" | "target_branch"
                                                    | "pipeline_branch" => {
                                                        client
                                                            .fetch_branches(&project_context)
                                                            .await
                                                    }
                                                    _ => Ok(Vec::new()),
                                                };
                                                if let Ok(items) = res {
                                                    let _ =
                                                        tx.send(Event::SelectorItemsFetched(items));
                                                } else {
                                                    let _ = tx.send(Event::SelectorItemsFetched(
                                                        Vec::new(),
                                                    ));
                                                }
                                            });
                                        }
                                    }
                                    continue;
                                }

                                if field_name == "Description" {
                                    // For new entities with empty description, show template selector
                                    if entity_iid == 0 || entity_type.starts_with("new_") {
                                        let raw_val = menu.fields[menu.selected_idx].value.clone();
                                        if raw_val.trim().is_empty() {
                                            let template_type = if entity_type == "new_mr" {
                                                "mr"
                                            } else {
                                                "issue"
                                            };
                                            let templates = list_templates(template_type);
                                            if !templates.is_empty() {
                                                let template_names: Vec<String> =
                                                    std::iter::once("None (blank)".to_string())
                                                        .chain(
                                                            templates
                                                                .iter()
                                                                .map(|(n, _)| n.clone()),
                                                        )
                                                        .collect();
                                                let field_type = if entity_type == "new_mr" {
                                                    "mr_template_selector"
                                                } else {
                                                    "issue_template_selector"
                                                };
                                                app.selector = Some(crate::app::Selector {
                                                    title: format!(
                                                        " Select {} Template ",
                                                        if template_type == "mr" {
                                                            "Merge Request"
                                                        } else {
                                                            "Issue"
                                                        }
                                                    ),
                                                    all_items: template_names,
                                                    selected_items: std::collections::HashSet::new(
                                                    ),
                                                    cursor_idx: 0,
                                                    search_query: String::new(),
                                                    is_filtering: false,
                                                    is_loading: false,
                                                    entity_iid: 0,
                                                    entity_type: entity_type.clone(),
                                                    field_type: if field_name.starts_with("Input: ")
                                                    {
                                                        field_name.clone()
                                                    } else {
                                                        field_type.to_string()
                                                    },
                                                    multi_select: false,
                                                    state: {
                                                        let mut s = ListState::default();
                                                        s.select(Some(0));
                                                        s
                                                    },
                                                });
                                                app.edit_menu = Some(menu);
                                                continue;
                                            }
                                        }
                                    }
                                    // Otherwise inline editing handles Description — Enter does nothing
                                    app.edit_menu = Some(menu);
                                    continue;
                                }

                                if field_name == "Due Date" || field_name == "Start Date" {
                                    let current_val =
                                        if entity_iid == 0 || entity_type.starts_with("new_") {
                                            menu.fields[menu.selected_idx].value.clone()
                                        } else {
                                            if entity_type == "issue" {
                                                app.issues
                                                    .items
                                                    .iter()
                                                    .find(|i| i.iid == entity_iid)
                                                    .and_then(|i| i.due_date.clone())
                                                    .unwrap_or_default()
                                            } else if entity_type == "milestone" {
                                                let m = app
                                                    .milestones
                                                    .items
                                                    .iter()
                                                    .find(|m| m.iid == entity_iid);
                                                if field_name == "Start Date" {
                                                    m.and_then(|m| m.start_date.clone())
                                                        .unwrap_or_default()
                                                } else {
                                                    m.and_then(|m| m.due_date.clone())
                                                        .unwrap_or_default()
                                                }
                                            } else {
                                                String::new()
                                            }
                                        };
                                    let action =
                                        if entity_iid != 0 && !entity_type.starts_with("new_") {
                                            let ft = match field_name.as_str() {
                                                "Due Date" => "due_date",
                                                "Start Date" => "start_date",
                                                _ => "",
                                            };
                                            crate::app::DatePickerAction::EditField {
                                                entity_iid,
                                                entity_type: entity_type.clone(),
                                                field_type: ft.to_string(),
                                            }
                                        } else {
                                            crate::app::DatePickerAction::EditNewField {
                                                field_idx: menu.selected_idx,
                                            }
                                        };
                                    app.date_picker = Some(crate::app::DatePicker::new(
                                        format!(" Select {}", field_name),
                                        &current_val,
                                        action,
                                    ));
                                    app.edit_menu = Some(menu);
                                    continue;
                                }

                                if field_name == "Weight"
                                    || field_name == "Variables"
                                    || field_name == "Inputs"
                                    || field_name == "Release Name"
                                {
                                    let current_val =
                                        if entity_iid == 0 || entity_type.starts_with("new_") {
                                            menu.fields[menu.selected_idx].value.clone()
                                        } else {
                                            let field_type = match field_name.as_str() {
                                                "Title" => "title",
                                                "Target Branch" => "target_branch",
                                                "Weight" => "weight",
                                                "Release Name" => "release_name",
                                                "Tag" => "tag",
                                                _ => "",
                                            };
                                            match field_type {
                                                "title" => {
                                                    if entity_type == "issue" {
                                                        app.issues
                                                            .items
                                                            .iter()
                                                            .find(|i| i.iid == entity_iid)
                                                            .map(|i| i.title.clone())
                                                            .unwrap_or_default()
                                                    } else if entity_type == "milestone" {
                                                        app.milestones
                                                            .items
                                                            .iter()
                                                            .find(|m| m.iid == entity_iid)
                                                            .map(|m| m.title.clone())
                                                            .unwrap_or_default()
                                                    } else {
                                                        app.mrs
                                                            .items
                                                            .iter()
                                                            .find(|m| m.iid == entity_iid)
                                                            .map(|m| m.title.clone())
                                                            .unwrap_or_default()
                                                    }
                                                }
                                                "target_branch" => app
                                                    .mrs
                                                    .items
                                                    .iter()
                                                    .find(|m| m.iid == entity_iid)
                                                    .map(|m| m.target_branch.clone())
                                                    .unwrap_or_default(),
                                                "weight" => "0".to_string(),
                                                "release_name" => app
                                                    .releases
                                                    .items
                                                    .get(entity_iid as usize)
                                                    .map(|r| r.name.clone())
                                                    .unwrap_or_default(),
                                                "tag" => app
                                                    .releases
                                                    .items
                                                    .get(entity_iid as usize)
                                                    .map(|r| r.tag_name.clone())
                                                    .unwrap_or_default(),
                                                _ => String::new(),
                                            }
                                        };

                                    let action = crate::app::TextInputAction::EditNewField {
                                        field_idx: menu.selected_idx,
                                    };

                                    app.text_input = Some(crate::app::TextInput {
                                        title: format!("Edit {}", field_name),
                                        cursor_idx: current_val.len(),
                                        value: current_val,
                                        action,
                                    });

                                    app.edit_menu = Some(menu);
                                    continue;
                                }
                            }
                            KeyCode::Backspace => {
                                if menu.editing {
                                    let field_name = if menu.selected_idx < menu.fields.len() {
                                        menu.fields[menu.selected_idx].label.clone()
                                    } else {
                                        String::new()
                                    };
                                    if field_name == "Title"
                                        || field_name == "Branch Name"
                                        || field_name == "Description"
                                    {
                                        if let Some(f) = menu.fields.get_mut(menu.selected_idx) {
                                            if let Some((byte_idx, _)) = f.value[..menu.cursor_pos]
                                                .char_indices()
                                                .next_back()
                                            {
                                                f.value.remove(byte_idx);
                                                menu.cursor_pos = byte_idx;
                                            }
                                        }
                                    }
                                }
                                app.edit_menu = Some(menu);
                            }
                            KeyCode::Char(c)
                                if key_event.modifiers.contains(KeyModifiers::CONTROL)
                                    && c == 'e' =>
                            {
                                let field_name = if menu.selected_idx < menu.fields.len() {
                                    menu.fields[menu.selected_idx].label.clone()
                                } else {
                                    String::new()
                                };
                                if field_name == "Description" {
                                    let entity_iid = menu.entity_iid;
                                    let entity_type = menu.entity_kind.legacy_string();
                                    let desc = if let Some(f) = menu.fields.iter().find(|f| {
                                        f.label == "Description"
                                            && f.kind == crate::app::FieldType::Text
                                    }) {
                                        f.value.clone()
                                    } else {
                                        String::new()
                                    };
                                    app.edit_menu = Some(menu);
                                    if let Some(new_desc) =
                                        crate::editor::edit_in_editor(&desc, &mut terminal)
                                    {
                                        if let Some(menu) = &mut app.edit_menu {
                                            if let Some(f) = menu.fields.iter_mut().find(|f| {
                                                f.label == "Description"
                                                    && f.kind == crate::app::FieldType::Text
                                            }) {
                                                f.value = new_desc;
                                            }
                                        }
                                    }
                                } else {
                                    app.edit_menu = Some(menu);
                                }
                            }
                            KeyCode::Char(c) => {
                                if menu.editing {
                                    let field_name = if menu.selected_idx < menu.fields.len() {
                                        menu.fields[menu.selected_idx].label.clone()
                                    } else {
                                        String::new()
                                    };
                                    if field_name == "Title"
                                        || field_name == "Branch Name"
                                        || field_name == "Description"
                                    {
                                        if let Some(f) = menu.fields.get_mut(menu.selected_idx) {
                                            f.value.insert(menu.cursor_pos, c);
                                            menu.cursor_pos += c.len_utf8();
                                        }
                                    }
                                }
                                app.edit_menu = Some(menu);
                            }
                            _ => {
                                app.edit_menu = Some(menu);
                            }
                        }
                        continue;
                    }

                    if let Some(mut diff_view) = app.diff_view.take() {
                        let in_selection = diff_view.selection_start.is_some();
                        match key_event.code {
                            KeyCode::Esc => {
                                if diff_view.search_active {
                                    diff_view.search_active = false;
                                    diff_view.clear_search();
                                } else if in_selection {
                                    diff_view.selection_start = None;
                                    diff_view.selection_end = None;
                                } else if !diff_view.search_query.is_empty() {
                                    diff_view.clear_search();
                                } else if !diff_view.focus_on_files {
                                    diff_view.focus_on_files = true;
                                } else if !diff_view.file_tree_visible {
                                    diff_view.file_tree_visible = true;
                                } else {
                                    if !app.draft_comments.is_empty() {
                                        app.confirm_popup =
                                            Some(crate::app::ConfirmAction::SubmitReview(
                                                diff_view.mr_iid,
                                            ));
                                    } else {
                                        app.diff_view = None;
                                        continue;
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('n')
                                if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                if !diff_view.focus_on_files && !diff_view.search_query.is_empty() {
                                    diff_view.search_next();
                                    diff_view.update_selected_file_from_cursor();
                                }
                                app.diff_view = Some(diff_view);
                                continue;
                            }
                            KeyCode::Char('N') => {
                                if key_event.modifiers.contains(KeyModifiers::CONTROL)
                                    && !diff_view.focus_on_files
                                    && !diff_view.search_query.is_empty()
                                {
                                    diff_view.search_prev();
                                    diff_view.update_selected_file_from_cursor();
                                }
                                app.diff_view = Some(diff_view);
                                continue;
                            }
                            // --- Search input mode (real-time) ---
                            _ if diff_view.search_active => {
                                match key_event.code {
                                    KeyCode::Esc => {
                                        diff_view.search_active = false;
                                    }
                                    KeyCode::Backspace => {
                                        diff_view.search_query.pop();
                                        if diff_view.search_query.is_empty() {
                                            diff_view.clear_search();
                                        } else {
                                            diff_view.search(&diff_view.search_query.clone());
                                        }
                                    }
                                    KeyCode::Char(c) => {
                                        diff_view.search_query.push(c);
                                        diff_view.search(&diff_view.search_query.clone());
                                    }
                                    _ => {}
                                }
                                app.diff_view = Some(diff_view);
                                continue;
                            }
                            KeyCode::Char('q') => {
                                if in_selection {
                                    diff_view.selection_start = None;
                                    diff_view.selection_end = None;
                                } else {
                                    if diff_view.search_active {
                                        diff_view.search_active = false;
                                    }
                                    if !app.draft_comments.is_empty() {
                                        app.confirm_popup =
                                            Some(crate::app::ConfirmAction::SubmitReview(
                                                diff_view.mr_iid,
                                            ));
                                    } else {
                                        app.diff_view = None;
                                        continue;
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Tab => {
                                diff_view.focus_on_files = !diff_view.focus_on_files;
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('h') | KeyCode::Left => {
                                if diff_view.focus_on_files {
                                    if !diff_view.visible_nodes.is_empty() {
                                        let node = &diff_view.visible_nodes
                                            [diff_view.selected_visible_idx];
                                        if node.is_dir && node.is_expanded {
                                            let path_id = node.path_id.clone();
                                            diff_view.root_node.toggle_expanded(&path_id, "");
                                            diff_view.rebuild_visible_nodes();
                                        }
                                    }
                                } else {
                                    diff_view.focus_on_files = true;
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('l') | KeyCode::Right => {
                                if diff_view.focus_on_files {
                                    if !diff_view.visible_nodes.is_empty() {
                                        let node = &diff_view.visible_nodes
                                            [diff_view.selected_visible_idx];
                                        if node.is_dir && !node.is_expanded {
                                            diff_view.root_node.toggle_expanded(&node.path_id, "");
                                            diff_view.rebuild_visible_nodes();
                                        } else {
                                            diff_view.focus_on_files = false;
                                        }
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Enter | KeyCode::Char(' ') => {
                                if diff_view.focus_on_files {
                                    // On a file in the tree → open it, switch to diff
                                    if !diff_view.visible_nodes.is_empty() {
                                        let node = &diff_view.visible_nodes
                                            [diff_view.selected_visible_idx];
                                        if node.is_dir {
                                            diff_view.root_node.toggle_expanded(&node.path_id, "");
                                            diff_view.rebuild_visible_nodes();
                                        } else {
                                            diff_view.focus_on_files = false;
                                        }
                                    }
                                } else if diff_view.file_tree_visible {
                                    // Diff focused, tree visible → zoom (hide tree)
                                    diff_view.file_tree_visible = false;
                                } else {
                                    // Diff zoomed, tree hidden → show tree and focus it
                                    diff_view.file_tree_visible = true;
                                    diff_view.focus_on_files = true;
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('z') => {
                                if diff_view.focus_on_files {
                                    diff_view.collapse_all();
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('Z') => {
                                if diff_view.focus_on_files {
                                    diff_view.expand_all();
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('[') => {
                                if !diff_view.focus_on_files {
                                    let current = diff_view.cursor_idx;
                                    if let Some(&prev_hunk) =
                                        diff_view.hunks.iter().rev().find(|&&h| h < current)
                                    {
                                        diff_view.cursor_idx = prev_hunk;
                                        diff_view.scroll_offset =
                                            diff_view.cursor_idx.saturating_sub(5);
                                        diff_view.update_selected_file_from_cursor();
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char(']') => {
                                if !diff_view.focus_on_files {
                                    let current = diff_view.cursor_idx;
                                    if let Some(&next_hunk) =
                                        diff_view.hunks.iter().find(|&&h| h > current)
                                    {
                                        diff_view.cursor_idx = next_hunk;
                                        diff_view.scroll_offset =
                                            diff_view.cursor_idx.saturating_sub(5);
                                        diff_view.update_selected_file_from_cursor();
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('d') => {
                                if !diff_view.focus_on_files {
                                    let old_side_by_side = diff_view.side_by_side;
                                    let old_cursor = diff_view.cursor_idx;
                                    diff_view.side_by_side = !diff_view.side_by_side;
                                    diff_view.update_active_lines();

                                    if old_side_by_side {
                                        if let Some(sline) =
                                            diff_view.side_by_side_lines.get(old_cursor)
                                        {
                                            let target_line =
                                                sline.right.as_ref().or(sline.left.as_ref());
                                            if let Some(target) = target_line {
                                                if let Some(new_idx) =
                                                    diff_view.lines.iter().position(|l| {
                                                        l.file_path == target.file_path
                                                            && l.old_line_num == target.old_line_num
                                                            && l.new_line_num == target.new_line_num
                                                            && l.line_type == target.line_type
                                                    })
                                                {
                                                    diff_view.cursor_idx = new_idx;
                                                }
                                            }
                                        }
                                    } else {
                                        if let Some(uline) = diff_view.lines.get(old_cursor) {
                                            if let Some(new_idx) =
                                                diff_view.side_by_side_lines.iter().position(|l| {
                                                    if uline.line_type
                                                        == crate::app::DiffLineType::HunkHeader
                                                        || uline.line_type
                                                            == crate::app::DiffLineType::Meta
                                                    {
                                                        l.line_type == uline.line_type
                                                            && l.left.as_ref().map_or(false, |x| {
                                                                x.content == uline.content
                                                            })
                                                    } else {
                                                        l.left.as_ref().map_or(false, |x| {
                                                            x.old_line_num == uline.old_line_num
                                                                && x.new_line_num
                                                                    == uline.new_line_num
                                                                && x.file_path == uline.file_path
                                                        }) || l.right.as_ref().map_or(false, |x| {
                                                            x.old_line_num == uline.old_line_num
                                                                && x.new_line_num
                                                                    == uline.new_line_num
                                                                && x.file_path == uline.file_path
                                                        })
                                                    }
                                                })
                                            {
                                                diff_view.cursor_idx = new_idx;
                                            }
                                        }
                                    }

                                    diff_view.scroll_offset =
                                        diff_view.cursor_idx.saturating_sub(5);
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('j') | KeyCode::Down => {
                                if diff_view.focus_on_files {
                                    if !diff_view.visible_nodes.is_empty() {
                                        let old_idx = diff_view.selected_visible_idx;
                                        diff_view.selected_visible_idx =
                                            (diff_view.selected_visible_idx + 1)
                                                .min(diff_view.visible_nodes.len() - 1);
                                        if diff_view.selected_visible_idx != old_idx {
                                            diff_view.cursor_idx = 0;
                                            diff_view.scroll_offset = 0;
                                            diff_view.file_tree_scroll_offset = 0;
                                            diff_view.update_active_lines();
                                        }
                                    }
                                } else {
                                    let active_len = if diff_view.side_by_side {
                                        diff_view.side_by_side_lines.len()
                                    } else {
                                        diff_view.lines.len()
                                    };
                                    if active_len > 0 {
                                        let new_idx =
                                            (diff_view.cursor_idx + 1).min(active_len - 1);
                                        if in_selection {
                                            diff_view.selection_end = Some(new_idx);
                                        }
                                        diff_view.cursor_idx = new_idx;
                                        diff_view.update_selected_file_from_cursor();
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                if diff_view.focus_on_files {
                                    if diff_view.selected_visible_idx > 0 {
                                        let old_idx = diff_view.selected_visible_idx;
                                        diff_view.selected_visible_idx -= 1;
                                        if diff_view.selected_visible_idx != old_idx {
                                            diff_view.cursor_idx = 0;
                                            diff_view.scroll_offset = 0;
                                            diff_view.file_tree_scroll_offset = 0;
                                            diff_view.update_active_lines();
                                        }
                                    }
                                } else {
                                    if diff_view.cursor_idx > 0 {
                                        let new_idx = diff_view.cursor_idx - 1;
                                        if in_selection {
                                            diff_view.selection_end = Some(new_idx);
                                        }
                                        diff_view.cursor_idx = new_idx;
                                        diff_view.update_selected_file_from_cursor();
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('J') => {
                                if diff_view.focus_on_files {
                                    if !diff_view.visible_nodes.is_empty() {
                                        let scroll_amount = 10;
                                        let old_idx = diff_view.selected_visible_idx;
                                        diff_view.selected_visible_idx =
                                            (diff_view.selected_visible_idx + scroll_amount)
                                                .min(diff_view.visible_nodes.len() - 1);
                                        if diff_view.selected_visible_idx != old_idx {
                                            diff_view.cursor_idx = 0;
                                            diff_view.scroll_offset = 0;
                                            diff_view.file_tree_scroll_offset = 0;
                                            diff_view.update_active_lines();
                                        }
                                    }
                                } else {
                                    let active_len = if diff_view.side_by_side {
                                        diff_view.side_by_side_lines.len()
                                    } else {
                                        diff_view.lines.len()
                                    };
                                    if active_len > 0 {
                                        let scroll_amount = 10;
                                        let new_idx = (diff_view.cursor_idx + scroll_amount)
                                            .min(active_len - 1);
                                        if in_selection && !diff_view.focus_on_files {
                                            diff_view.selection_end = Some(new_idx);
                                        }
                                        diff_view.cursor_idx = new_idx;
                                        if !diff_view.focus_on_files {
                                            diff_view.update_selected_file_from_cursor();
                                        }
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('K') => {
                                if diff_view.focus_on_files {
                                    let scroll_amount = 10;
                                    let old_idx = diff_view.selected_visible_idx;
                                    diff_view.selected_visible_idx = diff_view
                                        .selected_visible_idx
                                        .saturating_sub(scroll_amount);
                                    if diff_view.selected_visible_idx != old_idx {
                                        diff_view.cursor_idx = 0;
                                        diff_view.scroll_offset = 0;
                                        diff_view.file_tree_scroll_offset = 0;
                                        diff_view.update_active_lines();
                                    }
                                } else {
                                    let scroll_amount = 10;
                                    let new_idx =
                                        diff_view.cursor_idx.saturating_sub(scroll_amount);
                                    if in_selection && !diff_view.focus_on_files {
                                        diff_view.selection_end = Some(new_idx);
                                    }
                                    diff_view.cursor_idx = new_idx;
                                    if !diff_view.focus_on_files {
                                        diff_view.update_selected_file_from_cursor();
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('/') | KeyCode::Char('f') => {
                                if !diff_view.focus_on_files && !diff_view.search_active {
                                    diff_view.clear_search();
                                    diff_view.search_active = true;
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('v') | KeyCode::Char('V') => {
                                if !diff_view.focus_on_files {
                                    if in_selection {
                                        diff_view.selection_start = None;
                                        diff_view.selection_end = None;
                                        app.status_message =
                                            Some("Selection cancelled.".to_string());
                                    } else {
                                        diff_view.selection_start = Some(diff_view.cursor_idx);
                                        diff_view.selection_end = Some(diff_view.cursor_idx);
                                        app.status_message = Some(
                                            "Selection started. Use j/k to extend, Esc to cancel, c to comment."
                                                .to_string(),
                                        );
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('a') => {
                                if !diff_view.focus_on_files {
                                    let sline = if diff_view.side_by_side {
                                        diff_view
                                            .side_by_side_lines
                                            .get(diff_view.cursor_idx)
                                            .cloned()
                                    } else {
                                        diff_view.lines.get(diff_view.cursor_idx).map(|l| {
                                            crate::app::SideBySideLine {
                                                left: Some(l.clone()),
                                                right: Some(l.clone()),
                                                line_type: l.line_type.clone(),
                                            }
                                        })
                                    };

                                    if let Some(sline) = sline {
                                        let matching_current: Vec<_> = app
                                            .current_comments
                                            .iter()
                                            .filter(|c| {
                                                if c.system {
                                                    return false;
                                                }
                                                if let Some(ref pos) = c.position {
                                                    let path_matches =
                                                        sline.left.as_ref().map_or(false, |l| {
                                                            pos.old_path.as_deref()
                                                                == Some(&l.file_path)
                                                        }) || sline.right.as_ref().map_or(
                                                            false,
                                                            |r| {
                                                                pos.new_path.as_deref()
                                                                    == Some(&r.file_path)
                                                            },
                                                        );

                                                    path_matches
                                                        && ((pos.new_line.is_some()
                                                            && sline.right.as_ref().and_then(
                                                                |r| {
                                                                    r.new_line_num.map(|n| n as u64)
                                                                },
                                                            ) == pos.new_line)
                                                            || (pos.old_line.is_some()
                                                                && sline.left.as_ref().and_then(
                                                                    |l| {
                                                                        l.old_line_num
                                                                            .map(|n| n as u64)
                                                                    },
                                                                ) == pos.old_line))
                                                } else {
                                                    false
                                                }
                                            })
                                            .collect();

                                        if matching_current.is_empty() {
                                            app.status_message = Some(
                                                "No comments on this line to interact with."
                                                    .to_string(),
                                            );
                                        } else if matching_current.len() == 1 {
                                            let comment = matching_current[0];
                                            let comment_id = comment.id;
                                            let is_github = app.is_github();

                                            let mut actions = vec!["Reply to Thread".to_string()];

                                            if !is_github {
                                                let is_resolved = comment.resolved.unwrap_or(false);
                                                if is_resolved {
                                                    actions.push("Unresolve Thread".to_string());
                                                } else {
                                                    actions.push("Resolve Thread".to_string());
                                                }
                                            }

                                            actions.push("Edit Comment".to_string());
                                            actions.push("Delete Comment".to_string());

                                            app.selector = Some(crate::app::Selector {
                                                title: format!(
                                                    " Actions for Comment {} ",
                                                    comment_id
                                                ),
                                                all_items: actions,
                                                selected_items: std::collections::HashSet::new(),
                                                cursor_idx: 0,
                                                search_query: String::new(),
                                                is_filtering: false,
                                                is_loading: false,
                                                entity_iid: comment_id,
                                                entity_type: diff_view.mr_iid.to_string(),
                                                field_type: "comment_action_select".to_string(),
                                                multi_select: false,
                                                state: ListState::default(),
                                            });
                                        } else {
                                            let items: Vec<String> = matching_current
                                                .iter()
                                                .map(|c| {
                                                    let clean_body = c.body.replace('\n', " ");
                                                    let truncated = if clean_body.len() > 40 {
                                                        format!("{}...", &clean_body[..40])
                                                    } else {
                                                        clean_body
                                                    };
                                                    format!(
                                                        "ID: {} | @{}: {}",
                                                        c.id, c.author.username, truncated
                                                    )
                                                })
                                                .collect();

                                            app.selector = Some(crate::app::Selector {
                                                title: " Select Comment to Interact ".to_string(),
                                                all_items: items,
                                                selected_items: std::collections::HashSet::new(),
                                                cursor_idx: 0,
                                                search_query: String::new(),
                                                is_filtering: false,
                                                is_loading: false,
                                                entity_iid: diff_view.mr_iid,
                                                entity_type: "mr".to_string(),
                                                field_type: "comment_select".to_string(),
                                                multi_select: false,
                                                state: ListState::default(),
                                            });
                                        }
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('c') => {
                                if let Some(range) = diff_view.get_comment_range() {
                                    app.text_input = Some(crate::app::TextInput {
                                        title: format!(" Add Comment to {} ", range.file_path),
                                        value: String::new(),
                                        cursor_idx: 0,
                                        action: crate::app::TextInputAction::AddReviewComment {
                                            mr_iid: diff_view.mr_iid,
                                            file_path: range.file_path,
                                            line_num: range.line_num,
                                            old_line_num: range.old_line_num,
                                            end_line_num: range.end_line_num,
                                            end_old_line_num: range.end_old_line_num,
                                        },
                                    });
                                    // Clear selection after starting a comment
                                    diff_view.selection_start = None;
                                    diff_view.selection_end = None;
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('C') => {
                                if let Some(range) = diff_view.get_comment_range() {
                                    app.status_message =
                                        Some("Opening editor for comment...".to_string());
                                    let comment_content = edit_in_editor("", &mut terminal);
                                    if let Some(body) = comment_content {
                                        if !body.trim().is_empty() {
                                            if app.in_review_mode {
                                                app.draft_comments.push(crate::app::DraftComment {
                                                    file_path: range.file_path.clone(),
                                                    line_num: range.line_num,
                                                    old_line_num: range.old_line_num,
                                                    end_line_num: range.end_line_num,
                                                    end_old_line_num: range.end_old_line_num,
                                                    body,
                                                });
                                                app.status_message = Some(format!(
                                                    "Added draft comment. ({} pending)",
                                                    app.draft_comments.len()
                                                ));
                                            } else {
                                                let client = app.gitlab_client.clone().unwrap();
                                                let project = app.project_context.clone();
                                                let mr_iid = diff_view.mr_iid;
                                                let file_path = range.file_path.clone();
                                                let line_num = range.line_num;
                                                let old_line_num = range.old_line_num;
                                                let tx = events.sender();
                                                let tab = app.active_tab;
                                                tokio::spawn(async move {
                                                    match client
                                                        .add_mr_comment(
                                                            &project,
                                                            mr_iid,
                                                            &body,
                                                            Some(&file_path),
                                                            line_num.map(|v| v as u64),
                                                            old_line_num.map(|v| v as u64),
                                                        )
                                                        .await
                                                    {
                                                        Ok(_) => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    tab,
                                                                    Ok(()),
                                                                ));
                                                        }
                                                        Err(e) => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    tab,
                                                                    Err(e.to_string()),
                                                                ));
                                                        }
                                                    }
                                                });
                                            }
                                        }
                                    }
                                    // Clear selection after starting a comment
                                    diff_view.selection_start = None;
                                    diff_view.selection_end = None;
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('r') => {
                                let is_github = app.is_github();
                                app.selector = Some(crate::app::Selector {
                                    title: format!(
                                        " Submit {} Review ",
                                        if is_github {
                                            "Pull Request"
                                        } else {
                                            "Merge Request"
                                        }
                                    ),
                                    all_items: vec![
                                        "Approve".to_string(),
                                        "Request Changes".to_string(),
                                        "Comment".to_string(),
                                    ],
                                    selected_items: std::collections::HashSet::new(),
                                    cursor_idx: 0,
                                    search_query: String::new(),
                                    is_filtering: false,
                                    is_loading: false,
                                    entity_iid: diff_view.mr_iid,
                                    entity_type: "mr".to_string(),
                                    field_type: "review_submit_status".to_string(),
                                    multi_select: false,
                                    state: ListState::default(),
                                });
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('e') => {
                                if let Some(range) = diff_view.get_comment_range() {
                                    let content = range
                                        .lines
                                        .iter()
                                        .map(|l| {
                                            let c = l.content.as_str();
                                            if c.starts_with('+')
                                                || c.starts_with('-')
                                                || c.starts_with(' ')
                                            {
                                                if c.len() > 1 { &c[1..] } else { "" }
                                            } else {
                                                c
                                            }
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n");

                                    app.status_message =
                                        Some("Opening editor for code suggestion...".to_string());
                                    let ext = std::path::Path::new(&range.file_path)
                                        .extension()
                                        .and_then(|s| s.to_str())
                                        .unwrap_or("md");
                                    let suffix = format!(".{}", ext);
                                    let editor_content = edit_in_editor_with_suffix(
                                        &content,
                                        &suffix,
                                        &mut terminal,
                                    );
                                    if let Some(suggestion) = editor_content {
                                        let body = format!("```suggestion\n{}\n```", suggestion);

                                        if app.in_review_mode {
                                            app.draft_comments.push(crate::app::DraftComment {
                                                file_path: range.file_path.clone(),
                                                line_num: range.line_num,
                                                old_line_num: range.old_line_num,
                                                end_line_num: range.end_line_num,
                                                end_old_line_num: range.end_old_line_num,
                                                body,
                                            });
                                            app.status_message = Some(format!(
                                                "Added suggestion draft. ({} pending)",
                                                app.draft_comments.len()
                                            ));
                                        } else {
                                            let client = app.gitlab_client.clone().unwrap();
                                            let project = app.project_context.clone();
                                            let mr_iid = diff_view.mr_iid;
                                            let file_path = range.file_path.clone();
                                            let line_num = range.line_num;
                                            let old_line_num = range.old_line_num;
                                            let tx = events.sender();
                                            let tab = app.active_tab;
                                            tokio::spawn(async move {
                                                match client
                                                    .add_mr_comment(
                                                        &project,
                                                        mr_iid,
                                                        &body,
                                                        Some(&file_path),
                                                        line_num.map(|v| v as u64),
                                                        old_line_num.map(|v| v as u64),
                                                    )
                                                    .await
                                                {
                                                    Ok(_) => {
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            tab,
                                                            Ok(()),
                                                        ));
                                                    }
                                                    Err(e) => {
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            tab,
                                                            Err(e.to_string()),
                                                        ));
                                                    }
                                                }
                                            });
                                        }
                                    }
                                    diff_view.selection_start = None;
                                    diff_view.selection_end = None;
                                }
                                app.diff_view = Some(diff_view);
                            }
                            _ => {
                                app.diff_view = Some(diff_view);
                            }
                        }
                        continue;
                    }

                    if app.focus_column_checklist {
                        if app.save_menu_open {
                            match key_event.code {
                                KeyCode::Esc | KeyCode::Char(',') => {
                                    app.save_menu_open = false;
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    app.save_menu_selection = match app.save_menu_selection {
                                        Some(SaveMenu::Local) => Some(SaveMenu::Global),
                                        Some(SaveMenu::Global) => Some(SaveMenu::Cancel),
                                        Some(SaveMenu::Cancel) => Some(SaveMenu::Local),
                                        None => Some(SaveMenu::Local),
                                    };
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    app.save_menu_selection = match app.save_menu_selection {
                                        Some(SaveMenu::Local) => Some(SaveMenu::Cancel),
                                        Some(SaveMenu::Global) => Some(SaveMenu::Local),
                                        Some(SaveMenu::Cancel) => Some(SaveMenu::Global),
                                        None => Some(SaveMenu::Local),
                                    };
                                }
                                KeyCode::Enter => {
                                    if let Some(sel) = app.save_menu_selection {
                                        match sel {
                                            SaveMenu::Local | SaveMenu::Global => {
                                                app.save_layout(sel);
                                                app.save_menu_open = false;
                                                app.focus_column_checklist = false;
                                            }
                                            SaveMenu::Cancel => {
                                                app.save_menu_open = false;
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                            continue;
                        }

                        if app.editing_page_size {
                            match key_event.code {
                                KeyCode::Char(c) if c.is_ascii_digit() => {
                                    app.page_size_input.push(c);
                                }
                                KeyCode::Backspace => {
                                    app.page_size_input.pop();
                                }
                                KeyCode::Enter => {
                                    if let Ok(new_size) =
                                        app.page_size_input.trim().parse::<usize>()
                                    {
                                        if new_size > 0 {
                                            app.page_size = new_size;
                                            app.config.page_size = new_size;
                                            if let Some(ref mut client) = app.gitlab_client {
                                                client.page_size = new_size;
                                            }
                                            if let Some(client) = app.gitlab_client.clone() {
                                                app.start_loading_tab(app.active_tab);
                                                spawn_refresh_active_tab(
                                                    &client,
                                                    &app.project_context,
                                                    app.active_tab,
                                                    events.sender(),
                                                );
                                            }
                                        }
                                    }
                                    app.editing_page_size = false;
                                }
                                KeyCode::Esc => {
                                    app.editing_page_size = false;
                                }
                                _ => {}
                            }
                            continue;
                        }

                        let kind = app.kind();
                        let cols = app.active_tab.columns(kind);
                        let group_cols: Vec<&str> = cols.iter().copied().collect();
                        let cols_end = cols.len();
                        let group_end = cols_end + group_cols.len();
                        let order_end = group_end + 2;
                        let page_size_idx = order_end;
                        let theme_start = page_size_idx + 1;
                        let themes = crate::config::all_theme_presets();
                        let theme_end = theme_start + themes.len();
                        let max_idx = theme_end; // Save button is at index theme_end

                        match key_event.code {
                            KeyCode::Char(c)
                                if c.is_ascii_digit()
                                    && app.column_checklist_idx == page_size_idx =>
                            {
                                app.editing_page_size = true;
                                app.page_size_input = c.to_string();
                            }
                            KeyCode::Esc | KeyCode::Char(',') => {
                                app.focus_column_checklist = false;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                if app.column_checklist_idx < max_idx {
                                    app.column_checklist_idx += 1;
                                } else {
                                    app.column_checklist_idx = 0;
                                }
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                if app.column_checklist_idx > 0 {
                                    app.column_checklist_idx -= 1;
                                } else {
                                    app.column_checklist_idx = max_idx;
                                }
                            }
                            KeyCode::Char('J') => {
                                app.column_checklist_idx = match app.column_checklist_idx {
                                    idx if idx < cols_end => cols_end,
                                    idx if idx < group_end => group_end,
                                    idx if idx < order_end => page_size_idx,
                                    idx if idx == page_size_idx => theme_start,
                                    _ => 0,
                                };
                            }
                            KeyCode::Char('K') => {
                                app.column_checklist_idx = match app.column_checklist_idx {
                                    idx if idx >= theme_start => page_size_idx,
                                    idx if idx == page_size_idx => order_end,
                                    idx if idx >= group_end => 0,
                                    _ => order_end,
                                };
                            }
                            KeyCode::Char(' ') => {
                                let idx = app.column_checklist_idx;
                                if idx < cols_end {
                                    if let Some(col_name) = cols.get(idx) {
                                        let col_str = col_name.to_string();
                                        if let Some(set) =
                                            app.enabled_columns.get_mut(&app.active_tab)
                                        {
                                            if set.contains(&col_str) {
                                                set.remove(&col_str);
                                            } else {
                                                set.insert(col_str);
                                            }
                                            app.update_filter_selection();
                                        }
                                    }
                                } else if idx < group_end {
                                    let group_idx = idx - cols_end;
                                    if let Some(col) = group_cols.get(group_idx) {
                                        let current_group = app
                                            .group_by_column
                                            .get(&app.active_tab)
                                            .cloned()
                                            .flatten();
                                        if current_group.as_deref() == Some(col) {
                                            app.group_by_column.insert(app.active_tab, None);
                                        } else {
                                            app.group_by_column
                                                .insert(app.active_tab, Some(col.to_string()));
                                        }
                                        app.group_list_state.select(Some(0));
                                        app.update_filter_selection();
                                    }
                                } else if idx < order_end {
                                    app.group_ascending.insert(app.active_tab, idx == group_end);
                                    app.update_filter_selection();
                                } else if idx == page_size_idx {
                                    app.editing_page_size = true;
                                    app.page_size_input = app.page_size.to_string();
                                } else if idx < theme_end {
                                    let theme_idx = idx - theme_start;
                                    if let Some(name) = themes.get(theme_idx) {
                                        crate::config::set_theme_preset(name);
                                        app.config.theme_preset = Some(name.to_string());
                                    }
                                }
                                if let Some(client) = app.gitlab_client.clone() {
                                    app.start_loading_tab(app.active_tab);
                                    spawn_refresh_active_tab(
                                        &client,
                                        &app.project_context,
                                        app.active_tab,
                                        events.sender(),
                                    );
                                }
                            }
                            KeyCode::Enter => {
                                let idx = app.column_checklist_idx;
                                if idx < cols_end {
                                    if let Some(col_name) = cols.get(idx) {
                                        let col_str = col_name.to_string();
                                        let all_values = app
                                            .collect_unique_column_values(app.active_tab, &col_str);
                                        let selected = app
                                            .column_filters
                                            .get(&app.active_tab)
                                            .and_then(|f| f.get(&col_str))
                                            .cloned()
                                            .unwrap_or_default();
                                        app.column_filter_context =
                                            Some((app.active_tab, col_str.clone()));
                                        app.selector = Some(crate::app::Selector {
                                            title: format!("Filter by {}", col_name),
                                            all_items: all_values,
                                            selected_items: selected,
                                            cursor_idx: 0,
                                            search_query: String::new(),
                                            is_filtering: false,
                                            is_loading: false,
                                            entity_iid: 0,
                                            entity_type: String::new(),
                                            field_type: "column_filter".to_string(),
                                            multi_select: true,
                                            state: {
                                                let mut s = ratatui::widgets::ListState::default();
                                                s.select(Some(0));
                                                s
                                            },
                                        });
                                    }
                                } else if idx < group_end {
                                    let group_idx = idx - cols_end;
                                    if let Some(col) = group_cols.get(group_idx) {
                                        let current_group = app
                                            .group_by_column
                                            .get(&app.active_tab)
                                            .cloned()
                                            .flatten();
                                        if current_group.as_deref() == Some(col) {
                                            app.group_by_column.insert(app.active_tab, None);
                                        } else {
                                            app.group_by_column
                                                .insert(app.active_tab, Some(col.to_string()));
                                        }
                                        app.group_list_state.select(Some(0));
                                        app.update_filter_selection();
                                    }
                                    if let Some(client) = app.gitlab_client.clone() {
                                        app.start_loading_tab(app.active_tab);
                                        spawn_refresh_active_tab(
                                            &client,
                                            &app.project_context,
                                            app.active_tab,
                                            events.sender(),
                                        );
                                    }
                                } else if idx < order_end {
                                    app.group_ascending.insert(app.active_tab, idx == group_end);
                                    app.update_filter_selection();
                                    if let Some(client) = app.gitlab_client.clone() {
                                        app.start_loading_tab(app.active_tab);
                                        spawn_refresh_active_tab(
                                            &client,
                                            &app.project_context,
                                            app.active_tab,
                                            events.sender(),
                                        );
                                    }
                                } else if idx == page_size_idx {
                                    app.editing_page_size = true;
                                    app.page_size_input = app.page_size.to_string();
                                } else if idx < theme_end {
                                    let theme_idx = idx - theme_start;
                                    if let Some(name) = themes.get(theme_idx) {
                                        crate::config::set_theme_preset(name);
                                        app.config.theme_preset = Some(name.to_string());
                                    }
                                } else if idx == theme_end {
                                    app.save_menu_open = true;
                                    app.save_menu_selection = Some(SaveMenu::Local);
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if app.job_trace_searching {
                        match key_event.code {
                            KeyCode::Enter | KeyCode::Esc => {
                                app.job_trace_searching = false;
                            }
                            KeyCode::Backspace => {
                                app.job_trace_search_query.pop();
                            }
                            KeyCode::Char(c) => {
                                app.job_trace_search_query.push(c);
                            }
                            _ => {}
                        }
                        if !app.job_trace_search_query.is_empty() {
                            let query = app.job_trace_search_query.to_lowercase();
                            if let Some(trace) = &app.job_trace {
                                if let Some(line) = trace
                                    .lines()
                                    .position(|line| line.to_lowercase().contains(&query))
                                {
                                    app.detail_scroll = line as u16;
                                }
                            }
                        }
                        continue;
                    }

                    if app.active_tab == app::Tab::Jobs
                        && app.job_trace.is_some()
                        && keybinding_matches(&app.config.keybindings.jobs.trace_search, &key_event)
                    {
                        app.job_trace_search_query.clear();
                        app.job_trace_searching = true;
                        continue;
                    }

                    if app.is_typing_search {
                        match key_event.code {
                            KeyCode::Enter | KeyCode::Esc => app.is_typing_search = false,
                            KeyCode::Backspace => {
                                app.search_query.pop();
                                app.update_filter_selection();
                            }
                            KeyCode::Char(c) => {
                                app.search_query.push(c);
                                app.update_filter_selection();
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if keybinding_matches(&app.config.keybindings.global.search, &key_event)
                        && !app.is_typing_search
                        && app.text_input.is_none()
                        && app.edit_menu.is_none()
                        && app.selector.is_none()
                        && !app.focus_column_checklist
                    {
                        app.is_typing_search = true;
                        continue;
                    }

                    if keybinding_matches(&app.config.keybindings.global.global_search, &key_event)
                        && !app.is_typing_search
                        && app.text_input.is_none()
                        && app.edit_menu.is_none()
                        && app.selector.is_none()
                        && !app.focus_column_checklist
                    {
                        let mut items = Vec::new();
                        for issue in &app.issues.items {
                            items.push(format!("Issue #{}: {}", issue.iid, issue.title));
                        }
                        for mr in &app.mrs.items {
                            items.push(format!("MR !{}: {}", mr.iid, mr.title));
                        }

                        app.selector = Some(crate::app::Selector {
                            title: " Global Search ".to_string(),
                            all_items: items,
                            selected_items: std::collections::HashSet::new(),
                            cursor_idx: 0,
                            search_query: String::new(),
                            is_filtering: false,
                            is_loading: false,
                            entity_iid: 0,
                            entity_type: "global_search".to_string(),
                            field_type: "global_search".to_string(),
                            multi_select: false,
                            state: {
                                let mut s = ratatui::widgets::ListState::default();
                                s.select(Some(0));
                                s
                            },
                        });
                        continue;
                    }

                    if keybinding_matches(&app.config.keybindings.global.configure, &key_event)
                        && !app.focus_column_checklist
                        && app.text_input.is_none()
                        && app.edit_menu.is_none()
                        && app.selector.is_none()
                    {
                        app.focus_column_checklist = true;
                        app.column_checklist_idx = 0;
                        continue;
                    }

                    if key_event.code == KeyCode::Char(',') && !app.focus_column_checklist {
                        app.focus_column_checklist = true;
                        app.column_checklist_idx = 0;
                        continue;
                    }

                    handlers::tabs::handle_active_tab_key(
                        &mut app,
                        &key_event,
                        &mut terminal,
                        events.sender(),
                    )
                    .await;
                }
                _ => {}
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_value_pairs() {
        let input = "key1:val1,key2:val2, replicas:int(3), debug:bool(false) ";
        let pairs = parse_key_value_pairs(input);
        assert_eq!(
            pairs,
            vec![
                ("key1".to_string(), "val1".to_string()),
                ("key2".to_string(), "val2".to_string()),
                ("replicas".to_string(), "int(3)".to_string()),
                ("debug".to_string(), "bool(false)".to_string())
            ]
        );
    }
}
