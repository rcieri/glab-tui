#![allow(dead_code)]

use crate::backend::BackendKind;
use crate::config::Config;
use crate::utils::ui::StatefulTable;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::widgets::ListState;
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

pub use crate::config::SaveMenu;

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
    Branches,
    Environments,
    Terminal,
}

impl Tab {
    pub const ALL: [Tab; 11] = [
        Tab::Issues,
        Tab::MergeRequests,
        Tab::Pipelines,
        Tab::Jobs,
        Tab::Runners,
        Tab::Releases,
        Tab::Todos,
        Tab::Milestones,
        Tab::Branches,
        Tab::Environments,
        Tab::Terminal,
    ];

    pub fn to_str(&self) -> &'static str {
        match self {
            Tab::Issues => "issues",
            Tab::MergeRequests => "mrs",
            Tab::Pipelines => "pipelines",
            Tab::Jobs => "jobs",
            Tab::Runners => "runners",
            Tab::Releases => "releases",
            Tab::Todos => "todos",
            Tab::Milestones => "milestones",
            Tab::Branches => "branches",
            Tab::Environments => "environments",
            Tab::Terminal => "terminal",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "issues" => Some(Tab::Issues),
            "mrs" | "mergerequests" => Some(Tab::MergeRequests),
            "pipelines" => Some(Tab::Pipelines),
            "jobs" => Some(Tab::Jobs),
            "runners" => Some(Tab::Runners),
            "releases" => Some(Tab::Releases),
            "todos" => Some(Tab::Todos),
            "milestones" => Some(Tab::Milestones),
            "branches" => Some(Tab::Branches),
            "environments" => Some(Tab::Environments),
            "terminal" => Some(Tab::Terminal),
            _ => None,
        }
    }

    pub fn title(&self, kind: BackendKind) -> String {
        let icons = crate::config::ICONS.read().unwrap();
        match self {
            Tab::Issues => format!("{} Issues", icons.tab_issue),
            Tab::MergeRequests => {
                if kind.is_github() {
                    format!("{} PRs", icons.tab_pr)
                } else {
                    format!("{} MRs", icons.tab_pr)
                }
            }
            Tab::Pipelines => {
                if kind.is_github() {
                    format!("{} Actions", icons.tab_pipeline)
                } else {
                    format!("{} Pipelines", icons.tab_pipeline)
                }
            }
            Tab::Jobs => format!("{} Jobs", icons.tab_job),
            Tab::Runners => format!("{} Runners", icons.tab_runner),
            Tab::Releases => format!("{} Releases", icons.tab_release),
            Tab::Todos => {
                if kind.is_github() {
                    format!("{} Notifications", icons.tab_todo)
                } else {
                    format!("{} Todos", icons.tab_todo)
                }
            }
            Tab::Milestones => format!("{} Milestones", icons.tab_milestone),
            Tab::Branches => format!("{} Branches", icons.tab_branch),
            Tab::Environments => format!("{} Environments", icons.tab_environment),
            Tab::Terminal => format!("{} Terminal", icons.tab_terminal),
        }
    }

    pub fn columns(&self, kind: BackendKind) -> Vec<&'static str> {
        match self {
            Tab::Issues => {
                let mut cols = vec!["ID", "State", "Title", "Assignees", "Labels", "Milestone"];
                if !kind.is_github() {
                    cols.push("Due Date");
                }
                cols.push("Author");
                cols
            }
            Tab::MergeRequests => {
                let mut cols = vec![
                    "ID",
                    "State",
                    "Status",
                    "Title",
                    "Assignees",
                    "Reviewers",
                    "Labels",
                ];
                if kind.is_github() {
                    cols.push("Action");
                } else {
                    cols.push("Pipeline");
                }
                cols.push("Milestone");
                cols.push("Author");
                cols
            }
            Tab::Pipelines => {
                let mut cols = vec!["ID", "Status", "Ref"];
                if kind.is_github() {
                    cols.push("Name");
                    cols.push("Event");
                    cols.push("SHA");
                    cols.push("Actor");
                } else {
                    cols.push("Stages");
                }
                cols
            }
            Tab::Jobs => {
                let mut cols = vec!["ID", "Status", "Name", "Matrix"];
                if kind.is_github() {
                    cols.push("Runner");
                } else {
                    cols.push("Stage");
                }
                cols
            }
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
            Tab::Branches => vec!["Name", "Default", "Protected", "SHA"],
            Tab::Environments => vec!["Name", "State", "Deployment Status", "URL"],
            Tab::Terminal => vec![],
        }
    }

    pub fn default_columns(&self, kind: BackendKind) -> Vec<&'static str> {
        match self {
            Tab::Issues => {
                let mut cols = vec!["ID", "State", "Title", "Labels"];
                if !kind.is_github() {
                    cols.push("Due Date");
                }
                cols
            }
            Tab::MergeRequests => vec!["ID", "State", "Status", "Title", "Labels"],
            Tab::Pipelines => {
                if kind.is_github() {
                    vec!["Name", "Status", "Event", "Ref"]
                } else {
                    vec!["ID", "Status", "Stages", "Ref"]
                }
            }
            Tab::Jobs => {
                if kind.is_github() {
                    vec!["Name", "Status", "Ref"]
                } else {
                    vec!["ID", "Stage", "Status", "Name", "Matrix"]
                }
            }
            Tab::Runners => vec!["ID", "Description", "Status", "Active"],
            Tab::Releases => vec!["Tag", "Release Name", "Date"],
            Tab::Todos => vec!["State", "Project", "Type", "ID", "Title"],
            Tab::Milestones => vec!["ID", "State", "Title", "Progress", "Due Date"],
            Tab::Branches => vec!["Name", "Default", "Protected"],
            Tab::Environments => vec!["Name", "State", "Deployment Status"],
            Tab::Terminal => vec![],
        }
    }

    pub fn available_on_platform(&self, _kind: BackendKind) -> bool {
        true
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
        if query.is_empty() {
            return self
                .all_items
                .iter()
                .map(|item| (item.clone(), None))
                .collect();
        }

        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, String, Option<Vec<usize>>)> = self
            .all_items
            .iter()
            .filter_map(|item| {
                matcher
                    .fuzzy_indices(item, query)
                    .map(|(score, indices)| (score, item.clone(), Some(indices)))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));

        let mut items: Vec<(String, Option<Vec<usize>>)> = scored
            .into_iter()
            .map(|(_, item, indices)| (item, indices))
            .collect();

        let exact_match = self
            .all_items
            .iter()
            .any(|item| item.to_lowercase() == query.to_lowercase());
        if !exact_match {
            items.push((format!("+ Create \"{}\"", query), None));
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
    pub fuzzy_indices: Option<Vec<usize>>,
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
        old_file_path: Option<String>,
        is_new_file: bool,
        is_deleted_file: bool,
        line_idx: usize,
        additions: u32,
        deletions: u32,
    },
}

