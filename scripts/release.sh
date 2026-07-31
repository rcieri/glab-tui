#!/usr/bin/env bash
set -euo pipefail

# End-to-end release orchestrator for glab-tui.
#
# Usage: scripts/release.sh [patch|minor|major]   (default: patch)
#
# Walks the whole release: bumps the crate version, regenerates docs and demo
# GIFs locally (where `gh` is authenticated), opens a prepare PR, waits for you
# to review it, squash-merges it, tags and pushes the version, waits for the CI
# release build, then writes the release notes and pushes the Homebrew formula,
# Scoop manifest, Docker image, and crate.

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

REPO="rcieri/glab-tui"
INCREMENT="${1:-patch}"
RELEASE_WAIT_MIN="${RELEASE_WAIT_MIN:-45}"
REQUIRED_ASSETS=(
  glab-tui-linux-amd64.tar.gz
  glab-tui-linux-arm64.tar.gz
  glab-tui-macos-amd64.tar.gz
  glab-tui-macos-arm64.tar.gz
  glab-tui-windows-amd64.zip
)

die() { printf 'error: %s\n' "$*" >&2; exit 1; }
require() { command -v "$1" >/dev/null 2>&1 || die "missing required tool '$1' (${2:-})"; }
note() { printf '\n==> %s\n' "$*"; }

# ---------------------------------------------------------------------------
# Phase 0: preflight checks
# ---------------------------------------------------------------------------
preflight() {
  [[ -t 0 ]] || die "release.sh is interactive; run it in a terminal"
  require gh "see https://cli.github.com"
  require opencode "install from https://opencode.ai"
  require cargo "install Rust via https://rustup.rs"
  require jq "apt install jq / brew install jq"
  require vhs "go install github.com/charmbracelet/vhs@latest"
  require ttyd "apt install ttyd / brew install ttyd"
  require ffmpeg "apt install ffmpeg / brew install ffmpeg"
  require unzip "apt install unzip"
  gh auth status >/dev/null 2>&1 || die "not authenticated with gh; run 'gh auth login' first"
  fc-list 2>/dev/null | grep -qi "JetBrainsMono.*Nerd" || \
    die "JetBrainsMono Nerd Font not installed (download from https://github.com/ryanoasis/nerd-fonts)"
}

# ---------------------------------------------------------------------------
# Phase 1: determine next version and prepare the release PR
# ---------------------------------------------------------------------------
next_version() {
  git fetch --tags --prune
  local latest_tag version major minor patch
  latest_tag="$(git describe --tags --abbrev=0 2>/dev/null || echo v0.0.0)"
  version="${latest_tag#v}"
  IFS='.' read -r major minor patch <<< "$version"
  case "$INCREMENT" in
    major) major=$((major + 1)); minor=0; patch=0 ;;
    minor) minor=$((minor + 1)); patch=0 ;;
    patch) patch=$((patch + 1)) ;;
    *) die "invalid version increment '$INCREMENT' (expected patch|minor|major)" ;;
  esac
  VERSION="$major.$minor.$patch"
  NEW_TAG="v$VERSION"
  note "Latest tag: $latest_tag  next version: $NEW_TAG"
}

