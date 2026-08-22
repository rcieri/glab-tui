#![allow(dead_code)]

use crate::backend::BackendKind;
use crate::config::{Config, THEME, Theme};
use crate::domain::workflow_inputs::WorkflowInput;
use crate::utils::format::expand_tabs;
use crate::utils::ui::StatefulTable;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::ListState;
use std::collections::HashSet;
use std::sync::LazyLock;
use syntect::highlighting::Highlighter;
use syntect::highlighting::Style as SyntectStyle;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::parsing::{ParseState, ScopeStack};

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);
/// Single shared fuzzy matcher. Reusing one instance avoids allocating a
/// `SkimMatcherV2` on every filter keystroke (the search bar filters on each
/// key press).
static FUZZY_MATCHER: LazyLock<SkimMatcherV2> = LazyLock::new(SkimMatcherV2::default);

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
///
/// Colors are derived from the active theme's semantic tokens (mapped from each
/// token's syntect scope name) so highlighting always matches the active theme
/// rather than a hardcoded syntect palette. Font modifiers (bold/italic) come
/// from syntect's resolved style.
pub fn highlight_line_syntax(
    file_path: &str,
    line_content: &str,
    ext: Option<&str>,
) -> Option<Vec<(ratatui::style::Style, String)>> {
    let ext = ext.or_else(|| file_extension(file_path))?;
    let syntax = SYNTAX_SET
        .find_syntax_by_extension(ext)
        .or_else(|| SYNTAX_SET.find_syntax_by_extension("txt"))?;

    let theme = THEME.read().unwrap();
    let theme = theme.clone();

    // Remove the leading +/-/space for syntax highlighting, but keep the actual code.
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

    if code.is_empty() {
        return Some(vec![(
            syntect_style_to_ratatui(SyntectStyle::default()),
            code.to_string(),
        )]);
    }

    let mut parse_state = ParseState::new(syntax);
    let ops = parse_state.parse_line(code, &SYNTAX_SET).ok()?;
    if ops.is_empty() {
        return Some(vec![(
            Style::default().fg(theme.text_normal),
            code.to_string(),
        )]);
    }

    let syntax_theme = THEME_SET.themes.values().next()?;
    let highlighter = Highlighter::new(syntax_theme);
    let mut scope_stack = ScopeStack::new();
    let mut result: Vec<(Style, String)> = Vec::new();
    let mut pos = 0usize;
    for (end, op) in ops {
        if pos < end {
            let text = &code[pos..end];
            let top = scope_stack.as_slice().last().copied();
            let style = highlighter.style_for_stack(scope_stack.as_slice());
            let color = top
                .map(|s| scope_color(&format!("{}", s), &theme))
                .unwrap_or(theme.text_normal);
            let mut span_style = Style::default().fg(color);
            if style
                .font_style
                .contains(syntect::highlighting::FontStyle::BOLD)
            {
                span_style = span_style.add_modifier(Modifier::BOLD);
            }
            if style
                .font_style
                .contains(syntect::highlighting::FontStyle::ITALIC)
            {
                span_style = span_style.add_modifier(Modifier::ITALIC);
            }
            if style
                .font_style
                .contains(syntect::highlighting::FontStyle::UNDERLINE)
            {
                span_style = span_style.add_modifier(Modifier::UNDERLINED);
            }
            result.push((span_style, text.to_string()));
        }
        let _ = scope_stack.apply(&op);
        pos = end;
    }
    if pos < code.len() {
        let text = &code[pos..];
        let top = scope_stack.as_slice().last().copied();
        let color = top
            .map(|s| scope_color(&format!("{}", s), &theme))
            .unwrap_or(theme.text_normal);
        result.push((Style::default().fg(color), text.to_string()));
    }

    Some(result)
}

/// Map a syntect scope name (e.g. "keyword.control.rust") to a semantic theme
/// color so syntax highlighting tracks the active theme rather than a hardcoded
/// palette.
fn scope_color(name: &str, theme: &Theme) -> Color {
    if name.contains("comment") {
        theme.text_muted
    } else if name.contains("string")
        || name.contains("regexp")
        || name.contains("character")
        || name.contains("quote")
    {
        theme.green
    } else if name.contains("number") || name.contains("numeric") || name.contains("constant") {
        theme.red
    } else if name.contains("keyword")
        || name.contains("storage")
        || name.contains("control")
        || name.contains("operator")
    {
        theme.purple
    } else if name.contains("function")
        || name.contains("method")
        || name.contains("entity.name")
        || name.contains("decorator")
    {
        theme.yellow
    } else if name.contains("type")
        || name.contains("class")
        || name.contains("struct")
        || name.contains("enum")
        || name.contains("trait")
        || name.contains("interface")
        || name.contains("support.type")
    {
        theme.blue
    } else if name.contains("variable") || name.contains("property") {
        theme.text_normal
    } else if name.contains("tag") || name.contains("markup") {
        theme.blue
    } else {
        theme.text_normal
    }
}

