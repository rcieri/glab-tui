# Release Process (Local-First)

Releases are prepared, documented, and distributed from a maintainer's machine via a single orchestrator, `scripts/release.sh`. CI is only responsible for building the cross-platform release binaries. The demo GIFs must be recorded locally because `glab-tui` shells out to `gh`/`glab`, and CI tokens lack the permissions for a realistic recording.

Run `scripts/release.sh [patch|minor|major|nightly]` (default `patch`) and the script walks the full release:

> **Nightly Pre-Releases:** `scripts/release.sh nightly` (or tags matching `vX.Y.Z-nightly[-suffix]`) runs a stripped-down flow that skips `Cargo.toml` bumping, docs regeneration, prepare PRs, demo GIFs, review gates, Homebrew/Scoop manifest syncs, and crates.io publishing, while still building matrix binaries and updating GitHub release notes.

1. **Preflight** — checks `gh`/`opencode`/`cargo`/`jq`/`vhs`/`ttyd`/`ffmpeg`/`unzip`, `gh auth`, JetBrainsMono Nerd Font, and push access to both manifest repos (`rcieri/homebrew-glab-tui`, `rcieri/scoop-glab-tui`); exits non-zero with a clear message if a prerequisite is missing. Long-running steps run under the script's `spinner`/`progress_bar` helpers (animated spinner with captured logs, auto-disabled when not a TTY), and phases are numbered `1/7` … for progress reporting.
2. **Prepare** — computes the next tag from `git describe --tags`, bumps the crate version in `Cargo.toml`, prompts for the AI tool (`agy` or `opencode`) and model unless configured via `AI_TOOL`, `AGY_MODEL`, or `OPENCODE_MODEL`, regenerates `CHANGELOG.md`/`README.md` via the selected AI tool, rebuilds the demo GIFs against an authenticated `gh`, and opens a `chore: prepare release vX.Y.Z` PR.
3. **Review gate** — pauses for the maintainer to review the PR (CI checks run in the background); the script continues on Enter.
4. **Merge & tag** — squash-merges the PR with `--auto`, tags the merge commit and pushes `vX.Y.Z`. `.github/workflows/release.yml` builds the 11-target binary matrix (8 Linux + 3 non-Linux) and uploads them to the GitHub release.
5. **Wait for build** — polls the release until every required asset exists (timeout: `RELEASE_WAIT_MIN`, default 45 min). The required-asset list in `REQUIRED_ASSETS_STATIC` covers the fixed name patterns; the dynamic Ubuntu-latest build is required to produce at least two `ubuntu-<VERSION_ID>` assets whose VERSION_ID differs from the 22.04 / 24.04 baselines. The script protects against unbound variables and off-by-one asset-count math under `set -u`, and it can resume from a chosen phase (`--phase`) if an earlier step failed or was interrupted.
6. **Post-release** — generates `RELEASE_NOTES.md` via the selected AI tool (entries attribute their contributors as `(thanks @username)` and a `**Contributors**` section lists all `@username` handles since the previous tag), and updates the release body on GitHub.
7. **Publish** — pushes the Homebrew formula (`rcieri/homebrew-glab-tui`), Scoop manifest (`rcieri/scoop-glab-tui`), Docker image to GHCR, and publishes the crate to crates.io (skipped for nightly).

## Release Assets

`.github/workflows/release.yml` builds an 11-entry matrix (8 Linux + 3 non-Linux) on every `v*` tag push. **Linux builds target a glibc 2.35 baseline (Ubuntu 22.04) for backwards compatibility with older distros, plus per-LTS and `ubuntu-latest` variants for current distros, and a fully static musl fallback for non-glibc systems.** The `ubuntu-latest` runner's `VERSION_ID` is detected at build time and baked into the asset name (e.g. `glab-tui-linux-amd64-ubuntu-26.04.tar.gz` when `ubuntu-latest` resolves to Ubuntu 26.04), so the asset set adapts to future LTS releases without code changes.

Asset name pattern:

| OS | Arch | Asset name | Built on |
|---|---|---|---|
| Linux | x86_64 | `glab-tui-linux-amd64-ubuntu-22.04.tar.gz` | `ubuntu-22.04` runner (glibc 2.35 baseline) |
| Linux | x86_64 | `glab-tui-linux-amd64-ubuntu-24.04.tar.gz` | `ubuntu-24.04` runner (glibc 2.39) |
| Linux | x86_64 | `glab-tui-linux-amd64-ubuntu-<VERSION_ID>.tar.gz` | `ubuntu-latest` runner (current LTS) |
| Linux | aarch64 | `glab-tui-linux-arm64-ubuntu-{22.04,24.04,<VERSION_ID>}.tar.gz` | cross-compiled from `ubuntu-{22.04,24.04,latest}` x86_64 runners |
| Linux | x86_64 / aarch64 | `glab-tui-linux-{amd64,arm64}-musl.tar.gz` | `ubuntu-22.04` (static, no glibc dependency) |
| macOS | x86_64 / arm64 | `glab-tui-macos-{amd64,arm64}.tar.gz` | `macos-latest` |
| Windows | x86_64 | `glab-tui-windows-amd64.zip` | `windows-latest` |

The asset-suffix table in `release.yml` is the single source of truth — `linux_variant: ubuntu-latest` causes the build step to read `/etc/os-release` and bake `VERSION_ID` into the asset name.

### Asset selection logic

Three places pick the right asset for the running host:

- **`install.sh`** (`linux_asset_candidates`) — builds a candidate list from `/etc/os-release`. On Ubuntu it tries the local version first, then walks down through `ubuntu-24.04` → `ubuntu-22.04`, then falls back to `musl`. Non-Ubuntu Linux distros try `ubuntu-22.04` → `ubuntu-24.04` → `musl` directly. Override with `GLAB_TUI_ASSET=glab-tui-linux-amd64-musl.tar.gz` for testing.
- **`src/utils/update.rs`** (`linux_asset_candidates`) — mirrors `install.sh` for the in-app self-updater; queries the GitHub release's asset list through `curl` and picks the first match.
- **Homebrew formula** (`update_homebrew` in `release.sh`) — pins the fully static musl builds for Linux (x86_64 and arm64), which run on any Linux distro regardless of glibc version, and native macOS binaries.

Every install path verifies the downloaded archive against the release's `<asset>.sha256` (generated by `release.yml`) before extracting it, and refuses to install when the checksum asset is missing or does not match: `install.sh` (`verify_checksum`, via `sha256sum` or `shasum`), `install.ps1` (`Get-FileHash`) and the self-updater (`verify_archive_checksum`). Checksum lines are `<hex>  <name>`, or `<hex> *<name>` for the Windows zip.

Scoop's manifest (`rcieri/scoop-glab-tui`) is unaffected — Windows keeps the fixed `glab-tui-windows-amd64.zip` name.

During `Prepare` and `Post-release`, `release.sh` prompts to select the AI tool (`agy` or `opencode`) and model for the regenerated `CHANGELOG.md`/`README.md` and release notes; set `AI_TOOL` and `AGY_MODEL` / `OPENCODE_MODEL` to skip prompts.
