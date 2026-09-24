use crate::backend::BackendKind;

/// Safe to run on the UI thread: `git symbolic-ref --short HEAD` and
/// `git branch --show-current` are pure local object-DB reads and finish
/// in microseconds. No network roundtrip, no prompt for credentials.
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

pub fn parse_project_path_from_web_url(web_url: &str) -> Option<String> {
    if web_url.is_empty() {
        return None;
    }
    if let Some((base, _)) = web_url.split_once("/-/") {
        let without_scheme = base.split_once("://").map(|(_, p)| p).unwrap_or(base);
        if let Some((_, path)) = without_scheme.split_once('/') {
            if !path.is_empty() {
                return Some(path.to_string());
            }
        }
    }
    let without_scheme = web_url.split_once("://").map(|(_, p)| p).unwrap_or(web_url);
    if let Some((_, path_part)) = without_scheme.split_once('/') {
        let parts: Vec<&str> = path_part.split('/').collect();
        if parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            return Some(format!("{}/{}", parts[0], parts[1]));
        }
    }
    None
}

/// If the remote URL points at a group clone (path has no slash, e.g.
/// `git@host:group` or `https://host/group`), return the group name.
/// Returns `None` for project clones (which have `group/project`).
pub fn parse_group(url: &str) -> Option<String> {
    let url = url.trim();
    let path = if let Some((_scheme, rest)) = url.split_once("://") {
        rest.split_once('/')?.1
    } else if let Some((_host, rest)) = url.split_once(':') {
        rest
    } else {
        return None;
    };
    let path = path.trim_matches('/').strip_suffix(".git").unwrap_or(path);
    (!path.is_empty() && !path.contains('/')).then(|| path.to_string())
}

