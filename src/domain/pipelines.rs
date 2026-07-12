use crate::domain::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

// --- GitLab native types ---

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabPipeline {
    pub id: u64,
    pub status: String,
    pub r#ref: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabJob {
    pub id: u64,
    pub status: String,
    pub stage: String,
    /// Full name as returned by the API (may include matrix suffix like "[ubuntu, run:test]").
    pub name: String,
    /// The parsed matrix variant string, e.g. "ubuntu, run:test", if the job is a matrix job.
    #[serde(skip)]
    pub matrix: Option<String>,
}

// --- GitHub native types ---

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubWorkflowRun {
    pub id: u64,
    /// Workflow name (e.g. "CI", "Lint")
    pub name: String,
    /// Human-readable title (usually the commit message)
    pub display_title: String,
    /// Raw API status: "completed", "in_progress", "queued", "waiting", etc.
    pub status: String,
    pub conclusion: Option<String>,
    pub head_branch: String,
    /// Trigger event: "push", "pull_request", "schedule", etc.
    pub event: String,
    pub run_number: u64,
    pub head_sha: String,
    pub created_at: String,
    pub updated_at: String,
    pub workflow_id: u64,
    /// Workflow file path, e.g. ".github/workflows/ci.yml"
    pub path: String,
    /// Login of the actor who triggered the run
    #[serde(default)]
    pub actor_login: Option<String>,
}

/// Helper: deserialize the `actor` object to extract login.
#[derive(Debug, Deserialize)]
struct GithubActor {
    login: String,
}

/// Wrapper for /repos/{owner}/{repo}/actions/runs response.
#[derive(Debug, Deserialize)]
pub struct GithubWorkflowRuns {
    /// Deserialized as `Vec<serde_json::Value>` first so we can hand-pick fields.
    #[serde(rename = "workflow_runs")]
    workflow_runs: Vec<serde_json::Value>,
}

impl GithubWorkflowRuns {
    pub fn into_workflow_runs(self) -> Vec<GithubWorkflowRun> {
        self.workflow_runs
            .into_iter()
            .filter_map(|v| {
                let id = v.get("id")?.as_u64()?;
                let name = v
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let display_title = v
                    .get("display_title")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let status = v
                    .get("status")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let conclusion = v
                    .get("conclusion")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string());
                let head_branch = v
                    .get("head_branch")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let event = v
                    .get("event")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let run_number = v.get("run_number").and_then(|n| n.as_u64()).unwrap_or(0);
                let head_sha = v
                    .get("head_sha")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let created_at = v
                    .get("created_at")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let updated_at = v
                    .get("updated_at")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let workflow_id = v.get("workflow_id").and_then(|n| n.as_u64()).unwrap_or(0);
                let path = v
                    .get("path")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let actor_login = v
                    .get("actor")
                    .and_then(|a| a.get("login"))
                    .and_then(|l| l.as_str())
                    .map(|s| s.to_string());
                Some(GithubWorkflowRun {
                    id,
                    name,
                    display_title,
                    status,
                    conclusion,
                    head_branch,
                    event,
                    run_number,
                    head_sha,
                    created_at,
                    updated_at,
                    workflow_id,
                    path,
                    actor_login,
                })
            })
            .collect()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubActionJob {
    pub id: u64,
    pub status: String,
    pub conclusion: Option<String>,
    pub name: String,
    #[serde(default)]
    pub workflow_name: Option<String>,
    #[serde(default)]
    pub runner_name: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    /// Parsed matrix variant, e.g. "ubuntu-latest, 20". Not from API.
    #[serde(skip)]
    pub matrix: Option<String>,
}

/// Wrapper for /repos/{owner}/{repo}/actions/runs/{id}/jobs response.
#[derive(Debug, Deserialize)]
pub struct GithubWorkflowJobs {
    pub jobs: Vec<serde_json::Value>,
}

impl GithubWorkflowJobs {
    pub fn into_jobs(self) -> Vec<GithubActionJob> {
        self.jobs
            .into_iter()
            .filter_map(|v| {
                let id = v.get("id")?.as_u64()?;
                let status = v
                    .get("status")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let conclusion = v
                    .get("conclusion")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string());
                let name = v
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let workflow_name = v
                    .get("workflow_name")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string());
                let runner_name = v
                    .get("runner_name")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string());
                let started_at = v
                    .get("started_at")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string());
                let completed_at = v
                    .get("completed_at")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string());
                Some(GithubActionJob {
                    id,
                    status,
                    conclusion,
                    name,
                    workflow_name,
                    runner_name,
                    started_at,
                    completed_at,
                    matrix: None,
                })
            })
            .collect()
    }
}

