#!/bin/sh
set -eu

REPO="${REPO:-rcieri/glab-tui}"
PREFIX="${PREFIX:-$HOME/.local/bin}"

detect_os_arch() {
    os=$(uname -s | tr '[:upper:]' '[:lower:]')
    arch=$(uname -m)

    case "$os" in
        linux)  os="linux" ;;
        darwin) os="macos" ;;
        mingw*|msys*|cygwin*) os="windows" ;;
        *)
            echo "Unsupported OS: $os" >&2
            exit 1
            ;;
    esac

    case "$arch" in
        x86_64|amd64) arch="amd64" ;;
        aarch64|arm64) arch="arm64" ;;
        *)
            echo "Unsupported architecture: $arch" >&2
            exit 1
            ;;
    esac

    echo "${os}-${arch}"
}

# Detect Linux distro version (e.g. "ubuntu-22.04", "ubuntu-24.04").
# Returns empty string for unknown distros.
detect_linux_distro() {
    [ -r /etc/os-release ] || return 0
    # shellcheck disable=SC1091
    . /etc/os-release
    case "${ID:-}" in
        ubuntu)
            echo "ubuntu-${VERSION_ID:-}"
            ;;
        *)
            echo "${ID:-}"
            ;;
    esac
}

# Known Ubuntu LTS baselines, newest first. Used as the fallback chain when
# the local Ubuntu version isn't explicitly built for (or for non-Ubuntu
# distros that ship a glibc newer than 2.35).
ubuntu_lts_fallbacks() {
    echo "ubuntu-24.04"
    echo "ubuntu-22.04"
}

# Build the ordered list of Linux asset names to try, most-preferred first.
linux_asset_candidates() {
    local arch="$1" distro="$2"
    case "$distro" in
        ubuntu-*)
            # Try the local Ubuntu version first, then walk down the LTS chain,
            # and finally the fully-static musl build.
            echo "glab-tui-linux-${arch}-${distro}.tar.gz"
            ubuntu_lts_fallbacks | while IFS= read -r v; do
                [ "${distro}" != "${v}" ] && echo "glab-tui-linux-${arch}-${v}.tar.gz"
            done
            echo "glab-tui-linux-${arch}-musl.tar.gz"
            ;;
        *)
            echo "glab-tui-linux-${arch}-ubuntu-22.04.tar.gz"
            echo "glab-tui-linux-${arch}-ubuntu-24.04.tar.gz"
            echo "glab-tui-linux-${arch}-musl.tar.gz"
            ;;
    esac
}

fetch_latest_release() {
    url="https://api.github.com/repos/${REPO}/releases/latest"
    if [ -n "${GITHUB_TOKEN:-}" ]; then
        auth_header="Authorization: Bearer $GITHUB_TOKEN"
        if command -v curl >/dev/null 2>&1; then
            curl -sSfL -H "$auth_header" "$url"
        elif command -v wget >/dev/null 2>&1; then
            wget --header="$auth_header" -qO- "$url"
        else
            echo "Neither curl nor wget found" >&2
            exit 1
        fi
    else
        if command -v curl >/dev/null 2>&1; then
            curl -sSfL "$url"
        elif command -v wget >/dev/null 2>&1; then
            wget -qO- "$url"
        else
            echo "Neither curl nor wget found" >&2
            exit 1
        fi
    fi
}

# Pick the first asset from $release_json whose name appears in stdin (in order).
pick_asset_url() {
    local json="$1"
    while IFS= read -r candidate; do
        [ -z "$candidate" ] && continue
        url=$(echo "$json" | grep '"browser_download_url"' | grep "/${candidate}\"" | sed -n 's/.*"browser_download_url": *"\([^"]*\)".*/\1/p' | head -n 1)
        if [ -n "$url" ]; then
            echo "$url"
            return 0
        fi
    done
    return 1
}

download() {
    if command -v curl >/dev/null 2>&1; then
        curl -sSfL "$1" -o "$2"
    else
        wget -q "$1" -O "$2"
    fi
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        echo "Neither sha256sum nor shasum found; cannot verify the download" >&2
        return 1
    fi
}

# Compare an archive against the release's "<sha256>  <name>" checksum file.
verify_checksum() {
    archive="$1"
    checksum_file="$2"
    expected=$(head -n 1 "$checksum_file" | cut -d ' ' -f 1 | tr -d '\r' | tr '[:upper:]' '[:lower:]')
    actual=$(sha256_of "$archive") || exit 1
    if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
        echo "Checksum mismatch for $(basename "$archive"); refusing to install." >&2
        echo "  expected: ${expected:-<empty checksum file>}" >&2
        echo "  actual:   ${actual}" >&2
        exit 1
    fi
    echo "Checksum verified: sha256 ${actual}"
}

main() {
    platform=$(detect_os_arch)
    os="${platform%-*}"
    arch="${platform#*-}"

    case "$os" in
        windows) ext=".zip" ;;
        *)       ext=".tar.gz" ;;
    esac

    case "$os" in
        linux)
            distro=$(detect_linux_distro)
            candidates=$(linux_asset_candidates "$arch" "$distro")
            ;;
        *)
            candidates="glab-tui-${os}-${arch}${ext}"
            ;;
    esac

    if [ -n "${GLAB_TUI_ASSET:-}" ]; then
        echo "GLAB_TUI_ASSET override: $GLAB_TUI_ASSET"
        candidates="$GLAB_TUI_ASSET"
    fi

    echo "Fetching latest release for ${platform}..."

    json=$(fetch_latest_release)
    tag=$(echo "$json" | grep '"tag_name"' | sed 's/.*"tag_name": "\(.*\)",.*/\1/')

    if ! download_url=$(printf '%s\n' "$candidates" | pick_asset_url "$json"); then
        echo "No matching asset found. Tried:" >&2
        printf '  - %s\n' $candidates >&2
        exit 1
    fi
    asset=$(basename "$download_url")

    echo "Selected asset: $asset"

    if ! checksum_url=$(printf '%s\n' "${asset}.sha256" | pick_asset_url "$json"); then
        echo "Release ${tag} publishes no ${asset}.sha256; refusing to install an unverified binary" >&2
        exit 1
    fi

    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT INT TERM

    echo "Downloading ${asset}..."
    download "$download_url" "${tmpdir}/${asset}"
    download "$checksum_url" "${tmpdir}/${asset}.sha256"
    verify_checksum "${tmpdir}/${asset}" "${tmpdir}/${asset}.sha256"

    echo "Extracting..."
    case "$ext" in
        .zip)
            unzip -qo "${tmpdir}/${asset}" -d "$tmpdir"
            ;;
        *)
            tar -xzf "${tmpdir}/${asset}" -C "$tmpdir"
            ;;
    esac

    mkdir -p "$PREFIX"
    cp "${tmpdir}/glab-tui" "$PREFIX/glab-tui" 2>/dev/null || \
    cp "${tmpdir}/glab-tui.exe" "$PREFIX/glab-tui" 2>/dev/null || {
        echo "Binary not found in archive" >&2
        exit 1
    }
    chmod +x "$PREFIX/glab-tui"

    echo "Installed glab-tui ${tag} to ${PREFIX}/glab-tui"

    case :$PATH: in
        *:$PREFIX:*) ;;
        *) echo "Warning: ${PREFIX} is not in \$PATH. Add it to your shell profile:" ;;
    esac
}

main "$@"
