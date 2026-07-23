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
    let s = s.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = s.strip_prefix(quote).and_then(|s| s.strip_suffix(quote)) {
            return inner.trim().to_string();
        }
    }
    s.to_string()
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
            let (prefix, content) = split_gh_prefix(raw_line);

            let content_spans = if content.contains('\x1b') {
                parse_ansi_line(content, theme)
            } else {
                format_plain_line(content, theme)
            };

            if let Some(p) = prefix {
                let mut spans = vec![Span::styled(
                    p.to_string(),
                    Style::default().fg(theme.text_muted),
                )];
                spans.extend(content_spans);
                Line::from(spans)
            } else {
                Line::from(content_spans)
            }
        })
        .collect()
}

/// Strips the GitHub Actions log prefix `<job_name>\t<step_name>\t` if present.
/// Returns `(Some(prefix), content)` or `(None, whole_line)`.
fn split_gh_prefix(line: &str) -> (Option<&str>, &str) {
    if let Some(first_tab) = line.find('\t') {
        if let Some(second_tab) = line[first_tab + 1..].find('\t') {
            let prefix_end = first_tab + 1 + second_tab + 1;
            return (Some(&line[..prefix_end]), &line[prefix_end..]);
        }
    }
    (None, line)
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
    // Strip GitHub Actions timestamp if present: YYYY-MM-DDTHH:MM:SS.fffffffZ
    let (ts, rest) = strip_gh_ts(line);
    let body = rest.trim_start();

    let body_style = classify_line(body, &body.to_lowercase(), theme);

    let mut spans = Vec::new();
    if let Some(timestamp) = ts {
        spans.push(Span::styled(
            timestamp.to_string(),
            Style::default()
                .fg(theme.text_muted)
                .add_modifier(Modifier::ITALIC),
        ));
        if rest.len() > body.len() {
            // space between timestamp and body
            let space_len = rest.len() - body.len();
            spans.push(Span::styled(
                rest[..space_len].to_string(),
                Style::default().fg(theme.text_normal),
            ));
        }
    }
    spans.push(Span::styled(body.to_string(), body_style));
    spans
}

/// Strips a GitHub Actions timestamp (`YYYY-MM-DDTHH:MM:SS.fffffffZ`) from
/// the start of a line. Returns `(Some(ts), rest)` or `(None, original)`.
fn strip_gh_ts(line: &str) -> (Option<&str>, &str) {
    let bytes = line.as_bytes();
    // Need at least: YYYY-MM-DDTHH:MM:SS (19) + .f (2 more) + Z (1) = 22 minimum
    if bytes.len() >= 22
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'.'
        && bytes[0].is_ascii_digit()
    {
        let mut end = 20;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end < bytes.len() && bytes[end] == b'Z' {
            end += 1;
            return (Some(&line[..end]), &line[end..]);
        }
    }
    (None, line)
}

