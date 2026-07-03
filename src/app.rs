#![allow(dead_code)]

use crate::config::Config;
use crate::utils::ui::StatefulTable;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::style::Modifier;
use ratatui::widgets::{ListState, TableState};
use std::sync::LazyLock;
use syntect::highlighting::Style as SyntectStyle;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

fn file_extension(file_path: &str) -> Option<&str> {
    let file_name = file_path.rsplit(|c| c == '/' || c == '\\').next()?;
    let ext = file_name.rsplit('.').next()?;
    if ext.is_empty() || ext == file_name {
        None
    } else {
        Some(ext)
    }
}

/// Highlight a single line's content using syntect, returning colored spans.
pub fn highlight_line_syntax(
    file_path: &str,
    line_content: &str,
    ext: Option<&str>,
) -> Option<Vec<(ratatui::style::Style, String)>> {
    let ext = ext.or_else(|| file_extension(file_path))?;
    let syntax = SYNTAX_SET
        .find_syntax_by_extension(ext)
        .or_else(|| SYNTAX_SET.find_syntax_by_extension("txt"))?;

    let mut highlighter =
        syntect::easy::HighlightLines::new(syntax, &THEME_SET.themes["base16-eighties.dark"]);

    // Remove the leading +/-/space for syntax highlighting, but keep the actual code
    let code = if line_content.starts_with('+')
        || line_content.starts_with('-')
        || line_content.starts_with(' ')
    {
        if line_content.len() > 1 {
            &line_content[1..]
        } else {
            ""
        }
    } else {
        line_content
    };

    let ranges = highlighter.highlight_line(code, &SYNTAX_SET).ok()?;

    let result: Vec<_> = ranges
        .into_iter()
        .map(|(style, text)| (syntect_style_to_ratatui(style), text.to_string()))
        .collect();

    if result.is_empty() {
        Some(vec![(
            syntect_style_to_ratatui(SyntectStyle::default()),
            code.to_string(),
        )])
    } else {
        Some(result)
    }
}

fn syntect_style_to_ratatui(style: SyntectStyle) -> ratatui::style::Style {
    let fg = ratatui::style::Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
    let mut modifier = Modifier::empty();
    if style
        .font_style
        .contains(syntect::highlighting::FontStyle::BOLD)
    {
        modifier |= Modifier::BOLD;
    }
    if style
        .font_style
        .contains(syntect::highlighting::FontStyle::ITALIC)
    {
        modifier |= Modifier::ITALIC;
    }
    if style
        .font_style
        .contains(syntect::highlighting::FontStyle::UNDERLINE)
    {
        modifier |= Modifier::UNDERLINED;
    }
    ratatui::style::Style::default()
        .fg(fg)
        .add_modifier(modifier)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    #[default]
    Issues,
    MergeRequests,
    Pipelines,
    Jobs,
    Runners,
    Releases,
    Todos,
    Milestones,
    Terminal,
}

impl Tab {
    pub const ALL: [Tab; 9] = [
        Tab::Issues,
        Tab::MergeRequests,
        Tab::Pipelines,
        Tab::Jobs,
        Tab::Runners,
        Tab::Releases,
        Tab::Todos,
        Tab::Milestones,
        Tab::Terminal,
    ];

    pub fn title(&self, is_github: bool) -> &'static str {
        match self {
            Tab::Issues => "Issues",
            Tab::MergeRequests => {
                if is_github {
                    "PRs"
                } else {
                    "MRs"
                }
            }
            Tab::Pipelines => "Pipelines",
            Tab::Jobs => "Jobs",
            Tab::Runners => "Runners",
            Tab::Releases => "Releases",
            Tab::Todos => "Todos",
            Tab::Milestones => "Milestones",
            Tab::Terminal => "Terminal",
        }
    }

    pub fn columns(&self, is_github: bool) -> Vec<&'static str> {
        match self {
            Tab::Issues => {
                let mut cols = vec!["ID", "State", "Title", "Assignees", "Labels", "Milestone"];
                if !is_github {
                    cols.push("Due Date");
                }
                cols.push("Author");
                cols
            }
            Tab::MergeRequests => vec![
                "ID",
                "State",
                "Status",
                "Title",
                "Assignees",
                "Reviewers",
                "Labels",
                "Milestone",
                "Author",
            ],
            Tab::Pipelines => vec!["ID", "Status", "Stages", "Ref"],
            Tab::Jobs => vec!["ID", "Stage", "Status", "Name", "Matrix"],
            Tab::Runners => vec!["ID", "Description", "Status", "Active"],
            Tab::Releases => vec![
                "Tag",
                "Release Name",
                "Date",
                "Author",
                "Assets",
                "Description",
            ],
            Tab::Todos => vec!["State", "Project", "Type", "ID", "Title"],
            Tab::Milestones => vec!["ID", "State", "Title", "Progress", "Due Date"],
            Tab::Terminal => vec![],
        }
    }

    pub fn default_columns(&self, is_github: bool) -> Vec<&'static str> {
        match self {
            Tab::Issues => {
                let mut cols = vec!["ID", "State", "Title", "Labels"];
                if !is_github {
                    cols.push("Due Date");
                }
                cols
            }
            Tab::MergeRequests => vec!["ID", "State", "Status", "Title", "Labels"],
            Tab::Pipelines => vec!["ID", "Status", "Stages", "Ref"],
            Tab::Jobs => vec!["ID", "Stage", "Status", "Name", "Matrix"],
            Tab::Runners => vec!["ID", "Description", "Status", "Active"],
            Tab::Releases => vec!["Tag", "Release Name", "Date"],
            Tab::Todos => vec!["State", "Project", "Type", "ID", "Title"],
            Tab::Milestones => vec!["ID", "State", "Title", "Progress", "Due Date"],
            Tab::Terminal => vec![],
        }
    }
}

#[derive(Clone, Debug)]
pub struct EditMenu {
    pub title: String,
    pub fields: Vec<(String, String)>, // (Label, Value)
    pub selected_idx: usize,
    pub entity_iid: u64,
    pub entity_type: String, // "issue", "mr"
    pub state: ListState,
}

impl EditMenu {
    pub fn is_new(&self) -> bool {
        self.entity_type.starts_with("new_")
    }
}

#[derive(Clone, Debug)]
pub struct Selector {
    pub title: String,
    pub all_items: Vec<String>,
    pub selected_items: std::collections::HashSet<String>,
    pub cursor_idx: usize,
    pub search_query: String,
    pub is_filtering: bool,
    pub is_loading: bool,
    pub entity_iid: u64,
    pub entity_type: String, // "issue", "mr"
    pub field_type: String,  // "labels", "assignees", "reviewers", "milestone"
    pub multi_select: bool,
    pub state: ListState,
}

impl Selector {
    pub fn get_filtered_items_with_indices(&self) -> Vec<(String, Option<Vec<usize>>)> {
        let query = self.search_query.trim();
        let mut items: Vec<(String, Option<Vec<usize>>)> = if query.is_empty() {
            self.all_items
                .iter()
                .map(|item| (item.clone(), None))
                .collect()
        } else {
            let q = query.to_lowercase();
            self.all_items
                .iter()
                .filter(|item| item.to_lowercase().contains(&q))
                .map(|item| (item.clone(), None))
                .collect()
        };

        if !query.is_empty() {
            let exact_match = self
                .all_items
                .iter()
                .any(|item| item.to_lowercase() == query.to_lowercase());
            if !exact_match {
                items.push((format!("+ Create \"{}\"", query), None));
            }
        }
        items
    }

