//! The lists the API description checks answer to. Every call or fixture
//! field the pinned descriptions cannot confirm is either an exception (the
//! description cannot say, so the check skips it) or a finding (glab-tui's
//! code or a fixture is wrong and an issue tracks it). Each entry carries its
//! reason, a finding the issue that tracks it.
//!
//! A check that meets an entry calls `Ledger::excuse` with the list and the
//! entry's key, which records the hit. The lists only shrink: an entry no test
//! hit anywhere in a run is stale, since its cause is gone or no test makes
//! the call. Every test is its own process, so the hits go to a file in the
//! directory `GLAB_TUI_API_HITS_DIR` names, and `just e2e` runs
//! `every_list_entry_was_hit_by_some_test` after the rest, over that file.
//!
//! The corrections to the descriptions and the exceptions about single
//! response fields or whole operations (`absent_ok`, `spec_absent_ok`) are inputs of `scripts/gen-api-descriptions.py`, which
//! fails when one is stale. They are only listed here.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::PathBuf;

const HITS_DIR_ENV: &str = "GLAB_TUI_API_HITS_DIR";
const HITS_FILE: &str = "hits.log";

/// The hand-kept lists, in `tests/api-lists.json`.
const SHIPPED: &str = include_str!("../api-lists.json");

/// The generator's inputs per host.
const GENERATOR_INPUTS: [(&str, &str); 2] = [
    (
        "github",
        include_str!("../../scripts/api-descriptions/github.json"),
    ),
    (
        "gitlab",
        include_str!("../../scripts/api-descriptions/gitlab.json"),
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Exception,
    Finding,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Exception => "exception",
            Kind::Finding => "finding",
        }
    }
}

#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// What the check names the entry by, e.g. a host, method and path.
    pub key: String,
    pub reason: String,
    #[serde(default)]
    pub issue: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lists {
    exceptions: Vec<Entry>,
    findings: Vec<Entry>,
}

impl Lists {
    pub fn parse(json: &str) -> Result<Lists, String> {
        let lists: Lists = serde_json::from_str(json).map_err(|e| e.to_string())?;
        for kind in [Kind::Exception, Kind::Finding] {
            let mut keys = BTreeSet::new();
            for entry in lists.entries(kind) {
                let key = &entry.key;
                if key.is_empty() || key.chars().any(char::is_control) {
                    return Err(format!("{}: {key:?} is not a usable key", kind.name()));
                }
                if !keys.insert(key) {
                    return Err(format!("{} {key:?} is listed twice", kind.name()));
                }
                if entry.reason.trim().is_empty() {
                    return Err(format!("{} {key:?} has no reason", kind.name()));
                }
                if kind == Kind::Finding && !entry.issue.as_deref().is_some_and(is_issue_link) {
                    return Err(format!("finding {key:?} has no link to its issue"));
                }
            }
        }
        Ok(lists)
    }

    fn entries(&self, kind: Kind) -> &[Entry] {
        match kind {
            Kind::Exception => &self.exceptions,
            Kind::Finding => &self.findings,
        }
    }

    fn entry(&self, kind: Kind, key: &str) -> Option<&Entry> {
        self.entries(kind).iter().find(|entry| entry.key == key)
    }
}

fn is_issue_link(link: &str) -> bool {
    ["https://github.com/", "https://gitlab.com/"]
        .iter()
        .any(|host| {
            link.strip_prefix(host)
                .is_some_and(|rest| rest.contains("/issues/"))
        })
}

/// The entries some test hit, shared by every test process through a file.
pub struct Hits {
    file: PathBuf,
}

impl Hits {
    pub fn in_dir(dir: &std::path::Path) -> Hits {
        Hits {
            file: dir.join(HITS_FILE),
        }
    }

    fn record(&self, kind: Kind, key: &str) {
        // One write per line, so appends from several processes stay whole.
        let line = format!("{}\t{key}\n", kind.name());
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)
            .and_then(|mut file| file.write_all(line.as_bytes()));
        written.unwrap_or_else(|e| panic!("cannot record a hit in {:?}: {e}", self.file));
    }

    fn read(&self) -> BTreeSet<(Kind, String)> {
        let text = std::fs::read_to_string(&self.file).unwrap_or_default();
        text.lines()
            .filter_map(|line| {
                let (kind, key) = line.split_once('\t')?;
                let kind = [Kind::Exception, Kind::Finding]
                    .into_iter()
                    .find(|k| k.name() == kind)?;
                Some((kind, key.to_string()))
            })
            .collect()
    }
}

pub struct Ledger {
    lists: Lists,
    hits: Option<Hits>,
}

