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
    use crate::domain::client::JobsSource;
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
            opened_pipeline: None,
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
    fn diff_fetched_updates_diff_in_place_without_lifting_it_above_review_threads() {
        let mut app = with_open_diff(app_with_client());
        let overview = crate::app::ReviewThreadsOverview::new(&[], app.diff_view().unwrap());
        app.open_review_threads(overview);
        assert!(matches!(
            app.top_modal(),
            Some(crate::app::Modal::ReviewThreads(_))
        ));

        let updated_diff = crate::app::DiffView::new(
            4,
            "group/project".to_string(),
            "diff --git a/a b/b\n--- a/a\n+++ b/b\n@@ -1 +1 @@\n-old\n+new\n".to_string(),
        );
        update(
            &mut app,
            Event::DiffFetched {
                diff_view: Box::new(updated_diff),
                comments: Ok(vec![]),
            },
        );

        assert!(
            matches!(app.top_modal(), Some(crate::app::Modal::ReviewThreads(_))),
            "top modal must remain ReviewThreads, not Diff"
        );
        assert_eq!(
            app.diff_view().map(|d| d.lines.len()),
            Some(6),
            "diff view content was updated in place"
        );
    }

    #[test]
    fn diff_fetched_when_help_open_inserts_diff_at_bottom_preserving_help_on_top() {
        let mut app = App::default();
        app.open_help();
        assert!(matches!(app.top_modal(), Some(crate::app::Modal::Help)));

        let diff = crate::app::DiffView::new(
            4,
            "group/project".to_string(),
            "diff --git a/a b/b\n--- a/a\n+++ b/b\n@@ -1 +1 @@\n-old\n+new\n".to_string(),
        );
        update(
            &mut app,
            Event::DiffFetched {
                diff_view: Box::new(diff),
                comments: Ok(vec![]),
            },
        );

        assert!(
            matches!(app.top_modal(), Some(crate::app::Modal::Help)),
            "top modal must remain Help after a newly fetched diff arrives"
        );
        assert!(app.diff_view().is_some());
    }

    #[test]
    fn pr_stack_fetched_does_not_open_dialog_when_modal_open() {
        let mut app = App::default();
        let pr = ("group/project".to_string(), 42);
        app.pending_stack_action = Some((pr.clone(), crate::app::StackAction::Merge));
        app.open_help();
        assert!(matches!(app.top_modal(), Some(crate::app::Modal::Help)));

        update(
            &mut app,
            Event::PrStackFetched {
                project_path: pr.0,
                pr_number: pr.1,
                result: Ok(None),
            },
        );

        assert!(
            matches!(app.top_modal(), Some(crate::app::Modal::Help)),
            "Help must remain on top without merge dialog being pushed"
        );
        assert!(app.submit_dialog().is_none());
    }

    #[test]
    fn diff_search_enter_keeps_query_and_clears_active() {
        let mut app = App::default();
        let mut view = crate::app::DiffView::new(
            1,
            "owner/repo".to_string(),
            "diff --git a/b b/b\n--- a/b\n+++ b/b\n@@ -1 +1,2 @@\n-old\n+new\n".to_string(),
        );
        view.focus_on_files = false;
        view.search_active = true;
        app.open_diff_view(view);

        update(&mut app, press(KeyCode::Char('n'), KeyModifiers::NONE));
        assert_eq!(app.diff_view().unwrap().search_query, "n");
        assert!(app.diff_view().unwrap().search_active);

        update(&mut app, press(KeyCode::Enter, KeyModifiers::NONE));
        let diff = app.diff_view().expect("diff view open");
        assert_eq!(diff.search_query, "n");
        assert!(!diff.search_active);
    }

    #[test]
    fn diff_search_ctrl_key_does_not_append_to_query() {
        let mut app = App::default();
        let mut view = crate::app::DiffView::new(
            1,
            "owner/repo".to_string(),
            "diff --git a/b b/b\n--- a/b\n+++ b/b\n@@ -1 +1,2 @@\n-old\n+new\n".to_string(),
        );
        view.focus_on_files = false;
        view.search_active = true;
        app.open_diff_view(view);

        update(&mut app, press(KeyCode::Char('n'), KeyModifiers::NONE));
        assert_eq!(app.diff_view().unwrap().search_query, "n");

        update(&mut app, press(KeyCode::Char('x'), KeyModifiers::CONTROL));
        assert_eq!(app.diff_view().unwrap().search_query, "n");
    }

    #[test]
    fn a_tick_does_not_fetch_the_highlighted_pipelines_jobs_but_a_new_list_does() {
        let _guard = crate::config::lock_test_env();
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
    fn a_refetched_child_level_drops_the_cached_jobs_of_children_whose_status_changed() {
        let _guard = crate::config::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let level = |children| crate::domain::pipelines::ChildLevel {
            children,
            pending: Vec::new(),
        };
        let mut app = app_with_client();
        app.active_tab = Tab::Pipelines;
        app.pipelines.items = vec![pipeline(1002, "running")];
        app.pipelines.state.select(Some(0));
        app.descend_into(
            1002,
            level(vec![pipeline(2001, "running"), pipeline(2002, "success")]),
        );
        for id in [2001, 2002] {
            app.pipeline_jobs.insert(id, Vec::new());
        }

        update(
            &mut app,
            Event::ChildLevelFetched(
                1002,
                level(vec![pipeline(2001, "failed"), pipeline(2002, "success")]),
            ),
        );

        let cached: Vec<u64> = app.pipeline_jobs.keys().copied().collect();
        assert_eq!(cached, vec![2002]);
        assert_eq!(app.pipelines.state.selected(), Some(0));
        assert_eq!(
            app.fetching_pipelines,
            std::collections::HashSet::from([2001])
        );
    }

    #[test]
    fn entering_a_pipeline_again_drops_the_cached_jobs_of_its_children() {
        let _guard = crate::config::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let opened = |status: &str| Event::PipelineOpened {
            pipeline_id: 1002,
            opening: crate::handlers::tabs::PipelineOpening::Descend,
            result: Ok(crate::handlers::tabs::PipelineContents::Downstream(
                crate::domain::pipelines::ChildLevel {
                    children: vec![pipeline(2001, status)],
                    pending: Vec::new(),
                },
            )),
        };
        let mut app = app_with_client();
        app.active_tab = Tab::Pipelines;
        app.pipelines.items = vec![pipeline(1002, "running")];
        app.pipelines.state.select(Some(0));
        update(&mut app, opened("running"));
        update(&mut app, Event::PipelineJobs(2001, Vec::new()));
        assert!(app.ascend());

        update(&mut app, opened("failed"));

        assert!(!app.pipeline_jobs.contains_key(&2001));
        assert_eq!(
            app.fetching_pipelines,
            std::collections::HashSet::from([2001])
        );
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
    fn keys_typed_into_a_dialog_field_reach_the_field_not_quit_or_help() {
        use crate::form::{Button, ButtonRow, ButtonTone, Form, Group, GroupKind, LineInput};
        let mut app = App::default();
        let mut dialog =
            crate::app::SubmitDialog::build(crate::app::ConfirmAction::RebaseMr(12), &app);
        dialog.form = Form::new(
            "Rebase",
            vec![Group::new(GroupKind::LineInput(LineInput::new(
                String::new(),
                None,
            )))],
            ButtonRow::new(
                Button::new("Rebase", ButtonTone::Positive),
                [],
                Button::new("Cancel", ButtonTone::Neutral),
            ),
        );
        app.open_submit_dialog(dialog);
        update(&mut app, press(KeyCode::Tab, KeyModifiers::NONE));
        assert!(app.is_capturing_text());

        let quit = update(&mut app, press(KeyCode::Char('q'), KeyModifiers::NONE));
        update(&mut app, press(KeyCode::Char('?'), KeyModifiers::NONE));

        assert!(quit.is_empty(), "{quit:?}");
        assert!(!app.show_help());
        let typed = app.submit_dialog().and_then(|dialog| {
            dialog
                .form
                .groups()
                .iter()
                .find_map(|group| match &group.kind {
                    GroupKind::LineInput(input) => Some(input.buffer.value().to_string()),
                    _ => None,
                })
        });
        assert_eq!(typed.as_deref(), Some("q?"));
    }

    #[test]
    fn q_in_the_diff_view_closes_the_diff_instead_of_quitting() {
        let mut app = with_open_diff(App::default());

        let effects = update(&mut app, press(KeyCode::Char('q'), KeyModifiers::NONE));

        assert!(!effects.iter().any(|effect| matches!(effect, Effect::Quit)));
        assert!(app.diff_view().is_none());
    }

    #[test]
    fn q_on_a_confirmation_dialog_keeps_it_open_instead_of_quitting() {
        let mut app = App::default();
        app.open_submit_dialog(crate::app::SubmitDialog::build(
            crate::app::ConfirmAction::MarkAllTodosRead,
            &app,
        ));

        let effects = update(&mut app, press(KeyCode::Char('q'), KeyModifiers::NONE));

        assert!(!effects.iter().any(|effect| matches!(effect, Effect::Quit)));
        assert!(app.submit_dialog().is_some());
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

    fn pipeline(id: u64, status: &str) -> crate::domain::pipelines::Pipeline {
        serde_json::from_value(serde_json::json!({
            "id": id, "status": status, "ref": "main", "updated_at": "2026-01-01T00:00:00Z"
        }))
        .expect("pipeline fixture")
    }

    fn job(id: u64) -> crate::domain::pipelines::Job {
        serde_json::from_value(serde_json::json!({
            "id": id, "status": "running", "stage": "test", "name": format!("job-{id}")
        }))
        .expect("job fixture")
    }

    fn jobs(ids: &[u64]) -> Vec<crate::domain::pipelines::Job> {
        ids.iter().copied().map(job).collect()
    }

    fn open_log() -> crate::ui::job_trace::JobTraceView {
        crate::ui::job_trace::JobTraceView::new(crate::domain::job_trace::JobTrace::from_read(
            b"line 1\n",
            false,
        ))
    }

    /// The Jobs tab showing pipeline 7 with job 71 selected and its log open.
    fn jobs_tab_showing_pipeline_7() -> App {
        let mut app = app_with_client();
        app.active_tab = Tab::Jobs;
        app.active_pipeline_id = Some(7);
        app.jobs.items = jobs(&[70, 71]);
        app.jobs.state.select(Some(1));
        app.job_trace = Some(open_log());
        app
    }

    #[test]
    fn a_refresh_of_the_shown_pipeline_keeps_the_selected_job_and_its_log() {
        let mut app = jobs_tab_showing_pipeline_7();

        update(
            &mut app,
            Event::JobsTabFetched(JobsSource::Branch, 7, jobs(&[69, 70, 71])),
        );

        assert_eq!(app.selected_job().map(|job| job.id), Some(71));
        assert!(app.job_trace.is_some());
    }

    #[test]
    fn a_refresh_that_no_longer_lists_the_selected_job_closes_its_log() {
        let mut app = jobs_tab_showing_pipeline_7();

        update(
            &mut app,
            Event::JobsTabFetched(JobsSource::Branch, 7, jobs(&[70])),
        );

        assert_eq!(app.selected_job().map(|job| job.id), Some(70));
        assert!(app.job_trace.is_none());
    }

    #[test]
    fn a_retried_job_stays_selected_under_its_new_id_and_its_old_log_closes() {
        let mut app = jobs_tab_showing_pipeline_7();
        let mut retry = job(72);
        retry.name = "job-71".to_string();

        update(
            &mut app,
            Event::JobsTabFetched(JobsSource::Branch, 7, vec![job(70), retry]),
        );

        assert_eq!(app.selected_job().map(|job| job.id), Some(72));
        assert!(app.job_trace.is_none());
    }

    #[test]
    fn a_new_branch_pipeline_replaces_the_shown_one() {
        let mut app = jobs_tab_showing_pipeline_7();

        update(
            &mut app,
            Event::JobsTabFetched(JobsSource::Branch, 8, jobs(&[80, 81])),
        );

        assert_eq!(app.active_pipeline_id, Some(8));
        assert_eq!(app.selected_job().map(|job| job.id), Some(80));
        assert!(app.job_trace.is_none());
    }

    /// A pipeline opened from the Pipelines tab stays on the Jobs tab across
    /// a refresh and across a branch reply sent before it was opened.
    #[test]
    fn a_pipeline_opened_from_the_pipelines_tab_stays_open_across_refreshes() {
        let mut app = app_with_client();
        app.active_tab = Tab::Pipelines;
        app.pipelines.items = vec![
            serde_json::from_str(
                r#"{"id": 7, "status": "running", "ref": "main", "updated_at": "2026-01-01T00:00:00Z"}"#,
            )
            .expect("pipeline fixture"),
        ];
        app.pipelines.state.select(Some(0));

        update(
            &mut app,
            Event::PipelineOpened {
                pipeline_id: 7,
                opening: crate::handlers::tabs::PipelineOpening::Descend,
                result: Ok(crate::handlers::tabs::PipelineContents::Jobs(jobs(&[70]))),
            },
        );
        update(
            &mut app,
            Event::JobsTabFetched(JobsSource::Branch, 3, jobs(&[30])),
        );

        let opened = app
            .gitlab_client
            .as_ref()
            .and_then(|client| client.opened_pipeline.as_ref())
            .map(|pipeline| pipeline.id);
        assert_eq!(opened, Some(7));
        assert_eq!(app.active_pipeline_id, Some(7));
        assert_eq!(app.selected_job().map(|job| job.id), Some(70));
    }

    #[test]
    fn a_pipeline_picked_on_the_jobs_tab_replaces_the_opened_one() {
        let mut app = jobs_tab_showing_pipeline_7();
        if let Some(client) = app.gitlab_client.as_mut() {
            client.opened_pipeline = Some(Box::new(crate::domain::client::OpenedPipeline {
                id: 7,
                project: "group/project".to_string(),
                parents: Vec::new(),
            }));
        }
        app.open_selector(crate::app::Selector {
            title: " Select Pipeline ".to_string(),
            all_items: vec!["#9 — main".to_string()],
            selected_items: Default::default(),
            cursor_idx: 0,
            search_query: String::new(),
            is_filtering: false,
            is_loading: false,
            entity_iid: 0,
            entity_type: String::new(),
            field_type: "pipeline_select".to_string(),
            multi_select: false,
            state: ratatui::widgets::ListState::default(),
            request: None,
        });

        update(&mut app, press(KeyCode::Enter, KeyModifiers::NONE));
        update(
            &mut app,
            Event::JobsTabFetched(JobsSource::Opened(9), 9, jobs(&[90])),
        );

        let opened = app
            .gitlab_client
            .as_ref()
            .and_then(|client| client.opened_pipeline.as_ref())
            .map(|pipeline| pipeline.id);
        assert_eq!(opened, Some(9));
        assert_eq!(app.active_pipeline_id, Some(9));
        assert_eq!(app.selected_job().map(|job| job.id), Some(90));
    }

    #[test]
    fn a_retried_job_is_found_in_its_own_stage() {
        let mut app = jobs_tab_showing_pipeline_7();
        let in_stage = |id, name: &str, stage: &str| {
            let mut job = job(id);
            job.name = name.to_string();
            job.stage = stage.to_string();
            job
        };
        app.jobs.items = vec![
            in_stage(70, "deploy", "staging"),
            in_stage(71, "deploy", "production"),
        ];
        app.jobs.state.select(Some(1));

        update(
            &mut app,
            Event::JobsTabFetched(
                JobsSource::Branch,
                7,
                vec![
                    in_stage(70, "deploy", "staging"),
                    in_stage(72, "deploy", "production"),
                ],
            ),
        );

        assert_eq!(app.selected_job().map(|job| job.id), Some(72));
    }

    #[test]
    fn opening_a_pipeline_without_jobs_of_its_own_leaves_the_jobs_tab_alone() {
        let mut app = jobs_tab_showing_pipeline_7();
        app.active_tab = Tab::Pipelines;
        app.pipelines.items = vec![
            serde_json::from_str(
                r#"{"id": 9, "status": "running", "ref": "main", "updated_at": "2026-01-01T00:00:00Z"}"#,
            )
            .expect("pipeline fixture"),
        ];

        update(
            &mut app,
            Event::PipelineOpened {
                pipeline_id: 9,
                opening: crate::handlers::tabs::PipelineOpening::OwnJobs,
                result: Ok(crate::handlers::tabs::PipelineContents::Jobs(Vec::new())),
            },
        );

        let opened = app
            .gitlab_client
            .as_ref()
            .and_then(|client| client.opened_pipeline.as_ref());
        assert!(opened.is_none());
        assert_eq!(app.active_pipeline_id, Some(7));
        assert_eq!(app.selected_job().map(|job| job.id), Some(71));
    }

    fn opened_pipeline_id(app: &App) -> Option<u64> {
        app.gitlab_client
            .as_ref()
            .and_then(|client| client.opened_pipeline.as_ref())
            .map(|pipeline| pipeline.id)
    }

    #[test]
    fn esc_back_to_the_pipelines_tab_ends_the_opened_pipeline() {
        let mut app = jobs_tab_showing_pipeline_7();
        app.open_pipeline_in_jobs_tab(7);
        app.complete_loading_tab(Tab::Jobs, "Success");
        app.job_trace = None;
        app.detail_visible = false;
        app.details_zoomed = false;

        update(&mut app, press(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(app.active_tab, Tab::Pipelines);
        assert_eq!(opened_pipeline_id(&app), None);
        assert!(
            !app.refreshed_tabs.contains(&Tab::Jobs),
            "the jobs still listed belong to the pipeline that was open"
        );
    }

    /// The Jobs tab showing opened pipeline 7, left with `Esc` while a
    /// refresh of it is in flight.
    fn pipeline_7_closed_while_its_refresh_runs() -> App {
        let mut app = jobs_tab_showing_pipeline_7();
        app.open_pipeline_in_jobs_tab(7);
        app.complete_loading_tab(Tab::Jobs, "Success");
        app.start_loading_tab(Tab::Jobs);
        app.job_trace = None;
        app.detail_visible = false;
        app.details_zoomed = false;
        update(&mut app, press(KeyCode::Esc, KeyModifiers::NONE));
        app
    }

    #[test]
    fn a_reply_for_a_pipeline_closed_meanwhile_changes_nothing() {
        let mut app = pipeline_7_closed_while_its_refresh_runs();

        update(
            &mut app,
            Event::JobsTabFetched(JobsSource::Opened(7), 7, jobs(&[72])),
        );

        assert!(
            !app.loaded_tabs.contains(&Tab::Jobs),
            "the next visit fetches the branch pipeline"
        );
        assert!(!app.refreshed_tabs.contains(&Tab::Jobs));
        let shown: Vec<u64> = app.jobs.items.iter().map(|job| job.id).collect();
        assert_eq!(shown, vec![70, 71]);
    }

    #[test]
    fn returning_to_the_jobs_tab_after_esc_fetches_the_branch_pipeline() {
        let mut app = pipeline_7_closed_while_its_refresh_runs();
        assert!(!app.loading_tabs.contains(&Tab::Jobs));

        app.active_tab = Tab::Jobs;
        let effects = update(&mut app, press(KeyCode::F(5), KeyModifiers::NONE));

        assert!(app.loading_tabs.contains(&Tab::Jobs));
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Spawn(_)))
        );
    }

    #[test]
    fn a_branch_reply_dropped_after_opening_a_pipeline_leaves_the_tab_refreshable() {
        let mut app = app_with_client();
        app.start_loading_tab(Tab::Jobs);
        app.active_tab = Tab::Pipelines;
        app.pipelines.items = vec![
            serde_json::from_str(
                r#"{"id": 7, "status": "running", "ref": "main", "updated_at": "2026-01-01T00:00:00Z"}"#,
            )
            .expect("pipeline fixture"),
        ];
        app.pipelines.state.select(Some(0));
        update(
            &mut app,
            Event::PipelineOpened {
                pipeline_id: 7,
                opening: crate::handlers::tabs::PipelineOpening::Descend,
                result: Ok(crate::handlers::tabs::PipelineContents::Jobs(jobs(&[70]))),
            },
        );

        update(
            &mut app,
            Event::JobsTabFetched(JobsSource::Branch, 3, jobs(&[30])),
        );
        assert!(!app.loading_tabs.contains(&Tab::Jobs));
        let effects = update(&mut app, press(KeyCode::F(5), KeyModifiers::NONE));

        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Spawn(_)))
        );
        assert_eq!(app.selected_job().map(|job| job.id), Some(70));
    }

    #[test]
    fn the_jobs_tab_title_names_the_opened_pipeline_and_its_descent() {
        let mut app = app_with_client();
        app.pipelines.items = vec![pipeline(1002, "running")];
        assert_eq!(
            app.jobs_breadcrumb(),
            "",
            "the branch pipeline needs no path"
        );

        app.descend_into(
            1002,
            crate::domain::pipelines::ChildLevel {
                children: vec![pipeline(2001, "running")],
                pending: Vec::new(),
            },
        );
        app.open_pipeline_in_jobs_tab(2001);

        assert_eq!(app.jobs_breadcrumb(), "#1002 › #2001");
    }

    #[test]
    fn a_pipelines_tab_preview_leaves_the_jobs_tab_rows_alone() {
        let _home = crate::config::isolate_home();
        let mut app = jobs_tab_showing_pipeline_7();
        app.open_pipeline_in_jobs_tab(7);
        app.active_tab = Tab::Pipelines;
        app.pipelines.items = vec![
            serde_json::from_str(
                r#"{"id": 9, "status": "running", "ref": "main", "updated_at": "2026-01-01T00:00:00Z"}"#,
            )
            .expect("pipeline fixture"),
        ];
        app.pipelines.state.select(Some(0));

        update(&mut app, Event::PipelineJobs(9, jobs(&[90])));

        let shown: Vec<u64> = app.jobs.items.iter().map(|job| job.id).collect();
        assert_eq!(shown, vec![70, 71]);
        assert!(app.pipeline_jobs.contains_key(&9));
    }

    #[test]
    fn esc_on_the_pipelines_tab_leaves_the_jobs_tab_rows_and_log_alone() {
        let mut app = jobs_tab_showing_pipeline_7();
        app.active_tab = Tab::Pipelines;
        app.detail_visible = true;
        app.details_zoomed = true;

        update(&mut app, press(KeyCode::Esc, KeyModifiers::NONE));
        update(&mut app, press(KeyCode::Esc, KeyModifiers::NONE));
        update(&mut app, press(KeyCode::Esc, KeyModifiers::NONE));

        let shown: Vec<u64> = app.jobs.items.iter().map(|job| job.id).collect();
        assert_eq!(shown, vec![70, 71]);
        assert_eq!(app.selected_job().map(|job| job.id), Some(71));
        assert!(app.job_trace.is_some());
    }

    #[test]
    fn a_job_log_arriving_on_the_pipelines_tab_is_not_shown_there() {
        let mut app = jobs_tab_showing_pipeline_7();
        app.job_trace = None;
        app.job_trace_loading = true;
        app.active_tab = Tab::Pipelines;

        update(
            &mut app,
            Event::JobTraceFetched(
                71,
                Ok(crate::domain::job_trace::JobTrace::from_read(
                    b"line 1\n",
                    false,
                )),
            ),
        );

        assert!(app.job_trace.is_none());
        assert!(!app.details_zoomed);
        assert!(!app.job_trace_loading);
    }

    /// The Jobs tab with job 71's log open and followed, shown beside the
    /// job list.
    fn jobs_tab_following_job_71() -> App {
        let mut app = jobs_tab_showing_pipeline_7();
        app.job_trace_follow = true;
        app.detail_visible = true;
        app.details_zoomed = false;
        app
    }

    #[test]
    fn esc_on_the_zoomed_job_log_closes_it_and_ends_follow_mode() {
        let mut app = jobs_tab_following_job_71();
        app.details_zoomed = true;

        update(&mut app, press(KeyCode::Esc, KeyModifiers::NONE));

        assert!(app.job_trace.is_none());
        assert!(!app.job_trace_follow, "the next log opens unfollowed");
    }

    #[test]
    fn esc_on_the_jobs_tab_with_the_detail_pane_hidden_closes_the_log_and_ends_follow_mode() {
        let mut app = jobs_tab_following_job_71();
        app.detail_visible = false;
        app.detail_scroll_to_bottom = true;
        app.job_trace_scroll_to_line = Some(3);

        update(&mut app, press(KeyCode::Esc, KeyModifiers::NONE));

        assert!(app.job_trace.is_none());
        assert!(!app.job_trace_follow, "the next log opens unfollowed");
        assert!(
            !app.detail_scroll_to_bottom,
            "the next pane opens at its top"
        );
        assert_eq!(
            app.job_trace_scroll_to_line, None,
            "the next log does not jump to a line of this one"
        );
    }

    #[test]
    fn a_refresh_that_retried_the_followed_job_ends_follow_mode() {
        let mut app = jobs_tab_following_job_71();
        let mut retry = job(72);
        retry.name = "job-71".to_string();

        update(
            &mut app,
            Event::JobsTabFetched(JobsSource::Branch, 7, vec![job(70), retry]),
        );

        assert!(app.job_trace.is_none());
        assert!(!app.job_trace_follow, "the next log opens unfollowed");
    }

    #[test]
    fn a_new_branch_pipeline_ends_follow_mode_of_the_replaced_log() {
        let mut app = jobs_tab_following_job_71();

        update(
            &mut app,
            Event::JobsTabFetched(JobsSource::Branch, 8, jobs(&[80, 81])),
        );

        assert!(app.job_trace.is_none());
        assert!(!app.job_trace_follow, "the next log opens unfollowed");
    }

    #[test]
    fn opening_a_pipeline_ends_follow_mode_of_the_replaced_log() {
        let mut app = jobs_tab_following_job_71();
        app.active_tab = Tab::Pipelines;
        app.pipelines.items = vec![
            serde_json::from_str(
                r#"{"id": 9, "status": "running", "ref": "main", "updated_at": "2026-01-01T00:00:00Z"}"#,
            )
            .expect("pipeline fixture"),
        ];
        app.pipelines.state.select(Some(0));

        update(
            &mut app,
            Event::PipelineOpened {
                pipeline_id: 9,
                opening: crate::handlers::tabs::PipelineOpening::Descend,
                result: Ok(crate::handlers::tabs::PipelineContents::Jobs(jobs(&[90]))),
            },
        );

        assert!(app.job_trace.is_none());
        assert!(!app.job_trace_follow, "the next log opens unfollowed");
    }

    fn dashboard_with_failing_issues() -> App {
        let mut app = app_with_client();
        app.config.dashboard.enabled = true;
        app.scope = crate::scope::Scope::Repository("group/project".to_string());
        app.active_tab = Tab::Dashboard;
        update(
            &mut app,
            Event::FetchFailed(Tab::Issues, "issues are disabled".to_string()),
        );
        app.show_error("newer error".to_string());
        app
    }

    fn toast_text(app: &App) -> Option<&str> {
        app.toast_notice().map(|notice| notice.text())
    }

    #[test]
    fn a_failure_the_dashboard_card_already_shows_raises_no_second_toast() {
        let mut app = dashboard_with_failing_issues();

        update(
            &mut app,
            Event::FetchFailed(Tab::Issues, "issues are disabled".to_string()),
        );
        assert_eq!(toast_text(&app), Some("newer error"));

        update(
            &mut app,
            Event::FetchFailed(Tab::Issues, "HTTP 500".to_string()),
        );
        assert_eq!(toast_text(&app), Some("HTTP 500"));
    }

    #[test]
    fn a_full_dashboard_refresh_reports_an_unchanged_failure_again() {
        let mut app = dashboard_with_failing_issues();
        let client = app.gitlab_client.clone().expect("a client");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let tx = app.scoped_sender(tx);
        crate::fetch::spawn_refresh_dashboard(
            &mut crate::effect::Effects::default(),
            &client,
            &mut app,
            crate::dashboard::DashboardRefresh::Full,
            tx,
        );

        update(
            &mut app,
            Event::FetchFailed(Tab::Issues, "issues are disabled".to_string()),
        );
        assert_eq!(toast_text(&app), Some("issues are disabled"));
    }

    #[test]
    fn an_unchanged_failure_of_the_list_on_screen_is_still_raised() {
        let mut app = dashboard_with_failing_issues();
        app.active_tab = Tab::Issues;

        update(
            &mut app,
            Event::FetchFailed(Tab::Issues, "issues are disabled".to_string()),
        );
        assert_eq!(toast_text(&app), Some("issues are disabled"));
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
        let content_area = ratatui::layout::Rect::new(0, 5, 80, 20);
        app.content_rect = Some(content_area);
        app.layout = Some(crate::ui::layout::LayoutPlan {
            content_area,
            ..Default::default()
        });

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

    #[test]
    fn sidebar_clicks_switch_tabs_inside_the_border_only() {
        use crossterm::event::MouseEventKind;
        let click = |row: u16, column: u16| {
            Event::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            })
        };
        let mut app = app_with_client();
        app.active_tab = Tab::Issues;
        let sidebar_rect = ratatui::layout::Rect::new(0, 0, 22, 12);
        app.layout = Some(crate::ui::layout::LayoutPlan {
            sidebar_rect,
            ..Default::default()
        });

        update(&mut app, click(0, 5));
        assert_eq!(app.active_tab, Tab::Issues, "top border row");
        update(&mut app, click(11, 5));
        assert_eq!(app.active_tab, Tab::Issues, "bottom border row");
        update(&mut app, click(2, 0));
        assert_eq!(app.active_tab, Tab::Issues, "left border column");

        update(&mut app, click(2, 5));
        assert_eq!(app.active_tab, Tab::MergeRequests);
    }
}
