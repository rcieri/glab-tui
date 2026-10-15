use crate::config::Theme;
use ratatui::style::{Modifier, Style};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::LazyLock;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

/// The newline variant: its grammars match `\n` at line ends, so every line is
/// fed with its newline restored (see [`LineHighlighter::highlight`]).
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

pub type Syntax = &'static SyntaxReference;

/// What a run of source text is, independent of any colour. Colours are
/// resolved from the active theme at render time, so a theme reload recolors
/// text that was highlighted before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Plain,
    Comment,
    String,
    Constant,
    Keyword,
    Function,
    Type,
    Markup,
}

impl TokenKind {
    pub fn style(self, theme: &Theme) -> Style {
        let style = Style::default();
        match self {
            TokenKind::Plain => style.fg(theme.text_normal),
            TokenKind::Comment => style.fg(theme.text_muted).add_modifier(Modifier::ITALIC),
            TokenKind::String => style.fg(theme.green),
            TokenKind::Constant => style.fg(theme.red),
            TokenKind::Keyword => style.fg(theme.purple),
            TokenKind::Function => style.fg(theme.yellow),
            TokenKind::Type | TokenKind::Markup => style.fg(theme.blue),
        }
    }

    fn from_scope_name(name: &str) -> Self {
        let has = |needles: &[&str]| needles.iter().any(|n| name.contains(n));
        if has(&["comment"]) {
            TokenKind::Comment
        } else if has(&["string", "regexp", "character", "quote"]) {
            TokenKind::String
        } else if has(&["number", "numeric", "constant"]) {
            TokenKind::Constant
        } else if has(&["keyword", "storage", "control", "operator"]) {
            TokenKind::Keyword
        } else if has(&["function", "method", "entity.name", "decorator"]) {
            TokenKind::Function
        } else if has(&["type", "class", "struct", "enum", "trait", "interface"]) {
            TokenKind::Type
        } else if has(&["variable", "property"]) {
            TokenKind::Plain
        } else if has(&["tag", "markup"]) {
            TokenKind::Markup
        } else {
            TokenKind::Plain
        }
    }
}

/// A run of one [`TokenKind`], as a byte range into the highlighted line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxToken {
    pub kind: TokenKind,
    pub range: Range<usize>,
}

/// Resolves `tokens` against `theme`, yielding each run of `code` with its
/// style. A range that does not fall on `code`'s char boundaries is skipped
/// rather than sliced.
pub fn styled_tokens<'a>(
    tokens: &'a [SyntaxToken],
    code: &'a str,
    theme: &'a Theme,
) -> impl Iterator<Item = (Style, &'a str)> + 'a {
    tokens.iter().filter_map(move |token| {
        code.get(token.range.clone())
            .map(|text| (token.kind.style(theme), text))
    })
}

/// Picks the grammar for a file: by its whole name first (`Makefile`,
/// `Gemfile`), then by its extension. A file neither names is left to
/// [`syntax_for_first_line`].
pub fn syntax_for_path(path: &str) -> Option<Syntax> {
    let file_name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if file_name.is_empty() {
        return None;
    }
    SYNTAX_SET.find_syntax_by_extension(file_name).or_else(|| {
        let (stem, extension) = file_name.rsplit_once('.')?;
        if stem.is_empty() || extension.is_empty() {
            return None;
        }
        SYNTAX_SET.find_syntax_by_extension(extension)
    })
}

/// Picks the grammar from a file's first line: a shebang or an editor mode line.
pub fn syntax_for_first_line(line: &str) -> Option<Syntax> {
    SYNTAX_SET.find_syntax_by_first_line(line)
}

/// Picks the grammar for a fenced code block's language token, by extension or
/// by name.
pub fn syntax_for_token(token: &str) -> Option<Syntax> {
    SYNTAX_SET.find_syntax_by_token(token)
}

/// Classifies scopes once each. Formatting a scope to its name takes syntect's
/// global scope-repository lock and allocates, so the name is computed the
/// first time a scope is seen and the kind is looked up by the interned scope
/// afterwards.
#[derive(Default)]
pub struct ScopeKinds {
    kinds: HashMap<Scope, Option<TokenKind>>,
}

impl ScopeKinds {
    /// `None` for a `meta.*` scope: it marks the extent of a construct (a
    /// method body, a call's arguments) and says nothing about what the text
    /// in it is.
    fn kind_of(&mut self, scope: Scope) -> Option<TokenKind> {
        *self.kinds.entry(scope).or_insert_with(|| {
            let name = scope.build_string();
            (!name.starts_with("meta.")).then(|| TokenKind::from_scope_name(&name))
        })
    }
}

/// Highlights consecutive lines of one source text, carrying the parser state
/// from each line into the next so constructs spanning lines (block comments,
/// docstrings, heredocs) are classified correctly.
pub struct LineHighlighter {
    parse_state: ParseState,
    scope_stack: ScopeStack,
    line: String,
}

impl LineHighlighter {
    pub fn new(syntax: Syntax) -> Self {
        Self {
            parse_state: ParseState::new(syntax),
            scope_stack: ScopeStack::new(),
            line: String::new(),
        }
    }

    /// Highlights the next line of the text. `code` carries no line ending.
    /// `None` when the grammar fails to parse the line.
    pub fn highlight(
        &mut self,
        code: &str,
        scope_kinds: &mut ScopeKinds,
    ) -> Option<Vec<SyntaxToken>> {
        self.line.clear();
        self.line.push_str(code);
        self.line.push('\n');
        let ops = self.parse_state.parse_line(&self.line, &SYNTAX_SET).ok()?;

        let mut tokens: Vec<SyntaxToken> = Vec::new();
        let mut start = 0;
        for (end, op) in ops {
            let end = end.min(code.len());
            push_token(&mut tokens, self.top_kind(scope_kinds), start..end);
            start = start.max(end);
            // Fails only on a restore with nothing cleared, which changes nothing.
            let _ = self.scope_stack.apply(&op);
        }
        push_token(&mut tokens, self.top_kind(scope_kinds), start..code.len());
        Some(tokens)
    }

    fn top_kind(&self, scope_kinds: &mut ScopeKinds) -> TokenKind {
        self.scope_stack
            .as_slice()
            .iter()
            .rev()
            .find_map(|&scope| scope_kinds.kind_of(scope))
            .unwrap_or(TokenKind::Plain)
    }
}

fn push_token(tokens: &mut Vec<SyntaxToken>, kind: TokenKind, range: Range<usize>) {
    if range.is_empty() {
        return;
    }
    match tokens.last_mut() {
        Some(last) if last.kind == kind && last.range.end == range.start => {
            last.range.end = range.end;
        }
        _ => tokens.push(SyntaxToken { kind, range }),
    }
}

/// Highlights every line of `code` as one text.
pub fn highlight_code(syntax: Syntax, code: &str) -> Vec<Option<Vec<SyntaxToken>>> {
    let mut scope_kinds = ScopeKinds::default();
    let mut highlighter = LineHighlighter::new(syntax);
    code.lines()
        .map(|line| highlighter.highlight(line, &mut scope_kinds))
        .collect()
}

/// The side of a diff a content row belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffSide {
    Old,
    New,
    Both,
}

