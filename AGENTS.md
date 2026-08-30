# AI Agent Instructions for `glab-tui`

Welcome, AI Agent! This document contains essential context, architectural guidelines, and coding standards for navigating and contributing to `glab-tui`. Please adhere to these rules when analyzing the codebase, writing new features, or refactoring.

## 1. Project Overview

`glab-tui` is a Terminal User Interface (TUI) for managing GitLab and GitHub repositories. 
Instead of implementing full REST/GraphQL API clients, **`glab-tui` shells out to the official `glab` and `gh` CLIs** under the hood.

* **Primary Language:** Rust (Edition 2024)
* **TUI Framework:** `ratatui` (v0.30.1)
* **Syntax Highlighting:** `syntect` (v5, `default-fancy` features)
* **Markdown Rendering:** `pulldown-cmark` (v0.13.0, GFM tables/code/lists)
* **Async Runtime:** `tokio` (v1.38, full)
* **Async Traits:** `async-trait` (v0.1.92)
* **CLI Parsing:** `clap` (v4, derive)
* **Terminal Handling:** `crossterm` (v0.29)
* **Config/Themes:** `toml` (v1.1) crate; config at `~/.config/glab-tui/config.toml`
* **YAML:** `serde_yaml` (v0.9) — diagnostics output
* **Package:** `glab-tui-crate` (binary: `glab-tui`; current version `v0.9.0`)

### Dual-Engine Architecture
The application detects whether the current repository is hosted on GitHub or GitLab and instantiates either a `GlabBackend` or `GhBackend`. Detection is centralized in `git_helpers::detect_backend(remote_url, override_kind)` ([src/git_helpers.rs](src/git_helpers.rs)): `github.com` remotes (with or without `www.` prefix) resolve to GitHub; other hosts are probed with `gh auth status --active --hostname <host>` and `glab auth status --hostname <host>`, defaulting to GitLab when neither/both respond. A repo-local `backend = "github" | "gitlab"` config override always takes precedence — set it for SSH aliases or hosts serving both platforms. Always route backend detection through `detect_backend`; do not reimplement inline `github.com` string matching. Both backends implement the `Backend` trait ([src/backend/mod.rs](src/backend/mod.rs)). The domain layer ([src/domain/](src/domain/)) calls backend methods through `GitlabClient` ([src/domain/client.rs](src/domain/client.rs)). Runtime backend identification is available via the `BackendKind` enum (`BackendKind::GitLab` / `BackendKind::GitHub`) which also provides host-aware terminology through `BackendKind::term()`.

The `namespace/project` context passed as `-R <repo>` to every `glab`/`gh` call is extracted from the remote URL by `git_helpers::parse_project_path` ([src/git_helpers.rs](src/git_helpers.rs)), which keeps every path segment after the host so nested GitLab subgroup namespaces (`group/subgroup/project`) resolve correctly. Always use this helper — do not reimplement remote-URL parsing inline.

**Rule:** Never use `glab api` or `gh api` when a native subcommand exists. Prefer native subcommands — they use built-in pagination, auth, and output formatting. Only fall back to raw API calls for endpoints with no native CLI equivalent.

## 2. Directory Structure

* [src/main.rs](src/main.rs): Entry point. Sets up the terminal, initializes the `App`, handles the main `tokio` event loop, routes keypresses (via `keybinding_matches()`), and delegates UI rendering.
* [src/app.rs](src/app.rs): Contains the global `App` state, data models for UI components (`EditMenu`, `SubmitDialog`, `Selector`, `DiffView`, `DatePicker`), and fuzzy-filtering logic.
* [src/config.rs](src/config.rs): Config, theme, and icons system. Defines `Config`, `Theme`, `ThemeOverrides`, `Icons`, and all `KeybindingXxx` structs.
* [src/event.rs](src/event.rs): Defines the `Event` enum and the async `EventHandler` using `tokio::sync::mpsc`.
* [src/backend/](src/backend/): CLI backend layer.
    * [mod.rs](src/backend/mod.rs): `Backend` trait with ~40 methods covering all API interactions.
    * [glab.rs](src/backend/glab.rs): `GlabBackend` — shells out to `glab` CLI.
    * [gh.rs](src/backend/gh.rs): `GhBackend` — shells out to `gh` CLI.
* [src/domain/](src/domain/): Domain models and top-level API functions.
    * [client.rs](src/domain/client.rs): `GitlabClient` wrapper holding the backend, page_size, api_per_page, and event tx.
    * [issues.rs](src/domain/issues.rs): Issue structures and `list_issues`/`get_issue`.
    * [labels.rs](src/domain/labels.rs): `Label` structure carrying the API-provided color used for the Labels column.
    * [mr.rs](src/domain/mr.rs): MergeRequest, DiscussionNote, NotePosition structures.
    * [mr_state.rs](src/domain/mr_state.rs): MR review-state helpers — `ApprovalState`, `MergeabilityState`, `WorkflowStatus`, `derive_awaiting_you`, `rebase_gate`, and the cell/sort/filter display helpers for the Approval/Mergeable/Workflow columns.
    * [pipelines.rs](src/domain/pipelines.rs): Pipeline, Job structures and job deduplication logic.
    * [runners.rs](src/domain/runners.rs): Runner structures.
    * [releases.rs](src/domain/releases.rs): Release structures.
    * [notifications.rs](src/domain/notifications.rs): Notification structures (GitLab todos + GitHub notifications).
    * [milestones.rs](src/domain/milestones.rs): Milestone structures.
    * [branches.rs](src/domain/branches.rs): Branch structures.
    * [deployments.rs](src/domain/deployments.rs): Environment and Deployment structures.
    * [workflow_inputs.rs](src/domain/workflow_inputs.rs): `WorkflowInput` / `WorkflowInputType` for `workflow_dispatch` prompt fields.
* [src/fetch.rs](src/fetch.rs): `spawn_refresh_active_tab()` — dispatches per-tab data fetches; `derive_workflow()` — recomputes the derived MR `workflow` column after live fetches and cache loads.
* [src/git_helpers.rs](src/git_helpers.rs): Git helpers — `detect_backend` (remote host + CLI auth → `BackendKind`), `parse_project_path` (remote-URL → `namespace/project`), `parse_remote_host`, `get_current_branch`, `slugify`, `get_workflow_files`.
* [src/handlers/](src/handlers/): Keypress handlers split by concern.
    * [mod.rs](src/handlers/mod.rs): Module declarations.
    * [tabs.rs](src/handlers/tabs.rs): Per-tab keybindings (create/edit/delete/approve/merge/view-diff etc.).
    * [overlays.rs](src/handlers/overlays.rs): Overlay handlers (submit dialog, date picker, help, refresh, repo switcher).
* [src/utils/](src/utils/):
    * [cache.rs](src/utils/cache.rs): Offline caching at `~/.cache/glab-tui/<repo>.json`.
    * [format.rs](src/utils/format.rs): Time parsing, ANSI formatting, string truncation, tab expansion (`expand_tabs`), text wrapping (`wrap_text`).
    * [markdown.rs](src/utils/markdown.rs): CommonMark + GFM Markdown rendering via `pulldown-cmark`.
    * [ui.rs](src/utils/ui.rs): Wrappers for `ratatui` stateful lists and tables.
    * [update.rs](src/utils/update.rs): GitHub releases self-updater with multi-target Linux asset selection.
* [src/cli.rs](src/cli.rs): CLI subcommands (`doctor`, `clean-cache`) and ANSI-styled diagnostic output.
* [src/templates.rs](src/templates.rs): Default issue/MR description templates.
* [src/editor.rs](src/editor.rs): External editor integration (`$EDITOR`/`$VISUAL`).
* [src/entity_editor.rs](src/entity_editor.rs): Edit-menu field change logic and creation form helpers.
* [src/ui/](src/ui/): Ratatui render functions.
    * [mod.rs](src/ui/mod.rs): Re-exports and shared render helpers.
    * [inspector.rs](src/ui/inspector.rs): Unified entity inspector component (`render_entity_inspector`, `EntityDocument`, `InspectorMode`). Drives both read-only detail previews and interactive edit/create forms in a single-column layout.
    * [tabs.rs](src/ui/tabs.rs): Tab-specific render functions.
    * [overlays.rs](src/ui/overlays.rs): Overlay render functions (`SubmitDialog`, selectors, date picker, help).
    * [helpers.rs](src/ui/helpers.rs): Shared UI rendering helpers (`badge_style_for`, `render_fuzzy_cell`).
    * [diff.rs](src/ui/diff.rs): Diff view render functions.
    * [modal.rs](src/ui/modal.rs): Unified modal component.
* [src/themes/](src/themes/): 18 bundled theme TOML files (default, tokyo-night, gruvbox, nord, catppuccin-mocha, dracula, clean, deep-space, everforest-dark, monokai, one-dark, solarized-dark, synthwave-84, oled, github-dark-hc, rose-pine, rose-pine-moon, rose-pine-dawn).

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
* **Theme-safe:** `highlight_line_syntax` builds highlighting tokens dynamically derived from the active `THEME`'s semantic tokens (mapped from syntect scope names) while reusing syntect's resolved font modifiers (bold/italic/underline). Highlighting remains theme-safe (no hardcoded palette) while fuzzy search match highlights (`yellow_bg`) are preserved.
* The public function `highlight_line_syntax(file_path, line_content, ext)` returns `Option<Vec<(ratatui::style::Style, String)>>`.
* `DiffLine` contains an optional `syntax_highlighted: Option<Vec<(Style, String)>>` field populated during parsing.

### Entity Inspector & Form Architecture (`src/ui/inspector.rs`)
* **Unified Render Pipeline:** Read-only details preview and interactive edit/create modes share a single rendering path driven by `render_entity_inspector(f, &doc, area, mode, &label_colors)`. `InspectorMode` (`ReadOnly` vs `Interactive`) is the only structural switch, and `EntityDocument` is the single source of truth for both modes.
* **Single-Column Layout:** The markdown description spans full-width at the top, and the metadata fields list is cleanly stacked beneath it with aligned `│` separators.
* **Navigation & Inline Editing:** In interactive mode, `j`/`k`/arrows navigate across all editable rows including Description/Release Notes. `Enter` toggles inline editing (or opens external `$EDITOR` / `Ctrl+E`). `ReadOnly` and `Section` spacer fields are skipped during interactive navigation and render with a muted shield icon (`readonly` nerd font icon).
* **Mode Indicators:** The top banner right-aligns a vim/helix-style mode badge (`[NORMAL]`, `[PREVIEW]`, `[EDIT]`, `[CREATE]`) mapped to semantic badge theme colors. Pressing `Esc` in edit forms pops back to the previous zoom state predictably.
* **In-Menu Creation Forms:** Issue and MR/PR creation forms embed "Create from Issue" (fuzzy-searches existing issues, populates fields, links via `--related-issue` on GitLab or `Closes #N` on GitHub) and "Description Template" rows directly in the menu.
* **Bulk Edit Selection:** Bulk edit menus carry `InspectorContent::Custom` displaying the full list of selected `#iid` / titles in the descriptor pane so users can verify affected entities before submission.

### Visual Select Mode (Yazi-Style)
* `v` (`selection_toggle` keybinding) toggles select mode on the Issues and MRs tabs. Navigating (`j`/`k`/arrows) paints row selection dynamically.
* Selected rows display a 1-wide leftmost colored bar (`checked_bg`) and the top-right header shows a `[SELECT]` mode badge.
* `Space` (`select_issue` / `select_mr`) continues to toggle individual item selection.

### Unified Error Handling (`App::show_error`)
* All API errors, network failures, and guard violations route through `App::show_error(msg)` instead of setting `error_message` directly.
* Displays a 3-row floating rounded toast box with `status_failed` icon and auto-dismiss after 5 seconds.
* Automatically stamps the most recent running command in the terminal commands bar as failed, keeping both UI surfaces synchronized.

### Code Review & Diff System
* **Diff view** supports inline comments, code suggestions, draft reviews, dynamic gutter sizing, and tab expansion:
  - `DiscussionNote` / `NotePosition` structs in [src/domain/mr.rs](src/domain/mr.rs).
  - `list_mr_notes()` fetches notes for an MR via the API.
  - Draft comments are stored in `app.draft_comments: Vec<DraftComment>` and submitted atomically.
  - Current (already-pushed) comments live in `app.current_comments: Vec<DiscussionNote>`.
  - `DiffFetched` event uses named fields: `{ mr_iid, raw_diff, comments }`.
  - Leaving the diff view with pending drafts opens the `SubmitDialog` (`ConfirmAction::SubmitReview(mr_iid)`).
  - Open diff key is `D` (remappable via `keybindings.mrs.view_diff`).
* **Dynamic line numbers & tab expansion:** Gutter width is dynamically calculated in `DiffView::new` from the widest line number in the diff (floored at 4). Tabs are expanded to spaces at tab stops at diff parse time (`expand_tabs`) so Go/Makefiles maintain indentation without breaking syntax highlighting or search indices.
* **Suggestion rendering:** `format_comment_with_suggestions()` in [src/ui/helpers.rs](src/ui/helpers.rs) parses ` ```suggestion ` blocks from comment bodies and renders them as in-line diff (red for original, green for suggested).
* **Reviewed-file marks:** `m` toggles `DiffView::reviewed_files` (a `HashSet` of diff-relative file paths) for the selected file, or for every file below the selected directory; `M` toggles `DiffView::hide_reviewed`. Both are purely local — neither GitLab nor GitHub's "viewed" state is synced.
  - The tree is flattened through `DiffTreeNode::flatten_ex(depth, prefix, reviewed, hide_reviewed, out)`, which stamps `FlatDiffTreeNode::is_reviewed` (on a directory: every file below it is reviewed) and, when filtering, drops reviewed files plus directories left with no unreviewed file.
  - `DiffTreeNode::sync_expansion_to_review(before, after)` folds a directory when it becomes fully reviewed and unfolds it when it stops being fully reviewed, cascading up through parents.
  - Marks persist in `ProjectCache::reviewed_files` (`mr_iid → Vec<String>`), written via `App::store_reviewed_files_for_mr` + `save_cache` on every toggle and re-seeded by `DiffView::restore_review_state` on `DiffFetched`.

### MR Review State (Approval / Mergeable / Workflow)
* The MR/PR table's `Approval`, `Mergeable`, and `Workflow` columns are derived, not fetched. `ApprovalState` / `MergeabilityState` / `WorkflowStatus` and the display/sort/filter helpers live in [src/domain/mr_state.rs](src/domain/mr_state.rs); cell text uses ALL-CAPS display strings (e.g. `CONFLICT`, `REBASE`, `CLEAN`, `APPROVED`, `AWAITING`) that the column-filter picker also shows.
* **Data sources:** GitLab fills both axes with one bulk `glab api graphql` query over `mergeRequests(iids: [...])` (batched by `api_per_page`); GitHub derives them from the review/merge fields returned by `gh pr list` (`reviewDecision`, `latestReviews`, `mergeable`, `mergeStateStatus`, `reviewRequests`) plus the current login via `gh api user --jq .login`. Either axis may be `None` (unknown) — never a guessed value.
* `MergeRequest` carries `approval` and `mergeability` as `Option<…>` and a `#[serde(skip)]` derived `workflow`. After any load (live fetch or cache read), call `derive_workflow()` in [src/fetch.rs](src/fetch.rs) to recompute `workflow` from approval state — cached rows deserialize with it unset even though the approval state it reads was persisted.
* **Rebase gating:** `rebase_gate()` in [src/domain/mr_state.rs](src/domain/mr_state.rs) decides whether `R` may rebase — `Allowed`, `ResolveLocally` (conflicts), or `NotNeeded` — surfaced as a confirm popup or a user-facing error toast. Revoking approval (`A`) is GitLab-only; `gh pr review` has no revoke path.

### Cache & State Persistence
* Cache directory: `~/.cache/glab-tui/` (migrated from `~/.glab-tui-cache`).
* `ProjectCache` stores `enabled_columns`, `group_by_column`, `group_ascending`, `column_filters`, `labels`, `label_colors`, and `reviewed_files` in addition to API data.
* Cache is written on every successful data fetch; read on startup.

