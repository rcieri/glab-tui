use clap::{Parser, Subcommand};
use std::process::Command;

#[derive(Parser)]
#[command(name = "glab-tui")]
#[command(about = "GitLab/GitHub terminal user interface")]
#[command(version = env!("CARGO_PKG_VERSION"))]
pub struct Cli {
    #[arg(
        short = 'r',
        long = "repo",
        help = "Specify git repo context (e.g., group/repo)"
    )]
    pub repo: Option<String>,

    #[arg(
        short = 'd',
        long = "dir",
        help = "Specify local repository directory to run in"
    )]
    pub dir: Option<String>,

    #[arg(short = 'u', long = "update", help = "Check and install updates")]
    pub update: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Check system health and dependency availability
    Doctor,
    /// Remove stale cache entries for repos that no longer exist
    CleanCache {
        /// Preview what would be removed without actually deleting
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// Print the resolved configuration (merge of global + repo-local overrides)
    Config,
    /// List cached data files with sizes
    Cache,
    /// Open an entity in the browser without launching the TUI
    Open {
        /// Entity type: issue, mr, pr, pipeline, job, milestone
        entity: String,
        /// Entity ID (IID for issues/MRs, internal ID for pipelines/jobs)
        id: String,
    },
    /// List recently-used repositories
    Repos,
}

pub async fn run_doctor() {
    let mut has_backend = false;

    println!("glab-tui doctor");
    println!("================");
    println!();

    // ── glab ──
    match Command::new("glab").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let v = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("[PASS] glab: {}", v);
            has_backend = true;
        }
        _ => {
            println!("[FAIL] glab not found — install from https://gitlab.com/gitlab-org/cli");
        }
    }

    // ── gh ──
    match Command::new("gh").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let v = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("[PASS] gh:   {}", v);
            has_backend = true;
        }
        _ => {
            println!("[FAIL] gh not found — install from https://cli.github.com");
        }
    }

    // ── git ──
    match Command::new("git").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let v = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("[PASS] git:  {}", v);
        }
        _ => {
            println!("[FAIL] git not found");
        }
    }

    // ── Config ──
    let config_path = crate::config::Config::config_path();
    match std::fs::read_to_string(&config_path) {
        Ok(content) => {
            let valid = toml::from_str::<toml::Table>(&content).is_ok();
            if valid {
                println!(
                    "[PASS] Config: {} ({} bytes)",
                    config_path.display(),
                    content.len()
                );
            } else {
                println!(
                    "[WARN] Config file exists but is not valid TOML: {}",
                    config_path.display()
                );
            }
        }
        Err(_) => {
            println!(
                "[INFO] No config file found at {} (using defaults)",
                config_path.display()
            );
        }
    }

    // ── Cache ──
    let cache_dir = crate::utils::cache::get_cache_dir();
    if cache_dir.exists() {
        let file_count = std::fs::read_dir(&cache_dir)
            .map(|d| {
                d.filter(|e| e.as_ref().map(|f| f.path().is_file()).unwrap_or(false))
                    .count()
            })
            .unwrap_or(0);
        let total_size = crate::utils::cache::get_cache_dir_size();
        println!(
            "[PASS] Cache:  {} ({} files, {} KB)",
            cache_dir.display(),
            file_count,
            total_size / 1024
        );
    } else {
        println!("[INFO] No cache directory at {}", cache_dir.display());
    }

    // ── Current repo ──
    println!();
    println!("Repository context");
    println!("-------------------");
    match crate::domain::client::get_project_context().await {
        Ok(context) => {
            println!("  Remote:  {}", context);
            let is_github = match Command::new("git")
                .args(["remote", "get-url", "origin"])
                .output()
            {
                Ok(o) if o.status.success() => {
                    String::from_utf8_lossy(&o.stdout).contains("github.com")
                }
                _ => false,
            };
            println!("  Backend: {}", if is_github { "GitHub" } else { "GitLab" });
        }
        Err(_) => {
            println!("  No git remote detected (not inside a git repo?)");
        }
    }

    // ── Terminal ──
    let term = std::env::var("TERM").unwrap_or_else(|_| "unknown".to_string());
    println!("  TERM:    {}", term);

    println!();
    if has_backend {
        println!("Status: OK — at least one backend CLI is available.");
    } else {
        println!("Status: FAIL — neither glab nor gh is installed. At least one is required.");
        std::process::exit(1);
    }
}

