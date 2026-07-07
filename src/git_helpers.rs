pub fn get_current_branch() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !branch.is_empty() {
            return Some(branch);
        }
    }
    let output = std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !branch.is_empty() {
            return Some(branch);
        }
    }
    None
}

pub fn slugify(s: &str) -> String {
    let mut slug = String::with_capacity(s.len());
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if c.is_ascii() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

pub fn get_default_branch() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "origin/HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let branch = branch
            .strip_prefix("origin/")
            .unwrap_or(&branch)
            .to_string();
        if !branch.is_empty() && branch != "HEAD" {
            return Some(branch);
        }
    }
    None
}

pub fn get_branches() -> Vec<String> {
    let output = std::process::Command::new("git")
        .args(["branch", "-a"])
        .output()
        .ok();
    if let Some(output) = output {
        if output.status.success() {
            let mut branches: Vec<String> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| {
                    let line = line.trim();
                    if line.is_empty() {
                        return None;
                    }
                    let name = line.strip_prefix('*').unwrap_or(line).trim().to_string();
                    let name = name
                        .strip_prefix("remotes/origin/")
                        .unwrap_or(&name)
                        .to_string();
                    if name.is_empty() || name.contains(" -> ") {
                        return None;
                    }
                    Some(name)
                })
                .collect();
            branches.sort();
            branches.dedup();
            return branches;
        }
    }
    Vec::new()
}

/// Returns a list of workflow/CI files available in the repo.
/// For GitHub repos: scans `.github/workflows/*.yml` and `*.yaml`.
/// For GitLab repos: returns `.gitlab-ci.yml` if it exists, else empty.
pub fn get_workflow_files(is_github: bool) -> Vec<String> {
    // Determine the repo root via `git rev-parse --show-toplevel`
    let root = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| ".".to_string());

    if is_github {
        let workflows_dir = std::path::Path::new(&root)
            .join(".github")
            .join("workflows");
        let mut files: Vec<String> = std::fs::read_dir(&workflows_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if (ext == "yml" || ext == "yaml") && path.is_file() {
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect();
        files.sort();
        files
    } else {
        // GitLab: the primary CI file is `.gitlab-ci.yml`; also check for
        // include-able `.gitlab-ci-*.yml` files at the root.
        let mut files: Vec<String> = std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if !path.is_file() {
                    return None;
                }
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if (ext == "yml" || ext == "yaml")
                    && (name == ".gitlab-ci.yml" || name.starts_with(".gitlab-ci-"))
                {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        files.sort();
        files
    }
}
