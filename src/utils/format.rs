use chrono::{DateTime, Utc};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Replaces tab characters with spaces, advancing to the next `tab_width`
/// column each time.
///
/// Tab stops rather than a fixed number of spaces per tab, because a tab in the
/// middle of a line is an alignment request, not an indent: `gofmt` lines up
/// consecutive struct fields and their tags that way, and expanding each tab to
/// the same width would leave those columns ragged.
pub fn expand_tabs(text: &str, tab_width: usize) -> String {
    if !text.contains('\t') {
        return text.to_string();
    }
    let tab_width = tab_width.max(1);
    let mut out = String::with_capacity(text.len() + tab_width);
    let mut column = 0usize;
    for ch in text.chars() {
        if ch == '\t' {
            let pad = tab_width - (column % tab_width);
            out.push_str(&" ".repeat(pad));
            column += pad;
        } else {
            out.push(ch);
            column += 1;
        }
    }
    out
}

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

/// Word-wrap `text` to `width` columns, preserving blank lines between
/// paragraphs (separated by `\n`). Words are never broken mid-word.
pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.len() + 1 + word.len() <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                out.push(std::mem::take(&mut current));
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
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

pub fn strip_ansi_escapes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if let Some(next_c) = chars.next() {
                if next_c == '[' {
                    for seq_c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&seq_c) {
                            break;
                        }
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

pub fn parse_ansi_trace(trace: &str, theme: &crate::config::Theme) -> Vec<Line<'static>> {
    trace
        .lines()
        .map(|raw_line| {
            let (gl_ts, line) = strip_gl_ts(raw_line);
            let (prefix, content) = split_gh_prefix(line);

            let content_spans = if content.contains('\x1b') {
                parse_ansi_line(content, theme)
            } else {
                format_plain_line(content, theme)
            };

            let mut spans: Vec<Span<'static>> = Vec::new();
            if let Some(ts) = gl_ts {
                spans.push(Span::styled(
                    format!("{} ", ts),
                    Style::default()
                        .fg(theme.text_muted)
                        .add_modifier(Modifier::ITALIC),
                ));
            }
            if let Some(p) = prefix {
                spans.push(Span::styled(
                    p.to_string(),
                    Style::default().fg(theme.text_muted),
                ));
            }
            spans.extend(content_spans);
            Line::from(spans)
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
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if ch == '\u{1b}' && i + 1 < chars.len() && chars[i + 1] == '[' {
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
                if i >= chars.len() {
                    break;
                }
                let c = chars[i];
                i += 1;
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    if c == 'm' {
                        if !num_buf.is_empty() {
                            params.push(num_buf);
                        }
                        current_style = apply_sgr(&params, current_style, theme);
                    }
                    break;
                }
                match c {
                    ';' => {
                        if !num_buf.is_empty() {
                            params.push(std::mem::take(&mut num_buf));
                        } else {
                            params.push("0".to_string());
                        }
                    }
                    '0'..='9' => {
                        num_buf.push(c);
                    }
                    _ => {
                        num_buf.push(c);
                    }
                }
            }
        } else {
            current_text.push(ch);
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

/// Strips a GitLab Runner timestamped-log prefix (`FF_TIMESTAMPS`), which
/// prepends every line with an ISO 8601 timestamp, a space, and a 4-character
/// metadata field: two hex flag digits, a stream indicator (`O` stdout, `E`
/// stderr) and an append flag (`+` when the line continues the previous one, a
/// space otherwise) — e.g. `2024-05-14T11:19:20.000000Z 00O+`.
///
/// Returns `(Some(timestamp), content)`, dropping the metadata field the same
/// way GitLab's own log viewer does, or `(None, original)` when the prefix is
/// absent — which leaves GitHub Actions timestamps to `strip_gh_ts`.
fn strip_gl_ts(line: &str) -> (Option<&str>, &str) {
    let (Some(ts), rest) = strip_gh_ts(line) else {
        return (None, line);
    };
    let meta = rest.as_bytes();
    if meta.len() >= 5
        && meta[0] == b' '
        && meta[1].is_ascii_hexdigit()
        && meta[2].is_ascii_hexdigit()
        && (meta[3] == b'O' || meta[3] == b'E')
        && (meta[4] == b' ' || meta[4] == b'+')
    {
        return (Some(ts), &rest[5..]);
    }
    (None, line)
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
    fn test_expand_tabs_indentation() {
        assert_eq!(expand_tabs("no tabs here", 4), "no tabs here");
        assert_eq!(expand_tabs("\tone", 4), "    one");
        assert_eq!(expand_tabs("\t\ttwo", 4), "        two");
        assert_eq!(expand_tabs("\tone", 8), "        one");
        assert_eq!(expand_tabs("", 4), "");
    }

    #[test]
    fn test_expand_tabs_advances_to_the_next_stop() {
        // Not a fixed width per tab: the pad is whatever reaches the next stop.
        assert_eq!(expand_tabs("ab\tc", 4), "ab  c");
        assert_eq!(expand_tabs("abc\td", 4), "abc d");
        assert_eq!(expand_tabs("abcd\te", 4), "abcd    e");
    }

    #[test]
    fn test_expand_tabs_keeps_a_column_grid() {
        // Two lines whose text lands in the same stop keep the same column
        // after a tab, which a fixed number of spaces per tab would not give.
        let a = expand_tabs("ab\tX", 4);
        let b = expand_tabs("abc\tX", 4);
        assert_eq!(a.find('X'), b.find('X'));
    }

    #[test]
    fn test_expand_tabs_treats_zero_width_as_one() {
        // A misconfigured width must not divide by zero or eat the tab.
        assert_eq!(expand_tabs("\tx", 0), " x");
    }

    #[test]
    fn test_expand_tabs_leaves_multibyte_text_intact() {
        assert_eq!(expand_tabs("\tπλ→", 4), "    πλ→");
    }

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

    #[test]
    fn test_strip_ansi_escapes_preserves_non_ascii() {
        let input = "\u{1b}[32m✓\u{1b}[0m src/lib.rs — 3 tests ✔";
        assert_eq!(strip_ansi_escapes(input), "✓ src/lib.rs — 3 tests ✔");
    }

    #[test]
    fn test_parse_ansi_line_preserves_non_ascii() {
        let theme = crate::config::Theme::default();
        let spans = parse_ansi_line("\u{1b}[32m✓\u{1b}[0m tests passed — é 日本", &theme);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "✓ tests passed — é 日本");
    }

    #[test]
    fn test_strip_gl_ts_drops_metadata_field() {
        assert_eq!(
            strip_gl_ts("2024-05-14T11:19:20.000000Z 00O Preparing docker"),
            (Some("2024-05-14T11:19:20.000000Z"), "Preparing docker")
        );
        assert_eq!(
            strip_gl_ts("2024-05-14T11:19:20.000000Z 01E error: boom"),
            (Some("2024-05-14T11:19:20.000000Z"), "error: boom")
        );
        // `+` marks a line continuing the previous one
        assert_eq!(
            strip_gl_ts("2024-05-14T11:19:20.000000Z 00O+environment..."),
            (Some("2024-05-14T11:19:20.000000Z"), "environment...")
        );
    }

    #[test]
    fn test_strip_gl_ts_leaves_other_lines_untouched() {
        // GitHub Actions: same timestamp shape, no GitLab metadata field
        let gh = "2024-05-14T11:19:20.0000000Z Run actions/checkout@v4";
        assert_eq!(strip_gl_ts(gh), (None, gh));
        let plain = "$ cargo test";
        assert_eq!(strip_gl_ts(plain), (None, plain));
    }

    #[test]
    fn test_parse_ansi_trace_strips_gitlab_prefix() {
        let theme = crate::config::Theme::default();
        let trace =
            "2026-08-18T16:54:49.475670Z 01O \u{1b}[32m✓\u{1b}[0m src/foo.test.ts (19 tests)";
        let lines = parse_ansi_trace(trace, &theme);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(
            text,
            "2026-08-18T16:54:49.475670Z ✓ src/foo.test.ts (19 tests)"
        );
    }
}
