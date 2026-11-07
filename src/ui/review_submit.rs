use super::diff::centered_rect_fixed;
use super::modal::{button_style, clear_area};
use crate::app::{App, OverlayKind};
use crate::config::{Icons, Theme};
use crate::domain::review::draft_comments_phrase;
use crate::review_submit::{
    ReviewSubmit, ReviewSubmitFocus, ReviewSubmitLayout, ReviewVerdict, SubmissionState,
};
use crate::ui::render_ctx::RenderCtx;
use crate::utils::format::{sanitize_untrusted, wrap_ranges, wrap_text};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

const DIALOG_WIDTH: u16 = 64;
/// The border plus one padding column on each side, and the border alone
/// above and below.
const CONTENT_MARGIN: Margin = Margin::new(2, 1);
/// Rows of summary text the box shows, scrolled to keep the cursor's row.
const SUMMARY_ROWS: u16 = 3;
/// Most rows an error takes, so the buttons stay on screen.
const MAX_STATUS_ROWS: usize = 4;
/// Shown under a failure cut at `MAX_STATUS_ROWS`: its root cause comes last
/// in `{e:#}`, and the submission's Terminal-tab entry keeps all of it.
const CUT_ERROR_HINT: &str = "Full error in the Terminal tab";

pub(crate) fn render_review_submit(f: &mut Frame, ctx: &RenderCtx<'_>, app: &mut App, size: Rect) {
    let Some(mut overlay) = app.review_submit.take() else {
        return;
    };
    let theme = ctx.theme;
    let icons = ctx.icons;
    let kind = ctx.kind;
    let text_width = usize::from(
        DIALOG_WIDTH
            .min(size.width)
            .saturating_sub(2 * CONTENT_MARGIN.horizontal),
    );
    let draft_count = match overlay.verdict {
        ReviewVerdict::KeepAsDraft => app.all_draft_count(),
        _ => app.draft_comments.len(),
    };
    let notes = wrap_text(
        &draft_note(overlay.verdict, draft_count, kind.is_gitlab()),
        text_width,
    );
    let status = status_lines(&overlay.state, text_width, icons, &theme);
    let status_rows = if status.is_empty() {
        0
    } else {
        status.len() as u16 + 1
    };

    let content_rows = 1 + notes.len() as u16 + 1 + 1 + ReviewVerdict::ALL.len() as u16 + 1;
    let summary_box_rows = SUMMARY_ROWS + 2;
    let footer_rows = 3;
    let height = 2 + content_rows + summary_box_rows + status_rows + footer_rows;
    let area = centered_rect_fixed(DIALOG_WIDTH, height, size);
    app.overlay_stack.push((OverlayKind::ReviewSubmit, area));

    clear_area(f, ctx, area);
    f.render_widget(
        Block::default()
            .title(format!(
                " {} Submit {} Review ",
                icons.action_review,
                kind.term("mr")
            ))
            .title_style(
                Style::default()
                    .fg(theme.header_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_focused))
            .style(Style::default().bg(theme.bg)),
        area,
    );

    let [
        _,
        notes_area,
        _,
        verdict_header,
        verdicts_area,
        _,
        summary_area,
        status_area,
        _,
        separator,
        buttons_area,
        hints_area,
    ] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(notes.len() as u16),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(ReviewVerdict::ALL.len() as u16),
        Constraint::Length(1),
        Constraint::Length(summary_box_rows),
        Constraint::Length(status_rows),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area.inner(CONTENT_MARGIN));

    let text = Style::default().fg(theme.text_normal).bg(theme.bg);
    let muted = Style::default().fg(theme.text_muted).bg(theme.bg);
    f.render_widget(
        Paragraph::new(notes.into_iter().map(Line::from).collect::<Vec<_>>()).style(text),
        notes_area,
    );
    f.render_widget(
        Paragraph::new("Verdict").style(muted.add_modifier(Modifier::BOLD)),
        verdict_header,
    );
    f.render_widget(
        Paragraph::new(verdict_lines(&overlay, icons, &theme)),
        verdicts_area,
    );
    render_summary(f, &overlay, summary_area, &theme);
    f.render_widget(
        Paragraph::new(status),
        Rect {
            y: status_area.y.saturating_add(1),
            height: status_area.height.saturating_sub(1),
            ..status_area
        },
    );
    f.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(theme.border)),
        separator,
    );
    let [submit_button, _, cancel_button] = Layout::horizontal([
        Constraint::Ratio(1, 2),
        Constraint::Length(1),
        Constraint::Ratio(1, 2),
    ])
    .areas(buttons_area);
    render_buttons(f, &overlay, (submit_button, cancel_button), icons, &theme);
    f.render_widget(Paragraph::new(key_hints(&overlay)).style(muted), hints_area);

    overlay.layout = Some(ReviewSubmitLayout {
        verdicts: verdicts_area,
        summary: summary_area,
        submit_button,
        cancel_button,
    });
    app.review_submit = Some(overlay);
}

