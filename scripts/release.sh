#!/usr/bin/env bash
set -euo pipefail

# End-to-end release orchestrator for glab-tui.
#
# Usage: scripts/release.sh [patch|minor|major]
#
# With no argument, you are prompted to pick the release increment (patch is
# the default). You are also prompted to pick the opencode model used for the
# regenerated docs and release notes (the `opencode models` printout piped
# through fzf; set OPENCODE_MODEL to skip the prompt). Walks the whole
# release: bumps the crate version, regenerates docs and demo GIFs locally
# (where `gh` is authenticated), opens a prepare PR, waits for you to review
# it, squash-merges it, tags and pushes the version, waits for the CI release
# build, then writes the release notes and pushes the Homebrew formula, Scoop
# manifest, Docker image, and crate.

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

REPO="rcieri/glab-tui"
INCREMENT="${1:-}"
RELEASE_WAIT_MIN="${RELEASE_WAIT_MIN:-45}"
OPENCODE_MODEL_FROM_ENV="${OPENCODE_MODEL:-}"
OPENCODE_MODEL="${OPENCODE_MODEL:-opencode/big-pickle}"
REQUIRED_ASSETS=(
  glab-tui-linux-amd64.tar.gz
  glab-tui-linux-arm64.tar.gz
  glab-tui-macos-amd64.tar.gz
  glab-tui-macos-arm64.tar.gz
  glab-tui-windows-amd64.zip
)

# ---------------------------------------------------------------------------
# colors & output helpers (auto-disabled when not a TTY or NO_COLOR is set)
# ---------------------------------------------------------------------------
if [[ -t 1 ]] && [[ -z "${NO_COLOR:-}" ]]; then
  C_BOLD=$'\e[1m'; C_DIM=$'\e[2m'; C_RED=$'\e[31m'
  C_GREEN=$'\e[32m'; C_YELLOW=$'\e[33m'; C_CYAN=$'\e[36m'; C_RESET=$'\e[0m'
else
  C_BOLD='' C_DIM='' C_RED='' C_GREEN='' C_YELLOW='' C_CYAN='' C_RESET=''
fi

die() { printf '%serror:%s %s\n' "${C_BOLD}${C_RED}" "$C_RESET" "$*" >&2; exit 1; }
require() { command -v "$1" >/dev/null 2>&1 || die "missing required tool '$1' (${2:-})"; }
note() { printf '\n%s==>%s %s\n' "${C_BOLD}${C_CYAN}" "$C_RESET" "$*"; }
ok() { printf '%s✓%s %s\n' "$C_GREEN" "$C_RESET" "$*"; }
phase() { printf '\n%s── [ %s ] ──%s\n' "${C_BOLD}${C_YELLOW}" "$*" "$C_RESET"; }
banner() {
  printf '\n%s============================================%s\n' "${C_BOLD}${C_CYAN}" "$C_RESET"
  printf '%s  glab-tui release orchestrator%s\n' "${C_BOLD}" "$C_RESET"
  printf '%s============================================%s\n' "${C_BOLD}${C_CYAN}" "$C_RESET"
}

run_opencode() {
  note "opencode ($OPENCODE_MODEL), output logged to $TMP_DIR/opencode.log"
  if ! opencode run --auto --model "$OPENCODE_MODEL" "$1" >"$TMP_DIR/opencode.log" 2>&1; then
    tail -20 "$TMP_DIR/opencode.log" >&2
    die "opencode failed (log: $TMP_DIR/opencode.log)"
  fi
}

# ---------------------------------------------------------------------------
# opencode model selection (fzf over the `opencode models` printout)
# ---------------------------------------------------------------------------
PICK_RESULT=''

