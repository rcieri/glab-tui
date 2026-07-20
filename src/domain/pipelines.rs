use crate::domain::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Pipeline {
    pub id: u64,
    pub status: String,
    pub r#ref: String,
    pub updated_at: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub display_title: String,
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub head_sha: String,
    #[serde(default)]
    pub actor_login: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub duration_seconds: Option<u64>,
    #[serde(default)]
    pub started_at: Option<String>,
}

impl Pipeline {
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn ref_branch(&self) -> &str {
        &self.r#ref
    }
    pub fn updated_at(&self) -> &str {
        &self.updated_at
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn display_title(&self) -> &str {
        &self.display_title
    }
    pub fn event(&self) -> &str {
        &self.event
    }
    pub fn head_sha(&self) -> &str {
        &self.head_sha
    }
    pub fn actor_login(&self) -> &str {
        &self.actor_login
    }
    pub fn created_at(&self) -> Option<&str> {
        self.created_at.as_deref()
    }
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
    pub fn duration_seconds(&self) -> Option<u64> {
        self.duration_seconds
    }
    pub fn started_at(&self) -> Option<&str> {
        self.started_at.as_deref()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Job {
    pub id: u64,
    pub status: String,
    pub stage: String,
    pub name: String,
    #[serde(skip)]
    pub matrix: Option<String>,
    #[serde(skip)]
    pub duration_seconds: Option<u64>,
    #[serde(skip)]
    pub runner: Option<String>,
    #[serde(skip)]
    pub needs: Option<Vec<String>>,
    #[serde(skip)]
    pub steps: Option<Vec<JobStep>>,
    #[serde(skip)]
    pub tags: Option<Vec<String>>,
}

/// An individual step within a GitHub Actions job.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct JobStep {
    pub name: String,
    pub number: u64,
    pub status: String,
    pub conclusion: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

impl Job {
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn stage(&self) -> &str {
        &self.stage
    }
    pub fn matrix(&self) -> Option<&str> {
        self.matrix.as_deref()
    }
    pub fn duration_seconds(&self) -> Option<u64> {
        self.duration_seconds
    }
    pub fn runner(&self) -> Option<&str> {
        self.runner.as_deref()
    }
    pub fn needs(&self) -> Option<&[String]> {
        self.needs.as_deref()
    }
    pub fn steps(&self) -> Option<&[JobStep]> {
        self.steps.as_deref()
    }
    pub fn tags(&self) -> Option<&[String]> {
        self.tags.as_deref()
    }
    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }
    pub fn set_matrix(&mut self, matrix: Option<String>) {
        self.matrix = matrix;
    }
}

pub fn process_pipeline_jobs(all_jobs: Vec<Job>) -> Vec<Job> {
    let all_jobs: Vec<Job> = all_jobs
        .into_iter()
        .map(|mut job_item| {
            let name = job_item.name.clone();
            if let (Some(bracket_start), Some(bracket_end)) = (name.rfind('['), name.rfind(']')) {
                if bracket_end == name.len() - 1 {
                    let matrix_content = name[bracket_start + 1..bracket_end].trim().to_string();
                    let base_name = name[..bracket_start].trim().to_string();
                    job_item.name = base_name;
                    job_item.matrix = Some(matrix_content);
                }
            } else if let (Some(paren_start), Some(paren_end)) = (name.rfind('('), name.rfind(')'))
            {
                if paren_end == name.len() - 1 {
                    let matrix_content = name[paren_start + 1..paren_end].trim().to_string();
                    let base_name = name[..paren_start].trim().to_string();
                    job_item.name = base_name;
                    job_item.matrix = Some(matrix_content);
                }
            }
            job_item
        })
        .collect();

    let mut stage_min_id = std::collections::HashMap::new();
    for j in all_jobs.iter() {
        let stage = j.stage.clone();
        let entry = stage_min_id.entry(stage).or_insert(j.id);
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

pub async fn list_pipelines(client: &GitlabClient, project_path: &str) -> Result<Vec<Pipeline>> {
    client
        .backend
        .list_pipelines(project_path, client.page_size)
        .await
}

pub async fn list_pipeline_jobs(
    client: &GitlabClient,
    project_path: &str,
    pipeline_id: u64,
) -> Result<Vec<Job>> {
    client
        .backend
        .list_pipeline_jobs(project_path, pipeline_id, client.page_size)
        .await
}

/// Normalizes a GitHub Actions status/conclusion pair into a GitLab-compatible
/// status string. Handles both lowercase (GitHub REST API) and uppercase inputs.
pub fn normalize_github_status(status: &str, conclusion: Option<&str>) -> String {
    let status_lower = status.to_lowercase();
    if status_lower == "completed" {
        match conclusion.map(|c| c.to_lowercase()).as_deref() {
            Some("success") => "success",
            Some("failure") => "failed",
            Some("cancelled") | Some("canceled") => "canceled",
            Some("skipped") => "skipped",
            _ => "failed",
        }
    } else if status_lower == "in_progress" {
        "running"
    } else if matches!(status_lower.as_str(), "queued" | "waiting" | "pending") {
        "pending"
    } else {
        "pending"
    }
    .to_string()
}

pub async fn get_job_trace(
    client: &GitlabClient,
    project_path: &str,
    job_id: u64,
) -> Result<String> {
    client.backend.get_job_trace(project_path, job_id).await
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
            normalize_github_status("completed", Some("canceled")),
            "canceled"
        );
        assert_eq!(
            normalize_github_status("completed", Some("skipped")),
            "skipped"
        );
        assert_eq!(normalize_github_status("in_progress", None), "running");
        assert_eq!(normalize_github_status("queued", None), "pending");
        assert_eq!(normalize_github_status("waiting", None), "pending");
        assert_eq!(normalize_github_status("pending", None), "pending");
        assert_eq!(
            normalize_github_status("COMPLETED", Some("SUCCESS")),
            "success"
        );
        assert_eq!(
            normalize_github_status("COMPLETED", Some("CANCELLED")),
            "canceled"
        );
        assert_eq!(normalize_github_status("IN_PROGRESS", None), "running");
        assert_eq!(normalize_github_status("QUEUED", None), "pending");
    }

    #[test]
    fn test_process_pipeline_jobs() {
        let input_jobs = vec![
            Job {
                id: 101,
                status: "success".into(),
                stage: "build".into(),
                name: "compile-code".into(),
                matrix: None,
                duration_seconds: None,
                runner: None,
                needs: None,
                steps: None,
                tags: None,
            },
            Job {
                id: 102,
                status: "failed".into(),
                stage: "test".into(),
                name: "run-tests".into(),
                matrix: None,
                duration_seconds: None,
                runner: None,
                needs: None,
                steps: None,
                tags: None,
            },
            Job {
                id: 103,
                status: "success".into(),
                stage: "test".into(),
                name: "run-tests".into(),
                matrix: None,
                duration_seconds: None,
                runner: None,
                needs: None,
                steps: None,
                tags: None,
            },
            Job {
                id: 104,
                status: "running".into(),
                stage: "build".into(),
                name: "compile-code".into(),
                matrix: None,
                duration_seconds: None,
                runner: None,
                needs: None,
                steps: None,
                tags: None,
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
                status: "success".into(),
                stage: "test".into(),
                name: "run-tests [ubuntu, unit]".into(),
                matrix: None,
                duration_seconds: None,
                runner: None,
                needs: None,
                steps: None,
                tags: None,
            },
            Job {
                id: 202,
                status: "failed".into(),
                stage: "test".into(),
                name: "run-tests [windows, integration]".into(),
                matrix: None,
                duration_seconds: None,
                runner: None,
                needs: None,
                steps: None,
                tags: None,
            },
            Job {
                id: 203,
                status: "running".into(),
                stage: "test".into(),
                name: "run-tests [ubuntu, unit]".into(),
                matrix: None,
                duration_seconds: None,
                runner: None,
                needs: None,
                steps: None,
                tags: None,
            },
            Job {
                id: 204,
                status: "success".into(),
                stage: "test".into(),
                name: "lint".into(),
                matrix: None,
                duration_seconds: None,
                runner: None,
                needs: None,
                steps: None,
                tags: None,
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
    }

    #[test]
    fn test_process_pipeline_jobs_github_matrix_parsing() {
        let input_jobs = vec![Job {
            id: 301,
            status: "success".into(),
            stage: String::new(),
            name: "test-matrix (ubuntu-latest, 20)".into(),
            matrix: None,
            duration_seconds: None,
            runner: None,
            needs: None,
            steps: None,
            tags: None,
        }];
        let processed = process_pipeline_jobs(input_jobs);
        assert_eq!(processed.len(), 1);
        assert_eq!(processed[0].name, "test-matrix");
        assert_eq!(processed[0].matrix.as_deref(), Some("ubuntu-latest, 20"));
        assert_eq!(processed[0].stage, "");
    }
}
