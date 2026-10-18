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

    /// The project a listed issue, MR, milestone or pipeline belongs to. Rows fetched
    /// without a project of their own stand for the scope itself.
    pub fn project_of<'a>(&'a self, row_project_path: &'a str) -> &'a str {
        if row_project_path.is_empty() {
            self.as_str()
        } else {
            row_project_path
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self, Scope::Group(_))
    }

    pub fn is_repository(&self) -> bool {
        matches!(self, Scope::Repository(_))
    }

    /// Whether `project` is the repository in scope, or a project inside the
    /// group in scope (subgroups included). Paths compare case-insensitively,
    /// as GitLab and GitHub both treat them.
    pub fn contains_project(&self, project: &str) -> bool {
        match self {
            Scope::Repository(repo) => repo.eq_ignore_ascii_case(project),
            Scope::Group(group) => {
                project
                    .get(..group.len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case(group))
                    && project[group.len()..].starts_with('/')
            }
        }
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
            Scope::Group(s) => format!("Group: {s}"),
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

#[cfg(test)]
mod tests {
    use super::Scope;

    #[test]
    fn repository_scope_contains_only_its_own_project() {
        let scope = Scope::Repository("Owner/Repo".to_string());
        assert!(scope.contains_project("owner/repo"));
        assert!(!scope.contains_project("owner/repo-fork"));
        assert!(!scope.contains_project("other/repo"));
    }

    #[test]
    fn group_scope_contains_projects_of_the_group_and_its_subgroups() {
        let scope = Scope::Group("acme".to_string());
        assert!(scope.contains_project("acme/api"));
        assert!(scope.contains_project("ACME/platform/api"));
        assert!(!scope.contains_project("acme"));
        assert!(!scope.contains_project("acme-labs/api"));
        assert!(!scope.contains_project("other/acme/api"));
    }
}
