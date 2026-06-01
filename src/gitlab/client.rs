use anyhow::{Context, Result};
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct GitlabClient {
    pub is_github: bool,
}

impl GitlabClient {
    pub async fn new() -> Result<Self> {
        let is_github = match tokio::process::Command::new("git")
            .args(["remote", "get-url", "origin"])
            .output()
            .await {
                Ok(output) if output.status.success() => {
                    let url = String::from_utf8_lossy(&output.stdout);
                    url.contains("github.com")
                }
                _ => false
            };
        Ok(Self { is_github })
    }

    pub async fn fetch_api<T: serde::de::DeserializeOwned>(&self, endpoint: &str) -> Result<T> {
        if self.is_github {
            let gh_endpoint = gitlab_to_github_endpoint(endpoint);
            let output = Command::new("gh")
                .args(["api", &gh_endpoint])
                .output()
                .await
                .context("Failed to execute gh api command")?;

            if !output.status.success() {
                let err_msg = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("gh api failed: {}", err_msg);
            }

            let github_json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
            let translated_json = translate_json_to_gitlab(&gh_endpoint, github_json)?;
            
            let data: T = serde_json::from_value(translated_json)?;
            Ok(data)
        } else {
            let output = Command::new("glab")
                .args(["api", endpoint])
                .output()
                .await
                .context("Failed to execute glab api command")?;

            if !output.status.success() {
                let err_msg = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("glab api failed: {}", err_msg);
            }

            let data: T = serde_json::from_slice(&output.stdout)?;
            Ok(data)
        }
    }

    pub async fn fetch_raw_api(&self, endpoint: &str) -> Result<String> {
        if self.is_github {
            let gh_endpoint = gitlab_to_github_endpoint(endpoint);
            let output = Command::new("gh")
                .args(["api", &gh_endpoint])
                .output()
                .await
                .context("Failed to execute gh api command")?;

            if !output.status.success() {
                let err_msg = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("gh api failed: {}", err_msg);
            }

            Ok(String::from_utf8(output.stdout)?)
        } else {
            let output = Command::new("glab")
                .args(["api", endpoint])
                .output()
                .await
                .context("Failed to execute glab api command")?;

            if !output.status.success() {
                let err_msg = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("glab api failed: {}", err_msg);
            }

            Ok(String::from_utf8(output.stdout)?)
        }
    }

    pub async fn fetch_labels(&self, project_path: &str) -> Result<Vec<String>> {
        #[derive(serde::Deserialize)]
        struct GitlabLabel {
            name: String,
        }
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!("/projects/{}/labels?per_page=100", encoded_path);
        let labels: Vec<GitlabLabel> = self.fetch_api(&endpoint).await?;
        Ok(labels.into_iter().map(|l| l.name).collect())
    }

    pub async fn fetch_members(&self, project_path: &str) -> Result<Vec<String>> {
        #[derive(serde::Deserialize)]
        struct GitlabMember {
            username: String,
        }
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!("/projects/{}/members/all?per_page=100", encoded_path);
        let members: Vec<GitlabMember> = self.fetch_api(&endpoint).await?;
        Ok(members.into_iter().map(|m| format!("@{}", m.username)).collect())
    }

    pub async fn fetch_milestones(&self, project_path: &str) -> Result<Vec<String>> {
        #[derive(serde::Deserialize)]
        struct GitlabMilestone {
            title: String,
        }
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!("/projects/{}/milestones?state=active&per_page=100", encoded_path);
        let milestones: Vec<GitlabMilestone> = self.fetch_api(&endpoint).await?;
        Ok(milestones.into_iter().map(|m| m.title).collect())
    }
}

pub async fn get_project_context() -> Result<String> {
    // Execute `git remote get-url origin`
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
            let p = format!("{}/{}", parts[parts.len()-2], parts[parts.len()-1]);
            return Ok(p.trim_end_matches(".git").to_string());
        }
        "unknown/unknown"
    } else {
        "unknown/unknown"
    };

    Ok(path.trim_end_matches(".git").to_string())
}

fn gitlab_to_github_endpoint(endpoint: &str) -> String {
    let decoded = endpoint.replace("%2F", "/");
    let mut path = decoded.replace("/projects/", "/repos/");
    path = path.replace("state=opened", "state=open");
    path = path.replace("state=active", "state=open");
    path = path.replace("/merge_requests", "/pulls");
    path = path.replace("/pipelines", "/actions/runs");
    path = path.replace("/members/all", "/assignees");
    path = path.replace("/jobs/", "/actions/jobs/");
    path = path.replace("/trace", "/logs");
    path
}

