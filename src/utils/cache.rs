use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct ProjectCache {
    pub issues: Vec<crate::domain::issues::Issue>,
    pub mrs: Vec<crate::domain::mr::MergeRequest>,
    pub pipelines: Vec<crate::domain::pipelines::Pipeline>,
    pub runners: Vec<crate::domain::runners::Runner>,
    pub releases: Vec<crate::domain::releases::Release>,
    pub todos: Vec<crate::domain::notifications::Notification>,
    pub milestones: Vec<crate::domain::milestones::Milestone>,
    pub pipeline_jobs: HashMap<u64, Vec<crate::domain::pipelines::Job>>,
    pub branches: Vec<crate::domain::branches::Branch>,
    pub environments: Vec<crate::domain::deployments::Environment>,
    #[serde(default)]
    pub milestone_issues: HashMap<u64, Vec<crate::domain::issues::Issue>>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub label_colors: HashMap<String, String>,
    #[serde(default)]
    pub members: Vec<String>,
}

fn get_cache_file_path(project_context: &str) -> PathBuf {
    let mut path = get_cache_dir();
    let _ = fs::create_dir_all(&path);
    path.push(cache_file_name(project_context));
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
    let mut path = get_cache_dir();
    let _ = fs::create_dir_all(&path);
    path.push("recent_repos.json");
    path
}

pub fn get_recent_repos() -> Vec<String> {
    let path = get_recent_repos_file_path();
    if let Ok(content) = fs::read_to_string(&path) {
        if let Ok(repos) = serde_json::from_str::<Vec<String>>(&content) {
            return repos
                .into_iter()
                .filter(|r| std::path::Path::new(r).is_absolute())
                .collect();
        }
    }
    Vec::new()
}

pub fn add_recent_repo(repo_path: &str) {
    // Only cache directories that are git repos
    if !is_git_repo(repo_path) {
        return;
    }
    let mut repos = get_recent_repos();
    // Store only absolute paths
    let abs_path = std::path::PathBuf::from(repo_path)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(repo_path));
    let repo_path = abs_path.to_string_lossy().into_owned();
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

pub fn get_cache_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    let mut path = PathBuf::from(home);
    path.push(".cache");
    path.push("glab-tui");
    path
}

pub fn cache_file_name(project_context: &str) -> String {
    let safe_name = project_context.replace('/', "_").replace('\\', "_");
    format!("{}.json", safe_name)
}

pub fn get_cache_dir_size() -> u64 {
    let dir = get_cache_dir();
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            total += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    total
}

#[derive(Debug)]
pub struct CleanCacheResult {
    pub kept_repos: Vec<String>,
    pub removed_repos: Vec<String>,
    pub kept_files: Vec<String>,
    pub removed_files: Vec<String>,
    pub total_removed_size: u64,
}

pub fn clean_cache(dry_run: bool) -> CleanCacheResult {
    let mut result = CleanCacheResult {
        kept_repos: Vec::new(),
        removed_repos: Vec::new(),
        kept_files: Vec::new(),
        removed_files: Vec::new(),
        total_removed_size: 0,
    };

    // ── Prune recent_repos.json dead entries ──
    let recent_path = get_recent_repos_file_path();
    let recent_repos = get_recent_repos();
    let mut live_repos: Vec<String> = Vec::new();
    let mut dead_repos: Vec<String> = Vec::new();

    for r in &recent_repos {
        if is_git_repo(r) {
            live_repos.push(r.clone());
        } else {
            dead_repos.push(r.clone());
        }
    }

    result.kept_repos = live_repos.clone();
    result.removed_repos = dead_repos.clone();

    if !dry_run && !dead_repos.is_empty() {
        if let Ok(content) = serde_json::to_string(&live_repos) {
            let _ = fs::write(&recent_path, content);
        }
    }

    // ── Prune orphaned cache files ──
    // Collect valid cache file names from live repos' project contexts
    let mut valid_cache_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    for repo_path in &live_repos {
        if let Some(context) = get_project_context_for_path(repo_path) {
            valid_cache_files.insert(cache_file_name(&context));
        }
    }

    let cache_dir = get_cache_dir();
    if let Ok(entries) = fs::read_dir(&cache_dir) {
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            // Skip non-JSON files and special files
            if file_name == "recent_repos.json" || !file_name.ends_with(".json") {
                continue;
            }
            if valid_cache_files.contains(&file_name) {
                result.kept_files.push(file_name);
            } else {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                result.total_removed_size += size;
                result.removed_files.push(file_name.clone());
                if !dry_run {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }

    result
}

fn get_project_context_for_path(repo_path: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["-C", repo_path, "remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout);
    crate::git_helpers::parse_project_path(&url)
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

    // Recent repos are always absolute paths — show only valid git repos
    for abs_path in recent_paths {
        if !sorted_repos.contains(&abs_path) && is_git_repo(&abs_path) {
            sorted_repos.push(abs_path);
        }
    }

    // Add repos found in repos_dir as absolute paths
    for dirname in available_repos {
        let abs = repos_dir.join(&dirname).to_string_lossy().into_owned();
        if !sorted_repos.contains(&abs) && !sorted_repos.contains(&dirname) {
            sorted_repos.push(abs);
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

    #[test]
    fn test_project_cache_roundtrip() {
        let mut cache = ProjectCache::default();
        cache.labels = vec!["bug".to_string(), "enhancement".to_string()];
        cache.members = vec!["@user1".to_string(), "@user2".to_string()];

        let serialized = serde_json::to_string(&cache).unwrap();
        let deserialized: ProjectCache = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.labels[0], "bug");
        assert_eq!(deserialized.members[1], "@user2");
    }
}
