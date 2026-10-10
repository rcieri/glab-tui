use anyhow::{Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

const LATEST_RELEASE_API_URL: &str = "https://api.github.com/repos/rcieri/glab-tui/releases/latest";
const CHECKSUM_SUFFIX: &str = ".sha256";
const SHA256_HEX_LEN: usize = 64;

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

struct SelectedAsset<'a> {
    archive: &'a ReleaseAsset,
    checksum: &'a ReleaseAsset,
}

/// Picks the first candidate the release ships, and refuses a release that
/// does not publish its checksum: an unverifiable binary is never installed.
fn select_release_asset<'a>(
    release: &'a Release,
    candidates: &[String],
) -> Result<SelectedAsset<'a>> {
    let find = |name: &str| release.assets.iter().find(|a| a.name == name);
    let archive = candidates.iter().find_map(|c| find(c)).with_context(|| {
        let available: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
        format!(
            "no release asset matches this platform (tried {}); available: {}",
            candidates.join(", "),
            available.join(", ")
        )
    })?;
    let checksum_name = format!("{}{CHECKSUM_SUFFIX}", archive.name);
    let checksum = find(&checksum_name).with_context(|| {
        format!(
            "release {} publishes no {checksum_name}; refusing to install an unverified binary",
            release.tag_name
        )
    })?;
    Ok(SelectedAsset { archive, checksum })
}

/// Reads the digest from a `sha256sum`/`shasum` line: `<hex>  <name>`, or
/// `<hex> *<name>` when the checksum was generated in binary mode (Windows).
fn parse_published_sha256(checksum_file: &str, asset_name: &str) -> Result<String> {
    let mut fields = checksum_file.split_whitespace();
    let digest = fields
        .next()
        .with_context(|| format!("checksum file for {asset_name} is empty"))?;
    let named = fields.next().map(|name| name.trim_start_matches('*'));
    if named != Some(asset_name) {
        anyhow::bail!(
            "checksum file for {asset_name} names {}",
            named.unwrap_or("no file")
        );
    }
    if digest.len() != SHA256_HEX_LEN || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        anyhow::bail!("checksum file for {asset_name} holds no SHA-256 digest: {digest}");
    }
    Ok(digest.to_ascii_lowercase())
}

fn sha256_hex(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).with_context(|| format!("reading {}", path.display()))?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn verify_archive_checksum(archive: &Path, checksum_file: &str, asset_name: &str) -> Result<()> {
    let expected = parse_published_sha256(checksum_file, asset_name)?;
    let actual = sha256_hex(archive)?;
    if actual != expected {
        anyhow::bail!(
            "checksum mismatch for {asset_name}: expected sha256 {expected}, got {actual}"
        );
    }
    Ok(())
}

/// Runs `curl` and returns its stdout. The release lives on GitHub whatever
/// backend the user works with, and `glab` cannot reach it, so the updater
/// talks HTTPS through `curl` instead of requiring the `gh` CLI.
async fn curl(args: &[&std::ffi::OsStr]) -> Result<Vec<u8>> {
    let output = tokio::process::Command::new("curl")
        .args(["--silent", "--show-error", "--fail", "--location"])
        .args(args)
        .output()
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                anyhow::anyhow!("self-update needs `curl` on PATH")
            }
            _ => anyhow::Error::new(e).context("running curl"),
        })?;
    if !output.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(output.stdout)
}

async fn fetch_latest_release() -> Result<Release> {
    let body = curl(&[LATEST_RELEASE_API_URL.as_ref()])
        .await
        .context("checking the latest release on GitHub")?;
    serde_json::from_slice(&body).context("parsing the latest release from GitHub")
}

async fn download_asset(asset: &ReleaseAsset, destination: &Path) -> Result<()> {
    curl(&[
        "--output".as_ref(),
        destination.as_os_str(),
        asset.browser_download_url.as_ref(),
    ])
    .await
    .with_context(|| format!("downloading {}", asset.name))?;
    Ok(())
}

