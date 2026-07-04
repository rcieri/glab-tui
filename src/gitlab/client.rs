use anyhow::{Context, Result};
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct GitlabClient {
    pub is_github: bool,
    pub tx: Option<tokio::sync::mpsc::UnboundedSender<crate::event::Event>>,
}

fn get_api_description(endpoint: &str, is_github: bool) -> String {
    let pr_suffix = if is_github { "PR" } else { "MR" };
    let prs_suffix = if is_github { "PRs" } else { "MRs" };

    if endpoint.contains("/issues/") {
        "Fetching Issue".to_string()
    } else if endpoint.contains("/issues") {
        "Fetching Issues".to_string()
    } else if endpoint.contains("/merge_requests/") {
        format!("Fetching {}", pr_suffix)
    } else if endpoint.contains("/merge_requests") {
        format!("Fetching {}", prs_suffix)
    } else if endpoint.contains("/pipelines/") && endpoint.contains("/jobs") {
        "Fetching Jobs".to_string()
    } else if endpoint.contains("/pipelines") {
        "Fetching Pipelines".to_string()
    } else if endpoint.contains("/runners") {
        "Fetching Runners".to_string()
    } else if endpoint.contains("/releases") {
        "Fetching Releases".to_string()
    } else if endpoint.contains("/milestones/") && endpoint.contains("/issues") {
        "Fetching Milestone Issues".to_string()
    } else if endpoint.contains("/milestones") {
        "Fetching Milestones".to_string()
    } else if endpoint.contains("/labels") {
        "Fetching Labels".to_string()
    } else if endpoint.contains("/members") {
        "Fetching Members".to_string()
    } else if endpoint.contains("notifications") || endpoint.contains("todos") {
        "Fetching Notifications".to_string()
    } else {
        "Fetching API".to_string()
    }
}

