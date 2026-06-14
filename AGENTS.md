# AI Agent Instructions for `glab-tui`

Welcome, AI Agent! This document contains essential context, architectural guidelines, and coding standards for navigating and contributing to `glab-tui`. Please adhere to these rules when analyzing the codebase, writing new features, or refactoring.

## 1. Project Overview

`glab-tui` is a Terminal User Interface (TUI) for managing GitLab and GitHub repositories. 
Instead of implementing full REST/GraphQL API clients, **`glab-tui` shells out to the official `glab` and `gh` CLIs** under the hood.

* **Primary Language:** Rust (Edition 2024)
* **TUI Framework:** `ratatui` (v0.30.1)
* **Syntax Highlighting:** `syntect` (v5, `default-fancy` features)
* **Async Runtime:** `tokio`
* **Terminal Handling:** `crossterm`

### Dual-Engine Architecture
The application detects whether the current repository is hosted on GitHub or GitLab (via `git remote get-url origin`). It translates GitLab-style API endpoints (`/projects/...`) to GitHub-style API endpoints (`/repos/...`) on the fly inside `GitlabClient` ([src/gitlab/client.rs](src/gitlab/client.rs)). 

**Rule:** Always write domain logic assuming the GitLab API schema. `GitlabClient` will handle the translation to `gh api` output formatting automatically.

## 2. Directory Structure

* [src/main.rs](src/main.rs): Entry point. Sets up the terminal, initializes the `App`, handles the main `tokio` event loop, routes keypresses, and delegates UI rendering.
* [src/app.rs](src/app.rs): Contains the global `App` state, data models for UI components (`EditMenu`, `Selector`, `DiffView`), and fuzzy-filtering logic.
* [src/event.rs](src/event.rs): Defines the `Event` enum and the async `EventHandler` using `tokio::sync::mpsc`.
* [src/ui.rs](src/ui.rs): The purely functional rendering layer. Translates `App` state into `ratatui` widgets. Contains the global `THEME` constants.
* [src/gitlab/](src/gitlab/): Domain modules interfacing with Git CLI wrapper.
    * [client.rs](src/gitlab/client.rs): The core wrapper around `gh api` and `glab api`.
    * [issues.rs](src/gitlab/issues.rs): Issue structures and API integration.
    * [mr.rs](src/gitlab/mr.rs): Merge/Pull request structures and logic.
    * [pipelines.rs](src/gitlab/pipelines.rs): Pipeline and Job data models.
    * [runners.rs](src/gitlab/runners.rs): Runner configurations and actions.
    * [releases.rs](src/gitlab/releases.rs): Release listings and metadata.
    * [notifications.rs](src/gitlab/notifications.rs): GitLab todos and GitHub notifications.
    * [milestones.rs](src/gitlab/milestones.rs): Milestone configurations.
* [src/utils/](src/utils/):
    * [cache.rs](src/utils/cache.rs): Offline caching logic for repository context and API payloads.
    * [format.rs](src/utils/format.rs): Time parsing, markdown rendering, and string truncation.
    * [ui.rs](src/utils/ui.rs): Wrappers for `ratatui` stateful lists and tables.
    * [update.rs](src/utils/update.rs): GitHub releases self-updater logic.

## 3. Core Architectural Patterns

### State Management (`App`)
* **Single Source of Truth:** All application state lives in the `App` struct inside [src/app.rs](src/app.rs).
* **No Blocking in UI:** `ui::render` is called on every tick. Never perform I/O, API calls, or heavy computation inside [src/ui.rs](src/ui.rs).

### Event Loop & Async Operations
* User input (`crossterm` events) and background task results communicate with the main loop via the `Event` enum over a `tokio::sync::mpsc::UnboundedSender`.
* **Adding an API Call:** When adding a new API call:
    1. Spawn a `tokio::spawn` task in [src/main.rs](src/main.rs) (on keypress) or [src/app.rs](src/app.rs).
    2. Make the API call using `app.gitlab_client`.
    3. Send an `Event` back to the main thread (e.g., `tx.send(Event::MyDataFetched(data))`).
    4. Handle the event in the [src/main.rs](src/main.rs) event loop to update `app` state.

### External Editor Integration
* The application pauses the UI to open an external `$EDITOR` (or `$VISUAL`, defaulting to `helix`).
* This is done using `crossterm::terminal::LeaveAlternateScreen`. See `edit_in_editor` in [src/main.rs](src/main.rs) for the boilerplate. Do not reinvent this wheel.

### Syntax Highlighting (`syntect`)
* Line-level syntax highlighting is computed at diff-parse time in `DiffView::new` ([src/app.rs](src/app.rs)).
* `SYNTAX_SET` and `THEME_SET` are global `LazyLock` statics using `SyntaxSet::load_defaults_newlines()` and `ThemeSet::load_defaults()`.
* The public function `highlight_line_syntax(file_path, line_content, ext)` returns `Option<Vec<(ratatui::style::Style, String)>>`.
* `syntect_style_to_ratatui()` converts `syntect::highlighting::Style` → `ratatui::style::Style`.
* `DiffLine` contains an optional `syntax_highlighted: Option<Vec<(Style, String)>>` field populated during parsing.