fn translate_issue(v: &serde_json::Value) -> serde_json::Value {
    let iid = v.get("number").cloned().unwrap_or(serde_json::Value::Null);
    let title = v.get("title").cloned().unwrap_or(serde_json::Value::Null);
    let raw_state = v.get("state").and_then(|s| s.as_str()).unwrap_or("open");
    let state = if raw_state == "open" { "opened" } else { "closed" };
    
    let labels_val = v.get("labels").and_then(|l| l.as_array());
    let labels = match labels_val {
        Some(arr) => {
            let names: Vec<serde_json::Value> = arr.iter()
                .filter_map(|label| label.get("name").cloned())
                .collect();
            serde_json::Value::Array(names)
        }
        None => serde_json::Value::Array(vec![]),
    };
    
    let updated_at = v.get("updated_at").cloned().unwrap_or(serde_json::Value::Null);
    
    let username = v.get("user")
        .and_then(|u| u.get("login"))
        .and_then(|l| l.as_str())
        .unwrap_or("unknown");
    let author = serde_json::json!({ "username": username });
    
    let milestone = match v.get("milestone") {
        Some(m) if !m.is_null() => {
            let m_title = m.get("title").cloned().unwrap_or(serde_json::Value::Null);
            serde_json::json!({ "title": m_title })
        }
        _ => serde_json::Value::Null,
    };
    
    let assignees_val = v.get("assignees").and_then(|a| a.as_array());
    let assignees = match assignees_val {
        Some(arr) => {
            let list: Vec<serde_json::Value> = arr.iter()
                .map(|ass| {
                    let u = ass.get("login").and_then(|l| l.as_str()).unwrap_or("unknown");
                    serde_json::json!({ "username": u })
                })
                .collect();
            serde_json::Value::Array(list)
        }
        None => serde_json::Value::Array(vec![]),
    };
    
    let description = v.get("body").cloned().unwrap_or(serde_json::Value::Null);
    
    serde_json::json!({
        "iid": iid,
        "title": title,
        "state": state,
        "labels": labels,
        "updated_at": updated_at,
        "author": author,
        "milestone": milestone,
        "assignees": assignees,
        "description": description,
    })
}

fn translate_mr(v: &serde_json::Value) -> serde_json::Value {
    let iid = v.get("number").cloned().unwrap_or(serde_json::Value::Null);
    let title = v.get("title").cloned().unwrap_or(serde_json::Value::Null);
    let raw_state = v.get("state").and_then(|s| s.as_str()).unwrap_or("open");
    let state = if raw_state == "open" { "opened" } else { "closed" };
    
    let labels_val = v.get("labels").and_then(|l| l.as_array());
    let labels = match labels_val {
        Some(arr) => {
            let names: Vec<serde_json::Value> = arr.iter()
                .filter_map(|label| label.get("name").cloned())
                .collect();
            serde_json::Value::Array(names)
        }
        None => serde_json::Value::Array(vec![]),
    };
    
    let updated_at = v.get("updated_at").cloned().unwrap_or(serde_json::Value::Null);
    
    let username = v.get("user")
        .and_then(|u| u.get("login"))
        .and_then(|l| l.as_str())
        .unwrap_or("unknown");
    let author = serde_json::json!({ "username": username });
    
    let milestone = match v.get("milestone") {
        Some(m) if !m.is_null() => {
            let m_title = m.get("title").cloned().unwrap_or(serde_json::Value::Null);
            serde_json::json!({ "title": m_title })
        }
        _ => serde_json::Value::Null,
    };
    
    let assignees_val = v.get("assignees").and_then(|a| a.as_array());
    let assignees = match assignees_val {
        Some(arr) => {
            let list: Vec<serde_json::Value> = arr.iter()
                .map(|ass| {
                    let u = ass.get("login").and_then(|l| l.as_str()).unwrap_or("unknown");
                    serde_json::json!({ "username": u })
                })
                .collect();
            serde_json::Value::Array(list)
        }
        None => serde_json::Value::Array(vec![]),
    };
    
    let reviewers_val = v.get("requested_reviewers").and_then(|r| r.as_array());
    let reviewers = match reviewers_val {
        Some(arr) => {
            let list: Vec<serde_json::Value> = arr.iter()
                .map(|rev| {
                    let u = rev.get("login").and_then(|l| l.as_str()).unwrap_or("unknown");
                    serde_json::json!({ "username": u })
                })
                .collect();
            serde_json::Value::Array(list)
        }
        None => serde_json::Value::Array(vec![]),
    };
    
    let target_branch = v.get("base")
        .and_then(|b| b.get("ref"))
        .cloned()
        .unwrap_or_else(|| serde_json::Value::String("main".to_string()));
        
    let draft = v.get("draft").and_then(|d| d.as_bool()).unwrap_or(false);
    let description = v.get("body").cloned().unwrap_or(serde_json::Value::Null);
    
    serde_json::json!({
        "iid": iid,
        "title": title,
        "state": state,
        "labels": labels,
        "updated_at": updated_at,
        "author": author,
        "milestone": milestone,
        "assignees": assignees,
        "reviewers": reviewers,
        "target_branch": target_branch,
        "draft": draft,
        "description": description,
    })
}

