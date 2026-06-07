# AI Agent Instructions for `glab-tui`

Welcome, AI Agent! This document contains essential context, architectural guidelines, and coding standards for navigating and contributing to `glab-tui`. Please adhere to these rules when analyzing the codebase, writing new features, or refactoring.

## 1. Project Overview

`glab-tui` is a Terminal User Interface (TUI) for managing GitLab and GitHub repositories. 
Instead of implementing full REST/GraphQL API clients, **`glab-tui` shells out to the official `glab` and `gh` CLIs** under the hood.

* **Primary Language:** Rust (Edition 2024)
* **TUI Framework:** `ratatui` (v0.29.0)
* **Async Runtime:** `tokio`
* **Terminal Handling:** `crossterm`

### Dual-Engine Architecture
The application detects whether the current repository is hosted on GitHub or GitLab (via `git remote get-url origin`). It translates GitLab-style API endpoints (`/projects/...`) to GitHub-style API endpoints (`/repos/...`) on the fly inside `GitlabClient` (`src/gitlab/client.rs`). 

**Rule:** Always write domain logic assuming the GitLab API schema. `GitlabClient` will handle the translation to `gh api` output formatting automatically.

## 2. Directory Structure

* `src/main.rs`: Entry point. Sets up the terminal, initializes the `App`, handles the main `tokio` event loop, routes keypresses, and delegates UI rendering.
* `src/app.rs`: Contains the global `App` state, data models for UI components (`EditMenu`, `Selector`, `DiffView`), and fuzzy-filtering logic.
* `src/event.rs`: Defines the `Event` enum and the async `EventHandler` using `tokio::sync::mpsc`.
* `src/ui.rs`: The purely functional rendering layer. Translates `App` state into `ratatui` widgets. Contains the global `THEME` constants.
* `src/gitlab/`: Domain modules (Issues, Merge Requests, Pipelines, Jobs, Runners, Releases, Notifications, Milestones, Wiki).
    * `client.rs`: The core wrapper around `gh api` and `glab api`.
* `src/utils/`: 
    * `cache.rs`: Offline caching logic for repository context and API payloads.
    * `format.rs`: Time parsing, markdown rendering, and string truncation.
    * `ui.rs`: Wrappers for `ratatui` stateful lists and tables.
    * `update.rs`: GitHub releases self-updater logic.

## 3. Core Architectural Patterns

### State Management (`App`)
* **Single Source of Truth:** All application state lives in the `App` struct inside `app.rs`.
* **No Blocking in UI:** `ui::render` is called on every tick. Never perform I/O, API calls, or heavy computation inside `ui.rs`.

### Event Loop & Async Operations
* User input (`crossterm` events) and background task results communicate with the main loop via the `Event` enum over a `tokio::sync::mpsc::UnboundedSender`.
* **Adding an API Call:** When adding a new API call:
    1. Spawn a `tokio::spawn` task in `main.rs` (on keypress) or `app.rs`.
    2. Make the API call using `app.gitlab_client`.
    3. Send an `Event` back to the main thread (e.g., `tx.send(Event::MyDataFetched(data))`).
    4. Handle the event in the `main.rs` event loop to update `app` state.

### External Editor Integration
* The application pauses the UI to open an external `$EDITOR` (or `$VISUAL`, defaulting to `helix`).
* This is done using `crossterm::terminal::LeaveAlternateScreen`. See `edit_in_editor` in `main.rs` for the boilerplate. Do not reinvent this wheel.

## 4. UI & Rendering Guidelines (`ratatui`)

* **Colors & Theming:** Always use the constants defined in the `THEME` struct located in `src/ui.rs` (e.g., `THEME.bg`, `THEME.green`, `THEME.highlight_bg`). Do not hardcode raw RGB values unless implementing a specific unique component like the hashed label colors.
* **Fuzzy Matching:** Use `SkimMatcherV2` from the `fuzzy-matcher` crate for filtering tables and selector overlays. The `render_fuzzy_cell` helper in `ui.rs` handles highlighting matched characters in yellow.
* **Columns:** Table columns are dynamically configurable. Always check `app.is_column_visible(tab, "Column Name")` before rendering a cell or header.
* **Layout:** Use `ratatui::layout::Layout` to split screens. Avoid hardcoded fixed sizes where possible, use `Constraint::Percentage` or `Constraint::Fill(1)`.

## 5. Adding a New Feature (Workflow)

If asked to add a new Tab (e.g., "Deployments"):
1.  **Update State:** Add the tab to the `Tab` enum in `app.rs` (include it in `ALL`, `title()`, `columns()`, and `default_columns()`). Add a `StatefulTable<Deployment>` to `App`.
2.  **Define Domain Logic:** Create `src/gitlab/deployments.rs`. Define the `Deployment` struct with `serde` traits. Write a `list_deployments` function that uses `GitlabClient::fetch_api`.
3.  **Create Events:** Add `DeploymentsFetched(Vec<Deployment>)` to the `Event` enum in `event.rs`.
4.  **Handle Data Fetching:** In `main.rs`, update `spawn_refresh_active_tab` to fetch data for the new tab.
5.  **Handle UI Updates:** In `main.rs`, handle the `Event::DeploymentsFetched` to update `app.deployments.items` and trigger cache saving.
6.  **Handle Navigation:** In `main.rs`, handle `KeyCode::Down`/`Up` to navigate the table state.
7.  **Render:** In `ui.rs`, add a branch to `match app.active_tab` to construct the rows, table, and details preview pane.

## 6. Development & Quality Standards

* **Error Handling:** Use `anyhow::Result`. Bubble up errors and display them in the UI via `app.error_message`. Do not `unwrap()` or `panic!()` in UI or event handling code.
* **Dependencies:** Do not add large dependencies (like `reqwest` or `hyper`) for HTTP API calls. The architecture strictly dictates delegating HTTP requests to `gh` and `glab` CLI binaries via `tokio::process::Command` in `GitlabClient`.
* **Format & Lint:** Run `cargo fmt` and `cargo clippy -- -D warnings` before providing code. The CI enforces zero clippy warnings.
* **MSRV:** The Minimum Supported Rust Version is `1.88`. Ensure code is compatible.