use crate::app::App;
use crate::domain::review::{DraftComment, ReviewEvent};
use crate::effect::{EditPurpose, Effects, ForegroundJob};
use crate::event::{CommandOutcome, Event};
use crate::keybinding::KeyPress;
use crate::keybinding::keybinding_matches;
use crate::{app, handlers};
use crossterm::event::{KeyCode, KeyModifiers};
use tokio::sync::mpsc::UnboundedSender;

/// Runs a review mutation in the background and reports its outcome on the
/// MRs tab.
pub(crate) fn spawn_mr_review_task(
    fx: &mut Effects,
    tx: &UnboundedSender<Event>,
    task: impl Future<Output = anyhow::Result<()>> + Send + 'static,
) {
    let tx = tx.clone();
    fx.spawn(async move {
        let result = task
            .await
            .map(|()| CommandOutcome::Mutated)
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(Event::CommandCompleted(app::Tab::MergeRequests, result));
    });
}

/// Posts one diff comment right away, outside review mode, as a single-comment
/// review so it lands on its line exactly like a submitted draft would.
pub(crate) fn post_review_comment(
    app: &App,
    mr_iid: u64,
    comment: DraftComment,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    let Some(client) = app.gitlab_client.clone() else {
        return;
    };
    let project = app.project_path_for_open_mr(mr_iid);
    spawn_mr_review_task(fx, tx, async move {
        client
            .submit_review(
                &project,
                mr_iid,
                ReviewEvent::Comment,
                "",
                std::slice::from_ref(&comment),
            )
            .await
    });
}

