use pulldown_cmark::{
    Alignment as MarkdownAlignment, BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag,
    TagEnd,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::config::Theme;

pub fn render_markdown(markdown: &str, theme: &Theme) -> Vec<Line<'static>> {
    let options = Options::ENABLE_GFM
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_TABLES;
    MarkdownRenderer::new(theme).render(Parser::new_ext(markdown, options))
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
}

impl<'a> MarkdownRenderer<'a> {
    fn new(theme: &'a Theme) -> Self {
        Self {
            theme,
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
                    self.lines.push(Line::from(Span::styled(
                        "────────────────────────".to_string(),
                        Style::default().fg(self.theme.text_muted),
                    )));
                }
                Event::Html(html) | Event::InlineHtml(html) => {
                    let style = Style::default()
                        .fg(self.theme.text_muted)
                        .add_modifier(Modifier::DIM);
                    self.push_span(html.into_string(), style);
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
                self.heading_level = Some(level as u8);
                self.push_span(
                    "▌ ".to_string(),
                    Style::default().fg(self.heading_color(level as u8)),
                );
            }
            Tag::CodeBlock(kind) => {
                self.finish_line();
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
            TagEnd::Paragraph | TagEnd::Item | TagEnd::Heading(_) => {
                self.finish_line();
                if matches!(tag, TagEnd::Heading(_)) {
                    self.heading_level = None;
                }
            }
            TagEnd::CodeBlock => {
                if let Some(code_block) = self.code_block.take() {
                    self.render_code_block(code_block);
                }
            }
            TagEnd::BlockQuote(_) => {
                self.finish_line();
                self.blockquote_depth = self.blockquote_depth.saturating_sub(1);
            }
            TagEnd::List(_) => {
                self.finish_line();
                self.list_stack.pop();
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
                        format!(" ({})", image.destination),
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
                            format!(" ({})", link.destination),
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
        if let Some(language) = code_block.language {
            opening.push(Span::styled(
                format!(" {}", language),
                Style::default()
                    .fg(self.theme.purple)
                    .bg(self.theme.inactive_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        self.lines.push(Line::from(opening));

        for code_line in code_block.content.lines() {
            self.lines.push(Line::from(vec![
                Span::styled("│ ".to_string(), border_style),
                Span::styled(
                    code_line.to_string(),
                    Style::default()
                        .fg(self.theme.text_normal)
                        .bg(self.theme.inactive_bg),
                ),
            ]));
        }

        self.lines
            .push(Line::from(Span::styled("└─".to_string(), border_style)));
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

        let mut widths = vec![0; column_count];
        for row in &table.rows {
            for (column, cell) in row.iter().enumerate() {
                widths[column] = widths[column].max(Line::from(cell.spans.clone()).width());
            }
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
        cells: &[TableCellState],
        widths: &[usize],
        alignments: &[MarkdownAlignment],
        is_header: bool,
    ) -> Line<'static> {
        let border_style = Style::default().fg(self.theme.text_muted);
        let mut spans = vec![Span::styled("│".to_string(), border_style)];

        for (column, width) in widths.iter().enumerate() {
            let cell = cells.get(column);
            let cell_width = cell
                .map(|cell| Line::from(cell.spans.clone()).width())
                .unwrap_or(0);
            let padding = width.saturating_sub(cell_width);
            let alignment = alignments
                .get(column)
                .copied()
                .unwrap_or(MarkdownAlignment::None);
            let (left_padding, right_padding) = match alignment {
                MarkdownAlignment::Right => (padding, 0),
                MarkdownAlignment::Center => (padding / 2, padding - padding / 2),
                MarkdownAlignment::None | MarkdownAlignment::Left => (0, padding),
            };

            spans.push(Span::styled(
                format!(" {}", " ".repeat(left_padding)),
                Style::default().fg(self.theme.text_normal),
            ));
            if let Some(cell) = cell {
                spans.extend(cell.spans.iter().cloned().map(|mut span| {
                    if is_header {
                        span.style = span.style.add_modifier(Modifier::BOLD);
                    }
                    span
                }));
            }
            spans.push(Span::styled(
                format!("{} ", " ".repeat(right_padding)),
                Style::default().fg(self.theme.text_normal),
            ));
            spans.push(Span::styled("│".to_string(), border_style));
        }

        Line::from(spans)
    }

    fn push_text(&mut self, text: &str) {
        if let Some(image) = self.image.as_mut() {
            image.label.push_str(text);
            return;
        }
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
        if !self.current_line.is_empty() {
            self.lines
                .push(Line::from(std::mem::take(&mut self.current_line)));
        }
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
        render_markdown(markdown, &Theme::default())
    }

    #[test]
    fn test_render_markdown() {
        let md = "# Header1\n\n## Header2\n\n- Bullet `code` item\n\nNormal line with **bold** text\n\n> Quoted";
        let lines = render_markdown_for_test(md);
        let text: Vec<String> = lines.iter().map(ToString::to_string).collect();
        let theme = Theme::default();

        assert_eq!(
            text,
            vec![
                "▌ Header1",
                "▌ Header2",
                "• Bullet code item",
                "Normal line with bold text",
                "  ▌ Quoted",
            ]
        );
        assert!(lines[0].spans.iter().any(|span| span.content == "Header1"
            && span.style.fg == Some(theme.purple)
            && span.style.add_modifier.contains(Modifier::BOLD)
            && span.style.add_modifier.contains(Modifier::UNDERLINED)));
        assert!(lines[2].spans.iter().any(|span| span.content == "code"
            && span.style.fg == Some(theme.red)
            && span.style.bg == Some(theme.inactive_bg)));
        assert!(
            lines[3]
                .spans
                .iter()
                .any(|span| span.content == "bold"
                    && span.style.add_modifier.contains(Modifier::BOLD))
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
    fn test_render_markdown_formats_nested_ordered_and_task_lists() {
        let md = "1. First\n2. Second\n    - Nested\n\n- [x] Done\n- [ ] Pending";
        let lines = render_markdown_for_test(md);
        let text: Vec<String> = lines.iter().map(ToString::to_string).collect();

        assert_eq!(
            text,
            vec![
                "1. First",
                "2. Second",
                "  • Nested",
                "• ☑ Done",
                "• ☐ Pending",
            ]
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
            "┌─ rust\n│ fn main() {\n│     println!(\"hello\");\n│ }\n└─"
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