    pub fn get_filtered_items(&self) -> Vec<String> {
        self.get_filtered_items_with_indices()
            .into_iter()
            .map(|(item, _)| item)
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineType {
    Normal,
    Addition,
    Deletion,
    Meta,
    HunkHeader,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub content: String,
    pub line_type: DiffLineType,
    pub file_path: String,
    pub old_line_num: Option<u32>,
    pub new_line_num: Option<u32>,
    pub syntax_highlighted: Option<Vec<(ratatui::style::Style, String)>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffTreeNode {
    Directory {
        name: String,
        is_expanded: bool,
        children: Vec<DiffTreeNode>,
    },
    File {
        name: String,
        file_path: String,
        line_idx: usize,
    },
}

impl DiffTreeNode {
    pub fn insert(&mut self, path_parts: &[&str], full_path: &str, line_idx: usize) {
        if path_parts.is_empty() {
            return;
        }
        let name = path_parts[0].to_string();
        if path_parts.len() == 1 {
            match self {
                DiffTreeNode::Directory { children, .. } => {
                    let file_exists = children.iter().any(|child| match child {
                        DiffTreeNode::File { file_path: p, .. } => p == full_path,
                        _ => false,
                    });
                    if !file_exists {
                        children.push(DiffTreeNode::File {
                            name,
                            file_path: full_path.to_string(),
                            line_idx,
                        });
                    }
                }
                _ => {}
            }
        } else {
            match self {
                DiffTreeNode::Directory { children, .. } => {
                    if let Some(pos) = children.iter().position(|child| match child {
                        DiffTreeNode::Directory { name: n, .. } => n == &name,
                        _ => false,
                    }) {
                        children[pos].insert(&path_parts[1..], full_path, line_idx);
                    } else {
                        let mut new_dir = DiffTreeNode::Directory {
                            name,
                            is_expanded: true,
                            children: Vec::new(),
                        };
                        new_dir.insert(&path_parts[1..], full_path, line_idx);
                        children.push(new_dir);
                    }
                }
                _ => {}
            }
        }
    }

    pub fn flatten(&self, depth: usize, prefix: &str, out: &mut Vec<FlatDiffTreeNode>) {
        match self {
            DiffTreeNode::Directory {
                name,
                is_expanded,
                children,
            } => {
                let path_id = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", prefix, name)
                };
                if name != "root" {
                    out.push(FlatDiffTreeNode {
                        name: name.clone(),
                        depth,
                        is_dir: true,
                        is_expanded: *is_expanded,
                        file_path: None,
                        line_idx: None,
                        path_id: path_id.clone(),
                    });
                }
                if name == "root" || *is_expanded {
                    let mut sorted_children = children.clone();
                    sorted_children.sort_by(|a, b| {
                        let a_is_dir = match a {
                            DiffTreeNode::Directory { .. } => true,
                            _ => false,
                        };
                        let b_is_dir = match b {
                            DiffTreeNode::Directory { .. } => true,
                            _ => false,
                        };
                        b_is_dir.cmp(&a_is_dir).then_with(|| a.name().cmp(b.name()))
                    });
                    for child in sorted_children {
                        child.flatten(if name == "root" { 0 } else { depth + 1 }, &path_id, out);
                    }
                }
            }
            DiffTreeNode::File {
                name,
                file_path,
                line_idx,
            } => {
                let path_id = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", prefix, name)
                };
                out.push(FlatDiffTreeNode {
                    name: name.clone(),
                    depth,
                    is_dir: false,
                    is_expanded: false,
                    file_path: Some(file_path.clone()),
                    line_idx: Some(*line_idx),
                    path_id,
                });
            }
        }
    }

    pub fn name(&self) -> &str {
        match self {
            DiffTreeNode::Directory { name, .. } => name,
            DiffTreeNode::File { name, .. } => name,
        }
    }

    pub fn toggle_expanded(&mut self, target_path_id: &str, current_prefix: &str) -> bool {
        match self {
            DiffTreeNode::Directory {
                name,
                is_expanded,
                children,
            } => {
                let path_id = if current_prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", current_prefix, name)
                };
                if path_id == target_path_id {
                    *is_expanded = !*is_expanded;
                    return true;
                }
                for child in children {
                    if child.toggle_expanded(target_path_id, &path_id) {
                        return true;
                    }
                }
            }
            _ => {}
        }
        false
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlatDiffTreeNode {
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub is_expanded: bool,
    pub file_path: Option<String>,
    pub line_idx: Option<usize>,
    pub path_id: String,
}

#[derive(Clone, Debug)]
pub struct SideBySideLine {
    pub left: Option<DiffLine>,
    pub right: Option<DiffLine>,
    pub line_type: DiffLineType,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct DiffView {
    pub mr_iid: u64,
    pub raw_diff: String,
    pub all_lines: Vec<DiffLine>,
    pub lines: Vec<DiffLine>,
    pub cursor_idx: usize,
    pub hunks: Vec<usize>,
    pub scroll_offset: usize,
    pub root_node: DiffTreeNode,
    pub visible_nodes: Vec<FlatDiffTreeNode>,
    pub selected_visible_idx: usize,
    pub focus_on_files: bool,
    pub selection_start: Option<usize>,
    pub selection_end: Option<usize>,
    pub side_by_side: bool,
    pub side_by_side_lines: Vec<SideBySideLine>,
    pub viewport_height: usize,
}

fn strip_ansi_escapes(input: &str) -> String {
    let mut result = String::new();
    let mut in_escape = false;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            in_escape = true;
            if let Some(&'[') = chars.peek() {
                chars.next();
            }
            continue;
        }
        if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        result.push(c);
    }
    result
}

impl DiffView {
    pub fn new(mr_iid: u64, raw_diff: String) -> Self {
        let cleaned_diff = strip_ansi_escapes(&raw_diff);
        let mut all_lines = Vec::new();
        let mut current_file = String::new();
        let mut old_line_num = None;
        let mut new_line_num = None;
        let mut files = Vec::new();

        for line in cleaned_diff.lines() {
            let mut detected_file = None;
            if line.starts_with("diff --git") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    detected_file =
                        Some(parts[3].strip_prefix("b/").unwrap_or(parts[3]).to_string());
                }
            } else if line.starts_with("--- ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let path = parts[1];
                    if path != "/dev/null" && !path.is_empty() {
                        let cleaned_path = path.strip_prefix("a/").unwrap_or(path).to_string();
                        detected_file = Some(cleaned_path);
                    }
                }
            } else if line.starts_with("+++ ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let path = parts[1];
                    if path != "/dev/null" && !path.is_empty() {
                        let cleaned_path = path.strip_prefix("b/").unwrap_or(path).to_string();
                        detected_file = Some(cleaned_path);
                    }
                }
            }

            if let Some(file_path) = detected_file {
                current_file = file_path;
                let already_exists = files.iter().any(|(f, _)| f == &current_file);
                if !already_exists {
                    files.push((current_file.clone(), all_lines.len()));
                }
            }

            if line.starts_with("diff --git") {
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::Meta,
                    file_path: current_file.clone(),
                    old_line_num: None,
                    new_line_num: None,
                    syntax_highlighted: None,
                });
                old_line_num = None;
                new_line_num = None;
            } else if line.starts_with("--- ")
                || line.starts_with("+++ ")
                || line.starts_with("index ")
            {
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::Meta,
                    file_path: current_file.clone(),
                    old_line_num: None,
                    new_line_num: None,
                    syntax_highlighted: None,
                });
            } else if line.starts_with("@@ ") {
                if let Some(caps) = parse_hunk_header(line) {
                    old_line_num = Some(caps.0);
                    new_line_num = Some(caps.1);
                } else {
                    old_line_num = None;
                    new_line_num = None;
                }
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::HunkHeader,
                    file_path: current_file.clone(),
                    old_line_num: None,
                    new_line_num: None,
                    syntax_highlighted: None,
                });
            } else if line.starts_with('+') {
                let highlighted = highlight_line_syntax(&current_file, line, None);
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::Addition,
                    file_path: current_file.clone(),
                    old_line_num: None,
                    new_line_num: new_line_num,
                    syntax_highlighted: highlighted,
                });
                if let Some(ref mut n) = new_line_num {
                    *n += 1;
                }
            } else if line.starts_with('-') {
                let highlighted = highlight_line_syntax(&current_file, line, None);
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::Deletion,
                    file_path: current_file.clone(),
                    old_line_num: old_line_num,
                    new_line_num: None,
                    syntax_highlighted: highlighted,
                });
                if let Some(ref mut n) = old_line_num {
                    *n += 1;
                }
            } else {
                let highlighted = highlight_line_syntax(&current_file, line, None);
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::Normal,
                    file_path: current_file.clone(),
                    old_line_num: old_line_num,
                    new_line_num: new_line_num,
                    syntax_highlighted: highlighted,
                });
                if let Some(ref mut o) = old_line_num {
                    *o += 1;
                }
                if let Some(ref mut n) = new_line_num {
                    *n += 1;
                }
            }
        }

        let mut root_node = DiffTreeNode::Directory {
            name: "root".to_string(),
            is_expanded: true,
            children: Vec::new(),
        };

        for (file_path, line_idx) in &files {
            let parts: Vec<&str> = file_path.split(|c| c == '/' || c == '\\').collect();
            root_node.insert(&parts, file_path, *line_idx);
        }

        let mut visible_nodes = Vec::new();
        root_node.flatten(0, "", &mut visible_nodes);

        let mut view = Self {
            mr_iid,
            raw_diff,
            all_lines,
            lines: Vec::new(),
            cursor_idx: 0,
            hunks: Vec::new(),
            scroll_offset: 0,
            root_node,
            visible_nodes,
            selected_visible_idx: 0,
            focus_on_files: true,
            selection_start: None,
            selection_end: None,
            side_by_side: false,
            side_by_side_lines: Vec::new(),
            viewport_height: 15,
        };

        view.update_active_lines();
        view
    }

    pub fn update_active_lines(&mut self) {
        let new_lines = if self.visible_nodes.is_empty() {
            self.all_lines.clone()
        } else {
            let selected_node = &self.visible_nodes[self.selected_visible_idx];
            let rel_path = if selected_node.path_id == "root" {
                ""
            } else {
                selected_node
                    .path_id
                    .strip_prefix("root/")
                    .unwrap_or(&selected_node.path_id)
            };

            if selected_node.is_dir {
                if rel_path.is_empty() {
                    self.all_lines.clone()
                } else {
                    let prefix1 = format!("{}/", rel_path);
                    let prefix2 = format!("{}\\", rel_path);
                    self.all_lines
                        .iter()
                        .filter(|line| {
                            line.file_path.starts_with(&prefix1)
                                || line.file_path.starts_with(&prefix2)
                                || &line.file_path == rel_path
                        })
                        .cloned()
                        .collect()
                }
            } else {
                if !rel_path.is_empty() {
                    self.all_lines
                        .iter()
                        .filter(|line| &line.file_path == rel_path)
                        .cloned()
                        .collect()
                } else {
                    self.all_lines.clone()
                }
            }
        };

        self.lines = new_lines;
        self.side_by_side_lines = build_side_by_side_lines(&self.lines);

        let active_len = if self.side_by_side {
            self.side_by_side_lines.len()
        } else {
            self.lines.len()
        };

        if self.cursor_idx >= active_len {
            self.cursor_idx = active_len.saturating_sub(1);
        }

        self.hunks = if self.side_by_side {
            self.side_by_side_lines
                .iter()
                .enumerate()
                .filter(|(_, l)| l.line_type == DiffLineType::HunkHeader)
                .map(|(i, _)| i)
                .collect()
        } else {
            self.lines
                .iter()
                .enumerate()
                .filter(|(_, l)| l.line_type == DiffLineType::HunkHeader)
                .map(|(i, _)| i)
                .collect()
        };
    }

    pub fn update_selected_file_from_cursor(&mut self) {
        if self.visible_nodes.is_empty() {
            return;
        }
        let line_opt = if self.side_by_side {
            self.side_by_side_lines
                .get(self.cursor_idx)
                .and_then(|sline| sline.right.as_ref().or(sline.left.as_ref()).cloned())
        } else {
            self.lines.get(self.cursor_idx).cloned()
        };
        if let Some(line) = line_opt {
            let active_path = &line.file_path;
            if let Some(pos) = self.visible_nodes.iter().position(|node| {
                !node.is_dir
                    && node
                        .file_path
                        .as_ref()
                        .map(|p| p == active_path)
                        .unwrap_or(false)
            }) {
                self.selected_visible_idx = pos;
            }
        }
    }

    pub fn get_comment_range(&self) -> Option<CommentRange> {
        let selection = self.selection_start.zip(self.selection_end);

        if let Some((s, e)) = selection {
            if s != e {
                let min_idx = s.min(e);
                let max_idx = s.max(e);
                if self.side_by_side {
                    if min_idx >= self.side_by_side_lines.len()
                        || max_idx >= self.side_by_side_lines.len()
                    {
                        return None;
                    }
                    let has_any_right =
                        self.side_by_side_lines[min_idx..=max_idx]
                            .iter()
                            .any(|sline| {
                                sline
                                    .right
                                    .as_ref()
                                    .map_or(false, |r| r.new_line_num.is_some())
                            });

                    if has_any_right {
                        // Gather only right side lines (new file)
                        let right_lines: Vec<DiffLine> = self.side_by_side_lines[min_idx..=max_idx]
                            .iter()
                            .filter_map(|sline| sline.right.clone())
                            .collect();

                        let start_line = right_lines.first()?;
                        let end_line = right_lines.last()?;

                        return Some(CommentRange {
                            file_path: start_line.file_path.clone(),
                            line_num: start_line.new_line_num,
                            old_line_num: None,
                            end_line_num: end_line.new_line_num,
                            end_old_line_num: None,
                            lines: right_lines,
                        });
                    } else {
                        // Gather only left side lines (old file / deletions)
                        let left_lines: Vec<DiffLine> = self.side_by_side_lines[min_idx..=max_idx]
                            .iter()
                            .filter_map(|sline| sline.left.clone())
                            .collect();

                        let start_line = left_lines.first()?;
                        let end_line = left_lines.last()?;

                        return Some(CommentRange {
                            file_path: start_line.file_path.clone(),
                            line_num: None,
                            old_line_num: start_line.old_line_num,
                            end_line_num: None,
                            end_old_line_num: end_line.old_line_num,
                            lines: left_lines,
                        });
                    }
                } else {
                    // Unified view range
                    if min_idx >= self.lines.len() || max_idx >= self.lines.len() {
                        return None;
                    }
                    let lines = &self.lines[min_idx..=max_idx];
                    let has_any_addition_or_normal = lines.iter().any(|line| {
                        line.line_type != DiffLineType::Deletion && line.new_line_num.is_some()
                    });

                    if has_any_addition_or_normal {
                        let filtered_lines: Vec<DiffLine> = lines
                            .iter()
                            .filter(|line| {
                                line.line_type != DiffLineType::Deletion
                                    && line.new_line_num.is_some()
                            })
                            .cloned()
                            .collect();
                        let start_line = filtered_lines.first()?;
                        let end_line = filtered_lines.last()?;
                        return Some(CommentRange {
                            file_path: start_line.file_path.clone(),
                            line_num: start_line.new_line_num,
                            old_line_num: None,
                            end_line_num: end_line.new_line_num,
                            end_old_line_num: None,
                            lines: filtered_lines,
                        });
                    } else {
                        let filtered_lines: Vec<DiffLine> = lines
                            .iter()
                            .filter(|line| {
                                line.line_type == DiffLineType::Deletion
                                    && line.old_line_num.is_some()
                            })
                            .cloned()
                            .collect();
                        let start_line = filtered_lines.first()?;
                        let end_line = filtered_lines.last()?;
                        return Some(CommentRange {
                            file_path: start_line.file_path.clone(),
                            line_num: None,
                            old_line_num: start_line.old_line_num,
                            end_line_num: None,
                            end_old_line_num: end_line.old_line_num,
                            lines: filtered_lines,
                        });
                    }
                }
            }
        }

        // Single line (no selection)
        let sline_opt = if self.side_by_side {
            let sline = self.side_by_side_lines.get(self.cursor_idx)?;
            // Prefer right (new file) if it has a line number
            if let Some(ref r) = sline.right {
                if r.new_line_num.is_some() {
                    Some(r.clone())
                } else if let Some(ref l) = sline.left {
                    Some(l.clone())
                } else {
                    Some(r.clone())
                }
            } else {
                sline.left.clone()
            }
        } else {
            self.lines.get(self.cursor_idx).cloned()
        };

        let line = sline_opt?;
        if line.line_type == DiffLineType::Deletion {
            Some(CommentRange {
                file_path: line.file_path.clone(),
                line_num: None,
                old_line_num: line.old_line_num,
                end_line_num: None,
                end_old_line_num: None,
                lines: vec![line],
            })
        } else {
            Some(CommentRange {
                file_path: line.file_path.clone(),
                line_num: line.new_line_num,
                old_line_num: None,
                end_line_num: None,
                end_old_line_num: None,
                lines: vec![line],
            })
        }
    }
}

