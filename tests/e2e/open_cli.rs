//! End-to-end coverage for `glab-tui open <entity> <id>` on GitLab, whose
//! job and milestone pages are opened by URL through `git web--browse`
//! (the mock browser logs `browser <url>`).

use crate::{Sandbox, find_glab_tui_binary};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run_open(sandbox: &Sandbox, entity: &str, id: &str) {
    let status = Command::new(find_glab_tui_binary())
        .args(["open", entity, id])
        .current_dir(&sandbox.repo_dir)
        .env_remove("GLAB_TUI_CONFIG")
        .envs(sandbox.envs())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("glab-tui should start");
    assert!(
        status.success(),
        "glab-tui open {entity} {id} should succeed"
    );
}

/// Waits for the browser line: `open` does not wait for the browser to start.
fn wait_for_browser(sandbox: &Sandbox, url: &str) {
    let expected = format!("browser {url}\n");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let calls = std::fs::read_to_string(&sandbox.log_path).unwrap_or_default();
        if calls.contains(&expected) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "expected {expected:?}, got:\n{calls}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn open_job_opens_the_job_page_of_the_origin_project() {
    let sandbox = Sandbox::new(false).unwrap();
    run_open(&sandbox, "job", "4242");
    wait_for_browser(
        &sandbox,
        "https://gitlab.com/test-owner/test-repo/-/jobs/4242",
    );
}

#[test]
fn open_milestone_opens_the_milestone_page_of_the_origin_project() {
    let sandbox = Sandbox::new(false).unwrap();
    run_open(&sandbox, "milestone", "2");
    wait_for_browser(
        &sandbox,
        "https://gitlab.com/test-owner/test-repo/-/milestones/2",
    );
}
