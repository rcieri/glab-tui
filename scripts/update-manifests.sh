#!/bin/sh
set -eu

TAG="${1:?Usage: $0 <tag> (e.g. v2.3.1)}"
VERSION="${TAG#v}"
REPO="${REPO:-rcieri/glab-tui}"

# Strip 'v' prefix for Scoop
SCOOP_VERSION="$VERSION"

case "$TAG" in
    v*) ;;
    *) echo "Tag must start with 'v' (e.g. v2.3.1)" >&2; exit 1 ;;
esac

echo "Updating manifests for $TAG (version $VERSION)..."

ASSETS="
glab-tui-linux-amd64.tar.gz
glab-tui-linux-arm64.tar.gz
glab-tui-macos-amd64.tar.gz
glab-tui-macos-arm64.tar.gz
glab-tui-windows-amd64.zip
"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT INT TERM

echo "Downloading release assets..."
for asset in $ASSETS; do
    url="https://github.com/${REPO}/releases/download/${TAG}/${asset}"
    curl -sSfL "$url" -o "${TMPDIR}/${asset}"
done

echo "Computing checksums..."
LINUX_AMD64=$(sha256sum "${TMPDIR}/glab-tui-linux-amd64.tar.gz" | cut -d' ' -f1)
LINUX_ARM64=$(sha256sum "${TMPDIR}/glab-tui-linux-arm64.tar.gz" | cut -d' ' -f1)
MACOS_AMD64=$(sha256sum "${TMPDIR}/glab-tui-macos-amd64.tar.gz" | cut -d' ' -f1)
MACOS_ARM64=$(sha256sum "${TMPDIR}/glab-tui-macos-arm64.tar.gz" | cut -d' ' -f1)
WINDOWS_AMD64=$(sha256sum "${TMPDIR}/glab-tui-windows-amd64.zip" | cut -d' ' -f1)

# ---- Update Homebrew formula ----
echo "Updating Formula/glab-tui.rb..."
cat > Formula/glab-tui.rb <<FORMULA
class GlabTui < Formula
  desc "Terminal user interface for GitLab and GitHub"
  homepage "https://github.com/rcieri/glab-tui"
  license "MIT"

  depends_on "gh"
  depends_on "glab" => :recommended

  on_macos do
    on_intel do
      url "https://github.com/${REPO}/releases/download/v${VERSION}/glab-tui-macos-amd64.tar.gz"
      sha256 "${MACOS_AMD64}"
    end
    on_arm do
      url "https://github.com/${REPO}/releases/download/v${VERSION}/glab-tui-macos-arm64.tar.gz"
      sha256 "${MACOS_ARM64}"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/${REPO}/releases/download/v${VERSION}/glab-tui-linux-amd64.tar.gz"
      sha256 "${LINUX_AMD64}"
    end
    on_arm do
      url "https://github.com/${REPO}/releases/download/v${VERSION}/glab-tui-linux-arm64.tar.gz"
      sha256 "${LINUX_ARM64}"
    end
  end

  def install
    bin.install "glab-tui"
  end

  test do
    system "\#{bin}/glab-tui", "--help"
  end
end
FORMULA

# ---- Update Scoop manifest ----
echo "Updating scoop/glab-tui.json..."
cat > scoop/glab-tui.json <<SCOOP
{
    "version": "${SCOOP_VERSION}",
    "description": "A terminal user interface for GitLab and GitHub",
    "homepage": "https://github.com/rcieri/glab-tui",
    "license": "MIT",
    "architecture": {
        "64bit": {
            "url": "https://github.com/${REPO}/releases/download/v${VERSION}/glab-tui-windows-amd64.zip",
            "hash": "sha256:${WINDOWS_AMD64}"
        }
    },
    "bin": "glab-tui.exe",
    "checkver": {
        "github": "https://github.com/rcieri/glab-tui"
    },
    "autoupdate": {
        "architecture": {
            "64bit": {
                "url": "https://github.com/${REPO}/releases/download/v\$version/glab-tui-windows-amd64.zip",
                "hash": {
                    "url": "\$url.sha256"
                }
            }
        }
    }
}
SCOOP

echo "Done! Manifests updated for $TAG"