pub fn build_side_by_side_lines(lines: &[DiffLine]) -> Vec<SideBySideLine> {
    let mut side_lines = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        match line.line_type {
            DiffLineType::Meta | DiffLineType::HunkHeader => {
                side_lines.push(SideBySideLine {
                    left: Some(line.clone()),
                    right: Some(line.clone()),
                    line_type: line.line_type,
                });
                i += 1;
            }
            DiffLineType::Normal => {
                side_lines.push(SideBySideLine {
                    left: Some(line.clone()),
                    right: Some(line.clone()),
                    line_type: DiffLineType::Normal,
                });
                i += 1;
            }
            DiffLineType::Deletion | DiffLineType::Addition => {
                let mut deletions = Vec::new();
                while i < lines.len() && lines[i].line_type == DiffLineType::Deletion {
                    deletions.push(lines[i].clone());
                    i += 1;
                }
                let mut additions = Vec::new();
                while i < lines.len() && lines[i].line_type == DiffLineType::Addition {
                    additions.push(lines[i].clone());
                    i += 1;
                }

                let max_len = std::cmp::max(deletions.len(), additions.len());
                for j in 0..max_len {
                    let left = deletions.get(j).cloned();
                    let right = additions.get(j).cloned();
                    side_lines.push(SideBySideLine {
                        left,
                        right,
                        line_type: DiffLineType::Normal,
                    });
                }
            }
        }
    }
    side_lines
}

fn parse_hunk_header(header: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = header.split_whitespace().collect();
    if parts.len() >= 3 {
        let old_part = parts[1].strip_prefix('-')?;
        let new_part = parts[2].strip_prefix('+')?;

        let old_start = old_part.split(',').next()?.parse::<u32>().ok()?;
        let new_start = new_part.split(',').next()?.parse::<u32>().ok()?;
        Some((old_start, new_start))
    } else {
        None
    }
}

#[derive(Clone, Debug)]
pub struct CommentRange {
    pub file_path: String,
    pub line_num: Option<u32>,
    pub old_line_num: Option<u32>,
    pub end_line_num: Option<u32>,
    pub end_old_line_num: Option<u32>,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug)]
pub struct DraftComment {
    pub file_path: String,
    pub line_num: Option<u32>,
    pub old_line_num: Option<u32>,
    pub end_line_num: Option<u32>,
    pub end_old_line_num: Option<u32>,
    pub body: String,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum TextInputAction {
    EditField {
        entity_iid: u64,
        entity_type: String,
        field_type: String,
    },
    /// Edit a field inside a "new entity" EditMenu (iid=0). The value is
    /// written back to `edit_menu.fields[field_idx]` on confirm.
    EditNewField {
        field_idx: usize,
    },
    CreateIssue,
    AddReviewComment {
        mr_iid: u64,
        file_path: String,
        line_num: Option<u32>,
        old_line_num: Option<u32>,
        end_line_num: Option<u32>,
        end_old_line_num: Option<u32>,
    },
    EnterPipelineId,
    CreateRelease,
    CreateMilestone,
    SubmitReviewFinal {
        mr_iid: u64,
        status: String,
    },
    ReplyToComment {
        mr_iid: u64,
        comment_id: u64,
        discussion_id: String,
    },
}

#[derive(Clone, Debug)]
pub struct TextInput {
    pub title: String,
    pub value: String,
    pub cursor_idx: usize,
    pub action: TextInputAction,
}

#[derive(Clone, Debug)]
pub struct DatePicker {
    pub title: String,
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub action: DatePickerAction,
}

#[derive(Clone, Debug)]
pub enum DatePickerAction {
    EditField {
        entity_iid: u64,
        entity_type: String,
        field_type: String,
    },
    EditNewField {
        field_idx: usize,
    },
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

impl DatePicker {
    pub fn new(title: String, initial_date_str: &str, action: DatePickerAction) -> Self {
        use chrono::Datelike;
        let parsed_date = chrono::NaiveDate::parse_from_str(initial_date_str.trim(), "%Y-%m-%d")
            .ok()
            .or_else(|| {
                let now = chrono::Local::now();
                chrono::NaiveDate::from_ymd_opt(now.year(), now.month(), now.day())
            })
            .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(2026, 7, 3).unwrap());

        Self {
            title,
            year: parsed_date.year(),
            month: parsed_date.month(),
            day: parsed_date.day(),
            action,
        }
    }

    pub fn value_string(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    pub fn move_day(&mut self, offset: i32) {
        use chrono::Datelike;
        if let Some(current_date) = chrono::NaiveDate::from_ymd_opt(self.year, self.month, self.day)
        {
            let duration = chrono::Duration::days(offset as i64);
            if let Some(new_date) = current_date.checked_add_signed(duration) {
                self.year = new_date.year();
                self.month = new_date.month();
                self.day = new_date.day();
            }
        }
    }

    pub fn move_month(&mut self, offset: i32) {
        let mut new_month = self.month as i32 + offset;
        let mut new_year = self.year;
        while new_month > 12 {
            new_month -= 12;
            new_year += 1;
        }
        while new_month < 1 {
            new_month += 12;
            new_year -= 1;
        }
        self.year = new_year;
        self.month = new_month as u32;
        let max_days = days_in_month(self.year, self.month);
        if self.day > max_days {
            self.day = max_days;
        }
    }
}

#[derive(Debug, Clone)]
pub struct TerminalCommand {
    pub timestamp: String,
    pub command: String,
    pub status: String,
}

#[derive(Debug)]
pub enum GroupItem {
    Header(String),
    Item(usize),
}

#[derive(Clone, Debug)]
pub enum ConfirmAction {
    DeleteMilestone(u64),  // milestone iid
    DeleteRelease(String), // release tag_name
}

pub struct App {
    pub config: Config,
    pub active_tab: Tab,
    pub running: bool,
    pub project_context: String,
    pub gitlab_client: Option<crate::gitlab::client::GitlabClient>,
    pub terminal_commands: Vec<TerminalCommand>,
    pub issues: StatefulTable<crate::gitlab::issues::Issue>,
    pub mrs: StatefulTable<crate::gitlab::mr::MergeRequest>,
    pub pipelines: StatefulTable<crate::gitlab::pipelines::Pipeline>,
    pub search_query: String,
    pub is_typing_search: bool,
    pub selected_pipeline_jobs: Option<Vec<crate::gitlab::pipelines::Job>>,
    pub selected_job_index: Option<usize>,
    pub active_pipeline_id: Option<u64>,
    pub job_trace: Option<String>,
    pub error_message: Option<String>,
    pub runners: StatefulTable<crate::gitlab::runners::Runner>,
    pub releases: StatefulTable<crate::gitlab::releases::Release>,
    pub pipeline_jobs: std::collections::HashMap<u64, Vec<crate::gitlab::pipelines::Job>>,
    pub fetching_pipelines: std::collections::HashSet<u64>,
    pub loading_tabs: std::collections::HashSet<Tab>,
    pub loaded_tabs: std::collections::HashSet<Tab>,
    pub edit_menu: Option<EditMenu>,
    pub selector: Option<Selector>,
    pub text_input: Option<TextInput>,
    pub date_picker: Option<DatePicker>,
    pub jobs_list_state: TableState,
    pub job_trace_scroll: u16,
    pub issues_scroll: u16,
    pub mrs_scroll: u16,
    pub selected_pipelines: std::collections::HashSet<u64>,
    pub selected_jobs: std::collections::HashSet<u64>,
    pub details_zoomed: bool,
    pub job_trace_needs_scroll_to_bottom: bool,
    pub job_trace_loading: bool,
    pub collapse_matrix_jobs: bool,
    pub show_help: bool,
    pub help_search_query: String,
    pub diff_view: Option<DiffView>,
    pub current_comments: Vec<crate::gitlab::mr::DiscussionNote>,
    pub last_fetched_mr_iid: Option<u64>,
    pub show_submit_review_prompt: Option<u64>,
    pub confirm_popup: Option<ConfirmAction>,
    pub diff_loading: bool,
    pub todos: StatefulTable<crate::gitlab::notifications::Notification>,
    pub status_message: Option<String>,
    pub refreshed_tabs: std::collections::HashSet<Tab>,
    pub tx: Option<tokio::sync::mpsc::UnboundedSender<crate::event::Event>>,
    pub enabled_columns: std::collections::HashMap<Tab, std::collections::HashSet<String>>,
    pub focus_column_checklist: bool,
    pub column_checklist_idx: usize,
    pub in_review_mode: bool,
    pub draft_comments: Vec<DraftComment>,
    pub milestones: StatefulTable<crate::gitlab::milestones::Milestone>,
    pub selected_milestone_issues: Option<Vec<crate::gitlab::issues::Issue>>,
    pub selected_milestone_iid: Option<u64>,
    pub terminal_scroll: usize,
    pub group_by_column: Option<String>,
    pub group_ascending: bool,
    pub group_list_state: ratatui::widgets::ListState,
    pub group_items: Vec<GroupItem>,
    pub column_filters: std::collections::HashMap<
        Tab,
        std::collections::HashMap<String, std::collections::HashSet<String>>,
    >,
    pub column_filter_context: Option<(Tab, String)>,
}

impl Default for App {
    fn default() -> Self {
        let config = Config::load();
        Self {
            config,
            active_tab: Tab::default(),
            running: true,
            project_context: "group/repository".to_string(),
            gitlab_client: None,
            terminal_commands: vec![],
            issues: StatefulTable::with_items(vec![]),
            mrs: StatefulTable::with_items(vec![]),
            pipelines: StatefulTable::with_items(vec![]),
            search_query: String::new(),
            is_typing_search: false,
            selected_pipeline_jobs: None,
            selected_job_index: None,
            active_pipeline_id: None,
            job_trace: None,
            error_message: None,
            runners: StatefulTable::with_items(vec![]),
            releases: StatefulTable::with_items(vec![]),
            pipeline_jobs: std::collections::HashMap::new(),
            fetching_pipelines: std::collections::HashSet::new(),
            loading_tabs: std::collections::HashSet::new(),
            loaded_tabs: std::collections::HashSet::new(),
            edit_menu: None,
            selector: None,
            text_input: None,
            date_picker: None,
            jobs_list_state: TableState::default(),
            job_trace_scroll: 0,
            issues_scroll: 0,
            mrs_scroll: 0,
            selected_pipelines: std::collections::HashSet::new(),
            selected_jobs: std::collections::HashSet::new(),
            details_zoomed: false,
            job_trace_needs_scroll_to_bottom: false,
            job_trace_loading: false,
            collapse_matrix_jobs: false,
            show_help: false,
            help_search_query: String::new(),
            diff_view: None,
            current_comments: Vec::new(),
            last_fetched_mr_iid: None,
            show_submit_review_prompt: None,
            confirm_popup: None,
            diff_loading: false,
            todos: StatefulTable::with_items(vec![]),
            status_message: None,
            refreshed_tabs: std::collections::HashSet::new(),
            tx: None,
            enabled_columns: {
                let mut ec = std::collections::HashMap::new();
                for tab in Tab::ALL {
                    let set: std::collections::HashSet<String> = tab
                        .default_columns(false)
                        .iter()
                        .map(|s| s.to_string())
                        .collect();
                    ec.insert(tab, set);
                }
                ec
            },
            focus_column_checklist: false,
            column_checklist_idx: 0,
            in_review_mode: false,
            draft_comments: Vec::new(),
            milestones: StatefulTable::with_items(vec![]),
            selected_milestone_issues: None,
            selected_milestone_iid: None,
            terminal_scroll: 0,
            group_by_column: None,
            group_ascending: true,
            group_list_state: ratatui::widgets::ListState::default(),
            group_items: Vec::new(),
            column_filters: std::collections::HashMap::new(),
            column_filter_context: None,
        }
    }
}

impl App {
    pub fn start_loading_tab(&mut self, tab: Tab) {
        if !self.loading_tabs.contains(&tab) {
            self.loading_tabs.insert(tab);
        }
    }

    pub fn complete_loading_tab(&mut self, tab: Tab, _status: &str) {
        self.loading_tabs.remove(&tab);
        self.loaded_tabs.insert(tab);
        self.refreshed_tabs.insert(tab);
    }

    pub fn is_column_visible(&self, tab: Tab, col: &str) -> bool {
        let is_github = self
            .gitlab_client
            .as_ref()
            .map(|c| c.is_github)
            .unwrap_or(false);
        if is_github {
            if tab == Tab::Issues && col == "Due Date" {
                return false;
            }
            if tab == Tab::Milestones && col == "Start Date" {
                return false;
            }
        }
        if let Some(set) = self.enabled_columns.get(&tab) {
            set.contains(col)
        } else {
            true
        }
    }

    pub fn get_column_filter(
        &self,
        tab: Tab,
        col: &str,
    ) -> Option<&std::collections::HashSet<String>> {
        self.column_filters.get(&tab)?.get(col)
    }

    pub fn has_column_filter(&self, tab: Tab, col: &str) -> bool {
        self.get_column_filter(tab, col)
            .map_or(false, |v| !v.is_empty())
    }

