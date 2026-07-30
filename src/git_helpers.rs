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

/// Extracts the `namespace/project` path from a git remote URL.
///
/// Accepts `scheme://[user[:pass]@]host[:port]/namespace/project[.git]` and
/// scp-style `git@host:namespace/project[.git]`. Every segment after the host
/// is preserved, because GitLab namespaces can nest arbitrarily deep
/// (`group/subgroup/subsubgroup/project`).
///
/// Returns `None` when the URL has no parseable namespace.
pub fn parse_project_path(url: &str) -> Option<String> {
    let url = url.trim();
    // Drop everything up to and including the host, keeping the rest intact.
    // Splitting on "://" must be tried first, since those URLs also contain ':'.
    let path = if let Some((_scheme, rest)) = url.split_once("://") {
        rest.split_once('/')?.1
    } else if let Some((_host, rest)) = url.split_once(':') {
        rest
    } else {
        return None;
    };

    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    path.contains('/').then(|| path.to_string())
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

#[cfg(test)]
mod tests {
    use super::parse_project_path;

    #[test]
    fn keeps_nested_subgroups_over_https() {
        assert_eq!(
            parse_project_path("https://gitlab.example.com/dev/cbr/salesforce/salesforce.git")
                .as_deref(),
            Some("dev/cbr/salesforce/salesforce")
        );
    }

    #[test]
    fn keeps_nested_subgroups_over_scp_style_ssh() {
        assert_eq!(
            parse_project_path("git@gitlab.example.com:dev/cbr/salesforce/salesforce.git")
                .as_deref(),
            Some("dev/cbr/salesforce/salesforce")
        );
    }

    #[test]
    fn parses_single_namespace_https() {
        assert_eq!(
            parse_project_path("https://gitlab.com/group/repo.git").as_deref(),
            Some("group/repo")
        );
    }

    #[test]
    fn parses_ssh_scheme_with_port() {
        assert_eq!(
            parse_project_path("ssh://git@gitlab.example.com:2222/group/sub/repo.git").as_deref(),
            Some("group/sub/repo")
        );
    }

    #[test]
    fn parses_https_with_port() {
        assert_eq!(
            parse_project_path("https://gitlab.example.com:8443/group/sub/repo.git").as_deref(),
            Some("group/sub/repo")
        );
    }

    #[test]
    fn ignores_embedded_credentials() {
        assert_eq!(
            parse_project_path("https://user:token@gitlab.example.com/group/sub/repo.git")
                .as_deref(),
            Some("group/sub/repo")
        );
    }

    #[test]
    fn tolerates_missing_git_suffix_and_trailing_slash() {
        assert_eq!(
            parse_project_path("https://gitlab.example.com/group/sub/repo/").as_deref(),
            Some("group/sub/repo")
        );
    }

    #[test]
    fn preserves_project_names_containing_git() {
        assert_eq!(
            parse_project_path("https://gitlab.example.com/group/my.github.git").as_deref(),
            Some("group/my.github")
        );
    }

    #[test]
    fn rejects_urls_without_a_namespace() {
        assert_eq!(parse_project_path("https://gitlab.example.com/"), None);
        assert_eq!(parse_project_path("not-a-url"), None);
        assert_eq!(parse_project_path(""), None);
    }
}