// --- PipelineItem / JobItem enums (phase out over time) ---

/// Unified enum holding either a GitLab pipeline or GitHub workflow run.
/// The GitHub variant stores a pre-computed `effective_status` (normalized
/// GitLab-style status) so filtering/searching/grouping does not need to
/// recompute it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PipelineItem {
    Gitlab(GitlabPipeline),
    Github {
        run: GithubWorkflowRun,
        effective_status: String,
    },
    /// Legacy cache compat – old caches stored a `Pipeline` struct.
    /// Deserialized gracefully but should never appear in running code.
    #[serde(other)]
    Unknown,
}

/// Unified enum holding either a GitLab job or GitHub Actions job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobItem {
    Gitlab(GitlabJob),
    Github {
        job: GithubActionJob,
        effective_status: String,
    },
    #[serde(other)]
    Unknown,
}

// --- Accessor helpers ---

fn normalize_github_status(status: &str, conclusion: Option<&str>) -> String {
    if status == "completed" {
        match conclusion {
            Some("success") => "success",
            Some("failure") => "failed",
            Some("cancelled") => "canceled",
            Some("skipped") => "skipped",
            _ => "failed",
        }
    } else if status == "in_progress" {
        "running"
    } else if status == "queued" || status == "waiting" {
        "pending"
    } else {
        "pending"
    }
    .to_string()
}

impl PipelineItem {
    pub fn from_gitlab(p: GitlabPipeline) -> Self {
        PipelineItem::Gitlab(p)
    }

    pub fn from_github(run: GithubWorkflowRun) -> Self {
        let effective_status = normalize_github_status(&run.status, run.conclusion.as_deref());
        PipelineItem::Github {
            run,
            effective_status,
        }
    }

    pub fn id(&self) -> u64 {
        match self {
            PipelineItem::Gitlab(p) => p.id,
            PipelineItem::Github { run, .. } => run.id,
            PipelineItem::Unknown => 0,
        }
    }

    pub fn status(&self) -> &str {
        match self {
            PipelineItem::Gitlab(p) => &p.status,
            PipelineItem::Github {
                effective_status, ..
            } => effective_status,
            PipelineItem::Unknown => "unknown",
        }
    }

    pub fn ref_branch(&self) -> &str {
        match self {
            PipelineItem::Gitlab(p) => &p.r#ref,
            PipelineItem::Github { run, .. } => &run.head_branch,
            PipelineItem::Unknown => "",
        }
    }

    pub fn updated_at(&self) -> &str {
        match self {
            PipelineItem::Gitlab(p) => &p.updated_at,
            PipelineItem::Github { run, .. } => &run.updated_at,
            PipelineItem::Unknown => "",
        }
    }
}

impl JobItem {
    pub fn from_gitlab(j: GitlabJob) -> Self {
        JobItem::Gitlab(j)
    }

    pub fn from_github(job: GithubActionJob) -> Self {
        let effective_status = normalize_github_status(&job.status, job.conclusion.as_deref());
        JobItem::Github {
            job,
            effective_status,
        }
    }

    pub fn id(&self) -> u64 {
        match self {
            JobItem::Gitlab(j) => j.id,
            JobItem::Github { job, .. } => job.id,
            JobItem::Unknown => 0,
        }
    }

    pub fn status(&self) -> &str {
        match self {
            JobItem::Gitlab(j) => &j.status,
            JobItem::Github {
                effective_status, ..
            } => effective_status,
            JobItem::Unknown => "unknown",
        }
    }