bump_cargo_version() {
  note "Bumping Cargo.toml to version $VERSION"
  awk -v v="$VERSION" '
    BEGIN { in_pkg = 0 }
    /^\[package\]/ { in_pkg = 1 }
    /^\[/ && !/^\[package\]/ { in_pkg = 0 }
    in_pkg && /^version[[:space:]]*=/ { sub(/=.*/, "= \"" v "\""); print; next }
    { print }
  ' Cargo.toml > Cargo.toml.new && mv Cargo.toml.new Cargo.toml
}

prepare() {
  BRANCH="opencode-release/$NEW_TAG"
  if git show-ref --verify --quiet "refs/heads/$BRANCH"; then
    git checkout "$BRANCH"
  elif git ls-remote --exit-code --quiet origin "refs/heads/$BRANCH" 2>/dev/null; then
    git checkout -b "$BRANCH" "origin/$BRANCH"
  else
    git checkout -b "$BRANCH"
  fi

  bump_cargo_version
  note "Building release binary..."
  cargo build --release

  note "Regenerating CHANGELOG.md / AGENTS.md / README.md via opencode..."
  PROMPT="We are prepping a new repository release. The upcoming version tag is going to be: $NEW_TAG.

Your task is to analyze the git commits, merged pull requests, and codebase changes since the last version tag, and update the following three files directly in the workspace:

1. CHANGELOG.md: Prepend a beautifully structured, developer-friendly update section for version $NEW_TAG at the top of the file, cleanly breaking down Features, Bug Fixes, and Maintenance.
2. AGENTS.md: Update any agent guidelines, automation logs, or architecture schemas affected by our latest feature set or dependencies. Ensure versioning matrices match $NEW_TAG.
3. README.md: Scan for installation commands, setup instructions, or documentation badges displaying the old version string, and replace them cleanly with version $NEW_TAG.

The crate version in Cargo.toml and Cargo.lock has already been bumped to $VERSION; do not modify those files. Save and write these file modifications directly back into the working directory."

  MODEL_ARGS=()
  if [[ -n "${OPENCODE_MODEL:-}" ]]; then
    MODEL_ARGS=(--model "$OPENCODE_MODEL")
  fi
  opencode run --auto "${MODEL_ARGS[@]}" "$PROMPT"

  note "Generating demo GIFs..."
  export PATH="$ROOT/target/release:$PATH"
  "$ROOT/assets/generate-demos.sh"

  git add CHANGELOG.md AGENTS.md README.md Cargo.toml Cargo.lock assets/demo-*.gif
  if ! git diff --cached --quiet; then
    git commit -m "chore: prepare release $NEW_TAG"
  fi
  git push -u origin "$BRANCH"

  PR_NUMBER="$(gh pr list --repo "$REPO" --head "$BRANCH" --state open --json number --jq '.[0].number' 2>/dev/null || true)"
  if [[ -z "$PR_NUMBER" || "$PR_NUMBER" == "null" ]]; then
    note "Opening release preparation PR..."
    PR_URL="$(gh pr create --repo "$REPO" --base main --head "$BRANCH" \
      --title "chore: prepare release $NEW_TAG" \
      --body "Automated release preparation for **$NEW_TAG**.

Regenerated CHANGELOG.md, AGENTS.md, README.md, and demo GIFs. Bumped the crate version to $VERSION.

Review, then this script will merge and cut the release.")"
    PR_NUMBER="$(basename "$PR_URL")"
  else
    note "Reusing existing PR #$PR_NUMBER"
  fi
  PR_URL="https://github.com/$REPO/pull/$PR_NUMBER"
  note "Release preparation PR: $PR_URL"
}

# ---------------------------------------------------------------------------
# Phase 2: wait for review, then merge and tag
# ---------------------------------------------------------------------------
review_gate() {
  note "Pause for review"
  read -r -p "Review the PR (CI checks run in the background). Press Enter to squash-merge and continue the release... "
}

merge_and_tag() {
  note "Merging PR #$PR_NUMBER (squash, auto-merge when checks pass)..."
  if ! gh pr merge "$PR_NUMBER" --repo "$REPO" --squash --auto 2>/dev/null; then
    local state
    state="$(gh pr view "$PR_NUMBER" --repo "$REPO" --json state --jq '.state')"
    [[ "$state" == "MERGED" ]] || die "failed to merge PR #$PR_NUMBER (conflicts? not mergeable?)"
  fi
  for i in $(seq 1 120); do
    if [[ "$(gh pr view "$PR_NUMBER" --repo "$REPO" --json state --jq '.state')" == "MERGED" ]]; then
      note "PR #$PR_NUMBER merged."
      break
    fi
    [[ $i -eq 120 ]] && die "timed out waiting for PR #$PR_NUMBER to merge"
    sleep 10
  done

  note "Tagging $NEW_TAG on the merge commit..."
  local merge_sha
  merge_sha="$(gh pr view "$PR_NUMBER" --repo "$REPO" --json mergeCommit --jq '.mergeCommit.oid')"
  git fetch origin main
  git tag "$NEW_TAG" "$merge_sha"
  git push origin "$NEW_TAG"
  note "Tag pushed; release build running: https://github.com/$REPO/actions"
}

# ---------------------------------------------------------------------------
# Phase 3: wait for the CI release build
# ---------------------------------------------------------------------------
wait_for_release() {
  local total i current
  total=$((RELEASE_WAIT_MIN * 3)) # one check every 20s
  note "Waiting for release $NEW_TAG assets (timeout ${RELEASE_WAIT_MIN}m)..."
  for i in $(seq 1 "$total"); do
    current="$(gh release view "$NEW_TAG" --repo "$REPO" --json assets --jq '[.assets[].name] | length' 2>/dev/null || echo 0)"
    if [[ "$current" -ge "${#REQUIRED_ASSETS[@]}" ]]; then
      note "All ${#REQUIRED_ASSETS[@]} release assets present."
      return 0
    fi
    [[ $i -eq $total ]] && die "timed out waiting for release assets for $NEW_TAG"
    sleep 20
  done
}

# ---------------------------------------------------------------------------
# Phase 4: post-release (notes, Homebrew, Scoop)
# ---------------------------------------------------------------------------
update_homebrew() {
  local arch file sha macos_amd64 macos_arm64 linux_amd64 linux_arm64
  gh repo clone rcieri/homebrew-glab-tui "$TMP_DIR/homebrew-glab-tui" >/dev/null
  cd "$TMP_DIR/homebrew-glab-tui"

  for arch in macos-amd64 macos-arm64 linux-amd64 linux-arm64; do
    file="$TMP_DIR/glab-tui-${arch}.tar.gz"
    note "Fetching glab-tui-${arch}.tar.gz..."
    curl -sL "https://github.com/$REPO/releases/download/$NEW_TAG/glab-tui-${arch}.tar.gz" -o "$file"
    sha="$(sha256sum "$file" | cut -d' ' -f1)"
    case "$arch" in
      macos-amd64) macos_amd64=$sha ;;
      macos-arm64) macos_arm64=$sha ;;
      linux-amd64) linux_amd64=$sha ;;
      linux-arm64) linux_arm64=$sha ;;
    esac
  done

  sed -i "s|/download/v[0-9.]*/glab-tui-|/download/${NEW_TAG}/glab-tui-|g" Formula/glab-tui.rb
  sed -i "/glab-tui-macos-amd64/,/sha256/{s/sha256 \"[a-f0-9]*\"/sha256 \"${macos_amd64}\"/}" Formula/glab-tui.rb
  sed -i "/glab-tui-macos-arm64/,/sha256/{s/sha256 \"[a-f0-9]*\"/sha256 \"${macos_arm64}\"/}" Formula/glab-tui.rb
  sed -i "/glab-tui-linux-amd64/,/sha256/{s/sha256 \"[a-f0-9]*\"/sha256 \"${linux_amd64}\"/}" Formula/glab-tui.rb
  sed -i "/glab-tui-linux-arm64/,/sha256/{s/sha256 \"[a-f0-9]*\"/sha256 \"${linux_arm64}\"/}" Formula/glab-tui.rb

  git add Formula/glab-tui.rb
  if git diff --cached --quiet; then
    note "Homebrew formula already up to date"
  else
    git -c user.name="opencode-release[bot]" \
        -c user.email="opencode-release[bot]@users.noreply.github.com" \
        commit -m "Update to ${NEW_TAG}" >/dev/null
    git push
    note "Homebrew formula updated and pushed"
  fi
  cd "$ROOT"
}

