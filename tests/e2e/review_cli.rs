//! End-to-end coverage for the non-interactive `glab-tui review` subcommand.
//!
//! Both mocks serve `mr_diff.txt` as the diff. `glab` serves `mr_notes.json`
//! as the MR notes and `mr_view.json` (which carries the diff refs) as the MR;
//! `gh` serves `gh_pr_comments.json` and `gh_review_threads.json`. Request
//! bodies piped to `glab api`/`gh api` are appended to `<log>.stdin`.

use crate::{Sandbox, find_glab_tui_binary};
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run_review(sandbox: &Sandbox, args: &[&str], stdin: &str, extra: &[(&str, &str)]) -> Output {
    let mut child = Command::new(find_glab_tui_binary())
        .arg("review")
        .args(args)
        .current_dir(&sandbox.repo_dir)
        .env_remove("GLAB_TUI_CONFIG")
        .envs(sandbox.envs())
        .envs(extra.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("glab-tui should start");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn stdout_json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "review should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout should be JSON")
}

fn cli_calls(sandbox: &Sandbox) -> Vec<String> {
    std::fs::read_to_string(&sandbox.log_path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn request_bodies(sandbox: &Sandbox) -> Vec<Value> {
    let path = format!("{}.stdin", sandbox.log_path.display());
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("request body should be JSON"))
        .collect()
}

fn thread<'a>(threads: &'a Value, id: &str) -> &'a Value {
    threads
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap_or_else(|| panic!("thread {id} missing from {threads:#}"))
}

fn assert_failed_quietly(output: &Output, stderr_hint: &str) {
    assert!(!output.status.success(), "review should fail");
    assert!(
        output.stdout.is_empty(),
        "nothing may reach stdout on failure"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(stderr_hint),
        "stderr should mention '{stderr_hint}': {stderr}"
    );
}

const TWO_COMMENTS: &str = r#"[
  {"file": "src/lib.rs", "line": 3, "body": "Name this after the page fetch."},
  {"file": "src/lib.rs", "line": 2, "side": "old", "body": "Was old_fetch still needed?"}
]"#;

#[test]
fn test_review_threads_on_gitlab_groups_and_classifies_notes() {
    let sandbox = Sandbox::new(false).unwrap();
    let threads = stdout_json(&run_review(&sandbox, &["threads", "2"], "", &[]));

    assert_eq!(
        threads.as_array().map(Vec::len),
        Some(3),
        "system notes are dropped"
    );

    let anchored = thread(&threads, "disc-anchored");
    assert_eq!(anchored["classification"], "in-diff");
    assert_eq!(anchored["anchor"]["file"], "src/lib.rs");
    assert_eq!(anchored["anchor"]["line"], 3);
    assert_eq!(anchored["anchor"]["side"], "new");
    assert_eq!(anchored["resolved"], false);
    assert_eq!(anchored["notes"].as_array().map(Vec::len), Some(2));
    assert_eq!(anchored["notes"][1]["author"], "test-user");

    let general = thread(&threads, "disc-general");
    assert_eq!(general["classification"], "general");
    assert!(general["anchor"].is_null());

    assert_eq!(
        thread(&threads, "disc-outdated")["classification"],
        "outdated"
    );
}

#[test]
fn test_review_threads_on_github_reads_side_and_resolution() {
    let sandbox = Sandbox::new(true).unwrap();
    let threads = stdout_json(&run_review(&sandbox, &["threads", "2"], "", &[]));

    assert_eq!(
        threads.as_array().map(Vec::len),
        Some(4),
        "replies join their thread"
    );

    let anchored = thread(&threads, "201");
    assert_eq!(anchored["classification"], "in-diff");
    assert_eq!(anchored["notes"].as_array().map(Vec::len), Some(2));

    let removed_line = thread(&threads, "203");
    assert_eq!(removed_line["classification"], "in-diff");
    assert_eq!(removed_line["anchor"]["side"], "old");
    assert_eq!(removed_line["anchor"]["line"], 2);
    assert_eq!(removed_line["resolved"], true);

    assert_eq!(thread(&threads, "204")["classification"], "outdated");
}

