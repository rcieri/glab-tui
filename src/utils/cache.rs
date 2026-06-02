use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct ProjectCache {
    pub issues: Vec<crate::gitlab::issues::Issue>,
    pub mrs: Vec<crate::gitlab::mr::MergeRequest>,
    pub pipelines: Vec<crate::gitlab::pipelines::Pipeline>,
}

fn get_cache_file_path(project_context: &str) -> PathBuf {
    let safe_name = project_context.replace('/', "_").replace('\\', "_");
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    
    let mut path = PathBuf::from(home);
    path.push(".glab-tui-cache");
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