fn syntect_style_to_ratatui(style: SyntectStyle) -> ratatui::style::Style {
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
    ratatui::style::Style::default().add_modifier(modifier)
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

    pub fn is_high_churn(&self) -> bool {
        match self {
            Tab::Issues | Tab::MergeRequests | Tab::Pipelines | Tab::Jobs | Tab::Todos => true,
            Tab::Runners
            | Tab::Releases
            | Tab::Milestones
            | Tab::Branches
            | Tab::Environments
            | Tab::Terminal => false,
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
                    "Mergeable",
                    "Approval",
                    "Title",
                    "Assignees",
                    "Reviewers",
                    "Workflow",
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
                    cols.push("Source");
                    cols.push("Actor");
                }
                cols.push("Created");
                cols.push("Duration");
                cols
            }
            Tab::Jobs => {
                let mut cols = vec!["ID", "Status", "Name", "Matrix"];
                if kind.is_github() {
                    cols.push("Runner");
                    cols.push("Needs");
                } else {
                    cols.push("Stage");
                }
                cols.push("Duration");
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
            Tab::Todos => vec!["State", "Project", "Type", "ID", "Title", "Updated"],
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
            Tab::MergeRequests => vec![
                "ID",
                "State",
                "Status",
                "Mergeable",
                "Approval",
                "Title",
                "Labels",
            ],
            Tab::Pipelines => {
                if kind.is_github() {
                    vec!["Name", "Status", "Event", "Ref", "Created", "Duration"]
                } else {
                    vec![
                        "ID", "Status", "Stages", "Ref", "Source", "Created", "Duration",
                    ]
                }
            }
            Tab::Jobs => {
                if kind.is_github() {
                    vec!["Name", "Status", "Runner", "Duration"]
                } else {
                    vec!["ID", "Stage", "Status", "Name", "Matrix"]
                }
            }
            Tab::Runners => vec!["ID", "Description", "Status", "Active"],
            Tab::Releases => vec!["Tag", "Release Name", "Date"],
            Tab::Todos => vec!["State", "Project", "Type", "ID", "Title", "Updated"],
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

#[derive(Clone, Debug, PartialEq)]
pub enum EditEntityKind {
    CreateIssue,
    EditIssue,
    BulkEditIssues,
    CreateMr,
    EditMr,
    BulkEditMrs,
    CreateMilestone,
    EditMilestone,
    CreateRelease,
    CreatePipeline,
    CreateBranch,
}

impl EditEntityKind {
    pub fn is_create(&self) -> bool {
        matches!(
            self,
            Self::CreateIssue
                | Self::CreateMr
                | Self::CreateMilestone
                | Self::CreateRelease
                | Self::CreatePipeline
                | Self::CreateBranch
                | Self::BulkEditIssues
                | Self::BulkEditMrs
        )
    }

    pub fn needs_submit(&self) -> bool {
        matches!(
            self,
            Self::CreateIssue
                | Self::EditIssue
                | Self::CreateMr
                | Self::EditMr
                | Self::CreateMilestone
                | Self::EditMilestone
                | Self::CreateRelease
                | Self::CreatePipeline
                | Self::CreateBranch
        )
    }

    pub fn entity_name(&self) -> &str {
        match self {
            Self::CreateIssue | Self::EditIssue | Self::BulkEditIssues => "issue",
            Self::CreateMr | Self::EditMr | Self::BulkEditMrs => "mr",
            Self::CreateMilestone | Self::EditMilestone => "milestone",
            Self::CreateRelease => "release",
            Self::CreatePipeline => "pipeline",
            Self::CreateBranch => "branch",
        }
    }

    /// Return the legacy entity_type string for backward compat.
    pub fn legacy_string(&self) -> String {
        match self {
            Self::CreateIssue => "new_issue",
            Self::EditIssue => "issue",
            Self::BulkEditIssues => "new_bulk_edit_issues",
            Self::CreateMr => "new_mr",
            Self::EditMr => "mr",
            Self::BulkEditMrs => "new_bulk_edit_mrs",
            Self::CreateMilestone => "new_milestone",
            Self::EditMilestone => "milestone",
            Self::CreateRelease => "new_release",
            Self::CreatePipeline => "new_pipeline",
            Self::CreateBranch => "new_branch",
        }
        .to_string()
    }
}

#[derive(Clone, Debug)]
pub struct EditMenu {
    pub title: String,
    pub fields: Vec<Field>,
    pub selected_idx: usize,
    pub entity_iid: u64,
    pub entity_kind: EditEntityKind,
    pub state: ListState,
    pub workflow_inputs: Vec<WorkflowInput>,
    pub cursor_pos: usize,
    pub editing: bool,
    pub desc_scroll: u16,
}

impl EditMenu {
    pub fn is_new(&self) -> bool {
        self.entity_kind.needs_submit()
    }

    pub fn get_description_value(&self) -> String {
        self.fields
            .iter()
            .find(|f| f.label == "Description" && f.kind == FieldType::Text)
            .map(|f| f.value.clone())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldType {
    Text,
    MultiSelect,
    Date,
    Toggle,
    Ref,
    Section,
    ReadOnly,
}

#[derive(Clone, Debug)]
pub enum InspectorContent {
    Markdown(String),
    AnsiTrace { trace: String, wrap: bool },
    PipelineStages(Vec<crate::domain::pipelines::Job>),
    Custom(Vec<ratatui::text::Line<'static>>),
    Empty(&'static str),
}

#[derive(Clone, Debug)]
pub struct EntityDocument {
    pub title: String,
    pub fields: Vec<Field>,
    pub content: InspectorContent,
}

/// Semantic tone used to give a preview field a colored background, mirroring
/// the table's tone→color mapping (e.g. Approval/Mergeable columns). `None`
/// means "render with the default field styling".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldTone {
    Muted,
    Red,
    Yellow,
    Green,
    Blue,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub label: String,
    pub kind: FieldType,
    pub value: String,
    pub tone: Option<FieldTone>,
}

impl Field {
    pub fn text(label: &str, value: String) -> Self {
        Self {
            label: label.to_string(),
            kind: FieldType::Text,
            value,
            tone: None,
        }
    }
    pub fn multi_select(label: &str, value: String) -> Self {
        Self {
            label: label.to_string(),
            kind: FieldType::MultiSelect,
            value,
            tone: None,
        }
    }
    pub fn toggle(label: &str, value: String) -> Self {
        Self {
            label: label.to_string(),
            kind: FieldType::Toggle,
            value,
            tone: None,
        }
    }
    pub fn ref_field(label: &str, value: String) -> Self {
        Self {
            label: label.to_string(),
            kind: FieldType::Ref,
            value,
            tone: None,
        }
    }
    pub fn date(label: &str, value: String) -> Self {
        Self {
            label: label.to_string(),
            kind: FieldType::Date,
            value,
            tone: None,
        }
    }
    pub fn section(label: &str) -> Self {
        Self {
            label: label.to_string(),
            kind: FieldType::Section,
            value: String::new(),
            tone: None,
        }
    }
    pub fn read_only(label: &str, value: String) -> Self {
        Self {
            label: label.to_string(),
            kind: FieldType::ReadOnly,
            value,
            tone: None,
        }
    }
    /// Read-only field carrying a semantic `FieldTone` so the inspector renders
    /// it with a tone-driven colored background, matching the table's styling
    /// for the Approval/Mergeable columns.
    pub fn read_only_toned(label: &str, value: String, tone: FieldTone) -> Self {
        Self {
            label: label.to_string(),
            kind: FieldType::ReadOnly,
            value,
            tone: Some(tone),
        }
    }
    pub fn is_editable(&self) -> bool {
        self.kind != FieldType::Section && self.kind != FieldType::ReadOnly
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

        let matcher = &*FUZZY_MATCHER;
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
        self.flatten_ex(depth, prefix, &HashSet::new(), false, out);
    }

    /// Like `flatten`, but review-aware: every node carries `is_reviewed` (for a
    /// directory: every file below it is reviewed), and when `hide_reviewed` is
    /// set, reviewed files — plus directories left with nothing to show — are
    /// dropped from the flattened list.
    pub fn flatten_ex(
        &self,
        depth: usize,
        prefix: &str,
        reviewed: &HashSet<String>,
        hide_reviewed: bool,
        out: &mut Vec<FlatDiffTreeNode>,
    ) {
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
                if hide_reviewed && !self.has_unreviewed_file(reviewed) {
                    return;
                }
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
                        is_reviewed: self.is_fully_reviewed(reviewed),
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
                        child.flatten_ex(
                            if name == "root" { 0 } else { depth + 1 },
                            &path_id,
                            reviewed,
                            hide_reviewed,
                            out,
                        );
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
                let is_reviewed = reviewed.contains(file_path);
                if hide_reviewed && is_reviewed {
                    return;
                }
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
                    is_reviewed,
                });
            }
        }
    }

    /// Collects the paths of every file at or below this node.
    pub fn collect_file_paths(&self, out: &mut Vec<String>) {
        match self {
            DiffTreeNode::Directory { children, .. } => {
                for child in children {
                    child.collect_file_paths(out);
                }
            }
            DiffTreeNode::File { file_path, .. } => out.push(file_path.clone()),
        }
    }

    /// True when at least one file at or below this node is not reviewed.
    /// An empty directory counts as having nothing left to review.
    fn has_unreviewed_file(&self, reviewed: &HashSet<String>) -> bool {
        match self {
            DiffTreeNode::Directory { children, .. } => {
                children.iter().any(|c| c.has_unreviewed_file(reviewed))
            }
            DiffTreeNode::File { file_path, .. } => !reviewed.contains(file_path),
        }
    }

    /// True when this node holds at least one file and all of them are reviewed.
    fn is_fully_reviewed(&self, reviewed: &HashSet<String>) -> bool {
        match self {
            DiffTreeNode::Directory { children, .. } => {
                let mut has_file = false;
                for child in children {
                    match child {
                        DiffTreeNode::File { file_path, .. } => {
                            has_file = true;
                            if !reviewed.contains(file_path) {
                                return false;
                            }
                        }
                        DiffTreeNode::Directory { .. } => {
                            let mut paths = Vec::new();
                            child.collect_file_paths(&mut paths);
                            if !paths.is_empty() {
                                has_file = true;
                                if paths.iter().any(|p| !reviewed.contains(p)) {
                                    return false;
                                }
                            }
                        }
                    }
                }
                has_file
            }
            DiffTreeNode::File { file_path, .. } => reviewed.contains(file_path),
        }
    }

    /// Folds directories that just became fully reviewed, and unfolds those that
    /// stopped being fully reviewed, cascading up through parents. Only
    /// *transitions* between the two review sets are acted on, so a reviewed
    /// directory the user reopened by hand stays open until its state changes.
    fn sync_expansion_to_review(&mut self, before: &HashSet<String>, after: &HashSet<String>) {
        if !matches!(self, DiffTreeNode::Directory { .. }) {
            return;
        }
        let was_reviewed = self.is_fully_reviewed(before);
        let is_reviewed = self.is_fully_reviewed(after);
        if let DiffTreeNode::Directory {
            children,
            is_expanded,
            ..
        } = self
        {
            for child in children.iter_mut() {
                child.sync_expansion_to_review(before, after);
            }
            if is_reviewed && !was_reviewed {
                *is_expanded = false;
            } else if was_reviewed && !is_reviewed {
                *is_expanded = true;
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
    /// File marked as reviewed; on a directory, every file below it is reviewed.
    pub is_reviewed: bool,
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
    /// Cells the line-number column needs, so the separator after it lands in
    /// the same column on every row. Derived from the widest line number in the
    /// diff, never below [`MIN_LINE_NUMBER_WIDTH`].
    pub line_number_width: usize,
    /// Paths of files the user marked as reviewed (`m`), persisted per MR/PR in
    /// the project cache.
    pub reviewed_files: HashSet<String>,
    /// Filter reviewed files out of the tree (`M`).
    pub hide_reviewed: bool,
}

/// Narrowest line-number column, and what any diff under 10 000 lines gets —
/// the width the gutter always had before it was computed at all.
pub const MIN_LINE_NUMBER_WIDTH: usize = 4;

/// Cells needed to print the widest line number in `lines` without pushing the
/// separator that follows it out of column.
///
/// `{:>4}` is a *minimum* width, so a five-digit number silently took a fifth
/// cell and shifted the separator and the whole content of that row one column
/// right of its neighbours. Any diff touching a large file — a generated
/// OpenAPI spec, a lockfile — is full of such rows.
fn line_number_width(lines: &[DiffLine]) -> usize {
    lines
        .iter()
        .flat_map(|line| [line.old_line_num, line.new_line_num])
        .flatten()
        .max()
        .map_or(MIN_LINE_NUMBER_WIDTH, |widest| {
            MIN_LINE_NUMBER_WIDTH.max(widest.to_string().len())
        })
}

/// Columns per tab in the diff view. A literal tab occupies a single cell in
/// the rendered buffer, so a tab-indented file (Go, Makefiles) would otherwise
/// display with no indentation at all.
const DIFF_TAB_WIDTH: usize = 4;

/// Expands the tabs in one diff line, keeping its `+`/`-`/space marker.
///
/// The marker is part of the diff, not of the source line, so tab stops are
/// measured from the character after it. Counting the marker as column 0 would
/// make a single leading tab three spaces wide instead of four, and shift every
/// alignment tab on the line with it.
fn expand_diff_line_tabs(line: &str) -> String {
    match line.chars().next() {
        Some(marker @ ('+' | '-' | ' ')) => {
            let mut out = String::with_capacity(line.len());
            out.push(marker);
            out.push_str(&expand_tabs(&line[1..], DIFF_TAB_WIDTH));
            out
        }
        _ => expand_tabs(line, DIFF_TAB_WIDTH),
    }
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
        // Expand tabs once, here: everything downstream — the stored line
        // content, the syntect highlighting computed from it, the search's
        // fuzzy match indices, the side-by-side pairs — is derived from these
        // strings, so expanding at the single point where they are produced
        // keeps all of them agreeing on where a character sits.
        let cleaned_diff = strip_ansi_escapes(&raw_diff)
            .lines()
            .map(expand_diff_line_tabs)
            .collect::<Vec<_>>()
            .join("\n");
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

        let number_width = line_number_width(&all_lines);

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
            line_number_width: number_width,
            reviewed_files: HashSet::new(),
            hide_reviewed: false,
        };

        view.update_active_lines();
        view
    }

    /// Seeds the reviewed-file marks (restored from the project cache) and the
    /// hide-reviewed filter, then rebuilds the tree around them.
    pub fn restore_review_state(&mut self, reviewed: HashSet<String>, hide_reviewed: bool) {
        // Drop marks for files no longer in the diff so stale paths never leak
        // back into the cache.
        let mut known = Vec::new();
        self.root_node.collect_file_paths(&mut known);
        let known: HashSet<String> = known.into_iter().collect();
        self.reviewed_files = reviewed.into_iter().filter(|p| known.contains(p)).collect();
        self.hide_reviewed = hide_reviewed;
        // Directories already fully reviewed on a previous pass open folded.
        self.root_node
            .sync_expansion_to_review(&HashSet::new(), &self.reviewed_files);
        self.rebuild_visible_nodes_keep_position();
    }

    /// Files covered by the current tree selection: the selected file itself, or
    /// every file below the selected directory.
    pub fn selected_file_paths(&self) -> Vec<String> {
        let Some(node) = self.visible_nodes.get(self.selected_visible_idx) else {
            return Vec::new();
        };
        if !node.is_dir {
            return node.file_path.clone().into_iter().collect();
        }
        let rel_path = node.path_id.strip_prefix("root/").unwrap_or(&node.path_id);
        let prefix1 = format!("{}/", rel_path);
        let prefix2 = format!("{}\\", rel_path);
        let mut paths = Vec::new();
        self.root_node.collect_file_paths(&mut paths);
        paths.retain(|p| p.starts_with(&prefix1) || p.starts_with(&prefix2));
        paths
    }

    /// Marks or unmarks the current selection. A directory flips as a whole:
    /// fully reviewed → unmark everything, otherwise mark everything.
    /// Returns the affected file count and the new state.
    pub fn toggle_reviewed(&mut self) -> Option<(usize, bool)> {
        let paths = self.selected_file_paths();
        if paths.is_empty() {
            return None;
        }
        let mark = !paths.iter().all(|p| self.reviewed_files.contains(p));
        let before = self.reviewed_files.clone();
        for path in &paths {
            if mark {
                self.reviewed_files.insert(path.clone());
            } else {
                self.reviewed_files.remove(path);
            }
        }
        self.root_node
            .sync_expansion_to_review(&before, &self.reviewed_files);
        self.rebuild_visible_nodes_keep_position();
        Some((paths.len(), mark))
    }

    /// Toggles the "hide reviewed files" tree filter.
    pub fn toggle_hide_reviewed(&mut self) -> bool {
        self.hide_reviewed = !self.hide_reviewed;
        self.rebuild_visible_nodes_keep_position();
        self.hide_reviewed
    }

    /// (reviewed, total) file counts for the whole diff, ignoring the filter.
    pub fn review_progress(&self) -> (usize, usize) {
        let mut paths = Vec::new();
        self.root_node.collect_file_paths(&mut paths);
        let reviewed = paths
            .iter()
            .filter(|p| self.reviewed_files.contains(*p))
            .count();
        (reviewed, paths.len())
    }

    /// Rebuilds the tree after a review-state change. Unlike
    /// `rebuild_visible_nodes`, a selection that got filtered out falls back to
    /// the node that took its place instead of jumping back to the top.
    fn rebuild_visible_nodes_keep_position(&mut self) {
        let old_path = self
            .visible_nodes
            .get(self.selected_visible_idx)
            .map(|n| n.path_id.clone());
        let old_idx = self.selected_visible_idx;

        let mut visible = Vec::new();
        self.root_node.flatten_ex(
            0,
            "",
            &self.reviewed_files,
            self.hide_reviewed,
            &mut visible,
        );
        Self::copy_dir_counts_to_flat(&self.root_node, &mut visible);
        self.visible_nodes = visible;

        if self.visible_nodes.is_empty() {
            self.selected_visible_idx = 0;
            self.file_tree_scroll_offset = 0;
            self.cursor_idx = 0;
            self.scroll_offset = 0;
            self.update_active_lines();
            return;
        }

        if let Some(pos) = old_path
            .as_deref()
            .and_then(|p| self.visible_nodes.iter().position(|n| n.path_id == p))
        {
            self.selected_visible_idx = pos;
            self.update_active_lines();
            return;
        }

        // The selection is gone: it either folded into a parent that just became
        // fully reviewed, or the filter hid it. Follow the fold onto that parent
        // when there is one; otherwise hold the cursor slot so the next pending
        // file slides under it.
        let ancestor = old_path.as_deref().and_then(|old| {
            let mut path = old;
            while let Some((parent, _)) = path.rsplit_once('/') {
                if let Some(pos) = self
                    .visible_nodes
                    .iter()
                    .position(|n| n.path_id == parent && n.is_dir && !n.is_expanded)
                {
                    return Some(pos);
                }
                path = parent;
            }
            None
        });
        self.selected_visible_idx = ancestor.unwrap_or(old_idx.min(self.visible_nodes.len() - 1));
        self.cursor_idx = 0;
        self.scroll_offset = 0;
        self.update_active_lines();
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
        self.root_node.flatten_ex(
            0,
            "",
            &self.reviewed_files,
            self.hide_reviewed,
            &mut visible,
        );
        Self::copy_dir_counts_to_flat(&self.root_node, &mut visible);
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

        let matcher = &*FUZZY_MATCHER;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    EditMenu,
    Selector,
    DatePicker,
    Help,
    Configure,
    SaveMenu,
    ColumnFilter,
    ConfirmPopup,
}

#[derive(Clone, Debug)]
pub enum ConfirmAction {
    DeleteMilestone(u64),   // milestone iid
    DeleteRelease(String),  // release tag_name
    DeleteBranch(String),   // branch name
    DeleteIssue(u64),       // issue iid
    DeleteMr(u64),          // mr iid
    CloseIssue(u64),        // issue iid
    CloseMr(u64),           // mr iid
    MergeMr(u64),           // mr iid
    BulkMergeMrs(Vec<u64>), // mr iids (multiple selected)
    RevokeMr(u64),          // mr iid
    RebaseMr(u64),          // mr iid
    SubmitReview(u64),      // mr iid
}

/// Toggleable option shown inside a [`SubmitDialog`] body.
///
/// Used for actions that have additional knobs (merge options such as
/// "Squash commits" or "Delete source branch"). The user flips these
/// with `Space` / `Enter` before pressing the Submit button; nothing is
/// sent to the API until Submit is activated.
#[derive(Clone, Debug)]
pub struct SubmitOption {
    pub label: String,
    pub checked: bool,
}

impl SubmitOption {
    pub fn new(label: impl Into<String>, checked: bool) -> Self {
        Self {
            label: label.into(),
            checked,
        }
    }
}

/// A modal dialog that gates a mutating API call behind an explicit
/// Submit button click. Replaces the old two-button YES/NO popup
/// (issue #315).
///
/// Layout:
///
/// ```text
/// ┌─ <title> ─────────────────────┐
/// │                                │
/// │ <body, wrapped>                │
/// │                                │
/// │ [ ] <option label>             │   ← only if options is non-empty
/// │ [x] <option label>             │
/// │                                │
/// ├────────────────────────────────┤
/// │  [ Cancel ]      [ Submit ]    │
/// └────────────────────────────────┘
/// ```
///
/// Cursor layout: index `0` is Cancel, `1..=options.len()` are the
/// options (one per toggle), `options.len() + 1` is Submit.
///
/// Keyboard:
/// - `Tab` / `l` / `→` — next row
/// - `Shift-Tab` / `h` / `←` — previous row
/// - `Enter` — activate focused (Cancel closes, option toggles, Submit runs the action)
/// - `Space` — toggle the focused option (no-op on buttons)
/// - `Esc` — cancel (equivalent to Cancel)
#[derive(Clone, Debug)]
pub struct SubmitDialog {
    pub action: ConfirmAction,
    pub title: String,
    pub body: String,
    pub options: Vec<SubmitOption>,
    pub submit_label: String,
    /// `0` = Cancel, `1..=options.len()` = options, `options.len() + 1` = Submit.
    pub cursor_idx: usize,
}

impl SubmitDialog {
    /// Leftmost position: the Submit button (rendered on the left).
    pub const SUBMIT_IDX: usize = 0;

    /// Index of the Cancel button (rendered on the right, after the options).
    pub fn cancel_idx(&self) -> usize {
        self.options.len() + 1
    }

    pub fn is_on_submit(&self) -> bool {
        self.cursor_idx == Self::SUBMIT_IDX
    }

    pub fn is_on_cancel(&self) -> bool {
        self.cursor_idx == self.cancel_idx()
    }

    /// `None` when on a button, `Some(i)` when on the `i`-th option.
    pub fn option_idx(&self) -> Option<usize> {
        if self.cursor_idx >= 1 && self.cursor_idx <= self.options.len() {
            Some(self.cursor_idx - 1)
        } else {
            None
        }
    }

    /// Construct a SubmitDialog that defaults its cursor to Submit (left).
    /// Used for reversible actions (merge, rebase, submit review).
    pub fn new(
        action: ConfirmAction,
        title: impl Into<String>,
        body: impl Into<String>,
        submit_label: impl Into<String>,
        options: Vec<SubmitOption>,
    ) -> Self {
        Self {
            action,
            title: title.into(),
            body: body.into(),
            options,
            submit_label: submit_label.into(),
            cursor_idx: Self::SUBMIT_IDX,
        }
    }

    /// Construct a SubmitDialog that defaults its cursor to Cancel (right).
    /// Used for destructive actions where the safer default is opt-in.
    pub fn new_safe(
        action: ConfirmAction,
        title: impl Into<String>,
        body: impl Into<String>,
        submit_label: impl Into<String>,
        options: Vec<SubmitOption>,
    ) -> Self {
        let options_len = options.len();
        Self {
            action,
            title: title.into(),
            body: body.into(),
            options,
            submit_label: submit_label.into(),
            cursor_idx: options_len + 1,
        }
    }

    pub fn move_next(&mut self) {
        let max = self.cancel_idx();
        if self.cursor_idx < max {
            self.cursor_idx += 1;
        }
    }

    pub fn move_prev(&mut self) {
        if self.cursor_idx > Self::SUBMIT_IDX {
            self.cursor_idx -= 1;
        }
    }

    pub fn toggle_focused_option(&mut self) {
        if let Some(idx) = self.option_idx() {
            self.options[idx].checked = !self.options[idx].checked;
        }
    }

    /// Build the default SubmitDialog for a [`ConfirmAction`], pulling
    /// any context-aware body text from the live `App` (e.g. the merge
    /// target branch, the release tag, the issue/MR iid).
    ///
    /// Destructive actions default the cursor to Cancel; reversible
    /// actions (merge, rebase, submit review) default to Submit.
    pub fn build(action: ConfirmAction, app: &App) -> Self {
        let kind = app.kind();
        let mr = kind.term("mr");
        let mr_short = kind.term("mr_short");
        let marker = if kind.is_github() { "#" } else { "!" };

        let action_clone = action.clone();
        let (title, body, submit_label, options, safe) = match &action {
            ConfirmAction::DeleteMilestone(iid) => (
                format!("Delete Milestone #{iid}"),
                format!("Are you sure you want to delete milestone #{iid}?"),
                "Delete".to_string(),
                vec![],
                true,
            ),
            ConfirmAction::DeleteRelease(tag_name) => (
                format!("Delete Release {tag_name}"),
                format!("Are you sure you want to delete release {tag_name}?"),
                "Delete".to_string(),
                vec![],
                true,
            ),
            ConfirmAction::DeleteBranch(branch_name) => (
                format!("Delete Branch '{branch_name}'"),
                format!("Are you sure you want to delete branch \'{branch_name}\'?"),
                "Delete".to_string(),
                vec![],
                true,
            ),
            ConfirmAction::CloseIssue(iid) => (
                format!("Close Issue #{iid}"),
                format!("Are you sure you want to close issue #{iid}?"),
                "Close".to_string(),
                vec![],
                true,
            ),
            ConfirmAction::DeleteIssue(iid) => (
                format!("Delete Issue #{iid}"),
                format!("Are you sure you want to delete issue #{iid}? This action is permanent."),
                "Delete".to_string(),
                vec![],
                true,
            ),
            ConfirmAction::CloseMr(iid) => (
                format!("Close {mr} #{iid}"),
                format!("Are you sure you want to close {mr_short} #{iid}?"),
                "Close".to_string(),
                vec![],
                true,
            ),
            ConfirmAction::DeleteMr(iid) => (
                format!("Delete {mr} #{iid}"),
                format!(
                    "Are you sure you want to delete {mr_short} #{iid}? This action is permanent."
                ),
                "Delete".to_string(),
                vec![],
                true,
            ),
            ConfirmAction::MergeMr(iid) => {
                let (source, target) = app
                    .mrs
                    .items
                    .iter()
                    .find(|m| m.iid == *iid)
                    .map(|m| (m.source_branch.clone(), m.target_branch.clone()))
                    .unwrap_or_default();
                let branch_flow = if source.is_empty() && target.is_empty() {
                    String::new()
                } else {
                    format!("\n\n{marker}{iid}: {source} \u{2192} {target}")
                };
                let options = vec![
                    SubmitOption::new("Squash", true),
                    SubmitOption::new("Delete source branch", true),
                ];
                (
                    format!("Merge {mr} #{iid}"),
                    format!(
                        "Merge {mr_short} {marker}{iid} with the selected options.{branch_flow}"
                    ),
                    "Merge".to_string(),
                    options,
                    false,
                )
            }
            ConfirmAction::BulkMergeMrs(iids) => (
                format!("Merge {} {}s", iids.len(), mr),
                format!(
                    "Merge {} {}s with the selected options.",
                    iids.len(),
                    mr_short
                ),
                "Merge".to_string(),
                vec![
                    SubmitOption::new("Squash", true),
                    SubmitOption::new("Delete source branch", true),
                ],
                false,
            ),
            ConfirmAction::RevokeMr(iid) => (
                "Revoke Approval".to_string(),
                format!("Are you sure you want to revoke your approval on {mr_short} #{iid}?"),
                "Revoke".to_string(),
                vec![],
                true,
            ),
            ConfirmAction::RebaseMr(iid) => {
                let target = app
                    .mrs
                    .items
                    .iter()
                    .find(|m| m.iid == *iid)
                    .map(|m| m.target_branch.clone())
                    .unwrap_or_else(|| "target".to_string());
                (
                    format!("Rebase {mr_short} {marker}{iid}"),
                    format!(
                        "Rebase {marker}{iid} onto {target}?\n\nThis rewrites the commit history."
                    ),
                    "Rebase".to_string(),
                    vec![],
                    false,
                )
            }
            ConfirmAction::SubmitReview(iid) => (
                "Submit Review".to_string(),
                format!(
                    "You have pending draft comments on {mr_short} #{iid}.\nSubmit your review now?"
                ),
                "Submit".to_string(),
                vec![],
                false,
            ),
        };

        let cursor_idx = if safe {
            options.len() + 1
        } else {
            Self::SUBMIT_IDX
        };
        Self {
            action: action_clone,
            title,
            body,
            options,
            submit_label,
            cursor_idx,
        }
    }
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
    pub pending_pipeline_select: Option<u64>,
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
    pub selected_issues: std::collections::HashSet<u64>,
    pub selected_mrs: std::collections::HashSet<u64>,
    /// When true, moving the cursor through Issues/MRs marks each visited
    /// item into the selection set (yazi-style "select mode"). `Space` still
    /// toggles the current item individually regardless of this flag.
    pub select_mode: bool,
    pub details_zoomed: bool,
    /// Captured when the edit menu opens so Esc can restore the previous
    /// zoom state (e.g. back to the zoomed PREVIEW if the user entered
    /// edit via double-Enter, back to NORMAL if they entered via `e`).
    pub prev_details_zoomed: bool,
    pub detail_visible: bool,
    pub job_trace_needs_scroll_to_bottom: bool,
    pub job_trace_loading: bool,
    pub job_trace_wrap: bool,
    pub job_trace_search_query: String,
    pub job_trace_searching: bool,
    pub job_trace_follow: bool,
    pub job_trace_last_refresh: std::time::Instant,

    pub show_help: bool,
    pub help_search_query: String,
    pub diff_view: Option<DiffView>,
    pub current_comments: Vec<crate::domain::mr::DiscussionNote>,
    pub last_fetched_mr_iid: Option<u64>,

    pub submit_dialog: Option<SubmitDialog>,
    pub diff_loading: bool,
    pub todos: StatefulTable<crate::domain::notifications::Notification>,
    pub status_message: Option<String>,
    pub refreshed_tabs: std::collections::HashSet<Tab>,
    pub tx: Option<tokio::sync::mpsc::UnboundedSender<crate::event::Event>>,
    pub enabled_columns: std::collections::HashMap<Tab, std::collections::HashSet<String>>,
    pub focus_column_checklist: bool,
    pub column_checklist_idx: usize,
    pub in_review_mode: bool,
    /// Session-wide "hide reviewed files" preference for the diff file tree.
    pub hide_reviewed_files: bool,
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
    pub overlay_stack: Vec<(OverlayKind, Rect)>,
    pub cached_labels: Vec<String>,
    /// Real per-label colors fetched from the API (name → color).
    pub label_colors: std::collections::HashMap<String, ratatui::style::Color>,
    pub cached_members: Vec<String>,
    pub last_attr_refresh: std::time::Instant,
    pub pending_delete_milestone_iid: Option<u64>,
    pub pending_delete_release_tag: Option<String>,
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
            pending_pipeline_select: None,
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
            selected_issues: std::collections::HashSet::new(),
            selected_mrs: std::collections::HashSet::new(),
            select_mode: false,
            details_zoomed: false,
            prev_details_zoomed: false,
            detail_visible: false,
            job_trace_needs_scroll_to_bottom: false,
            job_trace_loading: false,
            job_trace_wrap: false,
            job_trace_search_query: String::new(),
            job_trace_searching: false,
            job_trace_follow: false,
            job_trace_last_refresh: std::time::Instant::now(),

            show_help: false,
            help_search_query: String::new(),
            diff_view: None,
            current_comments: Vec::new(),
            last_fetched_mr_iid: None,
            submit_dialog: None,
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
            hide_reviewed_files: false,
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
            overlay_stack: vec![],
            cached_labels: Vec::new(),
            label_colors: std::collections::HashMap::new(),
            cached_members: Vec::new(),
            last_attr_refresh: std::time::Instant::now(),
            pending_delete_milestone_iid: None,
            pending_delete_release_tag: None,
        }
    }
}

impl App {
    /// Open an edit/create menu, capturing the current zoom state so Esc
    /// can restore it. Use this helper instead of assigning to
    /// `edit_menu` directly so the prev-details-zoomed bookkeeping is
    /// consistent across all entry points (double-Enter, `e`, etc.).
    pub fn open_edit_menu(&mut self, menu: EditMenu) {
        self.prev_details_zoomed = self.details_zoomed;
        self.edit_menu = Some(menu);
    }

    /// Single entry point for surfacing a runtime error. Sets the transient
    /// error toast (`error_message`) and marks the failed terminal command so
    /// both UI surfaces stay in sync.
    ///
    /// The failed command is matched with the same two-tier preference the
    /// `CommandCompleted` handler used before this helper: a still-running
    /// command whose description names the underlying CLI (`glab`/`gh`) or a
    /// bulk/submit operation takes precedence, falling back to the most
    /// recent running command.
    pub fn show_error(&mut self, msg: String) {
        self.error_message_at = Some(std::time::Instant::now());
        let failed_status = format!("Failed: {}", msg);
        self.error_message = Some(msg);
        let pos = self
            .terminal_commands
            .iter()
            .rposition(|cmd| {
                (cmd.command.contains("glab")
                    || cmd.command.contains("gh")
                    || cmd.command.contains("submit")
                    || cmd.command.contains("bulk"))
                    && cmd.status == "Running"
            })
            .or_else(|| {
                self.terminal_commands
                    .iter()
                    .rposition(|cmd| cmd.status == "Running")
            });
        if let Some(pos) = pos {
            self.terminal_commands[pos].status = failed_status;
        }
    }

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

    /// Files marked as reviewed for an MR/PR, restored from the project cache.
    pub fn reviewed_files_for_mr(&self, mr_iid: u64) -> HashSet<String> {
        self.project_cache
            .reviewed_files
            .get(&mr_iid)
            .map(|paths| paths.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Writes the reviewed-file marks of an MR/PR back into the project cache.
    /// The caller persists the cache to disk.
    pub fn store_reviewed_files_for_mr(&mut self, mr_iid: u64, reviewed: &HashSet<String>) {
        if reviewed.is_empty() {
            self.project_cache.reviewed_files.remove(&mr_iid);
            return;
        }
        let mut paths: Vec<String> = reviewed.iter().cloned().collect();
        paths.sort();
        self.project_cache.reviewed_files.insert(mr_iid, paths);
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
        self.selected_issues.clear();
        self.selected_mrs.clear();
        self.select_mode = false;
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
        self.selected_issues.clear();
        self.selected_mrs.clear();
        self.select_mode = false;
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
        let matcher = &*FUZZY_MATCHER;
        let q = query.trim();
        items
            .iter()
            .filter(|item| {
                let mut matches = false;
                let mut check_match = |text: &str| {
                    if matcher.fuzzy_match(text, q).is_some() {
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
                "State" => vec![if item.state == "opened" {
                    "OPEN".to_string()
                } else {
                    "CLOSED".to_string()
                }],
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
        let matcher = &*FUZZY_MATCHER;
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
            if enabled_cols.contains("Approval") {
                for value in Self::mr_filter_values(item, "Approval") {
                    check_match(&value);
                }
            }
            if enabled_cols.contains("Mergeable") {
                for value in Self::mr_filter_values(item, "Mergeable") {
                    check_match(&value);
                }
            }
            if enabled_cols.contains("Workflow") {
                for v in Self::mr_filter_values(item, "Workflow") {
                    check_match(&v);
                }
            }

            if let Some(score) = best_score {
                scored_items.push((item, score));
            }
        }

        scored_items.sort_by(|a, b| b.1.cmp(&a.1));
        scored_items.into_iter().map(|(item, _)| item).collect()
    }

    /// The string a column contributes to MR sorting.
    ///
    /// One function for both sides of the comparator — previously this `match`
    /// was duplicated across `val_a` and `val_b`, so every new column had to be
    /// added twice. Values are strings because the comparator falls back to
    /// numeric ordering when both sides parse as `u64`.
    fn mr_sort_value(m: &crate::domain::mr::MergeRequest, col: &str) -> String {
        match col {
            "State" => m.state.clone(),
            "Author" => m.author.username.clone(),
            "Labels" => m.labels.first().cloned().unwrap_or_default(),
            "Milestone" => m
                .milestone
                .as_ref()
                .map(|ms| ms.title.clone())
                .unwrap_or_default(),
            "Assignees" => m
                .assignees
                .first()
                .map(|asg| asg.username.clone())
                .unwrap_or_default(),
            "Reviewers" => m
                .reviewers
                .first()
                .map(|rev| rev.username.clone())
                .unwrap_or_default(),
            "Status" => {
                if m.draft {
                    "Draft".to_string()
                } else {
                    "Ready".to_string()
                }
            }
            "ID" => m.iid.to_string(),
            "Title" => m.title.clone(),
            "Approval" => {
                crate::domain::mr_state::approval_sort_key(m.approval.as_ref()).to_string()
            }
            "Mergeable" => {
                crate::domain::mr_state::mergeable_sort_key(m.mergeability.as_ref()).to_string()
            }
            "Workflow" => crate::domain::mr_state::workflow_sort_key(m.workflow).to_string(),
            _ => String::new(),
        }
    }

    /// The filter values a column contributes for one MR.
    ///
    /// Returns several values when an MR carries more than one independent fact —
    /// a draft MR with unresolved discussions yields both "Draft" and
    /// "Unresolved discussions", so each is independently filterable.
    ///
    /// One function for BOTH filter call sites: `filtered_mrs` (which decides
    /// whether an MR matches an active filter) and `collect_unique_column_values`
    /// (which populates the picker's selectable options). They previously drifted,
    /// leaving values that matched but could never be selected.
    fn mr_filter_values(m: &crate::domain::mr::MergeRequest, col: &str) -> Vec<String> {
        match col {
            "Labels" => m.labels.clone(),
            "Assignees" => m.assignees.iter().map(|a| a.username.clone()).collect(),
            "Reviewers" => m.reviewers.iter().map(|r| r.username.clone()).collect(),
            "Author" => vec![m.author.username.clone()],
            "Milestone" => m
                .milestone
                .as_ref()
                .map(|ms| ms.title.clone())
                .into_iter()
                .collect(),
            "State" => vec![if m.state == "opened" {
                "OPEN".to_string()
            } else if m.state == "merged" {
                "MERGED".to_string()
            } else {
                "CLOSED".to_string()
            }],
            "Status" => crate::domain::mr_state::status_filter_values(
                m.draft,
                m.blocking_discussions_resolved,
            ),
            "ID" => vec![m.iid.to_string()],
            "Title" => vec![m.title.clone()],
            "Approval" => {
                vec![
                    match crate::domain::mr_state::approval_cell(m.approval.as_ref(), false).1 {
                        crate::domain::mr_state::ApprovalTone::Unknown => "—",
                        crate::domain::mr_state::ApprovalTone::ChangesRequested => "CHG",
                        crate::domain::mr_state::ApprovalTone::AwaitingYou => "AWAITING",
                        crate::domain::mr_state::ApprovalTone::Approved => "APPROVED",
                        crate::domain::mr_state::ApprovalTone::Pending => "REVIEW REQ",
                    }
                    .to_string(),
                ]
            }
            "Mergeable" => {
                vec![
                    match crate::domain::mr_state::mergeable_cell(m.mergeability.as_ref()).1 {
                        crate::domain::mr_state::MergeTone::Unknown => "—",
                        crate::domain::mr_state::MergeTone::Conflict => "CONFLICT",
                        crate::domain::mr_state::MergeTone::Rebase => "REBASE",
                        crate::domain::mr_state::MergeTone::Computing => "CHECKING",
                        crate::domain::mr_state::MergeTone::Clean => "CLEAN",
                    }
                    .to_string(),
                ]
            }
            "Workflow" => crate::domain::mr_state::workflow_cell_word(m.workflow)
                .map(|w| vec![w.to_string()])
                .unwrap_or_default(),
            _ => vec![],
        }
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
                let val_a = Self::mr_sort_value(a, col.as_str());
                let val_b = Self::mr_sort_value(b, col.as_str());
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
            |item, col| Self::mr_filter_values(item, col),
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
        let matcher = &*FUZZY_MATCHER;
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
                // The cell shows `format_ref` ("MR !2208"), not the raw
                // `refs/merge-requests/2208/head`. Matching only the raw ref
                // makes a visible row unfindable by what is on screen.
                let display_ref = crate::utils::format::format_ref(item.ref_branch());
                if display_ref != item.ref_branch() {
                    check_match(&display_ref);
                }
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
            if enabled_cols.contains("Name") {
                check_match(item.name());
            }
            if enabled_cols.contains("Event") {
                check_match(item.event());
            }
            if enabled_cols.contains("SHA") {
                check_match(item.head_sha());
            }
            if enabled_cols.contains("Actor") {
                check_match(item.actor_login());
            }
            if enabled_cols.contains("Created") {
                if let Some(created) = item.created_at() {
                    check_match(&crate::utils::format::time_ago(created));
                }
            }
            if enabled_cols.contains("Source") {
                if let Some(source) = item.source() {
                    check_match(source);
                }
            }
            if enabled_cols.contains("Duration") {
                if let Some(duration) = item.duration_seconds() {
                    check_match(&format!("{}m {}s", duration / 60, duration % 60));
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
                    "Name" => a.name().to_string(),
                    "Event" => a.event().to_string(),
                    "SHA" => a.head_sha().to_string(),
                    "Actor" => a.actor_login().to_string(),
                    "Created" => a
                        .created_at()
                        .map(|c| crate::utils::format::time_ago(c))
                        .unwrap_or_default(),
                    "Source" => a.source().unwrap_or_default().to_string(),
                    "Duration" => a
                        .duration_seconds()
                        .map(|d| d.to_string())
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                let val_b = match col.as_str() {
                    "Status" => b.status().to_string(),
                    "Ref" => b.ref_branch().to_string(),
                    "ID" => b.id().to_string(),
                    "Name" => b.name().to_string(),
                    "Event" => b.event().to_string(),
                    "SHA" => b.head_sha().to_string(),
                    "Actor" => b.actor_login().to_string(),
                    "Created" => b
                        .created_at()
                        .map(|c| crate::utils::format::time_ago(c))
                        .unwrap_or_default(),
                    "Source" => b.source().unwrap_or_default().to_string(),
                    "Duration" => b
                        .duration_seconds()
                        .map(|d| d.to_string())
                        .unwrap_or_default(),
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
            Self::pipeline_filter_values,
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
        let matcher = &*FUZZY_MATCHER;
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
            if enabled_cols.contains("Runner") {
                if let Some(runner) = item.runner() {
                    check_match(runner);
                }
            }
            if enabled_cols.contains("Needs") {
                for need in item.needs() {
                    check_match(need);
                }
            }
            if enabled_cols.contains("Duration") {
                if let Some(dur) = item.duration_seconds() {
                    check_match(&format!("{}m {}s", dur / 60, dur % 60));
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
                    "Runner" => a.runner().unwrap_or("-").to_string(),
                    "Duration" => a
                        .duration_seconds()
                        .map(|d| format!("{}m", d / 60))
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                let val_b = match col.as_str() {
                    "Status" => b.status().to_string(),
                    "Stage" => b.stage().to_string(),
                    "Name" => b.name().to_string(),
                    "ID" => b.id().to_string(),
                    "Runner" => b.runner().unwrap_or("-").to_string(),
                    "Duration" => b
                        .duration_seconds()
                        .map(|d| format!("{}m", d / 60))
                        .unwrap_or_default(),
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
                "Status" => vec![Self::pipeline_status_display(item.status()).to_string()],
                "Name" => vec![item.name().to_string()],
                _ => vec![],
            },
        );

        list
    }

    pub fn filter_runners_list<'a>(
        items: &'a [crate::domain::runners::Runner],
        query: &str,
        enabled_cols: &std::collections::HashSet<String>,
    ) -> Vec<&'a crate::domain::runners::Runner> {
        if query.trim().is_empty() {
            return items.iter().collect();
        }
        let matcher = &*FUZZY_MATCHER;
        let q = query.trim();
        items
            .iter()
            .filter(|item| {
                let mut matches = false;
                let mut check_match = |text: &str| {
                    if matcher.fuzzy_match(text, q).is_some() {
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
        let matcher = &*FUZZY_MATCHER;
        let q = query.trim();
        items
            .iter()
            .filter(|item| {
                let mut matches = false;
                let mut check_match = |text: &str| {
                    if matcher.fuzzy_match(text, q).is_some() {
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
        let matcher = &*FUZZY_MATCHER;
        let mut scored: Vec<(i64, &'a crate::domain::notifications::Notification)> = items
            .iter()
            .filter_map(|item| {
                let mut best: i64 = i64::MIN;
                let mut try_match = |text: &str| {
                    if let Some((score, _)) = matcher.fuzzy_indices(text, query) {
                        best = best.max(score);
                    }
                };
                if enabled_cols.contains("State") {
                    try_match(&item.state);
                    try_match(if item.state == "unread" || item.state == "pending" {
                        "NEW"
                    } else {
                        "READ"
                    });
                }
                if enabled_cols.contains("Project") {
                    try_match(&item.project_path);
                }
                if enabled_cols.contains("Type") {
                    try_match(&item.target_type);
                }
                if enabled_cols.contains("ID") {
                    try_match(&item.target_iid.to_string());
                    try_match(&format!("#{}", item.target_iid));
                }
                if enabled_cols.contains("Title") {
                    try_match(&item.title);
                }
                if enabled_cols.contains("Updated") {
                    try_match(&crate::utils::format::time_ago(&item.updated_at));
                }
                if best > i64::MIN {
                    Some((best, item))
                } else {
                    None
                }
            })
            .collect();
        scored.sort_by_key(|(score, _)| -(*score));
        scored.into_iter().map(|(_, item)| item).collect()
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
                    "Updated" => a.updated_at.clone(),
                    _ => String::new(),
                };
                let val_b = match col.as_str() {
                    "State" => b.state.clone(),
                    "Type" => b.target_type.clone(),
                    "Project" => b.project_path.clone(),
                    "ID" => b.target_iid.to_string(),
                    "Title" => b.title.clone(),
                    "Updated" => b.updated_at.clone(),
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
                "State" => vec![if item.state == "unread" || item.state == "pending" {
                    "NEW".to_string()
                } else {
                    "READ".to_string()
                }],
                "Project" => vec![item.project_path.clone()],
                "Type" => vec![item.target_type.clone()],
                "ID" => vec![item.id.clone()],
                "Title" => vec![item.title.clone()],
                "Updated" => vec![crate::utils::format::time_ago(&item.updated_at)],
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
        let matcher = &*FUZZY_MATCHER;
        let q = query.trim();
        items
            .iter()
            .filter(|item| {
                let mut matches = false;
                let mut check_match = |text: &str| {
                    if matcher.fuzzy_match(text, q).is_some() {
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
                "ID" => vec![item.iid.to_string()],
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
        let matcher = &*FUZZY_MATCHER;
        let q = query.trim();
        items
            .iter()
            .filter(|item| {
                let mut matches = false;
                let mut check_match = |text: &str| {
                    if matcher.fuzzy_match(text, q).is_some() {
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
        let matcher = &*FUZZY_MATCHER;
        let q = query.trim();
        items
            .iter()
            .filter(|item| {
                let mut matches = false;
                let mut check_match = |text: &str| {
                    if matcher.fuzzy_match(text, q).is_some() {
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

    /// Canonicalise a saved filter value to the display string that
    /// `mr_filter_values` / issue filter functions now produce.
    ///
    /// Older config files stored lowercase API values ("opened", "Draft", …).
    /// After the display-alignment change those no longer appear in the
    /// filter-value sets, so this mapping keeps pre-existing filters working.
    /// Maps raw API pipeline/job status strings to the uppercase display text
    /// shown in the table cell (e.g. `"success"` → `"SUCCESS"`, `"canceled"` → `"CANCEL"`).
    fn pipeline_status_display(raw: &str) -> &str {
        match raw {
            "success" => "SUCCESS",
            "failed" => "FAILED",
            "running" => "RUNNING",
            "canceled" | "cancelled" => "CANCEL",
            "pending" => "PENDING",
            "skipped" => "SKIP",
            "manual" => "MANUAL",
            "created" | "waiting_for_resource" | "preparing" => "PENDING",
            other => other,
        }
    }

    /// Filter values for one Pipelines column.
    ///
    /// Single source of truth for the column-filter picker, `filtered_pipelines`,
    /// and the table renderer. These three used to carry their own copies of this
    /// mapping and had drifted apart (raw `status` vs the `SUCCESS` display text,
    /// raw `created_at` vs `time_ago`, no `Source`/`Duration` in the renderer), so
    /// the rows drawn on screen were not the rows the rest of the app indexed into.
    ///
    /// The first value is the one the picker offers — always the text the cell
    /// shows. Later values are compatibility aliases (raw `Ref` for filters saved
    /// before the cell switched to `format_ref`).
    pub fn pipeline_filter_values(
        item: &crate::domain::pipelines::Pipeline,
        col: &str,
    ) -> Vec<String> {
        match col {
            "ID" => vec![item.id().to_string()],
            "Status" => vec![Self::pipeline_status_display(item.status()).to_string()],
            "Ref" => {
                let raw = item.ref_branch().to_string();
                let display = crate::utils::format::format_ref(&raw);
                if display == raw {
                    vec![raw]
                } else {
                    vec![display, raw]
                }
            }
            "Name" => vec![item.name().to_string()],
            "Event" => vec![item.event().to_string()],
            "SHA" => vec![item.head_sha().to_string()],
            "Actor" => vec![item.actor_login().to_string()],
            "Created" => item
                .created_at()
                .map(|c| vec![crate::utils::format::time_ago(c)])
                .unwrap_or_default(),
            "Source" => item
                .source()
                .map(|s| vec![s.to_string()])
                .unwrap_or_default(),
            "Duration" => item
                .duration_seconds()
                .map(|d| vec![format!("{}m {}s", d / 60, d % 60)])
                .unwrap_or_default(),
            _ => vec![],
        }
    }

    fn normalize_filter_value(v: &str) -> &str {
        match v {
            // State
            "opened" => "OPEN",
            "closed" => "CLOSED",
            "merged" => "MERGED",
            // Status
            "Draft" | "draft" => "DRAFT",
            "Ready" | "ready" => "READY",
            "Unresolved discussions" => "UNRESOLVED",
            // Approval
            "Changes requested" | "Changes Requested" => "CHG",
            "Awaiting you" | "Awaiting You" => "AWAITING",
            "Approved" | "approved" => "APPROVED",
            "Pending" | "pending" | "Review req" | "review req" => "REVIEW REQ",
            // Mergeable
            "Conflict" | "conflict" => "CONFLICT",
            "Needs rebase" | "needs rebase" => "REBASE",
            "Checking" | "checking" => "CHECKING",
            "Mergeable" | "mergeable" | "Clean" | "clean" => "CLEAN",
            // Workflow (old long labels -> abbreviated cell words)
            "Returned to you" => "Returned",
            "Review requested" => "Review req",
            "Your merge requests" => "Yours",
            "Approved by you" => "Approved",
            "Approved by others" => "By others",
            // Pipeline/Job status (raw API → display)
            "success" => "SUCCESS",
            "failed" => "FAILED",
            "running" => "RUNNING",
            "canceled" | "cancelled" => "CANCEL",
            "skipped" => "SKIP",
            "manual" => "MANUAL",
            // Todos state
            "unread" | "done" => {
                // "unread"→"NEW", "done"→"READ" — handled via pipeline_status_display
                // but normalize maps them too for saved-filter compat
                if v == "unread" { "NEW" } else { "READ" }
            }
            other => other,
        }
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
                    vals.iter().any(|v| {
                        let norm_v = Self::normalize_filter_value(v);
                        selected.contains(v)
                            || selected.contains(norm_v)
                            || selected.iter().any(|s| {
                                let norm_s = Self::normalize_filter_value(s);
                                norm_s == v.as_str() || norm_s == norm_v
                            })
                    })
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
                            let display = if item.state == "opened" {
                                "OPEN"
                            } else {
                                "CLOSED"
                            };
                            values.insert(display.to_string());
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
                    for v in Self::mr_filter_values(item, col) {
                        values.insert(v);
                    }
                }
            }
            Tab::Pipelines => {
                for item in &self.pipelines.items {
                    // Offer the displayed value only — the trailing entries of
                    // `pipeline_filter_values` are back-compat aliases, not
                    // separate choices.
                    if let Some(v) = Self::pipeline_filter_values(item, col).into_iter().next() {
                        values.insert(v);
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
                            values.insert(Self::pipeline_status_display(item.status()).to_string());
                        }
                        "Name" => {
                            values.insert(item.name().to_string());
                        }
                        "Runner" => {
                            if let Some(r) = item.runner() {
                                values.insert(r.to_string());
                            }
                        }
                        "Needs" => {
                            values.extend(item.needs().iter().cloned());
                        }
                        "Duration" => {
                            if let Some(d) = item.duration_seconds() {
                                values.insert(format!("{}m {}s", d / 60, d % 60));
                            }
                        }
                        _ => {}
                    };
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
                            let display = if item.state == "unread" || item.state == "pending" {
                                "NEW"
                            } else {
                                "READ"
                            };
                            values.insert(display.to_string());
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
                        "Updated" => {
                            values.insert(crate::utils::format::time_ago(&item.updated_at));
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
                                "--".to_string()
                            } else {
                                i.labels[0].clone()
                            }
                        }
                        "Milestone" => i
                            .milestone
                            .as_ref()
                            .map(|m| m.title.clone())
                            .unwrap_or_else(|| "--".to_string()),
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
                                "--".to_string()
                            } else {
                                m.labels[0].clone()
                            }
                        }
                        "Milestone" => m
                            .milestone
                            .as_ref()
                            .map(|m| m.title.clone())
                            .unwrap_or_else(|| "--".to_string()),
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
                                "--".to_string()
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
                        "Updated" => crate::utils::format::time_ago(&n.updated_at),
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
                        "Name" => p.name().to_string(),
                        "Event" => p.event().to_string(),
                        "SHA" => p.head_sha().to_string(),
                        "Actor" => p.actor_login().to_string(),
                        "Created" => p
                            .created_at()
                            .map(|c| crate::utils::format::time_ago(c))
                            .unwrap_or_default(),
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
                        "Runner" => j.runner().unwrap_or("-").to_string(),
                        "Duration" => j
                            .duration_seconds()
                            .map(|d| format!("{}m", d / 60))
                            .unwrap_or_default(),
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
                        "Start Date" => m.start_date.clone().unwrap_or_else(|| "--".to_string()),
                        "Due Date" => m.due_date.clone().unwrap_or_else(|| "--".to_string()),
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
    fn test_submit_dialog_cursor_and_toggle() {
        let mut dialog = SubmitDialog::new(
            ConfirmAction::MergeMr(42),
            "Merge MR #42",
            "Merge with the selected options.",
            "Merge",
            vec![
                SubmitOption::new("Squash", true),
                SubmitOption::new("Delete source branch", true),
            ],
        );

        // Default cursor sits on Submit (leftmost).
        assert!(dialog.is_on_submit());
        assert_eq!(dialog.cancel_idx(), 3);
        assert_eq!(dialog.cursor_idx, 0);

        // Move right/down: Submit -> Squash -> Delete source branch -> Cancel.
        dialog.move_next();
        assert_eq!(dialog.option_idx(), Some(0));
        dialog.move_next();
        assert_eq!(dialog.option_idx(), Some(1));
        dialog.move_next();
        assert!(dialog.is_on_cancel());
        dialog.move_next();
        assert!(dialog.is_on_cancel(), "Cancel must clamp at the right");

        // Move left/up: Cancel -> Delete source branch -> Squash -> Submit.
        dialog.move_prev();
        assert_eq!(dialog.option_idx(), Some(1));
        dialog.move_prev();
        assert_eq!(dialog.option_idx(), Some(0));
        dialog.move_prev();
        assert!(dialog.is_on_submit());
        dialog.move_prev();
        assert!(dialog.is_on_submit(), "Submit must clamp at the left");

        // Toggle flips state and stays in place.
        dialog.cursor_idx = 1;
        assert!(dialog.options[0].checked);
        dialog.toggle_focused_option();
        assert!(!dialog.options[0].checked);
        dialog.toggle_focused_option();
        assert!(dialog.options[0].checked);

        // Toggling on a button row is a no-op.
        dialog.cursor_idx = 0;
        let before = dialog.options[0].checked;
        dialog.toggle_focused_option();
        assert_eq!(dialog.options[0].checked, before);
    }

    #[test]
    fn test_submit_dialog_build_safe_vs_submit_focus() {
        let app = App::default();
        // Destructive action defaults to Cancel.
        let close = SubmitDialog::build(ConfirmAction::CloseIssue(7), &app);
        assert!(close.is_on_cancel());
        assert_eq!(close.options.len(), 0);
        assert_eq!(close.submit_label, "Close");

        // Reversible action defaults to Submit.
        let merge = SubmitDialog::build(ConfirmAction::MergeMr(99), &app);
        assert!(merge.is_on_submit());
        assert_eq!(merge.options.len(), 2);
        assert!(merge.options.iter().any(|o| o.label == "Squash"));
        assert!(
            merge
                .options
                .iter()
                .any(|o| o.label == "Delete source branch")
        );

        let rebase = SubmitDialog::build(ConfirmAction::RebaseMr(12), &app);
        assert!(rebase.is_on_submit());
        assert_eq!(rebase.submit_label, "Rebase");
    }

    #[test]
    fn test_highlight_line_syntax_returns_theme_colors() {
        // A Rust keyword line should produce spans carrying a non-default fg
        // color derived from the active theme (not a hardcoded palette).
        let spans = highlight_line_syntax("main.rs", "let x = 1;", None);
        let spans = spans.expect("highlighting should succeed");
        assert!(!spans.is_empty());
        // At least one token (e.g. the `let` keyword) should carry an fg color.
        assert!(spans.iter().any(|(style, _)| style.fg.is_some()));
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
            blocking_discussions_resolved: None,
            approval: None,
            mergeability: None,
            workflow: None,
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
            blocking_discussions_resolved: None,
            approval: None,
            mergeability: None,
            workflow: None,
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
            blocking_discussions_resolved: None,
            approval: None,
            mergeability: None,
            workflow: None,
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
    fn search_matches_conflicting_mr_when_mergeable_column_enabled() {
        let mut mr = mr_fixture(1, "opened", "alice", false, "unrelated title");
        mr.mergeability = Some(crate::domain::mr_state::MergeabilityState {
            conflicts: true,
            needs_rebase: false,
            computing: false,
        });
        let items = vec![mr];

        let with_mergeable: std::collections::HashSet<String> = ["ID", "Title", "Mergeable"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let matched = App::filter_mrs_list(&items, "conflict", &with_mergeable);
        assert_eq!(
            matched.len(),
            1,
            "expected the conflicting MR to match a 'conflict' search when Mergeable is enabled"
        );

        let without_mergeable: std::collections::HashSet<String> =
            ["ID", "Title"].iter().map(|s| s.to_string()).collect();
        let unmatched = App::filter_mrs_list(&items, "conflict", &without_mergeable);
        assert!(
            unmatched.is_empty(),
            "the column gate should suppress the match when Mergeable is disabled"
        );
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

    #[test]
    fn test_save_layout_and_active_tab_and_group_sorting() {
        let _guard = crate::config::TEST_ENV_MUTEX.lock().unwrap();
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
        let _guard = crate::config::TEST_ENV_MUTEX.lock().unwrap();
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

    /// A one-hunk diff starting at `start_line`, so the line numbers it
    /// produces have a chosen number of digits.
    fn diff_starting_at(start_line: u32) -> String {
        format!(
            "diff --git a/spec.yml b/spec.yml\n--- a/spec.yml\n+++ b/spec.yml\n@@ -{s},3 +{s},4 @@\n context\n+added\n-removed\n",
            s = start_line
        )
    }

    #[test]
    fn test_line_number_width_floors_at_the_narrow_gutter() {
        // Anything that fits the old fixed field keeps the old look.
        let view = DiffView::new(42, diff_starting_at(1));
        assert_eq!(view.line_number_width, MIN_LINE_NUMBER_WIDTH);
        let view = DiffView::new(42, diff_starting_at(9997));
        assert_eq!(view.line_number_width, MIN_LINE_NUMBER_WIDTH);
    }

    #[test]
    fn test_line_number_width_grows_for_a_wider_number() {
        // The bug: `{:>4}` is a minimum, so a five-digit number took a fifth
        // cell and shifted that row's separator and content one column right.
        let view = DiffView::new(42, diff_starting_at(10_848));
        assert_eq!(view.line_number_width, 5);

        let view = DiffView::new(42, diff_starting_at(100_000));
        assert_eq!(view.line_number_width, 6);
    }

    #[test]
    fn test_line_number_width_is_taken_from_the_widest_line_in_the_diff() {
        // A diff whose first hunk is narrow and whose last is not must use the
        // wider gutter throughout, or the two hunks disagree on the column.
        let diff = format!("{}{}", diff_starting_at(12), {
            let mut second = diff_starting_at(15_610);
            second.push('\n');
            second
        });
        let view = DiffView::new(42, diff);
        assert_eq!(view.line_number_width, 5);
    }

    #[test]
    fn test_line_number_width_handles_a_diff_with_no_numbers() {
        // Meta-only output (a rename with no hunks) has no line numbers at all.
        let view = DiffView::new(
            42,
            "diff --git a/a.txt b/b.txt\nsimilarity index 100%\nrename from a.txt\nrename to b.txt\n"
                .to_string(),
        );
        assert_eq!(view.line_number_width, MIN_LINE_NUMBER_WIDTH);
    }

    /// A diff of a tab-indented file, as `gofmt` would produce it.
    fn tab_indented_diff() -> String {
        [
            "diff --git a/handler.go b/handler.go",
            "index 1111111..2222222 100644",
            "--- a/handler.go",
            "+++ b/handler.go",
            "@@ -1,6 +1,7 @@",
            " func handle() error {",
            " \tif err := check(); err != nil {",
            "+\t\treturn fmt.Errorf(\"check: %w\", err)",
            "-\t\treturn err",
            " \t}",
            " }",
        ]
        .join("\n")
    }

    #[test]
    fn test_diff_view_expands_tabs_so_indentation_survives() {
        let view = DiffView::new(42, tab_indented_diff());

        let content: Vec<&str> = view
            .all_lines
            .iter()
            .map(|l| l.content.as_str())
            .filter(|c| c.contains("return") || c.contains("if err"))
            .collect();

        assert_eq!(
            content,
            vec![
                " \u{20}\u{20}\u{20}\u{20}if err := check(); err != nil {",
                "+        return fmt.Errorf(\"check: %w\", err)",
                "-        return err",
            ],
            "tabs must reach the rendered content as spaces, or the file displays flush-left"
        );
        assert!(
            !view.all_lines.iter().any(|l| l.content.contains('\t')),
            "no tab may survive into a rendered line"
        );
    }

    #[test]
    fn test_diff_view_keeps_the_marker_out_of_the_tab_stops() {
        let view = DiffView::new(42, tab_indented_diff());
        let added = view
            .all_lines
            .iter()
            .find(|l| l.line_type == DiffLineType::Addition)
            .expect("an addition");

        // One marker, then a full tab stop per tab — not three spaces because
        // the marker ate a column.
        assert!(added.content.starts_with('+'));
        assert_eq!(
            added.content[1..].len() - added.content[1..].trim_start().len(),
            8,
            "two tabs of source indent must survive as two full stops"
        );
    }

    #[test]
    fn test_diff_view_expands_tabs_in_the_highlighted_spans_too() {
        // A highlighted line renders from its spans, not from `content`, so a
        // tab surviving there would leave syntax-highlighted code sitting at a
        // different indent from everything else.
        let view = DiffView::new(42, tab_indented_diff());
        let mut highlighted_lines = 0;
        for line in &view.all_lines {
            if let Some(ref spans) = line.syntax_highlighted {
                highlighted_lines += 1;
                let text: String = spans.iter().map(|(_, t)| t.as_str()).collect();
                assert!(!text.contains('\t'), "a span kept its tab: {text:?}");
            }
        }
        assert!(
            highlighted_lines > 0,
            "fixture produced no highlighted lines, so this proves nothing"
        );
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

    fn review_fixture() -> DiffView {
        let diff = "\
diff --git a/src/app.rs b/src/app.rs
index 123456..789012 100644
--- a/src/app.rs
+++ b/src/app.rs
@@ -1,1 +1,1 @@
- old
+ new
diff --git a/src/main.rs b/src/main.rs
index 123456..789012 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,1 +1,1 @@
- old
+ new
diff --git a/README.md b/README.md
index 123456..789012 100644
--- a/README.md
+++ b/README.md
@@ -1,1 +1,1 @@
- old
+ new
";
        DiffView::new(42, diff.to_string())
    }

    #[test]
    fn test_toggle_reviewed_marks_selected_file() {
        let mut view = review_fixture();
        // Tree (directories sort first): src/, src/app.rs, src/main.rs, README.md
        assert_eq!(view.visible_nodes.len(), 4);
        assert_eq!(view.review_progress(), (0, 3));

        view.selected_visible_idx = 3; // README.md
        assert_eq!(view.toggle_reviewed(), Some((1, true)));
        assert!(view.reviewed_files.contains("README.md"));
        assert_eq!(view.review_progress(), (1, 3));
        assert!(view.visible_nodes[3].is_reviewed);
        // Still listed — only the filter hides files.
        assert_eq!(view.visible_nodes.len(), 4);

        assert_eq!(view.toggle_reviewed(), Some((1, false)));
        assert!(view.reviewed_files.is_empty());
        assert!(!view.visible_nodes[3].is_reviewed);
    }

    #[test]
    fn test_toggle_reviewed_on_directory_marks_every_file_below() {
        let mut view = review_fixture();
        view.selected_visible_idx = 0; // src/
        assert!(view.visible_nodes[0].is_dir);

        assert_eq!(view.toggle_reviewed(), Some((2, true)));
        assert_eq!(view.review_progress(), (2, 3));
        // The directory reads as reviewed once all of its files are.
        assert!(view.visible_nodes[0].is_reviewed);

        // Completing it also folds it, so reopen it to reach a single file.
        view.root_node.toggle_expanded("root/src", "");
        view.rebuild_visible_nodes();

        // Unmarking one file leaves the directory pending again.
        view.selected_visible_idx = 1; // src/app.rs
        assert_eq!(view.toggle_reviewed(), Some((1, false)));
        assert!(!view.visible_nodes[0].is_reviewed);
        assert_eq!(view.review_progress(), (1, 3));
    }

    #[test]
    fn test_hide_reviewed_filters_files_and_empty_directories() {
        let mut view = review_fixture();
        view.selected_visible_idx = 0; // src/
        view.toggle_reviewed();

        assert!(view.toggle_hide_reviewed());
        // Both src files are reviewed, so the directory drops out too.
        let names: Vec<&str> = view.visible_nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["README.md"]);

        // Unfiltering brings the completed directory back — folded, since all of
        // its files are reviewed.
        assert!(!view.toggle_hide_reviewed());
        let names: Vec<&str> = view.visible_nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["src", "README.md"]);
    }

    #[test]
    fn test_hide_reviewed_keeps_directory_with_pending_files_when_collapsed() {
        let mut view = review_fixture();
        view.reviewed_files.insert("src/app.rs".to_string());
        view.collapse_all();
        view.hide_reviewed = true;
        view.rebuild_visible_nodes();

        // src/ still holds an unreviewed file, so it must stay visible even
        // though collapsing means none of its children are flattened.
        let names: Vec<&str> = view.visible_nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["src", "README.md"]);
    }

    #[test]
    fn test_completing_a_directory_folds_it() {
        let mut view = review_fixture();
        // Tree: src/, src/app.rs, src/main.rs, README.md
        view.selected_visible_idx = 1; // src/app.rs
        view.toggle_reviewed();
        // One file left pending — the directory stays open.
        assert!(view.visible_nodes[0].is_expanded);
        assert_eq!(view.visible_nodes.len(), 4);

        view.selected_visible_idx = 2; // src/main.rs
        view.toggle_reviewed();

        // src/ is complete: folded, and its files no longer clutter the tree.
        let names: Vec<&str> = view.visible_nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["src", "README.md"]);
        assert!(!view.visible_nodes[0].is_expanded);
        assert!(view.visible_nodes[0].is_reviewed);
        // The cursor follows the fold onto the directory that just completed.
        assert_eq!(view.selected_visible_idx, 0);
    }

    #[test]
    fn test_folding_cascades_through_nested_directories() {
        let diff = "\
diff --git a/a/b/c/deep.rs b/a/b/c/deep.rs
index 123456..789012 100644
--- a/a/b/c/deep.rs
+++ b/a/b/c/deep.rs
@@ -1,1 +1,1 @@
- old
+ new
diff --git a/README.md b/README.md
index 123456..789012 100644
--- a/README.md
+++ b/README.md
@@ -1,1 +1,1 @@
- old
+ new
";
        let mut view = DiffView::new(42, diff.to_string());
        assert_eq!(view.visible_nodes.len(), 5); // a, b, c, deep.rs, README.md

        view.selected_visible_idx = 3; // a/b/c/deep.rs
        view.toggle_reviewed();

        // Every ancestor is complete, so the whole branch folds into `a`.
        let names: Vec<&str> = view.visible_nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["a", "README.md"]);
        assert!(!view.visible_nodes[0].is_expanded);
    }

    #[test]
    fn test_unmarking_a_folded_directory_reopens_it() {
        let mut view = review_fixture();
        view.selected_visible_idx = 0; // src/
        view.toggle_reviewed();
        assert!(!view.visible_nodes[0].is_expanded);

        // Pressing `m` again on the folded directory unmarks it and reopens it,
        // so the files that need another pass are visible again.
        view.toggle_reviewed();
        let names: Vec<&str> = view.visible_nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["src", "app.rs", "main.rs", "README.md"]);
        assert!(view.visible_nodes[0].is_expanded);
    }

    #[test]
    fn test_reopened_reviewed_directory_stays_open_on_unrelated_marks() {
        let mut view = review_fixture();
        view.selected_visible_idx = 0; // src/
        view.toggle_reviewed();
        assert!(!view.visible_nodes[0].is_expanded);

        // The user reopens the completed directory by hand to look again.
        view.root_node.toggle_expanded("root/src", "");
        view.rebuild_visible_nodes();
        assert!(view.visible_nodes[0].is_expanded);

        // Marking an unrelated file must not re-fold it: only a change in the
        // directory's own review state moves it.
        view.selected_visible_idx = 3; // README.md
        view.toggle_reviewed();
        assert!(view.visible_nodes[0].is_expanded);
    }

    #[test]
    fn test_restore_review_state_opens_completed_directories_folded() {
        let mut view = review_fixture();
        let cached: HashSet<String> = ["src/app.rs", "src/main.rs"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        view.restore_review_state(cached, false);

        let names: Vec<&str> = view.visible_nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["src", "README.md"]);
        assert!(!view.visible_nodes[0].is_expanded);
    }

    #[test]
    fn test_restore_review_state_drops_paths_no_longer_in_the_diff() {
        let mut view = review_fixture();
        let cached: HashSet<String> = ["src/app.rs", "deleted/elsewhere.rs"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        view.restore_review_state(cached, false);

        assert_eq!(
            view.reviewed_files,
            ["src/app.rs".to_string()].into_iter().collect()
        );
        assert_eq!(view.review_progress(), (1, 3));
    }

    #[test]
    fn test_marking_with_filter_on_advances_to_the_next_file() {
        let mut view = review_fixture();
        view.hide_reviewed = true;
        view.selected_visible_idx = 1; // src/app.rs
        view.toggle_reviewed();

        // app.rs vanished; the cursor keeps its slot, so the next pending file
        // slides underneath it instead of jumping back to the top.
        assert_eq!(view.selected_visible_idx, 1);
        assert_eq!(view.visible_nodes[1].name, "main.rs");
    }

    fn mr_fixture(
        iid: u64,
        state: &str,
        author: &str,
        draft: bool,
        title: &str,
    ) -> crate::domain::mr::MergeRequest {
        crate::domain::mr::MergeRequest {
            iid,
            title: title.to_string(),
            state: state.to_string(),
            labels: vec![],
            updated_at: "2026-07-29T00:00:00Z".to_string(),
            author: crate::domain::mr::Author {
                username: author.to_string(),
            },
            milestone: None,
            assignees: vec![],
            reviewers: vec![],
            target_branch: "main".to_string(),
            source_branch: "feature".to_string(),
            draft,
            description: None,
            head_pipeline: None,
            blocking_discussions_resolved: None,
            approval: None,
            mergeability: None,
            workflow: None,
        }
    }

    fn mr_enabled_cols(
        cols: &[&str],
    ) -> std::collections::HashMap<Tab, std::collections::HashSet<String>> {
        let mut m = std::collections::HashMap::new();
        m.insert(
            Tab::MergeRequests,
            cols.iter().map(|c| c.to_string()).collect(),
        );
        m
    }

    /// iids in the order `filtered_mrs_list` returns them when grouped by `col`.
    fn sorted_iids(
        items: &[crate::domain::mr::MergeRequest],
        col: &str,
        ascending: bool,
    ) -> Vec<u64> {
        let cols = mr_enabled_cols(&["ID", "State", "Author", "Status", "Title"]);
        App::filtered_mrs_list(items, "", &cols, ascending, &Some(col.to_string()))
            .iter()
            .map(|m| m.iid)
            .collect()
    }

    #[test]
    fn sorting_by_author_is_alphabetical() {
        let items = vec![
            mr_fixture(1, "opened", "zoe", false, "z"),
            mr_fixture(2, "opened", "alice", false, "a"),
            mr_fixture(3, "opened", "mallory", false, "m"),
        ];
        assert_eq!(sorted_iids(&items, "Author", true), vec![2, 3, 1]);
    }

    #[test]
    fn sorting_respects_descending_flag() {
        let items = vec![
            mr_fixture(1, "opened", "zoe", false, "z"),
            mr_fixture(2, "opened", "alice", false, "a"),
        ];
        assert_eq!(sorted_iids(&items, "Author", false), vec![1, 2]);
    }

    #[test]
    fn sorting_by_id_is_numeric_not_lexicographic() {
        // The comparator parses both sides as u64 when possible, so 9 < 10.
        let items = vec![
            mr_fixture(10, "opened", "a", false, "ten"),
            mr_fixture(9, "opened", "b", false, "nine"),
            mr_fixture(100, "opened", "c", false, "hundred"),
        ];
        assert_eq!(sorted_iids(&items, "ID", true), vec![9, 10, 100]);
    }

    #[test]
    fn sorting_by_status_uses_draft_ready_words() {
        let items = vec![
            mr_fixture(1, "opened", "a", false, "ready one"),
            mr_fixture(2, "opened", "b", true, "draft one"),
        ];
        // "Draft" < "Ready" alphabetically.
        assert_eq!(sorted_iids(&items, "Status", true), vec![2, 1]);
    }

    #[test]
    fn sorting_by_title_is_alphabetical() {
        let items = vec![
            mr_fixture(1, "opened", "a", false, "beta"),
            mr_fixture(2, "opened", "b", false, "alpha"),
        ];
        assert_eq!(sorted_iids(&items, "Title", true), vec![2, 1]);
    }

    #[test]
    fn sorting_by_state_groups_by_state_string() {
        let items = vec![
            mr_fixture(1, "opened", "a", false, "x"),
            mr_fixture(2, "closed", "b", false, "y"),
            mr_fixture(3, "merged", "c", false, "z"),
        ];
        // closed < merged < opened alphabetically.
        assert_eq!(sorted_iids(&items, "State", true), vec![2, 3, 1]);
    }

    #[test]
    fn sorting_by_unknown_column_is_stable_noop() {
        // Unrecognised columns fall through to empty strings, so all compare equal
        // and the original order is preserved.
        let items = vec![
            mr_fixture(3, "opened", "a", false, "x"),
            mr_fixture(1, "opened", "b", false, "y"),
            mr_fixture(2, "opened", "c", false, "z"),
        ];
        assert_eq!(sorted_iids(&items, "NoSuchColumn", true), vec![3, 1, 2]);
    }

    #[test]
    fn mr_columns_include_both_state_columns_on_both_hosts() {
        for kind in [BackendKind::GitLab, BackendKind::GitHub] {
            let cols = Tab::MergeRequests.columns(kind);
            assert!(cols.contains(&"Approval"), "missing Approval for {kind:?}");
            assert!(
                cols.contains(&"Mergeable"),
                "missing Mergeable for {kind:?}"
            );
        }
    }

    #[test]
    fn mr_default_columns_show_both_state_columns() {
        // Default-on, else the feature hides behind Tab -> configure.
        let cols = Tab::MergeRequests.default_columns(BackendKind::GitLab);
        assert!(cols.contains(&"Approval"));
        assert!(cols.contains(&"Mergeable"));
    }

    #[test]
    fn mr_default_columns_keep_pre_existing_defaults() {
        // Adding columns must not remove any.
        let cols = Tab::MergeRequests.default_columns(BackendKind::GitLab);
        for expected in ["ID", "State", "Status", "Title", "Labels"] {
            assert!(cols.contains(&expected), "lost default column {expected}");
        }
    }

    #[test]
    fn sorting_by_mergeable_puts_conflicts_first() {
        use crate::domain::mr_state::MergeabilityState;
        let mut conflicted = mr_fixture(1, "opened", "a", false, "conflicted");
        conflicted.mergeability = Some(MergeabilityState {
            conflicts: true,
            needs_rebase: false,
            computing: false,
        });
        let mut clean = mr_fixture(2, "opened", "b", false, "clean");
        clean.mergeability = Some(MergeabilityState {
            conflicts: false,
            needs_rebase: false,
            computing: false,
        });
        let unknown = mr_fixture(3, "opened", "c", false, "unknown");
        let items = vec![clean, unknown, conflicted];
        // conflict(0) < clean(3) < unknown(4)
        assert_eq!(sorted_iids(&items, "Mergeable", true), vec![1, 2, 3]);
    }

    // ── MR filter picker options (collect_unique_column_values) ──
    //
    // filtered_mrs (matching) and collect_unique_column_values (picker options)
    // are two separate call sites over the same data; they previously drifted,
    // leaving values that matched an active filter but could never be selected
    // from the picker. mr_filter_values is now the single source both call.

    #[test]
    fn collect_unique_column_values_status_includes_unresolved_discussions_flag() {
        let mut draft_unresolved = mr_fixture(1, "opened", "a", true, "draft with unresolved");
        draft_unresolved.blocking_discussions_resolved = Some(false);
        let mut app = App::default();
        app.mrs.items = vec![draft_unresolved];

        let values = app.collect_unique_column_values(Tab::MergeRequests, "Status");

        assert!(values.contains(&"DRAFT".to_string()));
        assert!(values.contains(&"UNRESOLVED".to_string()));
    }

    #[test]
    fn collect_unique_column_values_approval_offers_tone_label() {
        use crate::domain::mr_state::ApprovalState;
        let mut approved = mr_fixture(1, "opened", "a", false, "approved");
        approved.approval = Some(ApprovalState {
            approved: true,
            approvals_left: Some(0),
            approvals_required: Some(1),
            approved_by: vec!["chandler.anderson".to_string()],
            changes_requested: false,
            you_approved: true,
            awaiting_you: false,
            ..Default::default()
        });
        let mut app = App::default();
        app.mrs.items = vec![approved];

        let values = app.collect_unique_column_values(Tab::MergeRequests, "Approval");

        assert!(values.contains(&"APPROVED".to_string()));
    }

    #[test]
    fn collect_unique_column_values_mergeable_offers_conflict_label() {
        use crate::domain::mr_state::MergeabilityState;
        let mut conflicted = mr_fixture(1, "opened", "a", false, "conflicted");
        conflicted.mergeability = Some(MergeabilityState {
            conflicts: true,
            needs_rebase: false,
            computing: false,
        });
        let mut app = App::default();
        app.mrs.items = vec![conflicted];

        let values = app.collect_unique_column_values(Tab::MergeRequests, "Mergeable");

        assert!(values.contains(&"CONFLICT".to_string()));
    }

    #[test]
    fn collect_unique_column_values_agrees_with_mr_filter_values() {
        // The invariant that broke: every value the matching side (mr_filter_values)
        // would accept for an MR must also be offered by the picker side
        // (collect_unique_column_values), or a user can never select it.
        let mut draft_unresolved = mr_fixture(1, "opened", "a", true, "draft with unresolved");
        draft_unresolved.blocking_discussions_resolved = Some(false);
        // Give Workflow a real (non-`None`) value too, or its expected set
        // from `mr_filter_values` would be trivially empty and the parity
        // check below would pass without checking anything.
        draft_unresolved.workflow = Some(crate::domain::mr_state::WorkflowStatus::ReturnedToYou);
        let mut app = App::default();
        app.mrs.items = vec![draft_unresolved];

        for col in ["Status", "Approval", "Mergeable", "Workflow"] {
            let offered = app.collect_unique_column_values(Tab::MergeRequests, col);
            let expected = App::mr_filter_values(&app.mrs.items[0], col);
            for v in expected {
                assert!(
                    offered.contains(&v),
                    "column {col}: {v} matches via mr_filter_values but is not offered by collect_unique_column_values"
                );
            }
        }
    }

    #[test]
    fn column_filtering_works_for_pipelines_jobs_and_todos() {
        let mut app = App::default();

        // 1. Pipelines
        let p_success = crate::domain::pipelines::Pipeline {
            id: 1,
            status: "success".to_string(),
            r#ref: "main".to_string(),
            updated_at: "".to_string(),
            name: "".to_string(),
            display_title: "".to_string(),
            event: "".to_string(),
            head_sha: "".to_string(),
            actor_login: "".to_string(),
            duration_seconds: None,
            created_at: None,
            source: None,
        };
        let p_failed = crate::domain::pipelines::Pipeline {
            id: 2,
            status: "failed".to_string(),
            r#ref: "main".to_string(),
            updated_at: "".to_string(),
            name: "".to_string(),
            display_title: "".to_string(),
            event: "".to_string(),
            head_sha: "".to_string(),
            actor_login: "".to_string(),
            duration_seconds: None,
            created_at: None,
            source: None,
        };
        app.pipelines.items = vec![p_success, p_failed];

        // Filter Pipelines by new display value "SUCCESS"
        app.column_filters
            .entry(Tab::Pipelines)
            .or_default()
            .insert(
                "Status".to_string(),
                ["SUCCESS".to_string()].into_iter().collect(),
            );
        let res = app.filtered_pipelines();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].id, 1);

        // Filter Pipelines by legacy value "success"
        app.column_filters
            .entry(Tab::Pipelines)
            .or_default()
            .insert(
                "Status".to_string(),
                ["success".to_string()].into_iter().collect(),
            );
        let res_legacy = app.filtered_pipelines();
        assert_eq!(res_legacy.len(), 1);
        assert_eq!(res_legacy[0].id, 1);

        // 2. Todos
        let t_unread = crate::domain::notifications::Notification {
            id: "101".to_string(),
            state: "unread".to_string(),
            project_path: "org/repo".to_string(),
            target_type: "Issue".to_string(),
            target_iid: 1,
            title: "Task 1".to_string(),
            updated_at: "".to_string(),
        };
        let t_done = crate::domain::notifications::Notification {
            id: "102".to_string(),
            state: "done".to_string(),
            project_path: "org/repo".to_string(),
            target_type: "Issue".to_string(),
            target_iid: 2,
            title: "Task 2".to_string(),
            updated_at: "".to_string(),
        };
        app.todos.items = vec![t_unread, t_done];

        // 3. Issues
        let i_open = crate::domain::issues::Issue {
            iid: 1,
            title: "Issue 1".to_string(),
            state: "opened".to_string(),
            labels: vec![],
            updated_at: "".to_string(),
            created_at: None,
            closed_at: None,
            author: crate::domain::issues::Author {
                username: "user1".to_string(),
            },
            milestone: None,
            assignees: vec![],
            description: None,
            due_date: None,
        };
        let i_closed = crate::domain::issues::Issue {
            iid: 2,
            title: "Issue 2".to_string(),
            state: "closed".to_string(),
            labels: vec![],
            updated_at: "".to_string(),
            created_at: None,
            closed_at: None,
            author: crate::domain::issues::Author {
                username: "user2".to_string(),
            },
            milestone: None,
            assignees: vec![],
            description: None,
            due_date: None,
        };
        app.issues.items = vec![i_open, i_closed];

        // Filter Issues by new display value "OPEN"
        app.column_filters.entry(Tab::Issues).or_default().insert(
            "State".to_string(),
            ["OPEN".to_string()].into_iter().collect(),
        );
        let res_issues = app.filtered_issues();
        assert_eq!(res_issues.len(), 1);
        assert_eq!(res_issues[0].iid, 1);

        // Filter Issues by legacy value "opened"
        app.column_filters.entry(Tab::Issues).or_default().insert(
            "State".to_string(),
            ["opened".to_string()].into_iter().collect(),
        );
        let res_issues_legacy = app.filtered_issues();
        assert_eq!(res_issues_legacy.len(), 1);
        assert_eq!(res_issues_legacy[0].iid, 1);
    }

    #[test]
    fn workflow_column_is_offered_but_not_default() {
        for kind in [BackendKind::GitLab, BackendKind::GitHub] {
            assert!(
                Tab::MergeRequests.columns(kind).contains(&"Workflow"),
                "Workflow must be offered for {kind:?}"
            );
        }
        // Default-off: eight default columns collapse Title at 80 cols.
        assert!(
            !Tab::MergeRequests
                .default_columns(BackendKind::GitLab)
                .contains(&"Workflow")
        );
    }

    #[test]
    fn workflow_sorts_in_cascade_order_not_alphabetically() {
        use crate::domain::mr_state::WorkflowStatus;
        let mut returned = mr_fixture(1, "opened", "a", false, "returned");
        returned.workflow = Some(WorkflowStatus::ReturnedToYou);
        let mut yours = mr_fixture(2, "opened", "b", false, "yours");
        yours.workflow = Some(WorkflowStatus::YourMergeRequest);
        let mut approved = mr_fixture(3, "opened", "c", false, "approved");
        approved.workflow = Some(WorkflowStatus::ApprovedByYou);
        // Alphabetically "Approved…" < "Returned…" < "Your…"; by cascade the
        // order is Returned(0), Yours(2), Approved(3).
        let items = vec![approved, yours, returned];
        assert_eq!(sorted_iids(&items, "Workflow", true), vec![1, 2, 3]);
    }

    #[test]
    fn workflow_filter_offers_the_gitlab_label() {
        use crate::domain::mr_state::WorkflowStatus;
        let mut mr = mr_fixture(1, "opened", "a", false, "x");
        mr.workflow = Some(WorkflowStatus::ReturnedToYou);
        assert_eq!(
            App::mr_filter_values(&mr, "Workflow"),
            vec!["Returned".to_string()]
        );
    }

    #[test]
    fn workflow_search_matches_the_label() {
        use crate::domain::mr_state::WorkflowStatus;
        let mut mr = mr_fixture(1, "opened", "a", false, "unrelated title");
        mr.workflow = Some(WorkflowStatus::ReturnedToYou);
        let items = vec![mr];
        let with = mr_enabled_cols(&["ID", "Title", "Workflow"]);
        let cols_with: std::collections::HashSet<String> =
            with.get(&Tab::MergeRequests).unwrap().clone();
        assert_eq!(
            App::filter_mrs_list(&items, "returned", &cols_with).len(),
            1,
            "search must match the Workflow label when the column is enabled"
        );
        let without = mr_enabled_cols(&["ID", "Title"]);
        let cols_without: std::collections::HashSet<String> =
            without.get(&Tab::MergeRequests).unwrap().clone();
        assert_eq!(
            App::filter_mrs_list(&items, "returned", &cols_without).len(),
            0,
            "the column gate must still apply"
        );
    }

    #[test]
    fn pipeline_metadata_columns_filter_and_offer_values() {
        let mut app = App::default();
        app.pipelines.items = vec![
            crate::domain::pipelines::Pipeline {
                id: 1,
                status: "success".to_string(),
                r#ref: "main".to_string(),
                updated_at: String::new(),
                name: "CI".to_string(),
                display_title: "Build".to_string(),
                event: "push".to_string(),
                head_sha: "abc123".to_string(),
                actor_login: "alice".to_string(),
                duration_seconds: Some(125),
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                source: Some("push".to_string()),
            },
            crate::domain::pipelines::Pipeline {
                id: 2,
                status: "failed".to_string(),
                r#ref: "main".to_string(),
                updated_at: String::new(),
                name: "Deploy".to_string(),
                display_title: "Release".to_string(),
                event: "schedule".to_string(),
                head_sha: "def456".to_string(),
                actor_login: "bob".to_string(),
                duration_seconds: Some(65),
                created_at: Some("2026-01-02T00:00:00Z".to_string()),
                source: Some("schedule".to_string()),
            },
        ];
        app.column_filters
            .entry(Tab::Pipelines)
            .or_default()
            .insert(
                "Actor".to_string(),
                ["alice".to_string()].into_iter().collect(),
            );

        let filtered = app.filtered_pipelines();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, 1);
        assert!(
            app.collect_unique_column_values(Tab::Pipelines, "Duration")
                .contains(&"2m 5s".to_string())
        );
        assert!(
            app.collect_unique_column_values(Tab::Pipelines, "Source")
                .contains(&"schedule".to_string())
        );
    }

    #[test]
    fn show_error_sets_toast_and_marks_running_command() {
        let mut app = App::default();
        app.terminal_commands.push(TerminalCommand {
            timestamp: String::new(),
            command: "glab mr list".to_string(),
            status: "Running".to_string(),
        });

        app.show_error("boom".to_string());

        assert_eq!(app.error_message, Some("boom".to_string()));
        assert!(app.error_message_at.is_some());
        assert_eq!(app.terminal_commands[0].status, "Failed: boom");
    }

    #[test]
    fn show_error_prefers_cli_command_over_older_running() {
        let mut app = App::default();
        app.terminal_commands.push(TerminalCommand {
            timestamp: String::new(),
            command: "deferred task".to_string(),
            status: "Running".to_string(),
        });
        app.terminal_commands.push(TerminalCommand {
            timestamp: String::new(),
            command: "gh pr list".to_string(),
            status: "Running".to_string(),
        });

        app.show_error("boom".to_string());

        assert_eq!(app.terminal_commands[0].status, "Running");
        assert_eq!(app.terminal_commands[1].status, "Failed: boom");
    }

    #[test]
    fn show_error_falls_back_to_most_recent_running_when_no_cli() {
        let mut app = App::default();
        app.terminal_commands.push(TerminalCommand {
            timestamp: String::new(),
            command: "deferred task".to_string(),
            status: "Running".to_string(),
        });

        app.show_error("boom".to_string());

        assert_eq!(app.terminal_commands[0].status, "Failed: boom");
    }

    #[test]
    fn show_error_leaves_completed_commands_untouched() {
        let mut app = App::default();
        app.terminal_commands.push(TerminalCommand {
            timestamp: String::new(),
            command: "glab mr list".to_string(),
            status: "Success".to_string(),
        });

        app.show_error("boom".to_string());

        assert_eq!(app.terminal_commands[0].status, "Success");
    }

    #[test]
    fn pipeline_ref_is_searchable_and_filterable_by_its_displayed_text() {
        // GitLab MR pipelines carry `refs/merge-requests/<iid>/head`, but the
        // Ref cell shows "MR !<iid>". Searching or filtering by what the row
        // actually says used to match nothing, so a pipeline sitting visibly in
        // the table looked missing.
        let mut app = App::default();
        app.pipelines.items = vec![crate::domain::pipelines::Pipeline {
            id: 22598077,
            status: "running".to_string(),
            r#ref: "refs/merge-requests/2208/head".to_string(),
            updated_at: String::new(),
            name: String::new(),
            display_title: String::new(),
            event: "merge_request_event".to_string(),
            head_sha: "abc123".to_string(),
            actor_login: "alice".to_string(),
            duration_seconds: None,
            created_at: None,
            source: Some("merge_request_event".to_string()),
        }];
        let cols: std::collections::HashSet<String> = ["Ref".to_string()].into_iter().collect();
        let jobs = std::collections::HashMap::new();

        for query in ["MR !2208", "2208", "merge-requests"] {
            assert_eq!(
                App::filter_pipelines_list(&app.pipelines.items, query, &jobs, &cols).len(),
                1,
                "search for {query:?} must find the MR pipeline"
            );
        }

        // The picker offers the displayed text …
        let offered = app.collect_unique_column_values(Tab::Pipelines, "Ref");
        assert_eq!(offered, vec!["MR !2208".to_string()]);

        // … and filtering by it keeps the row.
        app.column_filters
            .entry(Tab::Pipelines)
            .or_default()
            .insert(
                "Ref".to_string(),
                ["MR !2208".to_string()].into_iter().collect(),
            );
        assert_eq!(app.filtered_pipelines().len(), 1);

        // A filter saved before the cell switched to `format_ref` still works.
        app.column_filters
            .entry(Tab::Pipelines)
            .or_default()
            .insert(
                "Ref".to_string(),
                ["refs/merge-requests/2208/head".to_string()]
                    .into_iter()
                    .collect(),
            );
        assert_eq!(app.filtered_pipelines().len(), 1);
    }
}
