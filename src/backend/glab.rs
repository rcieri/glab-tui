use super::Backend;
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
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

pub struct GlabBackend {
    tx: Option<UnboundedSender<Event>>,
}

impl GlabBackend {
    pub fn new() -> Self {
        Self { tx: None }
    }

    fn encode_path(project: &str) -> String {
        project.replace('/', "%2F")
    }

    async fn run_glab(&self, args: &[&str], desc: &str) -> Result<String> {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        let label = format!("{:<24}", desc.to_uppercase());
        let cmd_str = format!("glab {}", args.join(" "));
        if let Some(ref tx) = self.tx {
            let _ = tx.send(Event::TerminalCommandLogged {
                timestamp: timestamp.clone(),
                command: format!("{} {}", label, cmd_str),
                status: "Running".to_string(),
            });
        }

        let output = Command::new("glab")
            .args(args)
            .output()
            .await
            .with_context(|| format!("Failed to execute: glab {}", args.join(" ")))?;

        if output.status.success() {
            let s = String::from_utf8(output.stdout)?;
            if let Some(ref tx) = self.tx {
                let _ = tx.send(Event::TerminalCommandLogged {
                    timestamp,
                    command: format!("{} {}", label, cmd_str),
                    status: "Success".to_string(),
                });
            }
            Ok(s)
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if let Some(ref tx) = self.tx {
                let _ = tx.send(Event::TerminalCommandLogged {
                    timestamp,
                    command: format!("{} {}", label, cmd_str),
                    status: format!("Failed: {}", err_msg),
                });
            }
            anyhow::bail!("glab command failed: {}", err_msg)
        }
    }
}

