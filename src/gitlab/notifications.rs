use super::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub project_path: String,
    pub title: String,
    pub target_type: String, // "Issue" or "MergeRequest"
    pub target_iid: u64,
    pub state: String, // "unread" or "pending"
    pub updated_at: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubNotificationSubject {
    pub title: String,
    pub r#type: String,
    pub url: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubNotificationRepo {
    pub full_name: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubNotification {
    pub id: String,
    pub repository: GithubNotificationRepo,
    pub subject: GithubNotificationSubject,
    pub unread: bool,
    pub updated_at: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabTodoTarget {
    pub title: String,
    pub iid: u64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabTodoProject {
    pub path_with_namespace: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GitlabTodo {
    pub id: serde_json::Value,
    pub project: GitlabTodoProject,
    pub target: GitlabTodoTarget,
    pub target_type: String,
    pub state: String,
    pub updated_at: String,
}

pub async fn list_notifications(
    client: &GitlabClient,
    show_read: bool,
) -> Result<Vec<Notification>> {
    if client.is_github {
        let endpoint = if show_read {
            "notifications?all=true"
        } else {
            "notifications"
        };
        let raw = client.execute_github_api(endpoint, "GET", None).await?;
        let gh_notifs: Vec<GithubNotification> = serde_json::from_str(&raw)?;
        let mut list = Vec::new();
        for item in gh_notifs {
            let target_type = if item.subject.r#type == "PullRequest" {
                "MergeRequest".to_string()
            } else {
                "Issue".to_string()
            };

            let target_iid = item
                .subject
                .url
                .split('/')
                .last()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);

            let state = if item.unread {
                "unread".to_string()
            } else {
                "read".to_string()
            };

            list.push(Notification {
                id: item.id,
                project_path: item.repository.full_name,
                title: item.subject.title,
                target_type,
                target_iid,
                state,
                updated_at: item.updated_at,
            });
        }
        Ok(list)
    } else {
        let mut list = fetch_gitlab_todos(client, false).await?;
        if show_read {
            list.extend(fetch_gitlab_todos(client, true).await?);
        }
        Ok(list)
    }
}

async fn fetch_gitlab_todos(client: &GitlabClient, done: bool) -> Result<Vec<Notification>> {
    let endpoint = if done { "todos?state=done" } else { "todos" };
    let raw = client.execute_gitlab_api(endpoint, "GET", None).await?;
    let gl_todos: Vec<GitlabTodo> = serde_json::from_str(&raw)?;
    let mut list = Vec::new();
    for item in gl_todos {
        let id = match item.id {
            serde_json::Value::Number(num) => num.to_string(),
            serde_json::Value::String(s) => s,
            _ => String::new(),
        };
        list.push(Notification {
            id,
            project_path: item.project.path_with_namespace,
            title: item.target.title,
            target_type: item.target_type,
            target_iid: item.target.iid,
            state: item.state,
            updated_at: item.updated_at,
        });
    }
    Ok(list)
}

pub async fn mark_notification_as_read(client: &GitlabClient, id: &str) -> Result<()> {
    if client.is_github {
        let endpoint = format!("notifications/threads/{}", id);
        client.execute_github_api(&endpoint, "PATCH", None).await?;
        Ok(())
    } else {
        let endpoint = format!("todos/{}/mark_as_done", id);
        client.execute_gitlab_api(&endpoint, "POST", None).await?;
        Ok(())
    }
}
