//! A job log kept parsed for display, so a frame only parses what the log
//! gained since the previous one and copies only the lines on screen.

use super::helpers::rendered_line_count;
use crate::config::Theme;
use crate::domain::job_trace::{JobTrace, TraceUpdate};
use crate::utils::format::parse_ansi_trace;
use ratatui::text::Line;

#[derive(Debug, Default)]
pub struct JobTraceView {
    trace: JobTrace,
    lines: Vec<Line<'static>>,
    parsed: Option<Parsed>,
    wrapped: Option<WrappedRows>,
}

/// What `JobTraceView::lines` holds.
#[derive(Debug, Clone, Copy)]
struct Parsed {
    theme: Theme,
    text_len: usize,
    /// Bytes of text up to the end of its last complete line, parsed into
    /// `lines[..complete_lines]`. Only the line after it can still grow.
    complete_len: usize,
    complete_lines: usize,
}

/// Where each line starts once wrapped at `width`.
#[derive(Debug)]
struct WrappedRows {
    width: usize,
    /// Row each complete line counted so far starts at.
    line_starts: Vec<usize>,
    complete_rows: usize,
    /// Rows of the incomplete last line, which starts at `complete_rows`.
    partial_rows: usize,
    text_len: Option<usize>,
}

impl JobTraceView {
    pub fn new(trace: JobTrace) -> Self {
        Self {
            trace,
            ..Self::default()
        }
    }

    pub fn trace(&self) -> &JobTrace {
        &self.trace
    }

    /// Applies a follow-mode update to the log, dropping the parsed lines
    /// when it replaces the log. `false` when the update was stale.
    pub fn apply(&mut self, update: TraceUpdate) -> bool {
        let is_restart = matches!(update, TraceUpdate::Restart { .. });
        let is_applied = self.trace.apply(update);
        if is_applied && is_restart {
            self.forget_parsed_lines();
        }
        is_applied
    }

    /// Rows the log takes styled with `theme`: one per line, or wrapped at
    /// `wrap_width` cells.
    pub fn rows(&mut self, theme: &Theme, wrap_width: Option<usize>) -> usize {
        self.parse_new_text(theme);
        let Some(width) = wrap_width else {
            return self.lines.len();
        };
        self.count_wrapped_rows(width);
        self.wrapped
            .as_ref()
            .map_or(0, |rows| rows.complete_rows + rows.partial_rows)
    }

    /// The lines a pane `height` rows tall shows `scroll` rows down the log,
    /// and how many rows of the first one are above the pane.
    pub fn visible_lines(
        &mut self,
        theme: &Theme,
        wrap_width: Option<usize>,
        scroll: usize,
        height: usize,
    ) -> (Vec<Line<'static>>, usize) {
        self.parse_new_text(theme);
        let bottom = scroll.saturating_add(height);
        let Some(width) = wrap_width else {
            let end = bottom.min(self.lines.len());
            return (self.lines[scroll.min(end)..end].to_vec(), 0);
        };
        self.count_wrapped_rows(width);
        let (Some(parsed), Some(rows)) = (self.parsed, self.wrapped.as_ref()) else {
            return (Vec::new(), 0);
        };
        let has_partial_line = parsed.complete_lines < self.lines.len();
        let row_of = |line: usize| {
            rows.line_starts
                .get(line)
                .copied()
                .unwrap_or(rows.complete_rows)
        };
        let first = if has_partial_line && scroll >= rows.complete_rows {
            parsed.complete_lines
        } else {
            rows.line_starts
                .partition_point(|&start| start <= scroll)
                .saturating_sub(1)
        };
        let end = if has_partial_line && rows.complete_rows < bottom {
            self.lines.len()
        } else {
            rows.line_starts.partition_point(|&start| start < bottom)
        };
        let lines = self.lines[first..end.max(first)].to_vec();
        (lines, scroll.saturating_sub(row_of(first)))
    }

    fn forget_parsed_lines(&mut self) {
        self.lines.clear();
        self.parsed = None;
        self.wrapped = None;
    }

    fn parse_new_text(&mut self, theme: &Theme) {
        if self.parsed.is_some_and(|parsed| parsed.theme != *theme) {
            self.forget_parsed_lines();
        }
        let text = self.trace.text();
        let mut parsed = self.parsed.unwrap_or(Parsed {
            theme: *theme,
            text_len: 0,
            complete_len: 0,
            complete_lines: 0,
        });
        if self.parsed.is_some() && parsed.text_len == text.len() {
            return;
        }
        let fresh = &text[parsed.complete_len..];
        self.lines.truncate(parsed.complete_lines);
        self.lines.extend(parse_ansi_trace(fresh, theme));
        parsed.complete_len += fresh.rfind('\n').map_or(0, |end| end + 1);
        let has_partial_line = parsed.complete_len < text.len();
        parsed.complete_lines = self.lines.len() - usize::from(has_partial_line);
        parsed.text_len = text.len();
        self.parsed = Some(parsed);
    }

