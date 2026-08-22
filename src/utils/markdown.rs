use pulldown_cmark::{
    Alignment as MarkdownAlignment, BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag,
    TagEnd,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::app::highlight_line_syntax;
use crate::config::Theme;

pub fn render_markdown(markdown: &str, theme: &Theme, width: u16) -> Vec<Line<'static>> {
    let options = Options::ENABLE_GFM
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_TABLES;
    MarkdownRenderer::new(theme, width).render(Parser::new_ext(markdown, options))
}

fn decode_html_entities(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' {
            result.push(c);
            continue;
        }
        let mut entity = String::new();
        let mut terminated = false;
        for nc in chars.by_ref() {
            if nc == ';' {
                terminated = true;
                break;
            }
            entity.push(nc);
            if entity.len() > 10 {
                break;
            } // bail on malformed
        }
        if !terminated {
            result.push('&');
            result.push_str(&entity);
            continue;
        }
        match entity.as_str() {
            "amp" => result.push('&'),
            "lt" => result.push('<'),
            "gt" => result.push('>'),
            "quot" => result.push('"'),
            "apos" => result.push('\''),
            "nbsp" => result.push('\u{00A0}'),
            _ if entity.starts_with('#') => {
                let num_str = &entity[1..];
                let code = if num_str.starts_with('x') || num_str.starts_with('X') {
                    u32::from_str_radix(&num_str[1..], 16).ok()
                } else {
                    num_str.parse::<u32>().ok()
                };
                match code.and_then(char::from_u32) {
                    Some(ch) => result.push(ch),
                    None => {
                        result.push('&');
                        result.push_str(&entity);
                        result.push(';');
                    }
                }
            }
            _ => {
                result.push('&');
                result.push_str(&entity);
                result.push(';');
            }
        }
    }
    result
}

fn truncate_url(url: &str) -> String {
    const MAX_URL_LEN: usize = 40;
    let char_count = url.chars().count();
    if char_count <= MAX_URL_LEN {
        url.to_string()
    } else {
        let truncated: String = url.chars().take(MAX_URL_LEN - 1).collect();
        format!("{}…", truncated)
    }
}

/// Map a fenced code block language token (e.g. `"rust"`, `"python"`) to the
/// file extension syntect's `find_syntax_by_extension` expects (e.g. `"rs"`,
/// `"py"`). Returns `None` when the token is already the right extension or
/// simply unknown (caller falls back to the token itself).
fn resolve_syntax_lang(lang: &str) -> Option<&'static str> {
    // Compare case-insensitively by ASCII-lowering the input byte-by-byte.
    let lower = lang.to_ascii_lowercase();
    match lower.as_str() {
        "rust" => Some("rs"),
        "python" | "python3" => Some("py"),
        "javascript" => Some("js"),
        "typescript" => Some("ts"),
        "c++" | "cpp" => Some("cpp"),
        "c#" | "csharp" => Some("cs"),
        "kotlin" => Some("kt"),
        "swift" => Some("swift"),
        "scala" => Some("scala"),
        "haskell" => Some("hs"),
        "elixir" => Some("ex"),
        "erlang" => Some("erl"),
        "clojure" => Some("clj"),
        "ocaml" => Some("ml"),
        "f#" | "fsharp" => Some("fs"),
        "dockerfile" => Some("dockerfile"),
        "makefile" => Some("makefile"),
        "toml" => Some("toml"),
        "shell" | "zsh" | "bash" => Some("sh"),
        _ => None, // pass token through unchanged
    }
}

struct ListFrame {
    next_number: Option<u64>,
}

struct LinkState {
    destination: String,
    label: String,
}

struct CodeBlockState {
    language: Option<String>,
    content: String,
}

struct TableCellState {
    spans: Vec<Span<'static>>,
}

struct TableState {
    alignments: Vec<MarkdownAlignment>,
    rows: Vec<Vec<TableCellState>>,
    current_row: Vec<TableCellState>,
    current_cell: Vec<Span<'static>>,
}