# pick <prompt> <default> <candidate...>; each candidate is "value<TAB>label".
# Stores the chosen value in PICK_RESULT (defaults when nothing is picked).
pick() {
  local prompt="$1" default="$2"
  shift 2
  local -a lines=("$@")
  local chosen="" i choice
  if command -v fzf >/dev/null 2>&1; then
    chosen="$(printf '%s\n' "${lines[@]}" |
      fzf --prompt="$prompt> " --query="$default" --delimiter=$'\t' --with-nth=2 \
          --exit-0 --height=40% --border --layout=reverse 2>/dev/null || true)"
  else
    printf '\n%sChoose %s%s (default: %s)\n' "$C_BOLD" "$prompt" "$C_RESET" "$default"
    for i in "${!lines[@]}"; do
      printf '  %s%s)%s %s\n' "$C_BOLD" "$((i + 1))" "$C_RESET" "${lines[$i]#*$'\t'}"
    done
    read -r -p "Select [1-${#lines[@]}], Enter for default: " choice
    if [[ -z "$choice" ]]; then
      chosen="$default"
    elif [[ "$choice" =~ ^[0-9]+$ ]] && ((choice >= 1 && choice <= ${#lines[@]})); then
      chosen="${lines[$((choice - 1))]}"
    else
      die "invalid selection '$choice'"
    fi
  fi
  PICK_RESULT="${chosen%%$'\t'*}"
  if [[ -z "$PICK_RESULT" ]]; then
    PICK_RESULT="$default"
  fi
}

select_opencode_model() {
  local all_models selected current
  local -a model_lines=()

  all_models="$(opencode models)"
  [[ -n "$all_models" ]] || die "'opencode models' returned no models"
  current="${OPENCODE_MODEL:-opencode/big-pickle}"

  note "Select the opencode model used to regenerate docs and release notes"
  while read -r id; do
    model_lines+=("$id"$'\t'"$id")
  done <<< "$all_models"
  pick "model" "$current" "${model_lines[@]}"
  selected="$PICK_RESULT"

  OPENCODE_MODEL="$selected"
  grep -qxF "$OPENCODE_MODEL" <<< "$all_models" || \
    die "'$OPENCODE_MODEL' is not listed by 'opencode models'"
  ok "opencode model: $OPENCODE_MODEL"
}

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
  for repo in rcieri/homebrew-glab-tui rcieri/scoop-glab-tui; do
    gh api "repos/$repo" --jq '.permissions.push' | grep -q true || \
      die "no push access to $repo; grant your token write permission"
  done
  fc-list 2>/dev/null | grep -qi "JetBrainsMono.*Nerd" || \
    die "JetBrainsMono Nerd Font not installed (download from https://github.com/ryanoasis/nerd-fonts)"
}

# ---------------------------------------------------------------------------
# Phase 1: determine next version and prepare the release PR
# ---------------------------------------------------------------------------
next_version() {
  git fetch --tags --prune
  local latest_tag version base_major base_minor base_patch
  local major_v minor_v patch_v
  latest_tag="$(git describe --tags --abbrev=0 2>/dev/null || echo v0.0.0)"
  version="${latest_tag#v}"
  IFS='.' read -r base_major base_minor base_patch <<< "$version"

  major_v="$((base_major + 1)).0.0"
  minor_v="$base_major.$((base_minor + 1)).0"
  patch_v="$base_major.$base_minor.$((base_patch + 1))"

  if [[ -z "$INCREMENT" ]]; then
    printf '\n%sCurrent version:%s %s\n' "$C_BOLD" "$C_RESET" "$latest_tag"
    printf '  %s1)%s patch  -> v%s\n' "$C_BOLD" "$C_RESET" "$patch_v"
    printf '  %s2)%s minor  -> v%s\n' "$C_BOLD" "$C_RESET" "$minor_v"
    printf '  %s3)%s major  -> v%s\n' "$C_BOLD" "$C_RESET" "$major_v"
    read -r -p "Select release increment [1/2/3] (default patch): " choice
    case "${choice:-1}" in
      1|patch) VERSION="$patch_v" ;;
      2|minor) VERSION="$minor_v" ;;
      3|major) VERSION="$major_v" ;;
      *) die "invalid selection '$choice'" ;;
    esac
  else
    case "$INCREMENT" in
      major) VERSION="$major_v" ;;
      minor) VERSION="$minor_v" ;;
      patch) VERSION="$patch_v" ;;
      *) die "invalid version increment '$INCREMENT' (expected patch|minor|major)" ;;
    esac
  fi

  NEW_TAG="v$VERSION"
  note "Next version: $NEW_TAG"
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

  run_opencode "$PROMPT"
  ok "CHANGELOG.md / AGENTS.md / README.md regenerated"

  note "Generating demo GIFs..."
  export PATH="$ROOT/target/release:$PATH"
  "$ROOT/assets/generate-demos.sh"
  ok "demo GIFs regenerated"

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
  ok "Release preparation PR: $PR_URL"
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
      ok "PR #$PR_NUMBER merged"
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
  ok "tag $NEW_TAG pushed; release build: https://github.com/$REPO/actions"
}

