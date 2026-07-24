use clap::{Parser, Subcommand};
use std::process::Command;

// ── ANSI color helpers ──
const C_RESET: &str = "\x1b[0m";
const C_BOLD: &str = "\x1b[1m";
const C_DIM: &str = "\x1b[2m";
const C_GREEN: &str = "\x1b[32m";
const C_RED: &str = "\x1b[31m";
const C_YELLOW: &str = "\x1b[33m";
const C_BLUE: &str = "\x1b[34m";
const C_CYAN: &str = "\x1b[36m";

fn styled(text: &str, code: &str) -> String {
    format!("{}{}{}", code, text, C_RESET)
}

fn pass(label: &str, detail: &str) {
    println!(" {} {}", styled(label, C_GREEN), detail);
}

fn fail(label: &str, detail: &str) {
    println!(" {} {}", styled(label, C_RED), detail);
}

fn warn(label: &str, detail: &str) {
    println!(" {} {}", styled(label, C_YELLOW), detail);
}

fn info(label: &str, detail: &str) {
    println!(" {} {}", styled(label, C_DIM), detail);
}

fn header(text: &str) {
    println!("{}", styled(text, &format!("{}{}", C_BOLD, C_CYAN)));
}

fn subheader(text: &str) {
    println!("{}", styled(text, C_BOLD));
}

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

    header("glab-tui doctor");
    println!("{}", styled("================", C_CYAN));
    println!();

    // ── glab ──
    match Command::new("glab").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let v = String::from_utf8_lossy(&output.stdout).trim().to_string();
            pass("[PASS]", &format!("glab: {}", v));
            has_backend = true;
        }
        _ => {
            fail(
                "[FAIL]",
                "glab not found — install from https://gitlab.com/gitlab-org/cli",
            );
        }
    }

    // ── gh ──
    match Command::new("gh").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let v = String::from_utf8_lossy(&output.stdout).trim().to_string();
            pass("[PASS]", &format!("gh:   {}", v));
            has_backend = true;
        }
        _ => {
            fail(
                "[FAIL]",
                "gh not found — install from https://cli.github.com",
            );
        }
    }

    // ── git ──
    match Command::new("git").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let v = String::from_utf8_lossy(&output.stdout).trim().to_string();
            pass("[PASS]", &format!("git:  {}", v));
        }
        _ => {
            fail("[FAIL]", "git not found");
        }
    }

    // ── Config ──
    let config_path = crate::config::Config::config_path();
    match std::fs::read_to_string(&config_path) {
        Ok(content) => {
            let valid = toml::from_str::<toml::Table>(&content).is_ok();
            if valid {
                pass(
                    "[PASS]",
                    &format!(
                        "Config: {} ({} bytes)",
                        config_path.display(),
                        content.len()
                    ),
                );
            } else {
                warn(
                    "[WARN]",
                    &format!(
                        "Config file exists but is not valid TOML: {}",
                        config_path.display()
                    ),
                );
            }
        }
        Err(_) => {
            info(
                "[INFO]",
                &format!(
                    "No config file found at {} (using defaults)",
                    config_path.display()
                ),
            );
        }
    }

    // Repo-local config paths
    if let Some(root) = find_git_root() {
        let repo_configs = [
            root.join(".glab-tui").join("config.toml"),
            root.join(".config").join("glab-tui").join("config.toml"),
        ];
        for repo_path in &repo_configs {
            if repo_path.exists() {
                let size = std::fs::metadata(repo_path).map(|m| m.len()).unwrap_or(0);
                info(
                    "      ",
                    &format!("Repo: {} ({} bytes)", repo_path.display(), size),
                );
            }
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
        pass(
            "[PASS]",
            &format!(
                "Cache:  {} ({} files, {} KB)",
                cache_dir.display(),
                file_count,
                total_size / 1024
            ),
        );
    } else {
        info(
            "[INFO]",
            &format!("No cache directory at {}", cache_dir.display()),
        );
    }

    // ── Current repo ──
    println!();
    subheader("Repository context");
    println!("{}", styled("-------------------", C_DIM));
    match crate::domain::client::get_project_context().await {
        Ok(context) => {
            println!("  Remote:  {}", styled(&context, C_BOLD));
            let is_github = detect_github();
            let backend = if is_github {
                styled("GitHub", C_BLUE)
            } else {
                styled("GitLab", C_YELLOW)
            };
            println!("  Backend: {}", backend);
        }
        Err(_) => {
            println!(
                "  {}",
                styled("No git remote detected (not inside a git repo?)", C_DIM)
            );
        }
    }

    // ── Terminal ──
    let term = std::env::var("TERM").unwrap_or_else(|_| "unknown".to_string());
    println!("  TERM:    {}", styled(&term, C_DIM));

    println!();
    if has_backend {
        println!(
            "{} {}",
            styled("Status:", C_BOLD),
            styled("OK — at least one backend CLI is available.", C_GREEN)
        );
    } else {
        println!(
            "{} {}",
            styled("Status:", C_BOLD),
            styled(
                "FAIL — neither glab nor gh is installed. At least one is required.",
                C_RED
            )
        );
        std::process::exit(1);
    }
}