impl Ledger {
    /// The shipped lists. Hits are recorded when `GLAB_TUI_API_HITS_DIR` is set.
    pub fn shipped() -> Ledger {
        let lists = Lists::parse(SHIPPED).unwrap_or_else(|e| panic!("tests/api-lists.json: {e}"));
        let hits = std::env::var_os(HITS_DIR_ENV).map(|dir| Hits::in_dir(dir.as_ref()));
        Ledger { lists, hits }
    }

    /// Lists from `json`, with no hit recorded.
    pub fn with_lists(json: &str) -> Ledger {
        let lists = Lists::parse(json).unwrap_or_else(|e| panic!("lists: {e}"));
        Ledger { lists, hits: None }
    }

    /// The entry a check may excuse a mismatch with, recording that a test
    /// needed it. Call it only once the check has found a mismatch: every
    /// lookup that finds an entry counts as a hit.
    pub fn excuse(&self, kind: Kind, key: &str) -> Option<&Entry> {
        let entry = self.lists.entry(kind, key)?;
        if let Some(hits) = &self.hits {
            hits.record(kind, key);
        }
        Some(entry)
    }
}

/// The entries of `lists` split by whether `hit` holds them.
struct Audit<'a> {
    hit: Vec<(Kind, &'a Entry)>,
    stale: Vec<(Kind, &'a Entry)>,
}

fn audit<'a>(lists: &'a Lists, hit: &BTreeSet<(Kind, String)>) -> Audit<'a> {
    let mut audit = Audit {
        hit: Vec::new(),
        stale: Vec::new(),
    };
    for kind in [Kind::Exception, Kind::Finding] {
        for entry in lists.entries(kind) {
            let was_hit = hit.contains(&(kind, entry.key.clone()));
            let side = if was_hit {
                &mut audit.hit
            } else {
                &mut audit.stale
            };
            side.push((kind, entry));
        }
    }
    audit
}

fn describe(kind: Kind, entry: &Entry) -> String {
    let issue = entry
        .issue
        .as_deref()
        .map(|issue| format!(" ({issue})"))
        .unwrap_or_default();
    format!("{} {}: {}{issue}", kind.name(), entry.key, entry.reason)
}

/// One line per correction and per exception about a single response field
/// or an operation the description lacks, from the generator's inputs.
fn generator_entries(inputs: &[(&str, &str)]) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for &(host, json) in inputs {
        let input: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let text = |value: &serde_json::Value, field: &str| {
            value[field]
                .as_str()
                .filter(|text| !text.trim().is_empty())
                .map(str::to_string)
        };
        for correction in input["corrections"].as_array().into_iter().flatten() {
            let named = |field| {
                text(correction, field).ok_or(format!("{host}: a correction has no {field}"))
            };
            lines.push(format!(
                "correction {host}: {}.{} is {} ({}, entity {})",
                named("schema")?,
                named("field")?,
                named("type")?,
                named("upstream")?,
                named("entity")?,
            ));
        }
        for op in input["operations"].as_array().into_iter().flatten() {
            let call = format!(
                "{} {}",
                text(op, "method").unwrap_or_default().to_uppercase(),
                text(op, "path").unwrap_or_default()
            );
            if let Some(reason) = text(op, "spec_absent_ok") {
                lines.push(format!(
                    "exception {host}: {call} is not in the spec: {reason}"
                ));
            } else if op.get("spec_absent_ok").is_some() {
                return Err(format!("{host}: {call}: spec_absent_ok has no reason"));
            }
            for (field, reason) in op["absent_ok"].as_object().into_iter().flatten() {
                let reason = reason
                    .as_str()
                    .filter(|reason| !reason.trim().is_empty())
                    .ok_or(format!("{host}: {call}: absent_ok {field} has no reason"))?;
                lines.push(format!(
                    "exception {host}: {call} response field {field} is not in the spec: {reason}"
                ));
            }
        }
    }
    Ok(lines)
}

#[test]
fn the_shipped_lists_are_well_formed() {
    Lists::parse(SHIPPED).unwrap();
}

#[test]
fn the_generator_inputs_list_their_corrections_and_exceptions_with_reasons() {
    let input = r#"{
        "corrections": [
            {"schema": "S", "field": "f", "type": "integer",
             "upstream": "https://example.test/u", "entity": "E"}
        ],
        "operations": [
            {"method": "post", "path": "/x", "spec_absent_ok": "not described",
             "absent_ok": {"field": "never sent"}}
        ]
    }"#;
    let lines = generator_entries(&[("host", input)]).unwrap();
    assert_eq!(
        lines,
        [
            "correction host: S.f is integer (https://example.test/u, entity E)",
            "exception host: POST /x is not in the spec: not described",
            "exception host: POST /x response field field is not in the spec: never sent",
        ]
    );
    let reasonless = r#"{"operations": [{"method": "get", "path": "/y", "spec_absent_ok": " "}]}"#;
    assert!(generator_entries(&[("host", reasonless)]).is_err());
    assert!(!generator_entries(&GENERATOR_INPUTS).unwrap().is_empty());
}