/// `draft_count` is the drafts the verdict acts on: the ones a review posts,
/// or every kept one for Keep as draft, since drafts whose lines left the diff
/// are kept but never posted.
fn draft_note(verdict: ReviewVerdict, draft_count: usize, is_gitlab: bool) -> String {
    let drafts = draft_comments_phrase(draft_count);
    let mut note = match (verdict, draft_count) {
        (ReviewVerdict::KeepAsDraft, 0) => "There are no draft comments to keep.".to_string(),
        (ReviewVerdict::KeepAsDraft, _) => {
            format!("Nothing is posted. Kept for later: {drafts}.")
        }
        (ReviewVerdict::Approve, 0) => {
            "No draft comments: the review posts its verdict and summary.".to_string()
        }
        (ReviewVerdict::RequestChanges, 0) if !is_gitlab => {
            "No draft comments: the review posts its verdict and summary.".to_string()
        }
        (_, 0) => "No draft comments: only the summary is posted.".to_string(),
        _ => format!("{drafts} will be posted with the review."),
    };
    if verdict == ReviewVerdict::RequestChanges && is_gitlab {
        note.push_str("\nGitLab has no request-changes verdict, so none is posted.");
    }
    note
}

fn status_lines(
    state: &SubmissionState,
    width: usize,
    icons: &Icons,
    theme: &Theme,
) -> Vec<Line<'static>> {
    match state {
        SubmissionState::Editing => Vec::new(),
        SubmissionState::Submitting { .. } => vec![Line::from(Span::styled(
            format!("{} Submitting review…", icons.status_running),
            Style::default()
                .fg(theme.yellow)
                .add_modifier(Modifier::BOLD),
        ))],
        SubmissionState::Refused(reason) => {
            outcome_lines(&icons.status_pending, reason, width, theme.yellow).0
        }
        SubmissionState::Failed(error) => {
            let (mut lines, is_cut) = outcome_lines(&icons.status_failed, error, width, theme.red);
            if is_cut {
                lines.push(Line::from(Span::styled(
                    CUT_ERROR_HINT,
                    Style::default().fg(theme.text_muted),
                )));
            }
            lines
        }
    }
}

/// The message's rows, at most `MAX_STATUS_ROWS`, and whether it was cut to
/// fit them, in which case the last row ends in an ellipsis.
fn outcome_lines(icon: &str, text: &str, width: usize, color: Color) -> (Vec<Line<'static>>, bool) {
    let message = format!("{icon} {}", displayable(text));
    let rows = wrap_ranges(&message, width);
    let is_cut = rows.len() > MAX_STATUS_ROWS;
    let lines = rows
        .into_iter()
        .take(MAX_STATUS_ROWS)
        .enumerate()
        .map(|(index, range)| {
            let row = message[range].trim_start();
            let row = if is_cut && index + 1 == MAX_STATUS_ROWS {
                with_ellipsis(row, width)
            } else {
                row.to_string()
            };
            Line::from(Span::styled(row, Style::default().fg(color)))
        })
        .collect();
    (lines, is_cut)
}

fn with_ellipsis(row: &str, width: usize) -> String {
    let mut row = row.trim_end().to_string();
    while !row.is_empty() && Span::raw(row.as_str()).width() + 1 > width {
        row.pop();
    }
    row.push('…');
    row
}

fn verdict_lines(overlay: &ReviewSubmit, icons: &Icons, theme: &Theme) -> Vec<Line<'static>> {
    ReviewVerdict::ALL
        .iter()
        .map(|&verdict| {
            let is_selected = verdict == overlay.verdict;
            let radio = if is_selected {
                &icons.radio_on
            } else {
                &icons.radio_off
            };
            let style = if is_selected && overlay.focus == ReviewSubmitFocus::Verdicts {
                Style::default()
                    .fg(theme.highlight_bg)
                    .bg(theme.border_focused)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(theme.green).bg(theme.bg)
            } else {
                Style::default().fg(theme.text_normal).bg(theme.bg)
            };
            Line::from(Span::styled(
                format!(" {radio} {} ", verdict.label()),
                style,
            ))
        })
        .collect()
}