#[test]
fn test_review_submit_on_gitlab_publishes_every_comment_as_one_review() {
    let sandbox = Sandbox::new(false).unwrap();
    let input = sandbox.temp_dir.path().join("comments.json");
    std::fs::write(&input, TWO_COMMENTS).unwrap();

    let output = run_review(
        &sandbox,
        &[
            "submit",
            "2",
            "--event",
            "approve",
            "--input",
            input.to_str().unwrap(),
        ],
        "",
        &[],
    );
    let result = stdout_json(&output);
    assert_eq!(result["comments"], 2);

    let calls = cli_calls(&sandbox);
    let position = |needle: &str| calls.iter().position(|c| c.contains(needle));
    let drafts: Vec<usize> = calls
        .iter()
        .enumerate()
        .filter(|(_, c)| c.ends_with("/merge_requests/2/draft_notes"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(drafts.len(), 2, "one draft note per comment: {calls:#?}");
    assert_eq!(
        calls.iter().filter(|c| c.contains("bulk_publish")).count(),
        1,
        "drafts are published once: {calls:#?}"
    );
    let publish = position("bulk_publish").unwrap();
    assert!(
        drafts.iter().all(|&d| d < publish),
        "publish follows the drafts"
    );
    assert!(
        position("mr approve 2").unwrap() > publish,
        "approve follows publishing"
    );

    let bodies = request_bodies(&sandbox);
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["position"]["new_line"], 3);
    assert_eq!(bodies[0]["position"]["head_sha"], "head0000");
    assert_eq!(bodies[1]["position"]["old_line"], 2);
    assert!(bodies[1]["position"]["new_line"].is_null());
}

#[test]
fn test_review_submit_on_github_posts_one_review_from_stdin() {
    let sandbox = Sandbox::new(true).unwrap();
    let output = run_review(
        &sandbox,
        &[
            "submit", "2", "--event", "approve", "--body", "LGTM", "--input", "-",
        ],
        TWO_COMMENTS,
        &[],
    );
    stdout_json(&output);

    let reviews = cli_calls(&sandbox)
        .into_iter()
        .filter(|c| c.contains("/pulls/2/reviews"))
        .count();
    assert_eq!(reviews, 1, "a review is a single API call");

    let bodies = request_bodies(&sandbox);
    assert_eq!(bodies.len(), 1);
    let review = &bodies[0];
    assert_eq!(review["event"], "APPROVE");
    assert_eq!(review["body"], "LGTM");
    let comments = review["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2);
    assert_eq!(
        (&comments[0]["side"], &comments[0]["line"]),
        (&Value::from("RIGHT"), &Value::from(3))
    );
    assert_eq!(
        (&comments[1]["side"], &comments[1]["line"]),
        (&Value::from("LEFT"), &Value::from(2))
    );
}

#[test]
fn test_review_submit_rejects_an_anchor_outside_the_diff_before_posting() {
    let sandbox = Sandbox::new(false).unwrap();
    let output = run_review(
        &sandbox,
        &["submit", "2", "--event", "comment", "--input", "-"],
        r#"[{"file": "src/lib.rs", "line": 99, "body": "Not in this diff."}]"#,
        &[],
    );

    assert_failed_quietly(&output, "src/lib.rs:99 (new side) is not part of the diff");
    assert!(
        !cli_calls(&sandbox)
            .iter()
            .any(|c| c.contains("draft_notes")),
        "nothing is posted when an anchor is invalid"
    );
}

#[test]
fn test_review_threads_reports_a_missing_mr_on_stderr() {
    let sandbox = Sandbox::new(false).unwrap();
    let output = run_review(
        &sandbox,
        &["threads", "404"],
        "",
        &[("TEST_GLAB_FAIL_MATCH", "mr diff")],
    );

    assert_failed_quietly(&output, "404 Not Found");
}

#[test]
fn test_review_resolve_on_github_is_unsupported() {
    let sandbox = Sandbox::new(true).unwrap();
    let output = run_review(&sandbox, &["resolve", "2", "--thread", "201"], "", &[]);

    assert_failed_quietly(&output, "isn't supported on GitHub");
}