#[async_trait]
impl Backend for GlabBackend {
    fn program(&self) -> &'static str {
        "glab"
    }

    fn set_tx(&mut self, tx: UnboundedSender<Event>) {
        self.tx = Some(tx);
    }

    // ── Issues ──

    async fn list_issues(
        &self,
        project: &str,
        show_closed: bool,
        page_size: usize,
    ) -> Result<Vec<Issue>> {
        let state = if show_closed { "all" } else { "opened" };
        let encoded = Self::encode_path(project);
        let pages = page_size.div_ceil(100).max(1);
        let mut all: Vec<Issue> = Vec::new();
        for page in 1..=pages {
            let raw = self
                .run_glab(
                    &[
                        "issue",
                        "list",
                        "--output",
                        "json",
                        "-R",
                        &encoded,
                        "--state",
                        state,
                        "--page",
                        &page.to_string(),
                        "--per-page",
                        "100",
                    ],
                    "Fetching Issues",
                )
                .await?;
            #[derive(Deserialize)]
            struct GiIssue {
                iid: u64,
                title: String,
                state: String,
                #[serde(default)]
                labels: Vec<String>,
                updated_at: String,
                #[serde(default)]
                created_at: Option<String>,
                #[serde(default)]
                closed_at: Option<String>,
                author: GiAuthor,
                milestone: Option<GiMilestone>,
                #[serde(default)]
                assignees: Vec<GiAssignee>,
                #[serde(default)]
                description: Option<String>,
                #[serde(default)]
                due_date: Option<String>,
            }
            #[derive(Deserialize)]
            struct GiAuthor {
                username: String,
            }
            #[derive(Deserialize)]
            struct GiMilestone {
                title: String,
            }
            #[derive(Deserialize)]
            struct GiAssignee {
                username: String,
            }
            let issues: Vec<GiIssue> = serde_json::from_str(&raw).unwrap_or_default();
            let len = issues.len();
            all.extend(issues.into_iter().map(|i| {
                Issue {
                    iid: i.iid,
                    title: i.title,
                    state: i.state,
                    labels: i.labels,
                    updated_at: i.updated_at,
                    created_at: i.created_at,
                    closed_at: i.closed_at,
                    author: crate::domain::issues::Author {
                        username: i.author.username,
                    },
                    milestone: i
                        .milestone
                        .map(|m| crate::domain::issues::Milestone { title: m.title }),
                    assignees: i
                        .assignees
                        .into_iter()
                        .map(|a| crate::domain::issues::Assignee {
                            username: a.username,
                        })
                        .collect(),
                    description: i.description,
                    due_date: i.due_date,
                }
            }));
            if len < 100 {
                break;
            }
        }
        Ok(all)
    }

    async fn get_issue(&self, project: &str, iid: u64) -> Result<Issue> {
        let encoded = Self::encode_path(project);
        let raw = self
            .run_glab(
                &[
                    "issue",
                    "view",
                    &iid.to_string(),
                    "--output",
                    "json",
                    "-R",
                    &encoded,
                ],
                "Fetching Issue",
            )
            .await?;
        #[derive(Deserialize)]
        struct GiIssue {
            iid: u64,
            title: String,
            state: String,
            #[serde(default)]
            labels: Vec<String>,
            updated_at: String,
            #[serde(default)]
            created_at: Option<String>,
            #[serde(default)]
            closed_at: Option<String>,
            author: GiAuthor,
            milestone: Option<GiMilestone>,
            #[serde(default)]
            assignees: Vec<GiAssignee>,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            due_date: Option<String>,
        }
        #[derive(Deserialize)]
        struct GiAuthor {
            username: String,
        }
        #[derive(Deserialize)]
        struct GiMilestone {
            title: String,
        }
        #[derive(Deserialize)]
        struct GiAssignee {
            username: String,
        }
        let i: GiIssue = serde_json::from_str(&raw)?;
        Ok(Issue {
            iid: i.iid,
            title: i.title,
            state: i.state,
            labels: i.labels,
            updated_at: i.updated_at,
            created_at: i.created_at,
            closed_at: i.closed_at,
            author: crate::domain::issues::Author {
                username: i.author.username,
            },
            milestone: i
                .milestone
                .map(|m| crate::domain::issues::Milestone { title: m.title }),
            assignees: i
                .assignees
                .into_iter()
                .map(|a| crate::domain::issues::Assignee {
                    username: a.username,
                })
                .collect(),
            description: i.description,
            due_date: i.due_date,
        })
    }

    // ── Merge Requests ──

    async fn list_mrs(
        &self,
        project: &str,
        show_closed: bool,
        page_size: usize,
    ) -> Result<Vec<MergeRequest>> {
        let state = if show_closed { "all" } else { "opened" };
        let encoded = Self::encode_path(project);
        let pages = page_size.div_ceil(100).max(1);
        let mut all: Vec<MergeRequest> = Vec::new();
        for page in 1..=pages {
            let raw = self
                .run_glab(
                    &[
                        "mr",
                        "list",
                        "--output",
                        "json",
                        "-R",
                        &encoded,
                        "--state",
                        state,
                        "--page",
                        &page.to_string(),
                        "--per-page",
                        "100",
                    ],
                    "Fetching MRs",
                )
                .await?;
            #[derive(Deserialize)]
            struct GiMr {
                iid: u64,
                title: String,
                state: String,
                #[serde(default)]
                labels: Vec<String>,
                updated_at: String,
                author: GiAuthor,
                milestone: Option<GiMilestone>,
                #[serde(default)]
                assignees: Vec<GiAssignee>,
                #[serde(default)]
                reviewers: Vec<GiReviewer>,
                target_branch: String,
                #[serde(default)]
                source_branch: String,
                draft: bool,
                #[serde(default)]
                description: Option<String>,
                #[serde(default)]
                head_pipeline: Option<GiPipeline>,
            }
            #[derive(Deserialize)]
            struct GiAuthor {
                username: String,
            }
            #[derive(Deserialize)]
            struct GiMilestone {
                title: String,
            }
            #[derive(Deserialize)]
            struct GiAssignee {
                username: String,
            }
            #[derive(Deserialize)]
            struct GiReviewer {
                username: String,
            }
            #[derive(Deserialize)]
            struct GiPipeline {
                id: u64,
                status: String,
                #[serde(rename = "ref")]
                pipe_ref: String,
                updated_at: String,
            }
            let mrs: Vec<GiMr> = serde_json::from_str(&raw).unwrap_or_default();
            let len = mrs.len();
            all.extend(mrs.into_iter().map(|m| {
                MergeRequest {
                    iid: m.iid,
                    title: m.title,
                    state: m.state,
                    labels: m.labels,
                    updated_at: m.updated_at,
                    author: crate::domain::mr::Author {
                        username: m.author.username,
                    },
                    milestone: m
                        .milestone
                        .map(|ms| crate::domain::mr::Milestone { title: ms.title }),
                    assignees: m
                        .assignees
                        .into_iter()
                        .map(|a| crate::domain::mr::Assignee {
                            username: a.username,
                        })
                        .collect(),
                    reviewers: m
                        .reviewers
                        .into_iter()
                        .map(|r| crate::domain::mr::Reviewer {
                            username: r.username,
                        })
                        .collect(),
                    target_branch: m.target_branch,
                    source_branch: m.source_branch,
                    draft: m.draft,
                    description: m.description,
                    head_pipeline: m.head_pipeline.map(|p| Pipeline {
                        id: p.id,
                        status: p.status,
                        r#ref: p.pipe_ref,
                        updated_at: p.updated_at,
                        name: String::new(),
                        display_title: String::new(),
                        event: String::new(),
                        head_sha: String::new(),
                        actor_login: String::new(),
                    }),
                }
            }));
            if len < 100 {
                break;
            }
        }
        Ok(all)
    }

    async fn get_mr(&self, project: &str, iid: u64) -> Result<MergeRequest> {
        let encoded = Self::encode_path(project);
        let raw = self
            .run_glab(
                &[
                    "mr",
                    "view",
                    &iid.to_string(),
                    "--output",
                    "json",
                    "-R",
                    &encoded,
                ],
                "Fetching MR",
            )
            .await?;
        #[derive(Deserialize)]
        struct GiMr {
            iid: u64,
            title: String,
            state: String,
            #[serde(default)]
            labels: Vec<String>,
            updated_at: String,
            author: GiAuthor,
            milestone: Option<GiMilestone>,
            #[serde(default)]
            assignees: Vec<GiAssignee>,
            #[serde(default)]
            reviewers: Vec<GiReviewer>,
            target_branch: String,
            #[serde(default)]
            source_branch: String,
            draft: bool,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            head_pipeline: Option<GiPipeline>,
        }
        #[derive(Deserialize)]
        struct GiAuthor {
            username: String,
        }
        #[derive(Deserialize)]
        struct GiMilestone {
            title: String,
        }
        #[derive(Deserialize)]
        struct GiAssignee {
            username: String,
        }
        #[derive(Deserialize)]
        struct GiReviewer {
            username: String,
        }
        #[derive(Deserialize)]
        struct GiPipeline {
            id: u64,
            status: String,
            #[serde(rename = "ref")]
            pipe_ref: String,
            updated_at: String,
        }
        let m: GiMr = serde_json::from_str(&raw)?;
        Ok(MergeRequest {
            iid: m.iid,
            title: m.title,
            state: m.state,
            labels: m.labels,
            updated_at: m.updated_at,
            author: crate::domain::mr::Author {
                username: m.author.username,
            },
            milestone: m
                .milestone
                .map(|ms| crate::domain::mr::Milestone { title: ms.title }),
            assignees: m
                .assignees
                .into_iter()
                .map(|a| crate::domain::mr::Assignee {
                    username: a.username,
                })
                .collect(),
            reviewers: m
                .reviewers
                .into_iter()
                .map(|r| crate::domain::mr::Reviewer {
                    username: r.username,
                })
                .collect(),
            target_branch: m.target_branch,
            source_branch: m.source_branch,
            draft: m.draft,
            description: m.description,
            head_pipeline: m.head_pipeline.map(|p| Pipeline {
                id: p.id,
                status: p.status,
                r#ref: p.pipe_ref,
                updated_at: p.updated_at,
                name: String::new(),
                display_title: String::new(),
                event: String::new(),
                head_sha: String::new(),
                        actor_login: String::new(),
            }),
        })
    }

    async fn get_mr_diff(&self, project: &str, iid: u64) -> Result<String> {
        let encoded = Self::encode_path(project);
        self.run_glab(
            &["mr", "diff", &iid.to_string(), "-R", &encoded],
            "Fetching MR Diff",
        )
        .await
    }

    async fn list_mr_notes(
        &self,
        project: &str,
        mr_iid: u64,
        _page_size: usize,
    ) -> Result<Vec<DiscussionNote>> {
        let encoded = Self::encode_path(project);
        let raw = self
            .run_glab(
                &[
                    "mr",
                    "note",
                    "list",
                    &mr_iid.to_string(),
                    "--output",
                    "json",
                    "-R",
                    &encoded,
                ],
                "Fetching MR Notes",
            )
            .await?;
        #[derive(Deserialize)]
        struct GiNote {
            id: u64,
            body: String,
            author: GiAuthor,
            created_at: String,
            system: bool,
            #[serde(default)]
            position: Option<GiPosition>,
            #[serde(default)]
            discussion_id: Option<String>,
            #[serde(default)]
            resolved: Option<bool>,
            #[serde(default)]
            resolvable: Option<bool>,
        }
        #[derive(Deserialize)]
        struct GiAuthor {
            username: String,
        }
        #[derive(Deserialize)]
        struct GiPosition {
            #[serde(default)]
            new_path: Option<String>,
            #[serde(default)]
            old_path: Option<String>,
            #[serde(default)]
            new_line: Option<u64>,
            #[serde(default)]
            old_line: Option<u64>,
            #[serde(default)]
            start_line: Option<u64>,
            #[serde(default)]
            line_range: Option<serde_json::Value>,
        }
        let notes: Vec<GiNote> = serde_json::from_str(&raw)?;
        Ok(notes
            .into_iter()
            .map(|n| DiscussionNote {
                id: n.id,
                body: n.body,
                author: crate::domain::mr::Author {
                    username: n.author.username,
                },
                created_at: n.created_at,
                system: n.system,
                position: n.position.map(|p| crate::domain::mr::NotePosition {
                    new_path: p.new_path,
                    old_path: p.old_path,
                    new_line: p.new_line,
                    old_line: p.old_line,
                    start_line: p.start_line,
                    line_range: p.line_range,
                }),
                discussion_id: n.discussion_id,
                resolved: n.resolved,
                resolvable: n.resolvable,
            })
            .collect())
    }

    // ── Pipelines ──

    async fn list_pipelines(&self, project: &str, page_size: usize) -> Result<Vec<Pipeline>> {
        let encoded = Self::encode_path(project);
        let pages = page_size.div_ceil(100).max(1);
        let mut all: Vec<Pipeline> = Vec::new();
        for page in 1..=pages {
            let raw = self
                .run_glab(
                    &[
                        "ci",
                        "list",
                        "--output",
                        "json",
                        "-R",
                        &encoded,
                        "--page",
                        &page.to_string(),
                        "--per-page",
                        "100",
                    ],
                    "Fetching Pipelines",
                )
                .await?;
            #[derive(Deserialize)]
            struct GiPipe {
                id: u64,
                status: String,
                #[serde(rename = "ref")]
                pipe_ref: String,
                updated_at: String,
            }
            let pipes: Vec<GiPipe> = serde_json::from_str(&raw).unwrap_or_default();
            let len = pipes.len();
            all.extend(pipes.into_iter().map(|p| Pipeline {
                id: p.id,
                status: p.status,
                r#ref: p.pipe_ref,
                updated_at: p.updated_at,
                name: String::new(),
                display_title: String::new(),
                event: String::new(),
                head_sha: String::new(),
                        actor_login: String::new(),
            }));
            if len < 100 {
                break;
            }
        }
        Ok(all)
    }

    async fn list_pipeline_jobs(
        &self,
        project: &str,
        pipeline_id: u64,
        page_size: usize,
    ) -> Result<Vec<Job>> {
        let encoded = Self::encode_path(project);
        let endpoint = format!(
            "/projects/{}/pipelines/{}/jobs?per_page={}",
            encoded, pipeline_id, page_size
        );
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Pipeline Jobs")
            .await?;
        #[derive(Deserialize)]
        struct GiJob {
            id: u64,
            status: String,
            stage: String,
            name: String,
        }
        let jobs: Vec<GiJob> = serde_json::from_str(&raw)?;
        let all_jobs: Vec<Job> = jobs
            .into_iter()
            .map(|j| Job {
                id: j.id,
                status: j.status,
                stage: j.stage,
                name: j.name,
                matrix: None,
            })
            .collect();
        Ok(crate::domain::pipelines::process_pipeline_jobs(all_jobs))
    }

    async fn get_job_trace(&self, project: &str, job_id: u64) -> Result<String> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/jobs/{}/trace", encoded, job_id);
        self.raw_api(&endpoint, "GET", None, "Fetching Job Log")
            .await
    }

    async fn retry_pipeline(&self, project: &str, pipeline_id: u64) -> Result<()> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/pipelines/{}/retry", encoded, pipeline_id);
        self.raw_api(&endpoint, "POST", None, "Retrying Pipeline")
            .await?;
        Ok(())
    }

    async fn cancel_pipeline(&self, project: &str, pipeline_id: u64) -> Result<()> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/pipelines/{}/cancel", encoded, pipeline_id);
        self.raw_api(&endpoint, "POST", None, "Cancelling Pipeline")
            .await?;
        Ok(())
    }

    async fn retry_job(&self, project: &str, job_id: u64) -> Result<()> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/jobs/{}/retry", encoded, job_id);
        self.raw_api(&endpoint, "POST", None, "Retrying Job").await?;
        Ok(())
    }

    async fn cancel_job(&self, project: &str, job_id: u64) -> Result<()> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/jobs/{}/cancel", encoded, job_id);
        self.raw_api(&endpoint, "POST", None, "Cancelling Job")
            .await?;
        Ok(())
    }

    // ── Runners ──

    async fn list_runners(&self, project: &str, page_size: usize) -> Result<Vec<Runner>> {
        let encoded = Self::encode_path(project);
        let raw = self
            .run_glab(
                &[
                    "runner",
                    "list",
                    "--output",
                    "json",
                    "-R",
                    &encoded,
                    "--per-page",
                    &page_size.to_string(),
                ],
                "Fetching Runners",
            )
            .await?;
        #[derive(Deserialize)]
        struct GiRunner {
            id: u64,
            description: Option<String>,
            status: String,
            #[serde(default)]
            active: bool,
        }
        let runners: Vec<GiRunner> = serde_json::from_str(&raw)?;
        Ok(runners
            .into_iter()
            .map(|r| Runner {
                id: r.id,
                description: r.description,
                status: r.status,
                active: r.active,
            })
            .collect())
    }

    // ── Releases ──

    async fn list_releases(&self, project: &str, page_size: usize) -> Result<Vec<Release>> {
        let encoded = Self::encode_path(project);
        let raw = self
            .run_glab(
                &[
                    "release",
                    "list",
                    "--output",
                    "json",
                    "-R",
                    &encoded,
                    "--per-page",
                    &page_size.to_string(),
                ],
                "Fetching Releases",
            )
            .await?;
        #[derive(Deserialize)]
        struct GiRel {
            #[serde(default)]
            name: Option<String>,
            tag_name: String,
            released_at: String,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            author_name: Option<String>,
            #[serde(default)]
            commit: Option<GiRelCommit>,
            #[serde(default)]
            assets_link: Option<String>,
        }
        #[derive(Deserialize)]
        struct GiRelCommit {
            #[serde(default)]
            id: Option<String>,
            #[serde(default)]
            title: Option<String>,
        }
        let rels: Vec<GiRel> = serde_json::from_str(&raw)?;
        Ok(rels
            .into_iter()
            .map(|r| {
                let name = r.name.unwrap_or_else(|| r.tag_name.clone());
                let (commit_id, commit_title) = match r.commit {
                    Some(c) => (c.id, c.title),
                    None => (None, None),
                };
                Release {
                    name,
                    tag_name: r.tag_name,
                    released_at: r.released_at,
                    description: r.description,
                    author_name: r.author_name,
                    commit_id,
                    commit_title,
                    assets_link: r.assets_link,
                }
            })
            .collect())
    }

    async fn update_release(
        &self,
        project: &str,
        tag_name: &str,
        name: &str,
        description: &str,
    ) -> Result<()> {
        let encoded = Self::encode_path(project);
        self.run_glab(
            &[
                "release",
                "update",
                tag_name,
                "-R",
                &encoded,
                "-n",
                name,
                "-N",
                description,
            ],
            "Updating Release",
        )
        .await?;
        Ok(())
    }

    async fn delete_release(&self, project: &str, tag_name: &str) -> Result<()> {
        let encoded = Self::encode_path(project);
        self.run_glab(
            &["release", "delete", tag_name, "-R", &encoded, "-y"],
            "Deleting Release",
        )
        .await?;
        Ok(())
    }

    // ── Milestones ──

    async fn list_milestones(&self, project: &str, page_size: usize) -> Result<Vec<Milestone>> {
        let encoded = Self::encode_path(project);
        let raw = self
            .run_glab(
                &[
                    "milestone",
                    "list",
                    "--output",
                    "json",
                    "-R",
                    &encoded,
                    "--per-page",
                    &page_size.to_string(),
                ],
                "Fetching Milestones",
            )
            .await?;
        #[derive(Deserialize)]
        struct GiMs {
            id: u64,
            iid: u64,
            title: String,
            #[serde(default)]
            description: Option<String>,
            state: String,
            #[serde(default)]
            start_date: Option<String>,
            #[serde(default)]
            due_date: Option<String>,
            created_at: String,
        }
        let milestones: Vec<GiMs> = serde_json::from_str(&raw)?;
        Ok(milestones
            .into_iter()
            .map(|m| Milestone {
                id: m.id,
                iid: m.iid,
                title: m.title,
                description: m.description,
                state: m.state,
                start_date: m.start_date,
                due_date: m.due_date,
                created_at: m.created_at,
            })
            .collect())
    }

    async fn list_milestone_issues(
        &self,
        project: &str,
        milestone_iid: u64,
        page_size: usize,
    ) -> Result<Vec<Issue>> {
        let encoded = Self::encode_path(project);
        let endpoint = format!(
            "/projects/{}/milestones/{}/issues?per_page={}",
            encoded, milestone_iid, page_size
        );
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Milestone Issues")
            .await?;
        #[derive(Deserialize)]
        struct GiIssue {
            iid: u64,
            title: String,
            state: String,
            #[serde(default)]
            labels: Vec<String>,
            updated_at: String,
            #[serde(default)]
            created_at: Option<String>,
            #[serde(default)]
            closed_at: Option<String>,
            author: GiAuthor,
            milestone: Option<GiMilestone>,
            #[serde(default)]
            assignees: Vec<GiAssignee>,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            due_date: Option<String>,
        }
        #[derive(Deserialize)]
        struct GiAuthor {
            username: String,
        }
        #[derive(Deserialize)]
        struct GiMilestone {
            title: String,
        }
        #[derive(Deserialize)]
        struct GiAssignee {
            username: String,
        }
        let issues: Vec<GiIssue> = serde_json::from_str(&raw)?;
        Ok(issues
            .into_iter()
            .map(|i| Issue {
                iid: i.iid,
                title: i.title,
                state: i.state,
                labels: i.labels,
                updated_at: i.updated_at,
                created_at: i.created_at,
                closed_at: i.closed_at,
                author: crate::domain::issues::Author {
                    username: i.author.username,
                },
                milestone: i
                    .milestone
                    .map(|m| crate::domain::issues::Milestone { title: m.title }),
                assignees: i
                    .assignees
                    .into_iter()
                    .map(|a| crate::domain::issues::Assignee {
                        username: a.username,
                    })
                    .collect(),
                description: i.description,
                due_date: i.due_date,
            })
            .collect())
    }

    async fn update_milestone_state(
        &self,
        project: &str,
        milestone_iid: u64,
        close: bool,
    ) -> Result<()> {
        let encoded = Self::encode_path(project);
        let action = if close { "close" } else { "reopen" };
        self.run_glab(
            &[
                "milestone",
                action,
                &milestone_iid.to_string(),
                "-R",
                &encoded,
            ],
            "Updating Milestone State",
        )
        .await?;
        Ok(())
    }

    async fn update_milestone(
        &self,
        project: &str,
        milestone_iid: u64,
        title: &str,
        description: &str,
        start_date: Option<&str>,
        due_date: Option<&str>,
    ) -> Result<()> {
        let encoded = Self::encode_path(project);
        let mut args: Vec<String> = vec![
            "milestone".into(),
            "update".into(),
            milestone_iid.to_string(),
            "-R".into(),
            encoded,
            "--title".into(),
            title.into(),
            "--description".into(),
            description.into(),
        ];
        if let Some(start) = start_date {
            if !start.is_empty() {
                args.push("--start-date".into());
                args.push(start.into());
            }
        }
        if let Some(due) = due_date {
            if !due.is_empty() {
                args.push("--due-date".into());
                args.push(due.into());
            }
        }
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_glab(&args_refs, "Updating Milestone").await?;
        Ok(())
    }

    async fn delete_milestone(&self, project: &str, milestone_iid: u64) -> Result<()> {
        let encoded = Self::encode_path(project);
        self.run_glab(
            &[
                "milestone",
                "delete",
                &milestone_iid.to_string(),
                "-R",
                &encoded,
                "-y",
            ],
            "Deleting Milestone",
        )
        .await?;
        Ok(())
    }

    // ── Notifications ──

    async fn list_notifications(&self, show_read: bool) -> Result<Vec<Notification>> {
        // glab todo list does active todos; for "done" we use glab api
        let raw = self
            .run_glab(&["todo", "list", "--output=json"], "Fetching Todos")
            .await?;
        #[derive(Deserialize)]
        struct GiTodo {
            id: serde_json::Value,
            project: GiTodoProject,
            target: GiTodoTarget,
            target_type: String,
            state: String,
            updated_at: String,
        }
        #[derive(Deserialize)]
        struct GiTodoProject {
            path_with_namespace: String,
        }
        #[derive(Deserialize)]
        struct GiTodoTarget {
            title: String,
            iid: u64,
        }
        let todos: Vec<GiTodo> = serde_json::from_str(&raw)?;
        let mut list: Vec<Notification> = todos
            .into_iter()
            .map(|item| {
                let id = match item.id {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s,
                    _ => String::new(),
                };
                Notification {
                    id,
                    project_path: item.project.path_with_namespace,
                    title: item.target.title,
                    target_type: item.target_type,
                    target_iid: item.target.iid,
                    state: item.state,
                    updated_at: item.updated_at,
                }
            })
            .collect();
        if show_read {
            let endpoint = "todos?state=done";
            let raw = self
                .raw_api(endpoint, "GET", None, "Fetching Done Todos")
                .await?;
            let done_todos: Vec<GiTodo> = serde_json::from_str(&raw).unwrap_or_default();
            list.extend(done_todos.into_iter().map(|item| {
                let id = match item.id {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s,
                    _ => String::new(),
                };
                Notification {
                    id,
                    project_path: item.project.path_with_namespace,
                    title: item.target.title,
                    target_type: item.target_type,
                    target_iid: item.target.iid,
                    state: item.state,
                    updated_at: item.updated_at,
                }
            }));
        }
        Ok(list)
    }

    async fn mark_notification_as_read(&self, id: &str) -> Result<()> {
        let endpoint = format!("todos/{}/mark_as_done", id);
        self.raw_api(&endpoint, "POST", None, "Marking Todo Done")
            .await?;
        Ok(())
    }

    // ── Branches ──

    async fn list_branches(&self, project: &str, page_size: usize) -> Result<Vec<Branch>> {
        let encoded = Self::encode_path(project);
        let endpoint = format!(
            "/projects/{}/repository/branches?per_page={}",
            encoded, page_size
        );
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Branches")
            .await?;
        #[derive(Deserialize)]
        struct GiBr {
            name: String,
            #[serde(default)]
            default: Option<bool>,
            #[serde(default)]
            protected: Option<bool>,
            #[serde(default)]
            web_url: Option<String>,
            #[serde(default)]
            can_push: Option<bool>,
            commit: Option<GiBrCommit>,
        }
        #[derive(Deserialize)]
        struct GiBrCommit {
            id: String,
        }
        let gl_branches: Vec<GiBr> = serde_json::from_str(&raw)?;
        Ok(gl_branches
            .into_iter()
            .map(|b| Branch {
                name: b.name,
                default: b.default.unwrap_or(false),
                protected: b.protected.unwrap_or(false),
                web_url: b.web_url.unwrap_or_default(),
                can_push: b.can_push.unwrap_or(false),
                commit_sha: b.commit.as_ref().map(|c| c.id.clone()).unwrap_or_default(),
            })
            .collect())
    }

    async fn create_branch(
        &self,
        project: &str,
        branch_name: &str,
        ref_branch: &str,
    ) -> Result<()> {
        let encoded = Self::encode_path(project);
        let endpoint = format!(
            "/projects/{}/repository/branches?branch={}&ref={}",
            encoded, branch_name, ref_branch
        );
        self.raw_api(&endpoint, "POST", None, "Creating Branch")
            .await?;
        Ok(())
    }

    async fn delete_branch(&self, project: &str, branch_name: &str) -> Result<()> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/repository/branches/{}", encoded, branch_name);
        self.raw_api(&endpoint, "DELETE", None, "Deleting Branch")
            .await?;
        Ok(())
    }

    // ── Environments / Deployments ──

    async fn list_environments(&self, project: &str, page_size: usize) -> Result<Vec<Environment>> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/environments?per_page={}", encoded, page_size);
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Environments")
            .await?;
        #[derive(Deserialize)]
        struct GiEnv {
            id: u64,
            name: String,
            state: String,
            #[serde(default)]
            external_url: Option<String>,
            #[serde(default)]
            last_deployment: Option<GiDeploy>,
        }
        #[derive(Deserialize)]
        struct GiDeploy {
            id: u64,
            iid: u64,
            #[serde(rename = "ref")]
            ref_name: String,
            tag: bool,
            sha: String,
            status: String,
            created_at: String,
            updated_at: String,
            #[serde(default)]
            environment: Option<crate::domain::deployments::EnvironmentInfo>,
            #[serde(default)]
            deployable: Option<crate::domain::deployments::Deployable>,
            #[serde(default)]
            description: String,
            #[serde(default)]
            user: Option<crate::domain::deployments::DeploymentUser>,
        }
        let envs: Vec<GiEnv> = serde_json::from_str(&raw)?;
        Ok(envs
            .into_iter()
            .map(|e| Environment {
                id: e.id,
                name: e.name,
                state: e.state,
                external_url: e.external_url,
                last_deployment: e.last_deployment.map(|d| Deployment {
                    id: d.id,
                    iid: d.iid,
                    ref_name: d.ref_name,
                    tag: d.tag,
                    sha: d.sha,
                    status: d.status,
                    created_at: d.created_at,
                    updated_at: d.updated_at,
                    environment: d.environment,
                    deployable: d.deployable,
                    description: d.description,
                    user: d.user,
                }),
            })
            .collect())
    }

    async fn list_deployments(
        &self,
        project: &str,
        page_size: usize,
        environment: Option<&str>,
    ) -> Result<Vec<Deployment>> {
        let encoded = Self::encode_path(project);
        let mut endpoint = format!("/projects/{}/deployments?per_page={}", encoded, page_size);
        if let Some(env) = environment {
            endpoint.push_str(&format!("&environment={}", env));
        }
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Deployments")
            .await?;
        #[derive(Deserialize)]
        struct GiDeploy {
            id: u64,
            iid: u64,
            #[serde(rename = "ref")]
            ref_name: String,
            tag: bool,
            sha: String,
            status: String,
            created_at: String,
            updated_at: String,
            #[serde(default)]
            environment: Option<crate::domain::deployments::EnvironmentInfo>,
            #[serde(default)]
            deployable: Option<crate::domain::deployments::Deployable>,
            #[serde(default)]
            description: String,
            #[serde(default)]
            user: Option<crate::domain::deployments::DeploymentUser>,
        }
        let deploys: Vec<GiDeploy> = serde_json::from_str(&raw)?;
        Ok(deploys
            .into_iter()
            .map(|d| Deployment {
                id: d.id,
                iid: d.iid,
                ref_name: d.ref_name,
                tag: d.tag,
                sha: d.sha,
                status: d.status,
                created_at: d.created_at,
                updated_at: d.updated_at,
                environment: d.environment,
                deployable: d.deployable,
                description: d.description,
                user: d.user,
            })
            .collect())
    }

    // ── Labels / Members / Misc ──

    async fn fetch_labels(&self, project: &str) -> Result<Vec<String>> {
        let encoded = Self::encode_path(project);
        let raw = self
            .run_glab(
                &[
                    "label",
                    "list",
                    "--output",
                    "json",
                    "-R",
                    &encoded,
                    "--per-page",
                    "100",
                ],
                "Fetching Labels",
            )
            .await?;
        #[derive(Deserialize)]
        struct GiLabel {
            name: String,
        }
        let labels: Vec<GiLabel> = serde_json::from_str(&raw)?;
        Ok(labels.into_iter().map(|l| l.name).collect())
    }

    async fn fetch_members(&self, project: &str) -> Result<Vec<String>> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/members/all?per_page=100", encoded);
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Members")
            .await?;
        #[derive(Deserialize)]
        struct GiMember {
            username: String,
        }
        let members: Vec<GiMember> = serde_json::from_str(&raw)?;
        Ok(members
            .into_iter()
            .map(|m| format!("@{}", m.username))
            .collect())
    }

    async fn fetch_branch_names(&self, project: &str) -> Result<Vec<String>> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/repository/branches?per_page=100", encoded);
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Branch Names")
            .await?;
        #[derive(Deserialize)]
        struct GiBr {
            name: String,
        }
        let branches: Vec<GiBr> = serde_json::from_str(&raw)?;
        Ok(branches.into_iter().map(|b| b.name).collect())
    }

    async fn fetch_milestone_titles(&self, project: &str) -> Result<Vec<String>> {
        let encoded = Self::encode_path(project);
        let endpoint = format!("/projects/{}/milestones?state=active&per_page=100", encoded);
        let raw = self
            .raw_api(&endpoint, "GET", None, "Fetching Milestone Titles")
            .await?;
        #[derive(Deserialize)]
        struct GiMs {
            title: String,
        }
        let milestones: Vec<GiMs> = serde_json::from_str(&raw)?;
        Ok(milestones.into_iter().map(|m| m.title).collect())
    }

    // ── Raw API ──

    async fn raw_api(
        &self,
        endpoint: &str,
        method: &str,
        body: Option<&str>,
        desc: &str,
    ) -> Result<String> {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        let mut cmd_args: Vec<String> = vec!["api".into()];
        if method != "GET" {
            cmd_args.push("-X".into());
            cmd_args.push(method.into());
        }
        cmd_args.push(endpoint.into());
        let cmd_str = format!("glab {}", cmd_args.join(" "));
        let label = format!("{:<24}", desc.to_uppercase());
        if let Some(ref tx) = self.tx {
            let _ = tx.send(Event::TerminalCommandLogged {
                timestamp: timestamp.clone(),
                command: format!("{} {}", label, cmd_str),
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
                        let _ = tx.send(Event::TerminalCommandLogged {
                            timestamp,
                            command: format!("{} {}", label, cmd_str),
                            status: "Success".to_string(),
                        });
                    }
                    Ok(s)
                } else {
                    let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(Event::TerminalCommandLogged {
                            timestamp,
                            command: format!("{} {}", label, cmd_str),
                            status: format!("Failed: {}", err_msg),
                        });
                    }
                    anyhow::bail!("glab api failed: {}", err_msg)
                }
            }
            Err(e) => {
                let err_msg = format!("{}", e);
                if let Some(ref tx) = self.tx {
                    let _ = tx.send(Event::TerminalCommandLogged {
                        timestamp,
                        command: format!("{} {}", label, cmd_str),
                        status: format!("Failed: {}", err_msg),
                    });
                }
                Err(e.into())
            }
        }
    }
}
