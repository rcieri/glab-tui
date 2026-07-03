use chrono::{DateTime, Utc};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub fn truncate(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        None => String::from(s),
        Some((idx, _)) => {
            let mut truncated = String::from(&s[..idx]);
            truncated.push_str("...");
            truncated
        }
    }
}

pub fn time_ago(date_str: &str) -> String {
    if let Ok(parsed_time) = date_str.parse::<DateTime<Utc>>() {
        let now = Utc::now();
        let duration = now.signed_duration_since(parsed_time);

        let days = duration.num_days();
        if days > 0 {
            if days == 1 {
                return "1 day ago".to_string();
            }
            return format!("{} days ago", days);
        }

        let hours = duration.num_hours();
        if hours > 0 {
            if hours == 1 {
                return "1 hr ago".to_string();
            }
            return format!("{} hrs ago", hours);
        }

        let minutes = duration.num_minutes();
        if minutes > 0 {
            if minutes == 1 {
                return "1 min ago".to_string();
            }
            return format!("{} mins ago", minutes);
        }

        "just now".to_string()
    } else {
        date_str.to_string()
    }
}

pub fn format_ref(r#ref: &str) -> String {
    if let Some(mr_id) = r#ref
        .strip_prefix("refs/merge-requests/")
        .and_then(|s| s.strip_suffix("/merge"))
    {
        format!("MR !{}", mr_id)
    } else if let Some(mr_id) = r#ref
        .strip_prefix("refs/merge-requests/")
        .and_then(|s| s.split('/').next())
    {
        format!("MR !{}", mr_id)
    } else if let Some(branch) = r#ref.strip_prefix("refs/heads/") {
        branch.to_string()
    } else if let Some(tag) = r#ref.strip_prefix("refs/tags/") {
        tag.to_string()
    } else {
        r#ref.to_string()
    }
}

fn extract_quotes(s: &str) -> String {
    if let Some(first_idx) = s.find('"') {
        if let Some(last_idx) = s.rfind('"') {
            if first_idx < last_idx {
                return s[first_idx + 1..last_idx].trim().to_string();
            }
        }
    }
    if let Some(first_idx) = s.find('\'') {
        if let Some(last_idx) = s.rfind('\'') {
            if first_idx < last_idx {
                return s[first_idx + 1..last_idx].trim().to_string();
            }
        }
    }
    s.trim().to_string()
}

/// Extracts a status prefix (like Draft:, Resolve:, WIP:) from a Merge Request title.
/// Returns a tuple of (ExtractedPrefix, CleanedTitle).
pub fn parse_mr_title_prefix(title: &str) -> (String, String) {
    let title_trimmed = title.trim();
    let prefixes = [
        "draft:",
        "wip:",
        "resolve:",
        "resolves:",
        "[draft]",
        "[wip]",
        "[resolve]",
        "draft ",
        "wip ",
        "resolve ",
        "resolves ",
    ];

    let title_lower = title_trimmed.to_lowercase();
    for p in prefixes {
        if title_lower.starts_with(p) {
            let prefix_len = p.len();
            let mut prefix = title_trimmed[..prefix_len].trim().to_string();
            prefix = prefix
                .trim_end_matches(':')
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .to_string();
            let remaining = title_trimmed[prefix_len..].trim();
            return (prefix, extract_quotes(remaining));
        }
    }

    (String::new(), extract_quotes(title_trimmed))
}

pub fn render_markdown(markdown: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("# ") {
            let content = trimmed.strip_prefix("# ").unwrap_or(trimmed);
            lines.push(Line::from(vec![Span::styled(
                format!("# {}", content),
                Style::default()
                    .fg(Color::Rgb(187, 153, 238))
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )]));
        } else if trimmed.starts_with("## ") {
            let content = trimmed.strip_prefix("## ").unwrap_or(trimmed);
            lines.push(Line::from(vec![Span::styled(
                format!("## {}", content),
                Style::default()
                    .fg(Color::Rgb(97, 175, 239))
                    .add_modifier(Modifier::BOLD),
            )]));
        } else if trimmed.starts_with("### ") {
            let content = trimmed.strip_prefix("### ").unwrap_or(trimmed);
            lines.push(Line::from(vec![Span::styled(
                format!("### {}", content),
                Style::default()
                    .fg(Color::Rgb(152, 195, 121))
                    .add_modifier(Modifier::BOLD),
            )]));
        } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            let content = if trimmed.starts_with("- ") {
                trimmed.strip_prefix("- ").unwrap()
            } else {
                trimmed.strip_prefix("* ").unwrap()
            };
            let mut spans = vec![Span::styled(
                "  • ",
                Style::default()
                    .fg(Color::Rgb(187, 153, 238))
                    .add_modifier(Modifier::BOLD),
            )];
            spans.extend(parse_inline_styles(content));
            lines.push(Line::from(spans));
        } else if trimmed.starts_with("> ") {
            let content = trimmed.strip_prefix("> ").unwrap_or(trimmed);
            let mut spans = vec![Span::styled(
                "  ▌ ",
                Style::default().fg(Color::Rgb(127, 132, 142)),
            )];
            spans.extend(parse_inline_styles(content));
            lines.push(Line::from(spans));
        } else {
            lines.push(Line::from(parse_inline_styles(line)));
        }
    }
    lines
}

