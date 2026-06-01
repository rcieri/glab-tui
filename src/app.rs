use crate::utils::ui::StatefulTable;
use ratatui::widgets::{ListState, TableState};

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
        let query = self.search_query.to_lowercase();
        let mut items: Vec<String> = self.all_items.iter()
            .filter(|item| item.to_lowercase().contains(&query))
            .cloned()
            .collect();
            
        if !query.trim().is_empty() {
            let exact_match = self.all_items.iter().any(|item| item.to_lowercase() == query.trim());
            if !exact_match {
                items.insert(0, format!("+ Create \"{}\"", self.search_query.trim()));
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
        let sq = query.to_lowercase();
        items.iter()
            .filter(|i| {
                if sq.is_empty() {
                    return true;
                }
                // ID
                if format!("#{}", i.iid).contains(&sq) || i.iid.to_string().contains(&sq) {
                    return true;
                }
                // State
                if i.state.to_lowercase().contains(&sq) || (i.state == "opened" && "open".contains(&sq)) || (i.state == "closed" && "closed".contains(&sq)) {
                    return true;
                }
                // Title
                if i.title.to_lowercase().contains(&sq) {
                    return true;
                }
                // Author
                if i.author.username.to_lowercase().contains(&sq) || format!("@{}", i.author.username).to_lowercase().contains(&sq) {
                    return true;
                }
                // Updated Time
                if crate::utils::format::time_ago(&i.updated_at).to_lowercase().contains(&sq) || i.updated_at.to_lowercase().contains(&sq) {
                    return true;
                }
                // Milestone
                if let Some(m) = &i.milestone {
                    if m.title.to_lowercase().contains(&sq) {
                        return true;
                    }
                }
                // Labels
                for label in &i.labels {
                    if label.to_lowercase().contains(&sq) {
                        return true;
                    }
                }
                // Assignees
                for assignee in &i.assignees {
                    if assignee.username.to_lowercase().contains(&sq) || format!("@{}", assignee.username).to_lowercase().contains(&sq) {
                        return true;
                    }
                }
                // Description
                if let Some(desc) = &i.description {
                    if desc.to_lowercase().contains(&sq) {
                        return true;
                    }
                }
                false
            })
            .collect()
    }

    pub fn filtered_issues(&self) -> Vec<&crate::gitlab::issues::Issue> {
        Self::filter_issues_list(&self.issues.items, &self.search_query)
    }

    pub fn filter_mrs_list<'a>(items: &'a [crate::gitlab::mr::MergeRequest], query: &str) -> Vec<&'a crate::gitlab::mr::MergeRequest> {
        let sq = query.to_lowercase();
        items.iter()
            .filter(|m| {
                if sq.is_empty() {
                    return true;
                }
                // ID
                if format!("!{}", m.iid).contains(&sq) || m.iid.to_string().contains(&sq) {
                    return true;
                }
                // State
                if m.state.to_lowercase().contains(&sq) || (m.state == "opened" && "open".contains(&sq)) || (m.state == "merged" && "merged".contains(&sq)) || (m.state == "closed" && "closed".contains(&sq)) {
                    return true;
                }
                // Title
                if m.title.to_lowercase().contains(&sq) {
                    return true;
                }
                // Author
                if m.author.username.to_lowercase().contains(&sq) || format!("@{}", m.author.username).to_lowercase().contains(&sq) {
                    return true;
                }
                // Updated Time
                if crate::utils::format::time_ago(&m.updated_at).to_lowercase().contains(&sq) || m.updated_at.to_lowercase().contains(&sq) {
                    return true;
                }
                // Milestone
                if let Some(ms) = &m.milestone {
                    if ms.title.to_lowercase().contains(&sq) {
                        return true;
                    }
                }
                // Target Branch
                if m.target_branch.to_lowercase().contains(&sq) {
                    return true;
                }
                // Labels
                for label in &m.labels {
                    if label.to_lowercase().contains(&sq) {
                        return true;
                    }
                }
                // Assignees
                for assignee in &m.assignees {
                    if assignee.username.to_lowercase().contains(&sq) || format!("@{}", assignee.username).to_lowercase().contains(&sq) {
                        return true;
                    }
                }
                // Reviewers
                for reviewer in &m.reviewers {
                    if reviewer.username.to_lowercase().contains(&sq) || format!("@{}", reviewer.username).to_lowercase().contains(&sq) {
                        return true;
                    }
                }
                // Description
                if let Some(desc) = &m.description {
                    if desc.to_lowercase().contains(&sq) {
                        return true;
                    }
                }
                false
            })
            .collect()
    }

    pub fn filtered_mrs(&self) -> Vec<&crate::gitlab::mr::MergeRequest> {
        Self::filter_mrs_list(&self.mrs.items, &self.search_query)
    }

    pub fn filter_pipelines_list<'a>(
        items: &'a [crate::gitlab::pipelines::Pipeline],
        query: &str,
        pipeline_jobs: &std::collections::HashMap<u64, Vec<crate::gitlab::pipelines::Job>>,
    ) -> Vec<&'a crate::gitlab::pipelines::Pipeline> {
        let sq = query.to_lowercase();
        items.iter()
            .filter(|p| {
                if sq.is_empty() {
                    return true;
                }
                // ID
                if format!("#{}", p.id).contains(&sq) || p.id.to_string().contains(&sq) {
                    return true;
                }
                // Status
                if p.status.to_lowercase().contains(&sq) {
                    return true;
                }
                // Ref
                if p.r#ref.to_lowercase().contains(&sq) {
                    return true;
                }
                // Updated Time
                if crate::utils::format::time_ago(&p.updated_at).to_lowercase().contains(&sq) || p.updated_at.to_lowercase().contains(&sq) {
                    return true;
                }
                // Pipeline Jobs details
                if let Some(jobs) = pipeline_jobs.get(&p.id) {
                    for job in jobs {
                        if job.name.to_lowercase().contains(&sq) || job.stage.to_lowercase().contains(&sq) || job.status.to_lowercase().contains(&sq) {
                            return true;
                        }
                    }
                }
                false
            })
            .collect()
    }

    pub fn filtered_pipelines(&self) -> Vec<&crate::gitlab::pipelines::Pipeline> {
        Self::filter_pipelines_list(&self.pipelines.items, &self.search_query, &self.pipeline_jobs)
    }

    pub fn filter_runners_list<'a>(items: &'a [crate::gitlab::runners::Runner], query: &str) -> Vec<&'a crate::gitlab::runners::Runner> {
        let sq = query.to_lowercase();
        items.iter()
            .filter(|r| {
                if sq.is_empty() {
                    return true;
                }
                // ID
                if r.id.to_string().contains(&sq) {
                    return true;
                }
                // Description
                if let Some(desc) = &r.description {
                    if desc.to_lowercase().contains(&sq) {
                        return true;
                    }
                }
                // Status
                if r.status.to_lowercase().contains(&sq) {
                    return true;
                }
                // Active
                let active_str = if r.active { "active" } else { "inactive" };
                if active_str.contains(&sq) || r.active.to_string().contains(&sq) {
                    return true;
                }
                false
            })
            .collect()
    }

    pub fn filtered_runners(&self) -> Vec<&crate::gitlab::runners::Runner> {
        Self::filter_runners_list(&self.runners.items, &self.search_query)
    }

    pub fn filter_releases_list<'a>(items: &'a [crate::gitlab::releases::Release], query: &str) -> Vec<&'a crate::gitlab::releases::Release> {
        let sq = query.to_lowercase();
        items.iter()
            .filter(|r| {
                if sq.is_empty() {
                    return true;
                }
                // Tag Name
                if r.tag_name.to_lowercase().contains(&sq) {
                    return true;
                }
                // Name
                if r.name.to_lowercase().contains(&sq) {
                    return true;
                }
                // Date
                if r.released_at.to_lowercase().contains(&sq) || crate::utils::format::time_ago(&r.released_at).to_lowercase().contains(&sq) {
                    return true;
                }
                false
            })
            .collect()
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
