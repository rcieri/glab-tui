use crate::app::{App, SaveMenu};
use crate::effect::Effects;
use crate::event::Event;
use crate::fetch::spawn_refresh_active_tab;
use crate::keybinding::KeyPress;
use crossterm::event::KeyCode;
use ratatui::widgets::ListState;
use tokio::sync::mpsc::UnboundedSender;

/// Keys while the Configure View is open.
pub fn handle_key(
    app: &mut App,
    key_event: &KeyPress,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
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
        return;
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
                if let Ok(new_size) = app.page_size_input.trim().parse::<usize>() {
                    if new_size > 0 {
                        app.page_size = new_size;
                        app.config.page_size = new_size;
                        if let Some(ref mut client) = app.gitlab_client {
                            client.page_size = new_size;
                        }
                        if let Some(client) = app.gitlab_client.clone() {
                            app.start_loading_tab(app.active_tab);
                            spawn_refresh_active_tab(
                                fx,
                                &client,
                                &app.scope,
                                app.requested_state_queries(),
                                app.active_tab,
                                app.scoped_sender(tx.clone()),
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
        return;
    }

    let kind = app.kind();
    let cols = app.active_tab.columns(kind, app.scope.is_group());
    let group_cols: Vec<&str> = cols.to_vec();
    let cols_end = cols.len();
    let group_end = cols_end + group_cols.len();
    let order_end = group_end + 2;
    let page_size_idx = order_end;
    let prefetch_idx = page_size_idx + 1;
    let theme_idx = prefetch_idx + 1;
    let save_idx = theme_idx + 1;
    let max_idx = save_idx; // Save button is the last row

    match key_event.code {
        KeyCode::Char(c) if c.is_ascii_digit() && app.column_checklist_idx == page_size_idx => {
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
                idx if idx == page_size_idx => prefetch_idx,
                idx if idx == prefetch_idx => theme_idx,
                _ => 0,
            };
        }
        KeyCode::Char('K') => {
            app.column_checklist_idx = match app.column_checklist_idx {
                idx if idx == save_idx => theme_idx,
                idx if idx == theme_idx => prefetch_idx,
                idx if idx == prefetch_idx => page_size_idx,
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
                    if let Some(set) = app.enabled_columns.get_mut(&app.active_tab) {
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
                    let current_group = app.group_by_column.get(&app.active_tab).cloned().flatten();
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
            } else if idx == prefetch_idx {
                app.config.prefetch_tabs = !app.config.prefetch_tabs;
            } else if idx == theme_idx {
                let theme_list = crate::config::all_theme_presets();
                if !theme_list.is_empty() {
                    app.selector = Some(crate::app::Selector {
                        title: " Select Theme ".to_string(),
                        all_items: theme_list,
                        selected_items: std::collections::HashSet::new(),
                        cursor_idx: 0,
                        search_query: String::new(),
                        is_filtering: false,
                        is_loading: false,
                        entity_iid: 0,
                        entity_type: "theme_selector".to_string(),
                        field_type: "theme_selector".to_string(),
                        multi_select: false,
                        state: {
                            let mut s = ListState::default();
                            s.select(Some(0));
                            s
                        },
                        request: None,
                    });
                }
            }
            if let Some(client) = app.gitlab_client.clone() {
                app.start_loading_tab(app.active_tab);
                spawn_refresh_active_tab(
                    fx,
                    &client,
                    &app.scope,
                    app.requested_state_queries(),
                    app.active_tab,
                    app.scoped_sender(tx.clone()),
                );
            }
        }
        KeyCode::Enter => {
            let idx = app.column_checklist_idx;
            if idx < cols_end {
                if let Some(col_name) = cols.get(idx) {
                    let col_str = col_name.to_string();
                    let all_values = app.collect_unique_column_values(app.active_tab, &col_str);
                    let selected = app
                        .column_filters
                        .get(&app.active_tab)
                        .and_then(|f| f.get(&col_str))
                        .cloned()
                        .unwrap_or_default();
                    app.column_filter_context = Some((app.active_tab, col_str.clone()));
                    app.selector = Some(crate::app::Selector {
                        title: format!("Filter by {col_name}"),
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
                        request: None,
                    });
                }
            } else if idx < group_end {
                let group_idx = idx - cols_end;
                if let Some(col) = group_cols.get(group_idx) {
                    let current_group = app.group_by_column.get(&app.active_tab).cloned().flatten();
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
                        fx,
                        &client,
                        &app.scope,
                        app.requested_state_queries(),
                        app.active_tab,
                        app.scoped_sender(tx.clone()),
                    );
                }
            } else if idx < order_end {
                app.group_ascending.insert(app.active_tab, idx == group_end);
                app.update_filter_selection();
                if let Some(client) = app.gitlab_client.clone() {
                    app.start_loading_tab(app.active_tab);
                    spawn_refresh_active_tab(
                        fx,
                        &client,
                        &app.scope,
                        app.requested_state_queries(),
                        app.active_tab,
                        app.scoped_sender(tx.clone()),
                    );
                }
            } else if idx == page_size_idx {
                app.editing_page_size = true;
                app.page_size_input = app.page_size.to_string();
            } else if idx == prefetch_idx {
                app.config.prefetch_tabs = !app.config.prefetch_tabs;
            } else if idx == theme_idx {
                let theme_list = crate::config::all_theme_presets();
                if !theme_list.is_empty() {
                    app.selector = Some(crate::app::Selector {
                        title: " Select Theme ".to_string(),
                        all_items: theme_list,
                        selected_items: std::collections::HashSet::new(),
                        cursor_idx: 0,
                        search_query: String::new(),
                        is_filtering: false,
                        is_loading: false,
                        entity_iid: 0,
                        entity_type: "theme_selector".to_string(),
                        field_type: "theme_selector".to_string(),
                        multi_select: false,
                        state: {
                            let mut s = ListState::default();
                            s.select(Some(0));
                            s
                        },
                        request: None,
                    });
                }
            } else if idx == save_idx {
                app.save_menu_open = true;
                app.save_menu_selection = Some(SaveMenu::Local);
            }
        }
        _ => {}
    }
}