impl DiffTreeNode {
    pub fn insert(
        &mut self,
        path_parts: &[&str],
        full_path: &str,
        old_path: Option<&str>,
        is_new_file: bool,
        is_deleted_file: bool,
        additions: u32,
        deletions: u32,
        line_idx: usize,
    ) {
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
                            old_file_path: old_path.map(|s| s.to_string()),
                            is_new_file,
                            is_deleted_file,
                            line_idx,
                            additions,
                            deletions,
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
                        children[pos].insert(
                            &path_parts[1..],
                            full_path,
                            old_path,
                            is_new_file,
                            is_deleted_file,
                            additions,
                            deletions,
                            line_idx,
                        );
                    } else {
                        let mut new_dir = DiffTreeNode::Directory {
                            name,
                            is_expanded: true,
                            children: Vec::new(),
                        };
                        new_dir.insert(
                            &path_parts[1..],
                            full_path,
                            old_path,
                            is_new_file,
                            is_deleted_file,
                            additions,
                            deletions,
                            line_idx,
                        );
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
                        old_file_path: None,
                        is_new_file: false,
                        is_deleted_file: false,
                        line_idx: None,
                        path_id: path_id.clone(),
                        additions: 0,
                        deletions: 0,
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
                old_file_path,
                is_new_file,
                is_deleted_file,
                line_idx,
                additions,
                deletions,
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
                    old_file_path: old_file_path.clone(),
                    is_new_file: *is_new_file,
                    is_deleted_file: *is_deleted_file,
                    line_idx: Some(*line_idx),
                    path_id,
                    additions: *additions,
                    deletions: *deletions,
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
    pub old_file_path: Option<String>,
    pub is_new_file: bool,
    pub is_deleted_file: bool,
    pub line_idx: Option<usize>,
    pub path_id: String,
    pub additions: u32,
    pub deletions: u32,
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
    pub file_tree_scroll_offset: usize,
    pub root_node: DiffTreeNode,
    pub visible_nodes: Vec<FlatDiffTreeNode>,
    pub selected_visible_idx: usize,
    pub focus_on_files: bool,
    pub selection_start: Option<usize>,
    pub selection_end: Option<usize>,
    pub side_by_side: bool,
    pub side_by_side_lines: Vec<SideBySideLine>,
    pub viewport_height: usize,
    pub search_query: String,
    pub search_matches: Vec<usize>,
    pub search_cursor: usize,
    pub search_active: bool,
    pub file_tree_visible: bool,
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
    #[allow(clippy::too_many_lines)]
    pub fn new(mr_iid: u64, raw_diff: String) -> Self {
        let cleaned_diff = strip_ansi_escapes(&raw_diff);
        let mut all_lines = Vec::new();
        let mut current_file = String::new();
        let mut old_line_num = None;
        let mut new_line_num = None;
        let mut files: Vec<(String, Option<String>, bool, bool, usize)> = Vec::new();
        let mut change_counts: std::collections::HashMap<String, (u32, u32)> =
            std::collections::HashMap::new();

        // State tracking for renames
        let mut rename_from: Option<String> = None;
        let mut rename_to: Option<String> = None;

        struct DiffChunkMeta {
            new_path: Option<String>,
            old_path: Option<String>,
            is_new_file: bool,
            is_deleted_file: bool,
        }
        let mut chunk_meta: Option<DiffChunkMeta> = None;

        for line in cleaned_diff.lines() {
            // --- File header detection ---
            let mut detected_file: Option<String> = None;

            if line.starts_with("diff --git") {
                // Finish any previous chunk meta
                chunk_meta = None;
                rename_from = None;
                rename_to = None;
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let a_path = parts[2].strip_prefix("a/").unwrap_or(parts[2]);
                    let b_path = parts[3].strip_prefix("b/").unwrap_or(parts[3]);
                    if a_path != b_path {
                        rename_from = Some(a_path.to_string());
                        rename_to = Some(b_path.to_string());
                    }
                    detected_file = Some(b_path.to_string());
                }
            } else if line.starts_with("rename from ") {
                rename_from = Some(line[12..].trim().to_string());
                chunk_meta = Some(DiffChunkMeta {
                    new_path: chunk_meta
                        .as_ref()
                        .and_then(|m| m.new_path.clone())
                        .or_else(|| rename_to.clone()),
                    old_path: rename_from.clone(),
                    is_new_file: false,
                    is_deleted_file: false,
                });
            } else if line.starts_with("rename to ") {
                rename_to = Some(line[10..].trim().to_string());
                let new_path = rename_to.clone();
                let old_path = rename_from.clone();
                if let Some(ref new) = new_path {
                    current_file = new.clone();
                    let already_exists = files.iter().any(|(f, _, _, _, _)| f == new);
                    if !already_exists {
                        files.push((new.clone(), old_path.clone(), false, false, all_lines.len()));
                    }
                }
                chunk_meta = Some(DiffChunkMeta {
                    new_path,
                    old_path,
                    is_new_file: false,
                    is_deleted_file: false,
                });
            } else if line.starts_with("new file mode ") {
                chunk_meta = Some(DiffChunkMeta {
                    new_path: rename_to.clone(),
                    old_path: None,
                    is_new_file: true,
                    is_deleted_file: false,
                });
            } else if line.starts_with("deleted file mode ") {
                chunk_meta = Some(DiffChunkMeta {
                    new_path: None,
                    old_path: rename_from.clone(),
                    is_new_file: false,
                    is_deleted_file: true,
                });
            } else if line.starts_with("--- ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let path = parts[1];
                    if path != "/dev/null" && !path.is_empty() {
                        let cleaned_path = path.strip_prefix("a/").unwrap_or(path).to_string();
                        // Don't override current_file with old path during renames
                        if rename_from.is_none() || rename_from.as_deref() != Some(&cleaned_path) {
                            detected_file = Some(cleaned_path);
                        }
                    }
                }
            } else if line.starts_with("+++ ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let path = parts[1];
                    if path != "/dev/null" && !path.is_empty() {
                        let cleaned_path = path.strip_prefix("b/").unwrap_or(path).to_string();
                        detected_file = Some(cleaned_path.clone());
                        // Grow chunk_meta new_path if we don't have it yet
                        if chunk_meta.as_ref().map_or(true, |m| m.new_path.is_none()) {
                            chunk_meta = Some(DiffChunkMeta {
                                new_path: Some(cleaned_path.clone()),
                                old_path: rename_from.clone(),
                                is_new_file: chunk_meta.as_ref().map_or(false, |m| m.is_new_file),
                                is_deleted_file: chunk_meta
                                    .as_ref()
                                    .map_or(false, |m| m.is_deleted_file),
                            });
                        }
                    }
                }
            }

            if let Some(ref fp) = detected_file {
                current_file = fp.clone();
                let is_new = chunk_meta.as_ref().map_or(false, |m| m.is_new_file);
                let is_del = chunk_meta.as_ref().map_or(false, |m| m.is_deleted_file);
                let old_path = if rename_from.is_some() {
                    rename_from.clone()
                } else {
                    None
                };
                if let Some(existing) = files.iter_mut().find(|(f, _, _, _, _)| f == fp) {
                    if old_path.is_some() {
                        existing.1 = old_path;
                    }
                    if is_new {
                        existing.2 = true;
                    }
                    if is_del {
                        existing.3 = true;
                    }
                } else {
                    files.push((fp.clone(), old_path, is_new, is_del, all_lines.len()));
                }
            }

