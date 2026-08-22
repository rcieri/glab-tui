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
#
# You may resume from any phase by answering the "start from" prompt that
# appears after the banner. Phases that require state set by earlier phases
# (version tag, PR number, ...) will prompt for those values if they were
# skipped.

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

REPO="rcieri/glab-tui"
INCREMENT="${1:-}"
RELEASE_WAIT_MIN="${RELEASE_WAIT_MIN:-45}"
OPENCODE_MODEL_FROM_ENV="${OPENCODE_MODEL:-}"
OPENCODE_MODEL="${OPENCODE_MODEL:-opencode/big-pickle}"
REQUIRED_ASSETS_STATIC=(
  glab-tui-linux-amd64-ubuntu-22.04.tar.gz
  glab-tui-linux-amd64-ubuntu-24.04.tar.gz
  glab-tui-linux-arm64-ubuntu-22.04.tar.gz
  glab-tui-linux-arm64-ubuntu-24.04.tar.gz
  glab-tui-linux-amd64-musl.tar.gz
  glab-tui-linux-arm64-musl.tar.gz
  glab-tui-macos-amd64.tar.gz
  glab-tui-macos-arm64.tar.gz
  glab-tui-windows-amd64.zip
)

# The ubuntu-latest runner builds produce ubuntu-<VERSION_ID> assets whose
# version is unknown at release-script-run time (could be 26.04, 28.04, …).
# We require at least one such asset per arch in addition to REQUIRED_ASSETS_STATIC.
REQUIRED_ASSETS_DYNAMIC_GLOB='glab-tui-linux-{amd64,arm64}-ubuntu-*'

# ---------------------------------------------------------------------------
# colors & output helpers (auto-disabled when not a TTY or NO_COLOR is set)
# ---------------------------------------------------------------------------
if [[ -t 1 ]] && [[ -z "${NO_COLOR:-}" ]]; then
  C_BOLD=$'\e[1m'; C_DIM=$'\e[2m'; C_RED=$'\e[31m'
  C_GREEN=$'\e[32m'; C_YELLOW=$'\e[33m'; C_CYAN=$'\e[36m'; C_RESET=$'\e[0m'
else
  C_BOLD='' C_DIM='' C_RED='' C_GREEN='' C_YELLOW='' C_CYAN='' C_RESET=''
fi

die()     { printf '%serror:%s %s\n' "${C_BOLD}${C_RED}" "$C_RESET" "$*" >&2; exit 1; }
require() { command -v "$1" >/dev/null 2>&1 || die "missing required tool '$1' (${2:-})"; }
note()    { printf '\n%s==>%s %s\n' "${C_BOLD}${C_CYAN}" "$C_RESET" "$*"; }
ok()      { printf '%s✓%s %s\n' "$C_GREEN" "$C_RESET" "$*"; }
PHASE_COUNT=7
phase()   { printf '\n%s── [ %s/%s · %s ] ──%s\n' "${C_BOLD}${C_YELLOW}" "$1" "$PHASE_COUNT" "${*:2}" "$C_RESET"; }
banner()  {
  printf '\n%s============================================%s\n' "${C_BOLD}${C_CYAN}" "$C_RESET"
  printf '%s  glab-tui release orchestrator%s\n' "${C_BOLD}" "$C_RESET"
  printf '%s============================================%s\n' "${C_BOLD}${C_CYAN}" "$C_RESET"
}

# ---------------------------------------------------------------------------
# spinner / progress bar helpers (auto-disabled when not a TTY)
# ---------------------------------------------------------------------------
SPINNER_FRAMES=('⠋' '⠙' '⠹' '⠸' '⠼' '⠴' '⠦' '⠧' '⠇' '⠏')

