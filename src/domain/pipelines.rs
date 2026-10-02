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
    pub duration_seconds: Option<u64>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub project_path: String,
    #[serde(default)]
    pub web_url: Option<String>,
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
    pub fn duration_seconds(&self) -> Option<u64> {
        self.duration_seconds
    }
    pub fn created_at(&self) -> Option<&str> {
        self.created_at.as_deref()
    }
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
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
    #[serde(default)]
    pub duration_seconds: Option<u64>,
    #[serde(default)]
    pub runner: Option<String>,
    #[serde(default)]
    pub needs: Vec<String>,
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
    pub fn needs(&self) -> &[String] {
        &self.needs
    }
    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }
    pub fn set_matrix(&mut self, matrix: Option<String>) {
        self.matrix = matrix;
    }
}

/// A `trigger:` job in a parent pipeline. GitLab keeps these out of
/// `/pipelines/:id/jobs` and serves them from `/pipelines/:id/bridges`.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Bridge {
    pub id: u64,
    pub status: String,
    pub name: String,
    /// `None` for a trigger that has not spawned a pipeline yet, e.g. a
    /// `manual` one still waiting on someone.
    #[serde(rename = "downstream_pipeline")]
    pub downstream: Option<Downstream>,
}

/// The pipeline a bridge points at. GitLab embeds only these fields here —
/// no `source`, no `duration`, no `user`.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Downstream {
    pub id: u64,
    pub status: String,
    pub r#ref: String,
    pub sha: String,
    pub created_at: Option<String>,
    pub updated_at: String,
}

/// One level of the descent: the child pipelines reached through a parent's
/// bridges, plus the triggers that have not spawned anything yet.
#[derive(Debug, Default, Clone)]
pub struct ChildLevel {
    pub children: Vec<Pipeline>,
    pub pending: Vec<PendingTrigger>,
}

/// A `trigger:` job that has not spawned its pipeline yet — `manual`, `created`,
/// or failed before it got there. There is nothing to navigate into, so it is
/// kept out of the pipeline rows and rendered after them.
#[derive(Debug, Clone)]
pub struct PendingTrigger {
    /// The bridge's own job id, which is what
    /// `POST /projects/:id/jobs/:job_id/play` takes to start it.
    pub bridge_id: u64,
    pub name: String,
    pub status: String,
}

/// Split a parent pipeline's bridges into the two kinds of row its level shows.
/// A spawned bridge becomes its downstream pipeline, carrying the trigger job's
/// name so the row says which trigger produced it.
pub fn bridges_to_level(bridges: Vec<Bridge>) -> ChildLevel {
    let mut level = ChildLevel::default();
    for bridge in bridges {
        match bridge.downstream {
            Some(downstream) => level.children.push(Pipeline {
                id: downstream.id,
                status: downstream.status,
                r#ref: downstream.r#ref,
                updated_at: downstream.updated_at,
                name: bridge.name,
                display_title: String::new(),
                event: CHILD_SOURCE.to_string(),
                head_sha: downstream.sha,
                actor_login: String::new(),
                duration_seconds: None,
                created_at: downstream.created_at,
                source: Some(CHILD_SOURCE.to_string()),
                // GitLab's downstream_pipeline embed carries neither, so the
                // drill-down falls back to the parent's scope, as before.
                project_path: String::new(),
                web_url: None,
            }),
            None => level.pending.push(PendingTrigger {
                bridge_id: bridge.id,
                name: bridge.name,
                status: bridge.status,
            }),
        }
    }
    level
}

/// What GitLab calls the source of a pipeline reached through a bridge.
const CHILD_SOURCE: &str = "parent_pipeline";

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

