use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct ProjectCache {
    pub issues: Vec<crate::gitlab::issues::Issue>,
    pub mrs: Vec<crate::gitlab::mr::MergeRequest>,
    pub pipelines: Vec<crate::gitlab::pipelines::Pipeline>,
    pub runners: Vec<crate::gitlab::runners::Runner>,
    pub releases: Vec<crate::gitlab::releases::Release>,
    pub todos: Vec<crate::gitlab::notifications::Notification>,
    pub milestones: Vec<crate::gitlab::milestones::Milestone>,
    pub enabled_columns: HashMap<String, Vec<String>>,
    pub group_by_column: Option<String>,
    pub group_ascending: bool,
    pub column_filters: HashMap<String, HashMap<String, Vec<String>>>,
}

fn get_cache_file_path(project_context: &str) -> PathBuf {
    let safe_name = project_context.replace('/', "_").replace('\\', "_");
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());

    let mut path = PathBuf::from(home);
    path.push(".cache");
    path.push("glab-tui");
    let _ = fs::create_dir_all(&path);
    path.push(format!("{}.json", safe_name));
    path
}

pub fn load_cache(project_context: &str) -> ProjectCache {
    let path = get_cache_file_path(project_context);
    if let Ok(content) = fs::read_to_string(path) {
        if let Ok(cache) = serde_json::from_str(&content) {
            return cache;
        }
    }
    ProjectCache::default()
}

pub fn save_cache(project_context: &str, cache: &ProjectCache) {
    let path = get_cache_file_path(project_context);
    if let Ok(content) = serde_json::to_string(cache) {
        let _ = fs::write(path, content);
    }
}

fn get_recent_repos_file_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());

    let mut path = PathBuf::from(home);
    path.push(".cache");
    path.push("glab-tui");
    let _ = fs::create_dir_all(&path);
    path.push("recent_repos.json");
    path
}

pub fn get_recent_repos() -> Vec<String> {
    let path = get_recent_repos_file_path();
    if let Ok(content) = fs::read_to_string(path) {
        if let Ok(repos) = serde_json::from_str::<Vec<String>>(&content) {
            return repos;
        }
    }
    Vec::new()
}

pub fn add_recent_repo(repo_path: &str) {
    let mut repos = get_recent_repos();
    let repo_path = repo_path.to_string();
    if let Some(pos) = repos.iter().position(|r| r == &repo_path) {
        repos.remove(pos);
    }
    repos.insert(0, repo_path);
    repos.truncate(20);

    let path = get_recent_repos_file_path();
    if let Ok(content) = serde_json::to_string(&repos) {
        let _ = fs::write(path, content);
    }
}

pub fn is_git_repo(path: &str) -> bool {
    let mut p = PathBuf::from(path);
    p.push(".git");
    p.exists()
}

pub fn get_sibling_repos(current_dir: &str) -> Vec<String> {
    let mut sibling_repos = Vec::new();
    if let Ok(path) = PathBuf::from(current_dir).canonicalize() {
        if let Some(parent) = path.parent() {
            if let Ok(entries) = fs::read_dir(parent) {
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    if entry_path.is_dir() {
                        let mut git_path = entry_path.clone();
                        git_path.push(".git");
                        if git_path.exists() {
                            if let Some(p_str) = entry_path.to_str() {
                                sibling_repos.push(p_str.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    sibling_repos
}

pub fn get_repos_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("GLAB_TUI_REPOS_DIR") {
        PathBuf::from(dir)
    } else {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| cwd.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

pub fn get_repos_in_dir(repos_dir: &std::path::Path) -> Vec<String> {
    let mut repos = Vec::new();
    if let Ok(entries) = std::fs::read_dir(repos_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let mut git_path = path.clone();
                git_path.push(".git");
                if git_path.exists() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        repos.push(name.to_string());
                    }
                }
            }
        }
    }
    repos.sort();
    repos
}

pub fn get_switchable_repos() -> Vec<String> {
    let repos_dir = get_repos_dir();
    let available_repos = get_repos_in_dir(&repos_dir);
    let recent_paths = get_recent_repos();

    let mut sorted_repos = Vec::new();
    for path_str in recent_paths {
        let path = std::path::PathBuf::from(path_str);
        if let Some(parent) = path.parent() {
            if parent == repos_dir {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    let name_str = name.to_string();
                    if available_repos.contains(&name_str) && !sorted_repos.contains(&name_str) {
                        sorted_repos.push(name_str);
                    }
                }
            }
        }
    }

    for repo in available_repos {
        if !sorted_repos.contains(&repo) {
            sorted_repos.push(repo);
        }
    }

    sorted_repos
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_is_git_repo() {
        let dir = tempdir().unwrap();
        let path_str = dir.path().to_str().unwrap();
        assert!(!is_git_repo(path_str));

        let git_dir = dir.path().join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        assert!(is_git_repo(path_str));
    }

    #[test]
    fn test_get_sibling_repos() {
        let parent = tempdir().unwrap();
        let repo1 = parent.path().join("repo1");
        let repo2 = parent.path().join("repo2");
        let non_repo = parent.path().join("non_repo");

        fs::create_dir_all(&repo1.join(".git")).unwrap();
        fs::create_dir_all(&repo2.join(".git")).unwrap();
        fs::create_dir_all(&non_repo).unwrap();

        let repo1_str = repo1.to_str().unwrap();
        let siblings = get_sibling_repos(repo1_str);

        let has_repo2 = siblings.iter().any(|s| s.contains("repo2"));
        let has_non_repo = siblings.iter().any(|s| s.contains("non_repo"));

        assert!(has_repo2, "siblings should find repo2");
        assert!(!has_non_repo, "siblings should not find non_repo");
    }

    #[test]
    fn test_get_repos_in_dir() {
        let parent = tempdir().unwrap();
        let repo1 = parent.path().join("repo1");
        let repo2 = parent.path().join("repo2");
        let non_repo = parent.path().join("non_repo");

        fs::create_dir_all(&repo1.join(".git")).unwrap();
        fs::create_dir_all(&repo2.join(".git")).unwrap();
        fs::create_dir_all(&non_repo).unwrap();

        let repos = get_repos_in_dir(parent.path());
        assert_eq!(repos.len(), 2);
        assert!(repos.contains(&"repo1".to_string()));
        assert!(repos.contains(&"repo2".to_string()));
        assert!(!repos.contains(&"non_repo".to_string()));
    }

    #[test]
    fn test_repos_dir_env_var() {
        let temp_dir = tempdir().unwrap();
        let path_str = temp_dir.path().to_str().unwrap().to_string();

        unsafe {
            std::env::set_var("GLAB_TUI_REPOS_DIR", &path_str);
        }
        let repos_dir = get_repos_dir();
        assert_eq!(repos_dir, temp_dir.path().to_path_buf());
        unsafe {
            std::env::remove_var("GLAB_TUI_REPOS_DIR");
        }
    }
}