# spinner <label> <command...>
# Runs <command> with its output captured to $TMP_DIR/spinner.log while an
# animated spinner ticks in place on the current line. On failure the tail of
# the log is printed so you can see what went wrong. Returns the command's
# exit status so the caller's `set -e` / `if` semantics are preserved.
spinner() {
  local label="$1"
  shift
  if [[ ! -t 1 ]]; then
    "$@"
    return $?
  fi

  local log="${TMP_DIR}/spinner.log"
  : > "$log"

  # The animator runs in the background; the command stays in the foreground
  # so Ctrl-C and exit codes behave exactly as if it had been run directly.
  (
    local i=0 start=$SECONDS
    printf '\e[?25l'
    while true; do
      printf '\r\e[2K%s%s %s%s %s%ss%s' \
        "$C_YELLOW" "${SPINNER_FRAMES[i % ${#SPINNER_FRAMES[@]}]}" \
        "$C_BOLD" "$label" "$C_DIM" "$((SECONDS - start))" "$C_RESET"
      i=$((i + 1))
      sleep 0.1
    done
  ) &
  local animator=$!
  local status=0
  "$@" >"$log" 2>&1 || status=$?
  kill "$animator" 2>/dev/null || true
  wait "$animator" 2>/dev/null || true
  printf '\r\e[2K\e[?25h'
  if (( status != 0 )); then
    printf '%serror:%s %s\n' "${C_BOLD}${C_RED}" "$C_RESET" "$label" >&2
    tail -20 "$log" >&2
  fi
  return "$status"
}

# progress_bar <current> <total> <label...>
# Renders an in-place determinate progress bar, e.g. "[██████░░░░]  30% assets".
progress_bar() {
  local current="$1" total="$2"
  shift 2
  local width=20 pct=0 filled=0 empty=0 i bar=""
  if (( total > 0 )); then
    pct=$((current * 100 / total))
  fi
  filled=$((pct * width / 100))
  empty=$((width - filled))
  for ((i = 0; i < filled; i++)); do bar+="█"; done
  for ((i = 0; i < empty; i++)); do bar+="░"; done
  printf '\r\e[2K  %s[%s]%s %3d%%  %s%s%s' \
    "$C_YELLOW" "$bar" "$C_RESET" "$pct" "$C_DIM" "$*" "$C_RESET"
}