pub async fn list_pipelines(
    client: &GitlabClient,
    scope: &crate::scope::Scope,
) -> Result<Vec<Pipeline>> {
    client
        .backend
        .list_pipelines(scope, client.page_size, client.api_per_page)
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

pub async fn list_pipeline_bridges(
    client: &GitlabClient,
    project_path: &str,
    pipeline_id: u64,
) -> Result<Vec<Bridge>> {
    client
        .backend
        .list_pipeline_bridges(project_path, pipeline_id, client.page_size)
        .await
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
                needs: Vec::new(),
            },
            Job {
                id: 102,
                status: "failed".into(),
                stage: "test".into(),
                name: "run-tests".into(),
                matrix: None,
                duration_seconds: None,
                runner: None,
                needs: Vec::new(),
            },
            Job {
                id: 103,
                status: "success".into(),
                stage: "test".into(),
                name: "run-tests".into(),
                matrix: None,
                duration_seconds: None,
                runner: None,
                needs: Vec::new(),
            },
            Job {
                id: 104,
                status: "running".into(),
                stage: "build".into(),
                name: "compile-code".into(),
                matrix: None,
                duration_seconds: None,
                runner: None,
                needs: Vec::new(),
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
                needs: Vec::new(),
            },
            Job {
                id: 202,
                status: "failed".into(),
                stage: "test".into(),
                name: "run-tests [windows, integration]".into(),
                matrix: None,
                duration_seconds: None,
                runner: None,
                needs: Vec::new(),
            },
            Job {
                id: 203,
                status: "running".into(),
                stage: "test".into(),
                name: "run-tests [ubuntu, unit]".into(),
                matrix: None,
                duration_seconds: None,
                runner: None,
                needs: Vec::new(),
            },
            Job {
                id: 204,
                status: "success".into(),
                stage: "test".into(),
                name: "lint".into(),
                matrix: None,
                duration_seconds: None,
                runner: None,
                needs: Vec::new(),
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
            status: "completed".into(),
            stage: "build".into(),
            name: "test-matrix (ubuntu-latest, 20)".into(),
            matrix: None,
            duration_seconds: None,
            runner: None,
            needs: Vec::new(),
        }];
        let processed = process_pipeline_jobs(input_jobs);
        assert_eq!(processed.len(), 1);
        assert_eq!(processed[0].name, "test-matrix");
        assert_eq!(processed[0].matrix.as_deref(), Some("ubuntu-latest, 20"));
    }

    #[test]
    fn test_deserialize_bridges_spawned_and_pending() {
        let raw = r#"[
            {
                "id": 91,
                "name": "trigger:build",
                "stage": "triggers",
                "status": "success",
                "downstream_pipeline": {
                    "id": 2001,
                    "sha": "f62a4b2fb89754372a346f24659212eb8da13601",
                    "ref": "main",
                    "status": "running",
                    "created_at": "2026-09-05T10:00:00.000Z",
                    "updated_at": "2026-09-05T10:05:00.000Z",
                    "web_url": "https://example.com/diaspora/pipelines/2001"
                }
            },
            {
                "id": 92,
                "name": "trigger:deploy",
                "stage": "triggers",
                "status": "manual",
                "downstream_pipeline": null
            }
        ]"#;

        let bridges: Vec<Bridge> = serde_json::from_str(raw).expect("bridges response");

        assert_eq!(bridges.len(), 2);

        assert_eq!(bridges[0].name, "trigger:build");
        let downstream = bridges[0]
            .downstream
            .as_ref()
            .expect("spawned bridge carries a downstream pipeline");
        assert_eq!(downstream.id, 2001);
        assert_eq!(downstream.status, "running");
        assert_eq!(downstream.r#ref, "main");

        assert_eq!(bridges[1].name, "trigger:deploy");
        assert_eq!(bridges[1].status, "manual");
        assert!(
            bridges[1].downstream.is_none(),
            "a trigger that never spawned has no downstream pipeline"
        );
    }

    #[test]
    fn test_bridges_split_into_children_and_pending_triggers() {
        let bridges = vec![
            Bridge {
                id: 91,
                status: "success".into(),
                name: "trigger:build".into(),
                downstream: Some(Downstream {
                    id: 2001,
                    status: "running".into(),
                    r#ref: "main".into(),
                    sha: "f62a4b2".into(),
                    created_at: Some("2026-09-05T10:00:00.000Z".into()),
                    updated_at: "2026-09-05T10:05:00.000Z".into(),
                }),
            },
            Bridge {
                id: 92,
                status: "manual".into(),
                name: "trigger:deploy".into(),
                downstream: None,
            },
        ];

        let level = bridges_to_level(bridges);

        assert_eq!(
            level.children.len(),
            1,
            "only bridges that spawned a pipeline become navigable rows"
        );
        let child = &level.children[0];
        assert_eq!(
            child.id, 2001,
            "row identity comes from the downstream pipeline"
        );
        assert_eq!(child.status, "running");
        assert_eq!(child.r#ref, "main");
        assert_eq!(child.head_sha, "f62a4b2");
        assert_eq!(
            child.name, "trigger:build",
            "the trigger job name is what identifies a child in the list"
        );
        assert_eq!(child.source.as_deref(), Some("parent_pipeline"));
        assert_eq!(
            child.duration_seconds, None,
            "GitLab does not embed a duration in downstream_pipeline"
        );

        assert_eq!(level.pending.len(), 1);
        let pending = &level.pending[0];
        assert_eq!(
            pending.bridge_id, 92,
            "the bridge's own job id is what POST /jobs/:id/play needs"
        );
        assert_eq!(pending.name, "trigger:deploy");
        assert_eq!(pending.status, "manual");
    }
}
