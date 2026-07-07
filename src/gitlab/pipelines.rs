use super::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Pipeline {
    pub id: u64,
    pub status: String,
    pub r#ref: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabPipeline {
    pub id: u64,
    pub status: String,
    pub r#ref: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubWorkflowRun {
    pub id: u64,
    pub status: String,
    pub conclusion: Option<String>,
    pub head_branch: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubWorkflowRuns {
    pub workflow_runs: Vec<GithubWorkflowRun>,
}

impl From<GitlabPipeline> for Pipeline {
    fn from(gl: GitlabPipeline) -> Self {
        Self {
            id: gl.id,
            status: gl.status,
            r#ref: gl.r#ref,
            updated_at: gl.updated_at,
        }
    }
}

impl From<GithubWorkflowRun> for Pipeline {
    fn from(gh: GithubWorkflowRun) -> Self {
        let status = if gh.status == "completed" {
            match gh.conclusion.as_deref() {
                Some("success") => "success",
                Some("failure") => "failed",
                Some("cancelled") => "canceled",
                Some("skipped") => "skipped",
                _ => "failed",
            }
        } else if gh.status == "in_progress" {
            "running"
        } else {
            "pending"
        }
        .to_string();

        Self {
            id: gh.id,
            status,
            r#ref: gh.head_branch,
            updated_at: gh.updated_at,
        }
    }
}

pub async fn list_pipelines(client: &GitlabClient, project_path: &str) -> Result<Vec<Pipeline>> {
    if client.is_github {
        let endpoint = format!("/repos/{}/actions/runs?per_page=100", project_path);
        let raw = client.execute_github_api(&endpoint, "GET", None).await?;
        let runs: GithubWorkflowRuns = serde_json::from_str(&raw)?;
        Ok(runs.workflow_runs.into_iter().map(Pipeline::from).collect())
    } else {
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!("/projects/{}/pipelines?per_page=100", encoded_path);
        let raw = client.execute_gitlab_api(&endpoint, "GET", None).await?;
        let gl_pipes: Vec<GitlabPipeline> = serde_json::from_str(&raw)?;
        Ok(gl_pipes.into_iter().map(Pipeline::from).collect())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Job {
    pub id: u64,
    pub status: String,
    pub stage: String,
    /// Full name as returned by the API (may include matrix suffix like "[ubuntu, run:test]").
    pub name: String,
    /// The parsed matrix variant string, e.g. "ubuntu, run:test", if the job is a matrix job.
    #[serde(skip)]
    pub matrix: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabJob {
    pub id: u64,
    pub status: String,
    pub stage: String,
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubActionJob {
    pub id: u64,
    pub status: String,
    pub conclusion: Option<String>,
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubWorkflowJobs {
    pub jobs: Vec<GithubActionJob>,
}

impl From<GitlabJob> for Job {
    fn from(gl: GitlabJob) -> Self {
        Self {
            id: gl.id,
            status: gl.status,
            stage: gl.stage,
            name: gl.name,
            matrix: None,
        }
    }
}

impl From<GithubActionJob> for Job {
    fn from(gh: GithubActionJob) -> Self {
        let status = if gh.status == "completed" {
            match gh.conclusion.as_deref() {
                Some("success") => "success",
                Some("failure") => "failed",
                Some("cancelled") => "canceled",
                Some("skipped") => "skipped",
                _ => "failed",
            }
        } else if gh.status == "in_progress" {
            "running"
        } else {
            "pending"
        }
        .to_string();

        Self {
            id: gh.id,
            status,
            stage: "build".to_string(),
            name: gh.name,
            matrix: None,
        }
    }
}

pub async fn list_pipeline_jobs(
    client: &GitlabClient,
    project_path: &str,
    pipeline_id: u64,
) -> Result<Vec<Job>> {
    if client.is_github {
        let endpoint_page1 = format!(
            "/repos/{}/actions/runs/{}/jobs?per_page=100&page=1",
            project_path, pipeline_id
        );
        let raw = client
            .execute_github_api(&endpoint_page1, "GET", None)
            .await?;
        let gh_jobs_res: GithubWorkflowJobs = serde_json::from_str(&raw)?;
        let mut all_jobs: Vec<Job> = gh_jobs_res.jobs.into_iter().map(Job::from).collect();

        if all_jobs.len() == 100 {
            let mut handles = Vec::new();
            for page in 2..=10 {
                let endpoint = format!(
                    "/repos/{}/actions/runs/{}/jobs?per_page=100&page={}",
                    project_path, pipeline_id, page
                );
                let client_clone = client.clone();
                handles.push(tokio::spawn(async move {
                    if let Ok(raw) = client_clone
                        .execute_github_api(&endpoint, "GET", None)
                        .await
                    {
                        if let Ok(res) = serde_json::from_str::<GithubWorkflowJobs>(&raw) {
                            return res.jobs.into_iter().map(Job::from).collect::<Vec<Job>>();
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
                    if jobs_len < 100 {
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
            "/projects/{}/pipelines/{}/jobs?per_page=100&page=1",
            encoded_path, pipeline_id
        );
        let raw = client
            .execute_gitlab_api(&endpoint_page1, "GET", None)
            .await?;
        let gl_jobs: Vec<GitlabJob> = serde_json::from_str(&raw)?;
        let mut all_jobs: Vec<Job> = gl_jobs.into_iter().map(Job::from).collect();

        if all_jobs.len() == 100 {
            let mut handles = Vec::new();
            for page in 2..=10 {
                let endpoint = format!(
                    "/projects/{}/pipelines/{}/jobs?per_page=100&page={}",
                    encoded_path, pipeline_id, page
                );
                let client_clone = client.clone();
                handles.push(tokio::spawn(async move {
                    if let Ok(raw) = client_clone
                        .execute_gitlab_api(&endpoint, "GET", None)
                        .await
                    {
                        if let Ok(res) = serde_json::from_str::<Vec<GitlabJob>>(&raw) {
                            return res.into_iter().map(Job::from).collect::<Vec<Job>>();
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
                    if jobs_len < 100 {
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

pub fn process_pipeline_jobs(all_jobs: Vec<Job>) -> Vec<Job> {
    let all_jobs: Vec<Job> = all_jobs
        .into_iter()
        .map(|mut job| {
            if let (Some(bracket_start), Some(bracket_end)) =
                (job.name.rfind('['), job.name.rfind(']'))
            {
                if bracket_end == job.name.len() - 1 {
                    let matrix_content =
                        job.name[bracket_start + 1..bracket_end].trim().to_string();
                    let base_name = job.name[..bracket_start].trim().to_string();
                    job.matrix = Some(matrix_content);
                    job.name = base_name;
                }
            } else if let (Some(paren_start), Some(paren_end)) =
                (job.name.rfind('('), job.name.rfind(')'))
            {
                if paren_end == job.name.len() - 1 {
                    let matrix_content = job.name[paren_start + 1..paren_end].trim().to_string();
                    let base_name = job.name[..paren_start].trim().to_string();
                    job.matrix = Some(matrix_content);
                    job.name = base_name;
                }
            }
            job
        })
        .collect();

    let mut stage_min_id = std::collections::HashMap::new();
    for j in all_jobs.iter() {
        let entry = stage_min_id.entry(j.stage.clone()).or_insert(j.id);
        if j.id < *entry {
            *entry = j.id;
        }
    }

    let mut deduplicated: std::collections::HashMap<(String, Option<String>), Job> =
        std::collections::HashMap::new();
    for job in all_jobs {
        let key = (job.name.clone(), job.matrix.clone());
        let entry = deduplicated.entry(key).or_insert_with(|| job.clone());
        if job.id > entry.id {
            *entry = job;
        }
    }
    let mut all_jobs: Vec<Job> = deduplicated.into_values().collect();

    all_jobs.sort_by(|a, b| {
        let min_a = stage_min_id.get(&a.stage).cloned().unwrap_or(0);
        let min_b = stage_min_id.get(&b.stage).cloned().unwrap_or(0);
        if min_a != min_b {
            min_a.cmp(&min_b)
        } else if a.stage != b.stage {
            a.stage.cmp(&b.stage)
        } else {
            a.id.cmp(&b.id)
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
    fn test_process_pipeline_jobs() {
        let input_jobs = vec![
            Job {
                id: 101,
                status: "success".to_string(),
                stage: "build".to_string(),
                name: "compile-code".to_string(),
                matrix: None,
            },
            Job {
                id: 102,
                status: "failed".to_string(),
                stage: "test".to_string(),
                name: "run-tests".to_string(),
                matrix: None,
            },
            Job {
                id: 103,
                status: "success".to_string(),
                stage: "test".to_string(),
                name: "run-tests".to_string(),
                matrix: None,
            },
            Job {
                id: 104,
                status: "running".to_string(),
                stage: "build".to_string(),
                name: "compile-code".to_string(),
                matrix: None,
            },
        ];

        let processed = process_pipeline_jobs(input_jobs);

        assert_eq!(processed.len(), 2);

        let build_job = processed.iter().find(|j| j.name == "compile-code").unwrap();
        assert_eq!(build_job.id, 104);
        assert_eq!(build_job.status, "running");

        let test_job = processed.iter().find(|j| j.name == "run-tests").unwrap();
        assert_eq!(test_job.id, 103);
        assert_eq!(test_job.status, "success");

        assert_eq!(processed[0].stage, "build");
        assert_eq!(processed[1].stage, "test");
    }

    #[test]
    fn test_process_pipeline_jobs_matrix_parsing() {
        let input_jobs = vec![
            Job {
                id: 201,
                status: "success".to_string(),
                stage: "test".to_string(),
                name: "run-tests [ubuntu, unit]".to_string(),
                matrix: None,
            },
            Job {
                id: 202,
                status: "failed".to_string(),
                stage: "test".to_string(),
                name: "run-tests [windows, integration]".to_string(),
                matrix: None,
            },
            Job {
                id: 203,
                status: "running".to_string(),
                stage: "test".to_string(),
                name: "run-tests [ubuntu, unit]".to_string(),
                matrix: None,
            },
            Job {
                id: 204,
                status: "success".to_string(),
                stage: "test".to_string(),
                name: "lint".to_string(),
                matrix: None,
            },
        ];

        let processed = process_pipeline_jobs(input_jobs);

        assert_eq!(processed.len(), 3);

        let ubuntu = processed
            .iter()
            .find(|j| j.matrix.as_deref() == Some("ubuntu, unit"))
            .unwrap();
        assert_eq!(ubuntu.name, "run-tests");
        assert_eq!(ubuntu.id, 203);

        let windows = processed
            .iter()
            .find(|j| j.matrix.as_deref() == Some("windows, integration"))
            .unwrap();
        assert_eq!(windows.name, "run-tests");
        assert_eq!(windows.id, 202);

        let lint = processed.iter().find(|j| j.name == "lint").unwrap();
        assert!(lint.matrix.is_none());
    }

    #[test]
    fn test_process_pipeline_jobs_github_matrix_parsing() {
        let input_jobs = vec![Job {
            id: 301,
            status: "success".to_string(),
            stage: "test".to_string(),
            name: "test-matrix (ubuntu-latest, 20)".to_string(),
            matrix: None,
        }];
        let processed = process_pipeline_jobs(input_jobs);
        assert_eq!(processed.len(), 1);
        assert_eq!(processed[0].name, "test-matrix");
        assert_eq!(processed[0].matrix.as_deref(), Some("ubuntu-latest, 20"));
    }
}
