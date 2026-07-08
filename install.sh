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

main() {
    platform=$(detect_os_arch)
    os="${platform%-*}"
    arch="${platform#*-}"

    case "$os" in
        windows) ext=".zip" ;;
        *)       ext=".tar.gz" ;;
    esac

    asset="glab-tui-${os}-${arch}${ext}"

    echo "Fetching latest release for ${platform}..."

    json=$(fetch_latest_release)
    tag=$(echo "$json" | grep '"tag_name"' | sed 's/.*"tag_name": "\(.*\)",.*/\1/')
    download_url=$(echo "$json" | grep '"browser_download_url"' | grep "/${asset}\"" | sed 's/.*"browser_download_url": "\(.*\)".*/\1/')

    if [ -z "$download_url" ]; then
        echo "No asset found for ${platform}" >&2
        exit 1
    fi

    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT INT TERM

    echo "Downloading ${asset}..."
    if command -v curl >/dev/null 2>&1; then
        curl -sSfL "$download_url" -o "${tmpdir}/${asset}"
    else
        wget -q "$download_url" -O "${tmpdir}/${asset}"
    fi

    echo "Extracting..."
    case "$os" in
        windows)
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