fn translate_pipeline(v: &serde_json::Value) -> serde_json::Value {
    let id = v.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let status_raw = v.get("status").and_then(|s| s.as_str()).unwrap_or("queued");
    let conclusion_raw = v.get("conclusion").and_then(|c| c.as_str()).unwrap_or("");
    
    let status = if status_raw == "completed" {
        match conclusion_raw {
            "success" => "success",
            "failure" => "failed",
            "cancelled" => "canceled",
            "skipped" => "skipped",
            _ => "failed",
        }
    } else if status_raw == "in_progress" {
        "running"
    } else {
        "pending"
    };
    
    let r#ref = v.get("head_branch").cloned().unwrap_or_else(|| serde_json::Value::String("main".to_string()));
    let updated_at = v.get("updated_at").cloned().unwrap_or(serde_json::Value::Null);
    
    serde_json::json!({
        "id": id,
        "status": status,
        "ref": r#ref,
        "updated_at": updated_at,
    })
}

fn translate_job(v: &serde_json::Value) -> serde_json::Value {
    let id = v.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let status_raw = v.get("status").and_then(|s| s.as_str()).unwrap_or("queued");
    let conclusion_raw = v.get("conclusion").and_then(|c| c.as_str()).unwrap_or("");
    
    let status = if status_raw == "completed" {
        match conclusion_raw {
            "success" => "success",
            "failure" => "failed",
            "cancelled" => "canceled",
            "skipped" => "skipped",
            _ => "failed",
        }
    } else if status_raw == "in_progress" {
        "running"
    } else {
        "pending"
    };
    
    let name = v.get("name").cloned().unwrap_or(serde_json::Value::Null);
    
    serde_json::json!({
        "id": id,
        "status": status,
        "stage": "build",
        "name": name,
    })
}

fn translate_runner(v: &serde_json::Value) -> serde_json::Value {
    let id = v.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let description = v.get("name").cloned().unwrap_or(serde_json::Value::Null);
    let status = v.get("status").cloned().unwrap_or(serde_json::Value::Null);
    
    serde_json::json!({
        "id": id,
        "description": description,
        "status": status,
        "active": true,
    })
}

fn translate_release(v: &serde_json::Value) -> serde_json::Value {
    let tag_name = v.get("tag_name").cloned().unwrap_or(serde_json::Value::Null);
    let name = v.get("name").cloned().filter(|n| !n.is_null()).unwrap_or_else(|| tag_name.clone());
    let released_at = v.get("published_at").cloned().unwrap_or(serde_json::Value::Null);
    
    serde_json::json!({
        "name": name,
        "tag_name": tag_name,
        "released_at": released_at,
    })
}