#[derive(Default)]
enum FileSyntax {
    Resolved(Option<Syntax>),
    #[default]
    AwaitingFirstLine,
}

/// Highlights the content rows of a unified diff.
///
/// Each side of each file keeps its own parser state, fed only the rows that
/// exist on that side: context rows advance both, a removed line only the old
/// one, an added line only the new one. The states restart at every hunk,
/// since the lines between two hunks are not in the diff and whatever they
/// open or close cannot be known.
#[derive(Default)]
pub struct DiffHighlighter {
    scope_kinds: ScopeKinds,
    file_path: String,
    syntax: FileSyntax,
    old_side: Option<LineHighlighter>,
    new_side: Option<LineHighlighter>,
}

impl DiffHighlighter {
    pub fn start_hunk(&mut self) {
        self.old_side = None;
        self.new_side = None;
    }

    /// Highlights one content row of `file_path`. `code` is the row without
    /// its `+`/`-`/space marker; `line_number` is its number on `side` (the
    /// new side for context rows), used to recognise the file's first line.
    pub fn highlight_row(
        &mut self,
        file_path: &str,
        side: DiffSide,
        code: &str,
        line_number: Option<u32>,
    ) -> Option<Vec<SyntaxToken>> {
        if self.file_path != file_path {
            self.start_file(file_path);
        }
        let syntax = self.resolve_syntax(code, line_number)?;
        let scope_kinds = &mut self.scope_kinds;
        let mut highlight = |slot: &mut Option<LineHighlighter>| {
            slot.get_or_insert_with(|| LineHighlighter::new(syntax))
                .highlight(code, scope_kinds)
        };
        match side {
            DiffSide::Old => highlight(&mut self.old_side),
            DiffSide::New => highlight(&mut self.new_side),
            DiffSide::Both => {
                highlight(&mut self.old_side);
                highlight(&mut self.new_side)
            }
        }
    }

    fn start_file(&mut self, file_path: &str) {
        file_path.clone_into(&mut self.file_path);
        self.syntax = match syntax_for_path(file_path) {
            Some(syntax) => FileSyntax::Resolved(Some(syntax)),
            None => FileSyntax::AwaitingFirstLine,
        };
        self.start_hunk();
    }