pub fn run_clean_cache(dry_run: bool) {
    if dry_run {
        header("glab-tui clean-cache --dry-run");
        println!("{}", styled("===============================", C_CYAN));
        info("[INFO]", "(Preview only — no files will be removed)");
    } else {
        header("glab-tui clean-cache");
        println!("{}", styled("====================", C_CYAN));
    }
    println!();

    let result = crate::utils::cache::clean_cache(dry_run);

    // ── Recent repos ──
    subheader("Recent repositories:");
    if result.removed_repos.is_empty() {
        pass(
            "  OK",
            &format!("All {} entries are valid.", result.kept_repos.len()),
        );
    } else {
        for r in &result.removed_repos {
            println!("  {} {}", styled("[REMOVED]", C_RED), r);
        }
    }
    for r in &result.kept_repos {
        println!("  {} {}", styled("[KEPT]   ", C_GREEN), styled(r, C_DIM));
    }

    // ── Cache files ──
    println!();
    subheader("Cache files:");
    if result.removed_files.is_empty() {
        pass(
            "  OK",
            &format!(
                "All {} cache files are valid (no orphaned entries).",
                result.kept_files.len()
            ),
        );
    } else {
        for f in &result.removed_files {
            println!("  {} {}", styled("[REMOVED]", C_RED), f);
        }
    }
    for f in &result.kept_files {
        println!("  {} {}", styled("[KEPT]   ", C_GREEN), styled(f, C_DIM));
    }

    // ── Summary ──
    println!();
    let (action, dir_repos, dir_files, dir_kb) = (
        if dry_run { "Would remove" } else { "Removed" },
        result.removed_repos.len(),
        result.removed_files.len(),
        result.total_removed_size / 1024,
    );
    let summary_style = if dir_repos + dir_files > 0 {
        C_YELLOW
    } else {
        C_DIM
    };
    println!(
        "  {}: {} recent-repo entries, {} cache files ({} KB)",
        styled(action, summary_style),
        dir_repos,
        dir_files,
        dir_kb
    );
    println!(
        "  {}:    {} recent-repo entries, {} cache files",
        styled("Kept", C_GREEN),
        result.kept_repos.len(),
        result.kept_files.len()
    );
}

fn find_git_root() -> Option<std::path::PathBuf> {
    let mut current = std::env::current_dir().ok()?;
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        if !current.pop() {
            break;
        }
    }
    None
}

pub fn run_cache_list() {
    let cache_dir = crate::utils::cache::get_cache_dir();
    if !cache_dir.exists() {
        warn(
            "[WARN]",
            &format!("Cache directory does not exist: {}", cache_dir.display()),
        );
        return;
    }

    println!(
        "{} {}",
        styled("Cache directory:", C_BOLD),
        styled(&cache_dir.display().to_string(), C_DIM)
    );
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
        let name_style = if name == "recent_repos.json" {
            styled(name, C_DIM)
        } else {
            name.clone()
        };
        println!(
            "  {:<40} {:>9}  {}",
            name_style,
            styled(&format!("{} KB", size / 1024), C_BLUE),
            styled(modified, C_DIM)
        );
    }

    println!();
    println!(
        "  {}",
        styled(
            &format!("Total: {} files, {} KB", entries.len(), total / 1024),
            C_BOLD
        )
    );
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
                eprintln!(
                    "{}",
                    styled(
                        "Direct job browser URLs are not supported for GitHub Actions.",
                        C_YELLOW
                    )
                );
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
                "{} Unknown entity '{}'. Valid: issue, mr, pr, pipeline, job, milestone",
                styled("error:", C_RED),
                styled(entity, C_BOLD)
            );
            std::process::exit(1);
        }
    };

    println!(
        "{} {} {} {} ...",
        styled("Opening", C_GREEN),
        styled(entity, C_BOLD),
        styled(id, C_BOLD),
        styled("in browser", C_GREEN)
    );
    match Command::new(program).args(&subcommand).spawn() {
        Ok(_) => {}
        Err(e) => {
            eprintln!(
                "{} Failed to run {}: {}",
                styled("error:", C_RED),
                styled(program, C_BOLD),
                e
            );
            std::process::exit(1);
        }
    }
}

pub fn run_repos_list() {
    subheader("Recent repositories:");
    let recent = crate::utils::cache::get_recent_repos();
    if recent.is_empty() {
        println!("  {}", styled("(none)", C_DIM));
    } else {
        for r in &recent {
            if crate::utils::cache::is_git_repo(r) {
                println!("  {} {}", styled("[✓]", C_GREEN), r);
            } else {
                println!("  {} {}", styled("[✗]", C_RED), styled(r, C_DIM));
            }
        }
    }

    println!();
    subheader("Sibling repositories:");
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let siblings = crate::utils::cache::get_sibling_repos(&cwd);
    if siblings.is_empty() {
        println!("  {}", styled("(none)", C_DIM));
    } else {
        for s in &siblings {
            println!("  {}", styled(s, C_DIM));
        }
    }
}

pub async fn run_update() {
    println!("Checking for updates...");
    match crate::utils::update::perform_self_update().await {
        Ok(updated) => {
            if updated {
                println!(
                    "{}",
                    styled(
                        "Successfully updated to the latest version! Please restart glab-tui.",
                        C_GREEN
                    )
                );
            } else {
                println!("Already up to date.");
            }
        }
        Err(e) => {
            eprintln!("{}", styled(&format!("Update failed: {}", e), C_RED));
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
