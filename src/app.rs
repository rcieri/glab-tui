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
    pub fn get_filtered_items(&self) -> Vec<String> {
        let query = self.search_query.trim();
        let mut items = if query.is_empty() {
            self.all_items.clone()
        } else {
            let matcher = SkimMatcherV2::default();
            let mut scored: Vec<(String, i64)> = self.all_items.iter()
                .filter_map(|item| {
                    matcher.fuzzy_match(item, query).map(|score| (item.clone(), score))
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            scored.into_iter().map(|(item, _)| item).collect()
        };
            
        if !query.is_empty() {
            let exact_match = self.all_items.iter().any(|item| item.to_lowercase() == query.to_lowercase());
            if !exact_match {
                items.insert(0, format!("+ Create \"{}\"", query));
            }
        }
        items
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
}