            // --- Line classification ---
            if line.starts_with("diff --git") {
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::Meta,
                    file_path: current_file.clone(),
                    old_line_num: None,
                    new_line_num: None,
                    syntax_highlighted: None,
                    fuzzy_indices: None,
                });
                old_line_num = None;
                new_line_num = None;
            } else if line.starts_with("--- ")
                || line.starts_with("+++ ")
                || line.starts_with("index ")
                || line.starts_with("similarity index ")
                || line.starts_with("rename from ")
                || line.starts_with("rename to ")
                || line.starts_with("new file mode ")
                || line.starts_with("deleted file mode ")
                || line.starts_with("Binary files ")
                || line.starts_with("old mode ")
                || line.starts_with("new mode ")
                || line.starts_with("copy from ")
                || line.starts_with("copy to ")
                || line.starts_with("Subproject commit ")
            {
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::Meta,
                    file_path: current_file.clone(),
                    old_line_num: None,
                    new_line_num: None,
                    syntax_highlighted: None,
                    fuzzy_indices: None,
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
                    fuzzy_indices: None,
                });
            } else if line.starts_with('+') {
                let highlighted = highlight_line_syntax(&current_file, line, None);
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::Addition,
                    file_path: current_file.clone(),
                    old_line_num: None,
                    new_line_num,
                    syntax_highlighted: highlighted,
                    fuzzy_indices: None,
                });
                if let Some(ref mut n) = new_line_num {
                    *n += 1;
                }
                change_counts
                    .entry(current_file.clone())
                    .or_insert((0, 0))
                    .1 += 1;
            } else if line.starts_with('-') {
                let highlighted = highlight_line_syntax(&current_file, line, None);
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::Deletion,
                    file_path: current_file.clone(),
                    old_line_num,
                    new_line_num: None,
                    syntax_highlighted: highlighted,
                    fuzzy_indices: None,
                });
                if let Some(ref mut n) = old_line_num {
                    *n += 1;
                }
                change_counts
                    .entry(current_file.clone())
                    .or_insert((0, 0))
                    .0 += 1;
            } else {
                let highlighted = highlight_line_syntax(&current_file, line, None);
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::Normal,
                    file_path: current_file.clone(),
                    old_line_num,
                    new_line_num,
                    syntax_highlighted: highlighted,
                    fuzzy_indices: None,
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

        for (file_path, old_path, is_new, is_del, line_idx) in &files {
            let parts: Vec<&str> = file_path.split(|c| c == '/' || c == '\\').collect();
            let counts = change_counts.get(file_path).copied().unwrap_or((0, 0));
            root_node.insert(
                &parts,
                file_path,
                old_path.as_deref(),
                *is_new,
                *is_del,
                counts.1,
                counts.0,
                *line_idx,
            );
        }

        // Propagate counts up to directory nodes
        Self::compute_dir_counts(&mut root_node);

        let mut visible_nodes = Vec::new();
        root_node.flatten(0, "", &mut visible_nodes);

        // Copy directory counts to flat dir nodes
        Self::copy_dir_counts_to_flat(&root_node, &mut visible_nodes);

        let mut view = Self {
            mr_iid,
            raw_diff,
            all_lines,
            lines: Vec::new(),
            cursor_idx: 0,
            hunks: Vec::new(),
            scroll_offset: 0,
            file_tree_scroll_offset: 0,
            root_node,
            visible_nodes,
            selected_visible_idx: 0,
            focus_on_files: true,
            selection_start: None,
            selection_end: None,
            side_by_side: false,
            side_by_side_lines: Vec::new(),
            viewport_height: 15,
            search_query: String::new(),
            search_matches: Vec::new(),
            search_cursor: 0,
            search_active: false,
            file_tree_visible: true,
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

        // Rebuild search matches for the new active lines
        if !self.search_query.is_empty() {
            let query = self.search_query.clone();
            self.search(&query);
        }
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

    fn compute_dir_counts(node: &mut DiffTreeNode) -> (u32, u32) {
        match node {
            DiffTreeNode::Directory { children, .. } => {
                let mut total_adds = 0u32;
                let mut total_dels = 0u32;
                for child in children.iter_mut() {
                    let (a, d) = Self::compute_dir_counts(child);
                    total_adds += a;
                    total_dels += d;
                }
                (total_adds, total_dels)
            }
            DiffTreeNode::File {
                additions,
                deletions,
                ..
            } => (*additions, *deletions),
        }
    }

    fn compute_dir_counts_raw(node: &DiffTreeNode) -> (u32, u32) {
        match node {
            DiffTreeNode::Directory { children, .. } => {
                let mut total_adds = 0u32;
                let mut total_dels = 0u32;
                for child in children {
                    let (a, d) = Self::compute_dir_counts_raw(child);
                    total_adds += a;
                    total_dels += d;
                }
                (total_adds, total_dels)
            }
            DiffTreeNode::File {
                additions,
                deletions,
                ..
            } => (*additions, *deletions),
        }
    }

    fn copy_dir_counts_to_flat(root: &DiffTreeNode, flat: &mut [FlatDiffTreeNode]) {
        let mut stack: Vec<(&DiffTreeNode, String)> = vec![(root, String::new())];
        while let Some((node, prefix)) = stack.pop() {
            if let DiffTreeNode::Directory { name, children, .. } = node {
                let path_id = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", prefix, name)
                };
                let (adds, dels) = Self::compute_dir_counts_raw(node);
                if let Some(fnode) = flat.iter_mut().find(|n| n.is_dir && n.path_id == path_id) {
                    fnode.additions = adds;
                    fnode.deletions = dels;
                }
                for child in children.iter().rev() {
                    stack.push((child, path_id.clone()));
                }
            }
        }
    }

    pub fn rebuild_visible_nodes(&mut self) {
        // Preserve selected file path so cursor doesn't jump on dir expand/collapse
        let old_file_path = self
            .visible_nodes
            .get(self.selected_visible_idx)
            .and_then(|n| n.file_path.clone().or_else(|| Some(n.path_id.clone())));

        let mut visible = Vec::new();
        self.root_node.flatten(0, "", &mut visible);
        self.visible_nodes = visible;

        if let Some(ref old_path) = old_file_path {
            if let Some(pos) = self.visible_nodes.iter().position(|n| {
                n.file_path.as_deref() == Some(old_path.as_str()) || n.path_id == *old_path
            }) {
                // Same file/dir still selected — keep scroll offset, try keep cursor
                self.selected_visible_idx = pos;
                self.update_active_lines();
                return;
            }
        }
        // Selected node disappeared (e.g. collapsed directory) — reset
        self.selected_visible_idx = 0;
        self.file_tree_scroll_offset = 0;
        self.cursor_idx = 0;
        self.scroll_offset = 0;
        self.update_active_lines();
    }

    pub fn collapse_all(&mut self) {
        fn collapse_recursive(node: &mut DiffTreeNode) {
            if let DiffTreeNode::Directory {
                is_expanded,
                children,
                ..
            } = node
            {
                if *is_expanded {
                    *is_expanded = false;
                    for child in children {
                        collapse_recursive(child);
                    }
                }
            }
        }
        collapse_recursive(&mut self.root_node);
        self.rebuild_visible_nodes();
    }

    pub fn expand_all(&mut self) {
        fn expand_recursive(node: &mut DiffTreeNode) {
            if let DiffTreeNode::Directory {
                is_expanded,
                children,
                ..
            } = node
            {
                *is_expanded = true;
                for child in children {
                    expand_recursive(child);
                }
            }
        }
        expand_recursive(&mut self.root_node);
        self.rebuild_visible_nodes();
    }

    pub fn search(&mut self, query: &str) {
        self.search_query = query.to_string();
        self.search_matches.clear();

        // Clear previous fuzzy indices on all lines
        for line in &mut self.lines {
            line.fuzzy_indices = None;
        }

        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, usize)> = self
            .lines
            .iter_mut()
            .enumerate()
            .filter_map(|(i, line)| {
                let (score, indices) = matcher.fuzzy_indices(&line.content, query)?;
                line.fuzzy_indices = Some(indices);
                Some((score, i))
            })
            .collect();
        scored.sort_by_key(|(score, _)| -(*score));
        self.search_matches = scored.into_iter().map(|(_, i)| i).collect();
        self.search_cursor = 0;
        if let Some(&first_match) = self.search_matches.first() {
            self.cursor_idx = first_match;
            self.scroll_offset = self.cursor_idx.saturating_sub(5);
        }
    }

    pub fn search_next(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        self.search_cursor = (self.search_cursor + 1) % self.search_matches.len();
        if let Some(&pos) = self.search_matches.get(self.search_cursor) {
            self.cursor_idx = pos;
            self.scroll_offset = self.cursor_idx.saturating_sub(5);
        }
    }

    pub fn search_prev(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        self.search_cursor = self
            .search_cursor
            .checked_sub(1)
            .unwrap_or(self.search_matches.len() - 1);
        if let Some(&pos) = self.search_matches.get(self.search_cursor) {
            self.cursor_idx = pos;
            self.scroll_offset = self.cursor_idx.saturating_sub(5);
        }
    }

    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.search_matches.clear();
        self.search_cursor = 0;
        self.search_active = false;
        for line in &mut self.lines {
            line.fuzzy_indices = None;
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
    CreateBranch(String), // ref_branch name
    EditPageSize,
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
    DeleteBranch(String),  // branch name
    DeleteIssue(u64),      // issue iid
    DeleteMr(u64),         // mr iid
    CloseIssue(u64),       // issue iid
    CloseMr(u64),          // mr iid
    MergeMr(u64),          // mr iid
}

