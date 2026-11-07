//! The state transition at the heart of the event loop: `update` applies one
//! event to `App` and returns what the loop has to do about it.

mod events;
mod keys;
mod tick;

use crate::app::App;
use crate::effect::{Effect, Effects};
use crate::event::Event;

pub fn update(app: &mut App, event: Event) -> Vec<Effect> {
    let mut fx = Effects::default();
    let tx = app.tx.clone();
    let may_move_highlight = may_move_highlight(&event);
    match event {
        Event::Tick => tick::tick(app, &tx, &mut fx),
        Event::KeyPress(key_event) => keys::handle_key(app, &key_event, &tx, &mut fx),
        event => events::apply(app, event, &tx, &mut fx),
    }
    if may_move_highlight {
        crate::fetch::prefetch_highlighted(app, &tx, &mut fx);
    }
    fx.into_vec()
}

/// Whether `event` can change which row is highlighted, or what is cached
/// for it. The rest only log, notify or start fetches whose results arrive
/// as events of their own, so the preview's prefetch skips them.
fn may_move_highlight(event: &Event) -> bool {
    !matches!(
        event,
        Event::Tick
            | Event::Resize(..)
            | Event::CommandStarted(_)
            | Event::Notice(_)
            | Event::TerminalCommandLogged(_)
            | Event::CommandRerun { .. }
            | Event::ViewerFailed(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Tab, TextInput, TextInputAction};
    use crate::effect::{EditPurpose, ForegroundJob};
    use crate::event::CommandOutcome;
    use crate::keybinding::KeyPress;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn app_with_client() -> App {
        let mut app = App::default();
        app.gitlab_client = Some(crate::domain::client::GitlabClient {
            is_github: false,
            backend: crate::backend::create_backend(false),
            tx: None,
            page_size: 30,
            api_per_page: 30,
            show_read_notifications: false,
        });
        app
    }

    fn with_open_diff(mut app: App) -> App {
        app.open_diff_view(crate::app::DiffView::new(
            4,
            "group/project".to_string(),
            String::new(),
        ));
        app
    }

    fn spawned(effects: &[Effect]) -> usize {
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::Spawn(_)))
            .count()
    }

    fn press(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::KeyPress(KeyPress::from(KeyEvent::new(code, modifiers)))
    }

    #[test]
    fn opening_a_page_in_the_browser_refetches_neither_the_tab_nor_the_diff() {
        let mut app = with_open_diff(app_with_client());

        let effects = update(
            &mut app,
            Event::CommandCompleted(Tab::Issues, Ok(CommandOutcome::ReadOnly)),
        );

        assert_eq!(spawned(&effects), 0);
        assert!(!app.loading_tabs.contains(&Tab::Issues));
    }

    #[test]
    fn a_mutation_refetches_its_tab_and_the_open_diff() {
        let mut app = with_open_diff(app_with_client());

        let effects = update(
            &mut app,
            Event::CommandCompleted(Tab::Issues, Ok(CommandOutcome::Mutated)),
        );

        assert_eq!(spawned(&effects), 2);
        assert!(app.loading_tabs.contains(&Tab::Issues));
    }

    #[test]
    fn a_tick_does_not_fetch_the_highlighted_pipelines_jobs_but_a_new_list_does() {
        let _guard = crate::config::TEST_ENV_MUTEX.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let mut app = app_with_client();
        app.active_tab = Tab::Pipelines;
        app.pipelines.items = vec![
            serde_json::from_str(
                r#"{"id": 7, "status": "failed", "ref": "main", "updated_at": "2026-01-01T00:00:00Z"}"#,
            )
            .expect("pipeline fixture"),
        ];
        app.pipelines.state.select(Some(0));

        let effects = update(&mut app, Event::Tick);

        assert_eq!(spawned(&effects), 0);
        assert!(app.fetching_pipelines.is_empty());

        let pipelines = app.pipelines.items.clone();
        update(&mut app, Event::PipelinesFetched(pipelines));

        assert_eq!(app.fetching_pipelines, std::collections::HashSet::from([7]));
    }

    #[test]
    fn ctrl_e_in_a_text_input_hands_its_value_to_the_editor_and_takes_the_result_back() {
        let mut app = App::default();
        app.open_text_input(TextInput::new(
            " Page Size ",
            "30".to_string(),
            TextInputAction::EditPageSize,
        ));

        let effects = update(&mut app, press(KeyCode::Char('e'), KeyModifiers::CONTROL));

        let [Effect::Suspend(job)] = effects.as_slice() else {
            panic!("expected one editor handoff, got {effects:?}");
        };
        assert!(matches!(
            job.as_ref(),
            ForegroundJob::Edit { text, purpose: EditPurpose::TextInput, .. } if text == "30"
        ));

        update(
            &mut app,
            Event::EditorClosed {
                purpose: EditPurpose::TextInput,
                text: Some("50".to_string()),
            },
        );

        assert_eq!(app.text_input().map(|ti| ti.buffer.value()), Some("50"));
    }

    #[test]
    fn the_quit_key_asks_the_loop_to_stop() {
        let mut app = App::default();

        let effects = update(&mut app, press(KeyCode::Char('q'), KeyModifiers::NONE));

        assert!(matches!(effects.as_slice(), [Effect::Quit]));
    }

    #[test]
    fn ctrl_s_while_review_threads_is_open_does_not_open_repo_switcher() {
        let mut app = app_with_client();
        app.open_review_threads(crate::app::ReviewThreadsOverview::new(
            &[],
            &crate::app::DiffView::new(1, "p".to_string(), String::new()),
        ));
        assert!(app.review_threads().is_some());

        let effects = update(&mut app, press(KeyCode::Char('s'), KeyModifiers::CONTROL));

        assert!(effects.is_empty());
        assert!(app.selector().is_none());
        assert!(app.review_threads().is_some());
    }

    #[test]
    fn mouse_click_outside_modal_rect_does_not_fall_through_to_table() {
        use crossterm::event::MouseEventKind;
        let mut app = app_with_client();
        app.active_tab = Tab::Issues;
        app.issues.items = vec![
            serde_json::from_str(r#"{"iid": 1, "title": "Test Issue", "state": "opened", "updated_at": "2026-01-01T00:00:00Z", "author": {"username": "user"}}"#).unwrap(),
        ];
        app.issues.state.select(Some(0));
        app.content_rect = Some(ratatui::layout::Rect::new(0, 5, 80, 20));

        app.open_configure();
        assert!(app.has_modal());

        let click_event = Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: KeyModifiers::NONE,
        });

        update(&mut app, click_event);

        assert_eq!(app.issues.state.selected(), Some(0));
    }
}
