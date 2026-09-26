# glab-tui task runner

# Default recipe: list available recipes
default:
    @just --list

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
    cargo test --bin glab-tui

# Run end-to-end tests (single-threaded)
e2e:
    cargo test --test e2e -- --test-threads=1

# Run all CI checks locally (fmt check, clippy, unit tests, e2e tests)
check: fmt-check lint test e2e

# Generate code coverage summary (requires cargo-llvm-cov)
cov:
    cargo llvm-cov --all-features --workspace --summary-only

# Generate code coverage lcov report (requires cargo-llvm-cov)
cov-lcov:
    cargo llvm-cov --all-features --workspace --lcov --output-path lcov.info

# Regenerate demo GIFs using VHS (requires vhs, ttyd, ffmpeg)
demos:
    bash assets/generate-demos.sh
