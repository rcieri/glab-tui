use crate::backend::{Backend, BackendKind};
use anyhow::{Context, Result};

pub struct GitlabClient {
    pub is_github: bool,
    pub backend: Box<dyn Backend>,
    pub tx: Option<tokio::sync::mpsc::UnboundedSender<crate::event::Event>>,
    pub page_size: usize,
}

impl GitlabClient {
    pub fn kind(&self) -> BackendKind {
        self.backend.kind()
    }

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

    pub async fn start_job(&self, project_path: &str, job_id: u64) -> Result<()> {
        self.backend.start_job(project_path, job_id).await
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

    // ── Issue mutations ──
    pub async fn close_issue(&self, project: &str, iid: u64) -> Result<()> {
        self.backend.close_issue(project, iid).await
    }

    pub async fn reopen_issue(&self, project: &str, iid: u64) -> Result<()> {
        self.backend.reopen_issue(project, iid).await
    }

    pub async fn delete_issue(&self, project: &str, iid: u64) -> Result<()> {
        self.backend.delete_issue(project, iid).await
    }

    pub async fn create_issue(
        &self,
        project: &str,
        title: &str,
        description: &str,
        labels: &str,
        assignees: &str,
        milestone: &str,
        due_date: &str,
        weight: &str,
    ) -> Result<()> {
        self.backend
            .create_issue(
                project,
                title,
                description,
                labels,
                assignees,
                milestone,
                due_date,
                weight,
            )
            .await
    }

    // ── MR mutations ──
    pub async fn close_mr(&self, project: &str, iid: u64) -> Result<()> {
        self.backend.close_mr(project, iid).await
    }

    pub async fn reopen_mr(&self, project: &str, iid: u64) -> Result<()> {
        self.backend.reopen_mr(project, iid).await
    }

    pub async fn delete_mr(&self, project: &str, iid: u64) -> Result<()> {
        self.backend.delete_mr(project, iid).await
    }

    pub async fn approve_mr(&self, project: &str, iid: u64) -> Result<()> {
        self.backend.approve_mr(project, iid).await
    }

    pub async fn merge_mr(
        &self,
        project: &str,
        iid: u64,
        squash: bool,
        delete_branch: bool,
        strategy: Option<&str>,
    ) -> Result<()> {
        self.backend
            .merge_mr(project, iid, squash, delete_branch, strategy)
            .await
    }

    pub async fn toggle_mr_draft(&self, project: &str, iid: u64, is_draft: bool) -> Result<()> {
        self.backend.toggle_mr_draft(project, iid, is_draft).await
    }

    pub async fn create_mr(
        &self,
        project: &str,
        title: &str,
        description: &str,
        source_branch: &str,
        target_branch: &str,
        labels: &str,
        assignees: &str,
        reviewers: &str,
        milestone: &str,
        issue_iid: Option<u64>,
    ) -> Result<()> {
        self.backend
            .create_mr(
                project,
                title,
                description,
                source_branch,
                target_branch,
                labels,
                assignees,
                reviewers,
                milestone,
                issue_iid,
            )
            .await
    }

    pub async fn add_mr_comment(
        &self,
        project: &str,
        iid: u64,
        body: &str,
        file_path: Option<&str>,
        line: Option<u64>,
        old_line: Option<u64>,
    ) -> Result<()> {
        self.backend
            .add_mr_comment(project, iid, body, file_path, line, old_line)
            .await
    }

    // ── Pipeline mutations ──
    pub async fn run_pipeline(
        &self,
        project: &str,
        branch: &str,
        mr: bool,
        variables: &[(String, String)],
        inputs: &[(String, String)],
        workflow_file: &str,
    ) -> Result<()> {
        self.backend
            .run_pipeline(project, branch, mr, variables, inputs, workflow_file)
            .await
    }

    pub async fn download_artifact(
        &self,
        project: &str,
        ref_name: &str,
        job_name: &str,
    ) -> Result<()> {
        self.backend
            .download_artifact(project, ref_name, job_name)
            .await
    }

    // ── Runner mutations ──
    pub async fn pause_runner(&self, project: &str, runner_id: u64) -> Result<()> {
        self.backend.pause_runner(project, runner_id).await
    }

    pub async fn resume_runner(&self, project: &str, runner_id: u64) -> Result<()> {
        self.backend.resume_runner(project, runner_id).await
    }

    // ── Release mutations ──
    pub async fn create_release(
        &self,
        project: &str,
        tag: &str,
        name: &str,
        description: &str,
    ) -> Result<()> {
        self.backend
            .create_release(project, tag, name, description)
            .await
    }

    pub async fn delete_release(&self, project: &str, tag: &str) -> Result<()> {
        self.backend.delete_release(project, tag).await
    }

    // ── Milestone mutations ──
    pub async fn create_milestone(
        &self,
        project: &str,
        title: &str,
        description: &str,
        start_date: Option<&str>,
        due_date: Option<&str>,
    ) -> Result<()> {
        self.backend
            .create_milestone(project, title, description, start_date, due_date)
            .await
    }

    // ── Field updates ──
    pub async fn update_issue_title(&self, project: &str, iid: u64, title: &str) -> Result<()> {
        self.backend.update_issue_title(project, iid, title).await
    }
    pub async fn update_issue_description(
        &self,
        project: &str,
        iid: u64,
        desc: &str,
    ) -> Result<()> {
        self.backend
            .update_issue_description(project, iid, desc)
            .await
    }
    pub async fn update_issue_labels(
        &self,
        project: &str,
        iid: u64,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        self.backend
            .update_issue_labels(project, iid, add, remove)
            .await
    }
    pub async fn update_issue_assignees(
        &self,
        project: &str,
        iid: u64,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        self.backend
            .update_issue_assignees(project, iid, add, remove)
            .await
    }
    pub async fn update_issue_milestone(
        &self,
        project: &str,
        iid: u64,
        milestone: &str,
    ) -> Result<()> {
        self.backend
            .update_issue_milestone(project, iid, milestone)
            .await
    }
    pub async fn update_issue_due_date(
        &self,
        project: &str,
        iid: u64,
        due_date: &str,
    ) -> Result<()> {
        self.backend
            .update_issue_due_date(project, iid, due_date)
            .await
    }
    pub async fn update_issue_weight(&self, project: &str, iid: u64, weight: &str) -> Result<()> {
        self.backend.update_issue_weight(project, iid, weight).await
    }
    pub async fn update_issue_confidential(
        &self,
        project: &str,
        iid: u64,
        confidential: bool,
    ) -> Result<()> {
        self.backend
            .update_issue_confidential(project, iid, confidential)
            .await
    }
    pub async fn update_mr_title(&self, project: &str, iid: u64, title: &str) -> Result<()> {
        self.backend.update_mr_title(project, iid, title).await
    }
    pub async fn update_mr_description(&self, project: &str, iid: u64, desc: &str) -> Result<()> {
        self.backend.update_mr_description(project, iid, desc).await
    }
    pub async fn update_mr_labels(
        &self,
        project: &str,
        iid: u64,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        self.backend
            .update_mr_labels(project, iid, add, remove)
            .await
    }
    pub async fn update_mr_assignees(
        &self,
        project: &str,
        iid: u64,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        self.backend
            .update_mr_assignees(project, iid, add, remove)
            .await
    }
    pub async fn update_mr_reviewers(
        &self,
        project: &str,
        iid: u64,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        self.backend
            .update_mr_reviewers(project, iid, add, remove)
            .await
    }
    pub async fn update_mr_milestone(
        &self,
        project: &str,
        iid: u64,
        milestone: &str,
    ) -> Result<()> {
        self.backend
            .update_mr_milestone(project, iid, milestone)
            .await
    }
    pub async fn update_mr_target_branch(
        &self,
        project: &str,
        iid: u64,
        branch: &str,
    ) -> Result<()> {
        self.backend
            .update_mr_target_branch(project, iid, branch)
            .await
    }
    pub async fn open_in_browser(&self, project: &str, entity: &str, id: &str) -> Result<()> {
        self.backend.open_in_browser(project, entity, id).await
    }

    pub async fn open_pipeline_in_browser(&self, project: &str, id: &str) -> Result<()> {
        self.backend.open_pipeline_in_browser(project, id).await
    }
    pub async fn open_job_in_browser(&self, project: &str, id: &str) -> Result<()> {
        self.backend.open_job_in_browser(project, id).await
    }
    pub async fn open_milestone_in_browser(&self, project: &str, id: &str) -> Result<()> {
        self.backend.open_milestone_in_browser(project, id).await
    }

    pub async fn bulk_update_issues_labels(
        &self,
        project: &str,
        iids: &[u64],
        labels: &str,
    ) -> Result<()> {
        if labels.trim().is_empty() {
            return Ok(());
        }
        let add: Vec<String> = labels
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        for &iid in iids {
            self.backend
                .update_issue_labels(project, iid, &add, &[])
                .await?;
        }
        Ok(())
    }

    pub async fn bulk_update_issues_assignees(
        &self,
        project: &str,
        iids: &[u64],
        assignees: &str,
    ) -> Result<()> {
        if assignees.trim().is_empty() {
            return Ok(());
        }
        let add: Vec<String> = assignees
            .split(',')
            .map(|s| s.trim().trim_start_matches('@').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        for &iid in iids {
            self.backend
                .update_issue_assignees(project, iid, &add, &[])
                .await?;
        }
        Ok(())
    }

    pub async fn bulk_update_issues_milestone(
        &self,
        project: &str,
        iids: &[u64],
        milestone: &str,
    ) -> Result<()> {
        for &iid in iids {
            self.backend
                .update_issue_milestone(project, iid, milestone)
                .await?;
        }
        Ok(())
    }

    pub async fn bulk_update_mrs_labels(
        &self,
        project: &str,
        iids: &[u64],
        labels: &str,
    ) -> Result<()> {
        if labels.trim().is_empty() {
            return Ok(());
        }
        let add: Vec<String> = labels
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        for &iid in iids {
            self.backend
                .update_mr_labels(project, iid, &add, &[])
                .await?;
        }
        Ok(())
    }

    pub async fn bulk_update_mrs_assignees(
        &self,
        project: &str,
        iids: &[u64],
        assignees: &str,
    ) -> Result<()> {
        if assignees.trim().is_empty() {
            return Ok(());
        }
        let add: Vec<String> = assignees
            .split(',')
            .map(|s| s.trim().trim_start_matches('@').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        for &iid in iids {
            self.backend
                .update_mr_assignees(project, iid, &add, &[])
                .await?;
        }
        Ok(())
    }

    pub async fn bulk_update_mrs_milestone(
        &self,
        project: &str,
        iids: &[u64],
        milestone: &str,
    ) -> Result<()> {
        for &iid in iids {
            self.backend
                .update_mr_milestone(project, iid, milestone)
                .await?;
        }
        Ok(())
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
