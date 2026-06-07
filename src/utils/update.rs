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
    let target_arch = std::env::consts::ARCH;

    let arch_str = match target_arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => "amd64", // fallback
    };

    let asset_name = match target_os {
        "linux" => format!("glab-tui-linux-{}.tar.gz", arch_str),
        "macos" => format!("glab-tui-macos-{}.tar.gz", arch_str),
        "windows" => "glab-tui-windows-amd64.zip".to_string(),
        _ => anyhow::bail!("Unsupported operating system: {}", target_os),
    };

    let temp_dir = tempdir()?;
    let download_output = tokio::process::Command::new("gh")
        .args([
            "release",
            "download",
            latest_tag,
            "-R",
            "rcieri/glab-tui",
            "-p",
            &asset_name,
            "--dir",
            temp_dir.path().to_str().unwrap(),
        ])
        .output()
        .await?;

    if !download_output.status.success() {
        let err = String::from_utf8_lossy(&download_output.stderr);
        anyhow::bail!("Failed to download release binary: {}", err);
    }

    let archive_path = temp_dir.path().join(&asset_name);
    if !archive_path.exists() {
        anyhow::bail!(
            "Downloaded asset not found at expected path: {:?}",
            archive_path
        );
    }

    let extract_dir = temp_dir.path().join("extracted");
    fs::create_dir_all(&extract_dir)?;

    if target_os == "windows" {
        let output = tokio::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                    archive_path.to_str().unwrap(),
                    extract_dir.to_str().unwrap()
                ),
            ])
            .output()
            .await?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to unzip Windows release archive: {}", err);
        }
    } else {
        let output = tokio::process::Command::new("tar")
            .args([
                "-xzf",
                archive_path.to_str().unwrap(),
                "-C",
                extract_dir.to_str().unwrap(),
            ])
            .output()
            .await?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to untar Linux/macOS release archive: {}", err);
        }
    }

    let exe_filename = if target_os == "windows" {
        "glab-tui.exe"
    } else {
        "glab-tui"
    };
    let new_bin_path = extract_dir.join(exe_filename);
    if !new_bin_path.exists() {
        anyhow::bail!(
            "Extracted binary not found at expected path: {:?}",
            new_bin_path
        );
    }

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
