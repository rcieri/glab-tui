use crate::utils::ui::StatefulTable;
use ratatui::widgets::{ListState, TableState};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    #[default]
    Issues,
    MergeRequests,
    Pipelines,
    Runners,
    Releases,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Issues,
        Tab::MergeRequests,
        Tab::Pipelines,
        Tab::Runners,
        Tab::Releases,
    ];

    pub fn title(&self, is_github: bool) -> &'static str {
        match self {
            Tab::Issues => "Issues",
            Tab::MergeRequests => if is_github { "PRs" } else { "MRs" },
            Tab::Pipelines => "Pipelines",
            Tab::Runners => "Runners",
            Tab::Releases => "Releases",
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
            self.all_items.iter().map(|item| (item.clone(), None)).collect()
        } else {
            let matcher = SkimMatcherV2::default();
            let mut scored: Vec<(String, Vec<usize>, i64)> = self.all_items.iter()
                .filter_map(|item| {
                    matcher.fuzzy_indices(item, query).map(|(score, indices)| (item.clone(), indices, score))
                })
                .collect();
            scored.sort_by(|a, b| b.2.cmp(&a.2));
            scored.into_iter().map(|(item, indices, _)| (item, Some(indices))).collect()
        };
            
        if !query.is_empty() {
            let exact_match = self.all_items.iter().any(|item| item.to_lowercase() == query.to_lowercase());
            if !exact_match {
                items.insert(0, (format!("+ Create \"{}\"", query), None));
            }
        }
        items
    }

    pub fn get_filtered_items(&self) -> Vec<String> {
        self.get_filtered_items_with_indices().into_iter().map(|(item, _)| item).collect()
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
            DiffTreeNode::Directory { name, is_expanded, children } => {
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
                        let a_is_dir = match a { DiffTreeNode::Directory { .. } => true, _ => false };
                        let b_is_dir = match b { DiffTreeNode::Directory { .. } => true, _ => false };
                        b_is_dir.cmp(&a_is_dir).then_with(|| a.name().cmp(b.name()))
                    });
                    for child in sorted_children {
                        child.flatten(if name == "root" { 0 } else { depth + 1 }, &path_id, out);
                    }
                }
            }
            DiffTreeNode::File { name, file_path, line_idx } => {
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
            DiffTreeNode::Directory { name, is_expanded, children } => {
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
                    detected_file = Some(parts[3].strip_prefix("b/").unwrap_or(parts[3]).to_string());
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
                });
                old_line_num = None;
                new_line_num = None;
            } else if line.starts_with("--- ") || line.starts_with("+++ ") || line.starts_with("index ") {
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::Meta,
                    file_path: current_file.clone(),
                    old_line_num: None,
                    new_line_num: None,
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
                });
            } else if line.starts_with('+') {
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::Addition,
                    file_path: current_file.clone(),
                    old_line_num: None,
                    new_line_num: new_line_num,
                });
                if let Some(ref mut n) = new_line_num {
                    *n += 1;
                }
            } else if line.starts_with('-') {
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::Deletion,
                    file_path: current_file.clone(),
                    old_line_num: old_line_num,
                    new_line_num: None,
                });
                if let Some(ref mut n) = old_line_num {
                    *n += 1;
                }
            } else {
                all_lines.push(DiffLine {
                    content: line.to_string(),
                    line_type: DiffLineType::Normal,
                    file_path: current_file.clone(),
                    old_line_num: old_line_num,
                    new_line_num: new_line_num,
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
        };

        view.update_active_lines();
        view
    }

    pub fn update_active_lines(&mut self) {
        if self.visible_nodes.is_empty() {
            self.lines = self.all_lines.clone();
            self.hunks = self.lines.iter().enumerate()
                .filter(|(_, l)| l.line_type == DiffLineType::HunkHeader)
                .map(|(i, _)| i)
                .collect();
            return;
        }

        let selected_node = &self.visible_nodes[self.selected_visible_idx];
        let rel_path = if selected_node.path_id == "root" {
            ""
        } else {
            selected_node.path_id.strip_prefix("root/").unwrap_or(&selected_node.path_id)
        };

        let new_lines = if selected_node.is_dir {
            if rel_path.is_empty() {
                self.all_lines.clone()
            } else {
                let prefix1 = format!("{}/", rel_path);
                let prefix2 = format!("{}\\", rel_path);
                self.all_lines.iter()
                    .filter(|line| line.file_path.starts_with(&prefix1) || line.file_path.starts_with(&prefix2) || &line.file_path == rel_path)
                    .cloned()
                    .collect()
            }
        } else {
            if !rel_path.is_empty() {
                self.all_lines.iter()
                    .filter(|line| &line.file_path == rel_path)
                    .cloned()
                    .collect()
            } else {
                self.all_lines.clone()
            }
        };

        self.lines = new_lines;
        self.hunks = self.lines.iter().enumerate()
            .filter(|(_, l)| l.line_type == DiffLineType::HunkHeader)
            .map(|(i, _)| i)
            .collect();
    }

    pub fn update_selected_file_from_cursor(&mut self) {
        if self.visible_nodes.is_empty() {
            return;
        }
        if let Some(line) = self.lines.get(self.cursor_idx) {
            let active_path = &line.file_path;
            if let Some(pos) = self.visible_nodes.iter().position(|node| {
                !node.is_dir && node.file_path.as_ref().map(|p| p == active_path).unwrap_or(false)
            }) {
                self.selected_visible_idx = pos;
            }
        }
    }
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
pub enum TextInputAction {
    EditField {
        entity_iid: u64,
        entity_type: String,
        field_type: String,
    },
    CreateIssue,
    AddReviewComment {
        mr_iid: u64,
        file_path: String,
        line_num: Option<u32>,
        old_line_num: Option<u32>,
    },
}

#[derive(Clone, Debug)]
pub struct TextInput {
    pub title: String,
    pub value: String,
    pub cursor_idx: usize,
    pub action: TextInputAction,
}

pub struct App {
    pub active_tab: Tab,
    pub running: bool,
    pub project_context: String,
    pub gitlab_client: Option<crate::gitlab::client::GitlabClient>,
    pub issues: StatefulTable<crate::gitlab::issues::Issue>,
    pub mrs: StatefulTable<crate::gitlab::mr::MergeRequest>,
    pub pipelines: StatefulTable<crate::gitlab::pipelines::Pipeline>,
    pub search_query: String,
    pub is_typing_search: bool,
    pub selected_pipeline_jobs: Option<Vec<crate::gitlab::pipelines::Job>>,
    pub selected_job_index: Option<usize>,
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
    pub jobs_list_state: TableState,
    pub job_trace_scroll: u16,
    pub issues_scroll: u16,
    pub mrs_scroll: u16,
    pub selected_pipelines: std::collections::HashSet<u64>,
    pub selected_jobs: std::collections::HashSet<u64>,
    pub details_zoomed: bool,
    pub job_trace_needs_scroll_to_bottom: bool,
    pub show_help: bool,
    pub help_search_query: String,
    pub diff_view: Option<DiffView>,
    pub diff_loading: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            active_tab: Tab::default(),
            running: true,
            project_context: "group/repository".to_string(),
            gitlab_client: None,
            issues: StatefulTable::with_items(vec![]),
            mrs: StatefulTable::with_items(vec![]),
            pipelines: StatefulTable::with_items(vec![]),
            search_query: String::new(),
            is_typing_search: false,
            selected_pipeline_jobs: None,
            selected_job_index: None,
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
            jobs_list_state: TableState::default(),
            job_trace_scroll: 0,
            issues_scroll: 0,
            mrs_scroll: 0,
            selected_pipelines: std::collections::HashSet::new(),
            selected_jobs: std::collections::HashSet::new(),
            details_zoomed: false,
            job_trace_needs_scroll_to_bottom: false,
            show_help: false,
            help_search_query: String::new(),
            diff_view: None,
            diff_loading: false,
        }
    }
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tick(&mut self) {}

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn next_tab(&mut self) {
        let current_index = Tab::ALL.iter().position(|t| t == &self.active_tab).unwrap_or(0);
        let next_index = (current_index + 1) % Tab::ALL.len();
        self.active_tab = Tab::ALL[next_index];
        self.selected_pipelines.clear();
        self.selected_jobs.clear();
        self.details_zoomed = false;
        self.update_filter_selection();
    }

    pub fn previous_tab(&mut self) {
        let current_index = Tab::ALL.iter().position(|t| t == &self.active_tab).unwrap_or(0);
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

    pub fn filter_issues_list<'a>(items: &'a [crate::gitlab::issues::Issue], query: &str) -> Vec<&'a crate::gitlab::issues::Issue> {
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
            
            check_match(&format!("#{}", item.iid));
            check_match(&item.iid.to_string());
            check_match(&item.state);
            if item.state == "opened" {
                check_match("open");
            } else if item.state == "closed" {
                check_match("closed");
            }
            check_match(&item.title);
            check_match(&item.author.username);
            check_match(&format!("@{}", item.author.username));
            check_match(&crate::utils::format::time_ago(&item.updated_at));
            check_match(&item.updated_at);
            
            if let Some(m) = &item.milestone {
                check_match(&m.title);
            }
            for label in &item.labels {
                check_match(label);
            }
            for assignee in &item.assignees {
                check_match(&assignee.username);
                check_match(&format!("@{}", assignee.username));
            }
            if let Some(desc) = &item.description {
                check_match(desc);
            }
            
            if let Some(score) = best_score {
                scored_items.push((item, score));
            }
        }
        
        scored_items.sort_by(|a, b| b.1.cmp(&a.1));
        scored_items.into_iter().map(|(item, _)| item).collect()
    }

    pub fn filtered_issues(&self) -> Vec<&crate::gitlab::issues::Issue> {
        Self::filter_issues_list(&self.issues.items, &self.search_query)
    }

    pub fn filter_mrs_list<'a>(items: &'a [crate::gitlab::mr::MergeRequest], query: &str) -> Vec<&'a crate::gitlab::mr::MergeRequest> {
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
            
            check_match(&format!("!{}", item.iid));
            check_match(&item.iid.to_string());
            check_match(&item.state);
            if item.state == "opened" {
                check_match("open");
            } else if item.state == "merged" {
                check_match("merged");
            } else if item.state == "closed" {
                check_match("closed");
            }
            check_match(&item.title);
            check_match(&item.author.username);
            check_match(&format!("@{}", item.author.username));
            check_match(&crate::utils::format::time_ago(&item.updated_at));
            check_match(&item.updated_at);
            
            if let Some(ms) = &item.milestone {
                check_match(&ms.title);
            }
            check_match(&item.target_branch);
            for label in &item.labels {
                check_match(label);
            }
            for assignee in &item.assignees {
                check_match(&assignee.username);
                check_match(&format!("@{}", assignee.username));
            }
            for reviewer in &item.reviewers {
                check_match(&reviewer.username);
                check_match(&format!("@{}", reviewer.username));
            }
            if let Some(desc) = &item.description {
                check_match(desc);
            }
            
            if let Some(score) = best_score {
                scored_items.push((item, score));
            }
        }
        
        scored_items.sort_by(|a, b| b.1.cmp(&a.1));
        scored_items.into_iter().map(|(item, _)| item).collect()
    }

    pub fn filtered_mrs(&self) -> Vec<&crate::gitlab::mr::MergeRequest> {
        Self::filter_mrs_list(&self.mrs.items, &self.search_query)
    }

    pub fn filter_pipelines_list<'a>(
        items: &'a [crate::gitlab::pipelines::Pipeline],
        query: &str,
        pipeline_jobs: &std::collections::HashMap<u64, Vec<crate::gitlab::pipelines::Job>>,
    ) -> Vec<&'a crate::gitlab::pipelines::Pipeline> {
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
            
            check_match(&format!("#{}", item.id));
            check_match(&item.id.to_string());
            check_match(&item.status);
            check_match(&item.r#ref);
            check_match(&crate::utils::format::time_ago(&item.updated_at));
            check_match(&item.updated_at);
            
            if let Some(jobs) = pipeline_jobs.get(&item.id) {
                for job in jobs {
                    check_match(&job.name);
                    check_match(&job.stage);
                    check_match(&job.status);
                }
            }
            
            if let Some(score) = best_score {
                scored_items.push((item, score));
            }
        }
        
        scored_items.sort_by(|a, b| b.1.cmp(&a.1));
        scored_items.into_iter().map(|(item, _)| item).collect()
    }

    pub fn filtered_pipelines(&self) -> Vec<&crate::gitlab::pipelines::Pipeline> {
        Self::filter_pipelines_list(&self.pipelines.items, &self.search_query, &self.pipeline_jobs)
    }

    pub fn filter_runners_list<'a>(items: &'a [crate::gitlab::runners::Runner], query: &str) -> Vec<&'a crate::gitlab::runners::Runner> {
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
            
            check_match(&item.id.to_string());
            if let Some(desc) = &item.description {
                check_match(desc);
            }
            check_match(&item.status);
            let active_str = if item.active { "active" } else { "inactive" };
            check_match(active_str);
            check_match(&item.active.to_string());
            
            if let Some(score) = best_score {
                scored_items.push((item, score));
            }
        }
        
        scored_items.sort_by(|a, b| b.1.cmp(&a.1));
        scored_items.into_iter().map(|(item, _)| item).collect()
    }

    pub fn filtered_runners(&self) -> Vec<&crate::gitlab::runners::Runner> {
        Self::filter_runners_list(&self.runners.items, &self.search_query)
    }

    pub fn filter_releases_list<'a>(items: &'a [crate::gitlab::releases::Release], query: &str) -> Vec<&'a crate::gitlab::releases::Release> {
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
            
            check_match(&item.tag_name);
            check_match(&item.name);
            check_match(&item.released_at);
            check_match(&crate::utils::format::time_ago(&item.released_at));
            
            if let Some(score) = best_score {
                scored_items.push((item, score));
            }
        }
        
        scored_items.sort_by(|a, b| b.1.cmp(&a.1));
        scored_items.into_iter().map(|(item, _)| item).collect()
    }

    pub fn filtered_releases(&self) -> Vec<&crate::gitlab::releases::Release> {
        Self::filter_releases_list(&self.releases.items, &self.search_query)
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(diff_view.visible_nodes[1].file_path.as_deref(), Some("src/app.rs"));
        assert_eq!(diff_view.visible_nodes[1].line_idx, Some(0));
        
        assert_eq!(diff_view.visible_nodes[2].name, "main.rs");
        assert!(!diff_view.visible_nodes[2].is_dir);
        assert_eq!(diff_view.visible_nodes[2].file_path.as_deref(), Some("src/main.rs"));
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
        assert_eq!(color_view.visible_nodes[1].file_path.as_deref(), Some("src/app.rs"));
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
}
