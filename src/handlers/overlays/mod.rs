//! Key handling of the overlays drawn over a tab, one module per overlay.

pub mod configure;
pub mod date_picker;
pub mod edit_menu;
pub mod help;
pub mod review_threads;
pub mod selector;
pub mod submit_dialog;
pub mod switch_repo;
pub mod text_input;

pub use date_picker::handle_date_picker;
pub use help::{handle_help_keybinding, handle_help_overlay, move_help_selection};
pub use review_threads::handle_review_threads;
pub use submit_dialog::handle_submit_dialog;
pub use switch_repo::handle_switch_repo;

#[cfg(test)]
mod tests {
    use super::{
        handle_help_keybinding, handle_help_overlay, handle_review_threads, handle_submit_dialog,
    };
    use crate::app::{
        App, ConfirmAction, DiffView, EditEntityKind, EditMenu, Selector, SubmitDialog,
    };
    use crate::keybinding::KeyPress;
    use crate::notice::Notice;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn help_ctrl_d_u_move_half_a_page_without_typing_into_the_search() {
        let mut app = App::default();
        app.show_help = true;
        let ctrl = |c| KeyPress::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));

        handle_help_overlay(&mut app, &ctrl('d'));
        handle_help_overlay(&mut app, &ctrl('d'));
        assert_eq!(app.help_selected, 10);
        handle_help_overlay(&mut app, &ctrl('u'));
        assert_eq!(app.help_selected, 5);
        assert!(app.help_search_query.is_empty());
    }

    #[test]
    fn help_arrows_move_the_selection_and_typing_filters_from_the_top() {
        use crossterm::event::KeyModifiers;
        let mut app = App::default();
        app.show_help = true;
        let press = |code| KeyPress::Key(KeyEvent::new(code, KeyModifiers::NONE));

        handle_help_overlay(&mut app, &press(KeyCode::Down));
        handle_help_overlay(&mut app, &press(KeyCode::Down));
        handle_help_overlay(&mut app, &press(KeyCode::PageDown));
        handle_help_overlay(&mut app, &press(KeyCode::Up));
        assert_eq!(app.help_selected, 11);

        handle_help_overlay(&mut app, &press(KeyCode::Char('j')));
        assert_eq!(app.help_search_query, "j", "letters still filter");
        assert_eq!(app.help_selected, 0, "a new filter starts from the top");
    }

    #[test]
    fn help_search_consumes_q_instead_of_closing_or_quitting() {
        let mut app = App::default();
        app.show_help = true;

        let handled = handle_help_overlay(
            &mut app,
            &KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE).into(),
        );

        assert!(handled);
        assert_eq!(app.help_search_query, "q");
        assert!(app.show_help);
    }

    #[test]
    fn search_mode_blocks_switch_repo_and_refresh() {
        let tx = App::default().tx;
        let mut app = App::default();
        app.is_typing_search = true;

        let handled_refresh = crate::handlers::refresh::handle_refresh(
            &mut app,
            &KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE).into(),
            &tx,
            &mut crate::effect::Effects::default(),
        );
        assert!(!handled_refresh);

        let handled_switch = super::handle_switch_repo(
            &mut app,
            &KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE).into(),
        );
        assert!(!handled_switch);
    }

    #[test]
    fn help_keybinding_accessible_in_all_views() {
        let mut app = App::default();

        // 1. Normal view with '?' and F1
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE).into(),
        ));
        assert!(app.show_help);
        app.show_help = false;

        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE).into(),
        ));
        assert!(app.show_help);
        app.show_help = false;

        // 2. Edit menu (Inspector / Form) - not actively editing text
        app.edit_menu = Some(EditMenu {
            entity_project: String::new(),
            entity_iid: 1,
            entity_kind: EditEntityKind::EditIssue,
            title: "Edit Issue".to_string(),
            fields: vec![],
            initial_fields: std::collections::HashMap::new(),
            selected_idx: 0,
            editing: false,
            cursor_pos: 0,
            state: ratatui::widgets::ListState::default(),
            desc_scroll: 0,
            workflow_inputs: vec![],
            source_on_forge: false,
        });
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE).into(),
        ));
        assert!(app.show_help);
        app.show_help = false;

        // Edit menu - actively editing text: '?' types into field, but F1 opens help
        if let Some(ref mut menu) = app.edit_menu {
            menu.editing = true;
        }
        assert!(!handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE).into(),
        ));
        assert!(!app.show_help);
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE).into(),
        ));
        assert!(app.show_help);
        app.show_help = false;
        app.edit_menu = None;

        // 3. Diff View - not searching
        app.diff_view = Some(DiffView::new(
            1,
            String::new(),
            "diff --git a/a b/b\n--- a/a\n+++ b/b\n@@ -1 +1 @@\n-old\n+new\n".to_string(),
        ));
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE).into(),
        ));
        assert!(app.show_help);
        app.show_help = false;

        // Diff View - searching: '?' types into search, F1 opens help
        if let Some(ref mut diff_view) = app.diff_view {
            diff_view.search_active = true;
        }
        assert!(!handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE).into(),
        ));
        assert!(!app.show_help);
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE).into(),
        ));
        assert!(app.show_help);
        app.show_help = false;
        app.diff_view = None;

        // 4. Selector overlay
        app.selector = Some(Selector {
            title: "Select Label".to_string(),
            all_items: vec!["bug".to_string()],
            selected_items: std::collections::HashSet::new(),
            cursor_idx: 0,
            search_query: String::new(),
            is_filtering: false,
            is_loading: false,
            entity_iid: 1,
            entity_type: "issue".to_string(),
            field_type: "labels".to_string(),
            multi_select: true,
            state: ratatui::widgets::ListState::default(),
            request: None,
        });
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE).into(),
        ));
        assert!(app.show_help);
        app.show_help = false;
        app.selector = None;

        // 5. Column checklist
        app.focus_column_checklist = true;
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE).into(),
        ));
        assert!(app.show_help);
        app.show_help = false;
        app.focus_column_checklist = false;

        // 6. Date picker
        app.date_picker = Some(crate::app::DatePicker::new(
            "Select Date".to_string(),
            "2026-08-23",
            crate::app::DatePickerAction::EditNewField { field_idx: 0 },
        ));
        assert!(handle_help_keybinding(
            &mut app,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE).into(),
        ));
        assert!(app.show_help);
        app.show_help = false;
        app.date_picker = None;
    }

    const REVIEW_DIFF: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,3 @@
 fn a() {
+    b();
 }