fn parse_inline_styles(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let chars = text.chars().collect::<Vec<char>>();
    let mut i = 0;
    let mut current_segment = String::new();

    while i < chars.len() {
        if chars[i] == '`' {
            if !current_segment.is_empty() {
                spans.push(Span::styled(
                    current_segment.clone(),
                    Style::default().fg(Color::Rgb(171, 178, 191)),
                ));
                current_segment.clear();
            }
            i += 1;
            let mut code = String::new();
            while i < chars.len() && chars[i] != '`' {
                code.push(chars[i]);
                i += 1;
            }
            spans.push(Span::styled(
                code,
                Style::default()
                    .fg(Color::Rgb(224, 108, 117))
                    .bg(Color::Rgb(40, 44, 52)),
            ));
            if i < chars.len() {
                i += 1;
            }
        } else if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            if !current_segment.is_empty() {
                spans.push(Span::styled(
                    current_segment.clone(),
                    Style::default().fg(Color::Rgb(171, 178, 191)),
                ));
                current_segment.clear();
            }
            i += 2;
            let mut bold_text = String::new();
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '*') {
                bold_text.push(chars[i]);
                i += 1;
            }
            if i < chars.len()
                && (i + 1 >= chars.len() || !(chars[i] == '*' && chars[i + 1] == '*'))
            {
                bold_text.push(chars[i]);
                i += 1;
            }
            spans.push(Span::styled(
                bold_text,
                Style::default()
                    .fg(Color::Rgb(220, 223, 228))
                    .add_modifier(Modifier::BOLD),
            ));
            if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
                i += 2;
            }
        } else {
            current_segment.push(chars[i]);
            i += 1;
        }
    }

    if !current_segment.is_empty() {
        spans.push(Span::styled(
            current_segment,
            Style::default().fg(Color::Rgb(171, 178, 191)),
        ));
    }

    spans
}

pub fn strip_ansi_escapes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        if b == 0x1b {
            if let Some(next_b) = bytes.next() {
                if next_b == b'[' {
                    while let Some(next_c) = bytes.next() {
                        if (0x40..=0x7e).contains(&next_c) {
                            break;
                        }
                    }
                }
            }
        } else {
            result.push(b as char);
        }
    }
    result
}

pub fn format_job_trace(trace: &str, theme: &crate::config::Theme) -> Vec<Line<'static>> {
    let stripped = strip_ansi_escapes(trace);
    stripped
        .lines()
        .map(|line| {
            let mut style = Style::default().fg(theme.text_normal);
            let lower = line.to_lowercase();
            if lower.contains("error") || lower.contains("failed") || lower.contains("panic") {
                style = style.fg(theme.red).add_modifier(Modifier::BOLD);
            } else if lower.contains("warning") || lower.contains("warn") {
                style = style.fg(theme.yellow);
            } else if lower.contains("success")
                || lower.contains("successfully")
                || lower.contains("completed")
            {
                style = style.fg(theme.green);
            } else if line.trim_start().starts_with('$') {
                style = style.fg(theme.purple).add_modifier(Modifier::BOLD);
            } else if lower.contains("info") {
                style = style.fg(theme.blue);
            }
            Line::from(Span::styled(line.to_string(), style))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_ref() {
        assert_eq!(format_ref("refs/merge-requests/123/merge"), "MR !123");
        assert_eq!(format_ref("refs/merge-requests/456/head"), "MR !456");
        assert_eq!(format_ref("refs/heads/feature/login"), "feature/login");
        assert_eq!(format_ref("refs/tags/v1.2.3"), "v1.2.3");
        assert_eq!(format_ref("main"), "main");
    }

    #[test]
    fn test_render_markdown() {
        let md = "# Header1\n## Header2\n- Bullet `code` item\nNormal line with **bold** text";
        let lines = render_markdown(md);
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn test_parse_mr_title_prefix() {
        assert_eq!(
            parse_mr_title_prefix("Draft: Implement user login"),
            ("Draft".to_string(), "Implement user login".to_string())
        );
        assert_eq!(
            parse_mr_title_prefix("resolve: fix connection leak"),
            ("resolve".to_string(), "fix connection leak".to_string())
        );
        assert_eq!(
            parse_mr_title_prefix("Resolve \"Fix connection leak\""),
            ("Resolve".to_string(), "Fix connection leak".to_string())
        );
        assert_eq!(
            parse_mr_title_prefix("Resolve: \"Fix connection leak\""),
            ("Resolve".to_string(), "Fix connection leak".to_string())
        );
        assert_eq!(
            parse_mr_title_prefix("Resolve: \"Fix connection leak\" in db"),
            ("Resolve".to_string(), "Fix connection leak".to_string())
        );
        assert_eq!(
            parse_mr_title_prefix("[WIP] add new routes"),
            ("WIP".to_string(), "add new routes".to_string())
        );
        assert_eq!(
            parse_mr_title_prefix("Regular MR title without prefix"),
            (
                "".to_string(),
                "Regular MR title without prefix".to_string()
            )
        );
        assert_eq!(
            parse_mr_title_prefix("\"Title wrapped in quotes\""),
            ("".to_string(), "Title wrapped in quotes".to_string())
        );
        assert_eq!(
            parse_mr_title_prefix("Title with 'single quotes' in it"),
            ("".to_string(), "single quotes".to_string())
        );
    }

    #[test]
    fn test_strip_ansi_escapes() {
        let input = "\u{1b}[32m[SUCCESS]\u{1b}[0m Job finished successfully";
        assert_eq!(
            strip_ansi_escapes(input),
            "[SUCCESS] Job finished successfully"
        );
    }
}
