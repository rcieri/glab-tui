use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use tempfile::tempdir;

fn read_linux_distro() -> Option<String> {
    let contents = fs::read_to_string("/etc/os-release").ok()?;
    let mut id = None;
    let mut version_id = None;
    for line in contents.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("ID=") {
            id = Some(v.trim_matches('"').to_string());
        } else if let Some(v) = line.strip_prefix("VERSION_ID=") {
            version_id = Some(v.trim_matches('"').to_string());
        }
    }
    let id = id?;
    if id == "ubuntu" {
        Some(format!("ubuntu-{}", version_id.unwrap_or_default()))
    } else {
        Some(id)
    }
}

/// Known Ubuntu LTS baselines, newest first. Used as the fallback chain when
/// the local Ubuntu version isn't explicitly built for.
const UBUNTU_LTS_FALLBACKS: &[&str] = &["ubuntu-24.04", "ubuntu-22.04"];

fn push_unique(out: &mut Vec<String>, name: String) {
    if !out.contains(&name) {
        out.push(name);
    }
}

fn linux_asset_candidates(arch: &str, distro: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let base = format!("glab-tui-linux-{arch}");

    match distro.unwrap_or("") {
        d if d.starts_with("ubuntu-") => {
            // Prefer the locally-matching Ubuntu build first, then walk down
            // through known LTS baselines, and finally the static musl build.
            push_unique(&mut out, format!("{base}-{d}.tar.gz"));
            for v in UBUNTU_LTS_FALLBACKS {
                if *v != d {
                    push_unique(&mut out, format!("{base}-{v}.tar.gz"));
                }
            }
            push_unique(&mut out, format!("{base}-musl.tar.gz"));
        }
        _ => {
            push_unique(&mut out, format!("{base}-ubuntu-22.04.tar.gz"));
            push_unique(&mut out, format!("{base}-ubuntu-24.04.tar.gz"));
            push_unique(&mut out, format!("{base}-musl.tar.gz"));
        }
    }
    out
}

fn asset_candidates(os: &str, arch: &str) -> Vec<String> {
    match os {
        "linux" => linux_asset_candidates(arch, read_linux_distro().as_deref()),
        "macos" => vec![format!("glab-tui-macos-{arch}.tar.gz")],
        "windows" => vec!["glab-tui-windows-amd64.zip".to_string()],
        _ => Vec::new(),
    }
}

fn arch_str(target_arch: &str) -> &str {
    match target_arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => "amd64",
    }
}

pub async fn perform_self_update() -> Result<bool> {
    let output = tokio::process::Command::new("gh")
        .args([
            "release",
            "view",
            "-R",
            "rcieri/glab-tui",
            "--json",
            "tagName",
        ])
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("Failed to check for latest release from GitHub");
    }

    let json: Value = serde_json::from_slice(&output.stdout)?;
    let latest_tag = json
        .get("tagName")
        .and_then(|v| v.as_str())
        .context("No tagName in release")?;

    let current_version = env!("CARGO_PKG_VERSION");
    let current_tag = format!("v{}", current_version);
    if latest_tag == current_tag {
        return Ok(false);
    }

    let target_os = std::env::consts::OS;
    let target_arch = std::env::consts::ARCH;
    let arch = arch_str(target_arch);

    let candidates = asset_candidates(target_os, arch);
    if candidates.is_empty() {
        anyhow::bail!("Unsupported operating system: {}", target_os);
    }

    let temp_dir = tempdir()?;

    let assets_output = tokio::process::Command::new("gh")
        .args([
            "release",
            "view",
            latest_tag,
            "-R",
            "rcieri/glab-tui",
            "--json",
            "assets",
        ])
        .output()
        .await?;
    if !assets_output.status.success() {
        anyhow::bail!("Failed to list release assets");
    }
    let assets_json: Value = serde_json::from_slice(&assets_output.stdout)?;
    let available: Vec<&str> = assets_json
        .get("assets")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                .collect()
        })
        .unwrap_or_default();

    let asset_name = candidates
        .iter()
        .find(|c| available.iter().any(|a| *a == c.as_str()))
        .cloned()
        .context(format!(
            "No matching release asset for this platform ({} {}). Available: {}",
            target_os,
            arch,
            available.join(", ")
        ))?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ubuntu_22_falls_back_to_22_04_asset() {
        let candidates = linux_asset_candidates("amd64", Some("ubuntu-22.04"));
        assert_eq!(candidates[0], "glab-tui-linux-amd64-ubuntu-22.04.tar.gz");
        assert!(candidates.contains(&"glab-tui-linux-amd64-musl.tar.gz".to_string()));
    }

    #[test]
    fn ubuntu_24_prefers_24_04_asset() {
        let candidates = linux_asset_candidates("amd64", Some("ubuntu-24.04"));
        assert_eq!(candidates[0], "glab-tui-linux-amd64-ubuntu-24.04.tar.gz");
        assert_eq!(candidates[1], "glab-tui-linux-amd64-ubuntu-22.04.tar.gz");
        assert_eq!(candidates[2], "glab-tui-linux-amd64-musl.tar.gz");
    }

    #[test]
    fn future_ubuntu_prefers_local_then_walks_down() {
        let candidates = linux_asset_candidates("amd64", Some("ubuntu-26.04"));
        assert_eq!(candidates[0], "glab-tui-linux-amd64-ubuntu-26.04.tar.gz");
        assert_eq!(candidates[1], "glab-tui-linux-amd64-ubuntu-24.04.tar.gz");
        assert_eq!(candidates[2], "glab-tui-linux-amd64-ubuntu-22.04.tar.gz");
        assert_eq!(candidates[3], "glab-tui-linux-amd64-musl.tar.gz");
    }

    #[test]
    fn unknown_distro_falls_back_to_22_04() {
        let candidates = linux_asset_candidates("arm64", Some("fedora-39"));
        assert_eq!(candidates[0], "glab-tui-linux-arm64-ubuntu-22.04.tar.gz");
        assert_eq!(candidates[1], "glab-tui-linux-arm64-ubuntu-24.04.tar.gz");
        assert_eq!(candidates[2], "glab-tui-linux-arm64-musl.tar.gz");
    }

    #[test]
    fn macos_and_windows_keep_legacy_names() {
        assert_eq!(
            asset_candidates("macos", "arm64"),
            vec!["glab-tui-macos-arm64.tar.gz".to_string()]
        );
        assert_eq!(
            asset_candidates("windows", "amd64"),
            vec!["glab-tui-windows-amd64.zip".to_string()]
        );
    }

    #[test]
    fn arch_str_maps_x86_and_aarch64() {
        assert_eq!(arch_str("x86_64"), "amd64");
        assert_eq!(arch_str("aarch64"), "arm64");
        assert_eq!(arch_str("mystery"), "amd64");
    }
}
