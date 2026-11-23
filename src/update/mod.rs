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

    const README_DIFF: &str = "diff --git a/README.md b/README.md\n--- a/README.md\n\
                               +++ b/README.md\n@@ -1,1 +1,1 @@\n-old\n+new\n";

    fn readme_diff() -> crate::app::DiffView {
        crate::app::DiffView::new(42, "org/project".to_string(), README_DIFF.to_string())
    }

    fn diff_fetched(
        viewed_files: Option<Result<Box<crate::domain::viewed_files::PrViewedFiles>, String>>,
    ) -> Event {
        Event::DiffFetched {
            diff_view: Box::new(readme_diff()),
            comments: Ok(vec![]),
            viewed_files,
            fetched_at: crate::domain::viewed_files::SettleCount::default(),
        }
    }

    fn github_diff_fetched(pull_request_id: &str) -> Event {
        diff_fetched_with(README_DIFF, pull_request_id)
    }

    fn diff_fetched_with(raw_diff: &str, pull_request_id: &str) -> Event {
        Event::DiffFetched {
            diff_view: Box::new(crate::app::DiffView::new(
                42,
                "org/project".to_string(),
                raw_diff.to_string(),
            )),
            comments: Ok(vec![]),
            viewed_files: Some(Ok(Box::new(crate::domain::viewed_files::PrViewedFiles {
                pull_request_id: pull_request_id.to_string(),
                files: std::collections::HashMap::new(),
            }))),
            fetched_at: crate::domain::viewed_files::SettleCount::default(),
        }
    }

    #[test]
    fn a_failed_viewed_state_fetch_raises_one_error_and_keeps_the_local_marks() {
        let _guard = crate::config::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let mut app = App::default();
        let marks = readme_diff().marks_on_this_diff(&["README.md"]);
        app.store_review_marks("org/project", 42, marks);

        update(&mut app, diff_fetched(Some(Err("HTTP 403".to_string()))));

        let diff_view = app.diff_view().expect("diff opens");
        assert!(diff_view.reviewed_files.contains("README.md"));
        assert_eq!(diff_view.pull_request_id, None);
        assert!(app.viewed_syncs.is_empty());
        assert_eq!(
            app.toast_notice().map(crate::notice::Notice::text),
            Some(
                "Failed to load GitHub viewed state, so marks stay local until the next open \
                 replaces them with GitHub's (sync_viewed_files = false keeps them local): \
                 HTTP 403"
            )
        );
    }

    fn app_with_a_draft_off_the_readme_diff() -> App {
        let mut app = App::default();
        app.draft_comments = vec![crate::domain::review::DraftComment {
            file_path: "README.md".to_string(),
            line_num: Some(9),
            old_line_num: None,
            end_line_num: None,
            end_old_line_num: None,
            body: "nit".to_string(),
        }];
        app.persist_draft_comments("org/project", 42);
        app
    }

    #[test]
    fn a_failed_viewed_state_fetch_keeps_the_held_back_drafts_in_its_toast() {
        let _guard = crate::config::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let mut app = app_with_a_draft_off_the_readme_diff();

        update(&mut app, diff_fetched(Some(Err("HTTP 403".to_string()))));

        let toast = app.toast_notice().expect("an error toast");
        assert_eq!(
            toast.text(),
            "1 draft comment no longer matches the diff and was set aside. Failed to load \
             GitHub viewed state, so marks stay local until the next open replaces them with \
             GitHub's (sync_viewed_files = false keeps them local): HTTP 403"
        );
        assert_eq!(toast.detail(), Some(crate::notice::CLI_LOG_HINT));
    }

    #[test]
    fn held_back_drafts_alone_do_not_point_at_the_terminal_log() {
        let _guard = crate::config::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let mut app = app_with_a_draft_off_the_readme_diff();

        update(&mut app, diff_fetched(None));

        let toast = app.toast_notice().expect("an error toast");
        assert_eq!(
            toast.text(),
            "1 draft comment no longer matches the diff and was set aside"
        );
        assert_eq!(toast.detail(), None);
    }

    fn flush(app: &mut App, pull_request_id: &str) -> usize {
        spawned(&update(
            app,
            Event::ViewedSyncFlush(pull_request_id.to_string()),
        ))
    }

    #[test]
    fn a_viewed_mark_queued_just_before_closing_the_diff_still_reaches_github() {
        let _guard = crate::config::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let mut app = app_with_client();
        update(&mut app, github_diff_fetched("PR_1"));
        assert!(app.queue_viewed_sync("PR_1", &["README.md".to_string()], true));

        app.close_diff_view();

        assert_eq!(flush(&mut app, "PR_1"), 1);
    }

    #[test]
    fn a_request_sent_before_closing_the_diff_still_holds_back_the_reopened_ones() {
        let _guard = crate::config::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let mut app = app_with_client();
        update(&mut app, github_diff_fetched("PR_1"));
        app.queue_viewed_sync("PR_1", &["README.md".to_string()], true);
        assert_eq!(flush(&mut app, "PR_1"), 1);
        app.close_diff_view();

        update(&mut app, github_diff_fetched("PR_1"));
        assert!(
            app.diff_view()
                .unwrap()
                .reviewed_files
                .contains("README.md"),
            "the mark in flight shows on the reopened diff"
        );
        app.queue_viewed_sync("PR_1", &["README.md".to_string()], false);
        assert_eq!(flush(&mut app, "PR_1"), 0, "the first request is in flight");

        let effects = update(
            &mut app,
            Event::ViewedFilesSynced {
                pull_request_id: "PR_1".to_string(),
                changes: vec![("README.md".to_string(), true)],
                result: Ok(vec![]),
            },
        );
        assert_eq!(spawned(&effects), 1, "the follow-up flush");
        assert_eq!(flush(&mut app, "PR_1"), 1);
    }

    #[test]
    fn a_diff_fetched_before_a_viewed_request_settled_keeps_the_mark_it_left() {
        let _guard = crate::config::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let mut app = app_with_client();
        update(&mut app, github_diff_fetched("PR_1"));
        app.queue_viewed_sync("PR_1", &["README.md".to_string()], true);
        assert_eq!(flush(&mut app, "PR_1"), 1);
        // A refetch reads GitHub before the mark lands and arrives after it.
        let fetched_before_the_mark = github_diff_fetched("PR_1");
        update(
            &mut app,
            Event::ViewedFilesSynced {
                pull_request_id: "PR_1".to_string(),
                changes: vec![("README.md".to_string(), true)],
                result: Ok(vec![]),
            },
        );

        update(&mut app, fetched_before_the_mark);

        assert_eq!(
            app.diff_view().expect("diff stays open").reviewed_files,
            std::collections::HashSet::from(["README.md".to_string()])
        );
        assert_eq!(
            app.review_marks_for_mr("org/project", 42),
            readme_diff().marks_on_this_diff(&["README.md"])
        );
    }

    #[test]
    fn viewed_marks_queued_before_a_scope_switch_still_reach_github() {
        let _guard = crate::config::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let mut app = app_with_client();
        update(&mut app, github_diff_fetched("PR_1"));
        let paths: Vec<String> = (0..=crate::domain::viewed_files::VIEWED_SYNC_BATCH)
            .map(|i| format!("f{i:03}.rs"))
            .collect();
        app.queue_viewed_sync("PR_1", &paths, true);
        assert_eq!(flush(&mut app, "PR_1"), 1, "the first batch");
        app.close_diff_view();

        app.drill_into("org/other".to_string());
        // The new scope's client is still being built.
        app.gitlab_client = None;

        let first_batch = paths[..crate::domain::viewed_files::VIEWED_SYNC_BATCH]
            .iter()
            .map(|path| (path.clone(), true))
            .collect();
        let effects = update(
            &mut app,
            Event::ViewedFilesSynced {
                pull_request_id: "PR_1".to_string(),
                changes: first_batch,
                result: Ok(vec![]),
            },
        );
        assert_eq!(spawned(&effects), 1, "the follow-up flush");
        assert_eq!(flush(&mut app, "PR_1"), 1, "the batch left over");
    }

    fn app_with_a_readme_mark_on_its_way_and_the_diff_closed() -> App {
        let mut app = app_with_client();
        update(&mut app, github_diff_fetched("PR_1"));
        let marks = readme_diff().marks_on_this_diff(&["README.md"]);
        app.store_review_marks("org/project", 42, marks);
        app.queue_viewed_sync("PR_1", &["README.md".to_string()], true);
        assert_eq!(flush(&mut app, "PR_1"), 1);
        app.close_diff_view();
        app
    }

    fn readme_mark_failed() -> Event {
        Event::ViewedFilesSynced {
            pull_request_id: "PR_1".to_string(),
            changes: vec![("README.md".to_string(), true)],
            result: Err("HTTP 502".to_string()),
        }
    }

    #[test]
    fn a_viewed_mark_that_failed_after_its_diff_closed_drops_its_saved_mark() {
        let _guard = crate::config::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let mut app = app_with_a_readme_mark_on_its_way_and_the_diff_closed();

        update(&mut app, readme_mark_failed());

        assert_eq!(
            app.review_marks_for_mr("org/project", 42),
            std::collections::HashMap::new()
        );
    }

    #[test]
    fn a_viewed_mark_that_failed_reverts_on_a_diff_reopened_without_github_state() {
        let _guard = crate::config::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let mut app = app_with_a_readme_mark_on_its_way_and_the_diff_closed();
        update(&mut app, diff_fetched(Some(Err("HTTP 403".to_string()))));
        let reopened = app.diff_view().expect("diff opens");
        assert_eq!(reopened.pull_request_id, None);
        assert!(reopened.reviewed_files.contains("README.md"));

        update(&mut app, readme_mark_failed());

        let diff_view = app.diff_view().expect("diff stays open");
        assert_eq!(diff_view.reviewed_files, std::collections::HashSet::new());
        assert_eq!(
            app.review_marks_for_mr("org/project", 42),
            std::collections::HashMap::new()
        );
    }

    #[test]
    fn a_partly_refused_viewed_sync_reverts_only_the_refused_marks() {
        let _guard = crate::config::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let mut app = app_with_client();
        update(
            &mut app,
            diff_fetched_with(
                "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n\
                 diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n",
                "PR_1",
            ),
        );
        let both = vec!["a.rs".to_string(), "b.rs".to_string()];
        app.queue_viewed_sync("PR_1", &both, true);
        app.diff_view_mut()
            .expect("diff opens")
            .apply_review_corrections(&[("a.rs".to_string(), true), ("b.rs".to_string(), true)]);
        assert_eq!(flush(&mut app, "PR_1"), 1);

        update(
            &mut app,
            Event::ViewedFilesSynced {
                pull_request_id: "PR_1".to_string(),
                changes: vec![("a.rs".to_string(), true), ("b.rs".to_string(), true)],
                result: Ok(vec![crate::domain::viewed_files::RejectedViewedPath {
                    path: "b.rs".to_string(),
                    reason: "Path is not part of the pull request".to_string(),
                }]),
            },
        );

        let diff_view = app.diff_view().expect("diff stays open");
        assert_eq!(
            diff_view.reviewed_files,
            std::collections::HashSet::from(["a.rs".to_string()])
        );
        assert_eq!(
            app.toast_notice().map(crate::notice::Notice::text),
            Some(
                "GitHub refused the viewed state of 1 of 2 file(s), reverted: b.rs: Path is not \
                 part of the pull request"
            )
        );
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
    fn a_viewed_sync_result_for_another_projects_pull_request_with_the_same_number_is_dropped() {
        let _guard = crate::config::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let mut app = app_with_client();
        let mut diff_view = crate::app::DiffView::new(
            42,
            "org/project-b".to_string(),
            "diff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n\
             @@ -1,1 +1,1 @@\n-old\n+new\n"
                .to_string(),
        );
        diff_view.restore_review_state(
            &std::collections::HashMap::new(),
            Some(crate::domain::viewed_files::PrViewedFiles {
                pull_request_id: "PR_B".to_string(),
                files: std::collections::HashMap::new(),
            }),
            false,
        );
        app.reload_viewed_sync(
            &mut diff_view,
            crate::domain::viewed_files::SettleCount::default(),
        );
        app.open_diff_view(diff_view);

        // Project A's #42 marked README.md and was closed before it landed.
        update(
            &mut app,
            Event::ViewedFilesSynced {
                pull_request_id: "PR_A".to_string(),
                changes: vec![("README.md".to_string(), true)],
                result: Ok(vec![]),
            },
        );

        let diff_view = app.diff_view().expect("diff stays open");
        assert!(diff_view.reviewed_files.is_empty());
        let entry = app.viewed_syncs.get_mut("PR_B").expect("GitHub sync");
        entry.sync.queue(&["README.md".to_string()], true);
        assert_eq!(
            entry.sync.take_changes(),
            vec![("README.md".to_string(), true)],
            "project B's own mark still reaches GitHub"
        );
    }

    #[test]
    fn opening_another_projects_pull_request_with_the_same_number_leaves_its_marks_alone() {
        let _guard = crate::config::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::EnvGuard::isolate_home(home.path());
        let mut app = App::default();
        let diff = |added: &str| {
            format!(
                "diff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n\
                 @@ -1,1 +1,1 @@\n-old\n+{added}\n"
            )
        };
        let reviewed = crate::app::DiffView::new(42, "org/project-a".to_string(), diff("new"));
        let marks = reviewed.marks_on_this_diff(&["README.md"]);
        app.store_review_marks("org/project-a", 42, marks.clone());

        update(
            &mut app,
            Event::DiffFetched {
                diff_view: Box::new(crate::app::DiffView::new(
                    42,
                    "org/project-b".to_string(),
                    diff("other"),
                )),
                comments: Ok(vec![]),
                viewed_files: None,
                fetched_at: crate::domain::viewed_files::SettleCount::default(),
            },
        );

        assert_eq!(app.review_marks_for_mr("org/project-a", 42), marks);
        let diff_view = app.diff_view().expect("diff opens");
        assert!(diff_view.reviewed_files.is_empty());
        assert!(diff_view.changed_since_review.is_empty());
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
                viewed_files: None,
                fetched_at: crate::domain::viewed_files::SettleCount::default(),
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
                viewed_files: None,
                fetched_at: crate::domain::viewed_files::SettleCount::default(),
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
