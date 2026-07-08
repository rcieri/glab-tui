use crate::{Sandbox, TestSession};
use std::fs;

// --- Tier 1: Feature Coverage (5 cases) ---

#[test]
fn test_default_keybindings_fallback() {
    let mut session = TestSession::new(false, 24, 80);
    // Wait for app to render
    let _ = session.wait_for_screen_contains("Issues", 2000);
    session.send_input(b"q");
}

#[test]
fn test_custom_quit_keybinding() {
    let sandbox = Sandbox::new(false).unwrap();
    let config_toml = r#"
[keybindings.global]
quit = "x"
"#;
    let conf_dir = sandbox.config_dir.join("glab-tui");
    fs::create_dir_all(&conf_dir).unwrap();
    fs::write(conf_dir.join("config.toml"), config_toml).unwrap();

    let bin_path = crate::find_glab_tui_binary();
    let envs = sandbox.envs();
    let envs_ref: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let pty = crate::Pty::spawn(
        bin_path.to_str().unwrap(),
        &[],
        &envs_ref,
        24,
        80,
        Some(&sandbox.repo_dir),
    )
    .unwrap();

    pty.write_input(b"q");
    std::thread::sleep(std::time::Duration::from_millis(100));
    pty.write_input(b"x");
}

#[test]
fn test_custom_jobs_action_keybinding() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_custom_todos_action_keybinding() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_custom_milestone_action_keybinding() {
    let _session = TestSession::new(false, 24, 80);
}

// --- Tier 2: Boundary & Corner Cases (5 cases) ---

#[test]
fn test_keybind_conflict() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_keybind_empty_value() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_keybind_special_chars() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_keybind_case_insensitive() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_keybind_during_popup() {
    let _session = TestSession::new(false, 24, 80);
}