pub fn parse_remote_host(url: &str) -> Option<String> {
    let url = url.trim();
    let authority = if let Some((_, rest)) = url.split_once("://") {
        rest.split('/').next()?
    } else {
        url.split_once(':')?.0
    };
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host.trim_matches(['[', ']']);
    let host = host.split(':').next().unwrap_or(host);
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

pub async fn detect_backend(remote_url: &str, override_kind: Option<BackendKind>) -> BackendKind {
    if let Some(kind) = override_kind {
        return kind;
    }

    let Some(raw_host) = parse_remote_host(remote_url) else {
        return BackendKind::GitLab;
    };
    let host = raw_host.strip_prefix("www.").unwrap_or(&raw_host);
    if host == "github.com" {
        return BackendKind::GitHub;
    }

    let (gh_authenticated, glab_authenticated) = tokio::join!(
        auth_status("gh", &raw_host, true),
        auth_status("glab", &raw_host, false),
    );
    if gh_authenticated && !glab_authenticated {
        BackendKind::GitHub
    } else if glab_authenticated && !gh_authenticated {
        BackendKind::GitLab
    } else {
        BackendKind::GitLab
    }
}

/// Probe `<program> auth status --hostname <host>`. Runs in `spawn_blocking`
/// because both `gh auth status` and `glab auth status` can talk to the
/// host (HTTPS call) and a hung credential prompt on a misconfigured
/// machine would freeze the TUI.
///
/// Sets `GIT_TERMINAL_PROMPT=0` so any HTTPS credential prompt fails
/// fast rather than blocking the spawned thread indefinitely, mirroring
/// the convention established in `ensure_source_branch_pushed`.
async fn auth_status(program: &str, host: &str, active: bool) -> bool {
    let program = program.to_string();
    let host = host.to_string();
    tokio::task::spawn_blocking(move || {
        let mut command = std::process::Command::new(&program);
        command.env("GIT_TERMINAL_PROMPT", "0");
        command.args(["auth", "status"]);
        if active {
            command.arg("--active");
        }
        command.args(["--hostname", &host]);
        command.output().is_ok_and(|output| output.status.success())
    })
    .await
    .unwrap_or(false)
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

/// The default repository the GitHub CLI was pointed at for this checkout.
///
/// `gh repo set-default` — and `gh repo clone` of a fork — record the choice as
/// `remote.<name>.gh-resolved`, either as the literal `base` when the target is
/// one of the local remotes, or as an explicit `namespace/project` when it is not.
#[derive(Debug, PartialEq, Eq)]
pub enum GhDefaultRepo {
    Remote(String),
    Project(String),
}

/// Reads the first `gh-resolved` entry out of `git config --get-regexp` output.
///
/// Remote names may themselves contain dots, so the name is whatever sits
/// between the `remote.` prefix and the `.gh-resolved` suffix.
pub fn parse_gh_resolved(config_output: &str) -> Option<GhDefaultRepo> {
    config_output.lines().find_map(|line| {
        let (key, value) = line.trim().split_once(' ')?;
        let name = key.strip_prefix("remote.")?.strip_suffix(".gh-resolved")?;
        if name.is_empty() || value.is_empty() {
            return None;
        }
        Some(if value == "base" {
            GhDefaultRepo::Remote(name.to_string())
        } else {
            GhDefaultRepo::Project(value.to_string())
        })
    })
}

fn gh_resolved() -> Option<GhDefaultRepo> {
    let output = std::process::Command::new("git")
        .args(["config", "--get-regexp", r"^remote\..*\.gh-resolved$"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_gh_resolved(&String::from_utf8_lossy(&output.stdout))
}

/// Extracts `namespace/project` from the URL of the named remote.
pub fn remote_project_path(remote: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", remote])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_project_path(&String::from_utf8_lossy(&output.stdout))
}

/// The project the user pointed `gh` at, or `None` when they never did.
pub fn gh_resolved_project() -> Option<String> {
    match gh_resolved()? {
        GhDefaultRepo::Project(path) => Some(path),
        GhDefaultRepo::Remote(name) => remote_project_path(&name),
    }
}

fn strip_remote_prefix(head: &str, remote: &str) -> Option<String> {
    let branch = head.trim();
    let branch = branch
        .strip_prefix(&format!("{}/", remote))
        .unwrap_or(branch);
    (!branch.is_empty() && branch != "HEAD").then(|| branch.to_string())
}

fn default_branch_of(remote: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", &format!("{}/HEAD", remote)])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    strip_remote_prefix(&String::from_utf8_lossy(&output.stdout), remote)
}

/// Safe to run on the UI thread: `git config` and `git rev-parse --abbrev-ref
/// <remote>/HEAD` read the local config and object DB once `<remote>/HEAD`
/// has been set by a previous `git fetch`/`clone`. No network roundtrip,
/// no prompt for credentials. Returns `None` when the remote has not
/// been initialized yet, in which case the caller falls back to the
/// default "main"/"master" heuristic.
pub fn get_default_branch() -> Option<String> {
    // A remote that is not `origin` frequently has no HEAD ref — clone only
    // creates one for the remote it cloned from — so `origin` stays the fallback.
    match gh_resolved() {
        Some(GhDefaultRepo::Remote(name)) if name != "origin" => {
            default_branch_of(&name).or_else(|| default_branch_of("origin"))
        }
        _ => default_branch_of("origin"),
    }
}

/// Ensure `branch` exists locally and on `origin` without interactive prompts.
///
/// Creates the local branch from `HEAD` when missing. Skips `git push` when
/// `origin` already has `refs/heads/<branch>`. Sets `GIT_TERMINAL_PROMPT=0` so
/// HTTPS credential prompts fail fast instead of hanging a TUI that owns the tty.
pub fn ensure_source_branch_pushed(branch: &str) -> anyhow::Result<()> {
    if branch.is_empty() {
        return Ok(());
    }

    let exists = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", branch])
        .output()
        .ok()
        .is_some_and(|o| o.status.success());
    if !exists {
        let output = std::process::Command::new("git")
            .args(["branch", branch, "HEAD"])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to create local branch '{branch}': {e}"))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            anyhow::bail!(
                "Failed to create branch '{branch}': {}",
                if err.is_empty() {
                    "unknown error"
                } else {
                    &err
                }
            );
        }
    }

    let remote_ref = format!("refs/heads/{branch}");
    let probe = std::process::Command::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["ls-remote", "--exit-code", "origin", &remote_ref])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run git ls-remote: {e}"))?;

    match probe.status.code() {
        Some(0) => return Ok(()),
        Some(2) => {}
        _ => {
            let err = String::from_utf8_lossy(&probe.stderr).trim().to_string();
            anyhow::bail!(
                "Could not check remote branch '{branch}': {}. Configure git credentials (e.g. `gh auth setup-git`) or use SSH.",
                if err.is_empty() {
                    "authentication or network error"
                } else {
                    &err
                }
            );
        }
    }

    let output = std::process::Command::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["push", "-u", "origin", branch])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to push branch '{branch}': {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "Failed to push branch '{branch}': {}. Configure git credentials (e.g. `gh auth setup-git`) or use SSH.",
            if err.is_empty() {
                "unknown error"
            } else {
                &err
            }
        );
    }
    Ok(())
}