impl GitlabClient {
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
        Ok(Self {
            is_github,
            tx: None,
        })
    }

    pub async fn fetch_api<T: serde::de::DeserializeOwned>(&self, endpoint: &str) -> Result<T> {
        let desc = get_api_description(endpoint, self.is_github);
        let cmd_str = if self.is_github {
            let gh_endpoint = gitlab_to_github_endpoint(endpoint);
            format!("{}: gh api {}", desc, gh_endpoint)
        } else {
            format!("{}: glab api {}", desc, endpoint)
        };
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        if let Some(ref tx) = self.tx {
            let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                timestamp: timestamp.clone(),
                command: cmd_str.clone(),
                status: "Running".to_string(),
            });
        }

        let res = if self.is_github {
            let gh_endpoint = gitlab_to_github_endpoint(endpoint);
            let output = Command::new("gh")
                .args(["api", &gh_endpoint])
                .output()
                .await
                .context("Failed to execute gh api command");

            match output {
                Ok(out) => {
                    if out.status.success() {
                        let res_val: Result<T> = (|| {
                            let github_json: serde_json::Value =
                                serde_json::from_slice(&out.stdout)?;
                            let translated_json =
                                translate_json_to_gitlab(&gh_endpoint, github_json)?;
                            let data: T = serde_json::from_value(translated_json)?;
                            Ok(data)
                        })();
                        match res_val {
                            Ok(data) => {
                                if let Some(ref tx) = self.tx {
                                    let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                        timestamp: timestamp.clone(),
                                        command: cmd_str.clone(),
                                        status: "Success".to_string(),
                                    });
                                }
                                Ok(data)
                            }
                            Err(e) => {
                                let err_msg = format!("JSON error: {}", e);
                                if let Some(ref tx) = self.tx {
                                    let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                        timestamp: timestamp.clone(),
                                        command: cmd_str.clone(),
                                        status: format!("Failed: {}", err_msg),
                                    });
                                }
                                Err(e.into())
                            }
                        }
                    } else {
                        let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        if let Some(ref tx) = self.tx {
                            let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                timestamp: timestamp.clone(),
                                command: cmd_str.clone(),
                                status: format!("Failed: {}", err_msg),
                            });
                        }
                        anyhow::bail!("gh api failed: {}", err_msg)
                    }
                }
                Err(e) => {
                    let err_msg = format!("{}", e);
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: cmd_str.clone(),
                            status: format!("Failed: {}", err_msg),
                        });
                    }
                    Err(e.into())
                }
            }
        } else {
            let output = Command::new("glab")
                .args(["api", endpoint])
                .output()
                .await
                .context("Failed to execute glab api command");

            match output {
                Ok(out) => {
                    if out.status.success() {
                        let res_val: Result<T> =
                            serde_json::from_slice(&out.stdout).map_err(|e| e.into());
                        match res_val {
                            Ok(data) => {
                                if let Some(ref tx) = self.tx {
                                    let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                        timestamp: timestamp.clone(),
                                        command: cmd_str.clone(),
                                        status: "Success".to_string(),
                                    });
                                }
                                Ok(data)
                            }
                            Err(e) => {
                                let err_msg = format!("JSON error: {}", e);
                                if let Some(ref tx) = self.tx {
                                    let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                        timestamp: timestamp.clone(),
                                        command: cmd_str.clone(),
                                        status: format!("Failed: {}", err_msg),
                                    });
                                }
                                Err(e)
                            }
                        }
                    } else {
                        let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        if let Some(ref tx) = self.tx {
                            let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                timestamp: timestamp.clone(),
                                command: cmd_str.clone(),
                                status: format!("Failed: {}", err_msg),
                            });
                        }
                        anyhow::bail!("glab api failed: {}", err_msg)
                    }
                }
                Err(e) => {
                    let err_msg = format!("{}", e);
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: cmd_str.clone(),
                            status: format!("Failed: {}", err_msg),
                        });
                    }
                    Err(e.into())
                }
            }
        };
        res
    }

    pub async fn fetch_raw_api(&self, endpoint: &str) -> Result<String> {
        let desc = get_api_description(endpoint, self.is_github);
        let is_post = endpoint.contains("/retry") || endpoint.contains("/cancel");
        let is_logs = self.is_github && endpoint.contains("/jobs/") && endpoint.contains("/trace");
        let cmd_str = if is_logs {
            let parts: Vec<&str> = endpoint.split("/").collect();
            let mut job_id = "";
            for i in 0..parts.len() {
                if parts[i] == "jobs" && i + 1 < parts.len() {
                    job_id = parts[i + 1];
                    break;
                }
            }
            format!("{}: gh run view --job {} --log", desc, job_id)
        } else if self.is_github {
            let gh_endpoint = gitlab_to_github_endpoint(endpoint);
            let method = if is_post {
                "-X POST -H \"Content-Length: 0\" "
            } else {
                ""
            };
            format!("{}: gh api {}{}", desc, method, gh_endpoint)
        } else {
            let method = if is_post { "-X POST " } else { "" };
            format!("{}: glab api {}{}", desc, method, endpoint)
        };
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        if let Some(ref tx) = self.tx {
            let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                timestamp: timestamp.clone(),
                command: cmd_str.clone(),
                status: "Running".to_string(),
            });
        }

        let res = if is_logs {
            let parts: Vec<&str> = endpoint.split("/").collect();
            let mut job_id = "";
            for i in 0..parts.len() {
                if parts[i] == "jobs" && i + 1 < parts.len() {
                    job_id = parts[i + 1];
                    break;
                }
            }
            let mut cmd = Command::new("gh");
            cmd.arg("run");
            cmd.arg("view");
            cmd.arg("--job");
            cmd.arg(job_id);
            cmd.arg("--log");
            let output = cmd
                .output()
                .await
                .context("Failed to execute gh run view --job command");

            match output {
                Ok(out) => {
                    if out.status.success() {
                        let raw_str = String::from_utf8(out.stdout).map_err(|e| e.into());
                        match raw_str {
                            Ok(s) => {
                                if let Some(ref tx) = self.tx {
                                    let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                        timestamp: timestamp.clone(),
                                        command: cmd_str.clone(),
                                        status: "Success".to_string(),
                                    });
                                }
                                Ok(s)
                            }
                            Err(e) => {
                                let err_msg = format!("UTF-8 error: {}", e);
                                if let Some(ref tx) = self.tx {
                                    let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                        timestamp: timestamp.clone(),
                                        command: cmd_str.clone(),
                                        status: format!("Failed: {}", err_msg),
                                    });
                                }
                                Err(e)
                            }
                        }
                    } else {
                        let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        if let Some(ref tx) = self.tx {
                            let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                timestamp: timestamp.clone(),
                                command: cmd_str.clone(),
                                status: format!("Failed: {}", err_msg),
                            });
                        }
                        anyhow::bail!("gh run view failed: {}", err_msg)
                    }
                }
                Err(e) => {
                    let err_msg = format!("{}", e);
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: cmd_str.clone(),
                            status: format!("Failed: {}", err_msg),
                        });
                    }
                    Err(e.into())
                }
            }
        } else if self.is_github {
            let gh_endpoint = gitlab_to_github_endpoint(endpoint);
            let mut cmd = Command::new("gh");
            cmd.arg("api");
            if is_post {
                cmd.arg("-X");
                cmd.arg("POST");
                cmd.arg("-H");
                cmd.arg("Content-Length: 0");
            }
            cmd.arg(&gh_endpoint);
            let output = cmd
                .output()
                .await
                .context("Failed to execute gh api command");

            match output {
                Ok(out) => {
                    if out.status.success() {
                        let raw_str = String::from_utf8(out.stdout).map_err(|e| e.into());
                        match raw_str {
                            Ok(s) => {
                                if let Some(ref tx) = self.tx {
                                    let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                        timestamp: timestamp.clone(),
                                        command: cmd_str.clone(),
                                        status: "Success".to_string(),
                                    });
                                }
                                Ok(s)
                            }
                            Err(e) => {
                                let err_msg = format!("UTF-8 error: {}", e);
                                if let Some(ref tx) = self.tx {
                                    let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                        timestamp: timestamp.clone(),
                                        command: cmd_str.clone(),
                                        status: format!("Failed: {}", err_msg),
                                    });
                                }
                                Err(e)
                            }
                        }
                    } else {
                        let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        if let Some(ref tx) = self.tx {
                            let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                timestamp: timestamp.clone(),
                                command: cmd_str.clone(),
                                status: format!("Failed: {}", err_msg),
                            });
                        }
                        anyhow::bail!("gh api failed: {}", err_msg)
                    }
                }
                Err(e) => {
                    let err_msg = format!("{}", e);
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: cmd_str.clone(),
                            status: format!("Failed: {}", err_msg),
                        });
                    }
                    Err(e.into())
                }
            }
        } else {
            let mut cmd = Command::new("glab");
            cmd.arg("api");
            if is_post {
                cmd.arg("-X");
                cmd.arg("POST");
            }
            cmd.arg(endpoint);
            let output = cmd
                .output()
                .await
                .context("Failed to execute glab api command");

            match output {
                Ok(out) => {
                    if out.status.success() {
                        let raw_str = String::from_utf8(out.stdout).map_err(|e| e.into());
                        match raw_str {
                            Ok(s) => {
                                if let Some(ref tx) = self.tx {
                                    let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                        timestamp: timestamp.clone(),
                                        command: cmd_str.clone(),
                                        status: "Success".to_string(),
                                    });
                                }
                                Ok(s)
                            }
                            Err(e) => {
                                let err_msg = format!("UTF-8 error: {}", e);
                                if let Some(ref tx) = self.tx {
                                    let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                        timestamp: timestamp.clone(),
                                        command: cmd_str.clone(),
                                        status: format!("Failed: {}", err_msg),
                                    });
                                }
                                Err(e)
                            }
                        }
                    } else {
                        let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        if let Some(ref tx) = self.tx {
                            let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                                timestamp: timestamp.clone(),
                                command: cmd_str.clone(),
                                status: format!("Failed: {}", err_msg),
                            });
                        }
                        anyhow::bail!("glab api failed: {}", err_msg)
                    }
                }
                Err(e) => {
                    let err_msg = format!("{}", e);
                    if let Some(ref tx) = self.tx {
                        let _ = tx.send(crate::event::Event::TerminalCommandLogged {
                            timestamp: timestamp.clone(),
                            command: cmd_str.clone(),
                            status: format!("Failed: {}", err_msg),
                        });
                    }
                    Err(e.into())
                }
            }
        };
        res
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
        Ok(members
            .into_iter()
            .map(|m| format!("@{}", m.username))
            .collect())
    }

    pub async fn fetch_milestones(&self, project_path: &str) -> Result<Vec<String>> {
        #[derive(serde::Deserialize)]
        struct GitlabMilestone {
            title: String,
        }
        let encoded_path = project_path.replace("/", "%2F");
        let endpoint = format!(
            "/projects/{}/milestones?state=active&per_page=100",
            encoded_path
        );
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
            let p = format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]);
            return Ok(p.trim_end_matches(".git").to_string());
        }
        "unknown/unknown"
    } else {
        "unknown/unknown"
    };

    Ok(path.trim_end_matches(".git").to_string())
}