async fn fetch_asset_text(asset: &ReleaseAsset) -> Result<String> {
    let body = curl(&[asset.browser_download_url.as_ref()])
        .await
        .with_context(|| format!("downloading {}", asset.name))?;
    String::from_utf8(body).with_context(|| format!("{} is not text", asset.name))
}

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
    let release = fetch_latest_release().await?;
    let latest_tag = release.tag_name.as_str();

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

    let selected = select_release_asset(&release, &candidates)?;
    let asset_name = selected.archive.name.as_str();

    let temp_dir = tempdir()?;
    let archive_path = temp_dir.path().join(asset_name);
    download_asset(selected.archive, &archive_path).await?;
    let checksum_file = fetch_asset_text(selected.checksum).await?;
    verify_archive_checksum(&archive_path, &checksum_file, asset_name)?;

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

    fn find_file_recursive(dir: &std::path::Path, target_name: &str) -> Option<PathBuf> {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.file_name().map_or(false, |n| n == target_name) {
                    return Some(path);
                } else if path.is_dir() {
                    if let Some(found) = find_file_recursive(&path, target_name) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }

    let exe_filename = if target_os == "windows" {
        "glab-tui.exe"
    } else {
        "glab-tui"
    };
    let new_bin_path = find_file_recursive(&extract_dir, exe_filename).ok_or_else(|| {
        anyhow::anyhow!(
            "Extracted binary `{}` not found inside archive",
            exe_filename
        )
    })?;

    let current_exe = std::env::current_exe()?;

    let mut old_exe = current_exe.clone();
    old_exe.set_extension("old");
    let _ = fs::rename(&current_exe, &old_exe);

    let install_res = (|| -> Result<()> {
        fs::copy(&new_bin_path, &current_exe)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&current_exe)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&current_exe, perms)?;
        }
        Ok(())
    })();

    if let Err(e) = install_res {
        let _ = fs::rename(&old_exe, &current_exe);
        return Err(e).context("Failed to install update; original binary restored");
    }

    let _ = fs::remove_file(old_exe);

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn release_fixture(json: &str) -> Release {
        serde_json::from_str(json).expect("release fixture parses")
    }

    fn candidates(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn release_asset_selection_takes_first_shipped_candidate_with_its_checksum() {
        let release = release_fixture(
            r#"{"tag_name": "v9.9.9", "assets": [
                {"name": "glab-tui-linux-amd64-musl.tar.gz", "browser_download_url": "https://x/musl"},
                {"name": "glab-tui-linux-amd64-musl.tar.gz.sha256", "browser_download_url": "https://x/musl.sha256"},
                {"name": "glab-tui-linux-amd64-ubuntu-22.04.tar.gz", "browser_download_url": "https://x/2204"},
                {"name": "glab-tui-linux-amd64-ubuntu-22.04.tar.gz.sha256", "browser_download_url": "https://x/2204.sha256"}
            ]}"#,
        );
        let selected = select_release_asset(
            &release,
            &candidates(&[
                "glab-tui-linux-amd64-ubuntu-24.04.tar.gz",
                "glab-tui-linux-amd64-ubuntu-22.04.tar.gz",
                "glab-tui-linux-amd64-musl.tar.gz",
            ]),
        )
        .unwrap();
        assert_eq!(selected.archive.browser_download_url, "https://x/2204");
        assert_eq!(
            selected.checksum.browser_download_url,
            "https://x/2204.sha256"
        );
    }

    #[test]
    fn release_without_a_published_checksum_is_refused() {
        let release = release_fixture(
            r#"{"tag_name": "v9.9.9", "assets": [
                {"name": "glab-tui-macos-arm64.tar.gz", "browser_download_url": "https://x/mac"}
            ]}"#,
        );
        let err = select_release_asset(&release, &candidates(&["glab-tui-macos-arm64.tar.gz"]))
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            "release v9.9.9 publishes no glab-tui-macos-arm64.tar.gz.sha256; refusing to install an unverified binary"
        );
    }

    #[test]
    fn published_sha256_accepts_text_and_binary_mode_lines() {
        let asset = "glab-tui-linux-amd64-musl.tar.gz";
        assert_eq!(
            parse_published_sha256(&format!("{ABC_SHA256}  {asset}\n"), asset).unwrap(),
            ABC_SHA256
        );
        let windows_line = format!(
            "{} *glab-tui-windows-amd64.zip\r\n",
            ABC_SHA256.to_uppercase()
        );
        assert_eq!(
            parse_published_sha256(&windows_line, "glab-tui-windows-amd64.zip").unwrap(),
            ABC_SHA256
        );
    }

    #[test]
    fn published_sha256_for_another_asset_is_rejected() {
        let err = parse_published_sha256(
            &format!("{ABC_SHA256}  glab-tui-macos-arm64.tar.gz\n"),
            "glab-tui-linux-amd64-musl.tar.gz",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "checksum file for glab-tui-linux-amd64-musl.tar.gz names glab-tui-macos-arm64.tar.gz"
        );
    }

    #[test]
    fn published_sha256_must_be_a_full_hex_digest() {
        let asset = "a.tar.gz";
        assert_eq!(
            parse_published_sha256("deadbeef  a.tar.gz", asset)
                .unwrap_err()
                .to_string(),
            "checksum file for a.tar.gz holds no SHA-256 digest: deadbeef"
        );
        assert_eq!(
            parse_published_sha256("", asset).unwrap_err().to_string(),
            "checksum file for a.tar.gz is empty"
        );
    }

    #[test]
    fn archive_matching_its_published_checksum_verifies() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("a.tar.gz");
        fs::write(&archive, b"abc").unwrap();
        verify_archive_checksum(&archive, &format!("{ABC_SHA256}  a.tar.gz\n"), "a.tar.gz")
            .unwrap();
    }

    #[test]
    fn tampered_archive_fails_verification() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("a.tar.gz");
        fs::write(&archive, b"abd").unwrap();
        let err =
            verify_archive_checksum(&archive, &format!("{ABC_SHA256}  a.tar.gz\n"), "a.tar.gz")
                .unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "checksum mismatch for a.tar.gz: expected sha256 {ABC_SHA256}, got {}",
                sha256_hex(&archive).unwrap()
            )
        );
    }

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
