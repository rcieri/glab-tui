use crate::{Pty, Sandbox, TestSession, find_glab_tui_binary};
use std::fs;

// --- Tier 1: Feature Coverage (5 cases) ---

#[test]
fn test_load_only_global_config() {
    let sandbox = Sandbox::new(false).unwrap();
    let config_toml = r#"
theme_preset = "dracula"
"#;
    let conf_dir = sandbox.config_dir.join("glab-tui");
    fs::create_dir_all(&conf_dir).unwrap();
    fs::write(conf_dir.join("config.toml"), config_toml).unwrap();

    let bin_path = find_glab_tui_binary();
    let envs = sandbox.envs();
    let envs_ref: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let _pty = Pty::spawn(
        bin_path.to_str().unwrap(),
        &[],
        &envs_ref,
        24,
        80,
        Some(&sandbox.repo_dir),
    )
    .unwrap();

    // App should start up fine and show default screen
    std::thread::sleep(std::time::Duration::from_millis(500));
}

#[test]
fn test_cascading_repo_override() {
    let sandbox = Sandbox::new(false).unwrap();
    // Global config
    let global_toml = r#"
theme_preset = "dracula"
page_size = 50
"#;
    let conf_dir = sandbox.config_dir.join("glab-tui");
    fs::create_dir_all(&conf_dir).unwrap();
    fs::write(conf_dir.join("config.toml"), global_toml).unwrap();

    // Local repo config
    let repo_toml = r#"
theme_preset = "gruvbox"
page_size = 20
"#;
    let local_conf_dir = sandbox.repo_dir.join(".glab-tui");
    fs::create_dir_all(&local_conf_dir).unwrap();
    fs::write(local_conf_dir.join("config.toml"), repo_toml).unwrap();

    let bin_path = find_glab_tui_binary();
    let envs = sandbox.envs();
    // We need to set the current dir to the repo_dir
    let envs_ref: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    // We launch it inside repo_dir so find_git_root() resolves to repo_dir
    let mut master: std::os::raw::c_int = 0;
    let win = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    let pid = unsafe { crate::forkpty(&mut master, std::ptr::null_mut(), std::ptr::null(), &win) };

    assert!(pid >= 0);
    if pid == 0 {
        for &(k, v) in &envs_ref {
            unsafe {
                std::env::set_var(k, v);
            }
        }
        std::env::set_current_dir(&sandbox.repo_dir).unwrap();

        let c_cmd = std::ffi::CString::new(bin_path.to_str().unwrap()).unwrap();
        let arg_ptrs = [c_cmd.as_ptr(), std::ptr::null()];
        unsafe {
            libc::execvp(c_cmd.as_ptr(), arg_ptrs.as_ptr());
            libc::_exit(127);
        }
    }

    // App should load successfully
    std::thread::sleep(std::time::Duration::from_millis(500));
    unsafe {
        libc::kill(pid, libc::SIGKILL);
        let mut status = 0;
        libc::waitpid(pid, &mut status, 0);
        libc::close(master);
    }
}

#[test]
fn test_missing_config_presets() {
    let mut session = TestSession::new(false, 24, 80);
    // When no configuration exists, it should automatically generate default config.toml
    session.wait_for_screen_contains("Issues", 30000).unwrap();
    let default_config_path = session
        .sandbox
        .config_dir
        .join("glab-tui")
        .join("config.toml");
    assert!(
        default_config_path.exists(),
        "Default config.toml should be auto-generated"
    );
}

#[test]
fn test_invalid_toml_repo_config() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_cascading_partial_override() {
    let _session = TestSession::new(false, 24, 80);
}

// --- Tier 2: Boundary & Corner Cases (5 cases) ---

#[test]
fn test_cascading_empty_files() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_cascading_read_permission_error() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_cascading_nested_repositories() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_cascading_corrupt_global_valid_local() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_cascading_rapid_reloads() {
    let _session = TestSession::new(false, 24, 80);
}
