#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
CONFIG="$ROOT/.glab-tui/config.toml"

cd "$ROOT"

generate() {
    local tape="$1" theme="$2"
    echo "=== Generating $(basename "$tape" .tape) (theme: $theme) ==="
    printf 'theme_preset = "%s"\n' "$theme" > "$CONFIG"
    rm -f "${tape%.tape}.gif"
    vhs "$tape" 2>&1 | tail -1
    echo ""
}

mkdir -p "$ROOT/.glab-tui"

generate "$ROOT/assets/demo-overview.tape"  "default"
generate "$ROOT/assets/demo-search.tape"    "tokyo-night"
generate "$ROOT/assets/demo-selection.tape" "gruvbox"

echo "=== All demos generated ==="
ls -lh "$ROOT/assets/"*.gif
