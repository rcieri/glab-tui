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
    if let Some(pr_id) = r#ref
        .strip_prefix("refs/pull/")
        .and_then(|s| s.strip_suffix("/merge"))
    {
        format!("PR #{}", pr_id)
    } else if let Some(pr_id) = r#ref
        .strip_prefix("refs/pull/")
        .and_then(|s| s.strip_suffix("/head"))
    {
        format!("PR #{}", pr_id)
    } else if let Some(pr_id) = r#ref
        .strip_prefix("refs/pull/")
        .and_then(|s| s.split('/').next())
    {
        format!("PR #{}", pr_id)
    } else if let Some(mr_id) = r#ref
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

pub fn parse_ansi_trace(trace: &str, theme: &crate::config::Theme) -> Vec<Line<'static>> {
    trace
        .lines()
        .map(|raw_line| {
            if raw_line.contains('\x1b') {
                let spans = parse_ansi_line(raw_line, theme);
                Line::from(spans)
            } else {
                let spans = format_plain_line(raw_line, theme);
                Line::from(spans)
            }
        })
        .collect()
}

fn parse_ansi_line(line: &str, theme: &crate::config::Theme) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut current_style = Style::default().fg(theme.text_normal);
    let mut current_text = String::new();
    let bytes: Vec<u8> = line.bytes().collect();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            if !current_text.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut current_text),
                    current_style,
                ));
            }
            i += 2;
            let mut params = Vec::new();
            let mut num_buf = String::new();
            loop {
                if i >= bytes.len() {
                    break;
                }
                let c = bytes[i];
                i += 1;
                if (0x40..=0x7e).contains(&c) {
                    if c == b'm' {
                        if !num_buf.is_empty() {
                            params.push(num_buf);
                        }
                        current_style = apply_sgr(&params, current_style, theme);
                    }
                    break;
                }
                match c {
                    b';' => {
                        if !num_buf.is_empty() {
                            params.push(std::mem::take(&mut num_buf));
                        } else {
                            params.push("0".to_string());
                        }
                    }
                    b'0'..=b'9' => {
                        num_buf.push(c as char);
                    }
                    _ => {
                        num_buf.push(c as char);
                    }
                }
            }
        } else {
            current_text.push(b as char);
            i += 1;
        }
    }
    if !current_text.is_empty() {
        spans.push(Span::styled(current_text, current_style));
    }
    spans
}

fn apply_sgr(params: &[String], current: Style, theme: &crate::config::Theme) -> Style {
    let mut style = current;
    let mut i = 0;
    while i < params.len() {
        let p: u8 = params[i].parse().unwrap_or(0);
        match p {
            0 => {
                style = Style::default().fg(theme.text_normal);
            }
            1 => {
                style = style.add_modifier(Modifier::BOLD);
            }
            3 => {
                style = style.add_modifier(Modifier::ITALIC);
            }
            4 => {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            7 => {
                style = style.add_modifier(Modifier::REVERSED);
            }
            22 => {
                style = style.remove_modifier(Modifier::BOLD);
            }
            23 => {
                style = style.remove_modifier(Modifier::ITALIC);
            }
            24 => {
                style = style.remove_modifier(Modifier::UNDERLINED);
            }
            27 => {
                style = style.remove_modifier(Modifier::REVERSED);
            }
            30..=37 => {
                let c = match p {
                    30 => Color::Black,
                    31 => Color::Red,
                    32 => Color::Green,
                    33 => Color::Yellow,
                    34 => Color::Blue,
                    35 => Color::Magenta,
                    36 => Color::Cyan,
                    37 => Color::Gray,
                    _ => Color::Reset,
                };
                style = style.fg(c);
            }
            38 => {
                i += 1;
                continue;
            }
            39 => {
                style = style.fg(theme.text_normal);
            }
            40..=47 => {
                let c = match p {
                    40 => Color::Black,
                    41 => Color::Red,
                    42 => Color::Green,
                    43 => Color::Yellow,
                    44 => Color::Blue,
                    45 => Color::Magenta,
                    46 => Color::Cyan,
                    47 => Color::Gray,
                    _ => Color::Reset,
                };
                style = style.bg(c);
            }
            48 => {
                i += 1;
                continue;
            }
            49 => {
                style = style.bg(Color::Reset);
            }
            90..=97 => {
                let c = match p {
                    90 => Color::DarkGray,
                    91 => Color::LightRed,
                    92 => Color::LightGreen,
                    93 => Color::LightYellow,
                    94 => Color::LightBlue,
                    95 => Color::LightMagenta,
                    96 => Color::LightCyan,
                    97 => Color::White,
                    _ => Color::Reset,
                };
                style = style.fg(c);
            }
            100..=107 => {
                let c = match p {
                    100 => Color::DarkGray,
                    101 => Color::LightRed,
                    102 => Color::LightGreen,
                    103 => Color::LightYellow,
                    104 => Color::LightBlue,
                    105 => Color::LightMagenta,
                    106 => Color::LightCyan,
                    107 => Color::White,
                    _ => Color::Reset,
                };
                style = style.bg(c);
            }
            _ => {}
        }
        i += 1;
    }
    style
}

fn format_plain_line(line: &str, theme: &crate::config::Theme) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut pos = 0;
    let chars: Vec<char> = line.chars().collect();

    if let Some(end) = try_parse_timestamp(&chars) {
        spans.push(Span::styled(
            line[..end].to_string(),
            Style::default()
                .fg(theme.text_muted)
                .add_modifier(Modifier::ITALIC),
        ));
        pos = end;
    }

    if let Some((start, end)) = try_parse_log_level(&chars, pos) {
        // push any gap text before the log level
        if start > pos {
            spans.push(Span::styled(
                line[pos..start].to_string(),
                Style::default().fg(theme.text_normal),
            ));
        }
        let level_text = line[start..end].to_string();
        let lower = level_text.trim().to_lowercase();
        let level_upper = lower.trim_start_matches(|c: char| !c.is_alphanumeric());
        let (lvl_style, offset) = {
            let lvl = level_upper;
            let style = if lvl.contains("error") || lvl.contains("fatal") || lvl.contains("panic") {
                Style::default().fg(theme.red).add_modifier(Modifier::BOLD)
            } else if lvl.contains("warn") {
                Style::default().fg(theme.yellow)
            } else if lvl.contains("info") {
                Style::default().fg(theme.blue)
            } else if lvl.contains("debug") {
                Style::default().fg(theme.purple)
            } else if lvl.contains("trace") {
                Style::default().fg(theme.text_muted)
            } else {
                Style::default()
            };
            (style, 0)
        };
        if offset > 0 {
            spans.push(Span::styled(
                level_text[..offset].to_string(),
                Style::default().fg(theme.text_normal),
            ));
        }
        spans.push(Span::styled(level_text[offset..].to_string(), lvl_style));
        pos = end;
    }

    let remaining = &line[pos..];
    if !remaining.is_empty() {
        spans.push(Span::styled(
            remaining.to_string(),
            keyword_style(remaining, theme),
        ));
    }
    spans
}