    pub fn name(&self) -> &str {
        match self {
            JobItem::Gitlab(j) => &j.name,
            JobItem::Github { job, .. } => &job.name,
            JobItem::Unknown => "",
        }
    }

    pub fn stage(&self) -> &str {
        match self {
            JobItem::Gitlab(j) => &j.stage,
            JobItem::Github { .. } => "build",
            JobItem::Unknown => "",
        }
    }

    pub fn matrix(&self) -> Option<&str> {
        match self {
            JobItem::Gitlab(j) => j.matrix.as_deref(),
            JobItem::Github { job, .. } => job.matrix.as_deref(),
            JobItem::Unknown => None,
        }
    }

    pub fn set_matrix(&mut self, matrix: Option<String>) {
        match self {
            JobItem::Gitlab(j) => j.matrix = matrix,
            JobItem::Github { job, .. } => job.matrix = matrix,
            JobItem::Unknown => {}
        }
    }

    pub fn set_name(&mut self, name: String) {
        match self {
            JobItem::Gitlab(j) => j.name = name,
            JobItem::Github { job, .. } => job.name = name,
            JobItem::Unknown => {}
        }
    }
}

// --- API functions ---

pub async fn list_pipelines(
    client: &GitlabClient,
    project_path: &str,
) -> Result<Vec<PipelineItem>> {
    if client.is_github {
        let endpoint = format!(
            "/repos/{}/actions/runs?per_page={}",
            project_path, client.page_size
        );
        let raw = client.execute_github_api(&endpoint, "GET", None).await?;
        let runs: GithubWorkflowRuns = serde_json::from_str(&raw)?;
        Ok(runs
            .into_workflow_runs()
            .into_iter()
            .map(PipelineItem::from_github)
            .collect())
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!(
            "/projects/{}/pipelines?per_page={}",
            encoded_path, client.page_size
        );
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_pipes: Vec<GitlabPipeline> = serde_json::from_str(&raw)?;
        Ok(gl_pipes
            .into_iter()
            .map(PipelineItem::from_gitlab)
            .collect())
    }
}