fn lists() -> Lists {
    Lists::parse(
        r#"{
            "exceptions": [
                {"key": "github GET /a", "reason": "the spec cannot say"}
            ],
            "findings": [
                {"key": "gitlab GET /b", "reason": "tag is required",
                 "issue": "https://github.com/rcieri/glab-tui/issues/1"}
            ]
        }"#,
    )
    .unwrap()
}

#[test]
fn an_entry_without_a_reason_a_key_or_an_issue_link_is_refused() {
    for (json, error) in [
        (
            r#"{"exceptions": [{"key": "k", "reason": " "}], "findings": []}"#,
            "exception \"k\" has no reason",
        ),
        (
            r#"{"exceptions": [{"key": "", "reason": "r"}], "findings": []}"#,
            "exception: \"\" is not a usable key",
        ),
        (
            r#"{"exceptions": [], "findings": [{"key": "k", "reason": "r"}]}"#,
            "finding \"k\" has no link to its issue",
        ),
        (
            r#"{"exceptions": [], "findings": [{"key": "k", "reason": "r", "issue": "soon"}]}"#,
            "finding \"k\" has no link to its issue",
        ),
        (
            r#"{"exceptions": [{"key": "k", "reason": "r"}, {"key": "k", "reason": "s"}], "findings": []}"#,
            "exception \"k\" is listed twice",
        ),
    ] {
        assert_eq!(Lists::parse(json).unwrap_err(), error);
    }
}

#[test]
fn an_entry_no_test_hit_is_stale_and_one_that_was_hit_is_listed() {
    let lists = lists();
    let hit = BTreeSet::from([(Kind::Exception, "github GET /a".to_string())]);
    let audit = audit(&lists, &hit);
    let described = |entries: &[(Kind, &Entry)]| {
        entries
            .iter()
            .map(|(kind, entry)| describe(*kind, entry))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        described(&audit.hit),
        ["exception github GET /a: the spec cannot say"]
    );
    assert_eq!(
        described(&audit.stale),
        ["finding gitlab GET /b: tag is required (https://github.com/rcieri/glab-tui/issues/1)"]
    );
}

#[test]
fn the_same_key_in_the_other_list_does_not_count_as_a_hit() {
    let lists = lists();
    let hit = BTreeSet::from([(Kind::Finding, "github GET /a".to_string())]);
    assert_eq!(audit(&lists, &hit).stale.len(), 2);
}

#[test]
fn excusing_records_a_hit_that_another_process_reads() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger {
        lists: lists(),
        hits: Some(Hits::in_dir(dir.path())),
    };
    assert!(
        ledger
            .excuse(Kind::Exception, "github GET /unlisted")
            .is_none()
    );
    assert!(ledger.excuse(Kind::Exception, "github GET /a").is_some());
    let other_process = Hits::in_dir(dir.path());
    assert_eq!(
        other_process.read(),
        BTreeSet::from([(Kind::Exception, "github GET /a".to_string())])
    );
}

#[test]
fn excusing_without_a_hits_directory_records_nothing() {
    let ledger = Ledger {
        lists: lists(),
        hits: None,
    };
    assert!(ledger.excuse(Kind::Finding, "gitlab GET /b").is_some());
}

/// Run by `just e2e` after the other tests, over the hits they recorded.
#[test]
#[ignore = "needs the hits of a whole run: `just e2e` runs it"]
fn every_list_entry_was_hit_by_some_test() {
    let ledger = Ledger::shipped();
    let hits = ledger
        .hits
        .as_ref()
        .unwrap_or_else(|| panic!("{HITS_DIR_ENV} names no directory: run `just e2e`"));
    let audit = audit(&ledger.lists, &hits.read());
    println!("entries hit in this run:");
    for (kind, entry) in &audit.hit {
        println!("  {}", describe(*kind, entry));
    }
    println!("corrections and field exceptions from scripts/api-descriptions:");
    for line in generator_entries(&GENERATOR_INPUTS).unwrap() {
        println!("  {line}");
    }
    let stale: Vec<String> = audit
        .stale
        .iter()
        .map(|(kind, entry)| describe(*kind, entry))
        .collect();
    assert!(
        stale.is_empty(),
        "no test hit these entries: their cause is gone or no test makes the call. \
         Remove them from tests/api-lists.json:\n{}",
        stale.join("\n")
    );
}