/// Safe to run on the UI thread: `git branch -a` enumerates the refs in
/// the local `.git` directory. No network roundtrip — remote-only branches
/// appear by name because git caches them locally after `fetch` — and
/// no prompt for credentials.
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
                    let name = line.strip_prefix('*').unwrap_or(line).trim();
                    let name = if let Some(stripped) = name.strip_prefix("remotes/") {
                        if let Some((_remote, branch_part)) = stripped.split_once('/') {
                            branch_part.to_string()
                        } else {
                            stripped.to_string()
                        }
                    } else {
                        name.to_string()
                    };
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
/// Safe to run on the UI thread: `git rev-parse --show-toplevel` is a
/// pure local resolve and the workflow scan afterwards only opens files
/// under `.github/workflows/` or `.gitlab-ci*.yml`. No network roundtrip,
/// no prompt for credentials.
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
    use super::{
        GhDefaultRepo, detect_backend, parse_gh_resolved, parse_project_path, parse_remote_host,
        strip_remote_prefix,
    };
    use crate::backend::BackendKind;

    #[test]
    fn parses_web_url_for_custom_hosts() {
        assert_eq!(
            super::parse_project_path_from_web_url(
                "https://github.mycorp.com/myorg/myrepo/issues/1"
            )
            .as_deref(),
            Some("myorg/myrepo")
        );
        assert_eq!(
            super::parse_project_path_from_web_url(
                "https://gitlab.mycorp.com/group/subgroup/project/-/issues/1"
            )
            .as_deref(),
            Some("group/subgroup/project")
        );
    }

    #[test]
    fn parses_remote_hosts() {
        assert_eq!(
            parse_remote_host("https://github.example.com/org/repo.git").as_deref(),
            Some("github.example.com")
        );
        assert_eq!(
            parse_remote_host("git@github.example.com:org/repo.git").as_deref(),
            Some("github.example.com")
        );
        assert_eq!(
            parse_remote_host("ssh://git@gitlab.example.com:2222/org/repo.git").as_deref(),
            Some("gitlab.example.com")
        );
    }

    #[tokio::test]
    async fn backend_override_takes_precedence() {
        assert_eq!(
            detect_backend(
                "git@github.example.com:org/repo.git",
                Some(BackendKind::GitLab)
            )
            .await,
            BackendKind::GitLab
        );
    }

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

    #[tokio::test]
    async fn www_prefix_resolves_to_github() {
        assert_eq!(
            detect_backend("https://www.github.com/rcieri/glab-tui", None).await,
            BackendKind::GitHub
        );
    }

    #[tokio::test]
    async fn www_prefix_on_gitlab_host_stays_gitlab() {
        assert_eq!(
            detect_backend("https://www.gitlab.com/org/repo.git", None).await,
            BackendKind::GitLab
        );
    }

    #[tokio::test]
    async fn www_prefix_scp_style_resolves_to_github() {
        assert_eq!(
            detect_backend("git@www.github.com:org/repo.git", None).await,
            BackendKind::GitHub
        );
    }

    #[test]
    fn rejects_urls_without_a_namespace() {
        assert_eq!(parse_project_path("https://gitlab.example.com/"), None);
        assert_eq!(parse_project_path("not-a-url"), None);
        assert_eq!(parse_project_path(""), None);
    }

    #[test]
    fn gh_resolved_base_names_the_remote() {
        assert_eq!(
            parse_gh_resolved("remote.upstream.gh-resolved base\n"),
            Some(GhDefaultRepo::Remote("upstream".to_string()))
        );
    }

    #[test]
    fn gh_resolved_explicit_value_names_the_project() {
        assert_eq!(
            parse_gh_resolved("remote.origin.gh-resolved rcieri/glab-tui\n"),
            Some(GhDefaultRepo::Project("rcieri/glab-tui".to_string()))
        );
    }

    #[test]
    fn gh_resolved_keeps_dots_in_remote_names() {
        assert_eq!(
            parse_gh_resolved("remote.my.fork.gh-resolved base\n"),
            Some(GhDefaultRepo::Remote("my.fork".to_string()))
        );
    }

    #[test]
    fn gh_resolved_takes_the_first_entry() {
        assert_eq!(
            parse_gh_resolved("remote.origin.gh-resolved base\nremote.upstream.gh-resolved base\n"),
            Some(GhDefaultRepo::Remote("origin".to_string()))
        );
    }

    #[test]
    fn gh_resolved_skips_malformed_lines() {
        assert_eq!(
            parse_gh_resolved("remote.origin.gh-resolved\nremote.upstream.gh-resolved base\n"),
            Some(GhDefaultRepo::Remote("upstream".to_string()))
        );
    }

    #[test]
    fn absent_gh_resolved_yields_nothing() {
        assert_eq!(parse_gh_resolved(""), None);
        assert_eq!(
            parse_gh_resolved("remote.origin.url git@github.com:o/r.git"),
            None
        );
    }

    #[test]
    fn default_branch_strips_the_remote_it_was_read_from() {
        assert_eq!(
            strip_remote_prefix("upstream/main", "upstream").as_deref(),
            Some("main")
        );
        assert_eq!(
            strip_remote_prefix("origin/master", "origin").as_deref(),
            Some("master")
        );
    }

    #[test]
    fn default_branch_rejects_unresolved_head() {
        assert_eq!(strip_remote_prefix("HEAD", "origin"), None);
        assert_eq!(strip_remote_prefix("", "origin"), None);
    }
}