pub async fn list_pipeline_jobs(
    client: &GitlabClient,
    project_path: &str,
    pipeline_id: u64,
) -> Result<Vec<JobItem>> {
    if client.is_github {
        let endpoint_page1 = format!(
            "/repos/{}/actions/runs/{}/jobs?per_page={}&page=1",
            project_path, pipeline_id, client.page_size
        );
        let raw = client
            .execute_github_api(&endpoint_page1, "GET", None)
            .await?;
        let gh_jobs_res: GithubWorkflowJobs = serde_json::from_str(&raw)?;
        let mut all_jobs: Vec<JobItem> = gh_jobs_res
            .into_jobs()
            .into_iter()
            .map(JobItem::from_github)
            .collect();

        if all_jobs.len() == client.page_size {
            let mut handles = Vec::new();
            let limit = client.page_limit.unwrap_or(10) as usize;
            for page in 2..=limit {
                let endpoint = format!(
                    "/repos/{}/actions/runs/{}/jobs?per_page={}&page={}",
                    project_path, pipeline_id, client.page_size, page
                );
                let client_clone = client.clone();
                handles.push(tokio::spawn(async move {
                    if let Ok(raw) = client_clone
                        .execute_github_api(&endpoint, "GET", None)
                        .await
                    {
                        if let Ok(res) = serde_json::from_str::<GithubWorkflowJobs>(&raw) {
                            return res
                                .into_jobs()
                                .into_iter()
                                .map(JobItem::from_github)
                                .collect::<Vec<JobItem>>();
                        }
                    }
                    vec![]
                }));
            }

            for handle in handles {
                if let Ok(jobs) = handle.await {
                    if jobs.is_empty() {
                        break;
                    }
                    let jobs_len = jobs.len();
                    all_jobs.extend(jobs);
                    if jobs_len < client.page_size {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
        Ok(process_pipeline_jobs(all_jobs))
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint_page1 = format!(
            "/projects/{}/pipelines/{}/jobs?per_page={}&page=1",
            encoded_path, pipeline_id, client.page_size
        );
        let raw = client
            .execute_gitlab_api(&endpoint_page1, "GET", None)
            .await?;
        let gl_jobs: Vec<GitlabJob> = serde_json::from_str(&raw)?;
        let mut all_jobs: Vec<JobItem> = gl_jobs.into_iter().map(JobItem::from_gitlab).collect();

        if all_jobs.len() == client.page_size {
            let mut handles = Vec::new();
            let limit = client.page_limit.unwrap_or(10) as usize;
            for page in 2..=limit {
                let endpoint = format!(
                    "/projects/{}/pipelines/{}/jobs?per_page={}&page={}",
                    encoded_path, pipeline_id, client.page_size, page
                );
                let client_clone = client.clone();
                handles.push(tokio::spawn(async move {
                    if let Ok(raw) = client_clone
                        .execute_gitlab_api(&endpoint, "GET", None)
                        .await
                    {
                        if let Ok(res) = serde_json::from_str::<Vec<GitlabJob>>(&raw) {
                            return res
                                .into_iter()
                                .map(JobItem::from_gitlab)
                                .collect::<Vec<JobItem>>();
                        }
                    }
                    vec![]
                }));
            }

            for handle in handles {
                if let Ok(jobs) = handle.await {
                    if jobs.is_empty() {
                        break;
                    }
                    let jobs_len = jobs.len();
                    all_jobs.extend(jobs);
                    if jobs_len < client.page_size {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
        Ok(process_pipeline_jobs(all_jobs))
    }
}

pub fn process_pipeline_jobs(all_jobs: Vec<JobItem>) -> Vec<JobItem> {
    let all_jobs: Vec<JobItem> = all_jobs
        .into_iter()
        .map(|mut job_item| {
            let name = job_item.name().to_string();
            if let (Some(bracket_start), Some(bracket_end)) = (name.rfind('['), name.rfind(']')) {
                if bracket_end == name.len() - 1 {
                    let matrix_content = name[bracket_start + 1..bracket_end].trim().to_string();
                    let base_name = name[..bracket_start].trim().to_string();
                    job_item.set_name(base_name);
                    job_item.set_matrix(Some(matrix_content));
                }
            } else if let (Some(paren_start), Some(paren_end)) = (name.rfind('('), name.rfind(')'))
            {
                if paren_end == name.len() - 1 {
                    let matrix_content = name[paren_start + 1..paren_end].trim().to_string();
                    let base_name = name[..paren_start].trim().to_string();
                    job_item.set_name(base_name);
                    job_item.set_matrix(Some(matrix_content));
                }
            }
            job_item
        })
        .collect();

    let mut stage_min_id = std::collections::HashMap::new();
    for j in all_jobs.iter() {
        let stage = j.stage().to_string();
        let entry = stage_min_id.entry(stage).or_insert(j.id());
        if j.id() < *entry {
            *entry = j.id();
        }
    }

    let mut deduplicated: std::collections::HashMap<(String, Option<String>), JobItem> =
        std::collections::HashMap::new();
    for job in all_jobs {
        let key = (job.name().to_string(), job.matrix().map(|m| m.to_string()));
        let entry = deduplicated.entry(key).or_insert_with(|| job.clone());
        if job.id() > entry.id() {
            *entry = job;
        }
    }
    let mut all_jobs: Vec<JobItem> = deduplicated.into_values().collect();

    all_jobs.sort_by(|a, b| {
        let min_a = stage_min_id.get(a.stage()).cloned().unwrap_or(0);
        let min_b = stage_min_id.get(b.stage()).cloned().unwrap_or(0);
        if min_a != min_b {
            min_a.cmp(&min_b)
        } else if a.stage() != b.stage() {
            a.stage().cmp(b.stage())
        } else {
            a.id().cmp(&b.id())
        }
    });

    all_jobs
}

pub async fn get_job_trace(
    client: &GitlabClient,
    project_path: &str,
    job_id: u64,
) -> Result<String> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!("/projects/{}/jobs/{}/trace", encoded_path, job_id);
    client.fetch_raw_api(&endpoint).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_github_status() {
        assert_eq!(
            normalize_github_status("completed", Some("success")),
            "success"
        );
        assert_eq!(
            normalize_github_status("completed", Some("failure")),
            "failed"
        );
        assert_eq!(
            normalize_github_status("completed", Some("cancelled")),
            "canceled"
        );
        assert_eq!(
            normalize_github_status("completed", Some("skipped")),
            "skipped"
        );
        assert_eq!(normalize_github_status("in_progress", None), "running");
        assert_eq!(normalize_github_status("queued", None), "pending");
        assert_eq!(normalize_github_status("waiting", None), "pending");
    }

    #[test]
    fn test_pipeline_item_accessors() {
        let gl = PipelineItem::from_gitlab(GitlabPipeline {
            id: 1,
            status: "success".into(),
            r#ref: "main".into(),
            updated_at: "2024-01-01".into(),
        });
        assert_eq!(gl.id(), 1);
        assert_eq!(gl.status(), "success");
        assert_eq!(gl.ref_branch(), "main");
        assert_eq!(gl.updated_at(), "2024-01-01");

        let gh = PipelineItem::from_github(GithubWorkflowRun {
            id: 2,
            name: "CI".into(),
            display_title: "Fix bug".into(),
            status: "completed".into(),
            conclusion: Some("success".into()),
            head_branch: "feat/x".into(),
            event: "push".into(),
            run_number: 42,
            head_sha: "abc123".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-02".into(),
            workflow_id: 100,
            path: ".github/workflows/ci.yml".into(),
            actor_login: Some("octocat".into()),
        });
        assert_eq!(gh.id(), 2);
        assert_eq!(gh.status(), "success");
        assert_eq!(gh.ref_branch(), "feat/x");
        assert_eq!(gh.updated_at(), "2024-01-02");
    }

    #[test]
    fn test_job_item_accessors() {
        let gl = JobItem::from_gitlab(GitlabJob {
            id: 1,
            status: "success".into(),
            stage: "test".into(),
            name: "unit".into(),
            matrix: None,
        });
        assert_eq!(gl.id(), 1);
        assert_eq!(gl.status(), "success");
        assert_eq!(gl.stage(), "test");
        assert_eq!(gl.name(), "unit");

        let gh = JobItem::from_github(GithubActionJob {
            id: 2,
            status: "completed".into(),
            conclusion: Some("failure".into()),
            name: "lint".into(),
            workflow_name: Some("CI".into()),
            runner_name: None,
            started_at: None,
            completed_at: None,
            matrix: None,
        });
        assert_eq!(gh.id(), 2);
        assert_eq!(gh.status(), "failed");
        assert_eq!(gh.stage(), "build");
        assert_eq!(gh.name(), "lint");
    }

    #[test]
    fn test_process_pipeline_jobs() {
        let input_jobs = vec![
            JobItem::from_gitlab(GitlabJob {
                id: 101,
                status: "success".into(),
                stage: "build".into(),
                name: "compile-code".into(),
                matrix: None,
            }),
            JobItem::from_gitlab(GitlabJob {
                id: 102,
                status: "failed".into(),
                stage: "test".into(),
                name: "run-tests".into(),
                matrix: None,
            }),
            JobItem::from_gitlab(GitlabJob {
                id: 103,
                status: "success".into(),
                stage: "test".into(),
                name: "run-tests".into(),
                matrix: None,
            }),
            JobItem::from_gitlab(GitlabJob {
                id: 104,
                status: "running".into(),
                stage: "build".into(),
                name: "compile-code".into(),
                matrix: None,
            }),
        ];

        let processed = process_pipeline_jobs(input_jobs);

        assert_eq!(processed.len(), 2);
        let build_job = processed
            .iter()
            .find(|j| j.name() == "compile-code")
            .unwrap();
        assert_eq!(build_job.id(), 104);
        assert_eq!(build_job.status(), "running");

        let test_job = processed.iter().find(|j| j.name() == "run-tests").unwrap();
        assert_eq!(test_job.id(), 103);
        assert_eq!(test_job.status(), "success");

        assert_eq!(processed[0].stage(), "build");
        assert_eq!(processed[1].stage(), "test");
    }

    #[test]
    fn test_process_pipeline_jobs_matrix_parsing() {
        let input_jobs = vec![
            JobItem::from_gitlab(GitlabJob {
                id: 201,
                status: "success".into(),
                stage: "test".into(),
                name: "run-tests [ubuntu, unit]".into(),
                matrix: None,
            }),
            JobItem::from_gitlab(GitlabJob {
                id: 202,
                status: "failed".into(),
                stage: "test".into(),
                name: "run-tests [windows, integration]".into(),
                matrix: None,
            }),
            JobItem::from_gitlab(GitlabJob {
                id: 203,
                status: "running".into(),
                stage: "test".into(),
                name: "run-tests [ubuntu, unit]".into(),
                matrix: None,
            }),
            JobItem::from_gitlab(GitlabJob {
                id: 204,
                status: "success".into(),
                stage: "test".into(),
                name: "lint".into(),
                matrix: None,
            }),
        ];

        let processed = process_pipeline_jobs(input_jobs);
        assert_eq!(processed.len(), 3);

        let ubuntu = processed
            .iter()
            .find(|j| j.matrix() == Some("ubuntu, unit"))
            .unwrap();
        assert_eq!(ubuntu.name(), "run-tests");
        assert_eq!(ubuntu.id(), 203);

        let windows = processed
            .iter()
            .find(|j| j.matrix() == Some("windows, integration"))
            .unwrap();
        assert_eq!(windows.name(), "run-tests");
        assert_eq!(windows.id(), 202);
    }

    #[test]
    fn test_process_pipeline_jobs_github_matrix_parsing() {
        let input_jobs = vec![JobItem::from_github(GithubActionJob {
            id: 301,
            status: "completed".into(),
            conclusion: Some("success".into()),
            name: "test-matrix (ubuntu-latest, 20)".into(),
            workflow_name: None,
            runner_name: None,
            started_at: None,
            completed_at: None,
            matrix: None,
        })];
        let processed = process_pipeline_jobs(input_jobs);
        assert_eq!(processed.len(), 1);
        assert_eq!(processed[0].name(), "test-matrix");
        assert_eq!(processed[0].matrix(), Some("ubuntu-latest, 20"));
    }

    #[test]
    fn test_deserialize_github_workflow_runs() {
        let json_data = r#"{
            "total_count": 2,
            "workflow_runs": [
                {
                    "id": 1,
                    "name": "CI",
                    "display_title": "Fix bug",
                    "status": "completed",
                    "conclusion": "success",
                    "head_branch": "main",
                    "event": "push",
                    "run_number": 100,
                    "head_sha": "abc123",
                    "created_at": "2024-01-01T00:00:00Z",
                    "updated_at": "2024-01-01T01:00:00Z",
                    "workflow_id": 50,
                    "path": ".github/workflows/ci.yml",
                    "actor": {"login": "octocat"}
                }
            ]
        }"#;
        let runs: GithubWorkflowRuns = serde_json::from_str(json_data).unwrap();
        let parsed = runs.into_workflow_runs();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "CI");
        assert_eq!(parsed[0].display_title, "Fix bug");
        assert_eq!(parsed[0].event, "push");
        assert_eq!(parsed[0].run_number, 100);
        assert_eq!(parsed[0].head_sha, "abc123");
        assert_eq!(parsed[0].actor_login.as_deref(), Some("octocat"));
    }

    #[test]
    fn test_deserialize_github_workflow_jobs() {
        let json_data = r#"{
            "jobs": [
                {
                    "id": 1,
                    "status": "completed",
                    "conclusion": "success",
                    "name": "build",
                    "workflow_name": "CI",
                    "runner_name": "ubuntu-latest",
                    "started_at": "2024-01-01T00:00:00Z",
                    "completed_at": "2024-01-01T00:05:00Z"
                }
            ]
        }"#;
        let jobs_response: GithubWorkflowJobs = serde_json::from_str(json_data).unwrap();
        let parsed = jobs_response.into_jobs();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "build");
        assert_eq!(parsed[0].workflow_name.as_deref(), Some("CI"));
        assert_eq!(parsed[0].runner_name.as_deref(), Some("ubuntu-latest"));
    }
}