run_opencode() {
  note "opencode ($OPENCODE_MODEL), output logged to $TMP_DIR/spinner.log"
  if ! spinner "opencode ($OPENCODE_MODEL)" \
      opencode run --auto --model "$OPENCODE_MODEL" "$1"; then
    die "opencode failed (log: $TMP_DIR/spinner.log)"
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
# Starting-point picker
# ---------------------------------------------------------------------------
# Phase IDs (numeric so we can do >= comparisons)
PHASE_PREFLIGHT=0
PHASE_PREPARE=1
PHASE_GIFS=2
PHASE_REVIEW=3
PHASE_WAIT_CI=4
PHASE_POST_RELEASE=5
PHASE_PUBLISH=6

START_PHASE=$PHASE_PREFLIGHT   # default: run everything

select_start_phase() {
  local -a phase_lines=(
    "$PHASE_PREFLIGHT"$'\t'"0 · From the beginning  (preflight → prepare → GIFs → review → CI → post-release → publish)"
    "$PHASE_PREPARE"$'\t'"1 · Prepare docs & PR    (version bump, opencode docs, create PR — skips preflight)"
    "$PHASE_GIFS"$'\t'"2 · Generate GIFs only   (re-run generate-demos.sh, push, rebuild PR if needed)"
    "$PHASE_REVIEW"$'\t'"3 · Review & merge       (squash-merge an existing PR and tag)"
    "$PHASE_WAIT_CI"$'\t'"4 · Wait for CI build    (poll release assets for an already-tagged version)"
    "$PHASE_POST_RELEASE"$'\t'"5 · Post-release         (release notes, Homebrew formula, Scoop manifest — Homebrew/Scoop skipped for nightly)"
    "$PHASE_PUBLISH"$'\t'"6 · Publish              (Docker image → GHCR + crate → crates.io — crates.io skipped for nightly)"
  )

  note "Where would you like to start?"
  pick "start phase" "$PHASE_PREFLIGHT" "${phase_lines[@]}"
  START_PHASE="$PICK_RESULT"
  ok "Starting from phase $START_PHASE"
}

# ---------------------------------------------------------------------------
# State bootstrap helpers — prompt for values that skipped phases would set
# ---------------------------------------------------------------------------

# True when the current build targets a nightly tag (ends in `-nightly` or
# the `INCREMENT=nightly` CLI arg / menu pick was used). Nightlies skip the
# Cargo.toml bump, the prepare PR, Homebrew/Scoop syncs, and crates.io
# publish; they still get release notes and a `:nightly` Docker image.
is_nightly() {
  [[ "${INCREMENT:-}" == "nightly" ]] || [[ "${NEW_TAG:-}" == *-nightly* ]]
}

# Ensure NEW_TAG / VERSION are set; fetch from git tags or prompt.
ensure_version() {
  if [[ -n "${NEW_TAG:-}" ]]; then return; fi

  git fetch --tags --prune 2>/dev/null || true

  # For nightly invocations, auto-detect the most recent nightly tag so the
  # user doesn't have to paste it.
  if [[ "${INCREMENT:-}" == "nightly" ]]; then
    local latest_nightly
    latest_nightly="$(git tag --list 'v*-nightly*' --sort=-v:refname 2>/dev/null | head -n 1 || true)"
    if [[ -n "$latest_nightly" ]]; then
      NEW_TAG="$latest_nightly"
      VERSION="${NEW_TAG#v}"
      ok "Auto-detected nightly tag: $NEW_TAG"
      return 0
    fi
  fi

  local latest_tag
  latest_tag="$(git describe --tags --abbrev=0 2>/dev/null || echo "")"

  printf '\n%sEnter the release tag%s (e.g. v1.2.3 or v1.2.3-nightly[.YYYYMMDD])' "$C_BOLD" "$C_RESET"
  if [[ -n "$latest_tag" ]]; then
    printf ' [latest tag: %s]' "$latest_tag"
  fi
  printf ': '
  read -r NEW_TAG
  [[ -n "$NEW_TAG" ]] || die "version tag is required"
  [[ "$NEW_TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-nightly([-.]?[A-Za-z0-9]+)*)?$ ]] || \
    die "tag must match vX.Y.Z or vX.Y.Z[-nightly][-.suffix] (got '$NEW_TAG')"
  VERSION="${NEW_TAG#v}"
  ok "Version: $NEW_TAG"
}

# Ensure BRANCH is set (derived from NEW_TAG when not set by prepare()).
ensure_branch() {
  if [[ -n "${BRANCH:-}" ]]; then return; fi
  ensure_version
  BRANCH="opencode-release/$NEW_TAG"
}

# Ensure PR_NUMBER is set; look it up or prompt.
ensure_pr_number() {
  if [[ -n "${PR_NUMBER:-}" ]]; then return; fi
  ensure_branch

  PR_NUMBER="$(gh pr list --repo "$REPO" --head "$BRANCH" --state open --json number --jq '.[0].number' 2>/dev/null || true)"
  if [[ -z "$PR_NUMBER" || "$PR_NUMBER" == "null" ]]; then
    PR_NUMBER="$(gh pr list --repo "$REPO" --head "$BRANCH" --state merged --json number --jq '.[0].number' 2>/dev/null || true)"
  fi

  if [[ -z "$PR_NUMBER" || "$PR_NUMBER" == "null" ]]; then
    printf '\n%sEnter the PR number%s for branch %s: ' "$C_BOLD" "$C_RESET" "$BRANCH"
    read -r PR_NUMBER
    [[ "$PR_NUMBER" =~ ^[0-9]+$ ]] || die "PR number must be a positive integer"
  fi
  ok "PR #$PR_NUMBER"
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
  if ! spinner "Checking gh auth, push access, and fonts" bash -c '
    gh auth status >/dev/null 2>&1 || { echo "not authenticated with gh; run gh auth login first"; exit 1; }
    for repo in rcieri/homebrew-glab-tui rcieri/scoop-glab-tui; do
      gh api "repos/$repo" --jq ".permissions.push" | grep -q true || \
        { echo "no push access to $repo; grant your token write permission"; exit 1; }
    done
    fc-list 2>/dev/null | grep -qi "JetBrainsMono.*Nerd" || \
      { echo "JetBrainsMono Nerd Font not installed (see https://github.com/ryanoasis/nerd-fonts)"; exit 1; }
    exit 0
  '; then
    die "preflight checks failed (see $TMP_DIR/spinner.log)"
  fi
}

# ---------------------------------------------------------------------------
# Phase 1: determine next version and prepare the release PR
# ---------------------------------------------------------------------------
next_version() {
  git fetch --tags --prune
  local latest_tag version base_major base_minor base_patch
  local major_v minor_v patch_v date_suffix nightly_v
  latest_tag="$(git describe --tags --abbrev=0 2>/dev/null || echo v0.0.0)"
  version="${latest_tag#v}"
  IFS='.' read -r base_major base_minor base_patch <<< "$version"

  major_v="$((base_major + 1)).0.0"
  minor_v="$base_major.$((base_minor + 1)).0"
  patch_v="$base_major.$base_minor.$((base_patch + 1))"
  date_suffix="$(date -u +%Y%m%d)"
  nightly_v="${patch_v}-nightly.${date_suffix}"

  if [[ -z "$INCREMENT" ]]; then
    printf '\n%sCurrent version:%s %s\n' "$C_BOLD" "$C_RESET" "$latest_tag"
    printf '  %s1)%s patch              -> v%s\n' "$C_BOLD" "$C_RESET" "$patch_v"
    printf '  %s2)%s minor              -> v%s\n' "$C_BOLD" "$C_RESET" "$minor_v"
    printf '  %s3)%s major              -> v%s\n' "$C_BOLD" "$C_RESET" "$major_v"
    printf '  %s4)%s nightly (bare)     -> v%s-nightly\n' "$C_BOLD" "$C_RESET" "$patch_v"
    printf '  %s5)%s nightly (dated)    -> v%s  (pre-release, skip publish)\n' "$C_BOLD" "$C_RESET" "$nightly_v"
    read -r -p "Select release increment [1/2/3/4/5] (default patch): " choice
    case "${choice:-1}" in
      1|patch)   VERSION="$patch_v" ;;
      2|minor)   VERSION="$minor_v" ;;
      3|major)   VERSION="$major_v" ;;
      4|nightly) INCREMENT="nightly"; VERSION="${patch_v}-nightly" ;;
      5|nightly-dated)
                   INCREMENT="nightly"; VERSION="$nightly_v" ;;
      *) die "invalid selection '$choice'" ;;
    esac
  else
    case "$INCREMENT" in
      major)         VERSION="$major_v" ;;
      minor)         VERSION="$minor_v" ;;
      patch)         VERSION="$patch_v" ;;
      nightly)       VERSION="${patch_v}-nightly" ;;
      nightly-dated) VERSION="$nightly_v" ;;
      *) die "invalid version increment '$INCREMENT' (expected patch|minor|major|nightly|nightly-dated)" ;;
    esac
  fi

  NEW_TAG="v$VERSION"
  BRANCH="opencode-release/$NEW_TAG"
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
  ensure_version   # no-op if next_version() already ran

  if is_nightly; then
    note "Nightly release — skipping docs regeneration and PR creation"
    if [[ "${BUMP_CARGO_FOR_NIGHTLY:-}" == "1" ]]; then
      bump_cargo_version
      note "Bumped Cargo.toml to ${VERSION} (BUMP_CARGO_FOR_NIGHTLY=1) — commit this before tagging so the binary's --version stamp matches the tag"
    else
      note "Cargo.toml NOT bumped — the binary will report the previous stable version."
      note "Set BUMP_CARGO_FOR_NIGHTLY=1 to bump Cargo.toml to ${VERSION} for the binary's --version stamp."
    fi
    note "(nightly tag $NEW_TAG should already be pushed to origin; CI will build the assets)"
    return 0
  fi

  if git show-ref --verify --quiet "refs/heads/$BRANCH"; then
    git checkout "$BRANCH"
  elif git ls-remote --exit-code --quiet origin "refs/heads/$BRANCH" 2>/dev/null; then
    git checkout -b "$BRANCH" "origin/$BRANCH"
  else
    git checkout -b "$BRANCH"
  fi

  bump_cargo_version
  spinner "Building release binary" cargo build --release

  note "Regenerating CHANGELOG.md / AGENTS.md / README.md via opencode..."
  PROMPT="We are prepping a new repository release. The upcoming version tag is going to be: $NEW_TAG.