    pub fn set_column_filter(
        &mut self,
        tab: Tab,
        col: &str,
        values: std::collections::HashSet<String>,
    ) {
        self.column_filters
            .entry(tab)
            .or_default()
            .insert(col.to_string(), values);
    }

    pub fn remove_column_filter(&mut self, tab: Tab, col: &str) {
        if let Some(filters) = self.column_filters.get_mut(&tab) {
            filters.remove(col);
            if filters.is_empty() {
                self.column_filters.remove(&tab);
            }
        }
    }

    pub fn new() -> Self {
        let mut app = Self::default();
        app.apply_config();
        app
    }

    pub fn apply_config(&mut self) {
        for tab in Tab::ALL {
            let pane = match tab {
                Tab::Issues => &self.config.issues,
                Tab::MergeRequests => &self.config.mrs,
                Tab::Pipelines => &self.config.pipelines,
                Tab::Jobs => &self.config.jobs,
                Tab::Runners => &self.config.runners,
                Tab::Releases => &self.config.releases,
                Tab::Todos => &self.config.todos,
                Tab::Milestones => &self.config.milestones,
                Tab::Terminal => &self.config.terminal,
            };
            if let Some(cols) = &pane.columns {
                let col_set: std::collections::HashSet<String> = cols.iter().cloned().collect();
                self.enabled_columns.insert(tab, col_set);
            }
            if let Some(col) = &pane.group_by_column {
                self.group_by_column = Some(col.clone());
            }
            self.group_ascending = pane.group_ascending;
            for (col, vals) in &pane.column_filters {
                let entry = self.column_filters.entry(tab).or_default();
                entry.insert(col.clone(), vals.iter().cloned().collect());
            }
        }
    }

    pub fn tick(&mut self) {}

    pub fn unresolved_threads_count(&self) -> usize {
        use std::collections::HashMap;
        let mut thread_resolved: HashMap<String, bool> = HashMap::new();

        for c in &self.current_comments {
            if c.system {
                continue;
            }
            if c.resolvable.unwrap_or(false) {
                if let Some(ref disc_id) = c.discussion_id {
                    let is_resolved = c.resolved.unwrap_or(false);
                    let entry = thread_resolved.entry(disc_id.clone()).or_insert(true);
                    if !is_resolved {
                        *entry = false;
                    }
                }
            }
        }

        thread_resolved
            .values()
            .filter(|&&resolved| !resolved)
            .count()
    }

    pub fn unresolved_threads_count_for_path(&self, path: &str) -> usize {
        use std::collections::HashMap;
        let mut thread_resolved: HashMap<String, bool> = HashMap::new();

        for c in &self.current_comments {
            if c.system {
                continue;
            }
            if c.resolvable.unwrap_or(false) {
                if let Some(ref pos) = c.position {
                    let matches_path = |file_path: &str| {
                        file_path == path
                            || file_path.starts_with(&format!("{}/", path))
                            || path == "root"
                            || path.is_empty()
                    };
                    let path_matches = pos.old_path.as_deref().map_or(false, matches_path)
                        || pos.new_path.as_deref().map_or(false, matches_path);
                    if path_matches {
                        if let Some(ref disc_id) = c.discussion_id {
                            let is_resolved = c.resolved.unwrap_or(false);
                            let entry = thread_resolved.entry(disc_id.clone()).or_insert(true);
                            if !is_resolved {
                                *entry = false;
                            }
                        }
                    }
                }
            }
        }

        thread_resolved
            .values()
            .filter(|&&resolved| !resolved)
            .count()
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn next_tab(&mut self) {
        let current_index = Tab::ALL
            .iter()
            .position(|t| t == &self.active_tab)
            .unwrap_or(0);
        let next_index = (current_index + 1) % Tab::ALL.len();
        self.active_tab = Tab::ALL[next_index];
        self.selected_pipelines.clear();
        self.selected_jobs.clear();
        self.details_zoomed = false;
        self.update_filter_selection();
    }

    pub fn previous_tab(&mut self) {
        let current_index = Tab::ALL
            .iter()
            .position(|t| t == &self.active_tab)
            .unwrap_or(0);
        let prev_index = if current_index == 0 {
            Tab::ALL.len() - 1
        } else {
            current_index - 1
        };
        self.active_tab = Tab::ALL[prev_index];
        self.selected_pipelines.clear();
        self.selected_jobs.clear();
        self.details_zoomed = false;
        self.update_filter_selection();
    }

    pub fn filter_issues_list<'a>(
        items: &'a [crate::gitlab::issues::Issue],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::gitlab::issues::Issue> {
        if query.trim().is_empty() {
            return items.iter().collect();
        }
        let q = query.trim().to_lowercase();
        items
            .iter()
            .filter(|item| {
                let mut matches = false;
                let mut check_match = |text: &str| {
                    if text.to_lowercase().contains(&q) {
                        matches = true;
                    }
                };
                if enabled_cols.contains("ID") {
                    check_match(&format!("#{}", item.iid));
                    check_match(&item.iid.to_string());
                }
                if enabled_cols.contains("State") {
                    if item.state == "opened" {
                        check_match("OPEN");
                    } else if item.state == "closed" {
                        check_match("CLOSED");
                    }
                }
                if enabled_cols.contains("Title") {
                    check_match(&item.title);
                }
                if enabled_cols.contains("Author") {
                    check_match(&item.author.username);
                    check_match(&format!("@{}", item.author.username));
                }
                if enabled_cols.contains("Milestone") {
                    if let Some(m) = &item.milestone {
                        check_match(&m.title);
                    }
                }
                if enabled_cols.contains("Labels") {
                    for label in &item.labels {
                        check_match(label);
                    }
                }
                if enabled_cols.contains("Assignees") {
                    for assignee in &item.assignees {
                        check_match(&assignee.username);
                        check_match(&format!("@{}", assignee.username));
                    }
                }
                matches
            })
            .collect()
    }

    pub fn filtered_issues_list<'a>(
        items: &'a [crate::gitlab::issues::Issue],
        query: &str,
        enabled_columns: &std::collections::HashMap<Tab, std::collections::HashSet<String>>,
        ascending: bool,
        group_by_column: &Option<String>,
    ) -> Vec<&'a crate::gitlab::issues::Issue> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = enabled_columns.get(&Tab::Issues).unwrap_or(&default_set);
        let mut list = Self::filter_issues_list(items, query, enabled_cols);
        if let Some(col) = group_by_column {
            list.sort_by(|a, b| {
                let val_a = match col.as_str() {
                    "State" => a.state.clone(),
                    "Author" => a.author.username.clone(),
                    "Labels" => a.labels.first().cloned().unwrap_or_default(),
                    "Milestone" => a
                        .milestone
                        .as_ref()
                        .map(|m| m.title.clone())
                        .unwrap_or_default(),
                    "Assignees" => a
                        .assignees
                        .first()
                        .map(|asg| asg.username.clone())
                        .unwrap_or_default(),
                    "ID" => a.iid.to_string(),
                    "Title" => a.title.clone(),
                    _ => String::new(),
                };
                let val_b = match col.as_str() {
                    "State" => b.state.clone(),
                    "Author" => b.author.username.clone(),
                    "Labels" => b.labels.first().cloned().unwrap_or_default(),
                    "Milestone" => b
                        .milestone
                        .as_ref()
                        .map(|m| m.title.clone())
                        .unwrap_or_default(),
                    "Assignees" => b
                        .assignees
                        .first()
                        .map(|asg| asg.username.clone())
                        .unwrap_or_default(),
                    "ID" => b.iid.to_string(),
                    "Title" => b.title.clone(),
                    _ => String::new(),
                };
                let cmp = match (val_a.parse::<u64>(), val_b.parse::<u64>()) {
                    (Ok(a), Ok(b)) => a.cmp(&b),
                    _ => val_a.cmp(&val_b),
                };
                if !ascending { cmp.reverse() } else { cmp }
            });
        }
        list
    }

    pub fn filtered_issues(&self) -> Vec<&crate::gitlab::issues::Issue> {
        let mut list = Self::filtered_issues_list(
            &self.issues.items,
            &self.search_query,
            &self.enabled_columns,
            self.group_ascending,
            &self.group_by_column,
        );
        Self::apply_column_filters(&mut list, &self.column_filters, Tab::Issues, |item, col| {
            match col {
                "Labels" => item.labels.clone(),
                "Assignees" => item.assignees.iter().map(|a| a.username.clone()).collect(),
                "Author" => vec![item.author.username.clone()],
                "Milestone" => item
                    .milestone
                    .as_ref()
                    .map(|m| m.title.clone())
                    .into_iter()
                    .collect(),
                "State" => vec![item.state.clone()],
                "ID" => vec![item.iid.to_string()],
                "Title" => vec![item.title.clone()],
                _ => vec![],
            }
        });
        list
    }

    pub fn filter_mrs_list<'a>(
        items: &'a [crate::gitlab::mr::MergeRequest],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::gitlab::mr::MergeRequest> {
        if query.trim().is_empty() {
            return items.iter().collect();
        }
        let matcher = SkimMatcherV2::default();
        let mut scored_items = Vec::new();

        for item in items {
            let mut best_score = None;

            let mut check_match = |text: &str| {
                if let Some(score) = matcher.fuzzy_match(text, query) {
                    if best_score.is_none() || Some(score) > best_score {
                        best_score = Some(score);
                    }
                }
            };

            if enabled_cols.contains("ID") {
                check_match(&format!("!{}", item.iid));
                check_match(&item.iid.to_string());
            }
            if enabled_cols.contains("State") {
                if item.state == "opened" {
                    check_match("OPEN");
                } else if item.state == "merged" {
                    check_match("MERGED");
                } else if item.state == "closed" {
                    check_match("CLOSED");
                }
            }
            if enabled_cols.contains("Status") {
                let (prefix, _) = crate::utils::format::parse_mr_title_prefix(&item.title);
                if item.draft || prefix.to_lowercase() == "wip" || prefix.to_lowercase() == "draft"
                {
                    check_match("DRAFT");
                } else {
                    check_match("READY");
                }
            }
            if enabled_cols.contains("Title") {
                check_match(&item.title);
            }
            if enabled_cols.contains("Author") {
                check_match(&item.author.username);
                check_match(&format!("@{}", item.author.username));
            }
            if enabled_cols.contains("Milestone") {
                if let Some(ms) = &item.milestone {
                    check_match(&ms.title);
                }
            }
            if enabled_cols.contains("Labels") {
                for label in &item.labels {
                    check_match(label);
                }
            }
            if enabled_cols.contains("Assignees") {
                for assignee in &item.assignees {
                    check_match(&assignee.username);
                    check_match(&format!("@{}", assignee.username));
                }
            }
            if enabled_cols.contains("Reviewers") {
                for reviewer in &item.reviewers {
                    check_match(&reviewer.username);
                    check_match(&format!("@{}", reviewer.username));
                }
            }

            if let Some(score) = best_score {
                scored_items.push((item, score));
            }
        }

