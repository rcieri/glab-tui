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

pub async fn list_pipelines(client: &GitlabClient, project_path: &str) -> Result<Vec<Pipeline>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!("/projects/{}/pipelines?per_page=100", encoded_path);
    client.fetch_api(&endpoint).await
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

pub async fn list_pipeline_jobs(
    client: &GitlabClient,
    project_path: &str,
    pipeline_id: u64,
) -> Result<Vec<Job>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint_page1 = format!(
        "/projects/{}/pipelines/{}/jobs?per_page=100&page=1",
        encoded_path, pipeline_id
    );
    let mut all_jobs: Vec<Job> = client.fetch_api(&endpoint_page1).await?;

    if all_jobs.len() == 100 {
        let mut handles = Vec::new();
        for page in 2..=10 {
            let endpoint = format!(
                "/projects/{}/pipelines/{}/jobs?per_page=100&page={}",
                encoded_path, pipeline_id, page
            );
            let client_clone = client.clone();
            handles.push(tokio::spawn(async move {
                client_clone.fetch_api::<Vec<Job>>(&endpoint).await
            }));
        }

        for handle in handles {
            if let Ok(Ok(jobs)) = handle.await {
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

pub fn process_pipeline_jobs(all_jobs: Vec<Job>) -> Vec<Job> {
    // Parse matrix suffix from job names before deduplication.
    // GitLab matrix jobs look like: "build [ubuntu, run:test]"
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

    // Deduplicate jobs by (name, matrix) key, keeping only the one with the maximum ID (latest retry)
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
            // A retry of the failed job in the "test" stage
            Job {
                id: 103,
                status: "success".to_string(),
                stage: "test".to_string(),
                name: "run-tests".to_string(),
                matrix: None,
            },
            // A retry of the build job (which was already success but let's say retried anyway)
            Job {
                id: 104,
                status: "running".to_string(),
                stage: "build".to_string(),
                name: "compile-code".to_string(),
                matrix: None,
            },
        ];

        let processed = process_pipeline_jobs(input_jobs);

        // Deduplication check:
        // We expect only 2 jobs, because 'compile-code' (id 101 and 104) and 'run-tests' (id 102 and 103) were deduplicated.
        assert_eq!(processed.len(), 2);

        // Maximum ID (latest retry) check:
        // For 'compile-code', the latest is ID 104 (status: running)
        // For 'run-tests', the latest is ID 103 (status: success)
        let build_job = processed.iter().find(|j| j.name == "compile-code").unwrap();
        assert_eq!(build_job.id, 104);
        assert_eq!(build_job.status, "running");

        let test_job = processed.iter().find(|j| j.name == "run-tests").unwrap();
        assert_eq!(test_job.id, 103);
        assert_eq!(test_job.status, "success");

        // Stage ordering check:
        // The first run ID for 'build' stage is 101.
        // The first run ID for 'test' stage is 102.
        // Therefore, 'build' (101) < 'test' (102).
        // The processed jobs should be ordered by stage: build then test.
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
            // A retry of the ubuntu variant — should deduplicate by (name, matrix) not just name.
            Job {
                id: 203,
                status: "running".to_string(),
                stage: "test".to_string(),
                name: "run-tests [ubuntu, unit]".to_string(),
                matrix: None,
            },
            // A non-matrix job in the same stage.
            Job {
                id: 204,
                status: "success".to_string(),
                stage: "test".to_string(),
                name: "lint".to_string(),
                matrix: None,
            },
        ];

        let processed = process_pipeline_jobs(input_jobs);

        // 3 unique (name, matrix) combinations: (run-tests, ubuntu/unit), (run-tests, windows/integration), (lint, None)
        assert_eq!(processed.len(), 3);

        // Matrix field should be populated, base name should have brackets stripped
        let ubuntu = processed
            .iter()
            .find(|j| j.matrix.as_deref() == Some("ubuntu, unit"))
            .unwrap();
        assert_eq!(ubuntu.name, "run-tests");
        assert_eq!(ubuntu.id, 203); // latest retry wins

        let windows = processed
            .iter()
            .find(|j| j.matrix.as_deref() == Some("windows, integration"))
            .unwrap();
        assert_eq!(windows.name, "run-tests");
        assert_eq!(windows.id, 202);

        let lint = processed.iter().find(|j| j.name == "lint").unwrap();
        assert!(lint.matrix.is_none());
    }
}