### Code Review System
* **Diff view** supports inline comments, code suggestions, and draft reviews:
  - `DiscussionNote` / `NotePosition` structs in [src/gitlab/mr.rs](src/gitlab/mr.rs).
  - `list_mr_notes()` fetches notes for an MR via the API.
  - Draft comments are stored in `app.draft_comments: Vec<DraftComment>` and submitted atomically.
  - Current (already-pushed) comments live in `app.current_comments: Vec<DiscussionNote>`.
  - `DiffFetched` event now uses named fields: `{ mr_iid, raw_diff, comments }`.
* **Suggestion rendering:** `format_comment_with_suggestions()` in [src/ui.rs](src/ui.rs) parses ` ```suggestion ` blocks from comment bodies and renders them as in-line diff (red for original, green for suggested).

### Cache & State Persistence
* Cache directory: `~/.cache/glab-tui/` (migrated from `~/.glab-tui-cache`).
* `ProjectCache` now stores `enabled_columns`, `group_by_column`, `group_ascending`, and `column_filters` in addition to API data.
* Cache is written on every successful data fetch; read on startup.

### Column Configure Popup
* The configure overlay (`Tab`/`t`) has three sections: **COLUMNS** (checkbox toggle), **GROUP BY** (single-select), and **ORDER** (Ascending/Descending).
* Value-based column filtering is available by pressing `Enter` on a focused column item, which opens a selector overlay with distinct values for that column.
* Column filter state is tracked via `app.column_filter_context` and `app.column_filters: HashMap<Tab, HashMap<String, Vec<String>>>`.
* Group state is tracked via `app.group_by_column: Option<String>` and `app.group_ascending: bool`.

## 4. UI & Rendering Guidelines (`ratatui`)

* **Colors & Theming:** Always use the constants defined in the `THEME` struct located in [src/ui.rs](src/ui.rs) (e.g., `THEME.bg`, `THEME.green`, `THEME.highlight_bg`). Do not hardcode raw RGB values unless implementing a specific unique component like the hashed label colors.
* **Fuzzy Matching:** Use `SkimMatcherV2` from the `fuzzy-matcher` crate for filtering tables and selector overlays. The `render_fuzzy_cell` helper in [src/ui.rs](src/ui.rs) handles highlighting matched characters in yellow.
* **Columns:** Table columns are dynamically configurable. Always check `app.is_column_visible(tab, "Column Name")` before rendering a cell or header.
* **Layout:** Use `ratatui::layout::Layout` to split screens. Avoid hardcoded fixed sizes where possible, use `Constraint::Percentage` or `Constraint::Fill(1)`.

## 5. Adding a New Feature (Workflow)

If asked to add a new Tab (e.g., "Deployments"):
1.  **Update State:** Add the tab to the `Tab` enum in [src/app.rs](src/app.rs) (include it in `ALL`, `title()`, `columns()`, and `default_columns()`). Add a `StatefulTable<Deployment>` to `App`.
2.  **Define Domain Logic:** Create [src/gitlab/deployments.rs](src/gitlab/deployments.rs). Define the `Deployment` struct with `serde` traits. Write a `list_deployments` function that uses `GitlabClient::fetch_api`.
3.  **Create Events:** Add `DeploymentsFetched(Vec<Deployment>)` to the `Event` enum in [src/event.rs](src/event.rs).
4.  **Handle Data Fetching:** In [src/main.rs](src/main.rs), update `spawn_refresh_active_tab` to fetch data for the new tab.
5.  **Handle UI Updates:** In [src/main.rs](src/main.rs), handle the `Event::DeploymentsFetched` to update `app.deployments.items` and trigger cache saving.
6.  **Handle Navigation:** In [src/main.rs](src/main.rs), handle `KeyCode::Down`/`Up` to navigate the table state.
7.  **Render:** In [src/ui.rs](src/ui.rs), add a branch to `match app.active_tab` to construct the rows, table, and details preview pane.

## 6. Development & Quality Standards

* **Error Handling:** Use `anyhow::Result`. Bubble up errors and display them in the UI via `app.error_message`. Do not `unwrap()` or `panic!()` in UI or event handling code.
* **Dependencies:** Do not add large dependencies (like `reqwest` or `hyper`) for HTTP API calls. The architecture strictly dictates delegating HTTP requests to `gh` and `glab` CLI binaries via `tokio::process::Command` in `GitlabClient`.
* **Format & Lint:** Run `cargo fmt` and `cargo clippy -- -D warnings` before providing code. The CI enforces zero clippy warnings.
* **MSRV:** The Minimum Supported Rust Version is `1.85` (as required by edition 2024). Ensure code is compatible.