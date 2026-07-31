#!/usr/bin/env bash
set -euo pipefail

# Locally finishes a release after the tag build has published its binaries:
# generates release notes, updates the GitHub release body, and bumps the
# Homebrew formula and Scoop manifest. Replaces the old
# .github/workflows/post-release.yml.

TAG="${1:?usage: scripts/release/post.sh <tag> e.g. v0.9.0}"
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

REPO="rcieri/glab-tui"
ASSETS=(glab-tui-linux-amd64.tar.gz glab-tui-linux-arm64.tar.gz \
        glab-tui-macos-amd64.tar.gz glab-tui-macos-arm64.tar.gz \
        glab-tui-windows-amd64.zip)

die() { printf 'error: %s\n' "$*" >&2; exit 1; }
require() { command -v "$1" >/dev/null 2>&1 || die "missing required tool '$1' (${2:-})"; }

require gh "see https://cli.github.com"
require jq "apt install jq / brew install jq"
require curl
require sha256sum

git fetch --tags --prune
gh auth status >/dev/null 2>&1 || die "not authenticated with gh; run 'gh auth login' first"

PREV_TAG="$(git describe --tags --abbrev=0 "${TAG}^" 2>/dev/null || git describe --tags --abbrev=0 2>/dev/null || true)"
[[ -n "$PREV_TAG" ]] || die "could not determine the previous tag before $TAG"

# --- wait for CI to finish uploading release assets ------------------------------
echo "==> Waiting for release $TAG assets to be uploaded by CI..."
for i in $(seq 1 30); do
  UPLOADED="$(gh release view "$TAG" --repo "$REPO" --json assets --jq '[.assets[].name] | length' 2>/dev/null || echo 0)"
  if [[ "$UPLOADED" -ge "${#ASSETS[@]}" ]]; then
    echo "==> All ${#ASSETS[@]} assets present."
    break
  fi
  [[ "$i" -eq 30 ]] && die "timed out waiting for release assets for $TAG (see https://github.com/$REPO/actions)"
  sleep 10
done

# --- generate release notes via headless opencode ---------------------------------
require opencode "install from https://opencode.ai"
PROMPT="Read CHANGELOG.md and extract the section for version $TAG.

Also read the existing release notes for the previous tag $PREV_TAG (use \`gh release view $PREV_TAG --json body --jq .body\`) to match their formatting style.

Write the file RELEASE_NOTES.md matching the same format:
- Title \"## What's Changed\"
- Sections: ### Added / ### Fixed / ### Changed / ### Dependencies
- Entries start with bolded headline: \`- **Name** — Description with references (#123).\`
- End with: \`**Full Changelog**: https://github.com/rcieri/glab-tui/compare/$PREV_TAG...$TAG\`

Use the content from CHANGELOG.md for the current version as the source material."

MODEL_ARGS=()
if [[ -n "${OPENCODE_MODEL:-}" ]]; then
  MODEL_ARGS=(--model "$OPENCODE_MODEL")
fi
opencode run --auto "${MODEL_ARGS[@]}" "$PROMPT"
[[ -f RELEASE_NOTES.md ]] || die "RELEASE_NOTES.md was not generated"

echo "==> Updating release $TAG body..."
gh release edit "$TAG" --repo "$REPO" --notes-file RELEASE_NOTES.md

# --- update Homebrew formula -------------------------------------------------------
update_homebrew() {
  local tmp="$1" macos_amd64 macos_arm64 linux_amd64 linux_arm64 sha file
  gh repo clone rcieri/homebrew-glab-tui "$tmp/homebrew-glab-tui" >/dev/null
  cd "$tmp/homebrew-glab-tui"

  for arch in macos-amd64 macos-arm64 linux-amd64 linux-arm64; do
    file="$tmp/glab-tui-${arch}.tar.gz"
    echo "==> Fetching glab-tui-${arch}.tar.gz..."
    curl -sL "https://github.com/$REPO/releases/download/$TAG/glab-tui-${arch}.tar.gz" -o "$file"
    sha="$(sha256sum "$file" | cut -d' ' -f1)"
    case "$arch" in
      macos-amd64) macos_amd64=$sha ;;
      macos-arm64) macos_arm64=$sha ;;
      linux-amd64) linux_amd64=$sha ;;
      linux-arm64) linux_arm64=$sha ;;
    esac
  done

  sed -i "s|/download/v[0-9.]*/glab-tui-|/download/${TAG}/glab-tui-|g" Formula/glab-tui.rb
  sed -i "/glab-tui-macos-amd64/,/sha256/{s/sha256 \"[a-f0-9]*\"/sha256 \"${macos_amd64}\"/}" Formula/glab-tui.rb
  sed -i "/glab-tui-macos-arm64/,/sha256/{s/sha256 \"[a-f0-9]*\"/sha256 \"${macos_arm64}\"/}" Formula/glab-tui.rb
  sed -i "/glab-tui-linux-amd64/,/sha256/{s/sha256 \"[a-f0-9]*\"/sha256 \"${linux_amd64}\"/}" Formula/glab-tui.rb
  sed -i "/glab-tui-linux-arm64/,/sha256/{s/sha256 \"[a-f0-9]*\"/sha256 \"${linux_arm64}\"/}" Formula/glab-tui.rb

  git add Formula/glab-tui.rb
  if git diff --cached --quiet; then
    echo "==> Homebrew formula already up to date"
  else
    git -c user.name="opencode-release[bot]" \
        -c user.email="opencode-release[bot]@users.noreply.github.com" \
        commit -m "Update to ${TAG}" >/dev/null
    git push
    echo "==> Homebrew formula updated and pushed"
  fi
}

# --- update Scoop manifest ----------------------------------------------------------
update_scoop() {
  local tmp="$1" version sha
  gh repo clone rcieri/scoop-glab-tui "$tmp/scoop-glab-tui" >/dev/null
  cd "$tmp/scoop-glab-tui"

  version="${TAG#v}"
  echo "==> Fetching glab-tui-windows-amd64.zip..."
  curl -sL "https://github.com/$REPO/releases/download/$TAG/glab-tui-windows-amd64.zip" -o "$tmp/glab-tui-windows-amd64.zip"
  sha="$(sha256sum "$tmp/glab-tui-windows-amd64.zip" | cut -d' ' -f1)"

  jq --arg v "$version" --arg sha "$sha" \
    '.version = $v | .architecture."64bit".url = "https://github.com/rcieri/glab-tui/releases/download/v\($v)/glab-tui-windows-amd64.zip" | .architecture."64bit".hash = $sha' \
    bucket/glab-tui.json > bucket/glab-tui.json.tmp
  mv bucket/glab-tui.json.tmp bucket/glab-tui.json

  git add bucket/glab-tui.json
  if git diff --cached --quiet; then
    echo "==> Scoop manifest already up to date"
  else
    git -c user.name="opencode-release[bot]" \
        -c user.email="opencode-release[bot]@users.noreply.github.com" \
        commit -m "Update to ${TAG}" >/dev/null
    git push
    echo "==> Scoop manifest updated and pushed"
  fi
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
update_homebrew "$TMP"
update_scoop "$TMP"

echo "==> Post-release tasks complete for $TAG"
