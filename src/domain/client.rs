use crate::backend::Backend;
use anyhow::{Context, Result};
use tokio::process::Command;

pub struct GitlabClient {
    pub is_github: bool,
    pub backend: Box<dyn Backend>,
    pub tx: Option<tokio::sync::mpsc::UnboundedSender<crate::event::Event>>,
    pub page_size: usize,
    pub page_limit: Option<u32>,
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
        let backend = crate::backend::create_backend(is_github);
        Ok(Self {
            is_github,
            backend,
            tx: None,
            page_size: 100,
            page_limit: Some(10),
        })
    }

    pub fn program(&self) -> &'static str {
        self.backend.program()
    }

    pub async fn retry_pipeline(&self, project_path: &str, pipeline_id: u64) -> Result<()> {
        self.backend.retry_pipeline(project_path, pipeline_id).await
    }

    pub async fn cancel_pipeline(&self, project_path: &str, pipeline_id: u64) -> Result<()> {
        self.backend
            .cancel_pipeline(project_path, pipeline_id)
            .await
    }

    pub async fn retry_job(&self, project_path: &str, job_id: u64) -> Result<()> {
        self.backend.retry_job(project_path, job_id).await
    }

    pub async fn cancel_job(&self, project_path: &str, job_id: u64) -> Result<()> {
        self.backend.cancel_job(project_path, job_id).await
    }

    pub async fn fetch_labels(&self, project_path: &str) -> Result<Vec<String>> {
        self.backend.fetch_labels(project_path).await
    }

    pub async fn fetch_members(&self, project_path: &str) -> Result<Vec<String>> {
        self.backend.fetch_members(project_path).await
    }

    pub async fn fetch_branches(&self, project_path: &str) -> Result<Vec<String>> {
        self.backend.fetch_branch_names(project_path).await
    }

    pub async fn fetch_milestones(&self, project_path: &str) -> Result<Vec<String>> {
        self.backend.fetch_milestone_titles(project_path).await
    }

    pub async fn raw_api(
        &self,
        endpoint: &str,
        method: &str,
        body: Option<&str>,
    ) -> Result<String> {
        self.backend.raw_api(endpoint, method, body).await
    }
}

impl Clone for GitlabClient {
    fn clone(&self) -> Self {
        let mut backend = crate::backend::create_backend(self.is_github);
        if let Some(ref tx) = self.tx {
            backend.set_tx(tx.clone());
        }
        Self {
            is_github: self.is_github,
            backend,
            tx: self.tx.clone(),
            page_size: self.page_size,
            page_limit: self.page_limit,
        }
    }
}

impl std::fmt::Debug for GitlabClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitlabClient")
            .field("is_github", &self.is_github)
            .field("page_size", &self.page_size)
            .field("page_limit", &self.page_limit)
            .finish()
    }
}

impl GitlabClient {
    // Helper to generate human-readable action label from method and endpoint
    // Outputs concise, uppercase action label, e.g., "FETCHING BRANCHES", "CREATING BRANCH"
    fn generate_action_label(&self, endpoint: &str, method: &str) -> String {
        // Strip query params
        let path = endpoint.split('?').next().unwrap_or(endpoint);
        // Normalize endpoints by replacing IDs / project contexts
        let mut segments = Vec::new();
        for seg in path.split('/') {
            if seg.is_empty() || seg == "api" || seg == "v4" || seg == "repos" || seg == "projects"
            {
                continue;
            }
            // If it is numeric (ID) or matches a common project context placeholder, ignore or normalize it
            if seg.chars().all(|c| c.is_ascii_digit()) {
                segments.push("{id}");
            } else {
                segments.push(seg);
            }
        }

        let normalized = segments.join("/");
        let normalized_lower = normalized.to_lowercase();

        let action = match method {
            "GET" => "FETCHING",
            "POST" => "CREATING",
            "PUT" | "PATCH" => "UPDATING",
            "DELETE" => "DELETING",
            _ => "RUNNING",
        };

        // Match exact normalized structures
        let target = match normalized_lower.as_str() {
            s if s.contains("repository/branches") || s.contains("branches") => "BRANCHES",
            s if s.contains("environments") => "ENVIRONMENTS",
            s if s.contains("deployments") => "DEPLOYMENTS",
            s if s.contains("issues") => "ISSUES",
            s if s.contains("pulls") || s.contains("merge_requests") => {
                if self.is_github {
                    "PRS"
                } else {
                    "MRS"
                }
            }
            s if s.contains("releases") => "RELEASES",
            s if s.contains("milestones") => "MILESTONES",
            s if s.contains("todos") || s.contains("notifications") => "TODOS",
            s if s.contains("pipelines") || s.contains("actions/runs") => "PIPELINES",
            s if s.contains("jobs") && s.contains("trace") => "JOB LOGS",
            s if s.contains("jobs") || s.contains("actions/jobs") => "JOBS",
            s if s.contains("runners") => "RUNNERS",
            s if s.contains("labels") => "LABELS",
            s if s.contains("members") => "MEMBERS",
            _ => "RESOURCE",
        };

        format!("{} {}", action, target)
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

        let action_label = self.generate_action_label(endpoint, method);
        let cmd_log_str = format!("{}: {}", action_label, cmd_str);

        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        if let Some(ref tx) = self.tx {
            let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                timestamp: timestamp.clone(),
                command: cmd_log_str.clone(),
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
                            command: cmd_log_str.clone(),
                            status: "Success".to_string(),
                        });
                    }
                    Ok(s)
                } else {
                    let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: cmd_log_str.clone(),
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
                        command: cmd_log_str.clone(),
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

        let action_label = self.generate_action_label(endpoint, method);
        let cmd_log_str = format!("{}: {}", action_label, cmd_str);

        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        if let Some(ref tx) = self.tx {
            let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                timestamp: timestamp.clone(),
                command: cmd_log_str.clone(),
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
                            command: cmd_log_str.clone(),
                            status: "Success".to_string(),
                        });
                    }
                    Ok(s)
                } else {
                    let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: cmd_log_str.clone(),
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
                        command: cmd_log_str.clone(),
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
        let cmd_log_str = format!("{}: {}", desc.to_uppercase(), cmd_str);
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        if let Some(ref tx) = self.tx {
            let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                timestamp: timestamp.clone(),
                command: cmd_log_str.clone(),
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
                            command: cmd_log_str.clone(),
                            status: "Success".to_string(),
                        });
                    }
                    Ok(s)
                } else {
                    let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: cmd_log_str.clone(),
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
                        command: cmd_log_str.clone(),
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
}

pub async fn get_project_context() -> Result<String> {
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
    // Tests moved to domain files and backend modules.
}
