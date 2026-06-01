use super::client::GitlabClient;
use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Pipeline {
    pub id: u64,
    pub status: String,
    pub r#ref: String,
    pub updated_at: String,
}

pub async fn list_pipelines(client: &GitlabClient, project_path: &str) -> Result<Vec<Pipeline>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint = format!("/projects/{}/pipelines?per_page=20", encoded_path);
    client.fetch_api(&endpoint).await
}

#[derive(Debug, Deserialize, Clone)]
pub struct Job {
    pub id: u64,
    pub status: String,
    pub stage: String,
    pub name: String,
}

pub async fn list_pipeline_jobs(client: &GitlabClient, project_path: &str, pipeline_id: u64) -> Result<Vec<Job>> {
    let encoded_path = project_path.replace("/", "%2F");
    let endpoint_page1 = format!("/projects/{}/pipelines/{}/jobs?per_page=100&page=1", encoded_path, pipeline_id);
    let mut all_jobs: Vec<Job> = client.fetch_api(&endpoint_page1).await?;
    
    if all_jobs.len() == 100 {
        let mut handles = Vec::new();
        for page in 2..=10 {
            let endpoint = format!("/projects/{}/pipelines/{}/jobs?per_page=100&page={}", encoded_path, pipeline_id, page);
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
    let mut stage_min_id = std::collections::HashMap::new();
    for j in all_jobs.iter() {
        let entry = stage_min_id.entry(j.stage.clone()).or_insert(j.id);
        if j.id < *entry {
            *entry = j.id;
        }
    }

    // Deduplicate jobs by name, keeping only the one with the maximum ID (latest retry)
    let mut deduplicated: std::collections::HashMap<String, Job> = std::collections::HashMap::new();
    for job in all_jobs {
        let entry = deduplicated.entry(job.name.clone()).or_insert_with(|| job.clone());
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

pub async fn get_job_trace(client: &GitlabClient, project_path: &str, job_id: u64) -> Result<String> {
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
            },
            Job {
                id: 102,
                status: "failed".to_string(),
                stage: "test".to_string(),
                name: "run-tests".to_string(),
            },
            // A retry of the failed job in the "test" stage
            Job {
                id: 103,
                status: "success".to_string(),
                stage: "test".to_string(),
                name: "run-tests".to_string(),
            },
            // A retry of the build job (which was already success but let's say retried anyway)
            Job {
                id: 104,
                status: "running".to_string(),
                stage: "build".to_string(),
                name: "compile-code".to_string(),
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
}
