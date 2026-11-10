use crate::app::{App, SaveMenu};
use crate::configure_view::{ConfigureRow, configure_rows, move_tab, toggle_tab};
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
    let (save_menu_open, editing_page_size, column_checklist_idx) = match app.configure() {
        Some(c) => (
            c.save_menu_open,
            c.editing_page_size,
            c.column_checklist_idx,
        ),
        None => return,
    };

    if save_menu_open {
        match key_event.code {
            KeyCode::Esc | KeyCode::Char(',') => {
                if let Some(c) = app.configure_mut() {
                    c.save_menu_open = false;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(c) = app.configure_mut() {
                    c.save_menu_selection = match c.save_menu_selection {
                        Some(SaveMenu::Local) => Some(SaveMenu::Global),
                        Some(SaveMenu::Global) => Some(SaveMenu::Cancel),
                        Some(SaveMenu::Cancel) => Some(SaveMenu::Local),
                        None => Some(SaveMenu::Local),
                    };
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(c) = app.configure_mut() {
                    c.save_menu_selection = match c.save_menu_selection {
                        Some(SaveMenu::Local) => Some(SaveMenu::Cancel),
                        Some(SaveMenu::Global) => Some(SaveMenu::Local),
                        Some(SaveMenu::Cancel) => Some(SaveMenu::Global),
                        None => Some(SaveMenu::Local),
                    };
                }
            }
            KeyCode::Enter => {
                let selection = app.configure().and_then(|c| c.save_menu_selection);
                if let Some(sel) = selection {
                    match sel {
                        SaveMenu::Local | SaveMenu::Global => {
                            app.save_layout(sel);
                            app.close_configure();
                        }
                        SaveMenu::Cancel => {
                            if let Some(c) = app.configure_mut() {
                                c.save_menu_open = false;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        return;
    }

    if editing_page_size {
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
                        refresh_active_tab(app, tx, fx);
                    }
                }
                if let Some(c) = app.configure_mut() {
                    c.editing_page_size = false;
                }
            }
            KeyCode::Esc => {
                if let Some(c) = app.configure_mut() {
                    c.editing_page_size = false;
                }
            }
            _ => {}
        }
        return;
    }

    let rows = configure_rows(app);
    let cursor = column_checklist_idx.min(rows.len() - 1);
    let row = rows[cursor];

    match key_event.code {
        KeyCode::Char(c) if c.is_ascii_digit() && row == ConfigureRow::PageSize => {
            if let Some(c_cfg) = app.configure_mut() {
                c_cfg.editing_page_size = true;
            }
            app.page_size_input = c.to_string();
        }
        KeyCode::Esc | KeyCode::Char(',') => {
            app.close_configure();
        }
        KeyCode::Down | KeyCode::Char('j') => focus(app, (cursor + 1) % rows.len()),
        KeyCode::Up | KeyCode::Char('k') => focus(app, (cursor + rows.len() - 1) % rows.len()),
        KeyCode::Tab => focus(app, next_section_start(&rows, cursor)),
        KeyCode::BackTab => focus(app, previous_section_start(&rows, cursor)),
        KeyCode::Char(c @ ('J' | 'K')) => {
            let towards_end = c == 'J';
            let moved = match row {
                ConfigureRow::Column(column) => {
                    let tab = app.active_tab;
                    app.move_column(tab, column, towards_end)
                }
                ConfigureRow::Tab(tab) => move_tab(&mut app.config.layout, tab, towards_end),
                _ => false,
            };
            if moved {
                refocus(app, row);
            }
        }
        KeyCode::Left | KeyCode::Char('h') | KeyCode::Right | KeyCode::Char('l') => {
            if let ConfigureRow::Layout(setting) = row {
                let forward = matches!(key_event.code, KeyCode::Right | KeyCode::Char('l'));
                setting.step(&mut app.config.layout, forward);
            }
        }
        KeyCode::Enter => match row {
            ConfigureRow::Column(column) => open_column_filter(app, column),
            _ => activate_row(app, row, tx, fx),
        },
        KeyCode::Char(' ') => activate_row(app, row, tx, fx),
        _ => {}
    }
}

/// What `Space` (and a click) does to a row: toggle it, pick it, or open
/// what it edits.
pub(crate) fn activate_row(
    app: &mut App,
    row: ConfigureRow,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    let tab = app.active_tab;
    match row {
        ConfigureRow::Column(column) => {
            if let Some(set) = app.enabled_columns.get_mut(&tab) {
                if !set.remove(column) {
                    set.insert(column.to_string());
                }
                app.update_filter_selection();
            }
            refresh_active_tab(app, tx, fx);
        }
        ConfigureRow::GroupBy(column) => {
            let current_group = app.group_by_column.get(&tab).cloned().flatten();
            let next = (current_group.as_deref() != Some(column)).then(|| column.to_string());
            app.group_by_column.insert(tab, next);
            app.group_list_state.select(Some(0));
            app.update_filter_selection();
            refresh_active_tab(app, tx, fx);
        }
        ConfigureRow::Order { ascending } => {
            app.group_ascending.insert(tab, ascending);
            app.update_filter_selection();
            refresh_active_tab(app, tx, fx);
        }
        ConfigureRow::PageSize => {
            if let Some(c) = app.configure_mut() {
                c.editing_page_size = true;
            }
            app.page_size_input = app.page_size.to_string();
        }
        ConfigureRow::PrefetchTabs => {
            app.config.prefetch_tabs = !app.config.prefetch_tabs;
        }
        ConfigureRow::Theme => open_theme_selector(app),
        ConfigureRow::Tab(shown_tab) => {
            if !toggle_tab(&mut app.config.layout, shown_tab) {
                app.show_notice("The last shown tab cannot be hidden".to_string());
                return;
            }
            app.land_on_available_tab();
            if app.active_tab != tab {
                crate::handlers::tabs::load_entered_tab(app, tx, fx);
            }
            refocus(app, row);
        }
        ConfigureRow::Layout(setting) => setting.step(&mut app.config.layout, true),
        ConfigureRow::Save => {
            if let Some(c) = app.configure_mut() {
                c.save_menu_open = true;
                c.save_menu_selection = Some(SaveMenu::Local);
            }
        }
    }
}

pub(crate) fn focus(app: &mut App, idx: usize) {
    if let Some(c) = app.configure_mut() {
        c.column_checklist_idx = idx;
    }
}

/// Keeps the focus on `row` after a change moved it in the list.
fn refocus(app: &mut App, row: ConfigureRow) {
    if let Some(idx) = configure_rows(app).iter().position(|r| *r == row) {
        focus(app, idx);
    }
}

fn section_starts(rows: &[ConfigureRow]) -> Vec<usize> {
    (0..rows.len())
        .filter(|&idx| idx == 0 || rows[idx].section() != rows[idx - 1].section())
        .collect()
}

fn next_section_start(rows: &[ConfigureRow], cursor: usize) -> usize {
    section_starts(rows)
        .into_iter()
        .find(|&start| start > cursor)
        .unwrap_or(0)
}

fn previous_section_start(rows: &[ConfigureRow], cursor: usize) -> usize {
    let starts = section_starts(rows);
    let current = starts
        .iter()
        .copied()
        .filter(|&start| start <= cursor)
        .max()
        .unwrap_or(0);
    starts
        .iter()
        .copied()
        .filter(|&start| start < current)
        .max()
        .or_else(|| starts.last().copied())
        .unwrap_or(0)
}

fn refresh_active_tab(app: &mut App, tx: &UnboundedSender<Event>, fx: &mut Effects) {
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

fn open_column_filter(app: &mut App, column: &str) {
    let all_values = app.collect_unique_column_values(app.active_tab, column);
    let selected = app
        .column_filters
        .get(&app.active_tab)
        .and_then(|f| f.get(column))
        .cloned()
        .unwrap_or_default();
    app.column_filter_context = Some((app.active_tab, column.to_string()));
    app.open_selector(crate::app::Selector {
        title: format!("Filter by {column}"),
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
            let mut s = ListState::default();
            s.select(Some(0));
            s
        },
        request: None,
    });
}

fn open_theme_selector(app: &mut App) {
    let theme_list = crate::config::all_theme_presets();
    if theme_list.is_empty() {
        return;
    }
    app.open_selector(crate::app::Selector {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Tab;
    use crate::config::PreviewPlacement;
    use crate::configure_view::LayoutSetting;
    use crossterm::event::{KeyEvent, KeyModifiers};

    fn configure_open_on(tab: Tab) -> App {
        let mut app = App::default();
        app.active_tab = tab;
        app.open_configure();
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let key = KeyPress::Key(KeyEvent::new(code, KeyModifiers::NONE));
        handle_key(app, &key, &tx, &mut Effects::default());
    }

    fn focus_row(app: &mut App, row: ConfigureRow) {
        let idx = configure_rows(app).iter().position(|r| *r == row).unwrap();
        focus(app, idx);
    }

    fn focused(app: &App) -> ConfigureRow {
        configure_rows(app)[app.configure().unwrap().column_checklist_idx]
    }

    #[test]
    fn shift_j_and_k_move_the_focused_column_and_the_focus_follows_it() {
        let mut app = configure_open_on(Tab::Issues);
        focus_row(&mut app, ConfigureRow::Column("ID"));

        press(&mut app, KeyCode::Char('J'));
        assert_eq!(&app.ordered_columns(Tab::Issues)[..2], ["State", "ID"]);
        assert_eq!(focused(&app), ConfigureRow::Column("ID"));

        press(&mut app, KeyCode::Char('K'));
        assert_eq!(&app.ordered_columns(Tab::Issues)[..2], ["ID", "State"]);
    }

    #[test]
    fn shift_j_moves_a_tab_later_in_the_tab_list() {
        let mut app = configure_open_on(Tab::Issues);
        focus_row(&mut app, ConfigureRow::Tab(Tab::Issues));

        press(&mut app, KeyCode::Char('J'));

        assert_eq!(
            &app.available_tabs()[..2],
            [Tab::MergeRequests, Tab::Issues]
        );
        assert_eq!(focused(&app), ConfigureRow::Tab(Tab::Issues));
    }

    #[test]
    fn space_hides_the_active_tab_and_lands_on_the_first_shown_one() {
        let mut app = configure_open_on(Tab::Issues);
        focus_row(&mut app, ConfigureRow::Tab(Tab::Issues));

        press(&mut app, KeyCode::Char(' '));

        assert!(!app.available_tabs().contains(&Tab::Issues));
        assert_eq!(app.active_tab, Tab::MergeRequests);
        assert_eq!(focused(&app), ConfigureRow::Tab(Tab::Issues));
    }

    #[test]
    fn l_and_h_step_the_focused_layout_setting() {
        let mut app = configure_open_on(Tab::Issues);
        focus_row(&mut app, ConfigureRow::Layout(LayoutSetting::Preview));

        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.config.layout.preview, PreviewPlacement::Bottom);
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.config.layout.preview, PreviewPlacement::Right);
    }

    #[test]
    fn tab_and_back_tab_jump_between_sections() {
        let mut app = configure_open_on(Tab::Issues);
        focus_row(&mut app, ConfigureRow::Column("State"));

        press(&mut app, KeyCode::Tab);
        assert_eq!(focused(&app), ConfigureRow::GroupBy("ID"));
        press(&mut app, KeyCode::BackTab);
        assert_eq!(focused(&app), ConfigureRow::Column("ID"));
        press(&mut app, KeyCode::BackTab);
        assert_eq!(focused(&app), ConfigureRow::Save);
    }
}
