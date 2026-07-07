use anyhow::{Context, Result};
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct GitlabClient {
    pub is_github: bool,
    pub tx: Option<tokio::sync::mpsc::UnboundedSender<crate::event::Event>>,
    pub page_size: usize,
}

fn get_api_description(endpoint: &str, is_github: bool) -> String {
    let pr_suffix = if is_github { "PR" } else { "MR" };
    let prs_suffix = if is_github { "PRs" } else { "MRs" };

    if endpoint.contains("/issues/") {
        "Fetching Issue".to_string()
    } else if endpoint.contains("/issues") {
        "Fetching Issues".to_string()
    } else if endpoint.contains("/merge_requests/") || endpoint.contains("/pulls/") {
        format!("Fetching {}", pr_suffix)
    } else if endpoint.contains("/merge_requests") || endpoint.contains("/pulls") {
        format!("Fetching {}", prs_suffix)
    } else if (endpoint.contains("/pipelines/") && endpoint.contains("/jobs"))
        || (endpoint.contains("/actions/runs/") && endpoint.contains("/jobs"))
    {
        "Fetching Jobs".to_string()
    } else if endpoint.contains("/pipelines") || endpoint.contains("/actions/runs") {
        "Fetching Pipelines".to_string()
    } else if endpoint.contains("/runners") {
        "Fetching Runners".to_string()
    } else if endpoint.contains("/releases") {
        "Fetching Releases".to_string()
    } else if endpoint.contains("/milestones/") && endpoint.contains("/issues") {
        "Fetching Milestone Issues".to_string()
    } else if endpoint.contains("/milestones") {
        "Fetching Milestones".to_string()
    } else if endpoint.contains("/labels") {
        "Fetching Labels".to_string()
    } else if endpoint.contains("/members") || endpoint.contains("/collaborators") {
        "Fetching Members".to_string()
    } else if endpoint.contains("notifications") || endpoint.contains("todos") {
        "Fetching Notifications".to_string()
    } else {
        "Fetching API".to_string()
    }
}

impl GitlabClient {
    pub async fn new() -> Result<Self> {
        let is_github = match tokio::process::Command::new("git")
            .args(["remote", "get-url", "origin"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let url = String::from_utf8_lossy(&output.stdout);
                url.contains("github.com")
            }
            _ => false,
        };
        Ok(Self {
            is_github,
            tx: None,
            page_size: 100,
        })
    }