fn translate_json_to_gitlab(endpoint: &str, val: serde_json::Value) -> Result<serde_json::Value> {
    if endpoint.contains("/issues") {
        if val.is_array() {
            if let Some(arr) = val.as_array() {
                let list: Vec<serde_json::Value> = arr.iter()
                    .filter(|item| item.get("pull_request").is_none())
                    .map(translate_issue)
                    .collect();
                Ok(serde_json::Value::Array(list))
            } else {
                Ok(serde_json::Value::Array(vec![]))
            }
        } else {
            Ok(translate_issue(&val))
        }
    } else if endpoint.contains("/pulls") {
        if val.is_array() {
            if let Some(arr) = val.as_array() {
                let list: Vec<serde_json::Value> = arr.iter()
                    .map(translate_mr)
                    .collect();
                Ok(serde_json::Value::Array(list))
            } else {
                Ok(serde_json::Value::Array(vec![]))
            }
        } else {
            Ok(translate_mr(&val))
        }
    } else if endpoint.contains("/actions/runs") && endpoint.contains("/jobs") {
        if let Some(arr) = val.get("jobs").and_then(|j| j.as_array()) {
            let list: Vec<serde_json::Value> = arr.iter()
                .map(translate_job)
                .collect();
            Ok(serde_json::Value::Array(list))
        } else {
            Ok(serde_json::Value::Array(vec![]))
        }
    } else if endpoint.contains("/actions/runs") {
        if let Some(arr) = val.get("workflow_runs").and_then(|w| w.as_array()) {
            let list: Vec<serde_json::Value> = arr.iter()
                .map(translate_pipeline)
                .collect();
            Ok(serde_json::Value::Array(list))
        } else {
            Ok(serde_json::Value::Array(vec![]))
        }
    } else if endpoint.contains("/assignees") {
        if let Some(arr) = val.as_array() {
            let list: Vec<serde_json::Value> = arr.iter()
                .map(|u| {
                    let login = u.get("login").and_then(|l| l.as_str()).unwrap_or("unknown");
                    serde_json::Value::String(format!("@{}", login))
                })
                .collect();
            Ok(serde_json::Value::Array(list))
        } else {
            Ok(serde_json::Value::Array(vec![]))
        }
    } else if endpoint.contains("/milestones") {
        if let Some(arr) = val.as_array() {
            let list: Vec<serde_json::Value> = arr.iter()
                .filter_map(|m| m.get("title").cloned())
                .collect();
            Ok(serde_json::Value::Array(list))
        } else {
            Ok(serde_json::Value::Array(vec![]))
        }
    } else if endpoint.contains("/labels") {
        if let Some(arr) = val.as_array() {
            let list: Vec<serde_json::Value> = arr.iter()
                .filter_map(|l| l.get("name").cloned())
                .collect();
            Ok(serde_json::Value::Array(list))
        } else {
            Ok(serde_json::Value::Array(vec![]))
        }
    } else if endpoint.contains("/actions/runners") {
        if let Some(arr) = val.get("runners").and_then(|r| r.as_array()) {
            let list: Vec<serde_json::Value> = arr.iter()
                .map(translate_runner)
                .collect();
            Ok(serde_json::Value::Array(list))
        } else {
            Ok(serde_json::Value::Array(vec![]))
        }
    } else if endpoint.contains("/releases") {
        if val.is_array() {
            if let Some(arr) = val.as_array() {
                let list: Vec<serde_json::Value> = arr.iter()
                    .map(translate_release)
                    .collect();
                Ok(serde_json::Value::Array(list))
            } else {
                Ok(serde_json::Value::Array(vec![]))
            }
        } else {
            Ok(translate_release(&val))
        }
    } else {
        Ok(val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gitlab_to_github_endpoint() {
        assert_eq!(
            gitlab_to_github_endpoint("/projects/owner%2Frepo/issues?state=opened&per_page=100"),
            "/repos/owner/repo/issues?state=open&per_page=100"
        );
        assert_eq!(
            gitlab_to_github_endpoint("/projects/owner%2Frepo/merge_requests?state=opened&per_page=100"),
            "/repos/owner/repo/pulls?state=open&per_page=100"
        );
        assert_eq!(
            gitlab_to_github_endpoint("/projects/owner%2Frepo/jobs/123/trace"),
            "/repos/owner/repo/actions/jobs/123/logs"
        );
    }

    #[test]
    fn test_translate_issue_json() {
        let gh_issue = serde_json::json!({
            "number": 42,
            "title": "A github issue",
            "state": "open",
            "labels": [{"name": "bug"}],
            "updated_at": "2026-06-01T00:00:00Z",
            "user": {"login": "octocat"},
            "milestone": {"title": "v1.0"},
            "assignees": [{"login": "octocat"}],
            "body": "Issue description"
        });

        let gl_issue = translate_issue(&gh_issue);

        assert_eq!(gl_issue["iid"], 42);
        assert_eq!(gl_issue["title"], "A github issue");
        assert_eq!(gl_issue["state"], "opened");
        assert_eq!(gl_issue["labels"][0], "bug");
        assert_eq!(gl_issue["author"]["username"], "octocat");
        assert_eq!(gl_issue["milestone"]["title"], "v1.0");
        assert_eq!(gl_issue["assignees"][0]["username"], "octocat");
        assert_eq!(gl_issue["description"], "Issue description");
    }

    #[test]
    fn test_translate_issues_list_filtering_prs() {
        let gh_issues = serde_json::json!([
            {
                "number": 1,
                "title": "Normal issue",
                "state": "open",
                "labels": [],
                "updated_at": "2026-06-01T00:00:00Z",
                "user": {"login": "user"},
                "milestone": null,
                "assignees": [],
                "body": "desc"
            },
            {
                "number": 2,
                "title": "A pull request",
                "state": "open",
                "labels": [],
                "updated_at": "2026-06-01T00:00:00Z",
                "user": {"login": "user"},
                "milestone": null,
                "assignees": [],
                "body": "desc",
                "pull_request": {"url": "https://api.github.com/..."}
            }
        ]);

        let gl_issues = translate_json_to_gitlab("/repos/owner/repo/issues", gh_issues).unwrap();
        let arr = gl_issues.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["iid"], 1);
        assert_eq!(arr[0]["title"], "Normal issue");
    }
}
