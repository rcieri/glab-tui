use crate::backend::Backend;
use anyhow::{Context, Result};
use tokio::process::Command;

pub struct GitlabClient {
    pub is_github: bool,
    pub backend: Box<dyn Backend>,
    pub tx: Option<tokio::sync::mpsc::UnboundedSender<crate::event::Event>>,
    pub page_size: usize,
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
        })
    }

    pub fn program(&self) -> &'static str {
        self.backend.program()
    }

    pub fn muted(mut self) -> Self {
        self.tx = None;
        self.backend.clear_tx();
        self
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
        let branches = self
            .backend
            .list_branches(project_path, self.page_size)
            .await?;
        Ok(branches.into_iter().map(|b| b.name).collect())
    }

    pub async fn fetch_milestones(&self, project_path: &str) -> Result<Vec<String>> {
        let milestones = self
            .backend
            .list_milestones(project_path, self.page_size)
            .await?;
        Ok(milestones.into_iter().map(|m| m.title).collect())
    }

    pub async fn raw_api(
        &self,
        endpoint: &str,
        method: &str,
        body: Option<&str>,
        desc: &str,
    ) -> Result<String> {
        self.backend.raw_api(endpoint, method, body, desc).await
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
        }
    }
}

impl std::fmt::Debug for GitlabClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitlabClient")
            .field("is_github", &self.is_github)
            .field("page_size", &self.page_size)
            .finish()
    }
}

impl GitlabClient {
    pub async fn execute_raw_command(
        &self,
        program: &str,
        args: &[&str],
        desc: &str,
    ) -> Result<String> {
        let label = desc.to_uppercase();
        let cmd_str = format!("{} {}", program, args.join(" "));

        let output = Command::new(program)
            .args(args)
            .output()
            .await
            .context(format!("Failed to execute {} command", program));

        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        match output {
            Ok(out) => {
                if out.status.success() {
                    let s = String::from_utf8(out.stdout)?;
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: format!("{}: {}", label, cmd_str),
                            status: "Success".to_string(),
                        });
                    }
                    Ok(s)
                } else {
                    let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: format!("{}: {}", label, cmd_str),
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
                        command: format!("{}: {}", label, cmd_str),
                        status: format!("Failed: {}", err_msg),
                    });
                }
                Err(e.into())
            }
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