        scored_items.sort_by(|a, b| b.1.cmp(&a.1));
        scored_items.into_iter().map(|(item, _)| item).collect()
    }

    pub fn filtered_mrs_list<'a>(
        items: &'a [crate::gitlab::mr::MergeRequest],
        query: &str,
        enabled_columns: &std::collections::HashMap<Tab, std::collections::HashSet<String>>,
        ascending: bool,
        group_by_column: &Option<String>,
    ) -> Vec<&'a crate::gitlab::mr::MergeRequest> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = enabled_columns
            .get(&Tab::MergeRequests)
            .unwrap_or(&default_set);
        let mut list = Self::filter_mrs_list(items, query, enabled_cols);
        if let Some(col) = group_by_column {
            list.sort_by(|a, b| {
                let val_a = match col.as_str() {
                    "State" => a.state.clone(),
                    "Author" => a.author.username.clone(),
                    "Labels" => a.labels.first().cloned().unwrap_or_default(),
                    "Milestone" => a
                        .milestone
                        .as_ref()
                        .map(|m| m.title.clone())
                        .unwrap_or_default(),
                    "Assignees" => a
                        .assignees
                        .first()
                        .map(|asg| asg.username.clone())
                        .unwrap_or_default(),
                    "Reviewers" => a
                        .reviewers
                        .first()
                        .map(|rev| rev.username.clone())
                        .unwrap_or_default(),
                    "Status" => {
                        if a.draft {
                            "Draft".to_string()
                        } else {
                            "Ready".to_string()
                        }
                    }
                    "ID" => a.iid.to_string(),
                    "Title" => a.title.clone(),
                    _ => String::new(),
                };
                let val_b = match col.as_str() {
                    "State" => b.state.clone(),
                    "Author" => b.author.username.clone(),
                    "Labels" => b.labels.first().cloned().unwrap_or_default(),
                    "Milestone" => b
                        .milestone
                        .as_ref()
                        .map(|m| m.title.clone())
                        .unwrap_or_default(),
                    "Assignees" => b
                        .assignees
                        .first()
                        .map(|asg| asg.username.clone())
                        .unwrap_or_default(),
                    "Reviewers" => b
                        .reviewers
                        .first()
                        .map(|rev| rev.username.clone())
                        .unwrap_or_default(),
                    "Status" => {
                        if b.draft {
                            "Draft".to_string()
                        } else {
                            "Ready".to_string()
                        }
                    }
                    "ID" => b.iid.to_string(),
                    "Title" => b.title.clone(),
                    _ => String::new(),
                };
                let cmp = match (val_a.parse::<u64>(), val_b.parse::<u64>()) {
                    (Ok(a), Ok(b)) => a.cmp(&b),
                    _ => val_a.cmp(&val_b),
                };
                if !ascending { cmp.reverse() } else { cmp }
            });
        }
        list
    }

    pub fn filtered_mrs(&self) -> Vec<&crate::gitlab::mr::MergeRequest> {
        let mut list = Self::filtered_mrs_list(
            &self.mrs.items,
            &self.search_query,
            &self.enabled_columns,
            self.group_ascending,
            &self.group_by_column,
        );
        Self::apply_column_filters(
            &mut list,
            &self.column_filters,
            Tab::MergeRequests,
            |item, col| match col {
                "Labels" => item.labels.clone(),
                "Assignees" => item.assignees.iter().map(|a| a.username.clone()).collect(),
                "Reviewers" => item.reviewers.iter().map(|r| r.username.clone()).collect(),
                "Author" => vec![item.author.username.clone()],
                "Milestone" => item
                    .milestone
                    .as_ref()
                    .map(|m| m.title.clone())
                    .into_iter()
                    .collect(),
                "State" => vec![item.state.clone()],
                "Status" => vec![if item.draft {
                    "Draft".to_string()
                } else {
                    "Ready".to_string()
                }],
                "ID" => vec![item.iid.to_string()],
                "Title" => vec![item.title.clone()],
                _ => vec![],
            },
        );
        list
    }

    pub fn filter_pipelines_list<'a>(
        items: &'a [crate::gitlab::pipelines::Pipeline],
        query: &str,
        pipeline_jobs: &std::collections::HashMap<u64, Vec<crate::gitlab::pipelines::Job>>,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::gitlab::pipelines::Pipeline> {
        if query.trim().is_empty() {
            return items.iter().collect();
        }
        let q = query.trim().to_lowercase();
        items
            .iter()
            .filter(|item| {
                let mut matches = false;
                let mut check_match = |text: &str| {
                    if text.to_lowercase().contains(&q) {
                        matches = true;
                    }
                };
                if enabled_cols.contains("ID") {
                    check_match(&format!("#{}", item.id));
                    check_match(&item.id.to_string());
                }
                if enabled_cols.contains("Status") {
                    check_match(&item.status);
                }
                if enabled_cols.contains("Ref") {
                    check_match(&item.r#ref);
                }
                if enabled_cols.contains("Stages") {
                    if let Some(jobs) = pipeline_jobs.get(&item.id) {
                        for job in jobs {
                            check_match(&job.name);
                            check_match(&job.stage);
                            check_match(&job.status);
                        }
                    }
                }
                matches
            })
            .collect()
    }

    pub fn filtered_pipelines_list<'a>(
        items: &'a [crate::gitlab::pipelines::Pipeline],
        query: &str,
        pipeline_jobs: &std::collections::HashMap<u64, Vec<crate::gitlab::pipelines::Job>>,
        enabled_columns: &std::collections::HashMap<Tab, std::collections::HashSet<String>>,
        ascending: bool,
        group_by_column: &Option<String>,
    ) -> Vec<&'a crate::gitlab::pipelines::Pipeline> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = enabled_columns.get(&Tab::Pipelines).unwrap_or(&default_set);
        let mut list = Self::filter_pipelines_list(items, query, pipeline_jobs, enabled_cols);
        if let Some(col) = group_by_column {
            list.sort_by(|a, b| {
                let val_a = match col.as_str() {
                    "Status" => a.status.clone(),
                    "Ref" => a.r#ref.clone(),
                    "ID" => a.id.to_string(),
                    _ => String::new(),
                };
                let val_b = match col.as_str() {
                    "Status" => b.status.clone(),
                    "Ref" => b.r#ref.clone(),
                    "ID" => b.id.to_string(),
                    _ => String::new(),
                };
                let cmp = match (val_a.parse::<u64>(), val_b.parse::<u64>()) {
                    (Ok(a), Ok(b)) => a.cmp(&b),
                    _ => val_a.cmp(&val_b),
                };
                if !ascending { cmp.reverse() } else { cmp }
            });
        }
        list
    }

    pub fn filtered_pipelines(&self) -> Vec<&crate::gitlab::pipelines::Pipeline> {
        let mut list = Self::filtered_pipelines_list(
            &self.pipelines.items,
            &self.search_query,
            &self.pipeline_jobs,
            &self.enabled_columns,
            self.group_ascending,
            &self.group_by_column,
        );
        Self::apply_column_filters(
            &mut list,
            &self.column_filters,
            Tab::Pipelines,
            |item, col| match col {
                "ID" => vec![item.id.to_string()],
                "Status" => vec![item.status.clone()],
                "Ref" => vec![item.r#ref.clone()],
                _ => vec![],
            },
        );
        list
    }

    pub fn filter_jobs_list<'a>(
        items: &'a [crate::gitlab::pipelines::Job],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::gitlab::pipelines::Job> {
        if query.trim().is_empty() {
            return items.iter().collect();
        }
        let q = query.trim().to_lowercase();
        items
            .iter()
            .filter(|item| {
                let mut matches = false;
                let mut check_match = |text: &str| {
                    if text.to_lowercase().contains(&q) {
                        matches = true;
                    }
                };
                if enabled_cols.contains("ID") {
                    check_match(&item.id.to_string());
                }
                if enabled_cols.contains("Status") {
                    check_match(&item.status);
                }
                if enabled_cols.contains("Stage") {
                    check_match(&item.stage);
                }
                if enabled_cols.contains("Name") {
                    check_match(&item.name);
                }
                if enabled_cols.contains("Matrix") {
                    if let Some(matrix) = &item.matrix {
                        check_match(matrix);
                    }
                }
                matches
            })
            .collect()
    }

    pub fn filtered_jobs_list<'a>(
        items: &'a [crate::gitlab::pipelines::Job],
        query: &str,
        enabled_columns: &std::collections::HashMap<Tab, std::collections::HashSet<String>>,
        ascending: bool,
        group_by_column: &Option<String>,
    ) -> Vec<&'a crate::gitlab::pipelines::Job> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = enabled_columns.get(&Tab::Jobs).unwrap_or(&default_set);
        let mut list = Self::filter_jobs_list(items, query, enabled_cols);
        if let Some(col) = group_by_column {
            list.sort_by(|a, b| {
                let val_a = match col.as_str() {
                    "Status" => a.status.clone(),
                    "Stage" => a.stage.clone(),
                    "Name" => a.name.clone(),
                    "ID" => a.id.to_string(),
                    _ => String::new(),
                };
                let val_b = match col.as_str() {
                    "Status" => b.status.clone(),
                    "Stage" => b.stage.clone(),
                    "Name" => b.name.clone(),
                    "ID" => b.id.to_string(),
                    _ => String::new(),
                };
                let cmp = match (val_a.parse::<u64>(), val_b.parse::<u64>()) {
                    (Ok(a), Ok(b)) => a.cmp(&b),
                    _ => val_a.cmp(&val_b),
                };
                if !ascending { cmp.reverse() } else { cmp }
            });
        }
        list
    }

    pub fn filtered_jobs(&self) -> Vec<&crate::gitlab::pipelines::Job> {
        if let Some(jobs) = &self.selected_pipeline_jobs {
            let mut list = Self::filtered_jobs_list(
                jobs,
                &self.search_query,
                &self.enabled_columns,
                self.group_ascending,
                &self.group_by_column,
            );
            Self::apply_column_filters(&mut list, &self.column_filters, Tab::Jobs, |item, col| {
                match col {
                    "ID" => vec![item.id.to_string()],
                    "Stage" => vec![item.stage.clone()],
                    "Status" => vec![item.status.clone()],
                    "Name" => vec![item.name.clone()],
                    _ => vec![],
                }
            });

            if self.collapse_matrix_jobs {
                let mut collapsed: Vec<&crate::gitlab::pipelines::Job> = Vec::new();
                let mut seen_names = std::collections::HashSet::new();
                for job in list {
                    if seen_names.insert(&job.name) {
                        collapsed.push(job);
                    }
                }
                collapsed
            } else {
                list
            }
        } else {
            vec![]
        }
    }

    pub fn filter_runners_list<'a>(
        items: &'a [crate::gitlab::runners::Runner],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::gitlab::runners::Runner> {
        if query.trim().is_empty() {
            return items.iter().collect();
        }
        let q = query.trim().to_lowercase();
        items
            .iter()
            .filter(|item| {
                let mut matches = false;
                let mut check_match = |text: &str| {
                    if text.to_lowercase().contains(&q) {
                        matches = true;
                    }
                };
                if enabled_cols.contains("ID") {
                    check_match(&item.id.to_string());
                }
                if enabled_cols.contains("Description") {
                    if let Some(desc) = &item.description {
                        check_match(desc);
                    }
                }
                if enabled_cols.contains("Status") {
                    check_match(&item.status);
                }
                if enabled_cols.contains("Active") {
                    let active_str = if item.active { "active" } else { "inactive" };
                    check_match(active_str);
                    check_match(&item.active.to_string());
                }
                matches
            })
            .collect()
    }

    pub fn filtered_runners(&self) -> Vec<&crate::gitlab::runners::Runner> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = self
            .enabled_columns
            .get(&Tab::Runners)
            .unwrap_or(&default_set);
        let mut list: Vec<&crate::gitlab::runners::Runner> =
            Self::filter_runners_list(&self.runners.items, &self.search_query, enabled_cols);
        Self::apply_column_filters(
            &mut list,
            &self.column_filters,
            Tab::Runners,
            |item, col| match col {
                "ID" => vec![item.id.to_string()],
                "Status" => vec![item.status.clone()],
                "Active" => vec![item.active.to_string()],
                _ => vec![],
            },
        );
        list
    }

    pub fn filter_releases_list<'a>(
        items: &'a [crate::gitlab::releases::Release],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::gitlab::releases::Release> {
        if query.trim().is_empty() {
            return items.iter().collect();
        }
        let q = query.trim().to_lowercase();
        items
            .iter()
            .filter(|item| {
                let mut matches = false;
                let mut check_match = |text: &str| {
                    if text.to_lowercase().contains(&q) {
                        matches = true;
                    }
                };
                if enabled_cols.contains("Tag") {
                    check_match(&item.tag_name);
                }
                if enabled_cols.contains("Release Name") {
                    check_match(&item.name);
                }
                if enabled_cols.contains("Date") {
                    check_match(&item.released_at);
                    check_match(&crate::utils::format::time_ago(&item.released_at));
                }
                if enabled_cols.contains("Description") {
                    if let Some(ref desc) = item.description {
                        check_match(desc);
                    }
                }
                if enabled_cols.contains("Author") {
                    if let Some(ref a) = item.author_name {
                        check_match(a);
                    }
                }
                matches
            })
            .collect()
    }

    pub fn filtered_releases(&self) -> Vec<&crate::gitlab::releases::Release> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = self
            .enabled_columns
            .get(&Tab::Releases)
            .unwrap_or(&default_set);
        let mut list =
            Self::filter_releases_list(&self.releases.items, &self.search_query, enabled_cols);
        Self::apply_column_filters(
            &mut list,
            &self.column_filters,
            Tab::Releases,
            |item, col| match col {
                "Tag" => vec![item.tag_name.clone()],
                "Release Name" => vec![item.name.clone()],
                "Description" => item
                    .description
                    .clone()
                    .map(|d| vec![d])
                    .unwrap_or_default(),
                "Author" => item
                    .author_name
                    .clone()
                    .map(|a| vec![a])
                    .unwrap_or_default(),
                _ => vec![],
            },
        );
        list
    }

    pub fn filter_todos_list<'a>(
        items: &'a [crate::gitlab::notifications::Notification],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::gitlab::notifications::Notification> {
        if query.trim().is_empty() {
            return items.iter().collect();
        }
        let q = query.trim().to_lowercase();
        items
            .iter()
            .filter(|item| {
                let mut matches = false;
                let mut check_match = |text: &str| {
                    if text.to_lowercase().contains(&q) {
                        matches = true;
                    }
                };
                if enabled_cols.contains("State") {
                    check_match(&item.state);
                }
                if enabled_cols.contains("Project") {
                    check_match(&item.project_path);
                }
                if enabled_cols.contains("Type") {
                    check_match(&item.target_type);
                }
                if enabled_cols.contains("ID") {
                    check_match(&item.target_iid.to_string());
                    check_match(&format!("#{}", item.target_iid));
                }
                if enabled_cols.contains("Title") {
                    check_match(&item.title);
                }
                matches
            })
            .collect()
    }

    pub fn filtered_todos_list<'a>(
        items: &'a [crate::gitlab::notifications::Notification],
        query: &str,
        enabled_columns: &std::collections::HashMap<Tab, std::collections::HashSet<String>>,
        ascending: bool,
        group_by_column: &Option<String>,
    ) -> Vec<&'a crate::gitlab::notifications::Notification> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = enabled_columns.get(&Tab::Todos).unwrap_or(&default_set);
        let mut list = Self::filter_todos_list(items, query, enabled_cols);
        if let Some(col) = group_by_column {
            list.sort_by(|a, b| {
                let val_a = match col.as_str() {
                    "State" => a.state.clone(),
                    "Type" => a.target_type.clone(),
                    "Project" => a.project_path.clone(),
                    "ID" => a.target_iid.to_string(),
                    "Title" => a.title.clone(),
                    _ => String::new(),
                };
                let val_b = match col.as_str() {
                    "State" => b.state.clone(),
                    "Type" => b.target_type.clone(),
                    "Project" => b.project_path.clone(),
                    "ID" => b.target_iid.to_string(),
                    "Title" => b.title.clone(),
                    _ => String::new(),
                };
                let cmp = match (val_a.parse::<u64>(), val_b.parse::<u64>()) {
                    (Ok(a), Ok(b)) => a.cmp(&b),
                    _ => val_a.cmp(&val_b),
                };
                if !ascending { cmp.reverse() } else { cmp }
            });
        }
        list
    }

    pub fn filtered_todos(&self) -> Vec<&crate::gitlab::notifications::Notification> {
        let mut list = Self::filtered_todos_list(
            &self.todos.items,
            &self.search_query,
            &self.enabled_columns,
            self.group_ascending,
            &self.group_by_column,
        );
        Self::apply_column_filters(&mut list, &self.column_filters, Tab::Todos, |item, col| {
            match col {
                "State" => vec![item.state.clone()],
                "Project" => vec![item.project_path.clone()],
                "Type" => vec![item.target_type.clone()],
                "ID" => vec![item.id.clone()],
                "Title" => vec![item.title.clone()],
                _ => vec![],
            }
        });
        list
    }

    pub fn filter_milestones_list<'a>(
        items: &'a [crate::gitlab::milestones::Milestone],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::gitlab::milestones::Milestone> {
        if query.trim().is_empty() {
            return items.iter().collect();
        }
        let q = query.trim().to_lowercase();
        items
            .iter()
            .filter(|item| {
                let mut matches = false;
                let mut check_match = |text: &str| {
                    if text.to_lowercase().contains(&q) {
                        matches = true;
                    }
                };
                if enabled_cols.contains("ID") {
                    check_match(&item.iid.to_string());
                    check_match(&format!("#{}", item.iid));
                }
                if enabled_cols.contains("Title") {
                    check_match(&item.title);
                }
                if enabled_cols.contains("State") {
                    check_match(&item.state);
                }
                if enabled_cols.contains("Start Date") {
                    if let Some(d) = &item.start_date {
                        check_match(d);
                    }
                }
                if enabled_cols.contains("Due Date") {
                    if let Some(d) = &item.due_date {
                        check_match(d);
                    }
                }
                matches
            })
            .collect()
    }

    pub fn filtered_milestones(&self) -> Vec<&crate::gitlab::milestones::Milestone> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = self
            .enabled_columns
            .get(&Tab::Milestones)
            .unwrap_or(&default_set);
        let mut list: Vec<&crate::gitlab::milestones::Milestone> =
            Self::filter_milestones_list(&self.milestones.items, &self.search_query, enabled_cols);
        Self::apply_column_filters(
            &mut list,
            &self.column_filters,
            Tab::Milestones,
            |item, col| match col {
                "ID" => vec![item.id.to_string()],
                "Title" => vec![item.title.clone()],
                "State" => vec![item.state.clone()],
                _ => vec![],
            },
        );
        list
    }

    pub fn apply_column_filters<'a, T>(
        list: &mut Vec<&'a T>,
        column_filters: &std::collections::HashMap<
            Tab,
            std::collections::HashMap<String, std::collections::HashSet<String>>,
        >,
        tab: Tab,
        get_values: impl Fn(&T, &str) -> Vec<String>,
    ) {
        let Some(filters) = column_filters.get(&tab) else {
            return;
        };
        for (col, selected) in filters {
            if selected.is_empty() {
                continue;
            }
            let is_text = matches!(
                col.as_str(),
                "Title" | "Name" | "Ref" | "Tag" | "Release Name"
            );
            list.retain(|item| {
                let vals = get_values(item, col);
                if is_text {
                    vals.iter().any(|v| {
                        selected
                            .iter()
                            .any(|s| v.to_lowercase().contains(&s.to_lowercase()))
                    })
                } else {
                    vals.iter().any(|v| selected.contains(v))
                }
            });
        }
    }

    pub fn collect_unique_column_values(&self, tab: Tab, col: &str) -> Vec<String> {
        use std::collections::BTreeSet;
        let mut values: BTreeSet<String> = BTreeSet::new();
        match tab {
            Tab::Issues => {
                for item in &self.issues.items {
                    match col {
                        "ID" => {
                            values.insert(item.iid.to_string());
                        }
                        "State" => {
                            values.insert(item.state.clone());
                        }
                        "Title" => {
                            values.insert(item.title.clone());
                        }
                        "Labels" => {
                            for l in &item.labels {
                                values.insert(l.clone());
                            }
                        }
                        "Assignees" => {
                            for a in &item.assignees {
                                values.insert(a.username.clone());
                            }
                        }
                        "Author" => {
                            values.insert(item.author.username.clone());
                        }
                        "Milestone" => {
                            if let Some(m) = &item.milestone {
                                values.insert(m.title.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
            Tab::MergeRequests => {
                for item in &self.mrs.items {
                    match col {
                        "ID" => {
                            values.insert(item.iid.to_string());
                        }
                        "State" => {
                            values.insert(item.state.clone());
                        }
                        "Status" => {
                            values.insert(if item.draft {
                                "Draft".to_string()
                            } else {
                                "Ready".to_string()
                            });
                        }
                        "Title" => {
                            values.insert(item.title.clone());
                        }
                        "Labels" => {
                            for l in &item.labels {
                                values.insert(l.clone());
                            }
                        }
                        "Assignees" => {
                            for a in &item.assignees {
                                values.insert(a.username.clone());
                            }
                        }
                        "Reviewers" => {
                            for r in &item.reviewers {
                                values.insert(r.username.clone());
                            }
                        }
                        "Author" => {
                            values.insert(item.author.username.clone());
                        }
                        "Milestone" => {
                            if let Some(m) = &item.milestone {
                                values.insert(m.title.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
            Tab::Pipelines => {
                for item in &self.pipelines.items {
                    match col {
                        "ID" => {
                            values.insert(item.id.to_string());
                        }
                        "Status" => {
                            values.insert(item.status.clone());
                        }
                        "Ref" => {
                            values.insert(item.r#ref.clone());
                        }
                        _ => {}
                    }
                }
            }
            Tab::Jobs => {
                if let Some(jobs) = &self.selected_pipeline_jobs {
                    for item in jobs {
                        match col {
                            "ID" => {
                                values.insert(item.id.to_string());
                            }
                            "Stage" => {
                                values.insert(item.stage.clone());
                            }
                            "Status" => {
                                values.insert(item.status.clone());
                            }
                            "Name" => {
                                values.insert(item.name.clone());
                            }
                            _ => {}
                        }
                    }
                }
            }
            Tab::Runners => {
                for item in &self.runners.items {
                    match col {
                        "ID" => {
                            values.insert(item.id.to_string());
                        }
                        "Description" => {
                            if let Some(d) = &item.description {
                                values.insert(d.clone());
                            }
                        }
                        "Status" => {
                            values.insert(item.status.clone());
                        }
                        "Active" => {
                            values.insert(item.active.to_string());
                        }
                        _ => {}
                    }
                }
            }
            Tab::Releases => {
                for item in &self.releases.items {
                    match col {
                        "Tag" => {
                            values.insert(item.tag_name.clone());
                        }
                        "Release Name" => {
                            values.insert(item.name.clone());
                        }
                        "Author" => {
                            if let Some(ref a) = item.author_name {
                                values.insert(a.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
            Tab::Todos => {
                for item in &self.todos.items {
                    match col {
                        "State" => {
                            values.insert(item.state.clone());
                        }
                        "Project" => {
                            values.insert(item.project_path.clone());
                        }
                        "Type" => {
                            values.insert(item.target_type.clone());
                        }
                        "ID" => {
                            values.insert(item.id.clone());
                        }
                        "Title" => {
                            values.insert(item.title.clone());
                        }
                        _ => {}
                    }
                }
            }
            Tab::Milestones => {
                for item in &self.milestones.items {
                    match col {
                        "ID" => {
                            values.insert(item.id.to_string());
                        }
                        "Title" => {
                            values.insert(item.title.clone());
                        }
                        "State" => {
                            values.insert(item.state.clone());
                        }
                        _ => {}
                    }
                }
            }
            Tab::Terminal => {}
        }
        values.into_iter().collect()
    }

    pub fn rebuild_group_map(&mut self) {
        self.group_items.clear();
        let Some(col) = self.group_by_column.clone() else {
            return;
        };
        let column_label = col.clone();
        let groups: std::collections::BTreeMap<String, Vec<usize>> = match self.active_tab {
            Tab::Issues => {
                let items = self.filtered_issues();
                let mut map: std::collections::BTreeMap<String, Vec<usize>> =
                    std::collections::BTreeMap::new();
                for (idx, i) in items.iter().enumerate() {
                    let key = match col.as_str() {
                        "State" => i.state.clone(),
                        "Author" => i.author.username.clone(),
                        "Labels" => {
                            if i.labels.is_empty() {
                                "None".to_string()
                            } else {
                                i.labels[0].clone()
                            }
                        }
                        "Milestone" => i
                            .milestone
                            .as_ref()
                            .map(|m| m.title.clone())
                            .unwrap_or_else(|| "None".to_string()),
                        "Assignees" => {
                            if i.assignees.is_empty() {
                                "Unassigned".to_string()
                            } else {
                                i.assignees
                                    .iter()
                                    .map(|a| a.username.clone())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            }
                        }
                        "ID" => format!("#{}", i.iid),
                        "Title" => {
                            let c = i.title.chars().next().unwrap_or('?');
                            c.to_uppercase().to_string()
                        }
                        _ => "Unknown".to_string(),
                    };
                    map.entry(key).or_default().push(idx);
                }
                map
            }
            Tab::MergeRequests => {
                let items = self.filtered_mrs();
                let mut map: std::collections::BTreeMap<String, Vec<usize>> =
                    std::collections::BTreeMap::new();
                for (idx, m) in items.iter().enumerate() {
                    let key = match col.as_str() {
                        "State" => m.state.clone(),
                        "Author" => m.author.username.clone(),
                        "Labels" => {
                            if m.labels.is_empty() {
                                "None".to_string()
                            } else {
                                m.labels[0].clone()
                            }
                        }
                        "Milestone" => m
                            .milestone
                            .as_ref()
                            .map(|m| m.title.clone())
                            .unwrap_or_else(|| "None".to_string()),
                        "Assignees" => {
                            if m.assignees.is_empty() {
                                "Unassigned".to_string()
                            } else {
                                m.assignees
                                    .iter()
                                    .map(|a| a.username.clone())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            }
                        }
                        "Reviewers" => {
                            if m.reviewers.is_empty() {
                                "None".to_string()
                            } else {
                                m.reviewers
                                    .iter()
                                    .map(|r| r.username.clone())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            }
                        }
                        "Status" => {
                            if m.draft {
                                "Draft".to_string()
                            } else {
                                "Ready".to_string()
                            }
                        }
                        "ID" => format!("#{}", m.iid),
                        "Title" => {
                            let c = m.title.chars().next().unwrap_or('?');
                            c.to_uppercase().to_string()
                        }
                        _ => "Unknown".to_string(),
                    };
                    map.entry(key).or_default().push(idx);
                }
                map
            }
            Tab::Todos => {
                let items = self.filtered_todos();
                let mut map: std::collections::BTreeMap<String, Vec<usize>> =
                    std::collections::BTreeMap::new();
                for (idx, n) in items.iter().enumerate() {
                    let key = match col.as_str() {
                        "State" => n.state.clone(),
                        "Type" => n.target_type.clone(),
                        "Project" => n.project_path.clone(),
                        "ID" => format!("#{}", n.target_iid),
                        "Title" => {
                            let c = n.title.chars().next().unwrap_or('?');
                            c.to_uppercase().to_string()
                        }
                        _ => "Unknown".to_string(),
                    };
                    map.entry(key).or_default().push(idx);
                }
                map
            }
            Tab::Pipelines => {
                let items = self.filtered_pipelines();
                let mut map: std::collections::BTreeMap<String, Vec<usize>> =
                    std::collections::BTreeMap::new();
                for (idx, p) in items.iter().enumerate() {
                    let key = match col.as_str() {
                        "Status" => p.status.clone(),
                        "Ref" => p.r#ref.clone(),
                        "ID" => format!("#{}", p.id),
                        _ => "Unknown".to_string(),
                    };
                    map.entry(key).or_default().push(idx);
                }
                map
            }
            Tab::Jobs => {
                let items = self.filtered_jobs();
                let mut map: std::collections::BTreeMap<String, Vec<usize>> =
                    std::collections::BTreeMap::new();
                for (idx, j) in items.iter().enumerate() {
                    let key = match col.as_str() {
                        "Status" => j.status.clone(),
                        "Stage" => j.stage.clone(),
                        "Name" => j.name.clone(),
                        "ID" => format!("#{}", j.id),
                        _ => "Unknown".to_string(),
                    };
                    map.entry(key).or_default().push(idx);
                }
                map
            }
            Tab::Releases => {
                let items = self.filtered_releases();
                let mut map: std::collections::BTreeMap<String, Vec<usize>> =
                    std::collections::BTreeMap::new();
                for (idx, r) in items.iter().enumerate() {
                    let key = match col.as_str() {
                        "Date" => {
                            if r.released_at.len() >= 10 {
                                r.released_at[..10].to_string()
                            } else {
                                r.released_at.clone()
                            }
                        }
                        "Author" => r
                            .author_name
                            .clone()
                            .unwrap_or_else(|| "Unknown".to_string()),
                        "Tag" => r.tag_name.clone(),
                        "Release Name" => r.name.clone(),
                        _ => "Unknown".to_string(),
                    };
                    map.entry(key).or_default().push(idx);
                }
                map
            }
            Tab::Milestones => {
                let items = self.filtered_milestones();
                let mut map: std::collections::BTreeMap<String, Vec<usize>> =
                    std::collections::BTreeMap::new();
                for (idx, m) in items.iter().enumerate() {
                    let key = match col.as_str() {
                        "State" => m.state.clone(),
                        "Start Date" => m.start_date.clone().unwrap_or_else(|| "None".to_string()),
                        "Due Date" => m.due_date.clone().unwrap_or_else(|| "None".to_string()),
                        "Title" => m.title.clone(),
                        "ID" => format!("#{}", m.iid),
                        _ => "Unknown".to_string(),
                    };
                    map.entry(key).or_default().push(idx);
                }
                map
            }
            _ => return,
        };
        for (name, indices) in &groups {
            self.group_items
                .push(GroupItem::Header(format!("{}: {}", column_label, name)));
            for &i in indices {
                self.group_items.push(GroupItem::Item(i));
            }
        }
        let total = self.group_items.len();
        if total > 0 {
            if let Some(sel) = self.group_list_state.selected() {
                if sel >= total {
                    self.group_list_state.select(Some(total - 1));
                }
            } else {
                self.group_list_state.select(Some(0));
            }
        } else {
            self.group_list_state.select(None);
        }
    }

    pub fn update_filter_selection(&mut self) {
        match self.active_tab {
            Tab::Issues => {
                let len = self.filtered_issues().len();
                let sel = self.issues.state.selected();
                if len == 0 {
                    self.issues.state.select(None);
                } else {
                    match sel {
                        Some(idx) => {
                            if idx >= len {
                                self.issues.state.select(Some(len - 1));
                            }
                        }
                        None => {
                            self.issues.state.select(Some(0));
                        }
                    }
                }
            }
            Tab::MergeRequests => {
                let len = self.filtered_mrs().len();
                let sel = self.mrs.state.selected();
                if len == 0 {
                    self.mrs.state.select(None);
                } else {
                    match sel {
                        Some(idx) => {
                            if idx >= len {
                                self.mrs.state.select(Some(len - 1));
                            }
                        }
                        None => {
                            self.mrs.state.select(Some(0));
                        }
                    }
                }
            }
            Tab::Pipelines => {
                let len = self.filtered_pipelines().len();
                let sel = self.pipelines.state.selected();
                if len == 0 {
                    self.pipelines.state.select(None);
                } else {
                    match sel {
                        Some(idx) => {
                            if idx >= len {
                                self.pipelines.state.select(Some(len - 1));
                            }
                        }
                        None => {
                            self.pipelines.state.select(Some(0));
                        }
                    }
                }
            }
            Tab::Runners => {
                let len = self.filtered_runners().len();
                let sel = self.runners.state.selected();
                if len == 0 {
                    self.runners.state.select(None);
                } else {
                    match sel {
                        Some(idx) => {
                            if idx >= len {
                                self.runners.state.select(Some(len - 1));
                            }
                        }
                        None => {
                            self.runners.state.select(Some(0));
                        }
                    }
                }
            }
            Tab::Releases => {
                let len = self.filtered_releases().len();
                let sel = self.releases.state.selected();
                if len == 0 {
                    self.releases.state.select(None);
                } else {
                    match sel {
                        Some(idx) => {
                            if idx >= len {
                                self.releases.state.select(Some(len - 1));
                            }
                        }
                        None => {
                            self.releases.state.select(Some(0));
                        }
                    }
                }
            }
            Tab::Todos => {
                let len = self.filtered_todos().len();
                let sel = self.todos.state.selected();
                if len == 0 {
                    self.todos.state.select(None);
                } else {
                    match sel {
                        Some(idx) => {
                            if idx >= len {
                                self.todos.state.select(Some(len - 1));
                            }
                        }
                        None => {
                            self.todos.state.select(Some(0));
                        }
                    }
                }
            }
            Tab::Jobs => {
                let len = self.filtered_jobs().len();
                if len == 0 {
                    self.selected_job_index = None;
                    self.jobs_list_state.select(None);
                } else {
                    let idx = self.selected_job_index.unwrap_or(0).min(len - 1);
                    self.selected_job_index = Some(idx);
                    self.jobs_list_state.select(Some(idx));
                }
            }
            Tab::Milestones => {
                let len = self.filtered_milestones().len();
                let sel = self.milestones.state.selected();
                if len == 0 {
                    self.milestones.state.select(None);
                } else {
                    match sel {
                        Some(idx) => {
                            if idx >= len {
                                self.milestones.state.select(Some(len - 1));
                            }
                        }
                        None => {
                            self.milestones.state.select(Some(0));
                        }
                    }
                }
            }
            Tab::Terminal => {}
        }
        self.rebuild_group_map();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_date_picker_navigation() {
        let mut dp = DatePicker::new(
            "Select Date".to_string(),
            "2026-07-03",
            DatePickerAction::EditNewField { field_idx: 0 },
        );

        assert_eq!(dp.year, 2026);
        assert_eq!(dp.month, 7);
        assert_eq!(dp.day, 3);
        assert_eq!(dp.value_string(), "2026-07-03");

        // Move day forward by 1
        dp.move_day(1);
        assert_eq!(dp.value_string(), "2026-07-04");

        // Move day backward by 5
        dp.move_day(-5);
        assert_eq!(dp.value_string(), "2026-06-29");

        // Move month forward by 1
        dp.move_month(1);
        assert_eq!(dp.value_string(), "2026-07-29");

        // Move month backward by 2
        dp.move_month(-2);
        assert_eq!(dp.value_string(), "2026-05-29");
    }

    #[test]
    fn test_selector_fuzzy_matching() {
        let selector = Selector {
            title: "Labels".to_string(),
            all_items: vec![
                "bug".to_string(),
                "feature request".to_string(),
                "documentation".to_string(),
                "critical bug".to_string(),
            ],
            selected_items: std::collections::HashSet::new(),
            cursor_idx: 0,
            search_query: "bug".to_string(),
            is_filtering: true,
            is_loading: false,
            entity_iid: 1,
            entity_type: "issue".to_string(),
            field_type: "labels".to_string(),
            multi_select: true,
            state: ListState::default(),
        };

        let filtered = selector.get_filtered_items();
        // Since query is "bug", both "bug" and "critical bug" should match.
        // "bug" should be ranked higher than "critical bug" because "bug" is an exact match / matches at start.
        assert!(filtered.contains(&"bug".to_string()));
        assert!(filtered.contains(&"critical bug".to_string()));
        assert_eq!(filtered[0], "bug".to_string());
        assert_eq!(filtered[1], "critical bug".to_string());
    }

    #[test]
    fn test_mr_fuzzy_status_matching() {
        use crate::gitlab::mr::Author;
        use crate::gitlab::mr::MergeRequest;

        let author = Author {
            username: "johndoe".to_string(),
        };

        let mr_draft_meta = MergeRequest {
            iid: 1,
            title: "Some MR title".to_string(),
            state: "opened".to_string(),
            draft: true,
            author: author.clone(),
            updated_at: "2026-06-02T21:00:00Z".to_string(),
            target_branch: "main".to_string(),
            source_branch: "feature".to_string(),
            labels: vec![],
            assignees: vec![],
            reviewers: vec![],
            milestone: None,
            description: None,
        };

        let mr_draft_title = MergeRequest {
            iid: 2,
            title: "WIP: Another MR title".to_string(),
            state: "opened".to_string(),
            draft: false,
            author: author.clone(),
            updated_at: "2026-06-02T21:00:00Z".to_string(),
            target_branch: "main".to_string(),
            source_branch: "feature2".to_string(),
            labels: vec![],
            assignees: vec![],
            reviewers: vec![],
            milestone: None,
            description: None,
        };

        let mr_ready = MergeRequest {
            iid: 3,
            title: "Finished MR title".to_string(),
            state: "opened".to_string(),
            draft: false,
            author: author.clone(),
            updated_at: "2026-06-02T21:00:00Z".to_string(),
            target_branch: "main".to_string(),
            source_branch: "feature3".to_string(),
            labels: vec![],
            assignees: vec![],
            reviewers: vec![],
            milestone: None,
            description: None,
        };

        let items = vec![mr_draft_meta, mr_draft_title, mr_ready];
        let enabled_cols: std::collections::HashSet<String> = Tab::MergeRequests
            .columns(false)
            .iter()
            .map(|s| s.to_string())
            .collect();

        // Filter by "DRAFT"
        let filtered_draft = App::filter_mrs_list(&items, "DRAFT", &enabled_cols);
        assert_eq!(filtered_draft.len(), 2);
        assert_eq!(filtered_draft[0].iid, 1);
        assert_eq!(filtered_draft[1].iid, 2);

        // Filter by "READY"
        let filtered_ready = App::filter_mrs_list(&items, "READY", &enabled_cols);
        assert_eq!(filtered_ready.len(), 1);
        assert_eq!(filtered_ready[0].iid, 3);

        // Filter by state "OPEN"
        let filtered_open = App::filter_mrs_list(&items, "OPEN", &enabled_cols);
        assert_eq!(filtered_open.len(), 3);
        assert_eq!(filtered_open[0].iid, 1);
        assert_eq!(filtered_open[1].iid, 2);
        assert_eq!(filtered_open[2].iid, 3);
    }

    #[test]
    fn test_diff_view_file_navigation() {
        let diff_content = "\
diff --git a/src/app.rs b/src/app.rs
index 123456..789012 100644
--- a/src/app.rs
+++ b/src/app.rs
@@ -10,6 +10,7 @@
 some content
+new line 1
-deleted line 1
 normal line
diff --git a/src/main.rs b/src/main.rs
index abcdef..ffffff 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -20,6 +20,7 @@
 main content
+main new line 1
";
        let mut diff_view = DiffView::new(42, diff_content.to_string());

        // Check visible nodes (flattened tree)
        assert_eq!(diff_view.visible_nodes.len(), 3);

        assert_eq!(diff_view.visible_nodes[0].name, "src");
        assert!(diff_view.visible_nodes[0].is_dir);

        assert_eq!(diff_view.visible_nodes[1].name, "app.rs");
        assert!(!diff_view.visible_nodes[1].is_dir);
        assert_eq!(
            diff_view.visible_nodes[1].file_path.as_deref(),
            Some("src/app.rs")
        );
        assert_eq!(diff_view.visible_nodes[1].line_idx, Some(0));

        assert_eq!(diff_view.visible_nodes[2].name, "main.rs");
        assert!(!diff_view.visible_nodes[2].is_dir);
        assert_eq!(
            diff_view.visible_nodes[2].file_path.as_deref(),
            Some("src/main.rs")
        );
        assert_eq!(diff_view.visible_nodes[2].line_idx, Some(9));

        // Focus defaults to files panel
        assert!(diff_view.focus_on_files);
        assert_eq!(diff_view.selected_visible_idx, 0);

        // Verify update_selected_file_from_cursor
        diff_view.cursor_idx = 4;
        diff_view.update_selected_file_from_cursor();
        assert_eq!(diff_view.selected_visible_idx, 1);

        diff_view.cursor_idx = 10;
        diff_view.update_selected_file_from_cursor();
        assert_eq!(diff_view.selected_visible_idx, 2);

        // Verify ANSI escape code stripping
        let color_diff = "\
\u{1b}[33mdiff --git a/src/app.rs b/src/app.rs\u{1b}[0m
\u{1b}[34mindex 123456..789012 100644\u{1b}[0m
\u{1b}[31m--- a/src/app.rs\u{1b}[0m
\u{1b}[32m+++ b/src/app.rs\u{1b}[0m
@@ -10,6 +10,7 @@
 some content
\u{1b}[32m+new line 1\u{1b}[0m
\u{1b}[31m-deleted line 1\u{1b}[0m
";
        let color_view = DiffView::new(42, color_diff.to_string());
        assert_eq!(color_view.visible_nodes.len(), 2); // "src" directory and "app.rs" file
        assert_eq!(
            color_view.visible_nodes[1].file_path.as_deref(),
            Some("src/app.rs")
        );
        assert_eq!(color_view.lines[6].line_type, DiffLineType::Addition);
        assert_eq!(color_view.lines[7].line_type, DiffLineType::Deletion);
    }

    #[test]
    fn test_diff_view_glab_parsing() {
        let glab_diff = "\
--- README.md
+++ README.md
@@ -1,7 +1,30 @@
 organizational principles
--- vn-protocol
+++ vn-protocol
@@ -20,6 +20,7 @@
 some content
";
        let diff_view = DiffView::new(42, glab_diff.to_string());
        assert_eq!(diff_view.visible_nodes.len(), 2);
        assert_eq!(diff_view.visible_nodes[0].name, "README.md");
        assert_eq!(diff_view.visible_nodes[1].name, "vn-protocol");
        assert_eq!(diff_view.visible_nodes[0].line_idx, Some(0));
        assert_eq!(diff_view.visible_nodes[1].line_idx, Some(4));
    }

    #[test]
    fn test_column_toggle_checklist_defaults() {
        let app = App::default();
        assert!(!app.is_column_visible(Tab::Issues, "Assignees"));
        assert!(app.is_column_visible(Tab::Issues, "Labels"));
        assert!(!app.is_column_visible(Tab::Issues, "Milestone"));
        assert!(!app.is_column_visible(Tab::Issues, "Author"));

        assert!(!app.is_column_visible(Tab::MergeRequests, "Assignees"));
        assert!(!app.is_column_visible(Tab::MergeRequests, "Reviewers"));
        assert!(app.is_column_visible(Tab::MergeRequests, "Labels"));
        assert!(!app.is_column_visible(Tab::MergeRequests, "Milestone"));
        assert!(!app.is_column_visible(Tab::MergeRequests, "Author"));

        assert!(!app.focus_column_checklist);
        assert_eq!(app.column_checklist_idx, 0);
    }

    #[test]
    fn test_side_by_side_alignment() {
        let lines = vec![
            DiffLine {
                content: "@@ -1,2 +1,3 @@".to_string(),
                line_type: DiffLineType::HunkHeader,
                file_path: "foo.txt".to_string(),
                old_line_num: None,
                new_line_num: None,
                syntax_highlighted: None,
            },
            DiffLine {
                content: "-deleted line".to_string(),
                line_type: DiffLineType::Deletion,
                file_path: "foo.txt".to_string(),
                old_line_num: Some(1),
                new_line_num: None,
                syntax_highlighted: None,
            },
            DiffLine {
                content: "+added line 1".to_string(),
                line_type: DiffLineType::Addition,
                file_path: "foo.txt".to_string(),
                old_line_num: None,
                new_line_num: Some(1),
                syntax_highlighted: None,
            },
            DiffLine {
                content: "+added line 2".to_string(),
                line_type: DiffLineType::Addition,
                file_path: "foo.txt".to_string(),
                old_line_num: None,
                new_line_num: Some(2),
                syntax_highlighted: None,
            },
            DiffLine {
                content: " normal line".to_string(),
                line_type: DiffLineType::Normal,
                file_path: "foo.txt".to_string(),
                old_line_num: Some(2),
                new_line_num: Some(3),
                syntax_highlighted: None,
            },
        ];

        let side_by_side = build_side_by_side_lines(&lines);

        assert_eq!(side_by_side.len(), 4);

        assert_eq!(side_by_side[0].line_type, DiffLineType::HunkHeader);
        assert!(side_by_side[0].left.is_some());
        assert!(side_by_side[0].right.is_some());

        assert_eq!(side_by_side[1].line_type, DiffLineType::Normal);
        assert_eq!(
            side_by_side[1].left.as_ref().unwrap().content,
            "-deleted line"
        );
        assert_eq!(
            side_by_side[1].right.as_ref().unwrap().content,
            "+added line 1"
        );

        assert_eq!(side_by_side[2].line_type, DiffLineType::Normal);
        assert!(side_by_side[2].left.is_none());
        assert_eq!(
            side_by_side[2].right.as_ref().unwrap().content,
            "+added line 2"
        );

        assert_eq!(side_by_side[3].line_type, DiffLineType::Normal);
        assert_eq!(
            side_by_side[3].left.as_ref().unwrap().content,
            " normal line"
        );
        assert_eq!(
            side_by_side[3].right.as_ref().unwrap().content,
            " normal line"
        );
    }

    #[test]
    fn test_show_submit_review_prompt_defaults() {
        let app = App::default();
        assert_eq!(app.show_submit_review_prompt, None);
        assert!(!app.in_review_mode);
        assert!(app.draft_comments.is_empty());
    }

    #[test]
    fn test_get_comment_range() {
        let diff_content = "\
diff --git a/foo.txt b/foo.txt
index 123456..789012 100644
--- a/foo.txt
+++ b/foo.txt
@@ -1,3 +1,3 @@
 normal line 1
-deleted line 1
-deleted line 2
+added line 1
+added line 2
 normal line 2
";
        let mut diff_view = DiffView::new(42, diff_content.to_string());
        diff_view.side_by_side = true;
        diff_view.update_active_lines();

        // Let's test selection spanning rows 6 to 7
        diff_view.selection_start = Some(6);
        diff_view.selection_end = Some(7);

        let range = diff_view.get_comment_range().unwrap();
        assert_eq!(range.file_path, "foo.txt");
        assert_eq!(range.line_num, Some(2)); // added line 1 is new line 2
        assert_eq!(range.end_line_num, Some(3)); // added line 2 is new line 3
        assert_eq!(range.old_line_num, None);
        assert_eq!(range.end_old_line_num, None);
        assert_eq!(range.lines.len(), 2);
        assert_eq!(range.lines[0].content, "+added line 1");
        assert_eq!(range.lines[1].content, "+added line 2");

        let diff_content_2 = "\
diff --git a/foo.txt b/foo.txt
--- a/foo.txt
+++ b/foo.txt
@@ -1,4 +1,2 @@
-deleted line 1
-deleted line 2
-deleted line 3
+added line 1
";
        let mut diff_view_2 = DiffView::new(42, diff_content_2.to_string());
        diff_view_2.side_by_side = true;
        diff_view_2.update_active_lines();

        // Selecting rows 5 to 6 (which are purely deletions)
        diff_view_2.selection_start = Some(5);
        diff_view_2.selection_end = Some(6);
        let range_2 = diff_view_2.get_comment_range().unwrap();
        assert_eq!(range_2.line_num, None);
        assert_eq!(range_2.end_line_num, None);
        assert_eq!(range_2.old_line_num, Some(2)); // deleted line 2
        assert_eq!(range_2.end_old_line_num, Some(3)); // deleted line 3
        assert_eq!(range_2.lines.len(), 2);
        assert_eq!(range_2.lines[0].content, "-deleted line 2");
        assert_eq!(range_2.lines[1].content, "-deleted line 3");
    }

    #[test]
    fn test_unresolved_threads_count() {
        use crate::gitlab::mr::{Author, DiscussionNote, NotePosition};

        let author = Author {
            username: "tester".to_string(),
        };

        let mut app = App::new();

        // 1. Thread 1: unresolved
        let note1 = DiscussionNote {
            id: 1,
            body: "note 1".to_string(),
            author: author.clone(),
            created_at: "now".to_string(),
            system: false,
            position: Some(NotePosition {
                old_path: Some("src/main.rs".to_string()),
                new_path: Some("src/main.rs".to_string()),
                old_line: None,
                new_line: Some(10),
                start_line: None,
                line_range: None,
            }),
            discussion_id: Some("thread_1".to_string()),
            resolved: Some(false),
            resolvable: Some(true),
        };

        // 2. Thread 2: resolved
        let note2 = DiscussionNote {
            id: 2,
            body: "note 2".to_string(),
            author: author.clone(),
            created_at: "now".to_string(),
            system: false,
            position: Some(NotePosition {
                old_path: Some("src/main.rs".to_string()),
                new_path: Some("src/main.rs".to_string()),
                old_line: None,
                new_line: Some(20),
                start_line: None,
                line_range: None,
            }),
            discussion_id: Some("thread_2".to_string()),
            resolved: Some(true),
            resolvable: Some(true),
        };

        // 3. Thread 3: unresolved because one reply is unresolved
        let note3_1 = DiscussionNote {
            id: 3,
            body: "note 3.1".to_string(),
            author: author.clone(),
            created_at: "now".to_string(),
            system: false,
            position: Some(NotePosition {
                old_path: Some("src/lib.rs".to_string()),
                new_path: Some("src/lib.rs".to_string()),
                old_line: None,
                new_line: Some(5),
                start_line: None,
                line_range: None,
            }),
            discussion_id: Some("thread_3".to_string()),
            resolved: Some(true),
            resolvable: Some(true),
        };
        let note3_2 = DiscussionNote {
            id: 4,
            body: "note 3.2".to_string(),
            author: author.clone(),
            created_at: "now".to_string(),
            system: false,
            position: Some(NotePosition {
                old_path: Some("src/lib.rs".to_string()),
                new_path: Some("src/lib.rs".to_string()),
                old_line: None,
                new_line: Some(5),
                start_line: None,
                line_range: None,
            }),
            discussion_id: Some("thread_3".to_string()),
            resolved: Some(false),
            resolvable: Some(true),
        };

        // System comment (should be ignored)
        let note_system = DiscussionNote {
            id: 5,
            body: "system note".to_string(),
            author: author.clone(),
            created_at: "now".to_string(),
            system: true,
            position: None,
            discussion_id: Some("thread_system".to_string()),
            resolved: Some(false),
            resolvable: Some(true),
        };

        app.current_comments = vec![note1, note2, note3_1, note3_2, note_system];

        // Total unresolved threads should be 2 (thread_1 and thread_3)
        assert_eq!(app.unresolved_threads_count(), 2);

        // Path filtering
        // src/main.rs has thread_1 (unresolved) and thread_2 (resolved) -> 1 unresolved
        assert_eq!(app.unresolved_threads_count_for_path("src/main.rs"), 1);

        // src/lib.rs has thread_3 (unresolved) -> 1 unresolved
        assert_eq!(app.unresolved_threads_count_for_path("src/lib.rs"), 1);

        // src directory should capture both src/main.rs and src/lib.rs -> 2 unresolved
        assert_eq!(app.unresolved_threads_count_for_path("src"), 2);

        // unrelated path
        assert_eq!(app.unresolved_threads_count_for_path("other.txt"), 0);
    }
}