    pub async fn execute_github_api(
        &self,
        endpoint: &str,
        method: &str,
        body: Option<&str>,
    ) -> Result<String> {
        let mut cmd_str = "gh api".to_string();
        if method != "GET" {
            cmd_str.push_str(&format!(" -X {}", method));
        }
        if let Some(b) = body {
            if !b.is_empty() {
                cmd_str.push_str(" --input -");
            }
        }
        cmd_str.push_str(&format!(" {}", endpoint));

        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        if let Some(ref tx) = self.tx {
            let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                timestamp: timestamp.clone(),
                command: cmd_str.clone(),
                status: "Running".to_string(),
            });
        }

        let mut cmd = Command::new("gh");
        cmd.arg("api");
        if method != "GET" {
            cmd.arg("-X");
            cmd.arg(method);
        }
        if let Some(b) = body {
            if !b.is_empty() {
                cmd.arg("--input");
                cmd.arg("-");
                cmd.stdin(std::process::Stdio::piped());
            }
        }
        cmd.arg(endpoint);

        let output = if let Some(b) = body {
            if !b.is_empty() {
                let mut child = cmd.spawn().context("Failed to spawn gh api command")?;
                use tokio::io::AsyncWriteExt;
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(b.as_bytes()).await?;
                    stdin.flush().await?;
                }
                child.wait_with_output().await
            } else {
                cmd.output().await
            }
        } else {
            cmd.output().await
        };

        match output {
            Ok(out) => {
                if out.status.success() {
                    let s = String::from_utf8(out.stdout)?;
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: cmd_str.clone(),
                            status: "Success".to_string(),
                        });
                    }
                    Ok(s)
                } else {
                    let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: cmd_str.clone(),
                            status: format!("Failed: {}", err_msg),
                        });
                    }
                    anyhow::bail!("gh api failed: {}", err_msg)
                }
            }
            Err(e) => {
                let err_msg = format!("{}", e);
                if let Some(ref tx) = self.tx {
                    let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                        timestamp: timestamp.clone(),
                        command: cmd_str.clone(),
                        status: format!("Failed: {}", err_msg),
                    });
                }
                Err(e.into())
            }
        }
    }

    pub async fn execute_gitlab_api(
        &self,
        endpoint: &str,
        method: &str,
        body: Option<&str>,
    ) -> Result<String> {
        let mut cmd_str = "glab api".to_string();
        if method != "GET" {
            cmd_str.push_str(&format!(" -X {}", method));
        }
        if let Some(b) = body {
            if !b.is_empty() {
                cmd_str.push_str(" --input -");
            }
        }
        cmd_str.push_str(&format!(" {}", endpoint));

        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        if let Some(ref tx) = self.tx {
            let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                timestamp: timestamp.clone(),
                command: cmd_str.clone(),
                status: "Running".to_string(),
            });
        }

        let mut cmd = Command::new("glab");
        cmd.arg("api");
        if method != "GET" {
            cmd.arg("-X");
            cmd.arg(method);
        }
        if let Some(b) = body {
            if !b.is_empty() {
                cmd.arg("--input");
                cmd.arg("-");
                cmd.stdin(std::process::Stdio::piped());
            }
        }
        cmd.arg(endpoint);

        let output = if let Some(b) = body {
            if !b.is_empty() {
                let mut child = cmd.spawn().context("Failed to spawn glab api command")?;
                use tokio::io::AsyncWriteExt;
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(b.as_bytes()).await?;
                    stdin.flush().await?;
                }
                child.wait_with_output().await
            } else {
                cmd.output().await
            }
        } else {
            cmd.output().await
        };

        match output {
            Ok(out) => {
                if out.status.success() {
                    let s = String::from_utf8(out.stdout)?;
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: cmd_str.clone(),
                            status: "Success".to_string(),
                        });
                    }
                    Ok(s)
                } else {
                    let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: cmd_str.clone(),
                            status: format!("Failed: {}", err_msg),
                        });
                    }
                    anyhow::bail!("glab api failed: {}", err_msg)
                }
            }
            Err(e) => {
                let err_msg = format!("{}", e);
                if let Some(ref tx) = self.tx {
                    let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                        timestamp: timestamp.clone(),
                        command: cmd_str.clone(),
                        status: format!("Failed: {}", err_msg),
                    });
                }
                Err(e.into())
            }
        }
    }

    pub async fn execute_raw_command(
        &self,
        program: &str,
        args: &[&str],
        desc: &str,
    ) -> Result<String> {
        let cmd_str = format!("{} {}", program, args.join(" "));
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        if let Some(ref tx) = self.tx {
            let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                timestamp: timestamp.clone(),
                command: cmd_str.clone(),
                status: "Running".to_string(),
            });
        }

        let output = Command::new(program)
            .args(args)
            .output()
            .await
            .context(format!("Failed to execute {} command", program));

        match output {
            Ok(out) => {
                if out.status.success() {
                    let s = String::from_utf8(out.stdout)?;
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: cmd_str.clone(),
                            status: "Success".to_string(),
                        });
                    }
                    Ok(s)
                } else {
                    let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: cmd_str.clone(),
                            status: format!("Failed: {}", err_msg),
                        });
                    }
                    anyhow::bail!("{} failed: {}", program, err_msg)
                }
            }
            Err(e) => {
                let err_msg = format!("{}", e);
                if let Some(ref tx) = self.tx {
                    let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                        timestamp: timestamp.clone(),
                        command: cmd_str.clone(),
                        status: format!("Failed: {}", err_msg),
                    });
                }
                Err(e.into())
            }
        }
    }

    pub async fn fetch_raw_api(&self, endpoint: &str) -> Result<String> {
        let normalized = endpoint.trim_start_matches('/');
        let parts: Vec<&str> = normalized.split('/').collect();

        if self.is_github {
            if normalized.contains("/jobs/") && normalized.contains("/trace") {
                let job_id = parts
                    .iter()
                    .position(|&p| p == "jobs")
                    .and_then(|idx| parts.get(idx + 1))
                    .ok_or_else(|| anyhow::anyhow!("Could not extract job ID from endpoint"))?;
                self.execute_raw_command(
                    "gh",
                    &["run", "view", "--job", job_id, "--log"],
                    "Fetching Job Logs",
                )
                .await
            } else if normalized.contains("/pipelines/") && normalized.contains("/retry") {
                let project_path = parts
                    .get(1)
                    .map(|p| p.replace("%2F", "/"))
                    .unwrap_or_default();
                let pipeline_id = parts
                    .iter()
                    .position(|&p| p == "pipelines")
                    .and_then(|idx| parts.get(idx + 1))
                    .unwrap_or(&"");
                let gh_endpoint =
                    format!("/repos/{}/actions/runs/{}/rerun", project_path, pipeline_id);
                self.execute_github_api(&gh_endpoint, "POST", Some(""))
                    .await
            } else if normalized.contains("/pipelines/") && normalized.contains("/cancel") {
                let project_path = parts
                    .get(1)
                    .map(|p| p.replace("%2F", "/"))
                    .unwrap_or_default();
                let pipeline_id = parts
                    .iter()
                    .position(|&p| p == "pipelines")
                    .and_then(|idx| parts.get(idx + 1))
                    .unwrap_or(&"");
                let gh_endpoint = format!(
                    "/repos/{}/actions/runs/{}/cancel",
                    project_path, pipeline_id
                );
                self.execute_github_api(&gh_endpoint, "POST", Some(""))
                    .await
            } else if normalized.contains("/jobs/") && normalized.contains("/retry") {
                let project_path = parts
                    .get(1)
                    .map(|p| p.replace("%2F", "/"))
                    .unwrap_or_default();
                let job_id = parts
                    .iter()
                    .position(|&p| p == "jobs")
                    .and_then(|idx| parts.get(idx + 1))
                    .unwrap_or(&"");
                let gh_endpoint = format!("/repos/{}/actions/jobs/{}/rerun", project_path, job_id);
                self.execute_github_api(&gh_endpoint, "POST", Some(""))
                    .await
            } else if normalized.contains("/merge_requests/") && normalized.contains("/merge") {
                let project_path = parts
                    .get(1)
                    .map(|p| p.replace("%2F", "/"))
                    .unwrap_or_default();
                let mr_id = parts
                    .iter()
                    .position(|&p| p == "merge_requests")
                    .and_then(|idx| parts.get(idx + 1))
                    .unwrap_or(&"");
                let gh_endpoint = format!("/repos/{}/pulls/{}/merge", project_path, mr_id);
                self.execute_github_api(&gh_endpoint, "PUT", Some("")).await
            } else {
                anyhow::bail!("Unsupported GitHub endpoint in fetch_raw_api: {}", endpoint)
            }
        } else {
            let is_post = normalized.contains("/retry")
                || normalized.contains("/cancel")
                || normalized.contains("/merge");
            let method = if is_post { "POST" } else { "GET" };
            self.execute_gitlab_api(endpoint, method, None).await
        }
    }

    pub async fn fetch_labels(&self, project_path: &str) -> Result<Vec<String>> {
        if self.is_github {
            #[derive(serde::Deserialize)]
            struct GithubLabel {
                name: String,
            }
            let endpoint = format!("/repos/{}/labels?per_page=100", project_path);
            let raw = self.execute_github_api(&endpoint, "GET", None).await?;
            let labels: Vec<GithubLabel> = serde_json::from_str(&raw)?;
            Ok(labels.into_iter().map(|l| l.name).collect())
        } else {
            #[derive(serde::Deserialize)]
            struct GitlabLabel {
                name: String,
            }
            let encoded_path = project_path.replace("/", "%2F");
            let endpoint = format!("/projects/{}/labels?per_page=100", encoded_path);
            let raw = self.execute_gitlab_api(&endpoint, "GET", None).await?;
            let labels: Vec<GitlabLabel> = serde_json::from_str(&raw)?;
            Ok(labels.into_iter().map(|l| l.name).collect())
        }
    }

    pub async fn fetch_members(&self, project_path: &str) -> Result<Vec<String>> {
        if self.is_github {
            #[derive(serde::Deserialize)]
            struct GithubAssignee {
                login: String,
            }
            let endpoint = format!("/repos/{}/assignees?per_page=100", project_path);
            let raw = self.execute_github_api(&endpoint, "GET", None).await?;
            let assignees: Vec<GithubAssignee> = serde_json::from_str(&raw)?;
            Ok(assignees
                .into_iter()
                .map(|a| format!("@{}", a.login))
                .collect())
        } else {
            #[derive(serde::Deserialize)]
            struct GitlabMember {
                username: String,
            }
            let encoded_path = project_path.replace("/", "%2F");
            let endpoint = format!("/projects/{}/members/all?per_page=100", encoded_path);
            let raw = self.execute_gitlab_api(&endpoint, "GET", None).await?;
            let members: Vec<GitlabMember> = serde_json::from_str(&raw)?;
            Ok(members
                .into_iter()
                .map(|m| format!("@{}", m.username))
                .collect())
        }
    }

    pub async fn fetch_milestones(&self, project_path: &str) -> Result<Vec<String>> {
        if self.is_github {
            #[derive(serde::Deserialize)]
            struct GithubMilestone {
                title: String,
            }
            let endpoint = format!("/repos/{}/milestones?state=open&per_page=100", project_path);
            let raw = self.execute_github_api(&endpoint, "GET", None).await?;
            let milestones: Vec<GithubMilestone> = serde_json::from_str(&raw)?;
            Ok(milestones.into_iter().map(|m| m.title).collect())
        } else {
            #[derive(serde::Deserialize)]
            struct GitlabMilestone {
                title: String,
            }
            let encoded_path = project_path.replace("/", "%2F");
            let endpoint = format!(
                "/projects/{}/milestones?state=active&per_page=100",
                encoded_path
            );
            let raw = self.execute_gitlab_api(&endpoint, "GET", None).await?;
            let milestones: Vec<GitlabMilestone> = serde_json::from_str(&raw)?;
            Ok(milestones.into_iter().map(|m| m.title).collect())
        }
    }
}

