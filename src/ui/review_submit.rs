use super::diff::centered_rect_fixed;
use super::modal::clear_area;
use crate::app::{App, OverlayKind};
use crate::config::{ICONS, Icons, THEME, Theme};
use crate::review_submit::{
    ReviewSubmit, ReviewSubmitFocus, ReviewSubmitLayout, ReviewVerdict, SubmissionState,
};
use crate::utils::format::{sanitize_untrusted, wrap_text};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

const DIALOG_WIDTH: u16 = 64;
/// Text columns inside the border and the two-column padding on each side.
const TEXT_WIDTH: usize = DIALOG_WIDTH as usize - 4;
/// Rows of summary text the box shows; a longer summary shows its end.
const SUMMARY_ROWS: u16 = 3;
/// Most rows an error takes, so the buttons stay on screen.
const MAX_STATUS_ROWS: usize = 4;

pub(crate) fn render_review_submit(f: &mut Frame, app: &mut App, size: Rect) {
    let Some(mut overlay) = app.review_submit.take() else {
        return;
    };
    let theme = *THEME.read().unwrap();
    let icons = ICONS.read().unwrap();
    let kind = app.kind();
    let notes = wrap_text(
        &draft_note(overlay.verdict, app.draft_comments.len(), kind.is_gitlab()),
        TEXT_WIDTH,
    );
    let status = status_lines(&overlay.state, &icons, &theme);
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

    clear_area(f, area);
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
    .areas(area.inner(Margin::new(2, 1)));

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
        Paragraph::new(verdict_lines(&overlay, &icons, &theme)),
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
    render_buttons(f, &overlay, (submit_button, cancel_button), &icons, &theme);
    f.render_widget(Paragraph::new(key_hints(&overlay)).style(muted), hints_area);

    overlay.layout = Some(ReviewSubmitLayout {
        verdicts: verdicts_area,
        summary: summary_area,
        submit_button,
        cancel_button,
    });
    app.review_submit = Some(overlay);
}

fn draft_note(verdict: ReviewVerdict, draft_count: usize, is_gitlab: bool) -> String {
    let drafts = if draft_count == 1 {
        "1 draft comment".to_string()
    } else {
        format!("{draft_count} draft comments")
    };
    let mut note = match (verdict, draft_count) {
        (ReviewVerdict::KeepAsDraft, 0) => "There are no draft comments to keep.".to_string(),
        (ReviewVerdict::KeepAsDraft, _) => {
            format!("Nothing is posted. Kept for later: {drafts}.")
        }
        (_, 0) => "No draft comments: the review posts its verdict and summary.".to_string(),
        _ => format!("{drafts} will be posted with the review."),
    };
    if verdict == ReviewVerdict::RequestChanges && is_gitlab {
        note.push_str(
            "\nGitLab has no request-changes verdict: only the comments and summary are posted.",
        );
    }
    note
}

fn status_lines(state: &SubmissionState, icons: &Icons, theme: &Theme) -> Vec<Line<'static>> {
    match state {
        SubmissionState::Editing => Vec::new(),
        SubmissionState::Submitting { .. } => vec![Line::from(Span::styled(
            format!("{} Submitting review…", icons.status_running),
            Style::default()
                .fg(theme.yellow)
                .add_modifier(Modifier::BOLD),
        ))],
        SubmissionState::Failed(error) => {
            let message = format!("{} {}", icons.status_failed, sanitize_untrusted(error));
            wrap_text(&message, TEXT_WIDTH)
                .into_iter()
                .take(MAX_STATUS_ROWS)
                .map(|row| Line::from(Span::styled(row, Style::default().fg(theme.red))))
                .collect()
        }
    }
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

    let value = if is_focused {
        overlay.summary.value_with_cursor('▋')
    } else {
        overlay.summary.value().to_string()
    };
    let (rows, style) = if value.is_empty() {
        let placeholder = match overlay.verdict {
            ReviewVerdict::Approve => "Optional for an approval",
            ReviewVerdict::KeepAsDraft => "Not posted while the drafts are kept",
            _ => "Required without draft comments",
        };
        (
            vec![placeholder.to_string()],
            Style::default()
                .fg(theme.text_muted)
                .add_modifier(Modifier::ITALIC),
        )
    } else {
        let color = if overlay.verdict == ReviewVerdict::KeepAsDraft {
            theme.text_muted
        } else {
            theme.text_normal
        };
        (
            wrap_text(&sanitize_untrusted(&value), usize::from(inner.width)),
            Style::default().fg(color),
        )
    };
    let hidden = rows.len().saturating_sub(usize::from(inner.height));
    f.render_widget(
        Paragraph::new(rows.into_iter().map(Line::from).collect::<Vec<_>>())
            .style(style.bg(theme.bg))
            .scroll((u16::try_from(hidden).unwrap_or(u16::MAX), 0)),
        inner,
    );
}

fn render_buttons(
    f: &mut Frame,
    overlay: &ReviewSubmit,
    (submit_area, cancel_area): (Rect, Rect),
    icons: &Icons,
    theme: &Theme,
) {
    let is_submitting = overlay.is_submitting();
    let button_style = |is_focused: bool, accent: Option<Color>| {
        if is_submitting {
            return Style::default().fg(theme.text_muted).bg(theme.bg);
        }
        let (fg, bg) = match (is_focused, accent) {
            (true, Some(accent)) => (theme.bg, accent),
            (false, Some(accent)) => (accent, theme.bg),
            (true, None) => (theme.bg, theme.border_focused),
            (false, None) => (theme.text_normal, theme.bg),
        };
        let style = Style::default().fg(fg).bg(bg);
        if is_focused {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
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
            .style(button_style(
                overlay.focus == ReviewSubmitFocus::SubmitButton,
                Some(theme.green),
            )),
        submit_area,
    );
    f.render_widget(
        Paragraph::new(format!("{} Cancel", icons.check_off))
            .alignment(Alignment::Center)
            .style(button_style(
                overlay.focus == ReviewSubmitFocus::CancelButton,
                None,
            )),
        cancel_area,
    );
}

fn key_hints(overlay: &ReviewSubmit) -> &'static str {
    if overlay.is_submitting() {
        "Waiting for the server: keys are ignored until it answers"
    } else {
        "Tab next · j/k verdict · Enter submit · Esc cancel"
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
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        terminal
            .draw(|f| render_review_submit(f, app, f.area()))
            .unwrap();
        screen_text(&terminal)
    }

    fn app_with_overlay(state: SubmissionState) -> App {
        let mut app = App::default();
        let mut overlay = ReviewSubmit::new(7, "acme/widget".to_string());
        overlay.state = state;
        app.review_submit = Some(overlay);
        app
    }

    #[test]
    fn a_running_submission_shows_its_loading_state_on_the_overlay() {
        let mut app = App::default();
        let log_row = app.start_command("Submitting review on MR #7".to_string());
        let mut overlay = ReviewSubmit::new(7, "acme/widget".to_string());
        overlay.state = SubmissionState::Submitting { log_row };
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
            "No draft comments: the review posts its verdict and summary.\n\
             GitLab has no request-changes verdict: only the comments and summary are posted."
        );
    }
}
