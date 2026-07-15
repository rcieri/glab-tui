pub mod gh;
pub mod glab;

use crate::domain::branches::Branch;
use crate::domain::deployments::{Deployment, Environment};
use crate::domain::issues::Issue;
use crate::domain::milestones::Milestone;
use crate::domain::mr::{DiscussionNote, MergeRequest};
use crate::domain::notifications::Notification;
use crate::domain::pipelines::{Job, Pipeline};
use crate::domain::releases::Release;
use crate::domain::runners::Runner;
use crate::event::Event;
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;

#[async_trait]
pub trait Backend: Send + Sync {
    fn program(&self) -> &'static str;

    fn set_tx(&mut self, tx: UnboundedSender<Event>);

    // ── Issues ──
    async fn list_issues(
        &self,
        project: &str,
        show_closed: bool,
        page_size: usize,
    ) -> Result<Vec<Issue>>;
    async fn get_issue(&self, project: &str, iid: u64) -> Result<Issue>;
    async fn close_issue(&self, project: &str, iid: u64) -> Result<()>;
    async fn reopen_issue(&self, project: &str, iid: u64) -> Result<()>;
    async fn delete_issue(&self, project: &str, iid: u64) -> Result<()>;
    async fn create_issue(
        &self,
        project: &str,
        title: &str,
        description: &str,
        labels: &str,
        assignees: &str,
        milestone: &str,
        due_date: &str,
        weight: &str,
    ) -> Result<()>;
    async fn update_issue_title(&self, project: &str, iid: u64, title: &str) -> Result<()>;
    async fn update_issue_description(
        &self,
        project: &str,
        iid: u64,
        description: &str,
    ) -> Result<()>;
    async fn update_issue_labels(
        &self,
        project: &str,
        iid: u64,
        add_labels: &[String],
        remove_labels: &[String],
    ) -> Result<()>;
    async fn update_issue_assignees(
        &self,
        project: &str,
        iid: u64,
        add: &[String],
        remove: &[String],
    ) -> Result<()>;
    async fn update_issue_milestone(&self, project: &str, iid: u64, milestone: &str) -> Result<()>;
    async fn update_issue_due_date(&self, project: &str, iid: u64, due_date: &str) -> Result<()>;
    async fn update_issue_weight(&self, project: &str, iid: u64, weight: &str) -> Result<()>;
    async fn update_issue_confidential(
        &self,
        project: &str,
        iid: u64,
        confidential: bool,
    ) -> Result<()>;

    // ── Merge Requests ──
    async fn list_mrs(
        &self,
        project: &str,
        show_closed: bool,
        page_size: usize,
    ) -> Result<Vec<MergeRequest>>;
    async fn get_mr(&self, project: &str, iid: u64) -> Result<MergeRequest>;
    async fn get_mr_diff(&self, project: &str, iid: u64) -> Result<String>;
    async fn list_mr_notes(
        &self,
        project: &str,
        mr_iid: u64,
        page_size: usize,
    ) -> Result<Vec<DiscussionNote>>;
    async fn close_mr(&self, project: &str, iid: u64) -> Result<()>;
    async fn reopen_mr(&self, project: &str, iid: u64) -> Result<()>;
    async fn delete_mr(&self, project: &str, iid: u64) -> Result<()>;
    async fn approve_mr(&self, project: &str, iid: u64) -> Result<()>;
    async fn merge_mr(
        &self,
        project: &str,
        iid: u64,
        squash: bool,
        delete_branch: bool,
        strategy: Option<&str>,
    ) -> Result<()>;
    async fn toggle_mr_draft(&self, project: &str, iid: u64, is_draft: bool) -> Result<()>;
    async fn create_mr(
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
    ) -> Result<()>;
    async fn add_mr_comment(
        &self,
        project: &str,
        iid: u64,
        body: &str,
        file_path: Option<&str>,
        line: Option<u64>,
        old_line: Option<u64>,
    ) -> Result<()>;
    async fn update_mr_title(&self, project: &str, iid: u64, title: &str) -> Result<()>;
    async fn update_mr_description(&self, project: &str, iid: u64, description: &str)
    -> Result<()>;
    async fn update_mr_labels(
        &self,
        project: &str,
        iid: u64,
        add_labels: &[String],
        remove_labels: &[String],
    ) -> Result<()>;
    async fn update_mr_assignees(
        &self,
        project: &str,
        iid: u64,
        add: &[String],
        remove: &[String],
    ) -> Result<()>;
    async fn update_mr_reviewers(
        &self,
        project: &str,
        iid: u64,
        add: &[String],
        remove: &[String],
    ) -> Result<()>;
    async fn update_mr_milestone(&self, project: &str, iid: u64, milestone: &str) -> Result<()>;
    async fn update_mr_target_branch(&self, project: &str, iid: u64, branch: &str) -> Result<()>;

