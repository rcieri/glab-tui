use crate::config::THEME;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
};

pub(crate) fn format_comment_with_suggestions(
    body: &str,
    file_path: &str,
    start_new: Option<u64>,
    end_new: Option<u64>,
    start_old: Option<u64>,
    end_old: Option<u64>,
    all_lines: &[crate::app::DiffLine],
    prefix: &str,
    prefix_style: Style,
) -> Vec<(String, Style, Vec<(Style, String)>)> {
    let mut result_lines = Vec::new();
    let mut in_suggestion = false;
    let mut is_first = true;

    // Retrieve original lines for suggestion diff
    let mut original_lines: Vec<crate::app::DiffLine> = Vec::new();
    if let Some(oln) = start_old {
        let end_o = end_old.unwrap_or(oln);
        let min_o = oln.min(end_o);
        let max_o = oln.max(end_o);
        for dl in all_lines {
            if &dl.file_path == file_path {
                if let Some(num) = dl.old_line_num {
                    if num as u64 >= min_o && num as u64 <= max_o {
                        if dl.line_type != crate::app::DiffLineType::Addition
                            && !original_lines.iter().any(|ol| ol.content == dl.content)
                        {
                            original_lines.push(dl.clone());
                        }
                    }
                }
            }
        }
    } else if let Some(nln) = start_new {
        let end_n = end_new.unwrap_or(nln);
        let min_n = nln.min(end_n);
        let max_n = nln.max(end_n);
        for dl in all_lines {
            if &dl.file_path == file_path {
                if let Some(num) = dl.new_line_num {
                    if num as u64 >= min_n && num as u64 <= max_n {
                        if dl.line_type != crate::app::DiffLineType::Deletion
                            && !original_lines.iter().any(|ol| ol.content == dl.content)
                        {
                            original_lines.push(dl.clone());
                        }
                    }
                }
            }
        }
    }

    for body_line in body.lines() {
        let is_suggestion_start = body_line.trim().starts_with("```suggestion");
        let is_suggestion_end = in_suggestion && body_line.trim().starts_with("```");

        let current_prefix = if is_first {
            is_first = false;
            prefix.to_string()
        } else {
            " ".repeat(prefix.len())
        };

        if is_suggestion_start {
            in_suggestion = true;
            result_lines.push((
                current_prefix.clone(),
                prefix_style,
                vec![(
                    Style::default()
                        .fg(THEME.read().unwrap().green)
                        .add_modifier(Modifier::BOLD),
                    "┌─── Code Suggestion ───".to_string(),
                )],
            ));

            // Print original code as DELETIONS (red)
            for orig in &original_lines {
                let code_fg = Color::Rgb(220, 140, 140);
                let code_bg = Color::Rgb(55, 22, 28);
                let prefix_fg = Color::Rgb(255, 100, 100);

                let mut spans = vec![(
                    Style::default()
                        .fg(prefix_fg)
                        .bg(code_bg)
                        .add_modifier(Modifier::BOLD),
                    "│ - ".to_string(),
                )];

                // Strip leading space/minus/plus if present
                let clean_content = if orig.content.starts_with(' ')
                    || orig.content.starts_with('-')
                    || orig.content.starts_with('+')
                {
                    if orig.content.len() > 1 {
                        orig.content[1..].to_string()
                    } else {
                        String::new()
                    }
                } else {
                    orig.content.clone()
                };

                if let Some(ref highlighted) = orig.syntax_highlighted {
                    for (span_style, text) in highlighted {
                        let merged = span_style.fg(span_style.fg.unwrap_or(code_fg)).bg(code_bg);
                        spans.push((merged, text.clone()));
                    }
                } else {
                    spans.push((Style::default().fg(code_fg).bg(code_bg), clean_content));
                }

                result_lines.push((" ".repeat(prefix.len()), prefix_style, spans));
            }
        } else if is_suggestion_end {
            in_suggestion = false;
            result_lines.push((
                current_prefix,
                prefix_style,
                vec![(
                    Style::default()
                        .fg(THEME.read().unwrap().green)
                        .add_modifier(Modifier::BOLD),
                    "└─── End of Suggestion ───".to_string(),
                )],
            ));
        } else if in_suggestion {
            // Print suggested code as ADDITIONS (green)
            let code_fg = Color::Rgb(140, 220, 140);
            let code_bg = Color::Rgb(22, 48, 28);
            let prefix_fg = Color::Rgb(80, 220, 80);

            let mut spans = vec![(
                Style::default()
                    .fg(prefix_fg)
                    .bg(code_bg)
                    .add_modifier(Modifier::BOLD),
                "│ + ".to_string(),
            )];

            // Highlight body_line syntax
            let highlighted = crate::app::highlight_line_syntax(file_path, body_line, None);

            if let Some(ref hl) = highlighted {
                for (span_style, text) in hl {
                    let merged = span_style.fg(span_style.fg.unwrap_or(code_fg)).bg(code_bg);
                    spans.push((merged, text.clone()));
                }
            } else {
                spans.push((
                    Style::default().fg(code_fg).bg(code_bg),
                    body_line.to_string(),
                ));
            }

            result_lines.push((current_prefix, prefix_style, spans));
        } else {
            result_lines.push((
                current_prefix,
                prefix_style,
                vec![(
                    Style::default().fg(THEME.read().unwrap().text_normal),
                    body_line.to_string(),
                )],
            ));
        }
    }

    if result_lines.is_empty() {
        result_lines.push((
            prefix.to_string(),
            prefix_style,
            vec![(
                Style::default().fg(THEME.read().unwrap().text_normal),
                String::new(),
            )],
        ));
    }

    result_lines
}