struct MarkdownRenderer<'a> {
    theme: &'a Theme,
    width: u16,
    task_checked: bool,
    lines: Vec<Line<'static>>,

    current_line: Vec<Span<'static>>,
    list_stack: Vec<ListFrame>,
    links: Vec<LinkState>,
    image: Option<LinkState>,
    code_block: Option<CodeBlockState>,
    table: Option<TableState>,
    blockquote_depth: usize,
    heading_level: Option<u8>,
    bold_depth: usize,
    emphasis_depth: usize,
    strikethrough_depth: usize,
    /// Set to `true` after any block-level element (paragraph, heading, code
    /// block, list, blockquote, table, rule) finishes. The next block will
    /// prepend a blank separator line so the output breathes rather than running
    /// everything together. Cleared whenever we actually emit content.
    last_was_block: bool,
}

impl<'a> MarkdownRenderer<'a> {
    fn new(theme: &'a Theme, width: u16) -> Self {
        Self {
            theme,
            width,
            task_checked: false,
            lines: Vec::new(),
            current_line: Vec::new(),
            list_stack: Vec::new(),
            links: Vec::new(),
            image: None,
            code_block: None,
            table: None,
            blockquote_depth: 0,
            heading_level: None,
            bold_depth: 0,
            emphasis_depth: 0,
            strikethrough_depth: 0,
            last_was_block: false,
        }
    }

