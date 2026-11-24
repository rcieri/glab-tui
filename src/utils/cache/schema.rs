//! The on-disk layout of a project cache file:
//!
//! ```json
//! { "schema_version": 1, "prefs": { ... }, "data": { ... } }
//! ```
//!
//! `prefs` holds what the user made and the hosts cannot give back (reviewed
//! file marks, unsent review drafts). `data` holds API results, which the next
//! fetch replaces. The two are read independently, so a change to the shape of
//! any data field costs that data, never the prefs.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

use crate::domain::deployments::{Deployment, Environment};
use crate::domain::pipelines::Job;

/// Bumped whenever a change to `CachePrefs` or `CacheData` needs more than
/// `#[serde(default)]` to read files written before it. Each bump appends the
/// step that rewrites the previous version's file to `MIGRATIONS`.
pub const SCHEMA_VERSION: u64 = 1;

type Migration = fn(Map<String, Value>) -> Map<String, Value>;

/// `MIGRATIONS[n]` rewrites a version `n` file into version `n + 1`.
const MIGRATIONS: [Migration; SCHEMA_VERSION as usize] = [split_prefs_from_data];

/// Pipelines whose jobs the cache keeps. Jobs of every highlighted MR's head
/// pipeline and of every child pipeline opened land here between two pipeline
/// listings, which alone would let the map grow for as long as glab-tui runs.
pub const MAX_CACHED_PIPELINE_JOBS: usize = 100;

#[derive(Default, Clone)]
pub struct ProjectCache {
    pub prefs: CachePrefs,
    pub data: CacheData,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct CachePrefs {
    /// Files marked as reviewed in the diff view, keyed by MR/PR iid.
    #[serde(deserialize_with = "or_default")]
    pub reviewed_files: HashMap<u64, Vec<String>>,
    /// Draft review comments not yet submitted, by project path, then MR/PR
    /// iid. A group-scope cache is shared by every project in the group, and
    /// the same iid recurs across them.
    #[serde(deserialize_with = "or_default")]
    pub draft_comments: HashMap<String, HashMap<u64, Vec<crate::domain::review::DraftComment>>>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct CacheData {
    pub issues: Vec<crate::domain::issues::Issue>,
    /// The states `issues` were listed for. Caches written before this field
    /// existed always listed every state.
    pub issues_state_query: crate::domain::state_query::StateQuery,
    pub mrs: Vec<crate::domain::mr::MergeRequest>,
    /// The states `mrs` were listed for.
    pub mrs_state_query: crate::domain::state_query::StateQuery,
    pub pipelines: Vec<crate::domain::pipelines::Pipeline>,
    pub runners: Vec<crate::domain::runners::Runner>,
    pub releases: Vec<crate::domain::releases::Release>,
    pub todos: Vec<crate::domain::notifications::Notification>,
    pub milestones: Vec<crate::domain::milestones::Milestone>,
    /// At most `MAX_CACHED_PIPELINE_JOBS` pipelines, written through
    /// `set_pipeline_jobs`.
    pub pipeline_jobs: HashMap<u64, Vec<Job>>,
    pub branches: Vec<crate::domain::branches::Branch>,
    pub environments: Vec<Environment>,
    /// Deployments by environment name, only for listed environments.
    pub deployments: HashMap<String, Vec<Deployment>>,
    /// Issues by milestone iid, only for listed milestones.
    pub milestone_issues: HashMap<u64, Vec<crate::domain::issues::Issue>>,
    pub labels: Vec<String>,
    pub label_colors: HashMap<String, String>,
    pub members: Vec<String>,
}

impl CacheData {
    /// Keeps the jobs of the newest `MAX_CACHED_PIPELINE_JOBS` pipelines of
    /// `jobs`. Pipeline ids only grow, so the highest ids are the newest.
    pub fn set_pipeline_jobs(&mut self, jobs: &HashMap<u64, Vec<Job>>) {
        let mut ids: Vec<u64> = jobs.keys().copied().collect();
        if ids.len() > MAX_CACHED_PIPELINE_JOBS {
            ids.select_nth_unstable_by(MAX_CACHED_PIPELINE_JOBS - 1, |a, b| b.cmp(a));
            ids.truncate(MAX_CACHED_PIPELINE_JOBS);
        }
        self.pipeline_jobs = ids
            .into_iter()
            .filter_map(|id| {
                jobs.get(&id)
                    .map(|pipeline_jobs| (id, pipeline_jobs.clone()))
            })
            .collect();
    }

    /// Replaces the listed environments and forgets the deployments of every
    /// environment no longer among them.
    pub fn set_environments(&mut self, environments: &[Environment]) {
        self.environments = environments.to_vec();
        self.deployments
            .retain(|name, _| environments.iter().any(|env| env.name == *name));
    }

    /// Stores the deployments of a listed environment. Those of any other
    /// environment are not kept, as `set_environments` would drop them.
    pub fn set_deployments(&mut self, environment: &str, deployments: &[Deployment]) {
        if self.environments.iter().any(|env| env.name == environment) {
            self.deployments
                .insert(environment.to_string(), deployments.to_vec());
        }
    }
}

#[derive(Serialize)]
struct CacheFile<'a> {
    schema_version: u64,
    prefs: &'a CachePrefs,
    data: &'a CacheData,
}

impl ProjectCache {
    pub fn to_json(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(&CacheFile {
            schema_version: SCHEMA_VERSION,
            prefs: &self.prefs,
            data: &self.data,
        })
    }

