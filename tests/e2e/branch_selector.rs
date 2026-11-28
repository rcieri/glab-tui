//! Branch selectors inside the checkout the app runs in: the host's branches
//! come first, then the checkout's own branches the host does not list.

use crate::TestSession;
use std::process::Command;

const ASCII_ICONS: &str = "icons = \"ascii\"\n";
const HOST_BRANCH: &str = "feature/pagination";
const LOCAL_ONLY_BRANCH: &str = "only-local";

fn git(session: &TestSession, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(&session.sandbox.repo_dir)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed");
}

fn selector_rows(session: &TestSession) -> Vec<String> {
    session
        .emulator
        .get_text()
        .lines()
        .map(|row| row.trim().to_string())
        .collect()
}

#[test]
fn create_from_selector_lists_host_branches_and_local_only_branches() {
    let mut session = TestSession::with_config_and_args(
        false,
        30,
        140,
        Some(ASCII_ICONS),
        &["--tab", "branches"],
    );
    session
        .wait_for_screen_contains(HOST_BRANCH, 30000)
        .expect("the branches are listed");
    git(&session, &["commit", "--allow-empty", "-m", "init"]);
    git(&session, &["branch", LOCAL_ONLY_BRANCH]);

    session.press_keys(b"n");
    session
        .wait_for_screen_contains("Create Branch", 5000)
        .expect("n opens the create branch form");
    session.press_keys(b"j\r");
    session
        .wait_for_screen_contains(LOCAL_ONLY_BRANCH, 10000)
        .expect("the checkout's own branch is offered");

    let rows = selector_rows(&session);
    assert!(
        rows.iter()
            .any(|row| row.contains(&format!("- {LOCAL_ONLY_BRANCH}"))),
        "the local-only branch is a selector row:\n{}",
        rows.join("\n")
    );
    assert!(
        rows.iter()
            .any(|row| row.contains(&format!("- {HOST_BRANCH}"))),
        "the host's branch is a selector row:\n{}",
        rows.join("\n")
    );
}
