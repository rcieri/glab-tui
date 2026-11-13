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
        app.diff_view = Some(crate::app::DiffView::new(
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
        app.text_input = Some(TextInput::new(
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

        assert_eq!(
            app.text_input.as_ref().map(|ti| ti.buffer.value()),
            Some("50")
        );
    }

    #[test]
    fn the_quit_key_asks_the_loop_to_stop() {
        let mut app = App::default();

        let effects = update(&mut app, press(KeyCode::Char('q'), KeyModifiers::NONE));

        assert!(matches!(effects.as_slice(), [Effect::Quit]));
    }

    #[test]
    fn q_in_the_diff_view_closes_the_diff_instead_of_quitting() {
        let mut app = with_open_diff(App::default());

        let effects = update(&mut app, press(KeyCode::Char('q'), KeyModifiers::NONE));

        assert!(!effects.iter().any(|effect| matches!(effect, Effect::Quit)));
        assert!(app.diff_view.is_none());
    }

    #[test]
    fn q_on_a_confirmation_dialog_keeps_it_open_instead_of_quitting() {
        let mut app = App::default();
        app.submit_dialog = Some(crate::app::SubmitDialog::build(
            crate::app::ConfirmAction::MarkAllTodosRead,
            &app,
        ));

        let effects = update(&mut app, press(KeyCode::Char('q'), KeyModifiers::NONE));

        assert!(!effects.iter().any(|effect| matches!(effect, Effect::Quit)));
        assert!(app.submit_dialog.is_some());
    }

    #[test]
    fn only_esc_returns_from_a_project_narrowed_from_its_group_and_reloads_it() {
        let _home = crate::config::isolate_home();
        let group = crate::scope::Scope::Group("group".to_string());
        let project = crate::scope::Scope::Repository("group/project".to_string());
        let mut app = App::default();
        app.scope = group.clone();
        app.drill_into("group/project".to_string());

        update(&mut app, press(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.scope, project);

        let effects = update(&mut app, press(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.scope, group);
        assert!(app.loading_tabs.contains(&app.active_tab));
        assert_eq!(spawned(&effects), 1, "the group's client is built again");
    }

    fn app_with_searched_job_trace() -> App {
        let mut app = App::default();
        app.active_tab = Tab::Jobs;
        app.job_trace = Some(crate::ui::job_trace::JobTraceView::new(
            crate::domain::job_trace::JobTrace::from_read(b"error: boom\n", false),
        ));
        app.job_trace_search_query = "boom".to_string();
        app
    }

    #[test]
    fn esc_clears_the_job_trace_search_and_keeps_the_trace() {
        let mut app = app_with_searched_job_trace();

        update(&mut app, press(KeyCode::Esc, KeyModifiers::NONE));

        assert!(app.job_trace.is_some());
        assert_eq!(app.job_trace_search_query, "");
    }

    #[test]
    fn backspace_closes_the_job_trace_without_leaving_its_search_behind() {
        let mut app = app_with_searched_job_trace();

        update(&mut app, press(KeyCode::Backspace, KeyModifiers::NONE));

        assert!(app.job_trace.is_none());
        assert_eq!(app.job_trace_search_query, "");
    }
}