    /// Reads a cache file of any schema version. Unreadable sections come
    /// back empty instead of failing the whole file: prefs field by field,
    /// data as one, and the data of a file written by a newer glab-tui, whose
    /// fields may mean something else, is not read at all.
    pub fn from_json(content: &str) -> Self {
        let Ok(Value::Object(mut file)) = serde_json::from_str(content) else {
            return Self::default();
        };
        let version = match file.get("schema_version") {
            None => 0,
            Some(version) => version.as_u64().unwrap_or(u64::MAX),
        };
        if version < SCHEMA_VERSION {
            file = MIGRATIONS[version as usize..]
                .iter()
                .fold(file, |file, migrate| migrate(file));
        }
        let prefs = parse_section(file.remove("prefs"));
        let data = if version > SCHEMA_VERSION {
            CacheData::default()
        } else {
            parse_section(file.remove("data"))
        };
        Self { prefs, data }
    }
}

fn parse_section<T: DeserializeOwned + Default>(section: Option<Value>) -> T {
    section
        .and_then(|section| serde_json::from_value(section).ok())
        .unwrap_or_default()
}

/// Reads a field as its default when its stored shape no longer fits, so one
/// unreadable pref does not take the other prefs down with it.
fn or_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// Version 0 kept every field at the top level of the file.
fn split_prefs_from_data(mut file: Map<String, Value>) -> Map<String, Value> {
    let prefs: Map<String, Value> = ["reviewed_files", "draft_comments"]
        .into_iter()
        .filter_map(|key| file.remove_entry(key))
        .collect();
    Map::from_iter([
        ("prefs".to_string(), Value::Object(prefs)),
        ("data".to_string(), Value::Object(file)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::state_query::StateQuery;

    const DRAFT: &str = r#"{"file_path":"src/lib.rs","line_num":3,"body":"typo"}"#;

    fn draft_bodies(cache: &ProjectCache) -> Vec<&str> {
        cache.prefs.draft_comments["group/project"][&7]
            .iter()
            .map(|draft| draft.body.as_str())
            .collect()
    }

    #[test]
    fn a_saved_cache_reads_back_with_the_current_schema_version() {
        let mut cache = ProjectCache::default();
        cache
            .prefs
            .reviewed_files
            .insert(7, vec!["src/lib.rs".to_string()]);
        cache.data.labels = vec!["bug".to_string()];
        cache.data.mrs_state_query = StateQuery::Merged;

        let json = cache.to_json().unwrap();
        let file: Value = serde_json::from_slice(&json).unwrap();
        let reloaded = ProjectCache::from_json(std::str::from_utf8(&json).unwrap());

        assert_eq!(file["schema_version"], SCHEMA_VERSION);
        assert_eq!(reloaded.prefs.reviewed_files[&7], ["src/lib.rs"]);
        assert_eq!(reloaded.data.labels, ["bug"]);
        assert_eq!(reloaded.data.mrs_state_query, StateQuery::Merged);
    }

    #[test]
    fn a_version_0_file_keeps_its_prefs_and_data() {
        let legacy = format!(
            r#"{{"issues":[],"mrs":[],"pipelines":[],"runners":[],"releases":[],"todos":[],
            "milestones":[],"pipeline_jobs":{{}},"branches":[],"environments":[],
            "labels":["bug"],"reviewed_files":{{"7":["src/lib.rs"]}},
            "draft_comments":{{"group/project":{{"7":[{DRAFT}]}}}},
            "enabled_columns":{{}},"group_by_column":{{}}}}"#
        );

        let cache = ProjectCache::from_json(&legacy);

        assert_eq!(cache.prefs.reviewed_files[&7], ["src/lib.rs"]);
        assert_eq!(draft_bodies(&cache), ["typo"]);
        assert_eq!(cache.data.labels, ["bug"]);
        assert_eq!(cache.data.issues_state_query, StateQuery::All);
    }

    #[test]
    fn a_version_0_file_whose_data_no_longer_parses_keeps_its_prefs() {
        let legacy = format!(
            r#"{{"issues":"no longer a list","labels":["bug"],
            "reviewed_files":{{"7":["src/lib.rs"]}},
            "draft_comments":{{"group/project":{{"7":[{DRAFT}]}}}}}}"#
        );

        let cache = ProjectCache::from_json(&legacy);

        assert_eq!(cache.prefs.reviewed_files[&7], ["src/lib.rs"]);
        assert_eq!(draft_bodies(&cache), ["typo"]);
        assert!(cache.data.issues.is_empty());
        assert!(cache.data.labels.is_empty());
    }