pub struct App {
    pub config: Config,
    pub active_tab: Tab,
    pub running: bool,
    pub project_context: String,
    pub project_cache: crate::utils::cache::ProjectCache,
    pub gitlab_client: Option<crate::domain::client::GitlabClient>,
    pub terminal_commands: Vec<TerminalCommand>,
    pub terminal_wrap: bool,
    pub issues: StatefulTable<crate::domain::issues::Issue>,
    pub mrs: StatefulTable<crate::domain::mr::MergeRequest>,
    pub pipelines: StatefulTable<crate::domain::pipelines::Pipeline>,
    pub search_query: String,
    pub is_typing_search: bool,
    pub active_pipeline_id: Option<u64>,
    pub job_trace: Option<String>,
    pub error_message: Option<String>,
    pub error_message_at: Option<std::time::Instant>,
    pub runners: StatefulTable<crate::domain::runners::Runner>,
    pub releases: StatefulTable<crate::domain::releases::Release>,
    pub pipeline_jobs: std::collections::HashMap<u64, Vec<crate::domain::pipelines::Job>>,
    pub fetching_pipelines: std::collections::HashSet<u64>,
    pub loading_tabs: std::collections::HashSet<Tab>,
    pub loaded_tabs: std::collections::HashSet<Tab>,
    pub edit_menu: Option<EditMenu>,
    pub selector: Option<Selector>,
    pub text_input: Option<TextInput>,
    pub editing_page_size: bool,
    pub page_size_input: String,
    pub date_picker: Option<DatePicker>,
    pub jobs: StatefulTable<crate::domain::pipelines::Job>,
    pub detail_scroll: u16,
    pub selected_pipelines: std::collections::HashSet<u64>,
    pub selected_jobs: std::collections::HashSet<u64>,
    pub details_zoomed: bool,
    pub detail_visible: bool,
    pub job_trace_needs_scroll_to_bottom: bool,
    pub job_trace_loading: bool,
    pub job_trace_wrap: bool,
    pub collapse_matrix_jobs: bool,
    pub show_help: bool,
    pub help_search_query: String,
    pub diff_view: Option<DiffView>,
    pub current_comments: Vec<crate::domain::mr::DiscussionNote>,
    pub last_fetched_mr_iid: Option<u64>,
    pub show_submit_review_prompt: Option<u64>,
    pub confirm_popup: Option<ConfirmAction>,
    pub confirm_popup_selected_yes: bool,
    pub diff_loading: bool,
    pub todos: StatefulTable<crate::domain::notifications::Notification>,
    pub status_message: Option<String>,
    pub refreshed_tabs: std::collections::HashSet<Tab>,
    pub tx: Option<tokio::sync::mpsc::UnboundedSender<crate::event::Event>>,
    pub enabled_columns: std::collections::HashMap<Tab, std::collections::HashSet<String>>,
    pub focus_column_checklist: bool,
    pub column_checklist_idx: usize,
    pub in_review_mode: bool,
    pub draft_comments: Vec<DraftComment>,
    pub save_menu_open: bool,
    pub save_menu_selection: Option<SaveMenu>,
    pub page_size: usize,
    pub milestones: StatefulTable<crate::domain::milestones::Milestone>,
    pub selected_milestone_issues: Option<Vec<crate::domain::issues::Issue>>,
    pub selected_milestone_iid: Option<u64>,
    pub milestone_issues_cache: std::collections::HashMap<u64, Vec<crate::domain::issues::Issue>>,
    pub terminal_scroll: usize,
    pub branches: StatefulTable<crate::domain::branches::Branch>,
    pub environments: StatefulTable<crate::domain::deployments::Environment>,
    pub deployments: StatefulTable<crate::domain::deployments::Deployment>,
    pub group_by_column: std::collections::HashMap<Tab, Option<String>>,
    pub group_ascending: std::collections::HashMap<Tab, bool>,
    pub group_list_state: ratatui::widgets::ListState,
    pub group_items: Vec<GroupItem>,
    pub column_filters: std::collections::HashMap<
        Tab,
        std::collections::HashMap<String, std::collections::HashSet<String>>,
    >,
    pub column_filter_context: Option<(Tab, String)>,
    pub sidebar_rect: Option<Rect>,
    pub content_rect: Option<Rect>,
    pub detail_rect: Option<Rect>,
}