    fn render<'input>(mut self, parser: impl Iterator<Item = Event<'input>>) -> Vec<Line<'static>> {
        for event in parser {
            match event {
                Event::Start(tag) => self.start_tag(tag),
                Event::End(tag) => self.end_tag(tag),
                Event::Text(text) => {
                    if let Some(code_block) = self.code_block.as_mut() {
                        code_block.content.push_str(&text);
                    } else {
                        self.push_text(&text);
                    }
                }
                Event::Code(code) => {
                    let style = Style::default()
                        .fg(self.theme.red)
                        .bg(self.theme.inactive_bg);
                    self.push_span(code.into_string(), style);
                }
                Event::SoftBreak => self.push_text(" "),
                Event::HardBreak => self.finish_line(),
                Event::Rule => {
                    self.finish_line();
                    self.blank_between_blocks();
                    self.lines.push(Line::from(Span::styled(
                        "─".repeat(self.width.saturating_sub(2) as usize),
                        Style::default().fg(self.theme.text_muted),
                    )));
                    self.last_was_block = true;
                }
                Event::Html(html) | Event::InlineHtml(html) => {
                    let decoded = decode_html_entities(html.as_ref());
                    let style = Style::default()
                        .fg(self.theme.text_muted)
                        .add_modifier(Modifier::DIM);
                    self.push_span(decoded, style);
                }
                Event::FootnoteReference(label) => {
                    self.push_span(
                        format!("[{}]", label),
                        Style::default()
                            .fg(self.theme.text_muted)
                            .add_modifier(Modifier::ITALIC),
                    );
                }
                Event::TaskListMarker(checked) => {
                    self.task_checked = checked;
                    self.push_span(
                        if checked { "☑ " } else { "☐ " }.to_string(),
                        Style::default().fg(if checked {
                            self.theme.green
                        } else {
                            self.theme.text_muted
                        }),
                    );
                }
                Event::InlineMath(math) => self.push_text(&format!("${}$", math)),
                Event::DisplayMath(math) => self.push_text(&format!("$${}$$", math)),
            }
        }
        self.finish_line();
        self.lines
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.finish_line();
                self.blank_between_blocks();
                self.heading_level = Some(level as u8);
                self.push_span(
                    "▌ ".to_string(),
                    Style::default().fg(self.heading_color(level as u8)),
                );
            }
            Tag::CodeBlock(kind) => {
                self.finish_line();
                self.blank_between_blocks();
                let language = match kind {
                    CodeBlockKind::Indented => None,
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .filter(|language| !language.is_empty())
                        .map(str::to_string),
                };
                self.code_block = Some(CodeBlockState {
                    language,
                    content: String::new(),
                });
            }
            Tag::BlockQuote(kind) => {
                self.finish_line();
                self.blank_between_blocks();
                self.blockquote_depth += 1;
                if let Some(kind) = kind {
                    let (label, color) = match kind {
                        BlockQuoteKind::Note => ("NOTE", self.theme.blue),
                        BlockQuoteKind::Tip => ("TIP", self.theme.green),
                        BlockQuoteKind::Important => ("IMPORTANT", self.theme.purple),
                        BlockQuoteKind::Warning => ("WARNING", self.theme.yellow),
                        BlockQuoteKind::Caution => ("CAUTION", self.theme.red),
                    };
                    self.push_span(
                        label.to_string(),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    );
                    self.finish_line();
                }
            }
            Tag::List(first_number) => {
                // Only emit a blank separator before a top-level list, not for
                // nested lists (which are already inside a list item).
                if self.list_stack.is_empty() {
                    self.finish_line();
                    self.blank_between_blocks();
                }
                self.list_stack.push(ListFrame {
                    next_number: first_number,
                });
            }
            Tag::Item => {
                self.finish_line();
                let indent = "  ".repeat(self.list_stack.len().saturating_sub(1));
                let marker = self
                    .list_stack
                    .last_mut()
                    .map(|list| match list.next_number.as_mut() {
                        Some(number) => {
                            let marker = format!("{}. ", *number);
                            *number += 1;
                            marker
                        }
                        None => "• ".to_string(),
                    })
                    .unwrap_or_else(|| "• ".to_string());
                self.push_span(
                    format!("{}{}", indent, marker),
                    Style::default()
                        .fg(self.theme.purple)
                        .add_modifier(Modifier::BOLD),
                );
            }
            Tag::Table(alignments) => {
                self.finish_line();
                self.blank_between_blocks();
                self.table = Some(TableState {
                    alignments,
                    rows: Vec::new(),
                    current_row: Vec::new(),
                    current_cell: Vec::new(),
                });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.current_row.clear();
                }
            }
            Tag::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.current_cell.clear();
                }
            }
            Tag::Paragraph => {
                self.blank_between_blocks();
            }
            Tag::Strong => self.bold_depth += 1,
            Tag::Emphasis => self.emphasis_depth += 1,
            Tag::Strikethrough => self.strikethrough_depth += 1,
            Tag::Image { dest_url, .. } => {
                self.image = Some(LinkState {
                    destination: dest_url.into_string(),
                    label: String::new(),
                });
            }
            Tag::Link { dest_url, .. } => self.links.push(LinkState {
                destination: dest_url.into_string(),
                label: String::new(),
            }),
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.finish_line();
                self.last_was_block = true;
            }
            TagEnd::Item => {
                self.task_checked = false;
                self.finish_line();
            }
            TagEnd::Heading(_) => {
                self.finish_line();
                let was_h1 = self.heading_level == Some(1);
                self.heading_level = None;
                if was_h1 {
                    self.lines.push(Line::from(Span::styled(
                        "─".repeat(self.width.saturating_sub(2) as usize),
                        Style::default().fg(self.theme.text_muted),
                    )));
                }
                self.last_was_block = true;
            }
            TagEnd::CodeBlock => {
                if let Some(code_block) = self.code_block.take() {
                    self.render_code_block(code_block);
                }
                self.last_was_block = true;
            }
            TagEnd::BlockQuote(_) => {
                self.finish_line();
                self.blockquote_depth = self.blockquote_depth.saturating_sub(1);
                if self.blockquote_depth == 0 {
                    self.last_was_block = true;
                }
            }
            TagEnd::List(_) => {
                self.finish_line();
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.last_was_block = true;
                }
            }
            TagEnd::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.current_row.push(TableCellState {
                        spans: std::mem::take(&mut table.current_cell),
                    });
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.rows.push(std::mem::take(&mut table.current_row));
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.render_table(table);
                }
                self.last_was_block = true;
            }
            TagEnd::Strong => self.bold_depth = self.bold_depth.saturating_sub(1),
            TagEnd::Emphasis => self.emphasis_depth = self.emphasis_depth.saturating_sub(1),
            TagEnd::Strikethrough => {
                self.strikethrough_depth = self.strikethrough_depth.saturating_sub(1)
            }
            TagEnd::Image => {
                if let Some(image) = self.image.take() {
                    let label = if image.label.is_empty() {
                        "[image]".to_string()
                    } else {
                        format!("[image: {}]", image.label)
                    };
                    self.push_span(
                        label,
                        Style::default()
                            .fg(self.theme.text_muted)
                            .add_modifier(Modifier::ITALIC),
                    );
                    self.push_span(
                        format!(" ({})", truncate_url(&image.destination)),
                        Style::default()
                            .fg(self.theme.blue)
                            .add_modifier(Modifier::UNDERLINED),
                    );
                }
            }
            TagEnd::Link => {
                if let Some(link) = self.links.pop() {
                    if link.label != link.destination {
                        self.push_span(
                            format!(" ({})", truncate_url(&link.destination)),
                            Style::default()
                                .fg(self.theme.blue)
                                .add_modifier(Modifier::UNDERLINED),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn render_code_block(&mut self, code_block: CodeBlockState) {
        let border_style = Style::default()
            .fg(self.theme.text_muted)
            .bg(self.theme.inactive_bg);
        let mut opening = vec![Span::styled("┌─".to_string(), border_style)];
        if let Some(language) = &code_block.language {
            opening.push(Span::styled(
                format!(" {}", language),
                Style::default()
                    .fg(self.theme.purple)
                    .bg(self.theme.inactive_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        self.lines.push(Line::from(opening));

        // Resolve the fenced language token to the file extension syntect expects
        // (e.g. "rust" → "rs", "python" → "py"). If the token is already an
        // extension or is unknown, resolve_syntax_lang returns None and we use
        // the token itself as a fallback.
        let resolved: Option<String> = code_block
            .language
            .as_deref()
            .map(|lang| resolve_syntax_lang(lang).unwrap_or(lang).to_string());
        let lang_ext: Option<&str> = resolved.as_deref();

        for code_line in code_block.content.lines() {
            let content_spans: Vec<Span<'static>> = highlight_line_syntax("", code_line, lang_ext)
                .map(|highlighted| {
                    highlighted
                        .into_iter()
                        .map(|(style, text)| Span::styled(text, style.bg(self.theme.inactive_bg)))
                        .collect()
                })
                .unwrap_or_else(|| {
                    vec![Span::styled(
                        code_line.to_string(),
                        Style::default()
                            .fg(self.theme.text_normal)
                            .bg(self.theme.inactive_bg),
                    )]
                });
            let mut line_spans = vec![Span::styled("│ ".to_string(), border_style)];
            line_spans.extend(content_spans);
            self.lines.push(Line::from(line_spans));
        }

        self.lines
            .push(Line::from(Span::styled("└─".to_string(), border_style)));
        self.last_was_block = true;
    }

    fn render_table(&mut self, table: TableState) {
        if table.rows.is_empty() {
            return;
        }

        let column_count = table
            .rows
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
            .max(table.alignments.len());
        if column_count == 0 {
            return;
        }

        let mut widths = vec![0usize; column_count];
        for row in &table.rows {
            for (column, cell) in row.iter().enumerate() {
                widths[column] = widths[column].max(Line::from(cell.spans.clone()).width());
            }
        }
        // Cap each column so wide tables don't overflow the pane.
        const MAX_COL_WIDTH: usize = 30;
        for w in &mut widths {
            *w = (*w).min(MAX_COL_WIDTH);
        }

        self.lines
            .push(self.table_border_line(&widths, '┌', '┬', '┐'));
        for (row_index, row) in table.rows.iter().enumerate() {
            self.lines
                .push(self.table_row_line(row, &widths, &table.alignments, row_index == 0));
            if row_index == 0 && table.rows.len() > 1 {
                self.lines
                    .push(self.table_border_line(&widths, '├', '┼', '┤'));
            }
        }
        self.lines
            .push(self.table_border_line(&widths, '└', '┴', '┘'));
    }

    fn table_border_line(
        &self,
        widths: &[usize],
        left: char,
        junction: char,
        right: char,
    ) -> Line<'static> {
        let mut border = String::new();
        border.push(left);
        for (index, width) in widths.iter().enumerate() {
            if index > 0 {
                border.push(junction);
            }
            border.push_str(&"─".repeat(width + 2));
        }
        border.push(right);
        Line::from(Span::styled(
            border,
            Style::default().fg(self.theme.text_muted),
        ))
    }

    fn table_row_line(
        &self,
        row: &[TableCellState],
        widths: &[usize],
        alignments: &[MarkdownAlignment],
        is_header: bool,
    ) -> Line<'static> {
        let border_style = Style::default().fg(self.theme.border);
        let mut spans = vec![Span::styled("│ ", border_style)];

        for (i, width) in widths.iter().enumerate() {
            let cell = row.get(i);
            let alignment = alignments.get(i).unwrap_or(&MarkdownAlignment::None);

            let cell_text: String = cell
                .map(|c| c.spans.iter().map(|s| s.content.as_ref()).collect())
                .unwrap_or_default();
            let display_text = if cell_text.chars().count() > *width {
                let truncated: String = cell_text.chars().take(*width - 1).collect();
                format!("{}…", truncated)
            } else {
                cell_text
            };

            let text_len = display_text.chars().count();
            let pad_total = width.saturating_sub(text_len);
            let (pad_left, pad_right) = match alignment {
                MarkdownAlignment::Center => (pad_total / 2, pad_total - pad_total / 2),
                MarkdownAlignment::Right => (pad_total, 0),
                _ => (0, pad_total),
            };

            if pad_left > 0 {
                spans.push(Span::raw(" ".repeat(pad_left)));
            }

            let mut style = Style::default().fg(self.theme.text_normal);
            if is_header {
                style = style.add_modifier(Modifier::BOLD);
            }
            spans.push(Span::styled(display_text, style));

            if pad_right > 0 {
                spans.push(Span::raw(" ".repeat(pad_right)));
            }

            if i < widths.len() - 1 {
                spans.push(Span::styled(" │ ", border_style));
            }
        }

        spans.push(Span::styled(" │", border_style));
        Line::from(spans)
    }

    fn push_text(&mut self, text: &str) {
        // Images: buffer alt-text only; do NOT render inline. The buffered label
        // is emitted as "[image: <alt>] (url)" in end_tag(TagEnd::Image).
        if let Some(image) = self.image.as_mut() {
            image.label.push_str(text);
            return;
        }
        // Links: buffer the label for destination-dedup in end_tag(TagEnd::Link)
        // (so "click here (click here)" is collapsed to just "click here"), AND
        // render the text inline as a visible span so the link label appears in place.
        if let Some(link) = self.links.last_mut() {
            link.label.push_str(text);
        }
        self.push_span(text.to_string(), self.inline_style());
    }

    fn push_span(&mut self, content: String, style: Style) {
        if let Some(table) = self.table.as_mut() {
            table.current_cell.push(Span::styled(content, style));
            return;
        }
        if self.current_line.is_empty() && self.blockquote_depth > 0 {
            self.current_line.push(Span::styled(
                "  ▌ ".repeat(self.blockquote_depth),
                Style::default().fg(self.theme.text_muted),
            ));
        }
        self.current_line.push(Span::styled(content, style));
    }

    fn finish_line(&mut self) {
        // Intentionally a no-op when current_line is empty. pulldown-cmark emits
        // Tag::Paragraph / TagEnd::Paragraph pairs around every block, so blank
        // lines between paragraphs would call finish_line twice. The actual blank
        // line between blocks is emitted by blank_between_blocks() instead, which
        // is called at the *start* of each new top-level block.
        if !self.current_line.is_empty() {
            self.lines
                .push(Line::from(std::mem::take(&mut self.current_line)));
        }
    }

    /// Emits a single blank separator line before a new top-level block when the
    /// previous block has just finished. Skipped when we're inside a list (items
    /// run flush) or at the very start of output.
    fn blank_between_blocks(&mut self) {
        if self.last_was_block && self.list_stack.is_empty() && !self.lines.is_empty() {
            self.lines.push(Line::from(""));
        }
        self.last_was_block = false;
    }

    fn inline_style(&self) -> Style {
        let mut style = Style::default().fg(if self.links.is_empty() {
            self.heading_level
                .map(|level| self.heading_color(level))
                .unwrap_or(self.theme.text_normal)
        } else {
            self.theme.blue
        });
        if self.heading_level.is_some() || self.bold_depth > 0 {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.heading_level == Some(1) {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        if self.emphasis_depth > 0 {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.strikethrough_depth > 0 {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        if !self.links.is_empty() {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        if self.task_checked {
            style = style
                .fg(self.theme.text_muted)
                .add_modifier(Modifier::CROSSED_OUT);
        }
        style
    }

    fn heading_color(&self, level: u8) -> Color {
        match level {
            1 => self.theme.purple,
            2 => self.theme.blue,
            3 => self.theme.green,
            _ => self.theme.yellow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_markdown_for_test(markdown: &str) -> Vec<Line<'static>> {
        render_markdown(markdown, &Theme::default(), 80u16)
    }

    #[test]
    fn test_render_markdown() {
        let md = "# Header1\n\n## Header2\n\n- Bullet `code` item\n\nNormal line with **bold** text\n\n> Quoted";
        let lines = render_markdown_for_test(md);
        let text: Vec<String> = lines.iter().map(ToString::to_string).collect();
        let theme = Theme::default();

        // Blank lines are now emitted between each top-level block.
        assert_eq!(
            text,
            vec![
                "▌ Header1",
                "──────────────────────────────────────────────────────────────────────────────",
                "",
                "▌ Header2",
                "",
                "• Bullet code item",
                "",
                "Normal line with bold text",
                "",
                "  ▌ Quoted",
            ]
        );
        // Header1 is at index 0, Header2 at 3, list item at 5, paragraph at 7, quote at 9.
        assert!(lines[0].spans.iter().any(|span| span.content == "Header1"
            && span.style.fg == Some(theme.purple)
            && span.style.add_modifier.contains(Modifier::BOLD)
            && span.style.add_modifier.contains(Modifier::UNDERLINED)));
        assert!(lines[5].spans.iter().any(|span| span.content == "code"
            && span.style.fg == Some(theme.red)
            && span.style.bg == Some(theme.inactive_bg)));
        assert!(
            lines[7]
                .spans
                .iter()
                .any(|span| span.content == "bold"
                    && span.style.add_modifier.contains(Modifier::BOLD))
        );
    }

    #[test]
    fn test_render_markdown_formats_nested_ordered_and_task_lists() {
        // Two separate lists separated by a blank line in the source — a blank
        // separator line is emitted between them.
        let md = "1. First\n2. Second\n    - Nested\n\n- [x] Done\n- [ ] Pending";
        let lines = render_markdown_for_test(md);
        let text: Vec<String> = lines.iter().map(ToString::to_string).collect();

        assert_eq!(
            text,
            vec![
                "1. First",
                "2. Second",
                "  • Nested",
                "",
                "• ☑ Done",
                "• ☐ Pending",
            ]
        );
    }

    #[test]
    fn test_render_markdown_formats_emphasis_strikethrough_and_links() {
        let md = "Use *care* with ~~legacy~~ [docs](https://example.com).";
        let lines = render_markdown_for_test(md);

        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0].to_string(),
            "Use care with legacy docs (https://example.com)."
        );
        assert!(lines[0].spans.iter().any(
            |span| span.content == "care" && span.style.add_modifier.contains(Modifier::ITALIC)
        ));
        assert!(lines[0].spans.iter().any(|span| span.content == "legacy"
            && span.style.add_modifier.contains(Modifier::CROSSED_OUT)));
        assert!(
            lines[0].spans.iter().any(|span| span.content == "docs"
                && span.style.add_modifier.contains(Modifier::UNDERLINED))
        );
    }

    #[test]
    fn test_render_markdown_formats_fenced_code_blocks() {
        let md = "```rust\nfn main() {\n    println!(\"hello\");\n}\n```";
        let lines = render_markdown_for_test(md);
        let text = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            text,
            "┌─ rust\n│ fn main() {\n│    println!(\"hello\");\n│ }\n└─"
        );
    }

    #[test]
    fn test_render_markdown_formats_aligned_tables() {
        let md = "| Name | Status |\n|:-----|-------:|\n| API | Ready |";
        let text = render_markdown_for_test(md)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            text,
            "┌──────┬────────┐\n│ Name │ Status │\n├──────┼────────┤\n│ API  │  Ready │\n└──────┴────────┘"
        );
    }

    #[test]
    fn test_render_markdown_formats_gfm_alerts() {
        let lines = render_markdown_for_test("> [!WARNING]\n> Deploy carefully.");
        let text = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(text, "  ▌ WARNING\n  ▌ Deploy carefully.");
        assert!(lines[0].spans.iter().any(|span| span.content == "WARNING"
            && span.style.fg == Some(Theme::default().yellow)
            && span.style.add_modifier.contains(Modifier::BOLD)));
    }

    #[test]
    fn test_render_markdown_shows_image_alt_text_and_destination() {
        let lines = render_markdown_for_test(
            "See ![architecture diagram](https://example.com/diagram.png).",
        );

        assert_eq!(
            lines[0].to_string(),
            "See [image: architecture diagram] (https://example.com/diagram.png)."
        );
    }
}