fn gitlab_to_github_endpoint(endpoint: &str) -> String {
    let mut normalized = endpoint.to_string();
    if !normalized.starts_with('/') {
        normalized = format!("/{}", normalized);
    }
    let decoded = normalized.replace("%2F", "/");
    let mut path = decoded.replace("/projects/", "/repos/");
    path = path.replace("state=opened", "state=open");
    path = path.replace("state=active", "state=open");
    path = path.replace("/merge_requests", "/pulls");
    path = path.replace("/pipelines", "/actions/runs");
    path = path.replace("/members/all", "/assignees");
    path = path.replace("/jobs/", "/actions/jobs/");
    path = path.replace("/trace", "/logs");
    path = path.replace("/retry", "/rerun");
    path = path.replace("/notes", "/comments");
    if path.contains("/milestones/") && path.contains("/issues") {
        if let Some(milestone_id) = path
            .split("/milestones/")
            .nth(1)
            .and_then(|s| s.split('/').next())
        {
            let base_path = path.split("/milestones/").next().unwrap_or("");
            return format!("{}/issues?milestone={}&state=all", base_path, milestone_id);
        }
    }
    path
}

fn translate_issue(v: &serde_json::Value) -> serde_json::Value {
    let iid = v.get("number").cloned().unwrap_or(serde_json::Value::Null);
    let title = v.get("title").cloned().unwrap_or(serde_json::Value::Null);
    let raw_state = v.get("state").and_then(|s| s.as_str()).unwrap_or("open");
    let state = if raw_state == "open" {
        "opened"
    } else {
        "closed"
    };

    let labels_val = v.get("labels").and_then(|l| l.as_array());
    let labels = match labels_val {
        Some(arr) => {
            let names: Vec<serde_json::Value> = arr
                .iter()
                .filter_map(|label| label.get("name").cloned())
                .collect();
            serde_json::Value::Array(names)
        }
        None => serde_json::Value::Array(vec![]),
    };

    let updated_at = v
        .get("updated_at")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let created_at = v
        .get("created_at")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let closed_at = v
        .get("closed_at")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let username = v
        .get("user")
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
            let list: Vec<serde_json::Value> = arr
                .iter()
                .map(|ass| {
                    let u = ass
                        .get("login")
                        .and_then(|l| l.as_str())
                        .unwrap_or("unknown");
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
        "created_at": created_at,
        "closed_at": closed_at,
        "author": author,
        "milestone": milestone,
        "assignees": assignees,
        "description": description,
        "due_date": serde_json::Value::Null,
    })
}