impl Default for App {
    fn default() -> Self {
        let config = Config::load();
        Self {
            config: config.clone(),
            active_tab: Tab::default(),
            running: true,
            project_context: "group/repository".to_string(),
            project_cache: crate::utils::cache::ProjectCache::default(),
            gitlab_client: None,
            terminal_commands: vec![],
            terminal_wrap: false,
            issues: StatefulTable::with_items(vec![]),
            mrs: StatefulTable::with_items(vec![]),
            pipelines: StatefulTable::with_items(vec![]),
            search_query: String::new(),
            is_typing_search: false,
            active_pipeline_id: None,
            job_trace: None,
            error_message: None,
            error_message_at: None,
            runners: StatefulTable::with_items(vec![]),
            releases: StatefulTable::with_items(vec![]),
            pipeline_jobs: std::collections::HashMap::new(),
            fetching_pipelines: std::collections::HashSet::new(),
            loading_tabs: std::collections::HashSet::new(),
            loaded_tabs: std::collections::HashSet::new(),
            edit_menu: None,
            selector: None,
            text_input: None,
            editing_page_size: false,
            page_size_input: String::new(),
            date_picker: None,
            jobs: StatefulTable::with_items(vec![]),
            detail_scroll: 0,
            selected_pipelines: std::collections::HashSet::new(),
            selected_jobs: std::collections::HashSet::new(),
            details_zoomed: false,
            detail_visible: false,
            job_trace_needs_scroll_to_bottom: false,
            job_trace_loading: false,
            job_trace_wrap: false,
            collapse_matrix_jobs: false,
            show_help: false,
            help_search_query: String::new(),
            diff_view: None,
            current_comments: Vec::new(),
            last_fetched_mr_iid: None,
            show_submit_review_prompt: None,
            confirm_popup: None,
            confirm_popup_selected_yes: false,
            diff_loading: false,
            todos: StatefulTable::with_items(vec![]),
            status_message: None,
            refreshed_tabs: std::collections::HashSet::new(),
            tx: None,
            enabled_columns: {
                let mut ec = std::collections::HashMap::new();
                for tab in Tab::ALL {
                    let set: std::collections::HashSet<String> = tab
                        .default_columns(BackendKind::GitLab)
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
            save_menu_open: false,
            save_menu_selection: None,
            page_size: config.page_size,
            milestones: StatefulTable::with_items(vec![]),
            selected_milestone_issues: None,
            selected_milestone_iid: None,
            milestone_issues_cache: std::collections::HashMap::new(),
            terminal_scroll: 0,
            branches: StatefulTable::with_items(vec![]),
            environments: StatefulTable::with_items(vec![]),
            deployments: StatefulTable::with_items(vec![]),
            group_by_column: std::collections::HashMap::new(),
            group_ascending: std::collections::HashMap::new(),
            group_list_state: ratatui::widgets::ListState::default(),
            group_items: Vec::new(),
            column_filters: std::collections::HashMap::new(),
            column_filter_context: None,
            sidebar_rect: None,
            content_rect: None,
            detail_rect: None,
        }
    }
}

impl App {
    pub fn kind(&self) -> BackendKind {
        self.gitlab_client
            .as_ref()
            .map(|c| c.backend.kind())
            .unwrap_or(BackendKind::GitLab)
    }

    pub fn active_table_state_mut(&mut self) -> Option<&mut ratatui::widgets::TableState> {
        match self.active_tab {
            Tab::Issues => Some(&mut self.issues.state),
            Tab::MergeRequests => Some(&mut self.mrs.state),
            Tab::Pipelines => Some(&mut self.pipelines.state),
            Tab::Jobs => Some(&mut self.jobs.state),
            Tab::Runners => Some(&mut self.runners.state),
            Tab::Releases => Some(&mut self.releases.state),
            Tab::Todos => Some(&mut self.todos.state),
            Tab::Milestones => Some(&mut self.milestones.state),
            Tab::Branches => Some(&mut self.branches.state),
            Tab::Environments => Some(&mut self.environments.state),
            Tab::Terminal => None,
        }
    }

    pub fn is_github(&self) -> bool {
        self.kind().is_github()
    }

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
        if self.is_github() {
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

    pub fn available_tabs(&self) -> Vec<Tab> {
        let kind = self.kind();
        let mut tabs: Vec<Tab> = Tab::ALL
            .iter()
            .filter(|t| t.available_on_platform(kind))
            .copied()
            .collect();
        if let Some(disabled) = &self.config.disabled_tabs {
            tabs.retain(|t| !disabled.iter().any(|d| d == &t.title(kind)));
        }
        tabs
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
        if let Some(ref active_tab_str) = app.config.active_tab {
            if let Some(tab) = Tab::from_str(active_tab_str) {
                app.active_tab = tab;
            }
        }
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
                Tab::Branches => &self.config.branches,
                Tab::Environments => &self.config.environments,
                Tab::Terminal => &self.config.terminal,
            };
            if let Some(cols) = &pane.columns {
                let col_set: std::collections::HashSet<String> = cols.iter().cloned().collect();
                self.enabled_columns.insert(tab, col_set);
            }
            if let Some(col) = &pane.group_by_column {
                self.group_by_column.insert(tab, Some(col.clone()));
            } else {
                self.group_by_column.insert(tab, None);
            }
            self.group_ascending.insert(tab, pane.group_ascending);
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
        let tabs = self.available_tabs();
        if tabs.is_empty() {
            return;
        }
        let current_index = tabs.iter().position(|t| t == &self.active_tab).unwrap_or(0);
        let next_index = (current_index + 1) % tabs.len();
        self.active_tab = tabs[next_index];
        self.selected_pipelines.clear();
        self.selected_jobs.clear();
        self.details_zoomed = false;
        self.detail_visible = false;
        self.update_filter_selection();
    }

    pub fn previous_tab(&mut self) {
        let tabs = self.available_tabs();
        if tabs.is_empty() {
            return;
        }
        let current_index = tabs.iter().position(|t| t == &self.active_tab).unwrap_or(0);
        let prev_index = if current_index == 0 {
            tabs.len() - 1
        } else {
            current_index - 1
        };
        self.active_tab = tabs[prev_index];
        self.selected_pipelines.clear();
        self.selected_jobs.clear();
        self.details_zoomed = false;
        self.detail_visible = false;
        self.update_filter_selection();
    }

    pub fn filter_issues_list<'a>(
        items: &'a [crate::domain::issues::Issue],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::domain::issues::Issue> {
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
        items: &'a [crate::domain::issues::Issue],
        query: &str,
        enabled_columns: &std::collections::HashMap<Tab, std::collections::HashSet<String>>,
        ascending: bool,
        group_by_column: &Option<String>,
    ) -> Vec<&'a crate::domain::issues::Issue> {
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

    pub fn filtered_issues(&self) -> Vec<&crate::domain::issues::Issue> {
        let mut list = Self::filtered_issues_list(
            &self.issues.items,
            &self.search_query,
            &self.enabled_columns,
            self.group_ascending
                .get(&Tab::Issues)
                .copied()
                .unwrap_or(true),
            self.group_by_column.get(&Tab::Issues).unwrap_or(&None),
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
        items: &'a [crate::domain::mr::MergeRequest],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::domain::mr::MergeRequest> {
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
        items: &'a [crate::domain::mr::MergeRequest],
        query: &str,
        enabled_columns: &std::collections::HashMap<Tab, std::collections::HashSet<String>>,
        ascending: bool,
        group_by_column: &Option<String>,
    ) -> Vec<&'a crate::domain::mr::MergeRequest> {
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

    pub fn filtered_mrs(&self) -> Vec<&crate::domain::mr::MergeRequest> {
        let mut list = Self::filtered_mrs_list(
            &self.mrs.items,
            &self.search_query,
            &self.enabled_columns,
            self.group_ascending
                .get(&Tab::MergeRequests)
                .copied()
                .unwrap_or(true),
            self.group_by_column
                .get(&Tab::MergeRequests)
                .unwrap_or(&None),
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
        items: &'a [crate::domain::pipelines::Pipeline],
        query: &str,
        pipeline_jobs: &std::collections::HashMap<u64, Vec<crate::domain::pipelines::Job>>,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::domain::pipelines::Pipeline> {
        if query.trim().is_empty() {
            return items.iter().collect();
        }
        let matcher = SkimMatcherV2::default();
        let mut scored_items: Vec<(i64, &crate::domain::pipelines::Pipeline)> = Vec::new();

        for item in items {
            let mut best_score: Option<i64> = None;

            let mut check_match = |text: &str| {
                if let Some(score) = matcher.fuzzy_match(text, query) {
                    if best_score.is_none() || Some(score) > best_score {
                        best_score = Some(score);
                    }
                }
            };

            if enabled_cols.contains("ID") {
                check_match(&format!("#{}", item.id()));
                check_match(&item.id().to_string());
            }
            if enabled_cols.contains("Status") {
                check_match(item.status());
            }
            if enabled_cols.contains("Ref") {
                check_match(item.ref_branch());
            }
            if enabled_cols.contains("Stages") {
                if let Some(jobs) = pipeline_jobs.get(&item.id()) {
                    for job in jobs {
                        check_match(job.name());
                        check_match(job.stage());
                        check_match(job.status());
                    }
                }
            }

            if let Some(score) = best_score {
                scored_items.push((score, item));
            }
        }

        scored_items.sort_by(|a, b| b.0.cmp(&a.0));
        scored_items.into_iter().map(|(_, item)| item).collect()
    }

    pub fn filtered_pipelines_list<'a>(
        items: &'a [crate::domain::pipelines::Pipeline],
        query: &str,
        pipeline_jobs: &std::collections::HashMap<u64, Vec<crate::domain::pipelines::Job>>,
        enabled_columns: &std::collections::HashMap<Tab, std::collections::HashSet<String>>,
        ascending: bool,
        group_by_column: &Option<String>,
    ) -> Vec<&'a crate::domain::pipelines::Pipeline> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = enabled_columns.get(&Tab::Pipelines).unwrap_or(&default_set);
        let mut list = Self::filter_pipelines_list(items, query, pipeline_jobs, enabled_cols);
        if let Some(col) = group_by_column {
            list.sort_by(|a, b| {
                let val_a = match col.as_str() {
                    "Status" => a.status().to_string(),
                    "Ref" => a.ref_branch().to_string(),
                    "ID" => a.id().to_string(),
                    _ => String::new(),
                };
                let val_b = match col.as_str() {
                    "Status" => b.status().to_string(),
                    "Ref" => b.ref_branch().to_string(),
                    "ID" => b.id().to_string(),
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

    pub fn filtered_pipelines(&self) -> Vec<&crate::domain::pipelines::Pipeline> {
        let mut list = Self::filtered_pipelines_list(
            &self.pipelines.items,
            &self.search_query,
            &self.pipeline_jobs,
            &self.enabled_columns,
            self.group_ascending
                .get(&Tab::Pipelines)
                .copied()
                .unwrap_or(true),
            self.group_by_column.get(&Tab::Pipelines).unwrap_or(&None),
        );
        Self::apply_column_filters(
            &mut list,
            &self.column_filters,
            Tab::Pipelines,
            |item, col| match col {
                "ID" => vec![item.id().to_string()],
                "Status" => vec![item.status().to_string()],
                "Ref" => vec![item.ref_branch().to_string()],
                _ => vec![],
            },
        );
        list
    }

    pub fn filter_jobs_list<'a>(
        items: &'a [crate::domain::pipelines::Job],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::domain::pipelines::Job> {
        if query.trim().is_empty() {
            return items.iter().collect();
        }
        let matcher = SkimMatcherV2::default();
        let mut scored_items: Vec<(i64, &crate::domain::pipelines::Job)> = Vec::new();

        for item in items {
            let mut best_score: Option<i64> = None;

            let mut check_match = |text: &str| {
                if let Some(score) = matcher.fuzzy_match(text, query) {
                    if best_score.is_none() || Some(score) > best_score {
                        best_score = Some(score);
                    }
                }
            };

            if enabled_cols.contains("ID") {
                check_match(&item.id().to_string());
            }
            if enabled_cols.contains("Status") {
                check_match(item.status());
            }
            if enabled_cols.contains("Stage") {
                check_match(item.stage());
            }
            if enabled_cols.contains("Name") {
                check_match(item.name());
            }
            if enabled_cols.contains("Matrix") {
                if let Some(matrix) = item.matrix() {
                    check_match(matrix);
                }
            }

            if let Some(score) = best_score {
                scored_items.push((score, item));
            }
        }

        scored_items.sort_by(|a, b| b.0.cmp(&a.0));
        scored_items.into_iter().map(|(_, item)| item).collect()
    }

    pub fn filtered_jobs_list<'a>(
        items: &'a [crate::domain::pipelines::Job],
        query: &str,
        enabled_columns: &std::collections::HashMap<Tab, std::collections::HashSet<String>>,
        ascending: bool,
        group_by_column: &Option<String>,
    ) -> Vec<&'a crate::domain::pipelines::Job> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = enabled_columns.get(&Tab::Jobs).unwrap_or(&default_set);
        let mut list = Self::filter_jobs_list(items, query, enabled_cols);
        if let Some(col) = group_by_column {
            list.sort_by(|a, b| {
                let val_a = match col.as_str() {
                    "Status" => a.status().to_string(),
                    "Stage" => a.stage().to_string(),
                    "Name" => a.name().to_string(),
                    "ID" => a.id().to_string(),
                    _ => String::new(),
                };
                let val_b = match col.as_str() {
                    "Status" => b.status().to_string(),
                    "Stage" => b.stage().to_string(),
                    "Name" => b.name().to_string(),
                    "ID" => b.id().to_string(),
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

    pub fn filtered_jobs(&self) -> Vec<&crate::domain::pipelines::Job> {
        let mut list = Self::filtered_jobs_list(
            &self.jobs.items,
            &self.search_query,
            &self.enabled_columns,
            self.group_ascending
                .get(&Tab::Jobs)
                .copied()
                .unwrap_or(true),
            self.group_by_column.get(&Tab::Jobs).unwrap_or(&None),
        );
        Self::apply_column_filters(
            &mut list,
            &self.column_filters,
            Tab::Jobs,
            |item, col| match col {
                "ID" => vec![item.id().to_string()],
                "Stage" => vec![item.stage().to_string()],
                "Status" => vec![item.status().to_string()],
                "Name" => vec![item.name().to_string()],
                _ => vec![],
            },
        );

        if self.collapse_matrix_jobs {
            let mut collapsed: Vec<&crate::domain::pipelines::Job> = Vec::new();
            let mut seen_names = std::collections::HashSet::new();
            for job in list {
                if seen_names.insert(job.name().to_string()) {
                    collapsed.push(job);
                }
            }
            collapsed
        } else {
            list
        }
    }

    pub fn filter_runners_list<'a>(
        items: &'a [crate::domain::runners::Runner],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::domain::runners::Runner> {
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

    pub fn filtered_runners(&self) -> Vec<&crate::domain::runners::Runner> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = self
            .enabled_columns
            .get(&Tab::Runners)
            .unwrap_or(&default_set);
        let mut list: Vec<&crate::domain::runners::Runner> =
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
        items: &'a [crate::domain::releases::Release],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::domain::releases::Release> {
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

    pub fn filtered_releases_list<'a>(
        items: &'a [crate::domain::releases::Release],
        query: &str,
        enabled_columns: &std::collections::HashMap<Tab, std::collections::HashSet<String>>,
        ascending: bool,
        group_by_column: &Option<String>,
    ) -> Vec<&'a crate::domain::releases::Release> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = enabled_columns.get(&Tab::Releases).unwrap_or(&default_set);
        let mut list = Self::filter_releases_list(items, query, enabled_cols);
        if let Some(col) = group_by_column {
            list.sort_by(|a, b| {
                let val_a = match col.as_str() {
                    "Tag" => a.tag_name.clone(),
                    "Release Name" => a.name.clone(),
                    "Date" => a.released_at.clone(),
                    "Description" => a.description.clone().unwrap_or_default(),
                    "Author" => a.author_name.clone().unwrap_or_default(),
                    _ => String::new(),
                };
                let val_b = match col.as_str() {
                    "Tag" => b.tag_name.clone(),
                    "Release Name" => b.name.clone(),
                    "Date" => b.released_at.clone(),
                    "Description" => b.description.clone().unwrap_or_default(),
                    "Author" => b.author_name.clone().unwrap_or_default(),
                    _ => String::new(),
                };
                let cmp = val_a.cmp(&val_b);
                if !ascending { cmp.reverse() } else { cmp }
            });
        }
        list
    }

    pub fn filtered_releases(&self) -> Vec<&crate::domain::releases::Release> {
        let mut list = Self::filtered_releases_list(
            &self.releases.items,
            &self.search_query,
            &self.enabled_columns,
            self.group_ascending
                .get(&Tab::Releases)
                .copied()
                .unwrap_or(true),
            self.group_by_column.get(&Tab::Releases).unwrap_or(&None),
        );
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
        items: &'a [crate::domain::notifications::Notification],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::domain::notifications::Notification> {
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
        items: &'a [crate::domain::notifications::Notification],
        query: &str,
        enabled_columns: &std::collections::HashMap<Tab, std::collections::HashSet<String>>,
        ascending: bool,
        group_by_column: &Option<String>,
    ) -> Vec<&'a crate::domain::notifications::Notification> {
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

    pub fn filtered_todos(&self) -> Vec<&crate::domain::notifications::Notification> {
        let mut list = Self::filtered_todos_list(
            &self.todos.items,
            &self.search_query,
            &self.enabled_columns,
            self.group_ascending
                .get(&Tab::Todos)
                .copied()
                .unwrap_or(true),
            self.group_by_column.get(&Tab::Todos).unwrap_or(&None),
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
        items: &'a [crate::domain::milestones::Milestone],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::domain::milestones::Milestone> {
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

    pub fn filtered_milestones_list<'a>(
        items: &'a [crate::domain::milestones::Milestone],
        query: &str,
        enabled_columns: &std::collections::HashMap<Tab, std::collections::HashSet<String>>,
        ascending: bool,
        group_by_column: &Option<String>,
        milestone_issues_cache: &std::collections::HashMap<u64, Vec<crate::domain::issues::Issue>>,
    ) -> Vec<&'a crate::domain::milestones::Milestone> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = enabled_columns
            .get(&Tab::Milestones)
            .unwrap_or(&default_set);
        let mut list = Self::filter_milestones_list(items, query, enabled_cols);
        if let Some(col) = group_by_column {
            list.sort_by(|a, b| {
                let val_a = match col.as_str() {
                    "ID" => a.iid.to_string(),
                    "Title" => a.title.clone(),
                    "State" => a.state.clone(),
                    "Start Date" => a.start_date.clone().unwrap_or_default(),
                    "Due Date" => a.due_date.clone().unwrap_or_default(),
                    "Progress" => {
                        if let Some(issues) = milestone_issues_cache.get(&a.iid) {
                            let total = issues.len();
                            if total > 0 {
                                let closed = issues.iter().filter(|i| i.state == "closed").count();
                                let percent = (closed * 100) / total;
                                format!("{:03}%", percent)
                            } else {
                                "000%".to_string()
                            }
                        } else {
                            "000%".to_string()
                        }
                    }
                    _ => String::new(),
                };
                let val_b = match col.as_str() {
                    "ID" => b.iid.to_string(),
                    "Title" => b.title.clone(),
                    "State" => b.state.clone(),
                    "Start Date" => b.start_date.clone().unwrap_or_default(),
                    "Due Date" => b.due_date.clone().unwrap_or_default(),
                    "Progress" => {
                        if let Some(issues) = milestone_issues_cache.get(&b.iid) {
                            let total = issues.len();
                            if total > 0 {
                                let closed = issues.iter().filter(|i| i.state == "closed").count();
                                let percent = (closed * 100) / total;
                                format!("{:03}%", percent)
                            } else {
                                "000%".to_string()
                            }
                        } else {
                            "000%".to_string()
                        }
                    }
                    _ => String::new(),
                };
                let cmp = match (val_a.parse::<u64>(), val_b.parse::<u64>()) {
                    (Ok(a_num), Ok(b_num)) => a_num.cmp(&b_num),
                    _ => val_a.cmp(&val_b),
                };
                if !ascending { cmp.reverse() } else { cmp }
            });
        }
        list
    }

    pub fn filtered_milestones(&self) -> Vec<&crate::domain::milestones::Milestone> {
        let mut list = Self::filtered_milestones_list(
            &self.milestones.items,
            &self.search_query,
            &self.enabled_columns,
            self.group_ascending
                .get(&Tab::Milestones)
                .copied()
                .unwrap_or(true),
            self.group_by_column.get(&Tab::Milestones).unwrap_or(&None),
            &self.milestone_issues_cache,
        );
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

    pub fn filter_branches_list<'a>(
        items: &'a [crate::domain::branches::Branch],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::domain::branches::Branch> {
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
                if enabled_cols.contains("Name") {
                    check_match(&item.name);
                }
                if enabled_cols.contains("SHA") {
                    check_match(&item.commit_sha);
                }
                matches
            })
            .collect()
    }

    pub fn filtered_branches(&self) -> Vec<&crate::domain::branches::Branch> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = self
            .enabled_columns
            .get(&Tab::Branches)
            .unwrap_or(&default_set);
        let mut list =
            Self::filter_branches_list(&self.branches.items, &self.search_query, enabled_cols);
        Self::apply_column_filters(
            &mut list,
            &self.column_filters,
            Tab::Branches,
            |item, col| match col {
                "Name" => vec![item.name.clone()],
                "Default" => vec![item.default.to_string()],
                "Protected" => vec![item.protected.to_string()],
                _ => vec![],
            },
        );
        list
    }

    pub fn filter_environments_list<'a>(
        items: &'a [crate::domain::deployments::Environment],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::domain::deployments::Environment> {
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
                if enabled_cols.contains("Name") {
                    check_match(&item.name);
                }
                if enabled_cols.contains("State") {
                    check_match(&item.state);
                }
                matches
            })
            .collect()
    }

    pub fn filtered_environments(&self) -> Vec<&crate::domain::deployments::Environment> {
        let default_set = std::collections::HashSet::new();
        let enabled_cols = self
            .enabled_columns
            .get(&Tab::Environments)
            .unwrap_or(&default_set);
        let mut list = Self::filter_environments_list(
            &self.environments.items,
            &self.search_query,
            enabled_cols,
        );
        Self::apply_column_filters(
            &mut list,
            &self.column_filters,
            Tab::Environments,
            |item, col| match col {
                "Name" => vec![item.name.clone()],
                "State" => vec![item.state.clone()],
                "Deployment Status" => item
                    .last_deployment
                    .as_ref()
                    .map(|d| vec![d.status.clone()])
                    .unwrap_or_default(),
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
                            values.insert(item.id().to_string());
                        }
                        "Status" => {
                            values.insert(item.status().to_string());
                        }
                        "Ref" => {
                            values.insert(item.ref_branch().to_string());
                        }
                        _ => {} // Pipeline no longer carries GitHub-specific fields
                    }
                }
            }
            Tab::Jobs => {
                for item in &self.jobs.items {
                    match col {
                        "ID" => {
                            values.insert(item.id().to_string());
                        }
                        "Stage" => {
                            values.insert(item.stage().to_string());
                        }
                        "Status" => {
                            values.insert(item.status().to_string());
                        }
                        "Name" => {
                            values.insert(item.name().to_string());
                        }
                        _ => {}
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
            Tab::Branches => {
                for item in &self.branches.items {
                    match col {
                        "Name" => {
                            values.insert(item.name.clone());
                        }
                        "Default" => {
                            values.insert(item.default.to_string());
                        }
                        "Protected" => {
                            values.insert(item.protected.to_string());
                        }
                        _ => {}
                    }
                }
            }
            Tab::Environments => {
                for item in &self.environments.items {
                    match col {
                        "Name" => {
                            values.insert(item.name.clone());
                        }
                        "State" => {
                            values.insert(item.state.clone());
                        }
                        "Deployment Status" => {
                            if let Some(ref d) = item.last_deployment {
                                values.insert(d.status.clone());
                            }
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
        let Some(col) = self
            .group_by_column
            .get(&self.active_tab)
            .cloned()
            .flatten()
        else {
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
                        "Status" => p.status().to_string(),
                        "Ref" => p.ref_branch().to_string(),
                        "ID" => format!("#{}", p.id()),
                        "Name" => format!("#{}", p.id()),
                        "Event" => "Unknown".to_string(),
                        "SHA" => "Unknown".to_string(),
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
                        "Status" => j.status().to_string(),
                        "Stage" => j.stage().to_string(),
                        "Name" => j.name().to_string(),
                        "ID" => format!("#{}", j.id()),
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
                let sel = self.jobs.state.selected();
                if len == 0 {
                    self.jobs.state.select(None);
                } else {
                    match sel {
                        Some(idx) => {
                            if idx >= len {
                                self.jobs.state.select(Some(len - 1));
                            }
                        }
                        None => {
                            self.jobs.state.select(Some(0));
                        }
                    }
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
            Tab::Branches => {
                let len = self.filtered_branches().len();
                let sel = self.branches.state.selected();
                if len == 0 {
                    self.branches.state.select(None);
                } else {
                    match sel {
                        Some(idx) => {
                            if idx >= len {
                                self.branches.state.select(Some(len - 1));
                            }
                        }
                        None => {
                            self.branches.state.select(Some(0));
                        }
                    }
                }
            }
            Tab::Environments => {
                let len = self.filtered_environments().len();
                let sel = self.environments.state.selected();
                if len == 0 {
                    self.environments.state.select(None);
                } else {
                    match sel {
                        Some(idx) => {
                            if idx >= len {
                                self.environments.state.select(Some(len - 1));
                            }
                        }
                        None => {
                            self.environments.state.select(Some(0));
                        }
                    }
                }
            }
            Tab::Terminal => {}
        }
        self.rebuild_group_map();
    }

    pub fn save_layout(&self, target: SaveMenu) {
        let mut cfg = self.config.clone();

        fn sync_pane(
            tab: Tab,
            enabled_columns: &std::collections::HashMap<Tab, std::collections::HashSet<String>>,
            column_filters: &std::collections::HashMap<
                Tab,
                std::collections::HashMap<String, std::collections::HashSet<String>>,
            >,
            group_by_column_map: &std::collections::HashMap<Tab, Option<String>>,
            group_ascending_map: &std::collections::HashMap<Tab, bool>,
            pane: &mut crate::config::PaneConfig,
        ) {
            pane.columns = enabled_columns.get(&tab).map(|set| {
                let mut v: Vec<String> = set.iter().cloned().collect();
                v.sort();
                v
            });
            pane.column_filters = column_filters
                .get(&tab)
                .map(|filters| {
                    filters
                        .iter()
                        .map(|(k, v)| (k.clone(), v.iter().cloned().collect::<Vec<_>>()))
                        .collect()
                })
                .unwrap_or_default();
            pane.group_by_column = group_by_column_map.get(&tab).cloned().flatten();
            pane.group_ascending = group_ascending_map.get(&tab).copied().unwrap_or(true);
        }

        sync_pane(
            Tab::Issues,
            &self.enabled_columns,
            &self.column_filters,
            &self.group_by_column,
            &self.group_ascending,
            &mut cfg.issues,
        );
        sync_pane(
            Tab::MergeRequests,
            &self.enabled_columns,
            &self.column_filters,
            &self.group_by_column,
            &self.group_ascending,
            &mut cfg.mrs,
        );
        sync_pane(
            Tab::Pipelines,
            &self.enabled_columns,
            &self.column_filters,
            &self.group_by_column,
            &self.group_ascending,
            &mut cfg.pipelines,
        );
        sync_pane(
            Tab::Jobs,
            &self.enabled_columns,
            &self.column_filters,
            &self.group_by_column,
            &self.group_ascending,
            &mut cfg.jobs,
        );
        sync_pane(
            Tab::Runners,
            &self.enabled_columns,
            &self.column_filters,
            &self.group_by_column,
            &self.group_ascending,
            &mut cfg.runners,
        );
        sync_pane(
            Tab::Releases,
            &self.enabled_columns,
            &self.column_filters,
            &self.group_by_column,
            &self.group_ascending,
            &mut cfg.releases,
        );
        sync_pane(
            Tab::Todos,
            &self.enabled_columns,
            &self.column_filters,
            &self.group_by_column,
            &self.group_ascending,
            &mut cfg.todos,
        );
        sync_pane(
            Tab::Milestones,
            &self.enabled_columns,
            &self.column_filters,
            &self.group_by_column,
            &self.group_ascending,
            &mut cfg.milestones,
        );
        sync_pane(
            Tab::Branches,
            &self.enabled_columns,
            &self.column_filters,
            &self.group_by_column,
            &self.group_ascending,
            &mut cfg.branches,
        );
        sync_pane(
            Tab::Environments,
            &self.enabled_columns,
            &self.column_filters,
            &self.group_by_column,
            &self.group_ascending,
            &mut cfg.environments,
        );

        if let Err(e) = cfg.save_layout(target) {
            eprintln!("Failed to save layout: {}", e);
        }
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
        use crate::domain::mr::Author;
        use crate::domain::mr::MergeRequest;

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
            head_pipeline: None,
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
            head_pipeline: None,
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
            head_pipeline: None,
        };

        let items = vec![mr_draft_meta, mr_draft_title, mr_ready];
        let enabled_cols: std::collections::HashSet<String> = Tab::MergeRequests
            .columns(BackendKind::GitLab)
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
                fuzzy_indices: None,
            },
            DiffLine {
                content: "-deleted line".to_string(),
                line_type: DiffLineType::Deletion,
                file_path: "foo.txt".to_string(),
                old_line_num: Some(1),
                new_line_num: None,
                syntax_highlighted: None,
                fuzzy_indices: None,
            },
            DiffLine {
                content: "+added line 1".to_string(),
                line_type: DiffLineType::Addition,
                file_path: "foo.txt".to_string(),
                old_line_num: None,
                new_line_num: Some(1),
                syntax_highlighted: None,
                fuzzy_indices: None,
            },
            DiffLine {
                content: "+added line 2".to_string(),
                line_type: DiffLineType::Addition,
                file_path: "foo.txt".to_string(),
                old_line_num: None,
                new_line_num: Some(2),
                syntax_highlighted: None,
                fuzzy_indices: None,
            },
            DiffLine {
                content: " normal line".to_string(),
                line_type: DiffLineType::Normal,
                file_path: "foo.txt".to_string(),
                old_line_num: Some(2),
                new_line_num: Some(3),
                syntax_highlighted: None,
                fuzzy_indices: None,
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
        use crate::domain::mr::{Author, DiscussionNote, NotePosition};

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

    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_save_layout_and_active_tab_and_group_sorting() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().join("config.toml");
        let old_config = std::env::var("GLAB_TUI_CONFIG").ok();
        unsafe {
            std::env::set_var("GLAB_TUI_CONFIG", &config_path);
        }

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

        let mut app = App::new();
        app.active_tab = Tab::MergeRequests;
        app.group_by_column
            .insert(Tab::Issues, Some("Author".to_string()));
        app.group_ascending.insert(Tab::Issues, false);
        app.group_by_column
            .insert(Tab::MergeRequests, Some("State".to_string()));
        app.group_ascending.insert(Tab::MergeRequests, true);
        app.config.theme_preset = Some("tokyo-night".to_string());
        app.config.page_size = 250;

        // Save layout
        app.save_layout(SaveMenu::Global);
        let contents = std::fs::read_to_string(&config_path).unwrap();
        println!("Saved config contents:\n{}", contents);

        // Load new App and verify
        let app2 = App::new();
        assert_eq!(app2.active_tab, Tab::Issues); // active_tab should not be saved/restored
        assert_eq!(app2.config.theme_preset, Some("tokyo-night".to_string()));
        assert_eq!(app2.config.page_size, 250);
        assert_eq!(
            app2.group_by_column.get(&Tab::Issues).cloned().flatten(),
            Some("Author".to_string())
        );
        assert_eq!(app2.group_ascending.get(&Tab::Issues).copied(), Some(false));
        assert_eq!(
            app2.group_by_column
                .get(&Tab::MergeRequests)
                .cloned()
                .flatten(),
            Some("State".to_string())
        );
        assert_eq!(
            app2.group_ascending.get(&Tab::MergeRequests).copied(),
            Some(true)
        );

        std::env::set_current_dir(original_dir).unwrap();
        unsafe {
            if let Some(old) = old_config {
                std::env::set_var("GLAB_TUI_CONFIG", old);
            } else {
                std::env::remove_var("GLAB_TUI_CONFIG");
            }
        }
    }

    #[test]
    fn test_active_tab_to_str_from_str() {
        assert_eq!(Tab::Issues.to_str(), "issues");
        assert_eq!(Tab::from_str("issues"), Some(Tab::Issues));
        assert_eq!(Tab::from_str("mrs"), Some(Tab::MergeRequests));
        assert_eq!(Tab::from_str("mergerequests"), Some(Tab::MergeRequests));
        assert_eq!(Tab::from_str("invalid_tab"), None);
    }

    #[test]
    fn test_app_inline_page_size_defaults() {
        let app = App::new();
        assert!(!app.editing_page_size);
        assert_eq!(app.page_size_input, "");
    }

    #[test]
    fn test_save_layout_preserves_custom_settings() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().join("config.toml");
        let old_config = std::env::var("GLAB_TUI_CONFIG").ok();
        unsafe {
            std::env::set_var("GLAB_TUI_CONFIG", &config_path);
        }

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

        // Write an initial config with custom keybindings
        let initial_toml = r#"
[keybindings.global]
quit = "ctrl+c"
help = "h"
"#;
        std::fs::write(&config_path, initial_toml).unwrap();

        // Load App (which loads this config)
        let mut app = App::new();
        assert_eq!(app.config.keybindings.global.quit, "ctrl+c");
        assert_eq!(app.config.keybindings.global.help, "h");

        // Change layout/page size and save
        app.config.page_size = 456;
        app.save_layout(SaveMenu::Global);

        // Verify the saved file has both the new page_size and the old keybindings!
        let contents = std::fs::read_to_string(&config_path).unwrap();
        println!("Saved config contents:\n{}", contents);

        let val: toml::Value = toml::from_str(&contents).unwrap();
        let table = val.as_table().unwrap();

        // Assert layout changes are saved
        assert_eq!(
            table.get("page_size").and_then(|v| v.as_integer()),
            Some(456)
        );

        // Assert custom keybindings are preserved!
        let kb = table
            .get("keybindings")
            .and_then(|v| v.get("global"))
            .unwrap();
        assert_eq!(kb.get("quit").and_then(|v| v.as_str()), Some("ctrl+c"));
        assert_eq!(kb.get("help").and_then(|v| v.as_str()), Some("h"));

        std::env::set_current_dir(original_dir).unwrap();
        unsafe {
            if let Some(old) = old_config {
                std::env::set_var("GLAB_TUI_CONFIG", old);
            } else {
                std::env::remove_var("GLAB_TUI_CONFIG");
            }
        }
    }

    #[test]
    fn test_diff_view_rename_detection() {
        let diff = "\
diff --git a/src/old_name.rs b/src/new_name.rs
similarity index 85%
rename from src/old_name.rs
rename to src/new_name.rs
--- a/src/old_name.rs
+++ b/src/new_name.rs
@@ -10,6 +10,7 @@
  some content
+new line 1
";
        let view = DiffView::new(42, diff.to_string());
        let files: Vec<&str> = view
            .visible_nodes
            .iter()
            .filter(|n| !n.is_dir)
            .map(|n| n.name.as_str())
            .collect();
        assert_eq!(files, vec!["new_name.rs"]);

        let file_node = view
            .visible_nodes
            .iter()
            .find(|n| n.name == "new_name.rs")
            .unwrap();
        assert_eq!(file_node.old_file_path.as_deref(), Some("src/old_name.rs"));
        assert!(!file_node.is_new_file);
        assert!(!file_node.is_deleted_file);
    }

    #[test]
    fn test_diff_view_new_file_mode() {
        let diff = "\
diff --git a/src/new_module.rs b/src/new_module.rs
new file mode 100644
index 0000000..e69de29
--- /dev/null
+++ b/src/new_module.rs
@@ -0,0 +1,3 @@
+// New file
+fn main() {}
+
";
        let view = DiffView::new(42, diff.to_string());
        let file_node = view
            .visible_nodes
            .iter()
            .find(|n| n.name == "new_module.rs")
            .unwrap();
        assert!(file_node.is_new_file);
        assert!(!file_node.is_deleted_file);
        assert_eq!(file_node.old_file_path, None);
    }

    #[test]
    fn test_diff_view_deleted_file_mode() {
        let diff = "\
diff --git a/src/old_module.rs b/src/old_module.rs
deleted file mode 100644
index e69de29..0000000
--- a/src/old_module.rs
+++ /dev/null
@@ -1,3 +0,0 @@
-// Old file
-fn main() {}
-
";
        let view = DiffView::new(42, diff.to_string());
        let file_node = view
            .visible_nodes
            .iter()
            .find(|n| n.name == "old_module.rs")
            .unwrap();
        assert!(file_node.is_deleted_file);
        assert!(!file_node.is_new_file);
    }

    #[test]
    fn test_diff_view_binary_file_meta() {
        let diff = "\
diff --git a/bin/app b/bin/app
index abcdef..ffffff 100644
Binary files a/bin/app and b/bin/app differ
";
        let view = DiffView::new(42, diff.to_string());
        let meta_line = view
            .all_lines
            .iter()
            .find(|l| l.content.contains("Binary files"));
        assert!(meta_line.is_some());
        assert_eq!(meta_line.unwrap().line_type, DiffLineType::Meta);
    }

    #[test]
    fn test_diff_view_metadata_lines_are_meta() {
        let diff = "\
diff --git a/file.rs b/file.rs
index abcdef..ffffff 100644
old mode 100644
new mode 100755
--- a/file.rs
+++ b/file.rs
@@ -1,3 +1,4 @@
  line1
+line2
";
        let view = DiffView::new(42, diff.to_string());
        let old_mode = view
            .all_lines
            .iter()
            .find(|l| l.content.starts_with("old mode "));
        assert_eq!(old_mode.unwrap().line_type, DiffLineType::Meta);
        let new_mode = view
            .all_lines
            .iter()
            .find(|l| l.content.starts_with("new mode "));
        assert_eq!(new_mode.unwrap().line_type, DiffLineType::Meta);
    }

    #[test]
    fn test_diff_view_file_tree_scroll_offset_default() {
        let diff = "\
diff --git a/foo.txt b/foo.txt
index 123456..789012 100644
--- a/foo.txt
+++ b/foo.txt
@@ -1,1 +1,1 @@
- old
+ new
";
        let view = DiffView::new(42, diff.to_string());
        assert_eq!(view.file_tree_scroll_offset, 0);
    }
}
