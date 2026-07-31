#!/usr/bin/env bash
set -euo pipefail

# Locally prepares a release: bumps the version, regenerates docs via headless
# opencode, rebuilds the demo GIFs against a real authenticated `gh`, and opens
# a release preparation PR. Replaces the old .github/workflows/prepare-release.yml.

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

REPO="rcieri/glab-tui"
INCREMENT="${1:-patch}"

die() { printf 'error: %s\n' "$*" >&2; exit 1; }
require() { command -v "$1" >/dev/null 2>&1 || die "missing required tool '$1' (${2:-})"; }

# --- determine next version tag -------------------------------------------------
git fetch --tags --prune
LATEST_TAG="$(git describe --tags --abbrev=0 2>/dev/null || echo v0.0.0)"
VERSION="${LATEST_TAG#v}"
IFS='.' read -r major minor patch <<< "$VERSION"
case "$INCREMENT" in
  major) major=$((major + 1)); minor=0; patch=0 ;;
  minor) minor=$((minor + 1)); patch=0 ;;
  patch) patch=$((patch + 1)) ;;
  *) die "invalid version increment '$INCREMENT' (expected patch|minor|major)" ;;
esac
NEW_TAG="v$major.$minor.$patch"
echo "==> Latest tag: $LATEST_TAG  next version: $NEW_TAG"

# --- work on the release branch from the start ---------------------------------
BRANCH="opencode-release/$NEW_TAG"
if git show-ref --verify --quiet "refs/heads/$BRANCH"; then
  git checkout "$BRANCH"
elif git ls-remote --exit-code --quiet origin "refs/heads/$BRANCH" 2>/dev/null; then
  git checkout -b "$BRANCH" "origin/$BRANCH"
else
  git checkout -b "$BRANCH"
fi

# --- regenerate CHANGELOG / AGENTS / README via headless opencode ---------------
require opencode "install from https://opencode.ai"
PROMPT="We are prepping a new repository release. The upcoming version tag is going to be: $NEW_TAG.

Your task is to analyze the git commits, merged pull requests, and codebase changes since the last version tag, and update the following three files directly in the workspace:

1. CHANGELOG.md: Prepend a beautifully structured, developer-friendly update section for version $NEW_TAG at the top of the file, cleanly breaking down Features, Bug Fixes, and Maintenance.
2. AGENTS.md: Update any agent guidelines, automation logs, or architecture schemas affected by our latest feature set or dependencies. Ensure versioning matrices match $NEW_TAG.
3. README.md: Scan for installation commands, setup instructions, or documentation badges displaying the old version string, and replace them cleanly with version $NEW_TAG.

Save and write these file modifications directly back into the working directory."

MODEL_ARGS=()
if [[ -n "${OPENCODE_MODEL:-}" ]]; then
  MODEL_ARGS=(--model "$OPENCODE_MODEL")
fi
opencode run --auto "${MODEL_ARGS[@]}" "$PROMPT"

# --- build the binary the demo recordings will launch ---------------------------
require cargo "install Rust via https://rustup.rs"
cargo build --release

# --- verify demo recording prerequisites ----------------------------------------
require vhs "go install github.com/charmbracelet/vhs@latest"
require ttyd "apt install ttyd / brew install ttyd"
require ffmpeg "apt install ffmpeg / brew install ffmpeg"
require unzip "apt install unzip"
require gh "see https://cli.github.com"
gh auth status >/dev/null 2>&1 || die "not authenticated with gh; run 'gh auth login' first"
fc-list 2>/dev/null | grep -qi "JetBrainsMono.*Nerd" || \
  die "JetBrainsMono Nerd Font not installed (download from https://github.com/ryanoasis/nerd-fonts)"

# --- regenerate demo GIFs --------------------------------------------------------
export PATH="$ROOT/target/release:$PATH"
"$ROOT/assets/generate-demos.sh"

# --- commit, push, and open the release preparation PR ---------------------------
git add CHANGELOG.md AGENTS.md README.md assets/demo-*.gif
if git diff --cached --quiet; then
  echo "==> No changes to commit"
  exit 0
fi
git commit -m "chore: prepare release $NEW_TAG"
git push -u origin "$BRANCH"

if gh pr list --repo "$REPO" --head "$BRANCH" --state open --json number --jq 'length' | grep -q '^1$'; then
  echo "==> PR already open for $BRANCH"
else
  gh pr create --repo "$REPO" --base main --head "$BRANCH" \
    --title "chore: prepare release $NEW_TAG" \
    --body "Automated release preparation for **$NEW_TAG**.

- Regenerated CHANGELOG.md, AGENTS.md, and README.md
- Regenerated demo GIFs against live authenticated data

Review, merge, then tag \`$NEW_TAG\` to trigger the release build."
fi