    // ── Browser ──
    async fn open_in_browser(&self, project: &str, entity: &str, id: &str) -> Result<()>;

    // ── Pipelines ──
    async fn list_pipelines(&self, project: &str, page_size: usize) -> Result<Vec<Pipeline>>;
    async fn list_pipeline_jobs(
        &self,
        project: &str,
        pipeline_id: u64,
        page_size: usize,
    ) -> Result<Vec<Job>>;
    async fn get_job_trace(&self, project: &str, job_id: u64) -> Result<String>;

    // ── Pipeline / Job actions ──
    async fn retry_pipeline(&self, project: &str, pipeline_id: u64) -> Result<()>;
    async fn cancel_pipeline(&self, project: &str, pipeline_id: u64) -> Result<()>;
    async fn retry_job(&self, project: &str, job_id: u64) -> Result<()>;
    async fn cancel_job(&self, project: &str, job_id: u64) -> Result<()>;
    async fn run_pipeline(
        &self,
        project: &str,
        branch: &str,
        mr: bool,
        variables: &[(String, String)],
        inputs: &[(String, String)],
        workflow_file: &str,
    ) -> Result<()>;
    async fn download_artifact(&self, project: &str, ref_name: &str, job_name: &str) -> Result<()>;

    // ── Runners ──
    async fn list_runners(&self, project: &str, page_size: usize) -> Result<Vec<Runner>>;
    async fn pause_runner(&self, project: &str, runner_id: u64) -> Result<()>;
    async fn resume_runner(&self, project: &str, runner_id: u64) -> Result<()>;
    async fn update_runner_description(
        &self,
        project: &str,
        runner_id: u64,
        description: &str,
    ) -> Result<()>;

    // ── Releases ──
    async fn list_releases(&self, project: &str, page_size: usize) -> Result<Vec<Release>>;
    async fn create_release(
        &self,
        project: &str,
        tag: &str,
        name: &str,
        description: &str,
    ) -> Result<()>;
    async fn update_release(
        &self,
        project: &str,
        tag_name: &str,
        name: &str,
        description: &str,
    ) -> Result<()>;
    async fn delete_release(&self, project: &str, tag_name: &str) -> Result<()>;

    // ── Milestones ──
    async fn list_milestones(&self, project: &str, page_size: usize) -> Result<Vec<Milestone>>;
    async fn list_milestone_issues(
        &self,
        project: &str,
        milestone_iid: u64,
        page_size: usize,
    ) -> Result<Vec<Issue>>;
    async fn create_milestone(
        &self,
        project: &str,
        title: &str,
        description: &str,
        start_date: Option<&str>,
        due_date: Option<&str>,
    ) -> Result<()>;
    async fn update_milestone_state(
        &self,
        project: &str,
        milestone_iid: u64,
        close: bool,
    ) -> Result<()>;
    async fn update_milestone(
        &self,
        project: &str,
        milestone_iid: u64,
        title: &str,
        description: &str,
        start_date: Option<&str>,
        due_date: Option<&str>,
    ) -> Result<()>;
    async fn delete_milestone(&self, project: &str, milestone_iid: u64) -> Result<()>;

    // ── Notifications ──
    async fn list_notifications(&self, show_read: bool) -> Result<Vec<Notification>>;
    async fn mark_notification_as_read(&self, id: &str) -> Result<()>;

    // ── Branches ──
    async fn list_branches(&self, project: &str, page_size: usize) -> Result<Vec<Branch>>;
    async fn create_branch(&self, project: &str, branch_name: &str, ref_branch: &str)
    -> Result<()>;
    async fn delete_branch(&self, project: &str, branch_name: &str) -> Result<()>;

    // ── Environments / Deployments ──
    async fn list_environments(&self, project: &str, page_size: usize) -> Result<Vec<Environment>>;
    async fn list_deployments(
        &self,
        project: &str,
        page_size: usize,
        environment: Option<&str>,
    ) -> Result<Vec<Deployment>>;

    // ── Labels / Members / Misc ──
    async fn fetch_labels(&self, project: &str) -> Result<Vec<String>>;
    async fn fetch_members(&self, project: &str) -> Result<Vec<String>>;

    // ── Raw API fallback ──
    async fn raw_api(
        &self,
        endpoint: &str,
        method: &str,
        body: Option<&str>,
        desc: &str,
    ) -> Result<String>;
}

pub fn create_backend(project_url_contains_github: bool) -> Box<dyn Backend> {
    if project_url_contains_github {
        Box::new(gh::GhBackend::new())
    } else {
        Box::new(glab::GlabBackend::new())
    }
}
