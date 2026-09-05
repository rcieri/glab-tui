#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Scope {
    Repository(String),
    Group(String),
}

impl Scope {
    pub fn as_str(&self) -> &str {
        match self {
            Scope::Repository(s) | Scope::Group(s) => s,
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self, Scope::Group(_))
    }

    pub fn is_repository(&self) -> bool {
        matches!(self, Scope::Repository(_))
    }

    pub fn cli_repo_arg(&self) -> Option<(&str, &str)> {
        match self {
            Scope::Repository(s) => Some(("-R", s.as_str())),
            Scope::Group(_) => None,
        }
    }

    pub fn cli_group_arg(&self) -> Option<(&str, &str)> {
        match self {
            Scope::Repository(_) => None,
            Scope::Group(s) => Some(("--group", s.as_str())),
        }
    }

    pub fn api_path_prefix(&self) -> String {
        match self {
            Scope::Repository(s) => format!("projects/{}", s.replace('/', "%2F")),
            Scope::Group(s) => format!("groups/{}", s.replace('/', "%2F")),
        }
    }

    pub fn display(&self) -> String {
        match self {
            Scope::Repository(s) => s.clone(),
            Scope::Group(s) => format!("Group: {}", s),
        }
    }
}

impl Default for Scope {
    fn default() -> Self {
        Scope::Repository(String::new())
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}