";

    fn line_position(path: &str, new_line: u64) -> crate::domain::mr::NotePosition {
        crate::domain::mr::NotePosition {
            new_path: Some(path.to_string()),
            old_path: Some(path.to_string()),
            new_line: Some(new_line),
            old_line: None,
            start_line: None,
            line_range: None,
        }
    }

    fn review_note(
        id: u64,
        discussion: &str,
        position: Option<crate::domain::mr::NotePosition>,
    ) -> crate::domain::mr::DiscussionNote {
        crate::domain::mr::DiscussionNote {
            id: Some(id),
            body: format!("note {id}"),
            author: crate::domain::mr::Author {
                username: "alice".to_string(),
            },
            created_at: format!("2026-09-2{id}T10:00:00Z"),
            system: false,
            position,
            discussion_id: Some(discussion.to_string()),
            resolved: Some(false),
            resolvable: Some(true),
            kind: crate::domain::mr::NoteKind::Discussion,
        }
    }

    /// Diff view open on `REVIEW_DIFF` with the threads overlay showing `notes`.
    fn app_with_review_threads(notes: Vec<crate::domain::mr::DiscussionNote>) -> App {
        let mut app = App::default();
        let diff_view = DiffView::new(7, "acme/widget".to_string(), REVIEW_DIFF.to_string());
        app.review_threads = Some(crate::app::ReviewThreadsOverview::new(&notes, &diff_view));
        app.diff_view = Some(diff_view);
        app.current_comments = notes;
        app
    }

    fn press(app: &mut App, code: KeyCode) -> bool {
        handle_review_threads(app, &KeyEvent::new(code, KeyModifiers::NONE).into())
    }

    fn selected_root_id(app: &App) -> Option<u64> {
        app.review_threads
            .as_ref()
            .and_then(|o| o.selected_thread())
            .and_then(|t| t.root().id)
    }

    #[test]
    fn review_threads_ctrl_d_u_move_half_the_list_without_toggling_the_filter() {
        let mut app = app_with_review_threads(
            (1..=6)
                .map(|id| review_note(id, &format!("t{id}"), None))
                .collect(),
        );
        // 4 visible rows, so half a page is 2.
        app.review_threads.as_mut().unwrap().list_rect =
            Some(ratatui::layout::Rect::new(0, 0, 40, 4));
        let ctrl = |c| KeyPress::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));

        handle_review_threads(&mut app, &ctrl('d'));
        assert_eq!(selected_root_id(&app), Some(3));
        handle_review_threads(&mut app, &ctrl('d'));
        handle_review_threads(&mut app, &ctrl('d'));
        assert_eq!(selected_root_id(&app), Some(6), "stops on the last thread");
        handle_review_threads(&mut app, &ctrl('u'));
        assert_eq!(selected_root_id(&app), Some(4));
        assert!(!app.review_threads.as_ref().unwrap().unresolved_only);
    }

    #[test]
    fn review_threads_scroll_bindings_bound_to_sequences_jump_to_both_ends() {
        let mut app = app_with_review_threads(
            (1..=3)
                .map(|id| review_note(id, &format!("t{id}"), None))
                .collect(),
        );
        app.config
            .keybindings
            .set(crate::keymap::Action::GlobalScrollTop, &["gg"]);
        app.config
            .keybindings
            .set(crate::keymap::Action::GlobalScrollToEnd, &["ge"]);

        handle_review_threads(&mut app, &KeyPress::Sequence("ge".to_string()));
        assert_eq!(selected_root_id(&app), Some(3));
        handle_review_threads(&mut app, &KeyPress::Sequence("gg".to_string()));
        assert_eq!(selected_root_id(&app), Some(1));
    }

    #[test]
    fn review_threads_close_on_the_remapped_opening_key_only() {
        let mut app = app_with_review_threads(vec![review_note(1, "first", None)]);
        app.config
            .keybindings
            .set(crate::keymap::Action::DiffReviewThreads, &["R"]);

        press(&mut app, KeyCode::Char('T'));
        assert!(
            app.review_threads.is_some(),
            "the old default key closed it"
        );

        handle_review_threads(
            &mut app,
            &KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT).into(),
        );
        assert!(
            app.review_threads.is_none(),
            "the remapped key left it open"
        );
    }

    #[test]
    fn review_threads_navigation_wraps_around_both_ends() {
        let mut app = app_with_review_threads(vec![
            review_note(1, "first", None),
            review_note(2, "second", Some(line_position("src/lib.rs", 2))),
            review_note(3, "third", Some(line_position("src/lib.rs", 3))),
        ]);
        assert_eq!(selected_root_id(&app), Some(1));

        assert!(press(&mut app, KeyCode::Char('k')));
        assert_eq!(
            selected_root_id(&app),
            Some(3),
            "k on the first row wraps to the last"
        );

        press(&mut app, KeyCode::Char('j'));
        assert_eq!(
            selected_root_id(&app),
            Some(1),
            "j on the last row wraps to the first"
        );

        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(selected_root_id(&app), Some(1));

        press(&mut app, KeyCode::Up);
        assert_eq!(selected_root_id(&app), Some(3));
    }

    #[test]
    fn review_threads_enter_reaches_a_reviewed_file_hidden_by_the_filter() {
        let mut app = app_with_review_threads(vec![review_note(
            1,
            "anchored",
            Some(line_position("src/lib.rs", 2)),
        )]);
        app.hide_reviewed_files = true;
        if let Some(diff_view) = &mut app.diff_view {
            diff_view.restore_review_state(
                std::collections::HashSet::from(["src/lib.rs".to_string()]),
                true,
            );
        }

        assert!(press(&mut app, KeyCode::Enter));

        assert!(
            app.review_threads.is_none(),
            "a successful jump closes the overlay"
        );
        assert!(
            !app.hide_reviewed_files,
            "the session filter must follow, or the next re-fetch hides the file again"
        );
        let diff_view = app.diff_view.as_ref().unwrap();
        let line = &diff_view.lines[diff_view.cursor_idx];
        assert_eq!(
            (line.file_path.as_str(), line.new_line_num),
            ("src/lib.rs", Some(2))
        );
    }

    #[test]
    fn review_threads_enter_on_a_general_comment_keeps_the_overlay_open() {
        let mut app = app_with_review_threads(vec![review_note(1, "general", None)]);

        assert!(press(&mut app, KeyCode::Enter));

        assert!(app.review_threads.is_some());
        assert!(app.toast_notice().is_some());
    }

    #[test]
    fn review_threads_actions_open_a_selector_that_takes_the_keys() {
        let mut app = app_with_review_threads(vec![
            review_note(1, "thread", Some(line_position("src/lib.rs", 2))),
            review_note(2, "thread", Some(line_position("src/lib.rs", 2))),
        ]);

        assert!(press(&mut app, KeyCode::Char('a')));

        let selector = app.selector.as_ref().expect("a opens the comment picker");
        assert_eq!(selector.field_type, "comment_select");
        assert_eq!(selector.all_items.len(), 2);
        assert!(
            !press(&mut app, KeyCode::Char('j')),
            "keys belong to the selector on top"
        );
        assert!(app.review_threads.is_some());
    }

    /// Holds the env lock and points the cache dir at a tempdir, because the
    /// submit-review dialog writes drafts to the project cache.
    struct IsolatedCache {
        _env: crate::config::EnvGuard,
        _home: tempfile::TempDir,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    fn isolate_cache() -> IsolatedCache {
        let lock = crate::config::TEST_ENV_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let home = tempfile::tempdir().unwrap();
        let env = crate::config::EnvGuard::isolate_home(home.path());
        IsolatedCache {
            _env: env,
            _home: home,
            _lock: lock,
        }
    }

    fn draft(body: &str) -> crate::domain::review::DraftComment {
        crate::domain::review::DraftComment {
            file_path: "src/lib.rs".to_string(),
            line_num: Some(2),
            old_line_num: None,
            end_line_num: None,
            end_old_line_num: None,
            body: body.to_string(),
        }
    }

    fn bodies(drafts: &[crate::domain::review::DraftComment]) -> Vec<&str> {
        drafts.iter().map(|d| d.body.as_str()).collect()
    }

    const PROJECT: &str = "acme/widget";

    /// Diff view of MR 7 in review mode with two saved drafts, and the
    /// submit-review dialog that leaving it raises.
    fn app_leaving_the_diff_with_drafts() -> App {
        let mut app = App::default();
        app.diff_view = Some(DiffView::new(
            7,
            PROJECT.to_string(),
            REVIEW_DIFF.to_string(),
        ));
        app.in_review_mode = true;
        app.draft_comments = vec![draft("nit"), draft("typo")];
        app.persist_draft_comments(PROJECT, 7);
        app.submit_dialog = Some(SubmitDialog::build_with_project(
            ConfirmAction::SubmitReview(7),
            PROJECT.to_string(),
            &app,
        ));
        app
    }

    fn press_in_dialog(app: &mut App, codes: &[KeyCode]) {
        let tx = app.tx.clone();
        for &code in codes {
            assert!(handle_submit_dialog(
                app,
                &KeyEvent::new(code, KeyModifiers::NONE).into(),
                &tx,
                &mut crate::effect::Effects::default(),
            ));
        }
    }

    fn saved_drafts(app: &App, project_path: &str, mr_iid: u64) -> Vec<String> {
        crate::utils::cache::load_cache(app.scope.as_str())
            .draft_comments
            .get(project_path)
            .and_then(|drafts| drafts.get(&mr_iid))
            .map(|drafts| drafts.iter().map(|d| d.body.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn esc_on_the_submit_review_dialog_returns_to_the_diff_with_drafts_intact() {
        let _cache = isolate_cache();
        let mut app = app_leaving_the_diff_with_drafts();

        press_in_dialog(&mut app, &[KeyCode::Esc]);

        assert!(app.submit_dialog.is_none());
        assert_eq!(app.diff_view.as_ref().map(|d| d.mr_iid), Some(7));
        assert_eq!(bodies(&app.draft_comments), ["nit", "typo"]);
        assert!(app.in_review_mode);
        assert_eq!(saved_drafts(&app, PROJECT, 7), ["nit", "typo"]);
    }

    #[test]
    fn cancel_on_the_submit_review_dialog_returns_to_the_diff_with_drafts_intact() {
        let _cache = isolate_cache();
        let mut app = app_leaving_the_diff_with_drafts();

        press_in_dialog(&mut app, &[KeyCode::Right; 3]);
        assert!(app.submit_dialog.as_ref().is_some_and(|d| d.is_on_cancel()));
        press_in_dialog(&mut app, &[KeyCode::Enter]);

        assert!(app.submit_dialog.is_none());
        assert_eq!(app.diff_view.as_ref().map(|d| d.mr_iid), Some(7));
        assert_eq!(bodies(&app.draft_comments), ["nit", "typo"]);
        assert!(app.in_review_mode);
    }

    #[test]
    fn submit_on_the_submit_review_dialog_opens_the_review_overlay_with_drafts_intact() {
        let _cache = isolate_cache();
        let mut app = app_leaving_the_diff_with_drafts();

        press_in_dialog(&mut app, &[KeyCode::Enter]);

        assert!(app.submit_dialog.is_none());
        let overlay = app
            .review_submit
            .as_ref()
            .expect("Submit opens the overlay");
        assert_eq!(overlay.mr_iid, 7);
        assert_eq!(overlay.project_path, PROJECT);
        assert_eq!(app.diff_view.as_ref().map(|d| d.mr_iid), Some(7));
        assert_eq!(bodies(&app.draft_comments), ["nit", "typo"]);
        assert!(app.in_review_mode);
        assert_eq!(saved_drafts(&app, PROJECT, 7), ["nit", "typo"]);
    }

    #[test]
    fn keep_drafts_closes_the_diff_and_reopening_or_restarting_restores_them() {
        let _cache = isolate_cache();
        let mut app = app_leaving_the_diff_with_drafts();

        press_in_dialog(&mut app, &[KeyCode::Right, KeyCode::Enter]);

        assert!(app.submit_dialog.is_none());
        assert!(app.diff_view.is_none());
        assert_eq!(
            app.header_notice().map(Notice::text),
            Some("Kept 2 draft comments for later")
        );
        assert_eq!(
            bodies(&app.draft_comments_for_mr(PROJECT, 7)),
            ["nit", "typo"]
        );
        assert_eq!(saved_drafts(&app, PROJECT, 7), ["nit", "typo"]);
    }

    #[test]
    fn discard_drops_the_drafts_and_their_saved_copy_and_closes_the_diff() {
        let _cache = isolate_cache();
        let mut app = app_leaving_the_diff_with_drafts();

        press_in_dialog(&mut app, &[KeyCode::Right, KeyCode::Right, KeyCode::Enter]);

        assert!(app.submit_dialog.is_none());
        assert!(app.diff_view.is_none());
        assert!(app.draft_comments.is_empty());
        assert!(app.draft_comments_for_mr(PROJECT, 7).is_empty());
        assert!(saved_drafts(&app, PROJECT, 7).is_empty());
        assert_eq!(
            app.header_notice().map(Notice::text),
            Some("Discarded 2 draft comments")
        );
    }

    #[test]
    fn drafts_of_one_project_never_load_for_the_same_iid_in_another_project() {
        let _cache = isolate_cache();
        let mut app = app_leaving_the_diff_with_drafts();
        app.draft_comments = vec![draft("other project")];
        app.persist_draft_comments("acme/gadget", 7);

        assert_eq!(
            bodies(&app.draft_comments_for_mr(PROJECT, 7)),
            ["nit", "typo"]
        );
        assert_eq!(
            bodies(&app.draft_comments_for_mr("acme/gadget", 7)),
            ["other project"]
        );

        press_in_dialog(&mut app, &[KeyCode::Right, KeyCode::Right, KeyCode::Enter]);

        assert!(saved_drafts(&app, PROJECT, 7).is_empty());
        assert_eq!(saved_drafts(&app, "acme/gadget", 7), ["other project"]);
    }

    #[test]
    fn drafts_whose_lines_left_the_diff_are_held_back_but_kept_until_discarded() {
        let _cache = isolate_cache();
        let mut app = app_leaving_the_diff_with_drafts();
        let moved = crate::domain::review::DraftComment {
            line_num: Some(9),
            ..draft("moved")
        };
        let range_end_moved = crate::domain::review::DraftComment {
            end_line_num: Some(9),
            ..draft("range")
        };
        app.draft_comments = vec![draft("nit"), moved, range_end_moved];
        app.persist_draft_comments(PROJECT, 7);

        let diff_view = app.diff_view.take().expect("fixture opens a diff");
        assert_eq!(app.restore_draft_comments(&diff_view), 2);
        app.diff_view = Some(diff_view);
        assert_eq!(bodies(&app.draft_comments), ["nit"]);

        app.draft_comments.push(draft("new"));
        app.persist_draft_comments(PROJECT, 7);
        assert_eq!(
            saved_drafts(&app, PROJECT, 7),
            ["nit", "new", "moved", "range"]
        );

        press_in_dialog(&mut app, &[KeyCode::Right, KeyCode::Right, KeyCode::Enter]);

        assert!(app.stale_draft_comments.is_empty());
        assert!(saved_drafts(&app, PROJECT, 7).is_empty());
        assert_eq!(
            app.header_notice().map(Notice::text),
            Some("Discarded 4 draft comments")
        );
    }

    fn app_with_cancel_jobs_dialog() -> App {
        let job = |id| crate::domain::pipelines::Job {
            id,
            status: "running".to_string(),
            stage: "test".to_string(),
            name: format!("job-{id}"),
            matrix: None,
            duration_seconds: None,
            runner: None,
            ..Default::default()
        };
        let mut app = App::default();
        app.active_pipeline_id = Some(7);
        app.jobs.items = vec![job(1), job(2), job(3)];
        app.selected_jobs.extend([1, 3]);
        app.submit_dialog = Some(crate::app::SubmitDialog::build(
            crate::app::ConfirmAction::CancelJobs {
                pipeline_id: 7,
                job_ids: vec![1, 3],
            },
            &app,
        ));
        app
    }

    fn press_in_submit_dialog(app: &mut App, code: KeyCode) {
        let tx = app.tx.clone();
        super::handle_submit_dialog(
            app,
            &KeyEvent::new(code, KeyModifiers::NONE).into(),
            &tx,
            &mut crate::effect::Effects::default(),
        );
    }

    #[test]
    fn dismissing_the_cancel_jobs_dialog_cancels_nothing() {
        let mut app = app_with_cancel_jobs_dialog();

        press_in_submit_dialog(&mut app, KeyCode::Esc);

        assert!(app.submit_dialog.is_none());
        assert!(app.jobs.items.iter().all(|j| j.status == "running"));
        assert_eq!(app.selected_jobs.len(), 2);
    }

    #[test]
    fn remapping_dialog_cancel_off_esc_stops_esc_and_arms_the_new_key() {
        let mut app = app_with_cancel_jobs_dialog();
        app.config
            .keybindings
            .set(crate::keymap::Action::DialogCancel, &["x"]);

        press_in_submit_dialog(&mut app, KeyCode::Esc);
        assert!(
            app.submit_dialog.is_some(),
            "the old default key must no longer cancel the dialog"
        );

        press_in_submit_dialog(&mut app, KeyCode::Char('x'));
        assert!(
            app.submit_dialog.is_none(),
            "the new key must cancel the dialog"
        );
    }

    #[test]
    fn confirming_the_cancel_jobs_dialog_without_a_backend_cancels_nothing() {
        let mut app = app_with_cancel_jobs_dialog();

        press_in_submit_dialog(&mut app, KeyCode::Left);
        press_in_submit_dialog(&mut app, KeyCode::Enter);

        assert!(app.jobs.items.iter().all(|j| j.status == "running"));
        assert_eq!(app.selected_jobs.len(), 2);
    }

    #[tokio::test]
    async fn confirming_the_cancel_jobs_dialog_cancels_only_the_selected_jobs() {
        let mut app = app_with_cancel_jobs_dialog();
        app.gitlab_client = Some(crate::domain::client::GitlabClient {
            is_github: false,
            backend: crate::backend::create_backend(false),
            tx: None,
            page_size: 10,
            api_per_page: 10,
            show_read_notifications: false,
        });

        press_in_submit_dialog(&mut app, KeyCode::Left);
        press_in_submit_dialog(&mut app, KeyCode::Enter);

        let statuses: Vec<(u64, &str)> = app
            .jobs
            .items
            .iter()
            .map(|j| (j.id, j.status.as_str()))
            .collect();
        assert_eq!(
            statuses,
            vec![(1, "canceled"), (2, "running"), (3, "canceled")]
        );
        assert!(app.selected_jobs.is_empty());
    }
}
