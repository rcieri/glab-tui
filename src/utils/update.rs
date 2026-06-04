use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use tempfile::tempdir;

pub async fn perform_self_update() -> Result<bool> {
    let output = tokio::process::Command::new("gh")
        .args(["api", "repos/rcieri/glab-tui/releases/latest"])
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("Failed to check for latest release from GitHub");
    }

    let json: Value = serde_json::from_slice(&output.stdout)?;
    let latest_tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .context("No tag_name in release")?;

    let current_version = env!("CARGO_PKG_VERSION");
    let current_tag = format!("v{}", current_version);
    if latest_tag == current_tag {
        return Ok(false);
    }

    let target_os = std::env::consts::OS;
    let pattern = if target_os == "windows" { "*.exe" } else { "*" };

    let temp_dir = tempdir()?;
    let download_output = tokio::process::Command::new("gh")
        .args([
            "release",
            "download",
            latest_tag,
            "-R",
            "rcieri/glab-tui",
            "-p",
            pattern,
            "--dir",
            temp_dir.path().to_str().unwrap(),
        ])
        .output()
        .await?;

    if !download_output.status.success() {
        let err = String::from_utf8_lossy(&download_output.stderr);
        anyhow::bail!("Failed to download release binary: {}", err);
    }

    let entries = fs::read_dir(temp_dir.path())?;
    let mut downloaded_file_path = None;
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            downloaded_file_path = Some(entry.path());
            break;
        }
    }

    let new_bin_path = downloaded_file_path.context("No file was downloaded")?;
    let current_exe = std::env::current_exe()?;

    let mut old_exe = current_exe.clone();
    old_exe.set_extension("old");
    let _ = fs::rename(&current_exe, &old_exe);

    fs::copy(&new_bin_path, &current_exe)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&current_exe)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&current_exe, perms)?;
    }

    let _ = fs::remove_file(old_exe);

    Ok(true)
}
