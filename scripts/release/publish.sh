#!/usr/bin/env bash
set -euo pipefail

# Locally publishes a release: pushes the Docker image to GHCR and the crate to
# crates.io. Replaces the docker-push and publish-crate jobs of the old
# .github/workflows/release.yml. Run after the tag build has published the
# GitHub release, or any time after the version bump has been merged and tagged.

TAG="${1:?usage: scripts/release/publish.sh <tag> e.g. v0.9.0}"
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

REPO="rcieri/glab-tui"

die() { printf 'error: %s\n' "$*" >&2; exit 1; }
require() { command -v "$1" >/dev/null 2>&1 || die "missing required tool '$1' (${2:-})"; }

require cargo "install Rust via https://rustup.rs"
require jq "apt install jq / brew install jq"
require docker "see https://docs.docker.com/get-docker/"
require gh "see https://cli.github.com"

gh auth status >/dev/null 2>&1 || die "not authenticated with gh; run 'gh auth login' first"

# --- verify Cargo version matches the tag ------------------------------------------
PACKAGE_VERSION="$(cargo metadata --format-version 1 --no-deps 2>/dev/null | jq -r '.packages[0].version')"
TAG_VERSION="${TAG#v}"
if [[ "$PACKAGE_VERSION" != "$TAG_VERSION" ]]; then
  die "Cargo package version ($PACKAGE_VERSION) does not match tag version ($TAG_VERSION)"
fi
echo "==> Cargo version $PACKAGE_VERSION matches tag $TAG"

# --- push Docker image to GHCR -------------------------------------------------------
echo "==> Logging in to ghcr.io..."
USER="$(gh api user --jq .login)"
gh auth token | docker login ghcr.io -u "$USER" --password-stdin

TAGS_ARGS=(-t "ghcr.io/$REPO:$TAG")
if [[ "$TAG" != *-* ]]; then
  TAGS_ARGS+=(-t "ghcr.io/$REPO:latest")
fi

echo "==> Building and pushing Docker image..."
docker buildx build --push "${TAGS_ARGS[@]}" .

# --- publish crate to crates.io --------------------------------------------------------
echo "==> Publishing crate v$PACKAGE_VERSION to crates.io..."
cargo publish --locked

echo "==> Publish complete for $TAG"
