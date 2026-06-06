use anyhow::Result;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct WikiPage {
    pub title: String,
    pub path: String,
    pub content: String,
}

fn get_wiki_dir(project_context: &str) -> PathBuf {
    let safe_name = project_context.replace('/', "_").replace('\\', "_");
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());

    let mut path = PathBuf::from(home);
    path.push(".glab-tui-cache");
    path.push("wikis");
    path.push(safe_name);
    path
}

pub async fn load_wiki_pages(project_context: &str) -> Result<Vec<WikiPage>> {
    let wiki_dir = get_wiki_dir(project_context);

    // Get remote URL
    let origin_output = tokio::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .await?;

    let origin_url = String::from_utf8_lossy(&origin_output.stdout)
        .trim()
        .to_string();
    if origin_url.is_empty() {
        anyhow::bail!("No origin remote found to determine Wiki URL");
    }

    // Construct Wiki URL
    let wiki_url = if origin_url.ends_with(".git") {
        format!("{}.wiki.git", origin_url.strip_suffix(".git").unwrap())
    } else {
        format!("{}.wiki.git", origin_url)
    };

    // Clone or pull
    if wiki_dir.exists() {
        // Run git pull in wiki_dir
        let _ = tokio::process::Command::new("git")
            .arg("pull")
            .current_dir(&wiki_dir)
            .output()
            .await;
    } else {
        // Create parent dir and clone
        let _ = fs::create_dir_all(wiki_dir.parent().unwrap());
        let clone_status = tokio::process::Command::new("git")
            .args(["clone", &wiki_url, wiki_dir.to_string_lossy().as_ref()])
            .status()
            .await?;
        if !clone_status.success() {
            anyhow::bail!("Failed to clone wiki repository from {}", wiki_url);
        }
    }

    // Read all markdown files in wiki_dir
    let mut pages = Vec::new();
    if let Ok(entries) = fs::read_dir(&wiki_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "md") {
                if let Some(file_name) = path.file_stem().and_then(|s| s.to_str()) {
                    let title = file_name.replace('-', " ").replace('_', " ");
                    if let Ok(content) = fs::read_to_string(&path) {
                        pages.push(WikiPage {
                            title,
                            path: path.to_string_lossy().into_owned(),
                            content,
                        });
                    }
                }
            }
        }
    }

    // Sort pages by title
    pages.sort_by(|a, b| a.title.cmp(&b.title));

    Ok(pages)
}