### Config & Theme System
* Config is loaded via `Config::load()` in [src/config.rs](src/config.rs) at startup and stored on `App` as `app.config`.
* `Config` carries an optional `backend: Option<BackendKind>` (deserialized lowercase as `"github"` / `"gitlab"`). When set it overrides automatic backend detection everywhere — startup, repo switcher, diff review submission, and `doctor` — via `git_helpers::detect_backend(remote_url, config.backend)`.
* `Config` exposes both `page_size` (total item budget per tab) and `api_per_page` (items per HTTP request, clamped to GitLab's `1–100` `per_page` range via `api_per_page_clamped()`). Thread both through the `Backend` pagination methods; `_per_request` is a no-op on GitHub, which paginates with `--limit`.
* `fetch_label_colors` (default `true`) selects between the real label colors returned by `glab label list` / `gh label list` and the theme's label palette. The API colors are stored as a `name → Color` map on `app.label_colors` (populated from the cache at startup and refreshed on `RepoAttributesFetched`); light GitHub-style label colors fall back to the theme palette because they are unreadable as foreground text on dark themes (`is_light_color()` luminance check in [src/ui/helpers.rs](src/ui/helpers.rs)).
* `Config::load()` only reads existing config files (global then repo-local) and merges overrides; it **never** writes. `config.toml` is created solely by an explicit save (`save_layout` / the `save_view` keybinding), targeting either global (`~/.config/glab-tui/config.toml`) or repo-local (`.glab-tui/config.toml`). If no config file exists, the app boots from in-memory defaults.
* Theme selection: `Config` holds a `theme_preset: Option<String>` and optional per-color `ThemeOverrides`. At startup, `App::apply_config()` resolves the final `Theme` and writes it into the global `THEME` `RwLock`. `Theme::default()` derives directly from `src/themes/default.toml` — there is no hardcoded in-code fallback, so the bundled TOML is the single source of truth. Invalid user theme overrides automatically fall back to bundled presets.
* Icons: The global `ICONS` `RwLock` is initialized at startup with hardcoded nerd font defaults and is not user-configurable.
* Built-in theme presets are compiled into the binary via `include_str!` in `BUNDLED_THEMES` (18 presets including `oled`, `github-dark-hc`, and the Rosé Pine set). User themes in `~/.config/glab-tui/themes/` take precedence.
* **Rule:** Never hard-code RGB colors outside `src/themes/*.toml`. Add new semantic tokens (`diff_gutter_bg`, `diff_sep`, etc.) to `Theme` if needed.

### Keybinding System
* All keybinding defaults are defined via the `keybind_defaults!` macro in [src/config.rs](src/config.rs).
* At runtime, every keypress is matched against the config using `keybinding_matches(binding: &str, event: &KeyEvent) -> bool` in [src/main.rs](src/main.rs).
* **Pattern for all new action handlers:**
  ```rust
  _ if (key_event.code == KeyCode::Char('x')
      || keybinding_matches(&app.config.keybindings.tab.action, &key_event)) => { ... }
  ```
* Never add bare `KeyCode::Char('x') =>` match arms for user-facing actions. Always go through `keybinding_matches()` so users can remap.

### DatePicker
* `DatePicker` in [src/app.rs](src/app.rs) is a modal widget for selecting dates. It holds `year`, `month`, `day` and a `DatePickerAction` enum identifying which field it's editing.
* Open it by pushing `Some(DatePicker::new(...))` into `app.date_picker`; close it by setting to `None`.
* Navigation: `h`/`l` → previous/next month, `j`/`k` → previous/next day, `Enter` → confirm, `Esc` → cancel.

### Submit Dialog & Confirmations (`SubmitDialog`)
* Mutating and destructive actions (close/reopen issue/MR, merge MR, bulk merge, delete branch/release/milestone/issue/MR, rebase, revoke approval, submit review) open a `SubmitDialog` ([src/app.rs](src/app.rs)).
* Includes title, context-aware description body, optional toggleable options (squash, delete branch, auto-merge), and explicit `[ Submit ]` (left, idx 0) and `[ Cancel ]` (right) buttons.
* Navigation: `h`/`l` / Left/Right for horizontal button jumping, `j`/`k` / Up/Down for vertical option traversal, `Tab`/`BackTab` for full control cycling, `Space` to toggle options, `Enter` to activate focused button, `Esc` to cancel.
* Destructive actions (close, delete, revoke) default the cursor to Cancel; reversible actions (merge, rebase, review) default to Submit.
* Mouse clicks on button boxes or option rows are supported.

### Mouse Support
* Mouse events (`crossterm::event::MouseEvent`) are handled in the event loop for selecting tabs, scrolling tables, and interacting with overlays.
* All modal and overlay interactions (submit dialogs, selectors, date picker, help) have click handlers routed through their respective state components.
* Selector overlays compute mouse target positions based on search bar presence (determined by `field_type`) and footer height.
* Add new mouse handlers following the pattern in [src/handlers/overlays.rs](src/handlers/overlays.rs) and [src/handlers/tabs.rs](src/handlers/tabs.rs).

### Column Configure Popup
* The configure overlay (`Tab`) has three sections: **COLUMNS** (checkbox toggle), **GROUP BY** (single-select), and **ORDER** (Ascending/Descending).
* Column lists use `ListState` scrolling with position counters to handle short terminals gracefully.
* Value-based column filtering is available by pressing `Enter` on a focused column item, which opens a selector overlay with distinct values for that column.
* Column filter state is tracked via `app.column_filter_context` and `app.column_filters: HashMap<Tab, HashMap<String, Vec<String>>>`.
* Group state is tracked via `app.group_by_column: Option<String>` and `app.group_ascending: bool`.
* When rendering the MR/PR pipeline status column, check `is_github` to display "Pipeline" (GitLab) or "Action" (GitHub) terminology.
* MR/PR review-state columns (`Approval`, `Mergeable`, `Workflow`) are derived in [src/domain/mr_state.rs](src/domain/mr_state.rs).

## 4. UI & Rendering Guidelines (`ratatui`)

* **Colors & Theming:** Always use the `THEME` global (a `RwLock<Theme>` initialized from `app.config` at startup). Access it as `crate::config::THEME.read().unwrap()` or via the re-export in `ui.rs`. Do not hard-code raw RGB values; add new semantic color tokens to `src/config.rs` and all theme TOML files if needed. Every surface is theme-driven, including the diff view (`diff_addition_*`/`diff_deletion_*`/`diff_gutter_bg`/`diff_sep`/`comment_bg`/`comment_draft_bg`), markdown rendering, and diff selection/search-match highlights (`highlight_bg`/`yellow_bg`). Pass the resolved theme into render helpers instead of re-locking `THEME` inside them (see `render_markdown`).
* **Fuzzy Matching:** Use `SkimMatcherV2` from the `fuzzy-matcher` crate for filtering tables and selector overlays. The `render_fuzzy_cell` helper in [src/ui.rs](src/ui.rs) handles highlighting matched characters in yellow.
* **Columns:** Table columns are dynamically configurable. Always check `app.is_column_visible(tab, "Column Name")` before rendering a cell or header. GitHub-only or GitLab-only columns must also gate on `app.gitlab_client.is_some()` / `is_github`.
* **Layout:** Use `ratatui::layout::Layout` to split screens. Avoid hardcoded fixed sizes where possible, use `Constraint::Percentage` or `Constraint::Fill(1)`. Use `centered_rect_min()` for overlays to ensure minimum readable dimensions on small terminals.

## 5. Adding a New Feature (Workflow)

If asked to add a new Tab (e.g., "Deployments"):
1.  **Update State:** Add the tab to the `Tab` enum in [src/app.rs](src/app.rs) (include it in `ALL`, `title()`, `columns()`, and `default_columns()`). Add a `StatefulTable<Deployment>` to `App`.
2.  **Define Domain Logic:** Create [src/domain/deployments.rs](src/domain/deployments.rs). Define the `Deployment` struct with `serde` traits. Write a `list_deployments` function that delegates to the backend.
3.  **Add Backend Methods:** Add the relevant method to the `Backend` trait in [src/backend/mod.rs](src/backend/mod.rs) and implement it in both [glab.rs](src/backend/glab.rs) and [gh.rs](src/backend/gh.rs). Use native subcommands where available; fall back to `raw_api()` only if no native command exists.
4.  **Create Events:** Add `DeploymentsFetched(Vec<Deployment>)` to the `Event` enum in [src/event.rs](src/event.rs).
5.  **Handle Data Fetching:** In [src/main.rs](src/main.rs), update `spawn_refresh_active_tab` (in [src/fetch.rs](src/fetch.rs)) to fetch data for the new tab.
6.  **Handle UI Updates:** In [src/main.rs](src/main.rs), handle the `Event::DeploymentsFetched` to update `app.deployments.items` and trigger cache saving.
7.  **Handle Navigation:** In [src/main.rs](src/main.rs), handle `KeyCode::Down`/`Up` to navigate the table state.
8.  **Render:** In [src/ui/tabs.rs](src/ui/tabs.rs), add a branch to `match app.active_tab` to construct the rows, table, and details preview pane.

## 6. CLI Command Mapping

Every interaction with GitLab/GitHub goes through `glab` or `gh` CLI. This section documents every command used, organized by backend and operation.

### GlabBackend (`src/backend/glab.rs`)

#### Data Fetching — Native Subcommands

| Operation | Command | Pagination |
|---|---|---|
| List issues | `glab issue list --output json -R <repo> --state <s> --page N --per-page <api_per_page>` | Loops up to `page_size/api_per_page` pages |
| Get single issue | `glab issue view <iid> --output json -R <repo>` | N/A |
| List MRs | `glab mr list --output json -R <repo> --state <s> --page N --per-page <api_per_page>` | Loops up to `page_size/api_per_page` pages |
| Get single MR | `glab mr view <iid> --output json -R <repo>` | N/A |
| Get MR diff | `glab mr diff <iid> -R <repo>` | N/A |
| List MR notes | `glab mr note list <iid> --output json -R <repo>` | N/A |
| List pipelines | `glab ci list --output json -R <repo> --page N --per-page <api_per_page>` | Loops up to `page_size/api_per_page` pages |
| List runners | `glab runner list --output json -R <repo> --per-page <N>` | Single call |
| List releases | `glab release list --output json -R <repo> --per-page <N>` | Single call |
| List milestones | `glab milestone list --output json --project <repo> --per-page <N>` | Single call (`milestone list` requires `--project`/`--group`, not `-R`) |
| List milestone issues | `glab issue list --milestone <title> --all --output json -R <repo> --per-page <N>` | Single call (`--milestone` filters by milestone **title**, not iid — resolved from the selected milestone at the call site) |
| List todos | `glab todo list --output=json` | Single call |
| List labels | `glab label list --output json -R <repo> --per-page <api_per_page>` | Single call (label colors feed the Labels column) |

#### Mutations — Native Subcommands

| Operation | Command |
|---|---|
| Update release | `glab release update <tag> -R <repo> -n <name> -N <desc>` |
| Delete release | `glab release delete <tag> -R <repo> -y` |
| Close/reopen milestone | `glab milestone close\|reopen <iid> -R <repo>` |
| Update milestone | `glab milestone update <iid> -R <repo> --title ... --description ...` |
| Delete milestone | `glab milestone delete <iid> -R <repo> -y` |
| Cancel pipeline | `glab ci cancel pipeline <id> -R <repo>` |
| Retry job | `glab ci retry <job_id> -R <repo>` |
| Cancel job | `glab ci cancel job <id> -R <repo>` |
| Start manual job | `glab ci retry <job_id> -R <repo>` |
| Run pipeline (variables/inputs) | `glab ci run [--branch <ref>] [--mr] [--variables k:v ...] [--input k:v ...]` |
| Mark todo done | `glab todo done <id>` |
| Revoke MR approval | `glab mr revoke <iid> -R <repo>` |
| Rebase MR | `glab mr rebase <iid> -R <repo>` |

#### Data Fetching — Raw API (no native subcommand exists)

| Operation | Endpoint | Why raw API |
|---|---|---|
| List pipeline jobs | `GET /projects/{}/pipelines/{}/jobs?per_page=<N>` | `glab ci view` is interactive TUI; `glab ci get` returns nested pipeline object with different structure |
| Get job trace | `GET /projects/{}/jobs/{}/trace` | `glab ci trace` is interactive/streaming; we need programmatic text output |
| List done todos | `GET todos?state=done` | `glab todo list` only shows pending |
| List branches | `GET /projects/{}/repository/branches?per_page=<N>` | No `glab branch` command |
| Create branch | `POST /projects/{}/repository/branches?branch=...&ref=...` | No `glab branch` command |
| Delete branch | `DELETE /projects/{}/repository/branches/{}` | No `glab branch` command |
| List environments | `GET /projects/{}/environments?per_page=<N>` | No native command |
| List deployments | `GET /projects/{}/deployments?per_page=<N>` | No native command |
| List members | `GET /projects/{}/members/all?per_page=100` | `glab repo members` only has add/remove |
| Retry pipeline | `POST /projects/{}/pipelines/{}/retry` | `glab ci retry` is job-only; no pipeline retry subcommand |
| MR approval/mergeability state | `glab api graphql` over `mergeRequests(iids: [...])` | `glab mr list` exposes neither axis; one bulk query fills the Approval/Mergeable columns (batched by `api_per_page`) |
| List environments | `GET /projects/{}/environments?per_page=<N>` | No native command |
| List deployments | `GET /projects/{}/deployments?per_page=<N>` | No native command |

### GhBackend (`src/backend/gh.rs`)

#### Data Fetching — Native Subcommands

| Operation | Command | Pagination |
|---|---|---|
| List issues | `gh issue list --json number,title,state,... -R <repo> --state <s> --limit <N>` | Single `--limit` call (N = page_size × 10) |
| Get single issue | `gh issue view <iid> --json ... -R <repo>` | N/A |
| List PRs | `gh pr list --json number,title,state,... -R <repo> --state <s> --limit <N>` | Single `--limit` call; the JSON projection includes `reviewDecision`, `latestReviews`, `mergeable`, `mergeStateStatus`, `reviewRequests` to derive the Approval/Mergeable/Workflow columns |
| Get single PR | `gh pr view <iid> --json ... -R <repo>` | N/A |
| Get PR diff | `gh pr diff <iid> -R <repo>` | N/A |
| List actions/runs | `gh run list --json databaseId,status,... -R <repo> --limit <N>` | Single `--limit` call |
| List pipeline jobs | `gh run view <id> --json jobs --jq .jobs -R <repo>` | Single call |
| Get job trace | `gh run view --job <id> --log -R <repo>` | N/A |
| List releases | `gh release list --json name,tagName,... -R <repo> --limit <N>` | Single call |
| List milestone issues | `gh issue list --milestone <id> --state all --json ... -R <repo> --limit <N>` | Single call |
| List labels | `gh label list --json name,color -R <repo> --limit 100` | Single call (label colors feed the Labels column) |

#### Mutations — Native Subcommands

| Operation | Command |
|---|---|
| Retry run | `gh run rerun <id> -R <repo>` |
| Cancel run | `gh run cancel <id> -R <repo>` |
| Retry job | `gh run rerun --job <id> -R <repo>` |
| Update release | `gh release edit <tag> -R <repo> -t <name> -n <desc>` |
| Delete release | `gh release delete <tag> -R <repo> -y` |
| Update milestone state | `gh api -X PATCH repos/{}/milestones/{} -f state=...` |
| Rebase PR | `gh pr update-branch <iid> -R <repo> --rebase` |

#### Data Fetching — Raw API (no native subcommand exists)

| Operation | Endpoint | Why raw API |
|---|---|---|
| List PR review comments | `GET /repos/{}/pulls/{}/comments?per_page=<N>` | `gh pr view --json comments` lacks inline line/position fields needed for diff review |
| Get current user login | `gh api user --jq .login` | Needed to derive "your" workflow/approval state for the MR/PR review columns |
| Cancel job | `POST /repos/{}/actions/jobs/{}/cancel` | No per-job cancel in `gh` |
| List runners | `GET /repos/{}/actions/runners?per_page=<N>` | No native command |
| List milestones | `GET /repos/{}/milestones?state=all&per_page=<N>` | No `gh milestone` command |
| List notifications | `GET notifications[?all=true]` | No `gh notification` command |
| Mark notification read | `PATCH notifications/threads/{}` | No native command |
| List branches | `GET /repos/{}/branches?per_page=<N>` | No native command |
| Create branch | `POST /repos/{}/git/refs` | No native command |
| Delete branch | `DELETE /repos/{}/git/refs/heads/{}` | No native command |
| List environments | `GET /repos/{}/environments?per_page=<N>` | No native command |
| List deployments | `GET /repos/{}/deployments?per_page=<N>` | No native command |
| List members | `GET /repos/{}/assignees?per_page=100` | No native command |
| Update milestone | `PATCH repos/{}/milestones/{}` | No `gh milestone` command |
| Delete milestone | `DELETE repos/{}/milestones/{}` | No `gh milestone` command |
| List environments | `GET /repos/{}/environments?per_page=<N>` | No native command |
| List deployments | `GET /repos/{}/deployments?per_page=<N>` | No native command |
| List members | `GET /repos/{}/assignees?per_page=100` | No native command |

### Direct CLI Commands (`src/main.rs` — `run_cli()`)

These are user-triggered mutations that shell out directly to the CLI without going through the backend:

| Action | Command |
|---|---|
| Create issue | `gh issue create -e` / `glab issue create -y --title <t>` |
| Edit issue/MR | `gh issue edit` / `glab issue update` (with field flags) |
| Close issue/MR | `gh issue\|pr close <iid>` / `glab issue\|mr close <iid>` |
| Reopen issue/MR | `gh issue\|pr reopen <iid>` / `glab issue\|mr reopen <iid>` |
| Delete issue (Glab) | `glab issue delete <iid> -R <repo>` |
| Delete MR (Glab) | `glab mr delete <iid> -R <repo>` |
| Delete issue (GH) | `gh issue delete <iid> -R <repo> --yes` |
| Approve MR | `gh pr review <iid> --approve` / `glab mr approve <iid>` |
| Merge MR | `gh pr merge <iid> --delete-branch --squash` / `glab mr merge <iid> --squash --remove-source-branch --yes` (the `--yes` flag skips the CLI's interactive confirmation; multiple selected MRs are merged in sequence through `Backend::merge_mr`) |
| Toggle draft (→ ready) | `gh pr ready <iid>` / `glab mr update <iid> --ready` |
| Toggle draft (→ draft) | `gh pr ready <iid> --undo` / `glab mr update <iid> --draft` |
| Create release | `gh release create <tag> -F <changelog>` / `glab release create <tag> -F <changelog>` |
| Create milestone | `gh api POST repos/{}/milestones -f title=...` / `glab api POST projects/{}/milestones -f title=...` |
| Create branch | `glab api POST ...repository/branches` / `gh api POST ...git/refs` |
| Delete branch | `glab api DELETE ...repository/branches/{}` / `gh api DELETE ...git/refs/heads/{}` |
| Run pipeline | `gh workflow run` / `glab ci run --mr` |
| Open in browser | `gh issue\|pr\|run view --web` / `glab issue\|mr\|ci view -w` |
| Reply to comment | `gh api POST repos/{}/pulls/{}/comments` / `glab api POST projects/{}/merge_requests/{}/discussions/{}/notes` |
| Submit review | `gh api POST repos/{}/pulls/{}/reviews` / `glab api POST projects/{}/merge_requests/{}/...` |

> `glab ci run` notes: variables/inputs are passed via the plural `--variables k:v` / `--input k:v` flags (not `--variable`), and `--mr` is only passed when no variables or `workflow_dispatch` inputs are set.

### Exhaustive CLI Command & Flag Reference

Below is the complete reference of all available `glab` and `gh` subcommands and their options based on the currently installed CLI tools (`glab v1.114.0` / `gh v2.98.0`).

#### GitLab CLI (`glab`) Reference

#### `glab`

**Subcommands:**
- `api` — Make an authenticated request to the GitLab API.
- `artifact-registry` — Authenticate with GitLab Artifact Registry. (EXPERIMENTAL)
- `attestation` — Manage software attestations. (EXPERIMENTAL)
- `auth` — Manage authentication for glab.
- `changelog` — Generate changelogs from your project's commit history.
- `check-update` — Check for the latest glab version.
- `ci` — Work with GitLab CI/CD pipelines and jobs.
- `cluster` — Manage GitLab Agents for Kubernetes and their clusters.
- `config` — Manage glab settings.
- `container-registry` — Work with GitLab container registries.
- `dependency-firewall` — Configure and monitor GitLab Dependency Firewall for local package managers.
- `deploy-key` — Manage deploy keys.
- `duo` — Work with GitLab Duo.
- `gpg-key` — Manage GPG keys registered with your GitLab account.
- `incident` — Work with GitLab incidents.
- `issue` — Work with GitLab issues.
- `iteration` — Retrieve iteration information.
- `job` — Work with GitLab CI/CD jobs.
- `label` — Manage labels on remote.
- `mcp` — Work with a Model Context Protocol (MCP) server. (EXPERIMENTAL)
- `milestone` — Manage group or project milestones.
- `mr` — Create, view, and manage merge requests.
- `opentofu` — Work with the OpenTofu or Terraform integration.
- `orbit` — Gitlab Knowledge Graph commands. (EXPERIMENTAL)
- `packages` — Manage packages in the GitLab package registry.
- `release` — Manage GitLab releases.
- `repo` — Work with GitLab repositories and projects.
- `runner` — Manage GitLab CI/CD runners.
- `runner-controller` — Manage runner controllers. (EXPERIMENTAL)
- `schedule` — Work with GitLab CI/CD schedules.
- `search` — Search for code and resources in a GitLab project. (BETA)
- `securefile` — Manage secure files for a project.
- `security` — Manage GitLab security scan profiles for a project. (EXPERIMENTAL)
- `skills` — Manage glab agent skills. (EXPERIMENTAL)
- `snippet` — Create, view and manage snippets.
- `ssh-key` — Manage SSH keys registered with your GitLab account.
- `stack` — Create, manage, and work with stacked diffs. (EXPERIMENTAL)
- `todo` — Manage your to-do list.
- `token` — Manage personal, project, or group tokens.
- `user` — Interact with a GitLab user account.
- `variable` — Manage variables for a GitLab project or group.
- `whatsnew` — Show release notes for new versions of glab.
- `work-items` — Manage work items. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-v --version` | Show glab version information. |

##### `glab alias`

**Subcommands:**
- `delete` — Delete an alias.
- `list` — List aliases.
- `set` — Set an alias for a longer command.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab alias delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab alias list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab alias set`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-s --shell` | Declare an alias to be passed through a shell interpreter. |

##### `glab api`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-F --field` | Add a parameter of inferred type. Using this flag changes the default HTTP method to POST. |
| `--form` | Add a multipart form field. To upload a file, prefix the value with @ followed by the file path. To read from standard input, use @- (at most once). Using this flag changes the default HTTP method to POST. |
| `-H --header` | Add an additional HTTP request header. |
| `-h --help` | Show help for this command. |
| `--hostname` | The GitLab hostname for the request. Defaults to gitlab.com, or the authenticated host in the current Git directory. |
| `-i --include` | Include HTTP response headers in the output. |
| `--input` | The file to use as the body for the HTTP request. |
| `-X --method` | The HTTP method for the request. (GET) |
| `--output` | Format output as: json, ndjson. (json) |
| `--paginate` | Make additional HTTP requests to fetch all pages of results. |
| `-f --raw-field` | Add a string parameter. |
| `--silent` | Do not print the response body. |

##### `glab artifact-registry`

**Subcommands:**
- `get-token` — Get a short-lived access token for the GitLab Artifact Registry. (EXPERIMENTAL)
- `login` — Authenticate a package manager against the GitLab Artifact Registry. (EXPERIMENTAL)
- `status` — Check your access to the GitLab Artifact Registry. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab artifact-registry get-token`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--duration` | How long the token should remain valid. Must be between 1s and 12h0m0s. (15m0s) |
| `-h --help` | Show help for this command. |
| `--hostname` | Gitlab hostname to request the token from. Defaults to the configured GitLab instance. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |

###### `glab artifact-registry login`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--docker` | Configure Docker to authenticate against the registry. Writes to $DOCKER_CONFIG, or ~/.docker when it is unset. |
| `--duration` | How long the exchanged token should remain valid. Ignored for now: --docker is the only tool this command configures, and its credential helper mints a fresh token for every request. (0s) |
| `-h --help` | Show help for this command. |
| `--hostname` | Gitlab hostname to request the token from. Defaults to the configured GitLab instance. |
| `--registry` | Bare hostname of the registry to authenticate against. |

###### `glab artifact-registry status`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--hostname` | Gitlab hostname to check. Defaults to the configured GitLab instance. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |

##### `glab attestation`

**Subcommands:**
- `verify` — Verify the provenance of a specific artifact or file. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab attestation verify`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

##### `glab auth`

**Subcommands:**
- `configure-docker` — Register glab as a Docker credential helper.
- `docker-helper` — A Docker credential helper for GitLab container and artifact registries.
- `dpop-gen` — Generate a DPoP (demonstrating-proof-of-possession) proof JWT. (EXPERIMENTAL)
- `login` — Authenticate with a GitLab instance.
- `logout` — Log out from a GitLab instance.
- `status` — View authentication status.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab auth configure-docker`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab auth docker-helper`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab auth dpop-gen`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--hostname` | The hostname of the GitLab instance to authenticate with. (gitlab.com) |
| `--pat` | Personal access token (PAT) to generate a DPoP proof for. Defaults to the token set with 'glab auth login'. |
| `-p --private-key` | Location of the private SSH key on the local system. |

###### `glab auth login`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --api-host` | Hostname for the API endpoint, if different from --hostname. Accepts a hostname or hostname:port. Use only when the API is served from a different host than the Git remote. |
| `-p --api-protocol` | Api protocol. Options: https, http. |
| `--container-registry-domains` | Container registry and image dependency proxy domains, comma-separated. |
| `--device` | Use the OAuth 2.0 device authorization flow. Useful for headless environments where a local browser is not available. Requires GitLab 17.9 or later. |
| `-g --git-protocol` | Git protocol. Options: ssh, https, http. |
| `-h --help` | Show help for this command. |
| `--hostname` | The hostname of the GitLab instance to authenticate with. |
| `--insecure-storage` | Store the token as plaintext in the configuration file instead of the operating system's keyring. |
| `-j --job-token` | Ci job token. |
| `--ssh-hostname` | Ssh hostname for instances with a different SSH endpoint. A port is not required; Git uses the port from the remote URL. |
| `--stdin` | Read the token from standard input. |
| `-t --token` | Your GitLab access token. |
| `--web` | Skip the login type prompt and use web/OAuth login. |

###### `glab auth logout`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--hostname` | The hostname of the GitLab instance. |

###### `glab auth status`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --all` | Check the authentication status of all configured instances. |
| `-h --help` | Show help for this command. |
| `--hostname` | Check the authentication status of a specific instance. |
| `-t --show-token` | Display the authentication token. |

##### `glab changelog`

**Subcommands:**
- `generate` — Generate a changelog for the current project.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab changelog generate`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--config-file` | Path to the changelog configuration file in the project's Git repository. Defaults to '.gitlab/changelog_config.yml'. |
| `--date` | Date and time of the release, in ISO 8601 format (2016-03-11T03:45:40Z). Defaults to the current time. |
| `--from` | Start of the range of commits to use when generating the changelog, as a SHA. This commit is not included in the range. |
| `-h --help` | Show help for this command. |
| `--to` | End of the range of commits to use when generating the changelog, as a SHA. This commit is included in the range. Defaults to the HEAD of the project's default branch. |
| `--trailer` | The Git trailer to use to include commits. Defaults to 'Changelog'. |
| `-v --version` | Version to generate the changelog for. Must follow semantic versioning. Defaults to the version detected by 'git describe'. |

##### `glab check-update`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

##### `glab ci`

**Subcommands:**
- `artifact` — Download all artifacts from the last pipeline.
- `cancel` — Cancel a running pipeline or job.
- `config` — View and inspect GitLab CI/CD configuration.
- `delete` — Delete CI/CD pipelines.
- `get` — Get the details of a CI/CD pipeline.
- `lint` — Check if your `.gitlab-ci.yml` file is valid.
- `list` — List CI/CD pipelines.
- `retry` — Retry a CI/CD job.
- `run` — Create a new CI/CD pipeline.
- `run-trig` — Run a CI/CD pipeline trigger.
- `status` — View CI/CD pipeline status.
- `trace` — Trace a CI/CD job log in real time.
- `trigger` — Trigger a manual CI/CD job.
- `view` — View, run, retry, and cancel CI/CD pipeline jobs.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab ci artifact`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-p --path` | Path to download the artifact files. (./) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab ci cancel`

**Subcommands:**
- `job` — Cancel CI/CD jobs.
- `pipeline` — Cancel CI/CD pipelines.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab ci cancel job`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--dry-run` | Show which jobs would be canceled, without canceling them. |
| `-f --force` | Force-Cancel the job, even if it runs in a protected environment. |
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab ci cancel pipeline`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--dry-run` | Show which pipelines would be canceled, without canceling them. |
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab ci config`

**Subcommands:**
- `compile` — View the merged CI/CD configuration.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab ci config compile`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab ci delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--dry-run` | Simulate process, but do not delete anything. |
| `-h --help` | Show help for this command. |
| `--older-than` | Filter pipelines older than the given duration. Valid units: h, m, s, ms, us, ns. (0s) |
| `--page` | Page number. |
| `--paginate` | Make additional HTTP requests to fetch all pages of pipelines. Respects '--per-page'. |
| `--per-page` | Number of items to list per page. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--source` | Filter pipelines by source: api, chat, external, external_pull_request_event, merge_request_event, ondemand_dast_scan, ondemand_dast_validation, parent_pipeline, pipeline, push, schedule, security_orchestration_policy, trigger, web, webide. |
| `-s --status` | Delete pipelines by status: running, pending, success, failed, canceled, skipped, created, manual. |

###### `glab ci get`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b --branch` | Get the pipeline for a branch. Defaults to the current branch. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `--merge-request` | Show the pipeline for the given merge request <iid>. |
| `-F --output` | Format output. Options: text, json. (text) |
| `-p --pipeline-id` | Get the pipeline with the given <id>. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-s --status` | Show only jobs in the given state. Passed through to the API's scope parameter. |
| `-d --with-job-details` | Show extended job information. |
| `--with-variables` | Show variables in pipeline. Requires the Maintainer role. |

###### `glab ci lint`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--dry-run` | Run pipeline creation simulation. |
| `-h --help` | Show help for this command. |
| `--include-jobs` | Include the list of jobs that would exist in a static check or pipeline simulation. |
| `--ref` | When '--dry-run' is true, sets the branch or tag context for validating the CI/CD YAML configuration. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab ci list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-n --name` | Return only pipelines with the given name. |
| `-o --order` | Order pipelines by this field. Options: id, status, ref, updated_at, user_id. (id) |
| `-F --output` | Format output. Options: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. Defaults to the GitLab API default (20). |
| `-r --ref` | Return only pipelines for the given ref. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--scope` | Return only pipelines with the given scope. Options: running, pending, finished, branches, tags. |
| `--sha` | Return only pipelines with the given SHA. |
| `--sort` | Sort direction for '--order': asc or desc. (desc) |
| `--source` | Return only pipelines triggered by the given source. For the full list, see https://docs.gitlab.com/ci/jobs/job_rules/#ci_pipeline_source-predefined-variable. Commonly used options: merge_request_event, parent_pipeline, pipeline, push, trigger. |
| `-s --status` | Filter pipelines by status. Options: running, pending, success, failed, canceled, skipped, created, manual, waiting_for_resource, preparing, scheduled. |
| `-a --updated-after` | Return only pipelines updated after the specified date. Expected in ISO 8601 format (2019-03-15T08:00:00Z). |
| `-b --updated-before` | Return only pipelines updated before the specified date. Expected in ISO 8601 format (2019-03-15T08:00:00Z). |
| `-u --username` | Return only pipelines triggered by the given username. |
| `-y --yaml-errors` | Return only pipelines with invalid configurations. |

###### `glab ci retry`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b --branch` | The branch to search for the job. Defaults to the current branch. |
| `-h --help` | Show help for this command. |
| `-p --pipeline-id` | The pipeline ID to search for the job. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab ci run`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b --branch` | Create pipeline on branch or reference <string>. |
| `-h --help` | Show help for this command. |
| `-i --input` | Pass inputs to pipeline in format '<key>:<value>'. Cannot be used for merge request pipelines. See documentation for examples. |
| `--mr` | Run merge request pipeline instead of branch pipeline. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--variables` | Pass variables to the pipeline in the format <key>:<value>. Cannot be used for MR pipelines. |
| `--variables-env` | Pass variables to the pipeline in the format <key>:<value>. Cannot be used for MR pipelines. |
| `--variables-file` | Pass file contents as a file variable to the pipeline in the format <key>:<filename>. Cannot be used for MR pipelines. |
| `-f --variables-from` | Json file with variables for pipeline execution. Expects array of hashes, each with at least 'key' and 'value'. Cannot be used for MR pipelines. |
| `-w --web` | Open pipeline in a browser. Uses default browser, or browser specified in BROWSER environment variable. |

###### `glab ci run-trig`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b --branch` | Create pipeline on branch or reference <string>. |
| `-h --help` | Show help for this command. |
| `-i --input` | Pass inputs to pipeline in format '<key>:<value>'. Cannot be used for merge request pipelines. See documentation for examples. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-t --token` | Pipeline trigger token. Can be omitted only if the 'CI_JOB_TOKEN' environment variable is set. |
| `--variables` | Pass variables to pipeline in the format <key>:<value>. Multiple variables can be comma-separated or specified by repeating the flag. |

###### `glab ci status`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b --branch` | Check pipeline status for a branch. Defaults to the current branch. |
| `-c --compact` | Show status in compact format. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-l --live` | Show status in real time until the pipeline ends. |
| `-F --output` | Format output as: text, json. Note: JSON output is not compatible with --live, --wait, or --compact flags. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-w --wait` | Wait to return until the pipeline is finished, and provide output without a prompt. |

###### `glab ci trace`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b --branch` | The branch to search for the job. Defaults to the current branch. |
| `-h --help` | Show help for this command. |
| `-p --pipeline-id` | The pipeline ID to search for the job. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab ci trigger`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b --branch` | The branch to search for the job. Defaults to the current branch. |
| `-h --help` | Show help for this command. |
| `-p --pipeline-id` | The pipeline ID to search for the job. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab ci view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b --branch` | Check pipeline status for a branch or tag. Defaults to the current branch. |
| `-h --help` | Show help for this command. |
| `-p --pipelineid` | Check pipeline status for a specific pipeline ID. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-w --web` | Open pipeline in a browser. Uses the default browser, or the browser specified in the BROWSER environment variable. |

##### `glab cluster`

**Subcommands:**
- `agent` — Manage GitLab Agents for Kubernetes.
- `graph` — Query the Kubernetes object graph using the GitLab Agent for Kubernetes. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab cluster agent`

**Subcommands:**
- `bootstrap` — Bootstrap a GitLab Agent for Kubernetes in a project.
- `check_manifest_usage` — Find agents using deprecated GitOps manifest settings. (EXPERIMENTAL)
- `get-token` — Create a personal access token for a GitLab Agent for Kubernetes.
- `list` — List GitLab Agents for Kubernetes in a project.
- `token` — Manage GitLab Agents for Kubernetes tokens.
- `token-cache` — Manage cached GitLab Agent tokens.
- `update-kubeconfig` — Update your kubeconfig for use with a GitLab Agent for Kubernetes.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab cluster agent bootstrap`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--commit-author-email` | The Git commit author email to use. Conflicts with '--use-api-commit-author'. (noreply@glab.gitlab.com) |
| `--commit-author-name` | The Git commit author name to use. Conflicts with '--use-api-commit-author'. (glab) |
| `--create-environment` | Create an environment for the GitLab Agent. (true) |
| `--create-flux-environment` | Create an environment for FluxCD. Affects only the environment creation, not the use of Flux itself. Flux is always required for the bootstrap process. (true) |
| `--environment-flux-resource-path` | Flux resource path of the environment for the GitLab Agent. (helm.toolkit.fluxcd.io/v2beta1/namespaces/<helm-release-namespace>/helmreleases/<helm-release-name>) |
| `--environment-name` | Name of the environment for the GitLab Agent. (<helm-release-namespace>/<helm-release-name>) |
| `--environment-namespace` | Kubernetes namespace of the environment for the GitLab Agent. (<helm-release-namespace>) |
| `--flux-environment-flux-resource-path` | Flux resource path of the environment for FluxCD. (kustomize.toolkit.fluxcd.io/v1/namespaces/flux-system/kustomizations/flux-system) |
| `--flux-environment-name` | Name of the environment for FluxCD. (<flux-source-namespace>/<flux-source-name>) |
| `--flux-environment-namespace` | Kubernetes namespace of the environment for FluxCD. (<flux-source-namespace>) |
| `--flux-source-name` | Flux source name. (flux-system) |
| `--flux-source-namespace` | Flux source namespace. (flux-system) |
| `--flux-source-type` | Source type of the flux-system, like Git, OCI, or Helm. (git) |
| `--gitlab-agent-token-secret-name` | Name of the Secret where the token for the GitLab Agent is stored. The helm-release-target-namespace is implied for the namespace of the Secret. (gitlab-agent-token) |
| `--helm-release-filepath` | File path within the GitLab Agent project to commit the Flux HelmRelease to. (gitlab-agent-helm-release.yaml) |
| `--helm-release-name` | Name of the Flux HelmRelease manifest. (gitlab-agent) |
| `--helm-release-namespace` | Namespace of the Flux HelmRelease manifest. (flux-system) |
| `--helm-release-target-namespace` | Namespace of the GitLab Agent deployment. (gitlab-agent) |
| `--helm-release-values` | Local path to values.yaml files. Multiple files can be comma-separated or specified by repeating the flag. |
| `--helm-release-values-from` | Kubernetes object reference that contains the values.yaml data key in the format '<kind>/<name>', where 'kind' must be one of: (Secret, ConfigMap). Multiple references can be comma-separated or specified by repeating the flag. |
| `--helm-repository-address` | Address of the HelmRepository. (https://charts.gitlab.io) |
| `--helm-repository-filepath` | File path within the GitLab Agent project to commit the Flux HelmRepository to. (gitlab-helm-repository.yaml) |
| `--helm-repository-name` | Name of the Flux HelmRepository manifest. (gitlab) |
| `--helm-repository-namespace` | Namespace of the Flux HelmRepository manifest. (flux-system) |
| `-h --help` | Show help for this command. |
| `-b --manifest-branch` | Branch to commit the Flux Manifests to. Defaults to the project default branch. |
| `-p --manifest-path` | Location of directory in Git repository for storing the GitLab Agent for Kubernetes Helm resources. |
| `--no-reconcile` | Do not trigger Flux reconciliation for GitLab Agent for Kubernetes Flux resource. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--use-api-commit-author` | When creating Git commits use the user from the authenticated API request. Conflicts with '--commit-author-name' and '--commit-author-email'. |

###### `glab cluster agent check_manifest_usage`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --agent-page` | Page number for agents. (1) |
| `-A --agent-per-page` | Number of agents to list per page. (30) |
| `-g --group` | Group ID to check. |
| `-h --help` | Show help for this command. |
| `-p --page` | Page number for projects. (1) |
| `-P --per-page` | Number of projects to list per page. (30) |
| `-r --recursive` | Recursively check subgroups. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab cluster agent get-token`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --agent` | The numerical Agent ID to connect to. |
| `-c --cache-mode` | Mode to use for caching the token. Allowed values: keyring-filesystem-fallback, force-keyring, force-filesystem, no. (force-keyring) |
| `--check-revoked` | Check if a cached token is revoked. This requires an API call to GitLab which adds latency every time a cached token is accessed. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--token-expiry-duration` | Duration for how long the generated tokens should be valid for. Minimum is 1 day and the effective expiry is always at the end of the day, the time is ignored. (24h0m0s) |

###### `glab cluster agent list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab cluster agent token`

**Subcommands:**
- `list` — List tokens for a GitLab Agent for Kubernetes.
- `revoke` — Revoke an agent token.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab cluster agent token list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab cluster agent token revoke`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab cluster agent token-cache`

**Subcommands:**
- `clear` — Clear cached GitLab Agent tokens.
- `list` — List cached GitLab Agent tokens.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab cluster agent token-cache clear`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--agent` | Clear tokens for specific agent IDs only. |
| `--filesystem` | Clear tokens from filesystem cache. (true) |
| `-h --help` | Show help for this command. |
| `--keyring` | Clear tokens from keyring cache. (true) |
| `-R --repo` | Select another repository using the OWNER/REPO format. |
| `--revoke` | Revoke tokens on GitLab server before clearing cache. (true) |

###### `glab cluster agent token-cache list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--agent` | Filter by specific agent IDs. |
| `--filesystem` | Include tokens from filesystem cache. (true) |
| `-h --help` | Show help for this command. |
| `--keyring` | Include tokens from keyring cache. (true) |
| `-R --repo` | Select another repository using the OWNER/REPO format. |

###### `glab cluster agent update-kubeconfig`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --agent` | The numeric agent ID to create the kubeconfig entry for. |
| `-c --cache-mode` | Mode to use for caching the token. Allowed values: keyring-filesystem-fallback, force-keyring, force-filesystem, no. (force-keyring) |
| `--check-revoked` | Check if a cached token is revoked. Requires an API call to GitLab, which adds latency every time a cached token is accessed. |
| `-h --help` | Show help for this command. |
| `--kubeconfig` | Use a particular kubeconfig file. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--token-expiry-duration` | Duration for generated token's validity. Minimum is 1 day. Expires at end of day, and ignores time. (24h0m0s) |
| `-u --use-context` | Use as default context. |

###### `glab cluster graph`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --agent` | The numeric agent ID to connect to. |
| `--apps` | Watch deployments, replicasets, daemonsets, and statefulsets in the apps/v1 group. |
| `--batch` | Watch jobs and cronjobs in the batch/v1 group. |
| `--cluster-rbac` | Watch clusterroles and clusterrolebindings in the rbac.authorization.k8s.io/v1 group. |
| `--core` | Watch pods, secrets, configmaps, and serviceaccounts in the core/v1 group. |
| `--crd` | Watch customresourcedefinitions in the apiextensions.k8s.io/v1 group. |
| `-h --help` | Show help for this command. |
| `--ignore-arc-direction` | Ignore arc direction when evaluating root connectivity. Requires GitLab and agent version 18.3 or later. |
| `--listen-addr` | Address to listen on. (localhost:0) |
| `--listen-net` | Network on which to listen for connections. (tcp) |
| `--log-watch-request` | Log watch request to stdout. Helpful for debugging. |
| `-n --namespace` | Namespaces to watch. If not specified, all namespaces are watched with label and field selectors filtering. |
| `--ns-expression` | Cel expression to select namespaces. Evaluated before a namespace is watched and on any updates for the namespace object. |
| `--ns-field-selector` | Field selector to select namespaces. |
| `--ns-label-selector` | Label selector to select namespaces. |
| `--rbac` | Watch roles and rolebindings in the rbac.authorization.k8s.io/v1 group. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-r --resource` | Resources to watch. You can see the list of resources your cluster supports by running 'kubectl api-resources'. |
| `--root-expression` | Cel expression to select root objects. Requires GitLab and agent version 18.3 or later. |
| `--stdin` | Read watch request from standard input. |

##### `glab config`

**Subcommands:**
- `edit` — Opens the glab configuration file.
- `get` — Prints the value of a given configuration key.
- `set` — Updates configuration with the value of a given key.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --global` | Use global config file. |
| `-h --help` | Show help for this command. |

###### `glab config edit`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-l --local` | Open '.git/glab-cli/config.yml' file instead of the global '~/.config/glab-cli/config.yml' file. |

###### `glab config get`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --global` | Read from global config file (~/.config/glab-cli/config.yml). (default checks 'Environment variables → Local → Global') |
| `-h --help` | Show help for this command. |
| `--host` | Get per-host setting. |

###### `glab config set`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --global` | Write to global '~/.config/glab-cli/config.yml' file rather than the repository's '.git/glab-cli/config.yml' file. |
| `-h --help` | Show help for this command. |
| `--host` | Set per-host setting. |

##### `glab container-registry`

**Subcommands:**
- `repository` — Manage container registry repositories.
- `tag` — Manage container registry tags.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab container-registry repository`

**Subcommands:**
- `delete` — Delete a container registry repository.
- `list` — List container registry repositories.
- `view` — View a container registry repository.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab container-registry repository delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-y --yes` | Skip the confirmation prompt. |

###### `glab container-registry repository list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --group` | List container registry repositories for a group. |
| `-h --help` | Show help for this command. |
| `--include-tag-details` | Fetch digest, size, and creation time for included tags. Makes one API call per tag. Project JSON output only. Implies --include-tags. |
| `--include-tags` | Include tags in the response. Project repositories only. |
| `--include-tags-count` | Include the number of tags in the response. Project repositories only. (true) |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab container-registry repository view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--include-tags` | Include tags in the response. |
| `--include-tags-count` | Include the number of tags in the response. (true) |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab container-registry tag`

**Subcommands:**
- `delete` — Delete container registry tags.
- `list` — List container registry repository tags.
- `view` — View a container registry tag.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab container-registry tag delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--keep-n` | Keep the latest N matching tags. Bulk deletion only; scheduled asynchronously. |
| `--name-regex-delete` | Regular expression for tag names to delete. Bulk deletion only; scheduled asynchronously. |
| `--name-regex-keep` | Regular expression for tag names to keep. Bulk deletion only; scheduled asynchronously. |
| `--older-than` | Delete tags older than the given duration, such as 7d or 1month. Bulk deletion only; scheduled asynchronously. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-y --yes` | Skip the confirmation prompt. |

###### `glab container-registry tag list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--details` | Fetch digest, size, and creation time for each tag. Makes one API call per tag. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab container-registry tag view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

##### `glab dependency-firewall`

**Subcommands:**
- `ci-summary` — Summarize Dependency Firewall activity from the CI log.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab dependency-firewall ci-summary`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

##### `glab deploy-key`

**Subcommands:**
- `add` — Add a deploy key to a GitLab project.
- `delete` — Deletes a single deploy key specified by the ID.
- `get` — Returns a single deploy key specified by the ID.
- `list` — Get a list of deploy keys for the current project.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab deploy-key add`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c --can-push` | If true, deploy keys can be used for pushing code to the repository. |
| `-e --expires-at` | The expiration date of the deploy key, using the ISO-8601 format: YYYY-MM-DDTHH:MM:SSZ. |
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-t --title` | New deploy key's title. |

###### `glab deploy-key delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab deploy-key get`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab deploy-key list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--show-id` | Shows IDs of deploy keys. |

##### `glab duo`

**Subcommands:**
- `cli` — Run the GitLab Duo CLI.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab duo cli`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--install` | Install the GitLab Duo CLI binary without running it. |
| `--update` | Check for and install updates to the binary. |
| `-y --yes` | Skip confirmation prompts. |

##### `glab gpg-key`

**Subcommands:**
- `add` — Add a GPG key to your GitLab account.
- `delete` — Deletes a single GPG key specified by the ID.
- `get` — Returns a single GPG key specified by the ID.
- `list` — Get a list of GPG keys for the currently authenticated user.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab gpg-key add`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab gpg-key delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab gpg-key get`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab gpg-key list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--show-id` | Shows IDs of GPG keys. |

##### `glab incident`

**Subcommands:**
- `close` — Close an incident.
- `list` — List project incidents.
- `note` — Comment on an incident in GitLab.
- `reopen` — Reopen a resolved incident.
- `subscribe` — Subscribe to an incident.
- `unsubscribe` — Unsubscribe from an incident.
- `view` — Display the title, body, and other information about an incident.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab incident close`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab incident list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-A --all` | Get all incidents. |
| `-a --assignee` | Filter incident by assignee <username>. |
| `--author` | Filter incident by author <username>. |
| `-c --closed` | Get only closed incidents. |
| `-C --confidential` | Filter by confidential incidents. |
| `-e --epic` | List issues belonging to a given epic (requires --group, no pagination support). |
| `-g --group` | Select a group or subgroup. Ignored if a repo argument is set. |
| `-h --help` | Show help for this command. |
| `--in` | Search in: title, description. (title,description) |
| `--jq` | Filter JSON output with a jq expression. |
| `-l --label` | Filter incident by label <name>. Multiple labels can be comma-separated or specified by repeating the flag. |
| `-m --milestone` | Filter incident by milestone <id>. |
| `--not-assignee` | Filter incident by not being assigned to <username>. |
| `--not-author` | Filter incident by not being by author(s) <username>. |
| `--not-label` | Filter incident by lack of label <name>. Multiple labels can be comma-separated or specified by repeating the flag. |
| `--order` | Order incident by <field>. Order options: created_at, updated_at, priority, due_date, relative_position, label_priority, milestone_due, popularity, weight. (created_at) |
| `-O --output` | Options: 'text' or 'json'. (text) |
| `-F --output-format` | Options: 'details', 'ids', 'urls'. (details) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--search` | Search <string> in the fields defined by '--in'. |
| `-s --sort` | Sort direction for --order field: asc or desc. (desc) |

###### `glab incident note`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-m --message` | Message text. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab incident reopen`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab incident subscribe`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab incident unsubscribe`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab incident view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c --comments` | Show incident comments and activities. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (20) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-s --system-logs` | Show system activities and logs. |
| `-w --web` | Open incident in a browser. Uses the default browser, or the browser specified in the $BROWSER variable. |

##### `glab issue`

**Subcommands:**
- `board` — Work with GitLab issue boards in the given project.
- `close` — Close an issue.
- `create` — Create an issue.
- `delete` — Delete an issue.
- `list` — List project issues.
- `note` — Comment on an issue in GitLab.
- `reopen` — Reopen a closed issue.
- `subscribe` — Subscribe to an issue.
- `unsubscribe` — Unsubscribe from an issue.
- `update` — Update issue.
- `view` — Display the title, body, and other information about an issue.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab issue board`

**Subcommands:**
- `create` — Create a project issue board.
- `view` — View project issue board.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository using the OWNER/REPO format or the project ID. Supports group namespaces. |

###### `glab issue board create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-n --name` | The name of the new board. |
| `-R --repo` | Select another repository using the OWNER/REPO format or the project ID. Supports group namespaces. |

###### `glab issue board view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --assignee` | Filter board issues by assignee username. |
| `-h --help` | Show help for this command. |
| `-l --labels` | Filter board issues by labels. Multiple labels can be comma-separated or specified by repeating the flag. |
| `-m --milestone` | Filter board issues by milestone. |
| `--paginate` | Make additional HTTP requests to retrieve all board issues. |
| `-R --repo` | Select another repository using the OWNER/REPO format or the project ID. Supports group namespaces. |

###### `glab issue close`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab issue create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --assignee` | Assign issue to people by their `usernames`. Multiple usernames can be comma-separated or specified by repeating the flag. |
| `-c --confidential` | Set an issue to be confidential. |
| `-d --description` | Issue description. Set to "-" to open an editor. |
| `--due-date` | A date in 'YYYY-MM-DD' format. |
| `--epic` | Id of the epic to add the issue to. |
| `-h --help` | Show help for this command. |
| `-l --label` | Add label by name. Multiple labels can be comma-separated or specified by repeating the flag. |
| `--link-type` | Type for the issue link. (relates_to) |
| `--linked-issues` | The IIDs of issues that this issue links to. Multiple IIDs can be comma-separated or specified by repeating the flag. |
| `--linked-mr` | The IID of a merge request in which to resolve all issues. |
| `-m --milestone` | The global ID or title of a milestone to assign. |
| `--no-editor` | Don't open editor to enter a description. If set to true, uses prompt. |
| `--recover` | Save the options to a file if the issue fails to be created. If the file exists, the options will be loaded from the recovery file. (EXPERIMENTAL) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--template` | Name of a template in '.gitlab/issue_templates/' to pre-populate the description. The '.md' extension is optional. Templates are loaded from the local repository only. |
| `-e --time-estimate` | Set time estimate for the issue. |
| `-s --time-spent` | Set time spent for the issue. |
| `-t --title` | Issue title. |
| `--web` | Continue issue creation with web interface. |
| `-w --weight` | Issue weight. Valid values are greater than or equal to 0. |
| `-y --yes` | Don't prompt for confirmation to submit the issue. |

###### `glab issue delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab issue list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-A --all` | Get all issues. |
| `-a --assignee` | Filter issue by assignee <username>. |
| `--author` | Filter issue by author <username>. |
| `-c --closed` | Get only closed issues. |
| `-C --confidential` | Filter by confidential issues. |
| `-e --epic` | List issues belonging to a given epic (requires --group, no pagination support). |
| `-g --group` | Select a group or subgroup. Ignored if a repo argument is set. |
| `-h --help` | Show help for this command. |
| `--in` | Search in: title, description. (title,description) |
| `-t --issue-type` | Filter issue by its type. Options: issue, incident, test_case. |
| `-i --iteration` | Filter issue by iteration <id>. |
| `--jq` | Filter JSON output with a jq expression. |
| `-l --label` | Filter issue by label <name>. Multiple labels can be comma-separated or specified by repeating the flag. |
| `-m --milestone` | Filter issue by milestone <id>. |
| `--not-assignee` | Filter issue by not being assigned to <username>. |
| `--not-author` | Filter issue by not being by author(s) <username>. |
| `--not-label` | Filter issue by lack of label <name>. Multiple labels can be comma-separated or specified by repeating the flag. |
| `--order` | Order issue by <field>. Order options: created_at, updated_at, priority, due_date, relative_position, label_priority, milestone_due, popularity, weight. (created_at) |
| `-O --output` | Options: 'text' or 'json'. (text) |
| `-F --output-format` | Options: 'details', 'ids', 'urls'. (details) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--search` | Search <string> in the fields defined by '--in'. |
| `-s --sort` | Sort direction for --order field: asc or desc. (desc) |

###### `glab issue note`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-m --message` | Message text. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab issue reopen`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab issue subscribe`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab issue unsubscribe`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab issue update`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --assignee` | Assign users by username. Prefix with '!' or '-' to remove from existing assignees, or '+' to add new. Otherwise, replace existing assignees with these users. Multiple usernames can be comma-separated or specified by repeating the flag. |
| `-c --confidential` | Make issue confidential. |
| `-d --description` | Issue description. Set to "-" to open an editor. |
| `--due-date` | A date in 'YYYY-MM-DD' format. |
| `-h --help` | Show help for this command. |
| `-l --label` | Add labels. |
| `--lock-discussion` | Lock discussion on issue. |
| `-m --milestone` | Title of the milestone to assign Set to "" or 0 to unassign. |
| `-p --public` | Make issue public. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-t --title` | Title of issue. |
| `--unassign` | Unassign all users. |
| `-u --unlabel` | Remove labels. |
| `--unlock-discussion` | Unlock discussion on issue. |
| `-w --weight` | Set weight of the issue. |

###### `glab issue view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c --comments` | Show issue comments and activities. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (20) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-s --system-logs` | Show system activities and logs. |
| `-w --web` | Open issue in a browser. Uses the default browser, or the browser specified in the $BROWSER variable. |

##### `glab iteration`

**Subcommands:**
- `list` — List project iterations.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab iteration list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --group` | List iterations for a group. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

##### `glab job`

**Subcommands:**
- `artifact` — Download all artifacts from the most recent pipeline.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab job artifact`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-l --list-paths` | Print the paths of downloaded artifacts. |
| `-p --path` | Path to download the artifact files. (./) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

##### `glab label`

**Subcommands:**
- `create` — Create a label in a project.
- `delete` — Delete a label from a project.
- `edit` — Edit a label in a project.
- `get` — Get information about a single label by ID.
- `list` — List labels in a project or group.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab label create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c --color` | Color of the label, in plain or HEX code. (#428BCA) |
| `-d --description` | Label description. |
| `-h --help` | Show help for this command. |
| `-n --name` | Name of the label. |
| `-p --priority` | Label priority. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab label delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab label edit`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c --color` | The color of the label given in 6-digit hex notation with leading ‘#’ sign. |
| `-d --description` | Label description. |
| `-h --help` | Show help for this command. |
| `-l --label-id` | The label ID we are updating. |
| `-n --new-name` | The new name of the label. |
| `-p --priority` | Label priority. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab label get`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab label list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --group` | List labels for a group. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

##### `glab mcp`

**Subcommands:**
- `serve` — Start a MCP server with stdio transport. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab mcp serve`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

##### `glab milestone`

**Subcommands:**
- `create` — Create a milestone in a project or group.
- `delete` — Delete a milestone from a project or group.
- `edit` — Edit a milestone in a project or group.
- `get` — Get a milestone by ID in a project or group.
- `list` — List milestones in a project or group.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab milestone create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--description` | Description of the milestone. |
| `--due-date` | Due date for the milestone. Expected in ISO 8601 format (2025-04-15T08:00:00Z). |
| `--group` | The ID or URL-encoded path of the group. |
| `-h --help` | Show help for this command. |
| `--project` | The ID or URL-encoded path of the project. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--start-date` | Start date for the milestone. Expected in ISO 8601 format (2025-04-15T08:00:00Z). |
| `--title` | Title of the milestone. |

###### `glab milestone delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--group` | The ID or URL-encoded path of the group. |
| `-h --help` | Show help for this command. |
| `--project` | The ID or URL-encoded path of the project. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab milestone edit`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--description` | Description of the milestone. |
| `--due-date` | Due date for the milestone. Expected in ISO 8601 format (2025-04-15T08:00:00Z). |
| `--group` | The ID or URL-encoded path of the group. |
| `-h --help` | Show help for this command. |
| `--project` | The ID or URL-encoded path of the project. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--start-date` | Start date for the milestone. Expected in ISO 8601 format (2025-04-15T08:00:00Z). |
| `--state` | State of the milestone. Can be 'activate' or 'close'. |
| `--title` | Title of the milestone. |

###### `glab milestone get`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--group` | The ID or URL-encoded path of the group. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `--project` | The ID or URL-encoded path of the project. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab milestone list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--group` | The ID or URL-encoded path of the group. |
| `-h --help` | Show help for this command. |
| `--include-ancestors` | Include milestones from all parent groups. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (20) |
| `--project` | The ID or URL-encoded path of the project. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--search` | Return only milestones with a title or description matching the provided string. |
| `--show-id` | Show IDs in table output. |
| `--state` | Return only 'active' or 'closed' milestones. |
| `--title` | Return only the milestones having the given title. |

##### `glab mr`

**Subcommands:**
- `approve` — Approve merge requests.
- `approvers` — List eligible approvers for merge requests in any state.
- `checkout` — Check out an open merge request.
- `close` — Close a merge request.
- `create` — Create a new merge request.
- `delete` — Delete a merge request.
- `diff` — View changes in a merge request.
- `for` — Create a new merge request for an issue.
- `issues` — Get issues that close when a merge request is merged.
- `list` — List merge requests.
- `merge` — Merge or accept a merge request.
- `note` — Manage comments and discussions on a merge request.
- `rebase` — Rebase the source branch of a merge request against its target branch.
- `reopen` — Reopen a merge request.
- `revoke` — Revoke approval on a merge request.
- `subscribe` — Subscribe to a merge request.
- `todo` — Add a to-do item to a merge request.
- `unsubscribe` — Unsubscribe from a merge request.
- `update` — Update a merge request.
- `view` — Display the title, body, and other information about a merge request.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr approve`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-s --sha` | Sha, which must match the SHA of the HEAD commit of the merge request. |

###### `glab mr approvers`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr checkout`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b --branch` | Check out merge request with name <branch>. |
| `-f --force` | Reset local branch to remote when they have diverged. Refuses if working tree has changes that would be lost. |
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-u --set-upstream-to` | Set tracking of checked-out branch to [REMOTE/]BRANCH. |

###### `glab mr close`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--allow-collaboration` | Allow commits from other members. Set to true/false to override project defaults, or omit to use project settings. |
| `-a --assignee` | Assign merge request to people by their `usernames`. Multiple usernames can be comma-separated or specified by repeating the flag. |
| `--auto-merge` | Set the merge request to merge when all merge checks pass. |
| `--copy-issue-labels` | Copy labels from issue to the merge request. Used with --related-issue. |
| `--create-source-branch` | Create a source branch if it does not exist. |
| `-d --description` | Supply a description for the merge request. Set to "-" to open an editor. |
| `--draft` | Mark merge request as a draft. |
| `-f --fill` | Do not prompt for title or description, and just use commit info. Sets `push` to `true`, and pushes the branch. |
| `--fill-commit-body` | Fill description with each commit body when multiple commits. Can only be used with --fill. |
| `-H --head` | Select another head repository using the `OWNER/REPO` or `GROUP/NAMESPACE/REPO` format, the project ID, or the full URL. |
| `-h --help` | Show help for this command. |
| `-l --label` | Add label by name. Multiple labels can be comma-separated or specified by repeating the flag. |
| `-m --milestone` | The global ID or title of a milestone to assign. |
| `--no-editor` | Don't open editor to enter a description. If true, uses prompt. Defaults to false. |
| `--push` | Push committed changes after creating merge request. Make sure you have committed changes. |
| `--recover` | Save the options to a file if the merge request creation fails. If the file exists, the options are loaded from the recovery file. (EXPERIMENTAL) |
| `-i --related-issue` | Create a merge request for an issue. If --title is not provided, uses the issue title. |
| `--remove-source-branch` | Remove source branch on merge. Set to true/false to override project defaults, or omit to use project settings. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--reviewer` | Request review from users by their `usernames`. Multiple usernames can be comma-separated or specified by repeating the flag. |
| `--signoff` | Append a DCO signoff to the merge request description. |
| `-s --source-branch` | Create a merge request from this branch. Default is the current branch. |
| `--squash-before-merge` | Squash commits into a single commit when merging. Set to true/false to override project defaults, or omit to use project settings. |
| `-b --target-branch` | The target or base branch into which you want your code merged into. |
| `--template` | Name of a template in '.gitlab/merge_request_templates/' to pre-populate the description. The '.md' extension is optional. Templates are loaded from the local repository only. |
| `-t --title` | Supply a title for the merge request. |
| `-w --web` | Continue merge request creation in a browser. |
| `--wip` | Mark merge request as a draft. Alternative to --draft. |
| `-y --yes` | Skip submission confirmation prompt. Use --fill to skip all optional prompts. |

###### `glab mr delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr diff`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--color` | Use color in diff output: always, never, auto. (auto) |
| `-h --help` | Show help for this command. |
| `--raw` | Use raw diff format that can be piped to commands. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr for`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--allow-collaboration` | Allow commits from other members. |
| `-a --assignee` | Assign merge request to people by their IDs. Multiple values should be comma-separated. |
| `--draft` | Mark merge request as a draft. (true) |
| `-h --help` | Show help for this command. |
| `-l --label` | Add label by name. Multiple labels should be comma-separated. |
| `-m --milestone` | Add milestone by <id> for this merge request. (-1) |
| `--remove-source-branch` | Remove source branch on merge. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-b --target-branch` | The target or base branch into which you want your code merged. |
| `--wip` | Mark merge request as a work in progress. Overrides --draft. |
| `--with-labels` | Copy labels from issue to the merge request. |

###### `glab mr issues`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-A --all` | Get all merge requests. |
| `-a --assignee` | Get only merge requests assigned to users. Multiple users can be comma-separated or specified by repeating the flag. |
| `--author` | Filter merge request by author <username>. |
| `-c --closed` | Get only closed merge requests. |
| `--created-after` | Filter merge requests created after a certain date (ISO 8601 format). |
| `--created-before` | Filter merge requests created before a certain date (ISO 8601 format). |
| `--deployed-after` | Filter merge requests deployed after a certain date (ISO 8601 format). |
| `--deployed-before` | Filter merge requests deployed before a certain date (ISO 8601 format). |
| `-d --draft` | Filter by draft merge requests. |
| `--environment` | Filter merge requests deployed to the given environment <name>. |
| `-g --group` | Select a group/subgroup. This option is ignored if a repo argument is set. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-l --label` | Filter merge request by label <name>. Multiple labels can be comma-separated or specified by repeating the flag. |
| `-M --merged` | Get only merged merge requests. |
| `-m --milestone` | Filter merge request by milestone <id>. |
| `--not-draft` | Filter by non-draft merge requests. |
| `--not-label` | Filter merge requests by not having label <name>. Multiple labels can be comma-separated or specified by repeating the flag. |
| `-o --order` | Order merge requests by <field>. Order options: created_at, updated_at, merged_at, title, priority, label_priority, milestone_due, and popularity. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-r --reviewer` | Get only merge requests with users as reviewer. Multiple users can be comma-separated or specified by repeating the flag. |
| `--search` | Filter by <string> in title and description. |
| `-S --sort` | Sort direction for --order field: asc or desc. |
| `-s --source-branch` | Filter by source branch <name>. |
| `-t --target-branch` | Filter by target branch <name>. |

###### `glab mr merge`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--auto-merge` | Set auto-merge. (true) |
| `-h --help` | Show help for this command. |
| `-m --message` | Custom merge commit message. |
| `-r --rebase` | Rebase the commits onto the base branch. |
| `-d --remove-source-branch` | Remove source branch on merge. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--sha` | Merge only if the HEAD of the source branch matches this SHA. Use to ensure that only reviewed commits are merged. |
| `-s --squash` | Squash commits on merge. |
| `--squash-message` | Custom squash commit message. |
| `-y --yes` | Skip submission confirmation prompt. |

###### `glab mr note`

**Subcommands:**
- `create` — Create a comment or discussion on a merge request. (EXPERIMENTAL)
- `delete` — <note-id> [<id> | <branch>] [--flags]  Delete a note from a merge request. (EXPERIMENTAL)
- `list` — List merge request discussions. (EXPERIMENTAL)
- `reopen` — <discussion-id> [<id> | <branch>]      Reopen a discussion on a merge request. (EXPERIMENTAL)
- `resolve` — <discussion-id> [<id> | <branch>]     Resolve a discussion on a merge request. (EXPERIMENTAL)
- `update` — <note-id> [<id> | <branch>] [--flags]  Update the body of a note on a merge request. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr note create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--file` | File path for a diff comment, like <path/to/file>. Targets the latest merge request diff version. |
| `-h --help` | Show help for this command. |
| `--line` | Line in the new version. A single line number, like 42, or a range, like 10:15. |
| `-m --message` | Comment or note message. |
| `--old-line` | Line in the old version, for commenting on a removed line. |
| `--reply` | Reply to an existing discussion. Accepts a full discussion ID or a unique prefix of at least 8 characters. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--resolvable` | Create the note as a resolvable discussion thread. Set to false to create a non-resolvable note. (true) |
| `--unique` | Don't create a note if a note with the same body already exists. Reads all merge request comments first. |

###### `glab mr note delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-y --yes` | Skip confirmation prompt. |

###### `glab mr note list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--file` | Show only diff notes on this file path. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--state` | Resolution state: all, resolved, unresolved. (all) |
| `-t --type` | Note type: all, general, diff, system. (all) |

###### `glab mr note reopen`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr note resolve`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr note update`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-m --message` | New note body. If omitted, opens an editor or reads from stdin. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr rebase`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--skip-ci` | Rebase merge request while skipping CI/CD pipeline. |

###### `glab mr reopen`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr revoke`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr subscribe`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr todo`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr unsubscribe`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab mr update`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --assignee` | Assign users via username. Prefix with '!' or '-' to remove from existing assignees, '+' to add. Otherwise, replace existing assignees with given users. Multiple usernames can be comma-separated or specified by repeating the flag. |
| `-d --description` | Merge request description. Set to "-" to open an editor. |
| `--draft` | Mark merge request as a draft. |
| `-f --fill` | Do not prompt for title or body, and just use commit info. |
| `--fill-commit-body` | Fill body with each commit body when multiple commits. Can only be used with --fill. |
| `-h --help` | Show help for this command. |
| `-l --label` | Add labels. |
| `--lock-discussion` | Lock discussion on merge request. |
| `-m --milestone` | Title of the milestone to assign. Set to "" or 0 to unassign. |
| `-r --ready` | Mark merge request as ready to be reviewed and merged. |
| `--remove-source-branch` | Toggles the removal of the source branch on merge. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--reviewer` | Request review from users by their usernames. Prefix with '!' or '-' to remove from existing reviewers, '+' to add. Otherwise, replace existing reviewers with given users. Multiple usernames can be comma-separated or specified by repeating the flag. |
| `--squash-before-merge` | Toggles the option to squash commits into a single commit when merging. |
| `--target-branch` | Set target branch. |
| `-t --title` | Title of merge request. |
| `--unassign` | Unassign all users. |
| `-u --unlabel` | Remove labels. |
| `--unlock-discussion` | Unlock discussion on merge request. |
| `--wip` | Mark merge request as a work in progress. Alternative to --draft. |
| `-y --yes` | Skip confirmation prompt. |

###### `glab mr view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c --comments` | Show merge request comments and activities. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. |
| `-P --per-page` | Number of items to list per page. (20) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--resolved` | Show only resolved discussions (implies --comments). |
| `-s --system-logs` | Show system activities and logs. |
| `--unresolved` | Show only unresolved discussions (implies --comments). |
| `-w --web` | Open merge request in a browser. Uses default browser or browser specified in BROWSER variable. |

##### `glab opentofu`

**Subcommands:**
- `init` — Initialize OpenTofu or Terraform.
- `state` — Work with the OpenTofu or Terraform states.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab opentofu init`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b --binary` | Name or path of the OpenTofu or Terraform binary to use for the initialization. (tofu) |
| `-d --directory` | Directory of the OpenTofu or Terraform project to initialize. (.) |
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab opentofu state`

**Subcommands:**
- `delete` — Delete a state or a specific version of a state.
- `download` — Download the given state and output as JSON to stdout.
- `list` — List states.
- `lock` — Lock the given state.
- `unlock` — Unlock the given state.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab opentofu state delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-f --force` | Force delete the state without prompting. |
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab opentofu state download`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab opentofu state list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab opentofu state lock`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab opentofu state unlock`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

##### `glab orbit`

**Subcommands:**
- `local` — Run the Orbit local CLI (Experimental)
- `remote` — Interact with the remote GitLab Knowledge Graph. (EXPERIMENTAL)
- `setup` — Guided setup for Orbit: verify access, install the skill, install the local CLI. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab orbit local`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--install` | Install the Orbit local CLI binary without running it. |
| `--update` | Check for and install updates to the binary. |
| `-y --yes` | Skip confirmation prompts. |

###### `glab orbit remote`

**Subcommands:**
- `dsl` — Show the GitLab Knowledge Graph query DSL JSON Schema. (EXPERIMENTAL)
- `graph-status` — Show indexing progress for a namespace or project. (EXPERIMENTAL)
- `query` — Execute a GitLab Knowledge Graph query. (EXPERIMENTAL)
- `schema` — Show the GitLab Knowledge Graph ontology. (EXPERIMENTAL)
- `status` — Show GitLab Knowledge Graph cluster health. (EXPERIMENTAL)
- `tools` — Show the GitLab Knowledge Graph MCP tool manifest. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab orbit remote dsl`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--hostname` | Gitlab hostname to query. Defaults to the current repository's host or `gitlab.com`. |

###### `glab orbit remote graph-status`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--full-path` | Full path of a project or group, such as `gitlab-org/gitlab`. Cannot be used with the ID flags. |
| `-h --help` | Show help for this command. |
| `--hostname` | Gitlab hostname to query. Defaults to the current repository's host or `gitlab.com`. |
| `--jq` | Filter JSON output with a jq expression. |
| `--namespace-id` | Namespace (group) ID to inspect. Cannot be used with --project-id or --full-path. |
| `--project-id` | Project ID to inspect. Cannot be used with --namespace-id or --full-path. |
| `--response-format` | Server response format: `raw` (structured JSON) or `llm` (compact GOON/TOON for agents). (raw) |

###### `glab orbit remote query`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--hostname` | Gitlab hostname to query. Defaults to the current repository's host or `gitlab.com`. |
| `--response-format` | Server response format: `llm` (compact GOON/TOON for agents) or `raw` (structured JSON). (llm) |

###### `glab orbit remote schema`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--hostname` | Gitlab hostname to query. Defaults to the current repository's host or `gitlab.com`. |
| `--jq` | Filter JSON output with a jq expression. |

###### `glab orbit remote status`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--hostname` | Gitlab hostname to query. Defaults to the current repository's host or `gitlab.com`. |
| `--jq` | Filter JSON output with a jq expression. |

###### `glab orbit remote tools`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--hostname` | Gitlab hostname to query. Defaults to the current repository's host or `gitlab.com`. |
| `--jq` | Filter JSON output with a jq expression. |

###### `glab orbit setup`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --global` | Install the Orbit skill at user scope (`~/.agents/skills/`). |
| `-h --help` | Show help for this command. |
| `--hostname` | Gitlab hostname to verify. Defaults to the current repository's host or `gitlab.com`. |
| `--path` | Install the Orbit skill to the directory at `<path>`. |
| `--skip-local` | Skip the local CLI binary install step. |
| `--skip-skill` | Skip the agent-skill install step. |
| `--upgrade` | Re-Fetch the skill and update the local CLI binary in place. |
| `-y --yes` | Skip every confirmation prompt. |

##### `glab packages`

**Subcommands:**
- `delete` — Delete a package from a project's package registry.
- `download` — Download a file from a project's package registry.
- `list` — List packages in a project's package registry.
- `upload` — Upload a file to a project's package registry.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab packages delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-y --yes` | Skip the confirmation prompt. |

###### `glab packages download`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--filename` | Name of the file within the package to download. |
| `--force` | Overwrite the target file if it already exists. |
| `-h --help` | Show help for this command. |
| `-n --name` | Name of the package. |
| `--no-verify` | Do not verify the checksum of the downloaded file. Warning: when enabled, this setting allows the download of files that are corrupt or tampered with. |
| `-p --path` | Directory to save the file in (keeps its original name) or a full file path to rename it. Defaults to the original name in the current directory. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--version` | Version of the package. |

###### `glab packages list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-n --name` | Filter packages by name (substring match). |
| `--package-type` | Filter packages by type. One of: composer, conan, debian, generic, golang, helm, maven, npm, nuget, pypi, terraform_module. |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab packages upload`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--filename` | Name to store the file under. Defaults to the local file name. |
| `-h --help` | Show help for this command. |
| `-n --name` | Name of the package. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-v --version` | Version of the package. |

##### `glab release`

**Subcommands:**
- `create` — Create a new GitLab release, or update an existing one.
- `delete` — Delete a GitLab release.
- `download` — Download asset files from a GitLab release.
- `list` — List releases in a repository.
- `upload` — Upload release asset files or links to a GitLab release.
- `view` — View information about a GitLab release.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab release create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --assets-links` | Json string representation of assets links. See Examples for usage. |
| `-h --help` | Show help for this command. |
| `-m --milestone` | The title of each milestone the release is associated with. Multiple milestones can be comma-separated or specified by repeating the flag. |
| `-n --name` | The release name or title. |
| `--no-close-milestone` | Prevent closing milestones after creating the release. |
| `--no-update` | Prevent updating the existing release. |
| `-N --notes` | The release notes or description. Accepts Markdown. |
| `-F --notes-file` | Read release notes 'file'. To read from stdin, use '-'. |
| `--package-name` | The package name, when uploading assets to the generic package release with --use-package-registry. (release-assets) |
| `--publish-to-catalog` | (Experimental) Publish the release to the GitLab CI/CD catalog. |
| `-r --ref` | If the specified tag doesn't exist, create a release from the ref and tag it with the specified tag name. Accepts a commit SHA, tag name, or branch name. |
| `-D --released-at` | Iso 8601 datetime when the release was ready. Defaults to the current datetime. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-T --tag-message` | Message to use if creating a new annotated tag. |
| `--use-package-registry` | Upload release assets to the generic package registry of the project. Overrides the GITLAB_RELEASE_ASSETS_USE_PACKAGE_REGISTRY environment variable. |

###### `glab release delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-t --with-tag` | Delete the associated tag. |
| `-y --yes` | Skip the confirmation prompt. |

###### `glab release download`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-n --asset-name` | Download only assets that match the name or a glob pattern. |
| `-D --dir` | Directory to download the release assets to. (.) |
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab release list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab release upload`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --assets-links` | `Json` string representation of assets links, like: `--assets-links='[{"name": "Asset1", "url":"https://<domain>/some/location/1", "link_type": "other", "direct_asset_path": "path/to/file"}]'`. |
| `-h --help` | Show help for this command. |
| `--package-name` | The package name to use when uploading the assets to the generic package release with --use-package-registry. (release-assets) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--use-package-registry` | Upload release assets to the generic package registry of the project. Alternatively to this flag you may also set the GITLAB_RELEASE_ASSETS_USE_PACKAGE_REGISTRY environment variable to either the value true or 1. The flag takes precedence over this environment variable. |

###### `glab release view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-w --web` | Open the release in the browser. |

##### `glab repo`

**Subcommands:**
- `archive` — Get an archive of the repository.
- `clone` — [-- <gitflags>...] [--flags]  Clone a GitLab repository or project.
- `contributors` — Get repository contributors list.
- `create` — Create a new GitLab project/repository.
- `delete` — Delete an existing project on GitLab.
- `fork` — Fork a GitLab repository.
- `list` — Get list of repositories.
- `members` — Manage project members.
- `mirror` — Configure mirroring on an existing project to sync with a remote repository.
- `prune` — Delete local Git branches whose merge request has been merged.
- `publish` — Publishes resources in the project.
- `remote` — Manage Git remotes for a GitLab project.
- `search` — Search for GitLab repositories and projects by name.
- `transfer` — Transfer a repository to a new namespace.
- `update` — Update an existing GitLab project or repository.
- `view` — View a project or repository.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab repo archive`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-f --format` | Optional. Specify format if you want a downloaded archive: tar.gz, tar.bz2, tbz, tbz2, tb2, bz2, tar, zip. (zip) |
| `-h --help` | Show help for this command. |
| `-s --sha` | The commit SHA to download. A tag, branch reference, or SHA can be used. Defaults to the tip of the default branch if not specified. |

###### `glab repo clone`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --group` | Specify the group to clone repositories from. |
| `-p --preserve-namespace` | Clone the repository in a subdirectory based on namespace. |
| `--active` | Limit by project status. When true, returns active projects. When false, returns projects that are archived or marked for deletion. Used with the --group flag. |
| `-a --archived` | Limit by archived status. Use with '-a=false' to exclude archived repositories. Used with the --group flag. |
| `-G --include-subgroups` | Include projects in subgroups of this group. Default is true. Used with the --group flag. (true) |
| `-m --mine` | Limit by projects in the group owned by the current authenticated user. Used with the --group flag. |
| `-v --visibility` | Limit by visibility: public, internal, private. Used with the --group flag. |
| `-I --with-issues-enabled` | Limit by projects with the issues feature enabled. Default is false. Used with the --group flag. |
| `-M --with-mr-enabled` | Limit by projects with the merge request feature enabled. Default is false. Used with the --group flag. |
| `-S --with-shared` | Include projects shared to this group. Default is true. Used with the --group flag. (true) |
| `--paginate` | Make additional HTTP requests to fetch all pages of projects before cloning. Respects --per-page. |
| `--page` | Page number. (1) |
| `--per-page` | Number of items to list per page. (30) |
| `--wiki` | Clone the project's wiki repository. |
| `-h --help` | Show help for this command. |

###### `glab repo contributors`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-o --order` | Return contributors ordered by name, email, or commits (orders by commit date) fields. (commits) |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-s --sort` | Sort direction for --order field: asc or desc. |

###### `glab repo create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--defaultBranch` | Branch name for the new project, overriding both the GitLab instance default and your local git configuration. |
| `-d --description` | Description of the new project. Set to "-" to open an editor. |
| `-g --group` | Namespace or group for the new project. Defaults to the current user's namespace. |
| `-h --help` | Show help for this command. |
| `--internal` | Make project internal: visible to any authenticated user. Default. |
| `-n --name` | Name of the new project. |
| `-p --private` | Make project private: visible only to project members. |
| `-P --public` | Make project public: visible without any authentication. |
| `--readme` | Initialize project with `README.md`. The repository is cloned locally after creation to ensure the local branch matches the remote. |
| `--remoteName` | Remote name for the Git repository you're in. Defaults to `origin` if not provided. (origin) |
| `-s --skipGitInit` | Skip local repository setup (skips both 'git init' and cloning). |
| `-t --tag` | The list of tags for the project. |

###### `glab repo delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-y --yes` | Skip the confirmation prompt and immediately delete the project. |

###### `glab repo fork`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c --clone` | Clone the fork. Options: true, false. |
| `-h --help` | Show help for this command. |
| `-n --name` | The name assigned to the new project after forking. |
| `-p --path` | The path assigned to the new project after forking. |
| `--remote` | Add a remote for the fork. Options: true, false. |

###### `glab repo list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --all` | List all projects on the instance. Removes the ownership filter. Results are still paginated. Use --page to navigate. |
| `--archived` | Limit by archived status. Use 'false' to exclude archived repositories. Used with the '--group' flag. |
| `-g --group` | Return repositories in only the given group. |
| `-h --help` | Show help for this command. |
| `-G --include-subgroups` | Include projects in subgroups of this group. Default is false. Used with the '--group' flag. |
| `--jq` | Filter JSON output with a jq expression. |
| `--member` | List only projects of which you are a member. |
| `-m --mine` | List only projects you own. Default if no filters are provided. |
| `-o --order` | Return repositories ordered by id, name, path, created_at, updated_at, similarity, star_count, last_activity_at. (last_activity_at) |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-s --sort` | Sort direction for --order field: asc or desc. |
| `--starred` | List only starred projects. |
| `-u --user` | List user projects. |

###### `glab repo members`

**Subcommands:**
- `add` — Add a member to the project.
- `remove` — Remove a member from the project.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab repo members add`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-e --expires-at` | Expiration date for the membership (YYYY-MM-DD) |
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-r --role` | Role for the user (guest, reporter, developer, maintainer, owner) (developer) |
| `--role-id` | Id of a custom role defined in the project or group. |
| `-u --user-id` | User ID instead of username. |
| `--username` | Username instead of user-id. |

###### `glab repo members remove`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-u --user-id` | User ID instead of username. |
| `--username` | Username instead of user-id. |

###### `glab repo mirror`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--allow-divergence` | Determines if divergent refs are skipped. |
| `--direction` | Mirror direction. Options: pull, push. (pull) |
| `--enabled` | Determines if the mirror is enabled. (true) |
| `-h --help` | Show help for this command. |
| `--protected-branches-only` | Determines if only protected branches are mirrored. |
| `--url` | The target URL to which the repository is mirrored. |

###### `glab repo prune`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--dry-run` | Preview branches that would be deleted without deleting them. |
| `-e --exclude` | Branch name or glob pattern to exclude. Comma-separated or repeated. |
| `-h --help` | Show help for this command. |
| `--merged` | Use 'git branch --merged' instead of querying GitLab. Detects fast-forward merges only. |
| `-y --yes` | Skip the confirmation prompt. |

###### `glab repo publish`

**Subcommands:**
- `catalog` — [Experimental] Publishes CI/CD components to the catalog.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab repo publish catalog`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab repo remote`

**Subcommands:**
- `add` — Add a Git remote for a GitLab project.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab repo remote add`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-n --name` | Name for the remote (default: first path component) |
| `-p --protocol` | Git protocol: ssh, https (default: git_protocol config) |

###### `glab repo search`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (20) |
| `-s --search` | A string contained in the project name. |

###### `glab repo transfer`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-t --target-namespace` | The namespace where your project should be transferred to. |
| `-y --yes` | Warning: Skip confirmation prompt and force transfer operation. Transfer cannot be undone. |

###### `glab repo update`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--archive` | Whether the project should be archived. |
| `--defaultBranch` | New default branch for the project. |
| `-d --description` | New description for the project. |
| `-h --help` | Show help for this command. |

###### `glab repo view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b --branch` | View a specific branch of the repository. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-w --web` | Open a project in the browser. |

##### `glab runner`

**Subcommands:**
- `assign` — Assign a runner to a project.
- `delete` — Delete a runner.
- `jobs` — List jobs processed by a runner.
- `list` — List runners.
- `managers` — List runner managers.
- `unassign` — Unassign a runner from a project.
- `update` — Update a runner.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab runner assign`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab runner delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-f --force` | Skip confirmation prompt. |
| `-h --help` | Show help for this command. |

###### `glab runner jobs`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `--order-by` | Order jobs by: id. (id) |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--sort` | Sort order: asc or desc. (desc) |
| `--status` | Filter jobs by status: running, success, failed, canceled. |

###### `glab runner list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --group` | List runners for a group. Ignored if -R/--repo is set. |
| `-h --help` | Show help for this command. |
| `-i --instance` | List all runners available to the user (instance scope). |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab runner managers`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab runner unassign`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab runner update`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--pause` | Pause the runner. |
| `--unpause` | Resume a paused runner. |

##### `glab runner-controller`

**Subcommands:**
- `create` — Create a runner controller. (EXPERIMENTAL)
- `delete` — Delete a runner controller. (EXPERIMENTAL)
- `get` — Get details of a runner controller. (EXPERIMENTAL)
- `list` — List runner controllers. (EXPERIMENTAL)
- `scope` — Manage runner controller scopes. (EXPERIMENTAL)
- `token` — Manage runner controller tokens. (EXPERIMENTAL)
- `update` — Update a runner controller. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab runner-controller create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-d --description` | Description of the runner controller. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `--state` | State of the runner controller: disabled, enabled, dry_run. |

###### `glab runner-controller delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-f --force` | Skip confirmation prompt. |
| `-h --help` | Show help for this command. |

###### `glab runner-controller get`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |

###### `glab runner-controller list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items per page. (30) |

###### `glab runner-controller scope`

**Subcommands:**
- `create` — Create a scope for a runner controller. (EXPERIMENTAL)
- `delete` — Delete a scope from a runner controller. (EXPERIMENTAL)
- `list` — List scopes for a runner controller. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab runner-controller scope create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--instance` | Add an instance-level scope. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `--runner` | Add a runner-level scope for the specified runner ID. Multiple IDs can be comma-separated or specified by repeating the flag. |

###### `glab runner-controller scope delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-f --force` | Skip confirmation prompt. |
| `-h --help` | Show help for this command. |
| `--instance` | Remove an instance-level scope. |
| `--runner` | Remove a runner-level scope for the specified runner ID. Multiple IDs can be comma-separated or specified by repeating the flag. |

###### `glab runner-controller scope list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |

###### `glab runner-controller token`

**Subcommands:**
- `create` — Create a token for a runner controller. (EXPERIMENTAL)
- `list` — List tokens for a runner controller. (EXPERIMENTAL)
- `revoke` — Revoke a token from a runner controller. (EXPERIMENTAL)
- `rotate` — Rotate a token for a runner controller. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab runner-controller token create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-d --description` | Description of the token. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |

###### `glab runner-controller token list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items per page. (30) |

###### `glab runner-controller token revoke`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-f --force` | Skip confirmation prompt. |
| `-h --help` | Show help for this command. |

###### `glab runner-controller token rotate`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-f --force` | Skip confirmation prompt. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |

###### `glab runner-controller update`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-d --description` | Description of the runner controller. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `--state` | State of the runner controller: disabled, enabled, dry_run. |

##### `glab schedule`

**Subcommands:**
- `create` — Create a new pipeline schedule.
- `delete` — Delete a pipeline schedule by ID.
- `list` — List pipeline schedules in a project.
- `run` — Trigger a pipeline schedule to run immediately.
- `update` — Update a pipeline schedule.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab schedule create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--active` | Whether or not the schedule is active. (true) |
| `--cron` | Cron interval pattern. |
| `--cronTimeZone` | Cron timezone. (UTC) |
| `--description` | Description of the schedule. |
| `-h --help` | Show help for this command. |
| `--ref` | Target branch or tag. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--variable` | Pass variables to schedule in the format <key>:<value>. Repeat flag for multiple variables. |

###### `glab schedule delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab schedule list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab schedule run`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab schedule update`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--active` | Whether or not the schedule is active. (to not change) |
| `--create-variable` | Pass new variables to schedule in format <key>:<value>. |
| `--cron` | Cron interval pattern. |
| `--cronTimeZone` | Cron timezone. |
| `--delete-variable` | Pass variables you want to delete from schedule in format <key>. |
| `--description` | Description of the schedule. |
| `-h --help` | Show help for this command. |
| `--ref` | Target branch or tag. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--update-variable` | Pass updated variables to schedule in format <key>:<value>. |

##### `glab search`

**Subcommands:**
- `semantic` — Search project code using natural language.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab search semantic`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-d --directory-path` | Restrict search to files under this path (e.g. app/services/). |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `--knn` | Nearest neighbours to retrieve (1–100). Defaults to 64 server-side. |
| `-l --limit` | Maximum number of results (1–100). Defaults to 20 server-side. |
| `-F --output` | Format output as: text, json. (text) |
| `-q --query` | Natural language search query. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

##### `glab securefile`

**Subcommands:**
- `create` — Upload a new secure file to a project.
- `download` — Download one or more secure files from a project.
- `get` — Get details of a secure file by ID.
- `list` — List secure files in a project.
- `remove` — Remove a secure file from a project.
- `update` — Update a secure file in a project.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab securefile create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab securefile download`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--all` | Download all of a project's secure files. Files are downloaded with their original name and file extension. |
| `--force-download` | Force download file(s) even if checksum verification fails. Warning: when enabled, this setting allows the download of files that are corrupt or tampered with. |
| `-h --help` | Show help for this command. |
| `--id` | Id of the secure file to download. |
| `--name` | Name of the secure file to download. Saves the file with this name, or to the path specified by --path. |
| `--no-verify` | Do not verify the checksum of the downloaded file(s). Warning: when enabled, this setting allows the download of files that are corrupt or tampered with. |
| `--output-dir` | Output directory for files downloaded with --all. (.) |
| `-p --path` | Path to download the secure file to, including filename and extension. (./downloaded.tmp) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab securefile get`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab securefile list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab securefile remove`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--id` | Id of the secure file to remove. |
| `--name` | Name of the secure file to remove. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-y --yes` | Skip the confirmation prompt. |

###### `glab securefile update`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-y --yes` | Skip the confirmation prompt. |

##### `glab security`

**Subcommands:**
- `config` — Configure security scan profiles for a project. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab security config`

**Subcommands:**
- `disable` — Disable a security scan profile for a project. (EXPERIMENTAL)
- `enable` — Enable a security scan profile for a project. (EXPERIMENTAL)
- `status` — Show the status of a security scan profile for a project. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab security config disable`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab security config enable`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab security config status`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

##### `glab skills`

**Subcommands:**
- `install` — Install glab's bundled agent skills. (EXPERIMENTAL)
- `list` — List the available bundled agent skills. (EXPERIMENTAL)
- `update` — Update installed agent skills to the current shipped version. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab skills install`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-f --force` | Overwrite existing skill files. |
| `-g --global` | Install skills at user scope (~/.agents/skills/). |
| `-h --help` | Show help for this command. |
| `--path` | Install skills to the directory at <path>. |

###### `glab skills list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab skills update`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--all` | Update every installed skill. |
| `-h --help` | Show help for this command. |

##### `glab snippet`

**Subcommands:**
- `create` — -t <title> <file1>                                        [<file2>...] [--flags]  Create a new snippet.
- `glab` — -t <title> -f <filename>  # reads from stdin

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab snippet create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-d --description` | Description of the snippet. Set to "-" to open an editor. |
| `-f --filename` | Filename of the snippet in GitLab. |
| `-h --help` | Show help for this command. |
| `-p --personal` | Create a personal snippet. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-t --title` | (Required) Title of the snippet. |
| `-v --visibility` | Limit by visibility: 'public', 'internal', or 'private'. (private) |

##### `glab ssh-key`

**Subcommands:**
- `add` — Add an SSH key to your GitLab account.
- `delete` — Deletes a single SSH key specified by the ID.
- `get` — Returns a single SSH key specified by the ID.
- `list` — Get a list of SSH keys for the currently authenticated user.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab ssh-key add`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-e --expires-at` | The expiration date of the SSH key. Uses ISO 8601 format: YYYY-MM-DDTHH:MM:SSZ. |
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-t --title` | New SSH key's title. |
| `-u --usage-type` | Usage scope for the key. Possible values: 'auth', 'signing' or 'auth_and_signing'. Default value: 'auth_and_signing'. (auth_and_signing) |

###### `glab ssh-key delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab ssh-key get`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (20) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab ssh-key list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--show-id` | Shows IDs of SSH keys. |

##### `glab stack`

**Subcommands:**
- `amend` — Save more changes to a stacked diff. (EXPERIMENTAL)
- `create` — Create a new stacked diff. (EXPERIMENTAL)
- `first` — Moves to the first diff in the stack. (EXPERIMENTAL)
- `infer` — Add layers to a stack based on a range of commits. (EXPERIMENTAL)
- `last` — Moves to the last diff in the stack. (EXPERIMENTAL)
- `list` — Lists all entries in the stack. (EXPERIMENTAL)
- `move` — Moves to any selected entry in the stack. (EXPERIMENTAL)
- `next` — Moves to the next diff in the stack. (EXPERIMENTAL)
- `prev` — Moves to the previous diff in the stack. (EXPERIMENTAL)
- `reorder` — Reorder a stack of merge requests. (EXPERIMENTAL)
- `save` — Save your progress within a stacked diff. (EXPERIMENTAL)
- `switch` — Switch between stacks. (EXPERIMENTAL)
- `sync` — Sync and submit progress on a stacked diff. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab stack amend`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --all` | Automatically stage modified and deleted tracked files. |
| `-d --description` | A description of the change. |
| `-h --help` | Show help for this command. |
| `-m --message` | Alias for the description flag. |
| `--no-verify` | Bypass the pre-commit and commit-msg hooks of git-commit(1). |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--reword` | Only update the commit message without staging any files. |

###### `glab stack create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab stack first`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab stack infer`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-n --name` | Name for the new stack (used when creating a stack) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab stack last`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab stack list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab stack move`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab stack next`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab stack prev`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab stack reorder`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab stack save`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --all` | Automatically stage modified and deleted tracked files. |
| `-d --description` | Description of the change. |
| `-h --help` | Show help for this command. |
| `-m --message` | Alias for the description flag. |
| `--no-verify` | Bypass the pre-commit and commit-msg hooks of git-commit(1). |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab stack switch`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab stack sync`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --assignee` | Assign merge request to people by their `usernames`. Multiple usernames can be comma-separated or specified by repeating the flag. |
| `-h --help` | Show help for this command. |
| `-l --label` | Add label by `name`. Multiple labels can be comma-separated or specified by repeating the flag. |
| `--no-verify` | Bypass the pre-push hook. (See githooks(5) for more information.) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--reviewer` | Request review from users by their `usernames`. Multiple usernames can be comma-separated or specified by repeating the flag. |
| `--skip-mr-creation` | Skip creating merge requests for branches that don't have one yet. |
| `--update-base` | Rebase the stack onto the latest version of the base branch. |

##### `glab todo`

**Subcommands:**
- `done` — Mark a to-do item as done.
- `list` — List your to-do items.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab todo done`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--all` | Mark all pending to-do items as done. |
| `-h --help` | Show help for this command. |

###### `glab todo list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --action` | Filter by action: assigned, mentioned, build_failed, marked, approval_required, directly_addressed. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |
| `-s --state` | Filter by state: pending, done, all. (pending) |
| `-t --type` | Filter by target type: Issue, MergeRequest. |

##### `glab token`

**Subcommands:**
- `create` — Creates user, group, or project access tokens.
- `list` — List user, group, or project access tokens.
- `revoke` — Revoke user, group, or project access tokens.
- `rotate` — Rotate user, group, or project access tokens.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab token create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-A --access-level` | Access level of the token: one of 'guest', 'reporter', 'developer', 'maintainer', 'owner'. (no) |
| `--description` | Sets the token's description. |
| `-D --duration` | Sets the token lifetime in days. Accepts: days (30d), weeks (4w), or hours in multiples of 24 (24h, 168h, 720h). Maximum: 365d. The token expires at midnight UTC on the calculated date. (30d) |
| `-E --expires-at` | Sets the token's expiration date and time, in YYYY-MM-DD format. If not specified, --duration is used. (0001-01-01) |
| `-g --group` | Create a group access token. Ignored if a user or repository argument is set. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as 'text' for the token value, 'json' for the actual API token structure. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-S --scope` | Scopes for the token. Multiple scopes can be comma-separated or specified by repeating the flag. For a list, see https://docs.gitlab.com/user/profile/personal_access_tokens/#personal-access-token-scopes. ([read_repository]) |
| `-U --user` | Create a personal access token. For the current user, use @me. |

###### `glab token list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --active` | List only the active tokens. |
| `-g --group` | List group access tokens. Ignored if a user or repository argument is set. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. text provides a readable table, json outputs the tokens with metadata. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-U --user` | List personal access tokens. Use @me for the current user. |

###### `glab token revoke`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --group` | Revoke group access token. Ignored if a user or repository argument is set. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. 'text' provides the name and ID of the revoked token; 'json' outputs the token with metadata. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-U --user` | Revoke personal access token. Use @me for the current user. |

###### `glab token rotate`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-D --duration` | Sets the token lifetime in days. Accepts: days (30d), weeks (4w), or hours in multiples of 24 (24h, 168h, 720h). Maximum: 365d. The token expires at 00:00 UTC on the calculated date. (30d) |
| `-E --expires-at` | Sets the token's expiration date and time, in YYYY-MM-DD format. If not specified, --duration is used. (0001-01-01) |
| `-g --group` | Rotate group access token. Ignored if a user or repository argument is set. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. 'text' provides the new token value; 'json' outputs the token with metadata. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-U --user` | Rotate personal access token. Use @me for the current user. |

##### `glab user`

**Subcommands:**
- `events` — View user events.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab user events`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --all` | Get events from all projects. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: 'text', 'json'. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (30) |

##### `glab variable`

**Subcommands:**
- `delete` — Delete a variable for a project or group.
- `export` — Export variables from a project or group.
- `get` — Get a variable for a project or group.
- `import` — Import variables from a JSON file or standard input.
- `list` — List variables for a project or group.
- `set` — Create a new variable for a project or group.
- `update` — Update an existing variable for a project or group.

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab variable delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --group` | Delete variable from a group. |
| `-h --help` | Show help for this command. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-s --scope` | The 'environment_scope' of the variable. Options: all (*), or specific environments. (*) |

###### `glab variable export`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --group` | Select a group or subgroup. Ignored if a repository argument is set. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: json, export, env. (json) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (100) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-s --scope` | The environment_scope of the variables. Values: '*' (default), or specific environments. (*) |

###### `glab variable get`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --group` | Get variable for a group. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-s --scope` | The environment_scope of the variable. Values: all (*), or specific environments. (*) |

###### `glab variable import`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --group` | Select a group or subgroup. Ignored if a repository argument is set. |
| `-h --help` | Show help for this command. |
| `-i --input-file` | Read the variables JSON from this file instead of standard input. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--skip-existing` | Skip variables that already exist instead of failing. |

###### `glab variable list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --group` | Select a group or subgroup. Ignored if a repository argument is set. |
| `-h --help` | Show help for this command. |
| `-i --instance` | Display instance variables. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-p --page` | Page number. (1) |
| `-P --per-page` | Number of items to list per page. (20) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab variable set`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-d --description` | Set description of a variable. |
| `-g --group` | Set variable for a group. |
| `-h --help` | Show help for this command. |
| `--hidden` | Whether the variable is hidden. |
| `-m --masked` | Whether the variable is masked. |
| `-p --protected` | Whether the variable is protected. |
| `-r --raw` | Whether the variable is treated as a raw string. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-s --scope` | The environment_scope of the variable. Values: all (*), or specific environments. (*) |
| `-t --type` | The type of a variable: env_var, file. (env_var) |
| `-v --value` | The value of a variable. |

###### `glab variable update`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-d --description` | Set description of a variable. |
| `-g --group` | Set variable for a group. |
| `-h --help` | Show help for this command. |
| `-m --masked` | Whether the variable is masked. |
| `-p --protected` | Whether the variable is protected. |
| `-r --raw` | Whether the variable is treated as a raw string. |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-s --scope` | The environment_scope of the variable. Values: all (*), or specific environments. (*) |
| `-t --type` | The type of a variable: env_var, file. (env_var) |
| `-v --value` | The value of a variable. |

##### `glab whatsnew`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |
| `--latest` | Show release notes for the latest published release only. |
| `--since` | Show release notes for every release newer than this version. |

##### `glab work-items`

**Subcommands:**
- `create` — Create work items in a project or group. (EXPERIMENTAL)
- `delete` — Delete a work item in a project or group. (EXPERIMENTAL)
- `list` — List work items in a project or group. (EXPERIMENTAL)
- `update` — Update work items in a project or group. (EXPERIMENTAL)

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h --help` | Show help for this command. |

###### `glab work-items create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c --confidential` | Mark work item confidential. |
| `-d --description` | Description of the work item. Set to "-" to open an editor. |
| `-g --group` | Create work items for a group or subgroup. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `-t --title` | Add a title for the work item. |
| `-T --type` | Type of work item (epic, incident, issue, key_result, objective, requirement, task, test_case, ticket). |

###### `glab work-items delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-g --group` | Delete a work items from a group or subgroup. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |

###### `glab work-items list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--after` | Fetch items after this cursor (for pagination) |
| `-g --group` | List work items for a group or subgroup. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-F --output` | Format output as: text, json. (text) |
| `-P --per-page` | Number of items to list per page (max 100) (20) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--state` | Filter by state: opened, closed, all. (opened) |
| `-t --type` | Filter by work item type (epic, issue, task, etc.) Multiple types can be comma-separated or specified by repeating the flag. |

###### `glab work-items update`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a --assignee` | Update the work item assignee with the supplied GitLab usernames. |
| `-d --description` | Update the description for the work item. |
| `--duedate` | Update the due date for the work item. |
| `-g --group` | Update work items for a group or subgroup. |
| `-h --help` | Show help for this command. |
| `--jq` | Filter JSON output with a jq expression. |
| `-m --milestone` | Update the work item milestone with the title or ID. |
| `-F --output` | Format output as: text, json. (text) |
| `-R --repo` | Select another repository. You can use either OWNER/REPO or GROUP/NAMESPACE/REPO. The full URL or Git URL is also accepted. |
| `--startdate` | Update the start date for the work item. |
| `-t --title` | Update the title for the work item. |
| `-w --weight` | Update the weight value for the work item. |


#### GitHub CLI (`gh`) Reference

#### `gh`

**Subcommands:**
- `auth` — Authenticate gh and git with GitHub
- `browse` — Open repositories, issues, pull requests, and more in the browser
- `codespace` — Connect to and manage codespaces
- `discussion` — Work with GitHub Discussions (preview)
- `gist` — Manage gists
- `issue` — Manage issues
- `org` — Manage organizations
- `pr` — Manage pull requests
- `project` — Work with GitHub Projects.
- `release` — Manage releases
- `repo` — Manage repositories
- `skill` — Install and manage agent skills (preview)
- `cache` — Manage GitHub Actions caches
- `run` — View details about workflow runs
- `workflow` — View details about GitHub Actions workflows
- `agent-task` — Work with agent tasks (preview)
- `api` — Make an authenticated GitHub API request
- `attestation` — Work with artifact attestations
- `config` — Manage configuration for gh
- `copilot` — Run the GitHub Copilot CLI (preview)
- `extension` — Manage gh extensions
- `gpg-key` — Manage GPG keys
- `label` — Manage labels
- `licenses` — View third-party license information
- `preview` — Execute previews for gh features
- `ruleset` — View info about repo rulesets
- `search` — Search for repositories, issues, and pull requests
- `secret` — Manage GitHub secrets
- `ssh-key` — Manage SSH keys
- `status` — Print information about relevant issues, pull requests, and notifications across repositories
- `variable` — Manage GitHub Actions variables

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |
| `--version` | Show gh version |

##### `gh ALIAS`

##### `gh agent-task`

**Subcommands:**
- `create` — Create an agent task (preview)
- `list` — List agent tasks (preview)
- `view` — View an agent task session (preview)

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh agent-task create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b, --base string` | Base branch for the pull request (use default branch if not provided) |
| `-a, --custom-agent string` | Use a custom agent for the task. e.g., use 'my-agent' for the 'my-agent.md' agent |
| `--follow` | Follow agent session logs |
| `-F, --from-file file` | Read task description from file (use "-" to read from standard input) |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |
| `--help` | Show help for command |

###### `gh agent-task list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `-L, --limit int` | Maximum number of agent tasks to fetch (default 30) |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `-w, --web` | Open agent tasks in the browser |
| `--help` | Show help for command |

###### `gh agent-task view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--follow` | Follow agent session logs |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `--log` | Show agent session logs |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `-w, --web` | Open agent task in the browser |
| `--help` | Show help for command |

##### `gh alias`

**Subcommands:**
- `delete` — Delete set aliases
- `import` — Import aliases from a YAML file
- `list` — List your aliases
- `set` — Create a shortcut for a gh command

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh alias delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--all` | Delete all aliases |
| `--help` | Show help for command |

###### `gh alias import`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--clobber` | Overwrite existing aliases of the same name |
| `--help` | Show help for command |

###### `gh alias list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh alias set`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--clobber` | Overwrite existing aliases of the same name |
| `-s, --shell` | Declare an alias to be passed through a shell interpreter |
| `--help` | Show help for command |

##### `gh api`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--allow-escape-sequences` | Allow printing terminal escape sequences |
| `--cache duration` | Cache the response, e.g. "3600s", "60m", "1h" |
| `-F, --field key=value` | Add a typed parameter in key=value format (use "@<path>" or "@-" to read value from file or stdin) |
| `-H, --header key:value` | Add a HTTP request header in key:value format |
| `--hostname string` | The GitHub hostname for the request (default "github.com") |
| `-i, --include` | Include HTTP response status line and headers in the output |
| `--input file` | The file to use as body for the HTTP request (use "-" to read from standard input) |
| `-q, --jq string` | Query to select values from the response using jq syntax |
| `-X, --method string` | The HTTP method for the request (default "GET") |
| `--paginate` | Make additional HTTP requests to fetch all pages of results |
| `-p, --preview strings` | Opt into GitHub API previews (names should omit '-preview') |
| `-f, --raw-field key=value` | Add a string parameter in key=value format |
| `--silent` | Do not print the response body |
| `--slurp` | Use with "--paginate" to return an array of all pages of either JSON arrays or objects |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--verbose` | Include full HTTP request and response in the output |
| `--help` | Show help for command |

##### `gh attestation`

**Subcommands:**
- `download` — Download an artifact's attestations for offline use
- `trusted-root` — Output trusted_root.jsonl contents, likely for offline verification
- `verify` — Verify an artifact's integrity using attestations

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh attestation download`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-d, --digest-alg string` | The algorithm used to compute a digest of the artifact: {sha256\|sha512} (default "sha256") |
| `--hostname string` | Configure host to use |
| `-L, --limit int` | Maximum number of attestations to fetch (default 30) |
| `-o, --owner string` | GitHub organization to scope attestation lookup by |
| `--predicate-type string` | Filter attestations by provided predicate type |
| `-R, --repo string` | Repository name in the format <owner>/<repo> |
| `--help` | Show help for command |

###### `gh attestation trusted-root`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--hostname string` | Configure host to use |
| `--tuf-root string` | Path to the TUF root.json file on disk |
| `--tuf-url string` | URL to the TUF repository mirror |
| `--verify-only` | Don't output trusted_root.jsonl contents |
| `--help` | Show help for command |

###### `gh attestation verify`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b, --bundle string` | Path to bundle on disk, either a single bundle in a JSON file or a JSON lines file with multiple bundles |
| `--bundle-from-oci` | When verifying an OCI image, fetch the attestation bundle from the OCI registry instead of from GitHub |
| `--cert-identity string` | Enforce that the certificate's SubjectAlternativeName matches the provided value exactly |
| `-i, --cert-identity-regex string` | Enforce that the certificate's SubjectAlternativeName matches the provided regex |
| `--cert-oidc-issuer string` | Enforce that the issuer of the OIDC token matches the provided value (default "https://token.actions.githubusercontent.com") |
| `--custom-trusted-root string` | Path to a trusted_root.jsonl file; likely for offline verification |
| `--deny-self-hosted-runners` | Fail verification for attestations generated on self-hosted runners |
| `-d, --digest-alg string` | The algorithm used to compute a digest of the artifact: {sha256\|sha512} (default "sha256") |
| `--format string` | Output format: {json} |
| `--hostname string` | Configure host to use |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `-L, --limit int` | Maximum number of attestations to fetch (default 30) |
| `--no-public-good` | Do not verify attestations signed with Sigstore public good instance |
| `-o, --owner string` | GitHub organization to scope attestation lookup by |
| `--predicate-type string` | Enforce that verified attestations' predicate type matches the provided value (default "https://slsa.dev/provenance/v1") |
| `-R, --repo string` | Repository name in the format <owner>/<repo> |
| `--signer-digest string` | Enforce that the digest associated with the signer workflow matches the provided value |
| `--signer-repo string` | Enforce that the workflow that signed the attestation's repository matches the provided value (<owner>/<repo>) |
| `--signer-workflow string` | Enforce that the workflow that signed the attestation matches the provided value ([host/]<owner>/<repo>/<path>/<to>/<workflow>) |
| `--source-digest string` | Enforce that the digest associated with the source repository matches the provided value |
| `--source-ref string` | Enforce that the git ref associated with the source repository matches the provided value |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |

##### `gh auth`

**Subcommands:**
- `login` — Log in to a GitHub account
- `logout` — Log out of a GitHub account
- `refresh` — Refresh stored authentication credentials
- `setup-git` — Setup git with GitHub CLI
- `status` — Display active account and authentication state on each known GitHub host
- `switch` — Switch active GitHub account
- `token` — Print the authentication token gh uses for a hostname and account

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh auth login`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --clipboard` | Copy one-time OAuth device code to clipboard |
| `-p, --git-protocol string` | The protocol to use for git operations on this host: {ssh\|https} |
| `-h, --hostname string` | The hostname of the GitHub instance to authenticate with |
| `--insecure-storage` | Save authentication credentials in plain text instead of credential store |
| `-s, --scopes strings` | Additional authentication scopes to request |
| `--skip-ssh-key` | Skip generate/upload SSH key prompt |
| `-w, --web` | Open a browser to authenticate |
| `--with-token` | Read token from standard input |
| `--help` | Show help for command |

###### `gh auth logout`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h, --hostname string` | The hostname of the GitHub instance to log out of |
| `-u, --user string` | The account to log out of |
| `--help` | Show help for command |

###### `gh auth refresh`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --clipboard` | Copy one-time OAuth device code to clipboard |
| `-h, --hostname string` | The GitHub host to use for authentication |
| `--insecure-storage` | Save authentication credentials in plain text instead of credential store |
| `-r, --remove-scopes strings` | Authentication scopes to remove from gh |
| `--reset-scopes` | Reset authentication scopes to the default minimum set of scopes |
| `-s, --scopes strings` | Additional authentication scopes for gh to have |
| `--help` | Show help for command |

###### `gh auth setup-git`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-f, --force --hostname` | Force setup even if the host is not known. Must be used in conjunction with --hostname |
| `-h, --hostname string` | The hostname to configure git for |
| `--help` | Show help for command |

###### `gh auth status`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a, --active` | Display the active account only |
| `-h, --hostname string` | Check only a specific hostname's auth status |
| `--jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `-t, --show-token` | Display the auth token |
| `--template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |

###### `gh auth switch`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h, --hostname string` | The hostname of the GitHub instance to switch account for |
| `-u, --user string` | The account to switch to |
| `--help` | Show help for command |

###### `gh auth token`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h, --hostname string` | The hostname of the GitHub instance authenticated with |
| `-u, --user string` | The account to output the token for |
| `--help` | Show help for command |

##### `gh browse`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a, --actions` | Open repository actions |
| `--blame` | Open blame view for a file |
| `-b, --branch string` | Select another branch by passing in the branch name |
| `-c, --commit string[="last"]` | Select another commit by passing in the commit SHA, default is the last commit |
| `-n, --no-browser` | Print destination URL instead of opening the browser |
| `-p, --projects` | Open repository projects |
| `-r, --releases` | Open repository releases |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |
| `-s, --settings` | Open repository settings |
| `-w, --wiki` | Open repository wiki |
| `--help` | Show help for command |

##### `gh cache`

**Subcommands:**
- `delete` — Delete GitHub Actions caches
- `list` — List GitHub Actions caches

**Flags / Options:**

| Flag | Description |
|---|---|
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |
| `--help` | Show help for command |

###### `gh cache delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a, --all` | Delete all caches, can be used with --ref to delete all caches for a specific ref |
| `-r, --ref string` | Delete by cache key and ref, formatted as refs/heads/<branch name> or refs/pull/<number>/merge |
| `--succeed-on-no-caches --all` | Return exit code 0 if no caches found. Must be used in conjunction with --all |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh cache list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `-k, --key string` | Filter by cache key prefix |
| `-L, --limit int` | Maximum number of caches to fetch (default 30) |
| `-O, --order string` | Order of caches returned: {asc\|desc} (default "desc") |
| `-r, --ref string` | Filter by ref, formatted as refs/heads/<branch name> or refs/pull/<number>/merge |
| `-S, --sort string` | Sort fetched caches: {created_at\|last_accessed_at\|size_in_bytes} (default "last_accessed_at") |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

##### `gh codespace`

**Subcommands:**
- `code` — Open a codespace in Visual Studio Code
- `cp` — Copy files between local and remote file systems
- `create` — Create a codespace
- `delete` — Delete codespaces
- `edit` — Edit a codespace
- `jupyter` — Open a codespace in JupyterLab
- `list` — List codespaces
- `logs` — Access codespace logs
- `ports` — List ports in a codespace
- `rebuild` — Rebuild a codespace
- `ssh` — SSH into a codespace
- `stop` — Stop a running codespace
- `view` — View details about a codespace

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh codespace code`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --codespace string` | Name of the codespace |
| `--insiders` | Use the insiders version of Visual Studio Code |
| `-R, --repo string` | Filter codespace selection by repository name (user/repo) |
| `--repo-owner string` | Filter codespace selection by repository owner (username or org) |
| `-w, --web` | Use the web version of Visual Studio Code |
| `--help` | Show help for command |

###### `gh codespace cp`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --codespace string` | Name of the codespace |
| `-e, --expand` | Expand remote file names on remote shell |
| `-p, --profile string` | Name of the SSH profile to use |
| `-r, --recursive` | Recursively copy directories |
| `-R, --repo string` | Filter codespace selection by repository name (user/repo) |
| `--repo-owner string` | Filter codespace selection by repository owner (username or org) |
| `--help` | Show help for command |

###### `gh codespace create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b, --branch string` | Repository branch |
| `--default-permissions` | Do not prompt to accept additional permissions requested by the codespace |
| `--devcontainer-path string` | Path to the devcontainer.json file to use when creating codespace |
| `-d, --display-name string` | Display name for the codespace (48 characters or less) |
| `--idle-timeout duration` | Allowed inactivity before codespace is stopped, e.g. "10m", "1h" |
| `-l, --location string` | Location: {EastUs\|SouthEastAsia\|WestEurope\|WestUs2} (determined automatically if not provided) |
| `-m, --machine string` | Hardware specifications for the VM |
| `-R, --repo string` | Repository name with owner: user/repo |
| `--retention-period duration` | Allowed time after shutting down before the codespace is automatically deleted (maximum 30 days), e.g. "1h", "72h" |
| `-s, --status` | Show status of post-create command and dotfiles |
| `-w, --web` | Create codespace from browser, cannot be used with --display-name, --idle-timeout, or --retention-period |
| `--help` | Show help for command |

###### `gh codespace delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--all` | Delete all codespaces |
| `-c, --codespace string` | Name of the codespace |
| `--days N` | Delete codespaces older than N days |
| `-f, --force` | Skip confirmation for codespaces that contain unsaved changes |
| `-o, --org login` | The login handle of the organization (admin-only) |
| `-R, --repo string` | Filter codespace selection by repository name (user/repo) |
| `--repo-owner string` | Filter codespace selection by repository owner (username or org) |
| `-u, --user username` | The username to delete codespaces for (used with --org) |
| `--help` | Show help for command |

###### `gh codespace edit`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --codespace string` | Name of the codespace |
| `-d, --display-name string` | Set the display name |
| `-m, --machine string` | Set hardware specifications for the VM |
| `-R, --repo string` | Filter codespace selection by repository name (user/repo) |
| `--repo-owner string` | Filter codespace selection by repository owner (username or org) |
| `--help` | Show help for command |

###### `gh codespace jupyter`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --codespace string` | Name of the codespace |
| `-R, --repo string` | Filter codespace selection by repository name (user/repo) |
| `--repo-owner string` | Filter codespace selection by repository owner (username or org) |
| `--help` | Show help for command |

###### `gh codespace list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `-L, --limit int` | Maximum number of codespaces to list (default 30) |
| `-o, --org login` | The login handle of the organization to list codespaces for (admin-only) |
| `-R, --repo string` | Repository name with owner: user/repo |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `-u, --user username` | The username to list codespaces for (used with --org) |
| `-w, --web` | List codespaces in the web browser, cannot be used with --user or --org |
| `--help` | Show help for command |

###### `gh codespace logs`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --codespace string` | Name of the codespace |
| `-f, --follow` | Tail and follow the logs |
| `-R, --repo string` | Filter codespace selection by repository name (user/repo) |
| `--repo-owner string` | Filter codespace selection by repository owner (username or org) |
| `--help` | Show help for command |

###### `gh codespace ports`

**Subcommands:**
- `forward` — Forward ports
- `visibility` — Change the visibility of the forwarded port

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --codespace string` | Name of the codespace |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `-R, --repo string` | Filter codespace selection by repository name (user/repo) |
| `--repo-owner string` | Filter codespace selection by repository owner (username or org) |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |

###### `gh codespace ports forward`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--all-interfaces` | Listen on all network interfaces |
| `-c, --codespace string` | Name of the codespace |
| `--help` | Show help for command |
| `-R, --repo string` | Filter codespace selection by repository name (user/repo) |
| `--repo-owner string` | Filter codespace selection by repository owner (username or org) |

###### `gh codespace ports visibility`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --codespace string` | Name of the codespace |
| `--help` | Show help for command |
| `-R, --repo string` | Filter codespace selection by repository name (user/repo) |
| `--repo-owner string` | Filter codespace selection by repository owner (username or org) |

###### `gh codespace rebuild`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --codespace string` | Name of the codespace |
| `--full` | Perform a full rebuild |
| `-R, --repo string` | Filter codespace selection by repository name (user/repo) |
| `--repo-owner string` | Filter codespace selection by repository owner (username or org) |
| `--help` | Show help for command |

###### `gh codespace ssh`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --codespace string` | Name of the codespace |
| `--config` | Write OpenSSH configuration to stdout |
| `-d, --debug` | Log debug data to a file |
| `--debug-file string` | Path of the file log to |
| `--profile string` | Name of the SSH profile to use |
| `-R, --repo string` | Filter codespace selection by repository name (user/repo) |
| `--repo-owner string` | Filter codespace selection by repository owner (username or org) |
| `--server-port int` | SSH server port number (0 => pick unused) |
| `--help` | Show help for command |

###### `gh codespace stop`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --codespace string` | Name of the codespace |
| `-o, --org login` | The login handle of the organization (admin-only) |
| `-R, --repo string` | Filter codespace selection by repository name (user/repo) |
| `--repo-owner string` | Filter codespace selection by repository owner (username or org) |
| `-u, --user username` | The username to stop codespace for (used with --org) |
| `--help` | Show help for command |

###### `gh codespace view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --codespace string` | Name of the codespace |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `-R, --repo string` | Filter codespace selection by repository name (user/repo) |
| `--repo-owner string` | Filter codespace selection by repository owner (username or org) |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |

##### `gh config`

**Subcommands:**
- `clear-cache` — Clear the cli cache
- `get` — Print the value of a given configuration key
- `list` — Print a list of configuration keys and values
- `set` — Update configuration with a value for the given key

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh config clear-cache`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh config get`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h, --host string` | Get per-host setting |
| `--help` | Show help for command |

###### `gh config list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h, --host string` | Get per-host configuration |
| `--help` | Show help for command |

###### `gh config set`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-h, --host string` | Set per-host setting |
| `--help` | Show help for command |

##### `gh copilot`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--remove` | Remove the downloaded Copilot CLI |
| `--help` | Show help for command |

##### `gh discussion`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |
| `--help` | Show help for command |

##### `gh extension`

**Subcommands:**
- `browse` — Enter a UI for browsing, adding, and removing extensions
- `create` — Create a new extension
- `exec` — Execute an installed extension
- `install` — Install a gh extension from a repository
- `list` — List installed extension commands
- `remove` — Remove an installed extension
- `search` — Search extensions to the GitHub CLI
- `upgrade` — Upgrade installed extensions

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh extension browse`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--debug` | Log to /tmp/extBrowse-* |
| `-s, --single-column` | Render TUI with only one column of text |
| `--help` | Show help for command |

###### `gh extension create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--precompiled string` | Create a precompiled extension. Possible values: go, other |
| `--help` | Show help for command |

###### `gh extension exec`

###### `gh extension install`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--force` | Force upgrade extension, or ignore if latest already installed |
| `--pin string` | Pin extension to a release tag or commit ref |
| `--help` | Show help for command |

###### `gh extension list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh extension remove`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh extension search`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `--license strings` | Filter based on license type |
| `-L, --limit int` | Maximum number of extensions to fetch (default 30) |
| `--order string` | Order of repositories returned, ignored unless '--sort' flag is specified: {asc\|desc} (default "desc") |
| `--owner strings` | Filter on owner |
| `--sort string` | Sort fetched repositories: {forks\|help-wanted-issues\|stars\|updated} (default "best-match") |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `-w, --web` | Open the search query in the web browser |
| `--help` | Show help for command |

###### `gh extension upgrade`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--all` | Upgrade all extensions |
| `--dry-run` | Only display upgrades |
| `--force` | Force upgrade extension |
| `--help` | Show help for command |

##### `gh gist`

**Subcommands:**
- `clone` — Clone a gist locally
- `create` — Create a new gist
- `delete` — Delete a gist
- `edit` — Edit one of your gists
- `list` — List your gists
- `rename` — Rename a file in a gist
- `view` — View a gist

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh gist clone`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh gist create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-d, --desc string` | A description for this gist |
| `-f, --filename string` | Provide a filename to be used when reading from standard input |
| `-p, --public` | List the gist publicly (default "secret") |
| `-w, --web` | Open the web browser with created gist |
| `--help` | Show help for command |

###### `gh gist delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--yes` | Confirm deletion without prompting |
| `--help` | Show help for command |

###### `gh gist edit`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a, --add string` | Add a new file to the gist |
| `-d, --desc string` | New description for the gist |
| `-f, --filename string` | Select a file to edit |
| `-r, --remove string` | Remove a file from the gist |
| `--help` | Show help for command |

###### `gh gist list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--filter expression` | Filter gists using a regular expression |
| `--include-content` | Include gists' file content when filtering |
| `-L, --limit int` | Maximum number of gists to fetch (default 10) |
| `--public` | Show only public gists |
| `--secret` | Show only secret gists |
| `--help` | Show help for command |

###### `gh gist rename`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh gist view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--allow-escape-sequences` | Allow printing terminal escape sequences |
| `-f, --filename string` | Display a single file from the gist |
| `--files` | List file names from the gist |
| `-r, --raw` | Print raw instead of rendered gist contents |
| `-w, --web` | Open gist in the browser |
| `--help` | Show help for command |

##### `gh gpg-key`

**Subcommands:**
- `add` — Add a GPG key to your GitHub account
- `delete` — Delete a GPG key from your GitHub account
- `list` — Lists GPG keys in your GitHub account

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh gpg-key add`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-t, --title string` | Title for the new key |
| `--help` | Show help for command |

###### `gh gpg-key delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-y, --yes` | Skip the confirmation prompt |
| `--help` | Show help for command |

###### `gh gpg-key list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

##### `gh issue`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |
| `--help` | Show help for command |

##### `gh label`

**Subcommands:**
- `clone` — Clones labels from one repository to another
- `create` — Create a new label
- `delete` — Delete a label from a repository
- `edit` — Edit a label
- `list` — List labels in a repository

**Flags / Options:**

| Flag | Description |
|---|---|
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |
| `--help` | Show help for command |

###### `gh label clone`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-f, --force` | Overwrite labels in the destination repository |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh label create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --color string` | Color of the label |
| `-d, --description string` | Description of the label |
| `-f, --force` | Update the label color and description if label already exists |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh label delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--yes` | Confirm deletion without prompting |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh label edit`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-c, --color string` | Color of the label |
| `-d, --description string` | Description of the label |
| `-n, --name string` | New name of the label |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh label list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `-L, --limit int` | Maximum number of labels to fetch (default 30) |
| `--order string` | Order of labels returned: {asc\|desc} (default "asc") |
| `-S, --search string` | Search label names and descriptions |
| `--sort string` | Sort fetched labels: {created\|name} (default "created") |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `-w, --web` | List labels in the web browser |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

##### `gh licenses`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

##### `gh org`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

##### `gh pr`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |
| `--help` | Show help for command |

##### `gh preview`

**Subcommands:**
- `prompter` — Execute a test program to preview the prompter

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh preview prompter`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

##### `gh project`

**Subcommands:**
- `close` — Close a project
- `copy` — Copy a project
- `create` — Create a project
- `delete` — Delete a project
- `edit` — Edit a project
- `field-create` — Create a field in a project
- `field-delete` — Delete a field in a project
- `field-list` — List the fields in a project
- `item-add` — Add a pull request or an issue to a project
- `item-archive` — Archive an item in a project
- `item-create` — Create a draft issue item in a project
- `item-delete` — Delete an item from a project by ID
- `item-edit` — Edit an item in a project
- `item-list` — List the items in a project
- `link` — Link a project to a repository or a team
- `list` — List the projects for an owner
- `mark-template` — Mark a project as a template
- `unlink` — Unlink a project from a repository or a team
- `view` — View a project

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh project close`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--format string` | Output format: {json} |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--owner string` | Login of the owner. Use "@me" for the current user. |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--undo` | Reopen a closed project |
| `--help` | Show help for command |

###### `gh project copy`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--drafts` | Include draft issues when copying |
| `--format string` | Output format: {json} |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--source-owner string` | Login of the source owner. Use "@me" for the current user. |
| `--target-owner string` | Login of the target owner. Use "@me" for the current user. |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--title string` | Title for the new project |
| `--help` | Show help for command |

###### `gh project create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--format string` | Output format: {json} |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--owner string` | Login of the owner. Use "@me" for the current user. |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--title string` | Title for the project |
| `--help` | Show help for command |

###### `gh project delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--format string` | Output format: {json} |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--owner string` | Login of the owner. Use "@me" for the current user. |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |

###### `gh project edit`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-d, --description string` | New description of the project |
| `--format string` | Output format: {json} |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--owner string` | Login of the owner. Use "@me" for the current user. |
| `--readme string` | New readme for the project |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--title string` | New title for the project |
| `--visibility string` | Change project visibility: {PUBLIC\|PRIVATE} |
| `--help` | Show help for command |

###### `gh project field-create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--data-type string` | DataType of the new field.: {TEXT\|SINGLE_SELECT\|DATE\|NUMBER} |
| `--format string` | Output format: {json} |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--name string` | Name of the new field |
| `--owner string` | Login of the owner. Use "@me" for the current user. |
| `--single-select-options strings` | Options for SINGLE_SELECT data type |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |

###### `gh project field-delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--format string` | Output format: {json} |
| `--id string` | ID of the field to delete |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |

###### `gh project field-list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--format string` | Output format: {json} |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `-L, --limit int` | Maximum number of fields to fetch (default 30) |
| `--owner string` | Login of the owner. Use "@me" for the current user. |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |

###### `gh project item-add`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--format string` | Output format: {json} |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--owner string` | Login of the owner. Use "@me" for the current user. |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--url string` | URL of the issue or pull request to add to the project |
| `--help` | Show help for command |

###### `gh project item-archive`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--format string` | Output format: {json} |
| `--id string` | ID of the item to archive |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--owner string` | Login of the owner. Use "@me" for the current user. |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--undo` | Unarchive an item |
| `--help` | Show help for command |

###### `gh project item-create`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--body string` | Body for the draft issue |
| `--format string` | Output format: {json} |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--owner string` | Login of the owner. Use "@me" for the current user. |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--title string` | Title for the draft issue |
| `--help` | Show help for command |

###### `gh project item-delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--format string` | Output format: {json} |
| `--id string` | ID of the item to delete |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--owner string` | Login of the owner. Use "@me" for the current user. |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |

###### `gh project item-edit`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--body string` | Body of the draft issue item |
| `--clear` | Remove field value |
| `--date string` | Date value for the field (YYYY-MM-DD) |
| `--field string` | Name of the field to update |
| `--field-id string` | ID of the field to update |
| `--format string` | Output format: {json} |
| `--id string` | ID of the item to edit |
| `--iteration-id string` | ID of the iteration value to set on the field |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--number float` | Number value for the field |
| `--owner string` | Login of the owner. Use "@me" for the current user. |
| `--project-id string` | ID of the project to which the field belongs to |
| `--single-select-option-id string` | ID of the single select option value to set on the field |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--text string` | Text value for the field |
| `--title string` | Title of the draft issue item |
| `--url string` | URL of the issue or pull request whose project item to edit |
| `--value --field` | Value to set on the field named by --field |
| `--help` | Show help for command |

###### `gh project item-list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--field stringArray` | Name of a field to show as an extra column |
| `--field-id stringArray` | ID of a field to show as an extra column |
| `--format string` | Output format: {json} |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `-L, --limit int` | Maximum number of items to fetch (default 30) |
| `--owner string` | Login of the owner. Use "@me" for the current user |
| `--query string` | Filter items using the Projects filter syntax, e.g. "assignee:octocat -status:Done" |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |

###### `gh project link`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--owner string` | Login of the owner. Use "@me" for the current user. |
| `-R, --repo string` | The repository to be linked to this project |
| `-T, --team string` | The team to be linked to this project |
| `--help` | Show help for command |

###### `gh project list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--closed` | Include closed projects |
| `--format string` | Output format: {json} |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `-L, --limit int` | Maximum number of projects to fetch (default 30) |
| `--owner string` | Login of the owner |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `-w, --web` | Open projects list in the browser |
| `--help` | Show help for command |

###### `gh project mark-template`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--format string` | Output format: {json} |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--owner string` | Login of the org owner. |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--undo` | Unmark the project as a template. |
| `--help` | Show help for command |

###### `gh project unlink`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--owner string` | Login of the owner. Use "@me" for the current user. |
| `-R, --repo string` | The repository to be unlinked from this project |
| `-T, --team string` | The team to be unlinked from this project |
| `--help` | Show help for command |

###### `gh project view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--format string` | Output format: {json} |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--owner string` | Login of the owner. Use "@me" for the current user. |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `-w, --web` | Open a project in the browser |
| `--help` | Show help for command |

##### `gh release`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |
| `--help` | Show help for command |

##### `gh repo`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

##### `gh ruleset`

**Subcommands:**
- `check` — View rules that would apply to a given branch
- `list` — List rulesets for a repository or organization
- `view` — View information about a ruleset

**Flags / Options:**

| Flag | Description |
|---|---|
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |
| `--help` | Show help for command |

###### `gh ruleset check`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--default` | Check rules on default branch |
| `-w, --web` | Open the branch rules page in a web browser |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh ruleset list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-L, --limit int` | Maximum number of rulesets to list (default 30) |
| `-o, --org string` | List organization-wide rulesets for the provided organization |
| `-p, --parents` | Whether to include rulesets configured at higher levels that also apply (default true) |
| `-w, --web` | Open the list of rulesets in the web browser |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh ruleset view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-o, --org string` | Organization name if the provided ID is an organization-level ruleset |
| `-p, --parents` | Whether to include rulesets configured at higher levels that also apply (default true) |
| `-w, --web` | Open the ruleset in the browser |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

##### `gh run`

**Subcommands:**
- `cancel` — Cancel a workflow run
- `delete` — Delete a workflow run
- `download` — Download artifacts generated by a workflow run
- `list` — List recent workflow runs
- `rerun` — Rerun a run
- `view` — View a summary of a workflow run
- `watch` — Watch a run until it completes, showing its progress

**Flags / Options:**

| Flag | Description |
|---|---|
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |
| `--help` | Show help for command |

###### `gh run cancel`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--force` | Force cancel a workflow run |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh run delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh run download`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-D, --dir string` | The directory to download artifacts into (default ".") |
| `-n, --name stringArray` | Download artifacts that match any of the given names |
| `-p, --pattern stringArray` | Download artifacts that match a glob pattern |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh run list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a, --all` | Include disabled workflows |
| `-b, --branch string` | Filter runs by branch |
| `-c, --commit SHA` | Filter runs by the SHA of the commit |
| `--created date` | Filter runs by the date it was created |
| `-e, --event event` | Filter runs by which event triggered the run |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `-L, --limit int` | Maximum number of runs to fetch (default 20) |
| `-s, --status string` | Filter runs by status: {queued\|completed\|in_progress\|requested\|waiting\|pending\|action_required\|cancelled\|failure\|neutral\|skipped\|stale\|startup_failure\|success\|timed_out} |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `-u, --user string` | Filter runs by user who triggered the run |
| `-w, --workflow string` | Filter runs by workflow |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh run rerun`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-d, --debug` | Rerun with debug logging |
| `--failed` | Rerun only failed jobs, including dependencies |
| `-j, --job string` | Rerun a specific job ID from a run, including dependencies |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh run view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a, --attempt uint` | The attempt number of the workflow run |
| `--exit-status` | Exit with non-zero status if run failed |
| `-j, --job string` | View a specific job ID from a run |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `--log` | View full log for either a run or specific job |
| `--log-failed` | View the log for any failed steps in a run or specific job |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `-v, --verbose` | Show job steps |
| `-w, --web` | Open run in the browser |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh run watch`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--compact` | Show only relevant/failed steps |
| `--exit-status` | Exit with non-zero status if run fails |
| `-i, --interval int` | Refresh interval in seconds (default 3) |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

##### `gh search`

**Subcommands:**
- `code` — Search within code
- `commits` — Search for commits
- `issues` — Search for issues
- `prs` — Search for pull requests
- `repos` — Search for repositories

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh search code`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--extension string` | Filter on file extension |
| `--filename string` | Filter on filename |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `--language string` | Filter results by language |
| `-L, --limit int` | Maximum number of code results to fetch (default 30) |
| `--match strings` | Restrict search to file contents or file path: {file\|path} |
| `--owner strings` | Filter on owner |
| `-R, --repo OWNER/REPO` | Filter on repository, in OWNER/REPO format |
| `--size string` | Filter on size range, in kilobytes |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `-w, --web` | Open the search query in the web browser |
| `--help` | Show help for command |

###### `gh search commits`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--author string` | Filter by author |
| `--author-date date` | Filter based on authored date |
| `--author-email string` | Filter on author email |
| `--author-name string` | Filter on author name |
| `--committer string` | Filter by committer |
| `--committer-date date` | Filter based on committed date |
| `--committer-email string` | Filter on committer email |
| `--committer-name string` | Filter on committer name |
| `--hash string` | Filter by commit hash |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `-L, --limit int` | Maximum number of commits to fetch (default 30) |
| `--merge` | Filter on merge commits |
| `--order string` | Order of commits returned, ignored unless '--sort' flag is specified: {asc\|desc} (default "desc") |
| `--owner strings` | Filter on repository owner |
| `--parent string` | Filter by parent hash |
| `-R, --repo OWNER/REPO` | Filter on repository, in OWNER/REPO format |
| `--sort string` | Sort fetched commits: {author-date\|committer-date} (default "best-match") |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--tree string` | Filter by tree hash |
| `--visibility strings` | Filter based on repository visibility: {public\|private\|internal} |
| `-w, --web` | Open the search query in the web browser |
| `--help` | Show help for command |

###### `gh search issues`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--app string` | Filter by GitHub App author |
| `--archived` | Filter based on the repository archived state {true\|false} |
| `--assignee string` | Filter by assignee |
| `--author string` | Filter by author (use --app to filter by a GitHub App) |
| `--closed date` | Filter on closed at date |
| `--commenter user` | Filter based on comments by user |
| `--comments number` | Filter on number of comments |
| `--created date` | Filter based on created at date |
| `--include-prs` | Include pull requests in results |
| `--interactions number` | Filter on number of reactions and comments |
| `--involves user` | Filter based on involvement of user |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `--label strings` | Filter on label |
| `--language string` | Filter based on the coding language |
| `-L, --limit int` | Maximum number of results to fetch (default 30) |
| `--locked` | Filter on locked conversation status |
| `--match strings` | Restrict search to specific field of issue: {title\|body\|comments} |
| `--mentions user` | Filter based on user mentions |
| `--milestone title` | Filter by milestone title |
| `--no-assignee` | Filter on missing assignee |
| `--no-label` | Filter on missing label |
| `--no-milestone` | Filter on missing milestone |
| `--no-project` | Filter on missing project |
| `--order string` | Order of results returned, ignored unless '--sort' flag is specified: {asc\|desc} (default "desc") |
| `--owner strings` | Filter on repository owner |
| `--project owner/number` | Filter on project board owner/number |
| `--reactions number` | Filter on number of reactions |
| `-R, --repo OWNER/REPO` | Filter on repository, in OWNER/REPO format |
| `--search-type string` | Type of issue search to perform: {lexical\|semantic\|hybrid} (default "lexical") |
| `--sort string` | Sort fetched results: {comments\|created\|interactions\|reactions\|reactions-+1\|reactions--1\|reactions-heart\|reactions-smile\|reactions-tada\|reactions-thinking_face\|updated} (default "best-match") |
| `--state string` | Filter based on state: {open\|closed} |
| `--team-mentions string` | Filter based on team mentions |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--updated date` | Filter on last updated at date |
| `--visibility strings` | Filter based on repository visibility: {public\|private\|internal} |
| `-w, --web` | Open the search query in the web browser |
| `--help` | Show help for command |

###### `gh search prs`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--app string` | Filter by GitHub App author |
| `--archived` | Filter based on the repository archived state {true\|false} |
| `--assignee string` | Filter by assignee |
| `--author string` | Filter by author (use --app to filter by a GitHub App) |
| `-B, --base string` | Filter on base branch name |
| `--checks string` | Filter based on status of the checks: {pending\|success\|failure} |
| `--closed date` | Filter on closed at date |
| `--commenter user` | Filter based on comments by user |
| `--comments number` | Filter on number of comments |
| `--created date` | Filter based on created at date |
| `--draft` | Filter based on draft state |
| `-H, --head string` | Filter on head branch name |
| `--interactions number` | Filter on number of reactions and comments |
| `--involves user` | Filter based on involvement of user |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `--label strings` | Filter on label |
| `--language string` | Filter based on the coding language |
| `-L, --limit int` | Maximum number of results to fetch (default 30) |
| `--locked` | Filter on locked conversation status |
| `--match strings` | Restrict search to specific field of issue: {title\|body\|comments} |
| `--mentions user` | Filter based on user mentions |
| `--merged` | Filter based on merged state |
| `--merged-at date` | Filter on merged at date |
| `--milestone title` | Filter by milestone title |
| `--no-assignee` | Filter on missing assignee |
| `--no-label` | Filter on missing label |
| `--no-milestone` | Filter on missing milestone |
| `--no-project` | Filter on missing project |
| `--order string` | Order of results returned, ignored unless '--sort' flag is specified: {asc\|desc} (default "desc") |
| `--owner strings` | Filter on repository owner |
| `--project owner/number` | Filter on project board owner/number |
| `--reactions number` | Filter on number of reactions |
| `-R, --repo OWNER/REPO` | Filter on repository, in OWNER/REPO format |
| `--review string` | Filter based on review status: {none\|required\|approved\|changes_requested} |
| `--review-requested user` | Filter on user or team requested to review |
| `--reviewed-by user` | Filter on user who reviewed |
| `--sort string` | Sort fetched results: {comments\|reactions\|reactions-+1\|reactions--1\|reactions-smile\|reactions-thinking_face\|reactions-heart\|reactions-tada\|interactions\|created\|updated} (default "best-match") |
| `--state string` | Filter based on state: {open\|closed} |
| `--team-mentions string` | Filter based on team mentions |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--updated date` | Filter on last updated at date |
| `--visibility strings` | Filter based on repository visibility: {public\|private\|internal} |
| `-w, --web` | Open the search query in the web browser |
| `--help` | Show help for command |

###### `gh search repos`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--archived` | Filter based on the repository archived state {true\|false} |
| `--created date` | Filter based on created at date |
| `--followers number` | Filter based on number of followers |
| `--forks number` | Filter on number of forks |
| `--good-first-issues number` | Filter on number of issues with the 'good first issue' label |
| `--help-wanted-issues number` | Filter on number of issues with the 'help wanted' label |
| `--include-forks string` | Include forks in fetched repositories: {false\|true\|only} |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `--language string` | Filter based on the coding language |
| `--license strings` | Filter based on license type |
| `-L, --limit int` | Maximum number of repositories to fetch (default 30) |
| `--match strings` | Restrict search to specific field of repository: {name\|description\|readme} |
| `--number-topics number` | Filter on number of topics |
| `--order string` | Order of repositories returned, ignored unless '--sort' flag is specified: {asc\|desc} (default "desc") |
| `--owner strings` | Filter on owner |
| `--size string` | Filter on a size range, in kilobytes |
| `--sort string` | Sort fetched repositories: {forks\|help-wanted-issues\|stars\|updated} (default "best-match") |
| `--stars number` | Filter on number of stars |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--topic strings` | Filter on topic |
| `--updated date` | Filter on last updated at date |
| `--visibility strings` | Filter based on visibility: {public\|private\|internal} |
| `-w, --web` | Open the search query in the web browser |
| `--help` | Show help for command |

##### `gh secret`

**Subcommands:**
- `delete` — Delete secrets
- `list` — List secrets
- `set` — Create or update secrets

**Flags / Options:**

| Flag | Description |
|---|---|
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |
| `--help` | Show help for command |

###### `gh secret delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a, --app string` | Delete a secret for a specific application: {actions\|agents\|codespaces\|dependabot} |
| `-e, --env string` | Delete a secret for an environment |
| `-o, --org string` | Delete a secret for an organization |
| `-u, --user` | Delete a secret for your user |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh secret list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a, --app string` | List secrets for a specific application: {actions\|agents\|codespaces\|dependabot} |
| `-e, --env string` | List secrets for an environment |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `-o, --org string` | List secrets for an organization |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `-u, --user` | List a secret for your user |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh secret set`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a, --app string` | Set the application for a secret: {actions\|agents\|codespaces\|dependabot} |
| `-b, --body string` | The value for the secret (reads from standard input if not specified) |
| `-e, --env environment` | Set deployment environment secret |
| `-f, --env-file file` | Load secret names and values from a dotenv-formatted file |
| `--no-repos-selected` | No repositories can access the organization secret |
| `--no-store` | Print the encrypted, base64-encoded value instead of storing it on GitHub |
| `-o, --org organization` | Set organization secret |
| `-r, --repos repositories` | List of repositories that can access an organization or user secret |
| `-u, --user` | Set a secret for your user |
| `-v, --visibility string` | Set visibility for an organization secret: {all\|private\|selected} (default "private") |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

##### `gh skill`

**Subcommands:**
- `install` — Install agent skills from a GitHub repository (preview)
- `list` — List installed skills (preview)
- `preview` — Preview a skill from a GitHub repository (preview)
- `publish` — Validate and publish skills to a GitHub repository (preview)
- `search` — Search for skills across GitHub (preview)
- `update` — Update installed skills to their latest versions (preview)

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh skill install`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--agent string` | Target agent (see supported values above) |
| `--all` | Install all skills without prompting for skill selection |
| `--allow-hidden-dirs` | Include skills in hidden directories (e.g. .claude/skills/, .agents/skills/) |
| `--dir string` | Install to a custom directory (overrides --agent and --scope) |
| `-f, --force` | Overwrite existing skills without prompting |
| `--from-local` | Treat the argument as a local directory path instead of a repository |
| `--pin string` | Pin to a specific git tag or commit SHA |
| `--scope string` | Installation scope: {project\|user} (default "project") |
| `--upstream` | Install from the upstream source when a re-published skill is detected |
| `--help` | Show help for command |

###### `gh skill list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--agent string` | Filter by target agent: {github-copilot\|claude-code\|cursor\|codex\|gemini-cli\|antigravity\|antigravity-cli\|antigravity2.0\|adal\|amp\|augment\|bob\|cline\|codebuddy\|command-code\|continue\|cortex\|crush\|deepagents\|devin\|droid\|firebender\|goose\|grok\|iflow-cli\|junie\|kilo\|kimi-cli\|kiro-cli\|kode\|mcpjam\|mistral-vibe\|mux\|neovate\|openclaw\|opencode\|openhands\|pi\|pochi\|qoder\|qwen-code\|replit\|roo\|trae\|trae-cn\|universal\|warp\|zencoder} |
| `--dir string` | Scan a custom directory for installed skills |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `--scope string` | Filter by installation scope: {project\|user} |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |

###### `gh skill preview`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--allow-hidden-dirs` | Include skills in hidden directories (e.g. .claude/skills/, .agents/skills/) |
| `--help` | Show help for command |

###### `gh skill publish`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--dry-run` | Validate without publishing |
| `--fix` | Auto-fix issues where possible without publishing (e.g. strip install metadata) |
| `--tag string` | Version tag for the release (e.g. v1.0.0) |
| `--help` | Show help for command |

###### `gh skill search`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `-L, --limit int` | Maximum number of results per page (default 15) |
| `--owner string` | Filter results to a specific GitHub user or organization |
| `--page int` | Page number of results to fetch (default 1) |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |

###### `gh skill update`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--all` | Update all skills without prompting |
| `--dir string` | Scan a custom directory for installed skills |
| `--dry-run` | Report available updates without modifying files |
| `--force` | Re-download even if already up to date |
| `--unpin` | Clear pinned version and include pinned skills in update |
| `--help` | Show help for command |

##### `gh ssh-key`

**Subcommands:**
- `add` — Add an SSH key to your GitHub account
- `delete` — Delete an SSH key from your GitHub account
- `list` — Lists SSH keys in your GitHub account

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

###### `gh ssh-key add`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-t, --title string` | Title for the new key |
| `--type string` | Type of the ssh key: {authentication\|signing} (default "authentication") |
| `--help` | Show help for command |

###### `gh ssh-key delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-y, --yes` | Skip the confirmation prompt |
| `--help` | Show help for command |

###### `gh ssh-key list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |

##### `gh status`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-e, --exclude strings` | Comma separated list of repos to exclude in owner/name format |
| `-o, --org string` | Report status within an organization |
| `--help` | Show help for command |

##### `gh variable`

**Subcommands:**
- `delete` — Delete variables
- `get` — Get variables
- `list` — List variables
- `set` — Create or update variables

**Flags / Options:**

| Flag | Description |
|---|---|
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |
| `--help` | Show help for command |

###### `gh variable delete`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-e, --env string` | Delete a variable for an environment |
| `-o, --org string` | Delete a variable for an organization |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh variable get`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-e, --env string` | Get a variable for an environment |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `-o, --org string` | Get a variable for an organization |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh variable list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-e, --env string` | List variables for an environment |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `-o, --org string` | List variables for an organization |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh variable set`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-b, --body string` | The value for the variable (reads from standard input if not specified) |
| `-e, --env environment` | Set deployment environment variable |
| `-f, --env-file file` | Load variable names and values from a dotenv-formatted file |
| `-o, --org organization` | Set organization variable |
| `-r, --repos repositories` | List of repositories that can access an organization variable |
| `-v, --visibility string` | Set visibility for an organization variable: {all\|private\|selected} (default "private") |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

##### `gh workflow`

**Subcommands:**
- `disable` — Disable a workflow
- `enable` — Enable a workflow
- `list` — List workflows
- `run` — Run a workflow by creating a workflow_dispatch event
- `view` — View the summary of a workflow

**Flags / Options:**

| Flag | Description |
|---|---|
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |
| `--help` | Show help for command |

###### `gh workflow disable`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh workflow enable`

**Flags / Options:**

| Flag | Description |
|---|---|
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh workflow list`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-a, --all` | Include disabled workflows |
| `-q, --jq expression` | Filter JSON output using a jq expression |
| `--json fields` | Output JSON with the specified fields |
| `-L, --limit int` | Maximum number of workflows to fetch (default 50) |
| `-t, --template string` | Format JSON output using a Go template; see "gh help formatting" |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh workflow run`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-F, --field key=value` | Add a string parameter in key=value format, respecting @ syntax (see "gh help api"). |
| `--json` | Read workflow inputs as JSON via STDIN |
| `-f, --raw-field key=value` | Add a string parameter in key=value format |
| `-r, --ref string` | Branch or tag name which contains the version of the workflow file you'd like to run |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |

###### `gh workflow view`

**Flags / Options:**

| Flag | Description |
|---|---|
| `-r, --ref string` | The branch or tag name which contains the version of the workflow file you'd like to view |
| `-w, --web` | Open workflow in the browser |
| `-y, --yaml` | View the workflow yaml file |
| `--help` | Show help for command |
| `-R, --repo [HOST/]OWNER/REPO` | Select another repository using the [HOST/]OWNER/REPO format |



## 7. Development & Quality Standards

* **Error Handling:** Use `anyhow::Result`. Bubble up errors and display them in the UI via `App::show_error(msg)`. Do not `unwrap()` or `panic!()` in UI or event handling code.
* **Test env isolation:** Unit tests that mutate process-global environment variables (config paths via `GLAB_TUI_CONFIG`/`XDG_CONFIG_HOME`, cache dirs) must acquire `config::TEST_ENV_MUTEX` first — env vars are visible to every test thread, and overlapping mutations caused an intermittent Windows CI failure. Never introduce a second ad-hoc mutex for env mutation; reuse the crate-wide one.
* **Dependencies:** Do not add large dependencies (like `reqwest` or `hyper`) for HTTP API calls. The architecture strictly dictates delegating HTTP requests to `gh` and `glab` CLI binaries via `tokio::process::Command` in `GitlabClient`.
* **Format & Lint:** Run `cargo fmt` and `cargo clippy -- -D warnings` before providing code. The CI enforces zero clippy warnings.
* **MSRV:** The Minimum Supported Rust Version is `1.85` (as required by edition 2024). Ensure code is compatible.

## 8. Release Process (Local-First)

Releases are prepared, documented, and distributed from a maintainer's machine via a single orchestrator, `scripts/release.sh`. CI is only responsible for building the cross-platform release binaries. The demo GIFs must be recorded locally because `glab-tui` shells out to `gh`/`glab`, and CI tokens lack the permissions for a realistic recording.

Run `scripts/release.sh [patch|minor|major|nightly]` (default `patch`) and the script walks the full release:

> **Nightly Pre-Releases:** `scripts/release.sh nightly` (or tags matching `vX.Y.Z-nightly[-suffix]`) runs a stripped-down flow that skips `Cargo.toml` bumping, docs regeneration, prepare PRs, demo GIFs, review gates, Homebrew/Scoop manifest syncs, and crates.io publishing, while still building matrix binaries and updating GitHub release notes.

1. **Preflight** — checks `gh`/`opencode`/`cargo`/`jq`/`vhs`/`ttyd`/`ffmpeg`/`unzip`, `gh auth`, JetBrainsMono Nerd Font, and push access to both manifest repos (`rcieri/homebrew-glab-tui`, `rcieri/scoop-glab-tui`); exits non-zero with a clear message if a prerequisite is missing. Long-running steps run under the script's `spinner`/`progress_bar` helpers (animated spinner with captured logs, auto-disabled when not a TTY), and phases are numbered `1/7` … for progress reporting.
2. **Prepare** — computes the next tag from `git describe --tags`, bumps the crate version in `Cargo.toml`, prompts for the opencode model (provider → model → variant; see below) unless `OPENCODE_MODEL` is set, regenerates `CHANGELOG.md`/`AGENTS.md`/`README.md` via headless `opencode run`, rebuilds the demo GIFs against an authenticated `gh`, and opens a `chore: prepare release vX.Y.Z` PR.
3. **Review gate** — pauses for the maintainer to review the PR (CI checks run in the background); the script continues on Enter.
4. **Merge & tag** — squash-merges the PR with `--auto`, tags the merge commit and pushes `vX.Y.Z`. `.github/workflows/release.yml` builds the 11-target binary matrix (8 Linux + 3 non-Linux) and uploads them to the GitHub release.
5. **Wait for build** — polls the release until every required asset exists (timeout: `RELEASE_WAIT_MIN`, default 45 min). The required-asset list in `REQUIRED_ASSETS_STATIC` covers the fixed name patterns; the dynamic Ubuntu-latest build is required to produce at least two `ubuntu-<VERSION_ID>` assets whose VERSION_ID differs from the 22.04 / 24.04 baselines.
6. **Post-release** — generates `RELEASE_NOTES.md` via headless `opencode run` (entries attribute their contributors as `(thanks @username)` and a `**Contributors**` section lists all `@username` handles since the previous tag), edits the release body, and pushes the Homebrew formula and Scoop manifest. The manifest repos' scheduled auto-updaters have been removed; this local sync is the only update path.
7. **Publish** — pushes the Docker image to GHCR and publishes the crate to crates.io.

### Release Assets

`.github/workflows/release.yml` builds an 11-entry matrix (8 Linux + 3 non-Linux) on every `v*` tag push. **Linux builds target a glibc 2.35 baseline (Ubuntu 22.04) for backwards compatibility with older distros, plus per-LTS and `ubuntu-latest` variants for current distros, and a fully static musl fallback for non-glibc systems.** The `ubuntu-latest` runner's `VERSION_ID` is detected at build time and baked into the asset name (e.g. `glab-tui-linux-amd64-ubuntu-26.04.tar.gz` when `ubuntu-latest` resolves to Ubuntu 26.04), so the asset set adapts to future LTS releases without code changes.

Asset name pattern:

| OS | Arch | Asset name | Built on |
|---|---|---|---|
| Linux | x86_64 | `glab-tui-linux-amd64-ubuntu-22.04.tar.gz` | `ubuntu-22.04` runner (glibc 2.35 baseline) |
| Linux | x86_64 | `glab-tui-linux-amd64-ubuntu-24.04.tar.gz` | `ubuntu-24.04` runner (glibc 2.39) |
| Linux | x86_64 | `glab-tui-linux-amd64-ubuntu-<VERSION_ID>.tar.gz` | `ubuntu-latest` runner (current LTS) |
| Linux | aarch64 | `glab-tui-linux-arm64-ubuntu-{22.04,24.04,<VERSION_ID>}.tar.gz` | cross-compiled from `ubuntu-{22.04,24.04,latest}` x86_64 runners |
| Linux | x86_64 / aarch64 | `glab-tui-linux-{amd64,arm64}-musl.tar.gz` | `ubuntu-22.04` (static, no glibc dependency) |
| macOS | x86_64 / arm64 | `glab-tui-macos-{amd64,arm64}.tar.gz` | `macos-latest` |
| Windows | x86_64 | `glab-tui-windows-amd64.zip` | `windows-latest` |

The asset-suffix table in `release.yml` is the single source of truth — `linux_variant: ubuntu-latest` causes the build step to read `/etc/os-release` and bake `VERSION_ID` into the asset name.

#### Asset selection logic

Three places pick the right asset for the running host:

- **`install.sh`** (`linux_asset_candidates`) — builds a candidate list from `/etc/os-release`. On Ubuntu it tries the local version first, then walks down through `ubuntu-24.04` → `ubuntu-22.04`, then falls back to `musl`. Non-Ubuntu Linux distros try `ubuntu-22.04` → `ubuntu-24.04` → `musl` directly. Override with `GLAB_TUI_ASSET=glab-tui-linux-amd64-musl.tar.gz` for testing.
- **`src/utils/update.rs`** (`linux_asset_candidates`) — mirrors `install.sh` for the in-app self-updater; queries the GitHub release's asset list and picks the first match.
- **Homebrew formula** (`update_homebrew` in `release.sh`) — at release time, lists every Linux asset via the GitHub API and embeds the full `{variant => sha256}` hash into the formula. At install time, the formula's `linux_variant(sha_map)` picks by `OS::Version.from_symbol(:ubuntu)`, falling back to `ubuntu-24.04` → `ubuntu-22.04` → `musl` and finally to whichever asset is newest. This means the formula survives future LTS releases without being regenerated.

Scoop's manifest (`rcieri/scoop-glab-tui`) is unaffected — Windows keeps the fixed `glab-tui-windows-amd64.zip` name.

During `Prepare`, `release.sh` interactively walks through the opencode models available to the local `opencode` install (`provider -> model -> variant`) to pick the model used for the regenerated docs and release notes; set `OPENCODE_MODEL` to skip the prompt.