pub fn run_clean_cache(dry_run: bool) {
    if dry_run {
        println!("glab-tui clean-cache --dry-run");
        println!("===============================");
        println!("(Preview only — no files will be removed)");
    } else {
        println!("glab-tui clean-cache");
        println!("====================");
    }
    println!();

    let result = crate::utils::cache::clean_cache(dry_run);

    // ── Recent repos ──
    println!("Recent repositories:");
    if result.removed_repos.is_empty() {
        println!("  All {} entries are valid.", result.kept_repos.len());
    } else {
        for r in &result.removed_repos {
            println!("  [REMOVED] {}", r);
        }
    }
    for r in &result.kept_repos {
        println!("  [KEPT]    {}", r);
    }

    // ── Cache files ──
    println!();
    println!("Cache files:");
    if result.removed_files.is_empty() {
        println!(
            "  All {} cache files are valid (no orphaned entries).",
            result.kept_files.len()
        );
    } else {
        for f in &result.removed_files {
            println!("  [REMOVED] {}", f);
        }
    }
    for f in &result.kept_files {
        println!("  [KEPT]    {}", f);
    }

    // ── Summary ──
    println!();
    if dry_run {
        println!(
            "Would remove: {} recent-repo entries, {} cache files ({} KB)",
            result.removed_repos.len(),
            result.removed_files.len(),
            result.total_removed_size / 1024
        );
    } else {
        println!(
            "Removed: {} recent-repo entries, {} cache files ({} KB)",
            result.removed_repos.len(),
            result.removed_files.len(),
            result.total_removed_size / 1024
        );
    }
    println!(
        "Kept:    {} recent-repo entries, {} cache files",
        result.kept_repos.len(),
        result.kept_files.len()
    );
}

pub fn run_config_show() {
    let config = crate::config::Config::load();
    match toml::to_string_pretty(&config) {
        Ok(toml_str) => println!("{}", toml_str),
        Err(e) => eprintln!("Error serializing config: {}", e),
    }
}

pub fn run_cache_list() {
    let cache_dir = crate::utils::cache::get_cache_dir();
    if !cache_dir.exists() {
        println!("Cache directory does not exist: {}", cache_dir.display());
        return;
    }

    println!("Cache directory: {}", cache_dir.display());
    println!();

    let mut entries: Vec<(String, u64, String)> = Vec::new();
    if let Ok(dir_entries) = std::fs::read_dir(&cache_dir) {
        for entry in dir_entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| {
                    let duration = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                    chrono::DateTime::from_timestamp(
                        duration.as_secs() as i64,
                        duration.subsec_nanos(),
                    )
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "unknown".to_string())
                })
                .unwrap_or_else(|| "unknown".to_string());
            entries.push((name, size, modified));
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let total: u64 = entries.iter().map(|(_, s, _)| s).sum();
    for (name, size, modified) in &entries {
        if *name == "recent_repos.json" {
            println!("  {:<40} {:>8} KB  {}", name, size / 1024, modified);
        } else {
            println!("  {:<40} {:>8} KB  {}", name, size / 1024, modified);
        }
    }

    println!();
    println!("Total: {} files, {} KB", entries.len(), total / 1024);
}

pub fn run_open_in_browser(entity: &str, id: &str) {
    let is_github = detect_github();

    let (program, subcommand) = match entity {
        "issue" => {
            if is_github {
                ("gh", vec!["issue", "view", id, "--web"])
            } else {
                ("glab", vec!["issue", "view", id, "-w"])
            }
        }
        "mr" | "pr" => {
            if is_github {
                ("gh", vec!["pr", "view", id, "--web"])
            } else {
                ("glab", vec!["mr", "view", id, "-w"])
            }
        }
        "pipeline" => {
            if is_github {
                ("gh", vec!["run", "view", id, "--web"])
            } else {
                ("glab", vec!["ci", "view", id, "-w"])
            }
        }
        "job" => {
            if is_github {
                eprintln!("Direct job browser URLs are not supported for GitHub Actions.");
                return;
            } else {
                ("glab", vec!["ci", "view", id, "-w"])
            }
        }
        "milestone" => {
            if is_github {
                ("gh", vec!["issue", "list", "--milestone", id, "--web"])
            } else {
                ("glab", vec!["milestone", "view", id, "-w"])
            }
        }
        _ => {
            eprintln!(
                "Unknown entity '{}'. Valid: issue, mr, pr, pipeline, job, milestone",
                entity
            );
            std::process::exit(1);
        }
    };

    println!("Opening {} {} in browser...", entity, id);
    match Command::new(program).args(&subcommand).spawn() {
        Ok(_) => {}
        Err(e) => {
            eprintln!("Failed to run {}: {}", program, e);
            std::process::exit(1);
        }
    }
}

pub fn run_repos_list() {
    println!("Recent repositories:");
    let recent = crate::utils::cache::get_recent_repos();
    if recent.is_empty() {
        println!("  (none)");
    } else {
        for (i, r) in recent.iter().enumerate() {
            let marker = if crate::utils::cache::is_git_repo(r) {
                "[✓]"
            } else {
                "[✗]"
            };
            println!("  {} {}", marker, r);
        }
    }

    println!();
    println!("Sibling repositories:");
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let siblings = crate::utils::cache::get_sibling_repos(&cwd);
    if siblings.is_empty() {
        println!("  (none)");
    } else {
        for s in &siblings {
            println!("  {}", s);
        }
    }
}

pub async fn run_update() {
    println!("Checking for updates...");
    match crate::utils::update::perform_self_update().await {
        Ok(updated) => {
            if updated {
                println!("Successfully updated to the latest version! Please restart glab-tui.");
            } else {
                println!("Already up to date.");
            }
        }
        Err(e) => {
            eprintln!("Update failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn detect_github() -> bool {
    match Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).contains("github.com"),
        _ => false,
    }
}