fn classify_line(line: &str, lower: &str, theme: &crate::config::Theme) -> Style {
    // GitHub Actions section markers (checked first)
    if lower.starts_with("##[group]") {
        return Style::default().fg(theme.blue).add_modifier(Modifier::BOLD);
    }
    if lower.starts_with("##[endgroup]") {
        return Style::default().fg(theme.text_muted);
    }
    if lower.starts_with("##[command]") {
        return Style::default().fg(theme.purple);
    }
    if lower.starts_with("##[debug]") {
        return Style::default().fg(theme.text_muted);
    }
    if lower.starts_with("##[warning]") {
        return Style::default().fg(theme.yellow);
    }
    if lower.starts_with("##[error]") {
        return Style::default().fg(theme.red).add_modifier(Modifier::BOLD);
    }
    if lower.starts_with("##[section]") {
        return Style::default().fg(theme.blue);
    }

    // Error indicators (red bold)
    if lower.contains("error")
        || lower.contains("failed")
        || lower.contains("panic")
        || lower.contains("err!")
        || lower.contains("fail")
        || lower.contains("fatal")
        || lower.contains("aborted")
        || lower.contains("terminated")
        || lower.contains("traceback")
        || lower.contains("exception")
        || lower.contains("unresolved")
        || lower.contains("unstaged")
        || lower.starts_with("error[")
        || lower.starts_with("error:")
        || lower.contains("error code")
        || lower.contains("exit code")
        || lower.contains("exit status")
        || lower.contains("process completed with")
    {
        return Style::default().fg(theme.red).add_modifier(Modifier::BOLD);
    }
    // Rust compiler errors and backtraces
    if lower.starts_with("thread '") && lower.contains("panicked") {
        return Style::default().fg(theme.red).add_modifier(Modifier::BOLD);
    }
    if lower.starts_with("error[") || lower.starts_with("  --> ") {
        return Style::default().fg(theme.red);
    }

    // Warning indicators (yellow)
    if lower.contains("warning")
        || lower.contains("warn")
        || lower.contains("deprecated")
        || lower.contains("notice")
        || lower.starts_with("warning[")
        || lower.starts_with("warning:")
    {
        return Style::default().fg(theme.yellow);
    }

    // Success indicators (green)
    if lower.contains("success")
        || lower.contains("successfully")
        || lower.contains("completed")
        || lower.contains("finished")
        || lower.starts_with("pass")
        || lower.contains(" passed ")
        || lower.starts_with("ok ")
        || lower.starts_with("✓")
        || lower.contains("built")
        || lower.starts_with("--> using cache")
        || lower.starts_with("dependency successfully")
    {
        return Style::default().fg(theme.green);
    }

    // Shell commands (purple bold)
    if line.trim_start().starts_with('$') || line.trim_start().starts_with('>') {
        return Style::default()
            .fg(theme.purple)
            .add_modifier(Modifier::BOLD);
    }
    // GitLab CI section markers
    if lower.contains("section_start") || lower.contains("section_end") {
        return Style::default().fg(theme.blue);
    }

    // Info / informational (blue)
    if lower.contains("info")
        || lower.contains("running")
        || lower.contains("starting")
        || lower.contains("building")
        || lower.contains("compiling")
        || lower.contains("linking")
        || lower.contains("installing")
        || lower.contains("fetching")
        || lower.contains("cloning")
        || lower.contains("checking out")
        || lower.contains("downloading")
        || lower.contains("uploading")
        || lower.contains("pushing")
        || lower.contains("pulling")
        || lower.contains("syncing")
        || lower.contains("processing")
        || lower.contains("generating")
        || lower.contains("resolving")
    {
        return Style::default().fg(theme.blue);
    }

    // Debug / verbose (dim purple)
    if lower.contains("debug")
        || lower.contains("trace")
        || lower.contains("verbose")
        || lower.starts_with("+ ")
        || lower.starts_with("++ ")
    {
        return Style::default().fg(theme.purple);
    }

    // Test output patterns
    if lower.starts_with("not ok") {
        return Style::default().fg(theme.red).add_modifier(Modifier::BOLD);
    }
    if lower.starts_with("ok ") && !lower.contains("not ok") {
        return Style::default().fg(theme.green);
    }

    // Docker patterns
    if lower.starts_with("step ")
        || lower.starts_with("--->")
        || lower.starts_with("successfully tagged")
        || lower.starts_with("successfully built")
    {
        return Style::default().fg(theme.blue);
    }

    // Default
    Style::default().fg(theme.text_normal)
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
            (
                "Resolve".to_string(),
                "\"Fix connection leak\" in db".to_string()
            )
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
            (
                "".to_string(),
                "Title with 'single quotes' in it".to_string()
            )
        );
        assert_eq!(
            parse_mr_title_prefix("Fix \"bug\" in parser"),
            ("".to_string(), "Fix \"bug\" in parser".to_string())
        );
        assert_eq!(
            parse_mr_title_prefix("'wrapped in single quotes'"),
            ("".to_string(), "wrapped in single quotes".to_string())
        );
        assert_eq!(
            parse_mr_title_prefix("\"  padded  \""),
            ("".to_string(), "padded".to_string())
        );
        assert_eq!(
            parse_mr_title_prefix("  \"outer ws\"  "),
            ("".to_string(), "outer ws".to_string())
        );
        assert_eq!(
            parse_mr_title_prefix("\"\""),
            ("".to_string(), "".to_string())
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
