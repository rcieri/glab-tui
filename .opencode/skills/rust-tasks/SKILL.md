---
name: rust-tasks
description: Use when working on Rust code — building, testing, linting, formatting, refactoring, or adding dependencies. Covers cargo commands, clippy, edition 2024 patterns, and MSRV 1.85 compatibility.
---

## Rust workflow for glab-tui

### Build & check
```bash
cargo check
cargo build
cargo build --release
```

### Lint & format (required before every submission)
```bash
cargo fmt
cargo clippy -- -D warnings
```

### Test
```bash
cargo test
cargo test -- --nocapture  # for verbose output
```

### Adding a dependency
Add to `Cargo.toml` under `[dependencies]` (not `[dev-dependencies]` unless it's a test-only crate). Run `cargo build` after to verify resolution.

### Key crate patterns in this project
| Crate | Usage |
|-------|-------|
| `ratatui` | TUI rendering — `Table`, `List`, `Paragraph`, `Layout`, `Constraint` |
| `crossterm` | Terminal raw mode, key events, alternate screen |
| `tokio` | Async runtime with `tokio::spawn` for background API calls |
| `syntect` | Syntax highlighting — `SyntaxSet::load_defaults_newlines()`, `ThemeSet::load_defaults()` |
| `fuzzy-matcher` | `SkimMatcherV2` for column filtering |
| `toml` | Config/theme parsing |
| `serde` + `serde_json` | API response deserialization |
| `anyhow` | Error handling — prefer `anyhow::Result` |

### Edition 2024 notes
- `unsafe` blocks in `extern` blocks are no longer allowed
- `fn` pointer types with `unsafe` now need explicit `unsafe` qualifier
- New `gen` keyword is reserved
- Use `use foo::Bar;` not `extern crate foo;`