pub async fn get_project_context() -> Result<String> {
    // Execute `git remote get-url origin`
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .context("Failed to execute git command")?;

    if !output.status.success() {
        return Ok("unknown/unknown".to_string());
    }

    let url = String::from_utf8(output.stdout)?.trim().to_string();

    // Parse url to extract namespace/repo
    let path = if url.starts_with("git@") {
        url.split(':').nth(1).unwrap_or("unknown/unknown")
    } else if url.starts_with("http") {
        let parts: Vec<&str> = url.split('/').collect();
        if parts.len() >= 2 {
            let p = format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]);
            return Ok(p.trim_end_matches(".git").to_string());
        }
        "unknown/unknown"
    } else {
        "unknown/unknown"
    };

    Ok(path.trim_end_matches(".git").to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_deserialize_github_issue() {
        use crate::gitlab::issues::{GithubIssue, Issue};
        let json_data = r#"{
            "number": 42,
            "title": "A github issue",
            "state": "open",
            "labels": [{"name": "bug"}],
            "updated_at": "2026-06-01T00:00:00Z",
            "created_at": "2026-06-01T00:00:00Z",
            "closed_at": null,
            "user": {"login": "octocat"},
            "milestone": {"title": "v1.0"},
            "assignees": [{"login": "octocat"}],
            "body": "Issue description",
            "pull_request": null
        }"#;

        let gh_issue: GithubIssue = serde_json::from_str(json_data).unwrap();
        let issue = Issue::from(gh_issue);

        assert_eq!(issue.iid, 42);
        assert_eq!(issue.title, "A github issue");
        assert_eq!(issue.state, "opened");
        assert_eq!(issue.labels[0], "bug");
        assert_eq!(issue.author.username, "octocat");
        assert_eq!(issue.milestone.unwrap().title, "v1.0");
        assert_eq!(issue.assignees[0].username, "octocat");
        assert_eq!(issue.description.unwrap(), "Issue description");
    }

    #[test]
    fn test_deserialize_github_pull_request() {
        use crate::gitlab::mr::{GithubPullRequest, MergeRequest};
        let json_data = r#"{
            "number": 101,
            "title": "Fix a bug",
            "state": "open",
            "labels": [{"name": "enhancement"}],
            "updated_at": "2026-06-01T00:00:00Z",
            "user": {"login": "octocat"},
            "milestone": {"title": "v1.0"},
            "assignees": [{"login": "octocat"}],
            "requested_reviewers": [{"login": "reviewer1"}],
            "base": {"ref": "main"},
            "head": {"ref": "feature-bug"},
            "draft": false,
            "body": "PR description"
        }"#;

        let gh_pr: GithubPullRequest = serde_json::from_str(json_data).unwrap();
        let mr = MergeRequest::from(gh_pr);

        assert_eq!(mr.iid, 101);
        assert_eq!(mr.title, "Fix a bug");
        assert_eq!(mr.state, "opened");
        assert_eq!(mr.labels[0], "enhancement");
        assert_eq!(mr.author.username, "octocat");
        assert_eq!(mr.milestone.unwrap().title, "v1.0");
        assert_eq!(mr.assignees[0].username, "octocat");
        assert_eq!(mr.reviewers[0].username, "reviewer1");
        assert_eq!(mr.target_branch, "main");
        assert_eq!(mr.source_branch, "feature-bug");
        assert!(!mr.draft);
        assert_eq!(mr.description.unwrap(), "PR description");
    }

    #[test]
    fn test_github_issue_list_filters_prs() {
        use crate::gitlab::issues::GithubIssue;
        let json_data = r#"[
            {
                "number": 1,
                "title": "Normal issue",
                "state": "open",
                "labels": [],
                "updated_at": "2026-06-01T00:00:00Z",
                "user": {"login": "user"},
                "milestone": null,
                "assignees": [],
                "body": "desc",
                "pull_request": null
            },
            {
                "number": 2,
                "title": "A pull request",
                "state": "open",
                "labels": [],
                "updated_at": "2026-06-01T00:00:00Z",
                "user": {"login": "user"},
                "milestone": null,
                "assignees": [],
                "body": "desc",
                "pull_request": {"url": "https://api.github.com/..."}
            }
        ]"#;

        let gh_issues: Vec<GithubIssue> = serde_json::from_str(json_data).unwrap();
        let filtered: Vec<GithubIssue> = gh_issues
            .into_iter()
            .filter(|i| i.pull_request.is_none())
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].number, 1);
        assert_eq!(filtered[0].title, "Normal issue");
    }

    #[test]
    fn test_deserialize_github_pull_request_comments() {
        use crate::gitlab::mr::{DiscussionNote, GithubPullComment};
        let json_data = r#"[
            {
                "id": 1,
                "body": "a code comment",
                "path": "src/main.rs",
                "line": 42,
                "side": "RIGHT",
                "user": {"login": "octocat"},
                "created_at": "2026-06-01T00:00:00Z",
                "in_reply_to_id": null
            },
            {
                "id": 2,
                "body": "another code comment",
                "path": "src/main.rs",
                "line": 10,
                "side": "LEFT",
                "user": {"login": "octocat"},
                "created_at": "2026-06-01T00:00:00Z",
                "in_reply_to_id": 1
            }
        ]"#;

        let gh_comments: Vec<GithubPullComment> = serde_json::from_str(json_data).unwrap();
        let notes: Vec<DiscussionNote> =
            gh_comments.into_iter().map(DiscussionNote::from).collect();
        assert_eq!(notes.len(), 2);

        assert_eq!(notes[0].id, 1);
        assert_eq!(notes[0].body, "a code comment");
        assert_eq!(notes[0].author.username, "octocat");
        let pos1 = notes[0].position.as_ref().unwrap();
        assert_eq!(pos1.new_path.as_deref(), Some("src/main.rs"));
        assert_eq!(pos1.new_line, Some(42));
        assert!(pos1.old_line.is_none());

        assert_eq!(notes[1].id, 2);
        assert_eq!(notes[1].body, "another code comment");
        assert_eq!(notes[1].author.username, "octocat");
        let pos2 = notes[1].position.as_ref().unwrap();
        assert_eq!(pos2.new_path.as_deref(), Some("src/main.rs"));
        assert_eq!(pos2.old_line, Some(10));
        assert!(pos2.new_line.is_none());
        assert_eq!(notes[1].discussion_id.as_deref(), Some("1"));
    }
}