    /// The file's grammar. One without a recognised name is decided by its
    /// first row: the file's first line when the diff starts there, otherwise
    /// no grammar at all.
    fn resolve_syntax(&mut self, code: &str, line_number: Option<u32>) -> Option<Syntax> {
        match self.syntax {
            FileSyntax::Resolved(syntax) => syntax,
            FileSyntax::AwaitingFirstLine => {
                let syntax = (line_number == Some(1))
                    .then(|| syntax_for_first_line(code))
                    .flatten();
                self.syntax = FileSyntax::Resolved(syntax);
                syntax
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds_of(tokens: &[SyntaxToken], code: &str) -> Vec<(TokenKind, String)> {
        tokens
            .iter()
            .map(|t| (t.kind, code[t.range.clone()].to_string()))
            .collect()
    }

    fn syntax(name: &str) -> Syntax {
        SYNTAX_SET
            .find_syntax_by_name(name)
            .unwrap_or_else(|| panic!("no {name} grammar"))
    }

    #[test]
    fn block_comment_lines_after_the_opening_one_stay_comments() {
        let code = "/* opening\n   still inside\n*/ let x = 1;";
        let lines = highlight_code(syntax("Rust"), code);
        let inside = lines[1].as_ref().expect("second line highlighted");
        assert_eq!(
            kinds_of(inside, "   still inside"),
            [(TokenKind::Comment, "   still inside".to_string())]
        );
        let closing = lines[2].as_ref().expect("third line highlighted");
        assert_eq!(closing[0].kind, TokenKind::Comment);
        assert!(
            closing
                .iter()
                .any(|t| t.kind == TokenKind::Keyword && &"*/ let x = 1;"[t.range.clone()] == "let"),
            "code after the comment closes is highlighted as code: {closing:?}"
        );
    }

    #[test]
    fn an_identifier_inside_a_java_method_body_stays_plain() {
        let code = "class A {\n  void m() {\n    foo = bar;\n  }\n}";
        let lines = highlight_code(syntax("Java"), code);
        let body = lines[2].as_ref().expect("method body line highlighted");
        assert_eq!(
            kinds_of(body, "    foo = bar;"),
            [
                (TokenKind::Plain, "    foo ".to_string()),
                (TokenKind::Keyword, "=".to_string()),
                (TokenKind::Plain, " bar;".to_string()),
            ]
        );
    }

    #[test]
    fn extensionless_build_files_are_recognised_by_name() {
        assert_eq!(
            syntax_for_path("Makefile").map(|s| s.name.as_str()),
            Some("Makefile")
        );
        assert_eq!(
            syntax_for_path("src/GNUmakefile").map(|s| s.name.as_str()),
            Some("Makefile")
        );
    }

    #[test]
    fn a_dotfile_is_not_taken_for_an_extension() {
        assert_eq!(syntax_for_path("conf/.rs").map(|s| s.name.as_str()), None);
    }

    #[test]
    fn a_shebang_selects_the_grammar_of_an_extensionless_script() {
        assert_eq!(
            syntax_for_first_line("#!/usr/bin/env python3").map(|s| s.name.as_str()),
            Some("Python")
        );
    }

    #[test]
    fn diff_sides_keep_separate_state() {
        let mut highlighter = DiffHighlighter::default();
        highlighter.start_hunk();
        let removed = highlighter
            .highlight_row("a.rs", DiffSide::Old, "/* removed comment start", Some(1))
            .expect("removed line highlighted");
        assert_eq!(removed[0].kind, TokenKind::Comment);
        let added = highlighter
            .highlight_row("a.rs", DiffSide::New, "let x = 1;", Some(1))
            .expect("added line highlighted");
        assert_eq!(added[0].kind, TokenKind::Keyword);
        let context = highlighter
            .highlight_row("a.rs", DiffSide::Both, "fn main() {}", Some(2))
            .expect("context highlighted");
        assert_eq!(context[0].kind, TokenKind::Keyword);
    }

    #[test]
    fn an_extensionless_file_whose_diff_starts_mid_file_is_not_highlighted() {
        let mut highlighter = DiffHighlighter::default();
        assert_eq!(
            highlighter.highlight_row("bin/tool", DiffSide::New, "#!/bin/sh", Some(7)),
            None
        );
    }

    #[test]
    fn an_extensionless_script_is_highlighted_from_its_shebang() {
        let mut highlighter = DiffHighlighter::default();
        let shebang =
            highlighter.highlight_row("bin/tool", DiffSide::New, "#!/usr/bin/env python3", Some(1));
        assert!(shebang.is_some());
        let body = highlighter
            .highlight_row("bin/tool", DiffSide::New, "def main(): pass", Some(2))
            .expect("body highlighted");
        assert_eq!(body[0].kind, TokenKind::Keyword);
    }
}
