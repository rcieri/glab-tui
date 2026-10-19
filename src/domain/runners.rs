use crate::domain::client::GitlabClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct Runner {
    pub id: u64,
    pub description: Option<String>,
    pub status: String,
    pub active: bool,
    #[serde(default)]
    pub runner_type: Option<RunnerType>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub ip_address: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub contacted_at: Option<String>,
    #[serde(default)]
    pub web_url: Option<String>,
}

impl Runner {
    /// The state shown in the Status column. GitLab reports a paused runner
    /// as `paused` unless it is also `stale` or `never_contacted`, and
    /// whether it takes jobs matters more than either, so `active` decides
    /// first.
    pub fn status_kind(&self) -> RunnerStatus {
        if !self.active {
            return RunnerStatus::Paused;
        }
        match self.status.as_str() {
            "online" => RunnerStatus::Online,
            "paused" => RunnerStatus::Paused,
            "offline" => RunnerStatus::Offline,
            "stale" => RunnerStatus::Stale,
            "never_contacted" => RunnerStatus::NeverContacted,
            _ => RunnerStatus::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerStatus {
    Online,
    Paused,
    Offline,
    Stale,
    NeverContacted,
    Unknown,
}

impl RunnerStatus {
    /// The Status cell text, which the column filter also offers and matches.
    pub fn label(self) -> &'static str {
        match self {
            Self::Online => "ONLINE",
            Self::Paused => "PAUSED",
            Self::Offline => "OFFLINE",
            Self::Stale => "STALE",
            Self::NeverContacted => "NEVER CONTACTED",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// Where a GitLab runner is registered, from the API's `runner_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum RunnerType {
    #[serde(rename = "instance_type")]
    Instance,
    #[serde(rename = "group_type")]
    Group,
    #[serde(rename = "project_type")]
    Project,
    #[serde(other)]
    Unknown,
}

impl RunnerType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Instance => "Instance",
            Self::Group => "Group",
            Self::Project => "Project",
            Self::Unknown => "Unknown",
        }
    }
}

pub async fn list_runners(
    client: &GitlabClient,
    scope: &crate::scope::Scope,
) -> Result<Vec<Runner>> {
    client.backend.list_runners(scope, client.page_size).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(status: &str, active: bool) -> Runner {
        Runner {
            status: status.to_string(),
            active,
            ..Runner::default()
        }
    }

    #[test]
    fn status_kind_labels_every_gitlab_status() {
        let labels: Vec<&str> = [
            "online",
            "paused",
            "offline",
            "stale",
            "never_contacted",
            "not_a_state",
        ]
        .into_iter()
        .map(|status| runner(status, true).status_kind().label())
        .collect();
        assert_eq!(
            labels,
            [
                "ONLINE",
                "PAUSED",
                "OFFLINE",
                "STALE",
                "NEVER CONTACTED",
                "UNKNOWN"
            ]
        );
    }

    #[test]
    fn inactive_runner_is_paused_even_when_stale_or_never_contacted() {
        assert_eq!(runner("stale", false).status_kind(), RunnerStatus::Paused);
        assert_eq!(
            runner("never_contacted", false).status_kind(),
            RunnerStatus::Paused
        );
    }

    #[test]
    fn cached_runner_without_metadata_still_loads() {
        let runner: Runner = serde_json::from_str(
            r#"{"id": 7, "description": "old", "status": "online", "active": true}"#,
        )
        .unwrap();
        assert_eq!(runner.id, 7);
        assert!(runner.tags.is_empty());
        assert_eq!(runner.runner_type, None);
        assert_eq!(runner.web_url, None);
    }

    #[test]
    fn unknown_runner_type_does_not_reject_the_row() {
        let parsed: RunnerType = serde_json::from_str(r#""fleet_type""#).unwrap();
        assert_eq!(parsed, RunnerType::Unknown);
        let parsed: RunnerType = serde_json::from_str(r#""group_type""#).unwrap();
        assert_eq!(parsed.label(), "Group");
    }
}