    #[test]
    fn a_data_field_changing_shape_keeps_the_prefs() {
        let file = format!(
            r#"{{"schema_version":1,
            "prefs":{{"reviewed_files":{{"7":["src/lib.rs"]}},
                     "draft_comments":{{"group/project":{{"7":[{DRAFT}]}}}}}},
            "data":{{"pipelines":{{"was":"a list"}},"labels":["bug"]}}}}"#
        );

        let cache = ProjectCache::from_json(&file);

        assert_eq!(cache.prefs.reviewed_files[&7], ["src/lib.rs"]);
        assert_eq!(draft_bodies(&cache), ["typo"]);
        assert!(cache.data.pipelines.is_empty());
        assert!(cache.data.labels.is_empty());
    }

    #[test]
    fn an_unreadable_pref_leaves_the_other_prefs_alone() {
        let file = r#"{"schema_version":1,
            "prefs":{"reviewed_files":{"7":["src/lib.rs"]},"draft_comments":["not a map"]},
            "data":{"labels":["bug"]}}"#;

        let cache = ProjectCache::from_json(file);

        assert_eq!(cache.prefs.reviewed_files[&7], ["src/lib.rs"]);
        assert!(cache.prefs.draft_comments.is_empty());
        assert_eq!(cache.data.labels, ["bug"]);
    }

    #[test]
    fn a_file_from_a_newer_schema_keeps_its_prefs_and_drops_its_data() {
        let file = r#"{"schema_version":99,
            "prefs":{"reviewed_files":{"7":["src/lib.rs"]}},
            "data":{"labels":["bug"]}}"#;

        let cache = ProjectCache::from_json(file);

        assert_eq!(cache.prefs.reviewed_files[&7], ["src/lib.rs"]);
        assert!(cache.data.labels.is_empty());
    }

    #[test]
    fn a_file_that_is_not_json_reads_as_an_empty_cache() {
        let cache = ProjectCache::from_json("{\"schema_version\":1,\"prefs\":");

        assert!(cache.prefs.reviewed_files.is_empty());
        assert!(cache.data.labels.is_empty());
    }

    #[test]
    fn only_the_newest_pipelines_keep_their_jobs() {
        let jobs: HashMap<u64, Vec<Job>> = (1..=MAX_CACHED_PIPELINE_JOBS as u64 + 5)
            .map(|id| (id, Vec::new()))
            .collect();
        let mut data = CacheData::default();

        data.set_pipeline_jobs(&jobs);

        let mut kept: Vec<u64> = data.pipeline_jobs.into_keys().collect();
        kept.sort_unstable();
        let newest: Vec<u64> = (6..=MAX_CACHED_PIPELINE_JOBS as u64 + 5).collect();
        assert_eq!(kept, newest);
    }

    #[test]
    fn deployments_are_kept_only_for_listed_environments() {
        let environments = |names: &[&str]| -> Vec<Environment> {
            let listed: Vec<Value> = names
                .iter()
                .enumerate()
                .map(|(id, name)| serde_json::json!({"id": id, "name": name, "state": "available"}))
                .collect();
            serde_json::from_value(Value::Array(listed)).unwrap()
        };
        let deployments: Vec<Deployment> = serde_json::from_str(
            r#"[{"id":11,"iid":2,"ref_name":"main","tag":false,"sha":"abc","status":"success",
            "created_at":"2026-10-01T00:00:00Z","updated_at":"2026-10-01T00:00:00Z"}]"#,
        )
        .unwrap();
        let mut data = CacheData::default();
        data.set_environments(&environments(&["production", "staging"]));

        data.set_deployments("production", &deployments);
        data.set_deployments("review/gone", &deployments);
        let before_relisting: Vec<String> = data.deployments.keys().cloned().collect();
        data.set_environments(&environments(&["staging"]));

        assert_eq!(before_relisting, ["production"]);
        assert!(data.deployments.is_empty());
    }
}