    /// Counts rows for the lines parsed since the last count at `width`.
    fn count_wrapped_rows(&mut self, width: usize) {
        let Some(parsed) = self.parsed else {
            return;
        };
        if self.wrapped.as_ref().is_none_or(|rows| rows.width != width) {
            self.wrapped = Some(WrappedRows {
                width,
                line_starts: Vec::new(),
                complete_rows: 0,
                partial_rows: 0,
                text_len: None,
            });
        }
        let Some(rows) = self.wrapped.as_mut() else {
            return;
        };
        if rows.text_len == Some(parsed.text_len) {
            return;
        }
        for line in &self.lines[rows.line_starts.len()..parsed.complete_lines] {
            rows.line_starts.push(rows.complete_rows);
            rows.complete_rows += rendered_line_count(std::slice::from_ref(line), width, true);
        }
        rows.partial_rows = rendered_line_count(&self.lines[parsed.complete_lines..], width, true);
        rows.text_len = Some(parsed.text_len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace_of(text: &str) -> JobTrace {
        JobTrace::from_read(text.as_bytes(), false)
    }

    fn append(view: &mut JobTraceView, text: &str) {
        let update = view
            .trace()
            .cursor()
            .append_update(text.as_bytes().to_vec(), false);
        assert!(view.apply(update));
    }

    fn all_lines(view: &mut JobTraceView, theme: &Theme) -> Vec<Line<'static>> {
        view.visible_lines(theme, None, 0, usize::MAX).0
    }

    #[test]
    fn lines_after_appends_match_parsing_the_whole_log_at_once() {
        let theme = Theme::default();
        let mut view = JobTraceView::new(trace_of("step 1\n\x1b[31mfail"));
        assert_eq!(view.rows(&theme, None), 2);

        append(&mut view, "ed\x1b[0m\r\nstep 2\n");
        append(&mut view, "");
        append(&mut view, "step 3 starts");

        let whole = "step 1\n\x1b[31mfailed\x1b[0m\r\nstep 2\nstep 3 starts";
        assert_eq!(
            all_lines(&mut view, &theme),
            parse_ansi_trace(whole, &theme)
        );
    }

    #[test]
    fn wrapped_rows_after_appends_match_counting_the_whole_log() {
        let theme = Theme::default();
        let mut view = JobTraceView::new(trace_of("a long first line\nsecond"));
        let before = parse_ansi_trace("a long first line\nsecond", &theme);
        assert_eq!(
            view.rows(&theme, Some(6)),
            rendered_line_count(&before, 6, true)
        );

        append(&mut view, " line grows\nthird\n");

        let whole = parse_ansi_trace("a long first line\nsecond line grows\nthird\n", &theme);
        assert_eq!(
            view.rows(&theme, Some(6)),
            rendered_line_count(&whole, 6, true)
        );
        assert_eq!(
            view.rows(&theme, Some(40)),
            rendered_line_count(&whole, 40, true)
        );
    }

    #[test]
    fn visible_lines_start_at_the_line_holding_the_scrolled_to_row() {
        let theme = Theme::default();
        let text = "a long first line\nsecond\nthird";
        let lines = parse_ansi_trace(text, &theme);
        let mut view = JobTraceView::new(trace_of(text));

        assert_eq!(
            view.visible_lines(&theme, None, 1, 1),
            (lines[1..2].to_vec(), 0)
        );
        // At 6 cells the first line takes rows 0-2, "second" row 3, "third" row 4.
        assert_eq!(
            view.visible_lines(&theme, Some(6), 1, 3),
            (lines[0..2].to_vec(), 1)
        );
        assert_eq!(
            view.visible_lines(&theme, Some(6), 4, 2),
            (lines[2..].to_vec(), 0)
        );
    }

    #[test]
    fn theme_change_restyles_every_line() {
        let dark = Theme::default();
        let light = Theme::preset("catppuccin-latte").unwrap();
        let mut view = JobTraceView::new(trace_of("plain\n\x1b[1mbold\n"));
        all_lines(&mut view, &dark);

        assert_eq!(
            all_lines(&mut view, &light),
            parse_ansi_trace("plain\n\x1b[1mbold\n", &light)
        );
    }

    #[test]
    fn restart_drops_the_lines_of_the_replaced_log() {
        let theme = Theme::default();
        let mut view = JobTraceView::new(trace_of("old line\nold line 2\n"));
        all_lines(&mut view, &theme);

        let update = view.trace().cursor().restart_update(trace_of("new\n"));
        assert!(view.apply(update));

        assert_eq!(
            all_lines(&mut view, &theme),
            parse_ansi_trace("new\n", &theme)
        );
    }
}