fn translate_mr(v: &serde_json::Value) -> serde_json::Value {
    let iid = v.get("number").cloned().unwrap_or(serde_json::Value::Null);
    let title = v.get("title").cloned().unwrap_or(serde_json::Value::Null);
    let raw_state = v.get("state").and_then(|s| s.as_str()).unwrap_or("open");
    let state = if raw_state == "open" {
        "opened"
    } else {
        "closed"
    };

    let labels_val = v.get("labels").and_then(|l| l.as_array());
    let labels = match labels_val {
        Some(arr) => {
            let names: Vec<serde_json::Value> = arr
                .iter()
                .filter_map(|label| label.get("name").cloned())
                .collect();
            serde_json::Value::Array(names)
        }
        None => serde_json::Value::Array(vec![]),
    };

    let updated_at = v
        .get("updated_at")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let username = v
        .get("user")
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
            let list: Vec<serde_json::Value> = arr
                .iter()
                .map(|ass| {
                    let u = ass
                        .get("login")
                        .and_then(|l| l.as_str())
                        .unwrap_or("unknown");
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
            let list: Vec<serde_json::Value> = arr
                .iter()
                .map(|rev| {
                    let u = rev
                        .get("login")
                        .and_then(|l| l.as_str())
                        .unwrap_or("unknown");
                    serde_json::json!({ "username": u })
                })
                .collect();
            serde_json::Value::Array(list)
        }
        None => serde_json::Value::Array(vec![]),
    };

    let target_branch = v
        .get("base")
        .and_then(|b| b.get("ref"))
        .cloned()
        .unwrap_or_else(|| serde_json::Value::String("main".to_string()));

    let source_branch = v
        .get("head")
        .and_then(|h| h.get("ref"))
        .cloned()
        .unwrap_or_else(|| serde_json::Value::String("".to_string()));

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
        "source_branch": source_branch,
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

    let r#ref = v
        .get("head_branch")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::String("main".to_string()));
    let updated_at = v
        .get("updated_at")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

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
    let tag_name = v
        .get("tag_name")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let name = v
        .get("name")
        .cloned()
        .filter(|n| !n.is_null())
        .unwrap_or_else(|| tag_name.clone());
    let released_at = v
        .get("published_at")
        .and_then(|v| v.as_str())
        .map(|s| serde_json::Value::String(s.chars().take(10).collect()))
        .unwrap_or(serde_json::Value::Null);
    let description = v.get("body").cloned().unwrap_or(serde_json::Value::Null);
    let author_name = v
        .get("author")
        .and_then(|a| a.get("login"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let assets_link = v
        .get("assets")
        .and_then(|a| a.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("browser_download_url"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    serde_json::json!({
        "name": name,
        "tag_name": tag_name,
        "released_at": released_at,
        "description": description,
        "author_name": author_name,
        "commit_id": null,
        "commit_title": null,
        "assets_link": assets_link,
    })
}

fn translate_json_to_gitlab(endpoint: &str, val: serde_json::Value) -> Result<serde_json::Value> {
    if endpoint.contains("/pulls/") && endpoint.contains("/comments") {
        if let Some(arr) = val.as_array() {
            let list: Vec<serde_json::Value> = arr
                .iter()
                .map(|c| {
                    let id = c.get("id").cloned().unwrap_or(serde_json::Value::Null);
                    let body = c.get("body").cloned().unwrap_or(serde_json::Value::Null);
                    let username = c
                        .get("user")
                        .and_then(|u| u.get("login"))
                        .and_then(|l| l.as_str())
                        .unwrap_or("unknown");
                    let author = serde_json::json!({ "username": username });
                    let created_at = c
                        .get("created_at")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);

                    let path = c.get("path").and_then(|p| p.as_str());
                    let line = c.get("line").and_then(|l| l.as_u64());
                    let side = c.get("side").and_then(|s| s.as_str()).unwrap_or("RIGHT");
                    let start_line = c.get("start_line").and_then(|l| l.as_u64());

                    let position = if let Some(p) = path {
                        let (new_line, old_line) = if side == "LEFT" {
                            (serde_json::Value::Null, serde_json::json!(line))
                        } else {
                            (serde_json::json!(line), serde_json::Value::Null)
                        };
                        serde_json::json!({
                            "new_path": p,
                            "old_path": p,
                            "new_line": new_line,
                            "old_line": old_line,
                            "start_line": start_line,
                            "position_type": "text"
                        })
                    } else {
                        serde_json::Value::Null
                    };

                    let in_reply_to_id = c.get("in_reply_to_id").and_then(|v| v.as_u64());
                    let disc_id = in_reply_to_id
                        .map(|rid| rid.to_string())
                        .unwrap_or_else(|| id.as_u64().unwrap_or(0).to_string());

                    serde_json::json!({
                        "id": id,
                        "body": body,
                        "author": author,
                        "created_at": created_at,
                        "system": false,
                        "position": position,
                        "discussion_id": disc_id,
                        "resolved": false,
                        "resolvable": true
                    })
                })
                .collect();
            Ok(serde_json::Value::Array(list))
        } else {
            Ok(serde_json::Value::Array(vec![]))
        }
    } else if endpoint.contains("/issues/") && endpoint.contains("/comments") {
        if let Some(arr) = val.as_array() {
            let list: Vec<serde_json::Value> = arr
                .iter()
                .map(|c| {
                    let id = c.get("id").cloned().unwrap_or(serde_json::Value::Null);
                    let body = c.get("body").cloned().unwrap_or(serde_json::Value::Null);
                    let username = c
                        .get("user")
                        .and_then(|u| u.get("login"))
                        .and_then(|l| l.as_str())
                        .unwrap_or("unknown");
                    let author = serde_json::json!({ "username": username });
                    let created_at = c
                        .get("created_at")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    serde_json::json!({
                        "id": id,
                        "body": body,
                        "author": author,
                        "created_at": created_at,
                        "system": false,
                        "position": serde_json::Value::Null
                    })
                })
                .collect();
            Ok(serde_json::Value::Array(list))
        } else {
            Ok(serde_json::Value::Array(vec![]))
        }
    } else if endpoint.contains("/issues") {
        if val.is_array() {
            if let Some(arr) = val.as_array() {
                let list: Vec<serde_json::Value> = arr
                    .iter()
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
                let list: Vec<serde_json::Value> = arr.iter().map(translate_mr).collect();
                Ok(serde_json::Value::Array(list))
            } else {
                Ok(serde_json::Value::Array(vec![]))
            }
        } else {
            Ok(translate_mr(&val))
        }
    } else if endpoint.contains("/actions/runs") && endpoint.contains("/jobs") {
        if let Some(arr) = val.get("jobs").and_then(|j| j.as_array()) {
            let list: Vec<serde_json::Value> = arr.iter().map(translate_job).collect();
            Ok(serde_json::Value::Array(list))
        } else {
            Ok(serde_json::Value::Array(vec![]))
        }
    } else if endpoint.contains("/actions/runs") {
        if let Some(arr) = val.get("workflow_runs").and_then(|w| w.as_array()) {
            let list: Vec<serde_json::Value> = arr.iter().map(translate_pipeline).collect();
            Ok(serde_json::Value::Array(list))
        } else {
            Ok(serde_json::Value::Array(vec![]))
        }
    } else if endpoint.contains("/assignees") {
        if let Some(arr) = val.as_array() {
            let list: Vec<serde_json::Value> = arr
                .iter()
                .map(|u| {
                    let login = u.get("login").and_then(|l| l.as_str()).unwrap_or("unknown");
                    serde_json::json!({
                        "username": login
                    })
                })
                .collect();
            Ok(serde_json::Value::Array(list))
        } else {
            Ok(serde_json::Value::Array(vec![]))
        }
    } else if endpoint.contains("/milestones") {
        if let Some(arr) = val.as_array() {
            let list: Vec<serde_json::Value> = arr
                .iter()
                .map(|m| {
                    let title = m.get("title").cloned().unwrap_or(serde_json::Value::Null);
                    let id = m.get("id").cloned().unwrap_or(serde_json::Value::Null);
                    let number = m.get("number").cloned().unwrap_or(serde_json::Value::Null);
                    let description = m
                        .get("description")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let state = m.get("state").and_then(|s| s.as_str()).unwrap_or("open");
                    let gl_state = if state == "open" { "active" } else { "closed" };
                    let due_on = m
                        .get("due_on")
                        .and_then(|v| v.as_str())
                        .map(|s| serde_json::Value::String(s.chars().take(10).collect()))
                        .unwrap_or(serde_json::Value::Null);
                    let created_at = m
                        .get("created_at")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    serde_json::json!({
                        "id": id,
                        "iid": number,
                        "title": title,
                        "description": description,
                        "state": gl_state,
                        "start_date": serde_json::Value::Null,
                        "due_date": due_on,
                        "created_at": created_at
                    })
                })
                .collect();
            Ok(serde_json::Value::Array(list))
        } else {
            Ok(serde_json::Value::Array(vec![]))
        }
    } else if endpoint.contains("/labels") {
        if let Some(arr) = val.as_array() {
            let list: Vec<serde_json::Value> = arr
                .iter()
                .filter_map(|l| {
                    l.get("name").map(|name| {
                        serde_json::json!({
                            "name": name
                        })
                    })
                })
                .collect();
            Ok(serde_json::Value::Array(list))
        } else {
            Ok(serde_json::Value::Array(vec![]))
        }
    } else if endpoint.contains("/actions/runners") {
        if let Some(arr) = val.get("runners").and_then(|r| r.as_array()) {
            let list: Vec<serde_json::Value> = arr.iter().map(translate_runner).collect();
            Ok(serde_json::Value::Array(list))
        } else {
            Ok(serde_json::Value::Array(vec![]))
        }
    } else if endpoint.contains("/releases") {
        if val.is_array() {
            if let Some(arr) = val.as_array() {
                let list: Vec<serde_json::Value> = arr.iter().map(translate_release).collect();
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
            gitlab_to_github_endpoint(
                "/projects/owner%2Frepo/merge_requests?state=opened&per_page=100"
            ),
            "/repos/owner/repo/pulls?state=open&per_page=100"
        );
        assert_eq!(
            gitlab_to_github_endpoint("/projects/owner%2Frepo/jobs/123/trace"),
            "/repos/owner/repo/actions/jobs/123/logs"
        );
        // Test endpoints without leading slash
        assert_eq!(
            gitlab_to_github_endpoint("projects/owner%2Frepo/pipelines/123/cancel"),
            "/repos/owner/repo/actions/runs/123/cancel"
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
        assert_eq!(gl_issue["due_date"], serde_json::Value::Null);
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

    #[test]
    fn test_translate_pull_comments_json() {
        let gh_comments = serde_json::json!([
            {
                "id": 1,
                "body": "a code comment",
                "path": "src/main.rs",
                "line": 42,
                "side": "RIGHT",
                "user": {"login": "octocat"},
                "created_at": "2026-06-01T00:00:00Z"
            },
            {
                "id": 2,
                "body": "another code comment",
                "path": "src/main.rs",
                "line": 10,
                "side": "LEFT",
                "user": {"login": "octocat"},
                "created_at": "2026-06-01T00:00:00Z"
            }
        ]);

        let gl_notes =
            translate_json_to_gitlab("/repos/owner/repo/pulls/123/comments", gh_comments).unwrap();
        let arr = gl_notes.as_array().unwrap();
        assert_eq!(arr.len(), 2);

        assert_eq!(arr[0]["id"], 1);
        assert_eq!(arr[0]["body"], "a code comment");
        assert_eq!(arr[0]["author"]["username"], "octocat");
        assert_eq!(arr[0]["position"]["new_path"], "src/main.rs");
        assert_eq!(arr[0]["position"]["new_line"], 42);
        assert!(arr[0]["position"]["old_line"].is_null());

        assert_eq!(arr[1]["id"], 2);
        assert_eq!(arr[1]["body"], "another code comment");
        assert_eq!(arr[1]["author"]["username"], "octocat");
        assert_eq!(arr[1]["position"]["new_path"], "src/main.rs");
        assert_eq!(arr[1]["position"]["old_line"], 10);
        assert!(arr[1]["position"]["new_line"].is_null());
    }
}
