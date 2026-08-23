#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

# Ensure binary is built and available on PATH
if [[ -x "$ROOT/target/release/glab-tui" ]]; then
    export PATH="$ROOT/target/release:$PATH"
elif ! command -v glab-tui >/dev/null 2>&1; then
    echo "==> Building glab-tui in release mode..."
    cargo build --release
    export PATH="$ROOT/target/release:$PATH"
fi

ORIG_HOME="$HOME"
TMP_BASE="$(mktemp -d "${TMPDIR:-/tmp}/glab-tui-demos.XXXXXX")"
MAX_JOBS="${JOBS:-2}"

# Track background PIDs for graceful termination
ACTIVE_PIDS=()

cleanup() {
    # Disable traps to prevent re-entry
    trap - EXIT INT TERM

    # Terminate any lingering child processes
    if [[ ${#ACTIVE_PIDS[@]} -gt 0 ]]; then
        for pid in "${ACTIVE_PIDS[@]}"; do
            kill "$pid" 2>/dev/null || true
        done
        wait 2>/dev/null || true
    fi

    # Clean up worktrees
    for wt in "$TMP_BASE"/*/wt; do
        if [[ -d "$wt" ]]; then
            git worktree remove --force "$wt" 2>/dev/null || true
        fi
    done
    git worktree prune 2>/dev/null || true
    rm -rf "$TMP_BASE"
}
trap cleanup EXIT INT TERM

DEMOS=(
    "assets/demo-overview.tape:default"
    "assets/demo-search.tape:rose-pine-dawn"
    "assets/demo-selection.tape:gruvbox"
    "assets/demo-preview.tape:nord"
    "assets/demo-diff.tape:catppuccin-mocha"
)

mkdir -p "$ROOT/assets"

echo "==> Preparing environments for ${#DEMOS[@]} demos..."

# Pre-setup worktrees and isolated home/cache directories sequentially to prevent git lock contention
for entry in "${DEMOS[@]}"; do
    tape="${entry%%:*}"
    theme="${entry##*:}"
    name="$(basename "$tape" .tape)"

    demo_dir="$TMP_BASE/$name"
    wt_dir="$demo_dir/wt"
    home_dir="$demo_dir/home"
    tmp_dir="$demo_dir/tmp"

    mkdir -p "$demo_dir" "$tmp_dir" "$home_dir/.config" "$home_dir/.cache"

    # Seed authentication and cache from real HOME into isolated environment to prevent cache race conditions
    if [[ -d "$ORIG_HOME/.config/gh" ]]; then
        cp -r "$ORIG_HOME/.config/gh" "$home_dir/.config/" 2>/dev/null || true
    fi
    if [[ -d "$ORIG_HOME/.config/glab-cli" ]]; then
        cp -r "$ORIG_HOME/.config/glab-cli" "$home_dir/.config/" 2>/dev/null || true
    fi
    if [[ -d "$ORIG_HOME/.cache/glab-tui" ]]; then
        cp -r "$ORIG_HOME/.cache/glab-tui" "$home_dir/.cache/" 2>/dev/null || true
    fi
    if [[ -d "$ORIG_HOME/.config/glab-tui/themes" ]]; then
        mkdir -p "$home_dir/.config/glab-tui"
        cp -r "$ORIG_HOME/.config/glab-tui/themes" "$home_dir/.config/glab-tui/" 2>/dev/null || true
    fi

    # Create isolated git worktree
    git worktree add --detach "$wt_dir" HEAD >/dev/null 2>&1

    mkdir -p "$wt_dir/.glab-tui" "$wt_dir/assets"
    printf 'theme_preset = "%s"\n' "$theme" > "$wt_dir/.glab-tui/config.toml"
    cp "$ROOT/assets/${name}.tape" "$wt_dir/assets/"
done

echo "=== Generating ${#DEMOS[@]} demos (max $MAX_JOBS in parallel) ==="
failed=0

run_demo() {
    local entry="$1"
    local tape="${entry%%:*}"
    local theme="${entry##*:}"
    local name
    name="$(basename "$tape" .tape)"

    local demo_dir="$TMP_BASE/$name"
    local wt_dir="$demo_dir/wt"
    local home_dir="$demo_dir/home"
    local tmp_dir="$demo_dir/tmp"
    local log_file="$TMP_BASE/${name}.log"

    echo "==> [START] Generating $name (theme: $theme)..."
    (
        export HOME="$home_dir"
        export XDG_CONFIG_HOME="$home_dir/.config"
        export XDG_CACHE_HOME="$home_dir/.cache"
        export TMPDIR="$tmp_dir"

        # Prevent headless Chrome from attaching to user's display / Wayland compositor
        unset WAYLAND_DISPLAY
        export LIBGL_ALWAYS_SOFTWARE=1
        export QT_QPA_PLATFORM=offscreen

        cd "$wt_dir"
        rm -f "assets/${name}.gif"

        if vhs "assets/${name}.tape" > "$log_file" 2>&1 && [[ -f "assets/${name}.gif" ]]; then
            mv "assets/${name}.gif" "$ROOT/assets/${name}.gif"
            echo "✓ [DONE] Generated $name"
        else
            echo "✗ [ERROR] Failed to generate $name (log: $log_file)" >&2
            cat "$log_file" >&2
            exit 1
        fi
    ) < /dev/null
}

# Worker queue to run at most $MAX_JOBS in parallel
for entry in "${DEMOS[@]}"; do
    run_demo "$entry" &
    ACTIVE_PIDS+=($!)

    # If we reached MAX_JOBS concurrent processes, wait for one to finish
    while [[ ${#ACTIVE_PIDS[@]} -ge $MAX_JOBS ]]; do
        new_pids=()
        for pid in "${ACTIVE_PIDS[@]}"; do
            if kill -0 "$pid" 2>/dev/null; then
                new_pids+=("$pid")
            else
                if ! wait "$pid"; then
                    failed=$((failed + 1))
                fi
            fi
        done
        ACTIVE_PIDS=("${new_pids[@]}")
        if [[ ${#ACTIVE_PIDS[@]} -ge $MAX_JOBS ]]; then
            sleep 0.5
        fi
    done
done

# Wait for remaining active jobs
for pid in "${ACTIVE_PIDS[@]}"; do
    if ! wait "$pid"; then
        failed=$((failed + 1))
    fi
done
ACTIVE_PIDS=()

if (( failed > 0 )); then
    echo "=== $failed demo(s) failed to generate ===" >&2
    exit 1
fi

echo "=== All demos generated successfully ==="
ls -lh "$ROOT/assets/"*.gif