# ---------------------------------------------------------------------------
# Phase 3: wait for the CI release build
# ---------------------------------------------------------------------------
wait_for_release() {
  local total i current elapsed
  total=$((RELEASE_WAIT_MIN * 3)) # one check every 20s
  note "Waiting for release $NEW_TAG assets (timeout ${RELEASE_WAIT_MIN}m)..."
  for i in $(seq 1 "$total"); do
    current="$(gh release view "$NEW_TAG" --repo "$REPO" --json assets --jq '[.assets[].name] | length' 2>/dev/null || echo 0)"
    if [[ "$current" -ge "${#REQUIRED_ASSETS[@]}" ]]; then
      [[ -t 1 ]] && printf '\r\033[2K'
      ok "All ${#REQUIRED_ASSETS[@]} release assets present"
      return 0
    fi
    [[ $i -eq $total ]] && die "timed out waiting for release assets for $NEW_TAG"
    if [[ -t 1 ]]; then
      elapsed=$((i * 20 / 60))
      printf '\r  %smin elapsed - %s/%s assets...' "$elapsed" "$current" "${#REQUIRED_ASSETS[@]}"
    fi
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
    ok "Homebrew formula updated and pushed"
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
    ok "Scoop manifest updated and pushed"
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
- Attribute each entry to its contributor by appending \"(thanks @username)\" where the author can be determined from the PR/commit metadata (e.g. \`- **Name** — Description (#123) — thanks @username\`).
- End with a \`**Contributors**\` section listing every contributor since $prev_tag as a markdown list of \`@username\` handles, ordered by number of contributions.
- End with: \`**Full Changelog**: https://github.com/rcieri/glab-tui/compare/$prev_tag...$NEW_TAG\`

Use the content from CHANGELOG.md for the current version as the source material."

  run_opencode "$prompt"
  [[ -f RELEASE_NOTES.md ]] || die "RELEASE_NOTES.md was not generated"
  ok "RELEASE_NOTES.md generated"

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
  ok "Docker image pushed to GHCR"

  note "Publishing crate v$package_version to crates.io..."
  cargo publish --locked
  ok "crate v$package_version published to crates.io"
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------
main() {
  TMP_DIR="$(mktemp -d)"
  trap 'rm -rf "$TMP_DIR"' EXIT
  banner

  phase "Preflight"
  preflight

  phase "Prepare"
  next_version
  if [[ -z "$OPENCODE_MODEL_FROM_ENV" ]]; then
    select_opencode_model
  else
    ok "using OPENCODE_MODEL from environment: $OPENCODE_MODEL"
  fi
  prepare

  phase "Review & merge"
  review_gate
  merge_and_tag

  phase "Wait for CI build"
  wait_for_release

  phase "Post-release"
  post_release

  phase "Publish"
  publish

  git checkout main 2>/dev/null || true
  git branch -D "$BRANCH" 2>/dev/null || true
  ok "Release $NEW_TAG complete."
}

main "$@"
