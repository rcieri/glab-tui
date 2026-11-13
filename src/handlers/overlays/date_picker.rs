use crate::app::App;
use crate::effect::Effects;
use crate::entity_editor::{apply_field_text_change, rebuild_edit_menu};
use crate::event::Event;
use crossterm::event::{KeyCode, KeyEvent};
use tokio::sync::mpsc::UnboundedSender;

pub fn handle_date_picker(
    app: &mut App,
    key_event: &KeyEvent,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) -> bool {
    if let Some(mut date_picker) = app.pop_date_picker() {
        match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') => {}
            KeyCode::Char('h') | KeyCode::Left => {
                date_picker.move_day(-1);
                app.push_modal(crate::app::Modal::Date(Box::new(date_picker)));
            }
            KeyCode::Char('l') | KeyCode::Right => {
                date_picker.move_day(1);
                app.push_modal(crate::app::Modal::Date(Box::new(date_picker)));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                date_picker.move_day(-7);
                app.push_modal(crate::app::Modal::Date(Box::new(date_picker)));
            }
            KeyCode::Char('j') | KeyCode::Down => {
                date_picker.move_day(7);
                app.push_modal(crate::app::Modal::Date(Box::new(date_picker)));
            }
            KeyCode::Char('[') | KeyCode::PageUp => {
                date_picker.move_month(-1);
                app.push_modal(crate::app::Modal::Date(Box::new(date_picker)));
            }
            KeyCode::Char(']') | KeyCode::PageDown => {
                date_picker.move_month(1);
                app.push_modal(crate::app::Modal::Date(Box::new(date_picker)));
            }
            KeyCode::Enter => {
                let selected_val = date_picker.value_string();
                match date_picker.action {
                    crate::app::DatePickerAction::EditField {
                        entity_iid,
                        entity_type,
                        field_type,
                    } => {
                        apply_field_text_change(
                            app,
                            &entity_type,
                            entity_iid,
                            &field_type,
                            selected_val,
                            tx.clone(),
                            fx,
                        );
                        rebuild_edit_menu(app, &entity_type, entity_iid);
                    }
                    crate::app::DatePickerAction::EditNewField { field_idx } => {
                        if let Some(ref mut menu) = app.edit_menu_mut() {
                            if field_idx < menu.fields.len() {
                                menu.fields[field_idx].value = selected_val;
                            }
                        }
                    }
                }
            }
            _ => {
                app.push_modal(crate::app::Modal::Date(Box::new(date_picker)));
            }
        }
        return true;
    }
    false
}