/// Keys while the MR/PR diff view is open.
pub fn handle_key(
    app: &mut App,
    key_event: &KeyPress,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    let Some(mut diff_view) = app.diff_view.take() else {
        return;
    };
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
            } else if !app.draft_comments.is_empty() {
                app.submit_dialog = Some(crate::app::SubmitDialog::build_with_project(
                    crate::app::ConfirmAction::SubmitReview(diff_view.mr_iid),
                    diff_view.project_path.clone(),
                    app,
                ));
            } else {
                app.diff_view = None;
                return;
            }
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('n') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            if !diff_view.focus_on_files && !diff_view.search_query.is_empty() {
                diff_view.search_next();
                diff_view.update_selected_file_from_cursor();
            }
            app.diff_view = Some(diff_view);
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
        }
        // Ahead of the plain `d` arm, which ignores modifiers.
        _ if keybinding_matches(
            &app.config.keybindings.global.scroll_half_page_down,
            key_event,
        ) =>
        {
            let rows = (diff_view.viewport_height / 2).max(1);
            diff_view.move_by(true, rows, in_selection);
            app.diff_view = Some(diff_view);
        }
        _ if keybinding_matches(
            &app.config.keybindings.global.scroll_half_page_up,
            key_event,
        ) =>
        {
            let rows = (diff_view.viewport_height / 2).max(1);
            diff_view.move_by(false, rows, in_selection);
            app.diff_view = Some(diff_view);
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
                    app.submit_dialog = Some(crate::app::SubmitDialog::build_with_project(
                        crate::app::ConfirmAction::SubmitReview(diff_view.mr_iid),
                        diff_view.project_path.clone(),
                        app,
                    ));
                } else {
                    app.diff_view = None;
                    return;
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
                    let node = &diff_view.visible_nodes[diff_view.selected_visible_idx];
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
            if diff_view.focus_on_files && !diff_view.visible_nodes.is_empty() {
                let node = &diff_view.visible_nodes[diff_view.selected_visible_idx];
                if node.is_dir && !node.is_expanded {
                    diff_view.root_node.toggle_expanded(&node.path_id, "");
                    diff_view.rebuild_visible_nodes();
                } else {
                    diff_view.focus_on_files = false;
                }
            }
            app.diff_view = Some(diff_view);
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            if diff_view.focus_on_files {
                // On a file in the tree → open it, switch to diff
                if !diff_view.visible_nodes.is_empty() {
                    let node = &diff_view.visible_nodes[diff_view.selected_visible_idx];
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
        KeyCode::Char('m') => {
            // From the diff pane, mark whatever is under the
            // cursor rather than a stale tree selection.
            if !diff_view.focus_on_files {
                diff_view.update_selected_file_from_cursor();
            }
            let target = {
                let paths = diff_view.selected_file_paths();
                if paths.len() == 1 {
                    paths[0].clone()
                } else {
                    format!("{} files", paths.len())
                }
            };
            if let Some((_, marked)) = diff_view.toggle_reviewed() {
                app.store_reviewed_files_for_mr(diff_view.mr_iid, &diff_view.reviewed_files);
                crate::utils::cache::save_cache(app.scope.as_str(), &app.project_cache);
                let (reviewed, total) = diff_view.review_progress();
                app.show_notice(format!(
                    "{} {} ({}/{} reviewed)",
                    if marked { "Reviewed" } else { "Unmarked" },
                    target,
                    reviewed,
                    total
                ));
            }
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('M') => {
            let hidden = diff_view.toggle_hide_reviewed();
            app.hide_reviewed_files = hidden;
            let (reviewed, total) = diff_view.review_progress();
            app.show_notice(if hidden {
                format!("Hiding {reviewed} reviewed file(s)")
            } else {
                format!("Showing all {total} file(s)")
            });
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('[') => {
            if !diff_view.focus_on_files {
                let current = diff_view.cursor_idx;
                if let Some(&prev_hunk) = diff_view.hunks.iter().rev().find(|&&h| h < current) {
                    diff_view.cursor_idx = prev_hunk;
                    diff_view.scroll_offset = diff_view.cursor_idx.saturating_sub(5);
                    diff_view.update_selected_file_from_cursor();
                }
            }
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char(']') => {
            if !diff_view.focus_on_files {
                let current = diff_view.cursor_idx;
                if let Some(&next_hunk) = diff_view.hunks.iter().find(|&&h| h > current) {
                    diff_view.cursor_idx = next_hunk;
                    diff_view.scroll_offset = diff_view.cursor_idx.saturating_sub(5);
                    diff_view.update_selected_file_from_cursor();
                }
            }
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('d') => {
            let other = if diff_view.side_by_side {
                app::DiffLayout::Unified
            } else {
                app::DiffLayout::SideBySide
            };
            set_layout(app, &mut diff_view, other);
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('1') => {
            set_layout(app, &mut diff_view, app::DiffLayout::SideBySide);
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('2') => {
            set_layout(app, &mut diff_view, app::DiffLayout::Unified);
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('0') => {
            set_layout(app, &mut diff_view, app::DiffLayout::Auto);
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if diff_view.focus_on_files {
                if !diff_view.visible_nodes.is_empty() {
                    let old_idx = diff_view.selected_visible_idx;
                    diff_view.selected_visible_idx =
                        (diff_view.selected_visible_idx + 1).min(diff_view.visible_nodes.len() - 1);
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
                    let new_idx = (diff_view.cursor_idx + 1).min(active_len - 1);
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
            } else if diff_view.cursor_idx > 0 {
                let new_idx = diff_view.cursor_idx - 1;
                if in_selection {
                    diff_view.selection_end = Some(new_idx);
                }
                diff_view.cursor_idx = new_idx;
                diff_view.update_selected_file_from_cursor();
            }
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('J') => {
            diff_view.move_by(true, 10, in_selection);
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('K') => {
            diff_view.move_by(false, 10, in_selection);
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
                    app.show_notice("Selection cancelled.");
                } else {
                    diff_view.selection_start = Some(diff_view.cursor_idx);
                    diff_view.selection_end = Some(diff_view.cursor_idx);
                    app.show_notice(
                        "Selection started. Use j/k to extend, Esc to cancel, c to comment.",
                    );
                }
            }
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('a') => {
            if diff_view.focus_on_files {
                // `a` only acts on diff-side lines; the
                // tree opening with focus on files makes
                // the very first press silently no-op.
                // Surface that explicitly via the floating
                // toast (#483).
                app.show_error(
                    "Press `l` or `Tab` to focus the diff before pressing `a` on a line."
                        .to_string(),
                );
            } else {
                let sline = if diff_view.side_by_side {
                    diff_view
                        .side_by_side_lines
                        .get(diff_view.cursor_idx)
                        .cloned()
                } else {
                    diff_view
                        .lines
                        .get(diff_view.cursor_idx)
                        .map(|l| crate::app::SideBySideLine {
                            left: Some(l.clone()),
                            right: Some(l.clone()),
                            line_type: l.line_type,
                        })
                };

                if let Some(sline) = sline {
                    let matching_current: Vec<crate::domain::mr::DiscussionNote> = app
                        .current_comments
                        .iter()
                        .filter(|c| {
                            !c.system
                                && c.position.as_ref().is_some_and(|pos| {
                                    crate::app::note_position_anchors(
                                        pos,
                                        sline.left.as_ref(),
                                        sline.right.as_ref(),
                                    )
                                })
                        })
                        .cloned()
                        .collect();

                    let matching_drafts: Vec<usize> = app
                        .draft_comments
                        .iter()
                        .enumerate()
                        .filter(|(_, d)| d.matches_side(&sline))
                        .map(|(idx, _)| idx)
                        .collect();

                    if matching_current.is_empty() && matching_drafts.is_empty() {
                        // The diff overlay covers the
                        // status line in the bottom bar,
                        // so surface this via the same
                        // floating toast that API errors
                        // use (#483).
                        app.show_error("No comments on this line to interact with.".to_string());
                    } else if matching_current.len() + matching_drafts.len() == 1 {
                        if let Some(comment) = matching_current.first() {
                            let selector = crate::app::Selector::comment_actions(
                                comment,
                                diff_view.mr_iid,
                                app.is_github(),
                            );
                            match selector {
                                Some(selector) => app.selector = Some(selector),
                                None => app.show_error(
                                    "Review summaries cannot be replied to, edited or deleted from glab-tui.".to_string(),
                                ),
                            }
                        } else if let Some(&idx) = matching_drafts.first() {
                            // Drafts only support edit and
                            // delete — the upstream API
                            // reply/resolve semantics
                            // don't apply until the draft
                            // has been pushed.
                            app.selector = Some(crate::app::Selector::draft_actions(idx));
                        }
                    } else {
                        let mut items: Vec<String> = matching_current
                            .iter()
                            .filter_map(crate::app::Selector::comment_choice_item)
                            .collect();
                        items.extend(matching_drafts.iter().map(|&idx| {
                            crate::app::Selector::draft_choice_item(
                                idx,
                                &app.draft_comments[idx].body,
                            )
                        }));
                        app.selector = Some(crate::app::Selector::comment_choice(
                            items,
                            diff_view.mr_iid,
                        ));
                    }
                }
            }
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('T') => {
            app.review_threads = Some(crate::app::ReviewThreadsOverview::new(
                &app.current_comments,
                &diff_view,
            ));
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('c') => {
            if let Some(range) = diff_view.get_comment_range() {
                app.text_input = Some(crate::app::TextInput::new(
                    format!(" Add Comment to {} ", range.file_path),
                    String::new(),
                    crate::app::TextInputAction::AddReviewComment {
                        file_path: range.file_path,
                        line_num: range.line_num,
                        old_line_num: range.old_line_num,
                        end_line_num: range.end_line_num,
                        end_old_line_num: range.end_old_line_num,
                    },
                ));
                // Clear selection after starting a comment
                diff_view.selection_start = None;
                diff_view.selection_end = None;
            }
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('C') => {
            if let Some(range) = diff_view.get_comment_range() {
                app.show_notice("Opening editor for comment...");
                fx.suspend(ForegroundJob::Edit {
                    text: String::new(),
                    file_suffix: ".md".to_string(),
                    purpose: EditPurpose::DiffComment {
                        draft: draft_at(&range),
                    },
                });
                // Clear selection after starting a comment
                diff_view.selection_start = None;
                diff_view.selection_end = None;
            }
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('r') => {
            crate::handlers::review_submit::open(
                app,
                diff_view.mr_iid,
                diff_view.project_path.clone(),
            );
            app.diff_view = Some(diff_view);
        }
        KeyCode::Char('e') => {
            if let Some(range) = diff_view.get_comment_range() {
                let content = range
                    .lines
                    .iter()
                    .map(|l| crate::app::strip_diff_marker(&l.content))
                    .collect::<Vec<_>>()
                    .join("\n");

                app.show_notice("Opening editor for code suggestion...");
                let ext = std::path::Path::new(&range.file_path)
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("md");
                fx.suspend(ForegroundJob::Edit {
                    text: content,
                    file_suffix: format!(".{ext}"),
                    purpose: EditPurpose::DiffSuggestion {
                        draft: draft_at(&range),
                    },
                });
                diff_view.selection_start = None;
                diff_view.selection_end = None;
            }
            app.diff_view = Some(diff_view);
        }
        _ => {
            app.diff_view = Some(diff_view);
            handlers::custom_commands::run_bound_diff_command(app, key_event, tx, fx);
        }
    }
}

/// Sets the diff layout for the rest of the session. A pinned layout applies to
/// the open diff at once. `Auto` is resolved against the pane width on the next
/// draw.
fn set_layout(app: &mut App, diff_view: &mut app::DiffView, layout: app::DiffLayout) {
    app.diff_layout = layout;
    let notice = match layout {
        app::DiffLayout::Auto => "Diff layout follows the diff pane width again",
        app::DiffLayout::SideBySide => {
            diff_view.set_side_by_side(true);
            "Diff layout pinned to side-by-side (0 follows the width again)"
        }
        app::DiffLayout::Unified => {
            diff_view.set_side_by_side(false);
            "Diff layout pinned to unified (0 follows the width again)"
        }
    };
    app.show_notice(notice);
}

/// A draft anchored to `range`, its body still to be written.
fn draft_at(range: &crate::app::CommentRange) -> DraftComment {
    DraftComment {
        file_path: range.file_path.clone(),
        line_num: range.line_num,
        old_line_num: range.old_line_num,
        end_line_num: range.end_line_num,
        end_old_line_num: range.end_old_line_num,
        body: String::new(),
    }
}

/// Adds `comment` to the open diff's review when one is being written,
/// otherwise posts it right away. `noun` names it in the notice.
pub(crate) fn add_diff_comment(
    app: &mut App,
    comment: DraftComment,
    noun: &str,
    tx: &UnboundedSender<Event>,
    fx: &mut Effects,
) {
    let Some((project, mr_iid)) = app
        .diff_view
        .as_ref()
        .map(|d| (d.project_path.clone(), d.mr_iid))
    else {
        return;
    };
    if app.in_review_mode {
        app.draft_comments.push(comment);
        app.persist_draft_comments(&project, mr_iid);
        app.show_notice(format!(
            "Added {noun}. ({} pending)",
            app.draft_comments.len()
        ));
    } else {
        post_review_comment(app, mr_iid, comment, tx, fx);
    }
}