fn try_parse_timestamp(chars: &[char]) -> Option<usize> {
    // ISO 8601: YYYY-MM-DDTHH:MM:SS or YYYY-MM-DD HH:MM:SS
    if chars.len() >= 19
        && chars[0].is_ascii_digit()
        && chars[4] == '-'
        && chars[7] == '-'
        && (chars[10] == 'T' || chars[10] == ' ')
        && chars[13] == ':'
        && chars[16] == ':'
    {
        let mut end = 19;
        // optional .fff milliseconds
        if chars.len() > end && chars[end] == '.' {
            end += 1;
            while end < chars.len() && chars[end].is_ascii_digit() {
                end += 1;
            }
        }
        return Some(end);
    }
    // Time: HH:MM:SS
    if chars.len() >= 8 && chars[2] == ':' && chars[5] == ':' {
        let mut end = 8;
        if chars.len() > end && chars[end] == '.' {
            end += 1;
            while end < chars.len() && chars[end].is_ascii_digit() {
                end += 1;
            }
        }
        return Some(end);
    }
    None
}

fn try_parse_log_level(chars: &[char], start: usize) -> Option<(usize, usize)> {
    if start >= chars.len() {
        return None;
    }
    let slice: String = chars[start..].iter().collect();
    let lower = slice.to_lowercase();
    let level_patterns: &[(&str, &str)] = &[
        ("error", "ERROR"),
        ("warn", "WARN"),
        ("warning", "WARNING"),
        ("info", "INFO"),
        ("debug", "DEBUG"),
        ("trace", "TRACE"),
        ("fatal", "FATAL"),
        ("panic", "PANIC"),
    ];
    for (pat, _display) in level_patterns {
        // Match pattern with brackets like [ERROR] or (ERROR)
        for bracket_open in &["[", "(", " "] {
            let prefix = if *bracket_open == " " {
                " "
            } else {
                *bracket_open
            };
            let prefix_len = prefix.len();
            if chars.len() > start + prefix_len + pat.len() {
                let check: String = chars[start + prefix_len..].iter().take(pat.len()).collect();
                if check.to_lowercase() == *pat {
                    let prefix_match: String = chars[start..start + prefix_len].iter().collect();
                    if prefix_match == prefix || prefix_match == *bracket_open {
                        let lvl_end = start + prefix_len + pat.len();
                        let bracket_close = if *bracket_open == "[" {
                            "]"
                        } else if *bracket_open == "(" {
                            ")"
                        } else {
                            ""
                        };
                        let mut end = lvl_end;
                        if !bracket_close.is_empty()
                            && chars.len() > end
                            && chars[end].to_string() == bracket_close
                        {
                            end += 1;
                        }
                        if chars.len() > end && chars[end] == ':' {
                            end += 1;
                        }
                        return Some((start, end));
                    }
                }
            }
        }
    }
    None
}

fn keyword_style(line: &str, theme: &crate::config::Theme) -> Style {
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
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_ref() {
        assert_eq!(format_ref("refs/merge-requests/123/merge"), "MR !123");
        assert_eq!(format_ref("refs/merge-requests/456/head"), "MR !456");
        assert_eq!(format_ref("refs/pull/789/merge"), "PR #789");
        assert_eq!(format_ref("refs/pull/101/head"), "PR #101");
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