Your task is to analyze the git commits, merged pull requests, and codebase changes since the last version tag, and update the following three files directly in the workspace:

1. CHANGELOG.md: Prepend a beautifully structured, developer-friendly update section for version $NEW_TAG at the top of the file, cleanly breaking down Features, Bug Fixes, and Maintenance.
2. AGENTS.md: Update any agent guidelines, automation logs, or architecture schemas affected by our latest feature set or dependencies. Ensure versioning matrices match $NEW_TAG.
3. README.md: Scan for installation commands, setup instructions, or documentation badges displaying the old version string, and replace them cleanly with version $NEW_TAG.

The crate version in Cargo.toml and Cargo.lock has already been bumped to $VERSION; do not modify those files. Save and write these file modifications directly back into the working directory."

  run_opencode "$PROMPT"
  ok "CHANGELOG.md / AGENTS.md / README.md regenerated"
}

# ---------------------------------------------------------------------------
# Phase 2: generate demo GIFs
# ---------------------------------------------------------------------------
generate_gifs() {
  ensure_branch

  if is_nightly; then
    note "Nightly release — skipping GIF generation and PR creation"
    return 0
  fi

  # Make sure there is a built binary on PATH.
  if [[ ! -x "$ROOT/target/release/glab-tui" ]]; then
    spinner "Building release binary" cargo build --release
  fi

  export PATH="$ROOT/target/release:$PATH"
  spinner "Generating demo GIFs" "$ROOT/assets/generate-demos.sh"
  ok "demo GIFs regenerated"

  # Stage and commit the GIFs (and any other outstanding prepare changes).
  git add CHANGELOG.md AGENTS.md README.md Cargo.toml Cargo.lock assets/demo-*.gif
  if ! git diff --cached --quiet; then
    git commit -m "chore: prepare release $NEW_TAG"
  fi
  spinner "Pushing branch $BRANCH" git push -u origin "$BRANCH"

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
# Phase 3: wait for review, then merge and tag
# ---------------------------------------------------------------------------
review_gate() {
  if is_nightly; then
    note "Nightly release — no PR to review (nightly tag is pushed manually)"
    return 0
  fi
  ensure_pr_number
  PR_URL="https://github.com/$REPO/pull/$PR_NUMBER"
  note "Pause for review — PR: $PR_URL"
  read -r -p "Review the PR (CI checks run in the background). Press Enter to squash-merge and continue the release... "
}

merge_and_tag() {
  ensure_version

  if is_nightly; then
    note "Nightly release — tag $NEW_TAG should already be pushed to origin"
    # Verify the tag exists locally; bail out if not so the user knows.
    if ! git rev-parse --verify --quiet "refs/tags/$NEW_TAG" >/dev/null; then
      die "nightly tag $NEW_TAG not found locally — push it first (e.g. 'git push origin $NEW_TAG')"
    fi
    return 0
  fi

  ensure_pr_number

  note "Merging PR #$PR_NUMBER (squash, auto-merge when checks pass)..."
  if ! spinner "Merging PR #$PR_NUMBER" gh pr merge "$PR_NUMBER" --repo "$REPO" --squash --auto; then
    local state
    state="$(gh pr view "$PR_NUMBER" --repo "$REPO" --json state --jq '.state')"
    [[ "$state" == "MERGED" ]] || die "failed to merge PR #$PR_NUMBER (conflicts? not mergeable?)"
  fi
  export PR_NUMBER REPO
  if ! spinner "Waiting for PR #$PR_NUMBER to merge (up to 20m)" bash -c '
      for i in $(seq 1 120); do
        [[ "$(gh pr view "$PR_NUMBER" --repo "$REPO" --json state --jq .state)" == "MERGED" ]] && exit 0
        sleep 10
      done
      exit 1
    '; then
    die "timed out waiting for PR #$PR_NUMBER to merge"
  fi
  ok "PR #$PR_NUMBER merged"

  note "Tagging $NEW_TAG on the merge commit..."
  local merge_sha
  merge_sha="$(gh pr view "$PR_NUMBER" --repo "$REPO" --json mergeCommit --jq '.mergeCommit.oid')"
  git fetch origin main
  git tag "$NEW_TAG" "$merge_sha"
  git push origin "$NEW_TAG"
  ok "tag $NEW_TAG pushed; release build: https://github.com/$REPO/actions"
}

# ---------------------------------------------------------------------------
# Phase 4: wait for the CI release build
# ---------------------------------------------------------------------------
wait_for_release() {
  ensure_version

  local total i current elapsed required_min missing
  total=$((RELEASE_WAIT_MIN * 3)) # one check every 20s
  note "Waiting for release $NEW_TAG assets (timeout ${RELEASE_WAIT_MIN}m)..."
  for i in $(seq 1 "$total"); do
    current=$(gh release view "$NEW_TAG" --repo "$REPO" --json assets \
      --jq '[.assets[].name] | join("\n")' 2>/dev/null || echo "")
    if [[ -n "$current" ]]; then
      missing=()
      for asset in "${REQUIRED_ASSETS_STATIC[@]}"; do
        if ! grep -qxF "$asset" <<< "$current"; then
          missing+=("$asset")
        fi
      done
      # At least one ubuntu-* asset whose version differs from the static
      # 22.04 / 24.04 entries — that's the ubuntu-latest build.
      local dynamic_count
      dynamic_count=$(grep -E '^glab-tui-linux-(amd64|arm64)-ubuntu-([0-9]+\.[0-9]+)\.tar\.gz$' <<< "$current" \
        | grep -Ev 'ubuntu-22\.04|ubuntu-24\.04' | sort -u | wc -l)
      if [[ "${#missing[@]}" -eq 0 ]] && [[ "$dynamic_count" -ge 2 ]]; then
        [[ -t 1 ]] && printf '\r\e[2K'
        ok "All release assets present ($(echo "$current" | wc -l) total)"
        return 0
      fi
    fi
    [[ $i -eq $total ]] && die "timed out waiting for release assets for $NEW_TAG (missing: ${missing[*]:-none}, dynamic: $dynamic_count)"
    if [[ -t 1 ]]; then
      elapsed=$((i * 20 / 60))
      local seen="${#current[@]}"
      [[ -z "$current" ]] && seen=0 || seen=$(echo "$current" | wc -l)
      progress_bar "$seen" "20" "assets ($elapsed min elapsed)"
    fi
    sleep 20
  done
}

# ---------------------------------------------------------------------------
# Phase 5: post-release (notes, Homebrew, Scoop)
# ---------------------------------------------------------------------------
update_homebrew() {
  local macos_amd64 macos_arm64

  spinner "Cloning rcieri/homebrew-glab-tui" gh repo clone rcieri/homebrew-glab-tui "$TMP_DIR/homebrew-glab-tui"
  cd "$TMP_DIR/homebrew-glab-tui"

  fetch_sha() {
    local name="$1"
    spinner "Fetching ${name}" \
      curl -sL "https://github.com/$REPO/releases/download/${NEW_TAG}/${name}" -o "$TMP_DIR/${name}"
    sha256sum "$TMP_DIR/${name}" | cut -d' ' -f1
  }

  macos_amd64=$(fetch_sha glab-tui-macos-amd64.tar.gz)
  macos_arm64=$(fetch_sha glab-tui-macos-arm64.tar.gz)

  # Discover all Linux assets actually present in the release (the ubuntu-latest
  # runner's asset name carries a VERSION_ID we don't know up front).
  local assets_json
  assets_json=$(gh release view "$NEW_TAG" --repo "$REPO" --json assets --jq '.assets[].name' 2>/dev/null || true)

  declare -A linux_amd64_shas=()
  declare -A linux_arm64_shas=()
  local variant
  while IFS= read -r name; do
    [[ "$name" =~ ^glab-tui-linux-(amd64|arm64)-(.+)\.tar\.gz$ ]] || continue
    local arch="${BASH_REMATCH[1]}" variant="${BASH_REMATCH[2]}"
    local sha
    sha=$(fetch_sha "$name")
    if [[ "$arch" == "amd64" ]]; then
      linux_amd64_shas["$variant"]="$sha"
    else
      linux_arm64_shas["$variant"]="$sha"
    fi
  done <<< "$assets_json"

  if [[ "${#linux_amd64_shas[@]}" -eq 0 ]] || [[ "${#linux_arm64_shas[@]}" -eq 0 ]]; then
    die "no Linux assets found in release $NEW_TAG — did the build matrix finish?"
  fi

  # Render a Ruby hash literal from the bash assoc array (sorted by key).
  render_ruby_hash() {
    local -n arr=$1
    local indent="${2:-    }" k v first=1
    printf '%s{\n' "$indent"
    while IFS=$'\t' read -r k v; do
      [[ -z "$k" ]] && continue
      if (( first )); then first=0; else printf ',\n'; fi
      printf '%s  "%s" => "%s"' "$indent" "$k" "$v"
    done < <(for k in "${!arr[@]}"; do printf '%s\t%s\n' "$k" "${arr[$k]}"; done | sort)
    printf '\n%s}.freeze\n' "$indent"
  }

  local linux_amd64_hash
  linux_amd64_hash=$(render_ruby_hash linux_amd64_shas "    ")
  local linux_arm64_hash
  linux_arm64_hash=$(render_ruby_hash linux_arm64_shas "    ")

  cat > "$TMP_DIR/glab-tui.rb" <<EOF
class GlabTui < Formula
  desc "Terminal user interface for GitLab and GitHub"
  homepage "https://github.com/rcieri/glab-tui"
  license "MIT"

  depends_on "gh"
  depends_on "glab" => :recommended

  # Discovered at release time: variant (ubuntu-XX.YY or "musl") -> sha256.
$(printf '%s' "$linux_amd64_hash" | sed 's/^/  /')
$(printf '%s' "$linux_arm64_hash" | sed 's/^/  /')

  # Pick the best-matching variant for the local Ubuntu version. Non-Ubuntu
  # Linux distros fall back to the oldest Ubuntu LTS asset (broadest glibc
  # compatibility). If no Ubuntu version asset is available for the current
  # release, fall through to whichever LTS asset is newest.
  def self.linux_variant(sha_map)
    return nil if sha_map.nil? || sha_map.empty?
    v = OS::Version.from_symbol(:ubuntu)
    candidates = []
    if v
      candidates << "ubuntu-\#{v}"
      # Walk down through known LTS baselines (newest first) inserted at
      # the front of the candidate list.
      %w[24.04 22.04].each { |baseline| candidates << "ubuntu-\#{baseline}" unless "ubuntu-\#{v}" == "ubuntu-\#{baseline}" }
    else
      candidates = %w[ubuntu-24.04 ubuntu-22.04]
    end
    candidates << "musl"
    candidates.uniq.each do |c|
      return c if sha_map.key?(c)
    end
    sha_map.keys.first
  end

  on_macos do
    on_intel do
      url "https://github.com/rcieri/glab-tui/releases/download/${NEW_TAG}/glab-tui-macos-amd64.tar.gz"
      sha256 "${macos_amd64}"
    end
    on_arm do
      url "https://github.com/rcieri/glab-tui/releases/download/${NEW_TAG}/glab-tui-macos-arm64.tar.gz"
      sha256 "${macos_arm64}"
    end
  end

  on_linux do
    on_intel do
      amd64_variant = linux_variant(LINUX_AMD64_SHAS)
      url "https://github.com/rcieri/glab-tui/releases/download/${NEW_TAG}/glab-tui-linux-amd64-\#{amd64_variant}.tar.gz"
      sha256 LINUX_AMD64_SHAS.fetch(amd64_variant)
    end
    on_arm do
      arm64_variant = linux_variant(LINUX_ARM64_SHAS)
      url "https://github.com/rcieri/glab-tui/releases/download/${NEW_TAG}/glab-tui-linux-arm64-\#{arm64_variant}.tar.gz"
      sha256 LINUX_ARM64_SHAS.fetch(arm64_variant)
    end
  end

  livecheck do
    url :stable
    strategy :github_latest
  end

  def install
    bin.install "glab-tui"
  end

  test do
    system "\#{bin}/glab-tui", "--help"
  end
end
EOF

  mv "$TMP_DIR/glab-tui.rb" Formula/glab-tui.rb

  git add Formula/glab-tui.rb
  if git diff --cached --quiet; then
    note "Homebrew formula already up to date"
  else
    git -c user.name="opencode-release[bot]" \
        -c user.email="opencode-release[bot]@users.noreply.github.com" \
        commit -m "Update to ${NEW_TAG}" >/dev/null
    spinner "Pushing Homebrew formula" git push
    ok "Homebrew formula updated and pushed"
  fi
  cd "$ROOT"
}

update_scoop() {
  local version sha
  spinner "Cloning rcieri/scoop-glab-tui" gh repo clone rcieri/scoop-glab-tui "$TMP_DIR/scoop-glab-tui"
  cd "$TMP_DIR/scoop-glab-tui"

  version="${NEW_TAG#v}"
  spinner "Fetching glab-tui-windows-amd64.zip" \
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
    spinner "Pushing Scoop manifest" git push
    ok "Scoop manifest updated and pushed"
  fi
  cd "$ROOT"
}

post_release() {
  ensure_version

  local prev_tag prompt
  prev_tag="$(git describe --tags --abbrev=0 "${NEW_TAG}^" 2>/dev/null || git describe --tags --abbrev=0 2>/dev/null || true)"
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
  spinner "Updating release $NEW_TAG body" gh release edit "$NEW_TAG" --repo "$REPO" --notes-file RELEASE_NOTES.md

  if is_nightly; then
    note "Skipping Homebrew/Scoop manifest updates for nightly $NEW_TAG"
    return 0
  fi

  update_homebrew
  update_scoop
}

# ---------------------------------------------------------------------------
# Phase 6: publish (Docker image + crate)
# ---------------------------------------------------------------------------
publish() {
  ensure_version

  if is_nightly; then
    note "Skipping Docker push and crates.io publish for nightly $NEW_TAG"
    note "(pre-release versions are rejected by crates.io, and nightly Docker images are intentionally not pushed)"
    return 0
  fi

  local package_version tag_version user
  package_version="$(cargo metadata --format-version 1 --no-deps 2>/dev/null | jq -r '.packages[0].version')"
  tag_version="${NEW_TAG#v}"
  if [[ "$package_version" != "$tag_version" ]]; then
    die "Cargo package version ($package_version) does not match tag version ($tag_version)"
  fi

  require docker "see https://docs.docker.com/get-docker/"
  user="$(gh api user --jq .login)"
  spinner "Authenticating to GHCR" bash -c 'gh auth token | docker login ghcr.io -u "$0" --password-stdin' "$user"
  local tags_args=(-t "ghcr.io/$REPO:$NEW_TAG")
  if [[ "$NEW_TAG" != *-* ]]; then
    tags_args+=(-t "ghcr.io/$REPO:latest")
  fi
  spinner "Building & pushing Docker image to GHCR" docker buildx build --push "${tags_args[@]}" .
  ok "Docker image pushed to GHCR"

  spinner "Publishing crate v$package_version to crates.io" cargo publish --locked
  ok "crate v$package_version published to crates.io"
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------
main() {
  TMP_DIR="$(mktemp -d)"
  trap 'printf "\e[?25h" 2>/dev/null || true; rm -rf "$TMP_DIR"' EXIT
  banner

  # ── Starting-point picker ──────────────────────────────────────────────────
  select_start_phase

  # ── Phase 0: Preflight ────────────────────────────────────────────────────
  if [[ "$START_PHASE" -le "$PHASE_PREFLIGHT" ]]; then
    phase 1 "Preflight"
    preflight
  fi

  # ── Phase 1: Prepare (version bump, docs, PR) ─────────────────────────────
  if [[ "$START_PHASE" -le "$PHASE_PREPARE" ]]; then
    phase 2 "Prepare"
    next_version
    if [[ -z "$OPENCODE_MODEL_FROM_ENV" ]]; then
      select_opencode_model
    else
      ok "using OPENCODE_MODEL from environment: $OPENCODE_MODEL"
    fi
    prepare
  fi

  # ── Phase 2: Generate GIFs ────────────────────────────────────────────────
  if [[ "$START_PHASE" -le "$PHASE_GIFS" ]]; then
    phase 3 "Generate GIFs"
    # If jumping directly to this phase, hydrate required state.
    if [[ "$START_PHASE" -ge "$PHASE_GIFS" ]]; then
      ensure_version
      if [[ -z "$OPENCODE_MODEL_FROM_ENV" ]]; then
        select_opencode_model
      else
        ok "using OPENCODE_MODEL from environment: $OPENCODE_MODEL"
      fi
    fi
    generate_gifs
  fi

  # ── Phase 3: Review & merge ───────────────────────────────────────────────
  if [[ "$START_PHASE" -le "$PHASE_REVIEW" ]]; then
    phase 4 "Review & merge"
    review_gate
    merge_and_tag
  fi

  # ── Phase 4: Wait for CI build ────────────────────────────────────────────
  if [[ "$START_PHASE" -le "$PHASE_WAIT_CI" ]]; then
    phase 5 "Wait for CI build"
    if [[ "$START_PHASE" -ge "$PHASE_WAIT_CI" ]]; then
      ensure_version
    fi
    wait_for_release
  fi

  # ── Phase 5: Post-release (notes, Homebrew, Scoop) ───────────────────────
  if [[ "$START_PHASE" -le "$PHASE_POST_RELEASE" ]]; then
    phase 6 "Post-release"
    if [[ "$START_PHASE" -ge "$PHASE_POST_RELEASE" ]]; then
      ensure_version
      if [[ -z "$OPENCODE_MODEL_FROM_ENV" ]]; then
        select_opencode_model
      else
        ok "using OPENCODE_MODEL from environment: $OPENCODE_MODEL"
      fi
    fi
    post_release
  fi

  # ── Phase 6: Publish (Docker + crate) ────────────────────────────────────
  if [[ "$START_PHASE" -le "$PHASE_PUBLISH" ]]; then
    phase 7 "Publish"
    if [[ "$START_PHASE" -ge "$PHASE_PUBLISH" ]]; then
      ensure_version
    fi
    publish
  fi

  git checkout main 2>/dev/null || true
  git branch -D "${BRANCH:-}" 2>/dev/null || true
  ok "Release ${NEW_TAG:-} complete."
}

main "$@"