update_scoop() {
  local version sha
  gh repo clone rcieri/scoop-glab-tui "$TMP_DIR/scoop-glab-tui" >/dev/null
  cd "$TMP_DIR/scoop-glab-tui"

  version="${NEW_TAG#v}"
  note "Fetching glab-tui-windows-amd64.zip..."
  curl -sL "https://github.com/$REPO/releases/download/$NEW_TAG/glab-tui-windows-amd64.zip" -o "$TMP_DIR/glab-tui-windows-amd64.zip"
  sha="$(sha256sum "$TMP_DIR/glab-tui-windows-amd64.zip" | cut -d' ' -f1)"

  jq --arg v "$version" --arg sha "$sha" \
    '.version = $v | .architecture."64bit".url = "https://github.com/rcieri/glab-tui/releases/download/v\($v)/glab-tui-windows-amd64.zip" | .architecture."64bit".hash = $sha' \
    bucket/glab-tui.json > bucket/glab-tui.json.tmp
  mv bucket/glab-tui.json.tmp bucket/glab-tui.json

  git add bucket/glab-tui.json
  if git diff --cached --quiet; then
    note "Scoop manifest already up to date"
  else
    git -c user.name="opencode-release[bot]" \
        -c user.email="opencode-release[bot]@users.noreply.github.com" \
        commit -m "Update to ${NEW_TAG}" >/dev/null
    git push
    note "Scoop manifest updated and pushed"
  fi
  cd "$ROOT"
}