/// helper function to create a centered rect using up certain percentage of the available rect `r`
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Like centered_rect but enforces minimum dimensions so overlays remain usable
/// on small terminals. Will not exceed the available rect `r`.
pub(crate) fn centered_rect_min(
    percent_x: u16,
    percent_y: u16,
    min_w: u16,
    min_h: u16,
    r: Rect,
) -> Rect {
    let rect = centered_rect(percent_x, percent_y, r);
    let w = rect.width.max(min_w).min(r.width);
    let h = rect.height.max(min_h).min(r.height);
    let x = r.x + (r.width - w) / 2;
    let y = r.y + (r.height - h) / 2;
    Rect::new(x, y, w, h)
}

pub(crate) fn centered_rect_fixed(width: u16, height: u16, r: Rect) -> Rect {
    let w = width.min(r.width);
    let h = height.min(r.height);
    let x = r.x + (r.width - w) / 2;
    let y = r.y + (r.height - h) / 2;
    Rect::new(x, y, w, h)
}

pub fn count_wrapped_lines(text: &str, width: usize) -> usize {
    if width == 0 {
        return 0;
    }
    let mut total_lines = 0;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            total_lines += 1;
            continue;
        }
        let mut current_line_len = 0;
        let mut first = true;
        for word in line.split(' ') {
            let word_len = word.chars().count();
            if word_len == 0 {
                if current_line_len + 1 > width {
                    total_lines += 1;
                    current_line_len = 1;
                } else {
                    current_line_len += 1;
                }
                continue;
            }

            let space_needed = if first { 0 } else { 1 };
            if current_line_len + space_needed + word_len <= width {
                current_line_len += space_needed + word_len;
                first = false;
            } else {
                if word_len > width {
                    if current_line_len + space_needed < width {
                        let remaining_on_current = width - (current_line_len + space_needed);
                        let mut remaining_word_len = word_len - remaining_on_current;
                        total_lines += 1;
                        while remaining_word_len > width {
                            total_lines += 1;
                            remaining_word_len -= width;
                        }
                        current_line_len = remaining_word_len;
                    } else {
                        total_lines += 1;
                        let mut remaining_word_len = word_len;
                        while remaining_word_len > width {
                            total_lines += 1;
                            remaining_word_len -= width;
                        }
                        current_line_len = remaining_word_len;
                    }
                } else {
                    total_lines += 1;
                    current_line_len = word_len;
                }
                first = false;
            }
        }
        total_lines += 1;
    }

    if text.is_empty() {
        return 0;
    }
    total_lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pipelines::{GitlabJob, JobItem};
    use crate::ui::helpers::{
        floor_char_boundary, get_label_color, get_stages_summary, render_labels_cell,
    };

    fn make_job(id: u64, stage: &str, name: &str, status: &str) -> JobItem {
        JobItem::from_gitlab(GitlabJob {
            id,
            stage: stage.to_string(),
            name: name.to_string(),
            status: status.to_string(),
            matrix: None,
        })
    }

    #[test]
    fn test_get_stages_summary() {
        // Test case 1: Stage with mixed success and skipped jobs
        // This stage should be reported as "success" status, and 100% success rate.
        let jobs = vec![
            make_job(1, "build", "compile", "success"),
            make_job(2, "build", "cache", "skipped"),
            make_job(3, "test", "unit", "failed"),
            make_job(4, "test", "integration", "success"),
        ];

        let summaries = get_stages_summary(&jobs);
        assert_eq!(summaries.len(), 2);

        // Build stage verification (success + skipped = success/100%)
        let build = summaries.iter().find(|s| s.name == "build").unwrap();
        assert_eq!(build.status, "success");
        assert_eq!(build.success, 2);
        assert_eq!(build.total, 2);
        assert_eq!(build.percent, 100);

        // Test stage verification (failed + success = failed/50%)
        let test = summaries.iter().find(|s| s.name == "test").unwrap();
        assert_eq!(test.status, "failed");
        assert_eq!(test.success, 1);
        assert_eq!(test.total, 2);
        assert_eq!(test.percent, 50);
    }

    #[test]
    fn test_count_wrapped_lines() {
        assert_eq!(count_wrapped_lines("", 10), 0);
        assert_eq!(count_wrapped_lines("", 0), 0);
        assert_eq!(count_wrapped_lines("hello", 10), 1);
        assert_eq!(count_wrapped_lines("hello", 5), 1);
        assert_eq!(count_wrapped_lines("hello", 3), 2);
        assert_eq!(count_wrapped_lines("hello world", 5), 2);
        assert_eq!(count_wrapped_lines("a b c", 3), 2);
        assert_eq!(count_wrapped_lines("hello\nworld", 10), 2);
        assert_eq!(count_wrapped_lines("hello\nworld", 10), 2);
    }

    #[test]
    fn test_get_label_color() {
        let color1 = get_label_color("bug");
        let _color2 = get_label_color("feature");
        let color3 = get_label_color("bug");

        assert_eq!(color1, color3);
        match color1 {
            Color::Rgb(_, _, _) => {}
            _ => panic!("Expected Rgb color"),
        }
    }

    #[test]
    fn test_render_labels_cell() {
        let labels = vec!["bug".to_string(), "backend".to_string()];
        let cell_empty = render_labels_cell(&[], "", false, false, 24);
        let cell_str_empty = format!("{:?}", cell_empty);
        assert!(cell_str_empty.contains("—"));

        let cell_normal = render_labels_cell(&labels, "", false, false, 24);
        let cell_str = format!("{:?}", cell_normal);
        assert!(cell_str.contains("bug"));
        assert!(cell_str.contains("backend"));
    }

    #[test]
    fn test_format_comment_with_suggestions() {
        let body = "This is a comment\n```suggestion\nnew line content\n```\noutside suggestion";
        let file_path = "src/app.rs";
        let all_lines = vec![crate::app::DiffLine {
            content: "old line content".to_string(),
            line_type: crate::app::DiffLineType::Deletion,
            file_path: "src/app.rs".to_string(),
            old_line_num: Some(1),
            new_line_num: None,
            syntax_highlighted: None,
        }];
        let prefix = "prefix";
        let prefix_style = Style::default();

        let formatted = format_comment_with_suggestions(
            body,
            file_path,
            None,
            None,
            Some(1),
            Some(1),
            &all_lines,
            prefix,
            prefix_style,
        );

        assert_eq!(formatted.len(), 6);
        assert_eq!(formatted[0].2[0].1, "This is a comment");
        assert_eq!(formatted[1].2[0].1, "┌─── Code Suggestion ───");
        assert_eq!(formatted[2].2[0].1, "│ - ");
        assert_eq!(formatted[2].2[1].1, "old line content");
        assert_eq!(formatted[3].2[0].1, "│ + ");
        assert_eq!(formatted[3].2[1].1, "new line content");
        assert_eq!(formatted[4].2[0].1, "└─── End of Suggestion ───");
        assert_eq!(formatted[5].2[0].1, "outside suggestion");
    }

    #[test]
    fn test_floor_char_boundary() {
        let s = "👕hello";
        assert_eq!(floor_char_boundary(s, 0), 0);
        assert_eq!(floor_char_boundary(s, 1), 0);
        assert_eq!(floor_char_boundary(s, 2), 0);
        assert_eq!(floor_char_boundary(s, 3), 0);
        assert_eq!(floor_char_boundary(s, 4), 4);
        assert_eq!(floor_char_boundary(s, 5), 5);
        assert_eq!(floor_char_boundary(s, 999), s.len());
    }
}