fn render_summary(f: &mut Frame, overlay: &ReviewSubmit, area: Rect, theme: &Theme) {
    let is_focused = overlay.focus == ReviewSubmitFocus::Summary && !overlay.is_submitting();
    let block = Block::default()
        .title(" Summary (Ctrl+E: editor) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_focused {
            theme.border_focused
        } else {
            theme.border
        }))
        .style(Style::default().bg(theme.bg));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let summary = overlay.summary.value();
    if summary.is_empty() && !is_focused {
        let placeholder = match overlay.verdict {
            ReviewVerdict::Approve => "Optional for an approval",
            ReviewVerdict::KeepAsDraft => "Not posted while the drafts are kept",
            _ => "Required without draft comments",
        };
        f.render_widget(
            Paragraph::new(placeholder).style(
                Style::default()
                    .fg(theme.text_muted)
                    .bg(theme.bg)
                    .add_modifier(Modifier::ITALIC),
            ),
            inner,
        );
        return;
    }

    let text_style =
        Style::default()
            .bg(theme.bg)
            .fg(if overlay.verdict == ReviewVerdict::KeepAsDraft {
                theme.text_muted
            } else {
                theme.text_normal
            });
    let cursor_style = Style::default()
        .fg(theme.highlight_bg)
        .bg(theme.text_normal)
        .add_modifier(Modifier::SLOW_BLINK);
    let (before, after) = summary.split_at(overlay.summary.cursor_byte_offset());
    let before = displayable(before);
    let cursor = before.len();
    let text = before + &displayable(after);
    // One column stays free for the cursor at the end of a full row.
    let rows = wrap_ranges(&text, usize::from(inner.width).saturating_sub(1));
    let cursor_row = rows
        .iter()
        .rposition(|row| row.start <= cursor)
        .unwrap_or(0);
    let first_visible = (cursor_row + 1).saturating_sub(usize::from(inner.height));
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(first_visible)
        .take(usize::from(inner.height))
        .map(|(index, row)| {
            let row_text = &text[row.clone()];
            if !is_focused || index != cursor_row {
                return Line::from(Span::styled(row_text.to_string(), text_style));
            }
            let (head, tail) = row_text.split_at(cursor - row.start);
            let mut tail = tail.chars();
            let under_cursor = tail.next().map_or_else(|| " ".to_string(), String::from);
            Line::from(vec![
                Span::styled(head.to_string(), text_style),
                Span::styled(under_cursor, cursor_style),
                Span::styled(tail.as_str().to_string(), text_style),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).style(text_style), inner);
}

/// The summary as drawn: escape sequences removed, and tabs and carriage
/// returns shown as one space so byte offsets past them do not move.
fn displayable(text: &str) -> String {
    sanitize_untrusted(text).replace(['\t', '\r'], " ")
}

fn render_buttons(
    f: &mut Frame,
    overlay: &ReviewSubmit,
    (submit_area, cancel_area): (Rect, Rect),
    icons: &Icons,
    theme: &Theme,
) {
    let is_submitting = overlay.is_submitting();
    let style_for = |is_focused: bool, accent: Option<Color>| {
        if is_submitting {
            Style::default().fg(theme.text_muted).bg(theme.bg)
        } else {
            button_style(theme, is_focused, accent)
        }
    };
    let submit_label = match (is_submitting, overlay.verdict) {
        (true, _) => "Submitting…",
        (false, ReviewVerdict::KeepAsDraft) => "Keep drafts",
        (false, _) => "Submit",
    };
    f.render_widget(
        Paragraph::new(format!("{} {submit_label}", icons.check_on))
            .alignment(Alignment::Center)
            .style(style_for(
                overlay.focus == ReviewSubmitFocus::SubmitButton,
                Some(theme.green),
            )),
        submit_area,
    );
    f.render_widget(
        Paragraph::new(format!("{} Cancel", icons.check_off))
            .alignment(Alignment::Center)
            .style(style_for(
                overlay.focus == ReviewSubmitFocus::CancelButton,
                None,
            )),
        cancel_area,
    );
}

fn key_hints(overlay: &ReviewSubmit) -> &'static str {
    if overlay.is_submitting() {
        return "Waiting for the server: keys are ignored until it answers";
    }
    match overlay.focus {
        ReviewSubmitFocus::Verdicts => "j/k verdict · Tab summary · Enter submit · Esc cancel",
        ReviewSubmitFocus::Summary => "Type the summary · Tab next · Enter submit · Esc cancel",
        ReviewSubmitFocus::SubmitButton | ReviewSubmitFocus::CancelButton => {
            "h/l button · Tab next · Enter select · Esc cancel"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn screen_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn render(app: &mut App) -> String {
        render_at_width(app, 100)
    }

    fn render_at_width(app: &mut App, width: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
        let icons = crate::config::Icons::nerd();
        let ctx = RenderCtx::new(app, &icons);
        terminal
            .draw(|f| render_review_submit(f, &ctx, app, f.area()))
            .unwrap();
        screen_text(&terminal)
    }

    fn row_texts(screen: &str) -> Vec<String> {
        screen
            .lines()
            .map(|row| {
                row.trim_matches(|c: char| c == '│' || c.is_whitespace())
                    .to_string()
            })
            .collect()
    }

    fn app_with_overlay(state: SubmissionState) -> App {
        let mut app = App::default();
        let mut overlay = ReviewSubmit::new(7, "acme/widget".to_string());
        overlay.state = state;
        app.review_submit = Some(overlay);
        app
    }

    #[test]
    fn the_note_and_an_error_wrap_to_the_dialog_a_narrow_terminal_draws() {
        let mut app = app_with_overlay(SubmissionState::Failed(
            "HTTP 422: Review body is required when no draft comments are pending".to_string(),
        ));
        if let Some(overlay) = app.review_submit.as_mut() {
            overlay.verdict = ReviewVerdict::Approve;
        }

        let rows = row_texts(&render_at_width(&mut app, 60));

        let note = rows
            .iter()
            .position(|row| row == "No draft comments: the review posts its verdict and")
            .unwrap_or_else(|| panic!("the note wraps at 56 columns: {rows:#?}"));
        assert_eq!(rows[note + 1], "summary.");
        assert!(
            rows.iter().any(|row| row.ends_with("pending")),
            "the error keeps its last word: {rows:#?}"
        );
    }

    #[test]
    fn an_error_word_wider_than_the_dialog_is_split_instead_of_clipped() {
        let path = "src/main/java/com/acme/billing/service/impl/payments/UserServiceImpl.java";
        let mut app = app_with_overlay(SubmissionState::Failed(format!(
            "draft note on {path} failed"
        )));

        let rows = row_texts(&render_at_width(&mut app, 60));

        let (head, tail) = path.split_at(56);
        let split = rows
            .iter()
            .position(|row| row == head)
            .unwrap_or_else(|| panic!("the path fills a 56-column row: {rows:#?}"));
        assert_eq!(rows[split + 1], format!("{tail} failed"));
    }

    #[test]
    fn an_error_past_its_last_row_ends_in_an_ellipsis_and_points_at_the_terminal_tab() {
        let error = "draft note on src/lib.rs failed: ".repeat(8) + "root cause";
        let mut app = app_with_overlay(SubmissionState::Failed(error));

        let rows = row_texts(&render_at_width(&mut app, 60));

        let last = rows
            .iter()
            .position(|row| row.ends_with('…'))
            .unwrap_or_else(|| panic!("the cut error ends in an ellipsis: {rows:#?}"));
        assert!(rows[last].chars().count() <= 56, "{rows:#?}");
        assert_eq!(rows[last + 1], "Full error in the Terminal tab");
        assert!(
            !rows.iter().any(|row| row.contains("root cause")),
            "{rows:#?}"
        );
    }

    #[test]
    fn an_error_that_fits_has_no_terminal_tab_hint() {
        let mut app = app_with_overlay(SubmissionState::Failed(
            "HTTP 422: Review body is required".to_string(),
        ));

        let rows = row_texts(&render(&mut app));

        assert!(
            !rows
                .iter()
                .any(|row| row == "Full error in the Terminal tab"),
            "{rows:#?}"
        );
    }

    fn app_with_summary(summary: &str) -> App {
        let mut app = app_with_overlay(SubmissionState::Editing);
        if let Some(overlay) = app.review_submit.as_mut() {
            overlay.summary.set_value(summary.to_string());
            overlay.focus = ReviewSubmitFocus::Summary;
        }
        app
    }

    #[test]
    fn the_summary_box_follows_the_cursor_into_earlier_rows() {
        let mut app = app_with_summary("one\ntwo\nthree\nfour\nfive");
        let rows = row_texts(&render(&mut app));
        assert!(rows.iter().any(|row| row == "five"), "{rows:#?}");
        assert!(!rows.iter().any(|row| row == "one"), "{rows:#?}");

        if let Some(overlay) = app.review_submit.as_mut() {
            for _ in 0.."one\ntwo\nthree\nfour\nfive".len() {
                overlay.summary.move_cursor_left();
            }
        }
        let rows = row_texts(&render(&mut app));

        assert!(rows.iter().any(|row| row == "one"), "{rows:#?}");
        assert!(!rows.iter().any(|row| row == "five"), "{rows:#?}");
    }

    #[test]
    fn the_summary_box_keeps_runs_of_spaces() {
        let mut app = app_with_summary("- a\n  - b   c");
        if let Some(overlay) = app.review_submit.as_mut() {
            overlay.focus = ReviewSubmitFocus::Verdicts;
        }

        let rows = row_texts(&render(&mut app));

        assert!(rows.iter().any(|row| row == "- b   c"), "{rows:#?}");
        let screen = render(&mut app);
        assert!(screen.contains("│  - b   c"), "{screen}");
    }

    #[test]
    fn a_running_submission_shows_its_loading_state_on_the_overlay() {
        let mut app = App::default();
        let log_row = app.start_command(format!("{}MR #7", crate::app::REVIEW_SUBMIT_LOG_PREFIX));
        let mut overlay = ReviewSubmit::new(7, "acme/widget".to_string());
        overlay.state = SubmissionState::Submitting {
            log_row,
            drafts: Vec::new(),
        };
        app.review_submit = Some(overlay);

        let screen = render(&mut app);

        assert!(screen.contains("Submitting review…"), "{screen}");
        assert!(screen.contains("Submitting…"), "{screen}");
        assert!(
            screen.contains("keys are ignored until it answers"),
            "{screen}"
        );
    }

    #[test]
    fn a_failed_submission_shows_its_error_on_the_overlay() {
        let mut app = app_with_overlay(SubmissionState::Failed(
            "HTTP 422: Review body is required".to_string(),
        ));

        let screen = render(&mut app);

        assert!(
            screen.contains("HTTP 422: Review body is required"),
            "{screen}"
        );
        assert!(screen.contains("Submit Merge Request Review"), "{screen}");
        assert!(
            app.review_submit
                .as_ref()
                .and_then(|overlay| overlay.layout)
                .is_some(),
            "the drawn layout is kept for mouse clicks"
        );
    }

    #[test]
    fn keep_as_draft_counts_the_drafts_whose_lines_left_the_diff() {
        let mut app = app_with_overlay(SubmissionState::Editing);
        app.stale_draft_comments = vec![crate::domain::review::DraftComment {
            file_path: "src/lib.rs".to_string(),
            line_num: Some(3),
            old_line_num: None,
            end_line_num: None,
            end_old_line_num: None,
            body: "moved".to_string(),
        }];
        if let Some(overlay) = app.review_submit.as_mut() {
            overlay.verdict = ReviewVerdict::KeepAsDraft;
        }

        let rows = row_texts(&render(&mut app));

        assert!(
            rows.iter()
                .any(|row| row == "Nothing is posted. Kept for later: 1 draft comment."),
            "{rows:#?}"
        );

        if let Some(overlay) = app.review_submit.as_mut() {
            overlay.verdict = ReviewVerdict::Comment;
        }
        let rows = row_texts(&render(&mut app));

        assert!(
            rows.iter()
                .any(|row| row == "No draft comments: only the summary is posted."),
            "a moved draft is never posted: {rows:#?}"
        );
    }

    #[test]
    fn the_draft_note_counts_what_the_verdict_posts() {
        assert_eq!(
            draft_note(ReviewVerdict::Comment, 1, false),
            "1 draft comment will be posted with the review."
        );
        assert_eq!(
            draft_note(ReviewVerdict::KeepAsDraft, 2, false),
            "Nothing is posted. Kept for later: 2 draft comments."
        );
        assert_eq!(
            draft_note(ReviewVerdict::RequestChanges, 0, true),
            "No draft comments: only the summary is posted.\n\
             GitLab has no request-changes verdict, so none is posted."
        );
        assert_eq!(
            draft_note(ReviewVerdict::RequestChanges, 0, false),
            "No draft comments: the review posts its verdict and summary."
        );
        assert_eq!(
            draft_note(ReviewVerdict::Comment, 0, false),
            "No draft comments: only the summary is posted."
        );
        assert_eq!(
            draft_note(ReviewVerdict::Approve, 0, true),
            "No draft comments: the review posts its verdict and summary."
        );
    }
}
