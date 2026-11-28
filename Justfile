# glab-tui task runner

# Default recipe: list available recipes
default:
    @just --list

# Build the release binary (target/release/glab-tui)
build:
    cargo build --release

# Run a debug build, passing arguments to glab-tui (e.g. just run -r owner/repo)
run *args:
    cargo run -- {{args}}

# Build, then copy the binary to the first directory in PATH
install: build
    @dest="${PATH%%:*}"; cp target/release/glab-tui "$dest/" && echo "Installed glab-tui to $dest"

# Format code with rustfmt
fmt:
    cargo fmt --all

# Check formatting without modifying files (as CI runs it)
fmt-check:
    cargo fmt --all -- --check

# Run Clippy with warnings denied (as CI runs it)
lint:
    RUSTFLAGS="-Dwarnings" cargo clippy --all-targets --all-features

# Run unit tests
test:
    cargo test --lib --bin glab-tui

# Run end-to-end tests, one process per test, then check that every entry of tests/api-lists.json was hit (requires cargo-nextest)
e2e:
    #!/usr/bin/env bash
    set -euo pipefail
    GLAB_TUI_API_HITS_DIR="$(mktemp -d)"
    export GLAB_TUI_API_HITS_DIR
    trap 'rm -rf "$GLAB_TUI_API_HITS_DIR"' EXIT
    cargo nextest run --test e2e --no-fail-fast
    cargo nextest run --test e2e --run-ignored ignored-only --success-output final \
        -E 'test(=api_lists::every_list_entry_was_hit_by_some_test)'

# Check advisories, licenses, duplicate versions and sources (requires cargo-deny)
deny:
    cargo deny check

# Fuzz one parser: diff_parser, markdown or ansi_trace (requires nightly and cargo-fuzz)
fuzz target seconds="60":
    cargo +nightly fuzz run {{target}} -- -max_total_time={{seconds}}

# Regenerate tests/api-descriptions from the pinned API descriptions (requires python3, curl, PyYAML)
gen-api-descriptions:
    python3 scripts/gen-api-descriptions.py

# Test the API description generator (requires python3)
test-api-descriptions:
    python3 -m unittest discover -s scripts -p 'test_*.py'

# Check that AGENTS.md stays within the ceiling in scripts/check-agents-md-size.sh
agents-md-size:
    bash scripts/check-agents-md-size.sh

# Run all CI checks locally (AGENTS.md size, fmt check, clippy, unit tests, e2e tests)
check: agents-md-size fmt-check lint test e2e

# Generate code coverage summary (requires cargo-llvm-cov)
cov:
    cargo llvm-cov --all-features --workspace --summary-only

# Generate code coverage lcov report (requires cargo-llvm-cov)
cov-lcov:
    cargo llvm-cov --all-features --workspace --lcov --output-path lcov.info

# Regenerate demo GIFs using VHS (requires vhs, ttyd, ffmpeg)
demos:
    bash assets/generate-demos.sh