post_release() {
  local prev_tag prompt
  prev_tag="$(git describe --tags --abbrev=0 "$NEW_TAG^" 2>/dev/null || git describe --tags --abbrev=0 2>/dev/null || true)"
  [[ -n "$prev_tag" ]] || die "could not determine the previous tag before $NEW_TAG"

  note "Generating RELEASE_NOTES.md via opencode..."
  prompt="Read CHANGELOG.md and extract the section for version $NEW_TAG.

Also read the existing release notes for the previous tag $prev_tag (use \`gh release view $prev_tag --json body --jq .body\`) to match their formatting style.

Write the file RELEASE_NOTES.md matching the same format:
- Title \"## What's Changed\"
- Sections: ### Added / ### Fixed / ### Changed / ### Dependencies
- Entries start with bolded headline: \`- **Name** — Description with references (#123).\`
- End with: \`**Full Changelog**: https://github.com/rcieri/glab-tui/compare/$prev_tag...$NEW_TAG\`

Use the content from CHANGELOG.md for the current version as the source material."

  MODEL_ARGS=()
  if [[ -n "${OPENCODE_MODEL:-}" ]]; then
    MODEL_ARGS=(--model "$OPENCODE_MODEL")
  fi
  opencode run --auto "${MODEL_ARGS[@]}" "$prompt"
  [[ -f RELEASE_NOTES.md ]] || die "RELEASE_NOTES.md was not generated"

  note "Updating release $NEW_TAG body..."
  gh release edit "$NEW_TAG" --repo "$REPO" --notes-file RELEASE_NOTES.md

  update_homebrew
  update_scoop
}

# ---------------------------------------------------------------------------
# Phase 5: publish (Docker image + crate)
# ---------------------------------------------------------------------------
publish() {
  local package_version tag_version user
  package_version="$(cargo metadata --format-version 1 --no-deps 2>/dev/null | jq -r '.packages[0].version')"
  tag_version="${NEW_TAG#v}"
  if [[ "$package_version" != "$tag_version" ]]; then
    die "Cargo package version ($package_version) does not match tag version ($tag_version)"
  fi

  note "Pushing Docker image to GHCR..."
  require docker "see https://docs.docker.com/get-docker/"
  user="$(gh api user --jq .login)"
  gh auth token | docker login ghcr.io -u "$user" --password-stdin
  local tags_args=(-t "ghcr.io/$REPO:$NEW_TAG")
  if [[ "$NEW_TAG" != *-* ]]; then
    tags_args+=(-t "ghcr.io/$REPO:latest")
  fi
  docker buildx build --push "${tags_args[@]}" .

  note "Publishing crate v$package_version to crates.io..."
  cargo publish --locked
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------
main() {
  preflight
  next_version
  prepare
  review_gate
  merge_and_tag
  wait_for_release
  TMP_DIR="$(mktemp -d)"
  trap 'rm -rf "$TMP_DIR"' EXIT
  post_release
  publish
  git checkout main 2>/dev/null || true
  git branch -D "$BRANCH" 2>/dev/null || true
  note "Release $NEW_TAG complete."
}

main "$@"
