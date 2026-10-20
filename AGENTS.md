# AI Agent Instructions for `glab-tui`

Welcome, AI Agent! This document contains essential context, architectural guidelines, and coding standards for navigating and contributing to `glab-tui`. Please adhere to these rules when analyzing the codebase, writing new features, or refactoring.

## 1. Project Overview

`glab-tui` is a Terminal User Interface (TUI) for managing GitLab and GitHub repositories. 
Instead of implementing full REST/GraphQL API clients, **`glab-tui` shells out to the official `glab` and `gh` CLIs** under the hood.

* **Primary Language:** Rust (Edition 2024)
* **TUI Framework:** `ratatui` (v0.30.1)
* **Syntax Highlighting:** `syntect` (v5, `parsing` + `default-syntaxes` + `regex-fancy` features)
* **Markdown Rendering:** `pulldown-cmark` (v0.13.0, GFM tables/code/lists)
* **Async Runtime:** `tokio` (v1.38, full)
* **Async Traits:** `async-trait` (v0.1.92)
* **CLI Parsing:** `clap` (v4, derive)
* **Terminal Handling:** `crossterm` (v0.29)
* **Clipboard:** `arboard` (v3.6)
* **Config/Themes:** `toml` (v1.1) crate; config at `~/.config/glab-tui/config.toml`
* **YAML:** `serde-saphyr` (v1.1, `deserialize` only) — parses `workflow_dispatch` inputs from GitHub workflow files. It replaced the deprecated `serde_yaml`. serde-saphyr 1.2+ needs Rust 1.89, so the MSRV-aware resolver holds it at 1.1.
* **Process & Session Handling:** `libc` (v0.2) — Unix setsid for background commands & signal masking during terminal handoff
* **Package:** `glab-tui-crate` (binary: `glab-tui`; current version `v0.9.3`)

### Dual-Engine Architecture
The application detects whether the current repository is hosted on GitHub or GitLab and instantiates either a `GlabBackend` or `GhBackend`. Detection is centralized in `git_helpers::detect_backend(remote_url, override_kind)` ([src/git_helpers.rs](src/git_helpers.rs)): `github.com` remotes (with or without `www.` prefix) resolve to GitHub; other hosts are probed with `gh auth status --active --hostname <host>` and `glab auth status --hostname <host>`, defaulting to GitLab when neither/both respond. Without an `origin` remote (e.g. `-r owner/repo` outside a checkout), `git_helpers::detect_backend_without_remote` applies the same rule to `gh auth status --active` / `glab auth status` with no hostname. A repo-local `backend = "github" | "gitlab"` config override always takes precedence — set it for SSH aliases or hosts serving both platforms. Always route backend detection through `detect_backend`; do not reimplement inline `github.com` string matching. Both backends implement the `Backend` trait ([src/backend/mod.rs](src/backend/mod.rs)). The domain layer ([src/domain/](src/domain/)) calls backend methods through `GitlabClient` ([src/domain/client.rs](src/domain/client.rs)). Runtime backend identification is available via the `BackendKind` enum (`BackendKind::GitLab` / `BackendKind::GitHub`) which also provides host-aware terminology through `BackendKind::term()`.

The `namespace/project` context passed as `-R <repo>` to every `glab`/`gh` call is extracted from the remote URL by `git_helpers::parse_project_path` ([src/git_helpers.rs](src/git_helpers.rs)), which keeps every path segment after the host so nested GitLab subgroup namespaces (`group/subgroup/project`) resolve correctly. Always use this helper — do not reimplement remote-URL parsing inline.

Group/org-level browsing is supported via the `Scope` enum ([src/scope.rs](src/scope.rs)) and the `-g`/`--group` CLI flag (`src/cli.rs`). A `Scope::Group` is inferred from the remote URL by `git_helpers::parse_group` (paths without a slash), from the `--group` CLI/`config.group` value, or from `parse_project_path_from_web_url` (web URLs). All listing backend methods (`list_issues`, `list_mrs`, `list_pipelines`, `list_runners`, `list_releases`, `list_milestones`, `list_branches`, `list_environments`, `list_deployments`, `fetch_members`) accept a `&Scope` and use `cli_group_arg()` to inject `-g` (glab) / org flags instead of `-R`. Recent groups are persisted in `recent_groups.json` ([src/utils/cache.rs](src/utils/cache.rs)). In group scope, entity creation and edit forms embed a selectable "Project" field that dynamically fetches project-scoped labels, assignees, and milestones upon selection.

**Group scope on GitLab is partial too.** Issues, MRs, pipelines, milestones, labels and members aggregate across the group, but `GlabBackend::list_runners`, `list_releases`, `list_branches`, `list_environments` and `list_deployments` return `Err("Group-level … not supported")`, so the Runners, Releases, Branches and Environments (including its deployments) tabs only work in repository scope. On GitHub, only a subset of the GH backend methods support `Scope::Group` (i.e. an org scope); the rest return `Err` and the UI shows an error toast. The currently supported GH backend methods with `Scope::Group` are: `list_issues` (via `search/issues?q=owner:org+is:issue`), `list_mrs` (via `search/issues?q=owner:org+is:pr`), `list_pipelines` (enumerates `orgs/{org}/repos` then `repos/{repo}/actions/runs` per repo), and `fetch_members` (via `/orgs/{org}/members`). `list_runners`, `list_releases`, `list_milestones`, `list_branches`, `list_environments`, and `list_deployments` still return `Err("Org-level … not supported on GitHub")`; `list_labels` returns an empty result. Surfacing GitHub orgs in the Switch overlay (`recent_groups.json` + cached-repo remote probe) therefore exposes this partial coverage — selecting an org is safe on Issues/MRs/Pipelines/Members tabs and produces error toasts elsewhere.

**Rule:** Never use `glab api` or `gh api` when a native subcommand exists. Prefer native subcommands — they use built-in pagination, auth, and output formatting. Only fall back to raw API calls for endpoints with no native CLI equivalent.

## 2. Directory Structure

* [src/lib.rs](src/lib.rs): Library crate root (`glab_tui_crate`). Declares every module and the crate-root re-exports (`AppTerminal`, `keybinding_matches`, `spawn_refresh_active_tab`, the `git_helpers`/`templates`/`editor`/`entity_editor` globs). The binary and the fuzz targets both link it, so anything `main.rs` calls must be `pub`. Module unit tests run with `cargo test --lib`.
* [src/main.rs](src/main.rs): Binary entry point (`use glab_tui_crate::*`). Sets up the terminal, initializes the `App`, handles the main `tokio` event loop, routes keypresses (via `keybinding_matches()`), and delegates UI rendering.
* [src/app.rs](src/app.rs): Contains the global `App` state, data models for UI components (`EditMenu`, `SubmitDialog`, `Selector`, `DiffView`, `DatePicker`), and fuzzy-filtering logic.
* [src/config.rs](src/config.rs): Config, theme, and icons system. Defines `Config`, `Theme`, `ThemeOverrides`, `Icons`, and all `KeybindingXxx` structs.
* [src/event.rs](src/event.rs): Defines the `Event` enum and the async `EventHandler` using `tokio::sync::mpsc`.
* [src/backend/](src/backend/): CLI backend layer.
    * [bounded_output.rs](src/backend/bounded_output.rs): `output_bounded`, the subprocess reader behind the command runners. Keeps at most a given number of stdout bytes and kills a child that writes more.
    * [rate_limit.rs](src/backend/rate_limit.rs): `ApiRateLimiter` — automatic 429/graphql rate-limit detection and retry with exponential backoff plus bulk-operation pacing (`pace_bulk_operation`).
    * [subprocess.rs](src/backend/subprocess.rs): `subprocess::output(&mut Command, input)` — the one way to run a `gh`/`glab` call: pipes stdout/stderr, writes `input` to stdin, and kills the child once it outlives `command_timeout_seconds` (`config::command_timeout()`), returning an error that names the command. `output_capped` does the same keeping at most a given number of bytes of each stream and draining the rest, so the child still runs to completion.
    * [logged_command.rs](src/backend/logged_command.rs): `LoggedCommand`, how every runner reports a finished `glab`/`gh`/`git` call to the Terminal tab: `start` before spawning, `finish(tx, status, ChildOutput)` sends `Event::TerminalCommandLogged` with the entry's `Execution` (start time, duration, exit code, stdout/stderr cut to `MAX_CAPTURED_OUTPUT_BYTES`, and the `Invocation`). `failure_reason(&ChildOutput)` is the reason every runner and `rerun` put after `Failed: ` (trimmed stderr, else the command and how it ended, plus GitHub's error details from the body of a `gh api` call). `rerun(invocation)` runs a recorded call again through `subprocess::output_capped` (one byte past `MAX_CAPTURED_OUTPUT_BYTES` per stream) under one `RATE_LIMITER` slot. New runners must log through it so their entries can be inspected and re-run.
    * [mod.rs](src/backend/mod.rs): `Backend` trait with ~40 methods covering all API interactions plus `IssueUpdate`/`MrUpdate` field structs for batched entity edits and `NewIssue`/`NewMr`/`MergeOptions` for creation and merge calls.
    * [glab.rs](src/backend/glab.rs): `GlabBackend` — shells out to `glab` CLI.
    * [gh.rs](src/backend/gh.rs): `GhBackend` — shells out to `gh` CLI.
* [src/domain/](src/domain/): Domain models and top-level API functions.
    * [client.rs](src/domain/client.rs): `GitlabClient` wrapper holding the backend, page_size, api_per_page, and event tx.
    * [issues.rs](src/domain/issues.rs): Issue structures and `list_issues`/`get_issue`.
    * [job_trace.rs](src/domain/job_trace.rs): `JobTrace` / `TraceCursor` / `TraceUpdate` and `JOB_TRACE_MAX_BYTES`: a job log read in bounded pieces, appended to by follow mode. `JobTrace::next_failure_line` finds the next line reporting a failure (`FAILURE_WORDS` in the line's `trace_line_content`, whole words that keep `-`, `.` and `/`, not next to a zero count) for the Jobs tab's `jump_to_failure` key.
    * [labels.rs](src/domain/labels.rs): `Label` structure carrying the API-provided color used for the Labels column.
    * [mr.rs](src/domain/mr.rs): MergeRequest, DiscussionNote, NotePosition structures.
    * [mr_state.rs](src/domain/mr_state.rs): MR review-state helpers — `ApprovalState`, `MergeabilityState`, `WorkflowStatus`, `derive_awaiting_you`, `rebase_gate`, and the cell/sort/filter display helpers for the Approval/Mergeable/Workflow columns.
    * [review_threads.rs](src/domain/review_threads.rs): `ReviewThread` / `ThreadAnchor` and `group_threads()` — groups MR/PR notes into discussion threads and classifies each as general, in-diff, or outdated.
    * [pipelines.rs](src/domain/pipelines.rs): Pipeline, Job and `CommitSummary` structures, job deduplication logic, and `triggering_commit()` (GitHub lists a run's commit with the run, GitLab only with each job, so the inspector reads it from the pipeline's jobs on GitLab).
    * [runners.rs](src/domain/runners.rs): Runner structures.
    * [releases.rs](src/domain/releases.rs): Release structures.
    * [notifications.rs](src/domain/notifications.rs): Notification structures (GitLab todos + GitHub notifications).
    * [milestones.rs](src/domain/milestones.rs): Milestone structures.
    * [branches.rs](src/domain/branches.rs): Branch structures.
    * [deployments.rs](src/domain/deployments.rs): Environment and Deployment structures.
    * [workflow_inputs.rs](src/domain/workflow_inputs.rs): `WorkflowInput` / `WorkflowInputType` for `workflow_dispatch` prompt fields.
* [src/fetch.rs](src/fetch.rs): `spawn_refresh_active_tab()` — dispatches per-tab data fetches (routed through `&Scope`); `spawn_refresh_all_tabs()` — background tab prefetcher when `config.prefetch_tabs` is enabled; `derive_workflow()` — recomputes the derived MR `workflow` column after live fetches and cache loads.
* [src/git_helpers.rs](src/git_helpers.rs): Git helpers — `detect_backend` (remote host + CLI auth → `BackendKind`), `parse_project_path` (remote-URL → `namespace/project`), `parse_project_path_from_web_url`, `parse_group`, `parse_remote_host`, `get_current_branch`, `repo_root`, `slugify`, `get_workflow_files`.
* [src/custom_commands.rs](src/custom_commands.rs): User-defined `[[custom_keybindings.<pane>]]` commands — `CustomCommands::load` (validation, built-in shadow detection, `ConfigProblem` reports), `CommandPane` (universal / issues / mrs / jobs / diff and their template arguments), gh-dash style `{{.Name}}` templating (`render`, `TemplateValues`, `GLAB_TUI_*` environment) and `shell_process` (`$SHELL -c`).
* [src/command_log.rs](src/command_log.rs): `CommandLog`, the Terminal tab's history behind `App::terminal_commands`. It keeps the newest `MAX_LOG_ENTRIES` (1,000) `TerminalCommand`s and hands out `LogRow` handles that stay valid while older entries are dropped or the log is cleared. Update entries through `find_latest` / `set_status` / `settle` / `App::settle_command`, never by position, since positions shift on every eviction. Each entry carries an `Execution` (`started_at`, `duration`, `exit_code`, captured `stdout`/`stderr`, and the `Invocation` that makes it re-runnable; entries that only describe an action have no invocation). The Terminal tab filters it with the global search bar and `StatusFilter` (`App::filtered_terminal_commands`, `cycle_status_filter` key), acts on the bottom row of the view (`App::terminal_cursor` / `selected_terminal_row`), or on the inspected entry while the inspector is open: copy command (`copy_command`), copy output (`copy_output`), re-run (`rerun_command`), and clears it (`clear_log`).
* [src/scope.rs](src/scope.rs): `Scope` enum (`Repository` / `Group`) — `as_str()`, `is_group()`/`is_repository()`, `cli_repo_arg()`/`cli_group_arg()` (glab/gh flag injection), `api_path_prefix()` (with `%2F` URL encoding), and `Display`.
* [src/handlers/](src/handlers/): Keypress handlers split by concern.
    * [mod.rs](src/handlers/mod.rs): Module declarations.
    * [tabs.rs](src/handlers/tabs.rs): Per-tab keybindings (create/edit/delete/approve/merge/view-diff etc.).
    * [overlays.rs](src/handlers/overlays.rs): Overlay handlers (submit dialog, date picker, help, refresh, repo switcher).
    * [custom_commands.rs](src/handlers/custom_commands.rs): `run_bound_command` — resolves the highlighted row's template values, runs the custom command through `editor::suspend_while` (or a background thread) and logs the outcome.
    * [todos.rs](src/handlers/todos.rs): Todos tab actions (open target and mark read, mark done, mark all read, unsubscribe, show-read toggle). Every mutation goes through `spawn_todo_change`, which reports `Event::TodoChanged`.
    * [terminal.rs](src/handlers/terminal.rs): Terminal tab keys (`handle_tab_key`), the command inspector's keys (`handle_inspector_key`, before the tab handler in the main loop) and `rerun`, which logs a `RERUN_LOG_PREFIX` row and settles it from `Event::CommandRerun`.
* [src/utils/](src/utils/):
    * [cache.rs](src/utils/cache.rs): Offline caching at `~/.cache/glab-tui/<repo>.json` plus recent-groups state (`recent_groups.json`).
    * [format.rs](src/utils/format.rs): Time parsing, ANSI formatting, string truncation, tab expansion (`expand_tabs`), text wrapping (`wrap_text`). `trace_line_content` is a job log line without the GitLab Runner timestamp or the `<job>\t<step>\t` prefix of `gh run view --log` (only split off when a GitHub timestamp follows it), the same split `parse_ansi_trace` renders.
    * [markdown.rs](src/utils/markdown.rs): CommonMark + GFM Markdown rendering via `pulldown-cmark`.
    * [ui.rs](src/utils/ui.rs): Wrappers for `ratatui` stateful lists and tables.
    * [private_fs.rs](src/utils/private_fs.rs): Owner-only `create_dir_all` (`0700`), `create_owned_dir` (`0700`, also tightening an existing directory) and `write` (`0600`, also tightening an existing file) for everything glab-tui persists; plain `std::fs` off Unix.
    * [update.rs](src/utils/update.rs): GitHub releases self-updater with multi-target Linux asset selection. Talks to GitHub through `curl` (never `gh`, so GitLab-only installs can update) and verifies the archive against the release's `<asset>.sha256` before extracting or replacing the running binary.
* [src/cli.rs](src/cli.rs): CLI subcommands (`doctor`, `clean-cache`, `cache`, `open`, `repos`, `review`), flags (`--config` sets `config::use_config_file`, which `Config::config_path()` honours before `GLAB_TUI_CONFIG`/XDG; `-r/--repo` and `-d/--dir` are global, and `--dir` is applied before any subcommand runs) and ANSI-styled diagnostic output.
    * [cli/review.rs](src/cli/review.rs): `glab-tui review threads|comment|submit|reply|resolve` — non-interactive review for scripts and agents. JSON on stdout, errors on stderr with a non-zero exit. `--input` is a JSON array of `{ file, line, end_line?, side?: "old"|"new", body }`; anchors are checked against the diff (`DiffView::new` + `DiffView::contains_anchor`, same classification as the review threads overlay) before anything is posted. E2E coverage in `tests/e2e/review_cli.rs`; the `gh`/`glab` mocks append `--input -` request bodies to `<TEST_LOG_PATH>.stdin` and fail any call matching `TEST_GH_FAIL_MATCH` / `TEST_GLAB_FAIL_MATCH`.
* [src/templates.rs](src/templates.rs): Default issue/MR description templates.
* [src/editor.rs](src/editor.rs): External editor integration (`$EDITOR`/`$VISUAL`) and `suspend_and_run` / `suspend_while`, the shared terminal handoff for any foreground child process (or several in one handoff).
* [src/entity_editor.rs](src/entity_editor.rs): Edit-menu field change logic and creation form helpers.
* [src/ui/](src/ui/): Ratatui render functions.
    * [mod.rs](src/ui/mod.rs): Re-exports and shared render helpers.
    * [inspector.rs](src/ui/inspector.rs): Unified entity inspector component (`render_entity_inspector`, `EntityDocument`, `InspectorMode`). Drives both read-only detail previews and interactive edit/create forms in a single-column layout.
    * [job_trace.rs](src/ui/job_trace.rs): `JobTraceView`, the Jobs pane's job log with its lines parsed incrementally and only the visible ones handed to the renderer. `row_of_line()` maps a log line to its (wrapped) row, and `jump_to_line()` does the same for a jump while remembering the line. `failure_search_start()` is that line while the last draw kept it on screen, else the line the last draw put at the top of the pane. `viewport_height()` is the rows the last draw gave the log, which the page-scroll keys step by on the Jobs tab, since the job header above the log takes part of `detail_rect`.
    * [tabs.rs](src/ui/tabs.rs): Tab-specific render functions.
    * [overlays.rs](src/ui/overlays.rs): Overlay render functions (`SubmitDialog`, selectors, date picker, help).
    * [review_threads.rs](src/ui/review_threads.rs): Review threads overlay (thread list + full-thread preview) opened with `T` from the diff view.
    * [terminal_inspector.rs](src/ui/terminal_inspector.rs): The Terminal tab's command inspector (`App::terminal_inspector`, opened with `Enter`/`P`): status, exit code, full command, start time, duration, error, stderr and stdout. It follows its `LogRow` and closes once that entry leaves the log.
    * [helpers.rs](src/ui/helpers.rs): Shared UI rendering helpers (`badge_style_for`, `render_fuzzy_cell`).
    * [diff.rs](src/ui/diff.rs): Diff view render functions.
    * [modal.rs](src/ui/modal.rs): Unified modal component.
    * [list_tab/](src/ui/list_tab/): `ListTab<C: ColumnDef>` — rows, a cached filtered/sorted view, and a cursor keyed by `Row::key()` (not a view index). Each `ColumnDef` enum (`RunnerColumn`, `ReleaseColumn`, `BranchColumn`, `EnvironmentColumn`) defines label, width, cell, search text, filter values and sort key once; `Tab::columns()` reads its labels. `App::sync_list_views()` rebuilds the active tab's view only when its rows or inputs (search, enabled columns, column filters, group-by sort) change; it runs before each draw and in `update_filter_selection`. Hidden tabs are not rebuilt, because the search query is shared by every tab and a hidden view rebuilt with it would move that tab's cursor. Mutate rows only through `set_items` / `update` / `retain` so the view invalidates; navigate through `App::list_navigation_mut`. Runners, Releases, Branches and Environments use it; the other tabs still use `StatefulTable`.
* [src/themes/](src/themes/): 38 bundled theme TOML files (default, clean, tokyo-night, tokyo-night-storm, oled, github-dark-hc, gruvbox, gruvbox-material, nord, nord-light, catppuccin-mocha, catppuccin-macchiato, catppuccin-frappe, catppuccin-latte, dracula, dracula-light, deep-space, solarized-dark, solarized-light, monokai, one-dark, synthwave-84, everforest-dark, rose-pine, rose-pine-moon, rose-pine-dawn, kanagawa, cyberpunk, ayu-dark, ayu-mirage, ayu-light, night-owl, poimandres, vesper, sonokai, pop-dark, adwaita-dark, adwaita-light).
* [fuzz/](fuzz/): `cargo-fuzz` targets (own workspace, nightly only) for the parsers that take untrusted input: `diff_parser` (`DiffView::new`), `markdown` (`render_markdown`, with a fuzzed width) and `ansi_trace` (`parse_ansi_trace`). `.github/workflows/fuzz.yml` runs each for 60 seconds, seeded from `tests/fixtures/mr_diff.txt`, `README.md`/`CHANGELOG.md` and `fuzz/seeds/ansi_trace.log`. Run one locally with `just fuzz <target>`.
* [deny.toml](deny.toml) / [.cargo/audit.toml](.cargo/audit.toml): supply-chain policy for `cargo deny check` and `cargo audit` (`.github/workflows/audit.yml`). `cargo audit` only reads `.cargo/audit.toml`, never a root `audit.toml`. Every advisory ignore and duplicate-version skip carries the reason it is allowed. Re-check the syntect-only ignores (`bincode`, `yaml-rust`) whenever syntect is upgraded.

## 3. Core Architectural Patterns

### State Management (`App`)
* **Single Source of Truth:** All application state lives in the `App` struct inside [src/app.rs](src/app.rs).
* **No Blocking in UI:** `ui::render` ([src/ui/mod.rs](src/ui/mod.rs)) is called on every tick. Never perform I/O, API calls, or heavy computation anywhere under [src/ui/](src/ui/).

### Event Loop & Async Operations
* User input (`crossterm` events) and background task results communicate with the main loop via the `Event` enum over a `tokio::sync::mpsc::UnboundedSender`.
* **Adding an API Call:** When adding a new API call:
    1. Spawn a `tokio::spawn` task in [src/main.rs](src/main.rs) (on keypress) or [src/app.rs](src/app.rs).
    2. Make the API call using `app.gitlab_client`.
    3. Send an `Event` back to the main thread (e.g., `tx.send(Event::MyDataFetched(data))`). Fetches whose result belongs to the current repo/group send through `app.scoped_sender(events.sender())` (an `event::ScopedSender`): it tags the event with `app.scope_generation`, which `reset_on_scope_change` bumps, and the event loop drops results from an older generation before any handler runs, so a fetch that outlives a scope switch never writes into the new scope's tabs or cache.
    4. Handle the event in the [src/main.rs](src/main.rs) event loop to update `app` state.
* **Never await a remote CLI call inside a key handler.** `handle_active_tab_key` runs on the main loop, so an inline `.await` on `glab`/`gh` freezes rendering and input until it returns. Spawn it and apply the result from an `Event` (e.g. `spawn_open_pipeline` → `Event::PipelineOpened` → `apply_opened_pipeline`, which drops replies for a pipeline no longer listed). Local-only `git` reads that [src/git_helpers.rs](src/git_helpers.rs) documents as safe on the UI thread (`get_current_branch`, `get_default_branch`, `repo_root`) may run inline. A `git` call that can reach the network, such as `git fetch`, is spawned like a remote call.
* **Input runs on a dedicated OS thread** (`EventHandler::new` in [src/event.rs](src/event.rs)): crossterm's `poll`/`read` block and must never occupy a tokio worker.
* **Scope switches build the client off the loop.** `spawn_client_for_scope` clears `app.gitlab_client` and runs `GitlabClient::new` (`git remote` + `gh`/`glab auth status`) in a task. Its `Event::ClientReady { scope, result }` goes through `app.scoped_sender`, so a build that a later switch superseded is dropped by generation, even one for the same scope. `apply_client_ready` installs the client that survives and starts the refresh.

### External Editor Integration
* The application pauses the UI to open an external `$EDITOR` (or `$VISUAL`, defaulting to `helix`).
* This is done using `crossterm::terminal::LeaveAlternateScreen` inside `editor::suspend_and_run` ([src/editor.rs](src/editor.rs)), which `edit_in_editor`, `view_in_editor` (read-only display, e.g. the Jobs tab's job trace) and custom commands (via `suspend_while`) share. It also holds off SIGINT/SIGQUIT in glab-tui while the child owns the terminal, so Ctrl+C stops the child only. Any other foreground child process must go through it too. Do not reinvent this wheel.
* Always build the editor process with `editor::editor_command`, which runs `$EDITOR` through the shell so values with arguments (`code --wait`) work. The job-trace editor key fetches the trace first when it is not loaded (`App::pending_job_trace_editor`, consumed on `JobTraceFetched`).

### Syntax Highlighting (`syntect`)
* All syntect use lives in [src/syntax.rs](src/syntax.rs). `SYNTAX_SET` is a `LazyLock` over `SyntaxSet::load_defaults_newlines()`, so `LineHighlighter::highlight` feeds every line with its `\n` restored. There is no syntect theme: colors come only from the app `THEME`.
* **Stateful:** `LineHighlighter` carries the parse state from line to line. Markdown fences (`highlight_code`) and code suggestions highlight a block as one text. `DiffView::new` drives a `DiffHighlighter`, which keeps one state per side of the current file (context rows feed both, removals the old side, additions the new side) and restarts both at every hunk header.
* **Grammar detection:** `syntax_for_path` matches the whole file name first (`Makefile`), then the extension. A file it cannot name is decided by `syntax_for_first_line` (shebang, mode line) only when its first diff row is line 1. Fences go through `syntax_for_token`.
* **Theme-safe:** `DiffLine::syntax_highlighted` stores `SyntaxToken { kind: TokenKind, range }` runs over `DiffLine::code()` (the line without its diff marker), never resolved colors. Renderers call `DiffLine::syntax_spans(&theme)` / `styled_tokens`, which map each `TokenKind` to a theme token via `TokenKind::style`, so a theme reload recolors an open diff. Scope names are classified once per interned `Scope` by `ScopeKinds`. A token takes the kind of its innermost non-`meta.*` scope: region scopes such as `meta.method.body.java` stay open across lines and would otherwise color every identifier inside them.
* `DiffView::without_highlighting` parses the same structure with no highlighting. The headless `glab-tui review` subcommand uses it, since it only checks anchors.

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
* Non-error feedback goes through `App::show_notice(text)`, the only way to set `App::status_message` (`StatusMessage` in [src/app.rs](src/app.rs)), rendered muted in the header bar until `STATUS_NOTICE_LIFETIME` passes. Stale data is tracked per tab in `App::stale_tabs` (`Tab → StaleData`) and drawn as a red badge next to the notice only while that tab is active: `App::mark_cached_tabs_stale` flags every tab filled from the offline cache, a `FetchFailed` over cached rows flags its tab `RefreshFailed` instead of toasting, and only that tab's own `*Fetched` arm clears it. Because of that, only list refreshes send `FetchFailed`: other background failures send `CommandCompleted(Err)`, and background successes send `Event::Notice`.
* Background mutations never discard their `Result`. Optimistic pipeline/job status flips go through `run_optimistic_mutation` ([src/handlers/tabs.rs](src/handlers/tabs.rs)): failed rows are restored via `PipelineStatusRevert`/`JobStatusRevert`, and all failures surface as one `CommandCompleted(Err)` toast. A failed fetch sends exactly one error event.

### Jobs Tab (current branch)
* `fetch::fetch_current_branch_jobs` reads the branch with `git_helpers::get_current_branch`, then asks `Backend::find_branch_pipeline_id` for the newest pipeline of that branch (on GitLab also its open MR's `refs/merge-requests/<iid>/head` pipeline). Never list every MR or pipeline to find it.
* Starting a manual job is GitLab only. On GitHub the `start_job` key is omitted from the help overlay and does nothing when pressed (it is still consumed, so the custom-command shadow report stays accurate).
* The preview goes through `render_entity_inspector` with `entity_editor::build_job_document`: the highlighted job's metadata above the pipeline's stage summary, or, while a trace is open or loading, as a fields-only header above the separate `Trace` pane (`render_job_trace`). The header is sized from its wrapped field rows (`inspector::read_only_fields_height`, since a `List` drops an item that does not fit whole) and capped at half the pane, so the log keeps most rows.
* Scrolling the log to a line goes through `App::job_trace_scroll_to_line`, resolved by `render_job_trace` with `JobTraceView::jump_to_line`, because only the draw knows the wrapped width. `jump_to_failure` (`E`) searches from `JobTraceView::failure_search_start()`, so a failure on the last page, which the scroll clamp keeps from reaching the top of the pane, is passed on the next press; without a trace it fetches one and sets `App::pending_job_trace_failure`, consumed on `JobTraceFetched` like `pending_job_trace_editor`.
* `copy_url` (`y`) copies `Job::web_url` (GitLab `web_url`, GitHub `url` from `gh run view --json jobs`) and `copy_id` (`Y`) the job id. `copy_sha` defaults to `Alt+y` on this tab.

### Environments Tab (deployments, rollback, stop)
* `view_deployments` (`Enter`) fetches the highlighted environment's deployments, and `Event::DeploymentsFetched` goes through `App::show_deployments`, which ignores an environment no longer highlighted and keeps the focus and the selected deployment (by id) when the shown environment is refetched. `App::discard_stale_deployments` drops them once another environment is highlighted.
* `EnvironmentDeployments::is_focused` decides where navigation goes: `handlers::tabs::handle_focused_deployments_key` takes `j`/`k`/arrows/`Home`/`End` and `Esc`/`Backspace` (unfocus) before the environment list sees them. Focus only counts while the detail pane is on screen (`detail_visible` and a `detail_rect` from the last frame), so hiding it with `q` or a terminal under 90 columns hands the keys back to the environment list. `focus_deployments` defaults to `Tab`, which is also `global.configure`, so the event loop skips the configure check while `App::is_deployments_focus_key` holds (deployments listed on the Environments tab). `,` still opens the Configure View there.
* The detail pane shows the deployments table over `build_deployment_document` for the selected one. Without listed deployments it shows `build_environment_document`, whose content lists the configured action keys.
* `rollback_deployment` (`R`) and `stop_environment` (`d`) open `ConfirmAction::RollbackDeployment` / `StopEnvironment` (both default to Cancel). GitLab rolls back by re-running the deployment's job (`Deployment::job_id`), so a deployment without one is refused before the dialog opens. GitHub rolls back by creating a deployment of the commit, which deploys nothing by itself: it only takes effect once a workflow running `on: deployment` picks it up, and the dialog says so. A successful rollback refetches the environment list and its deployments.
* `view_deployment_log` (`e`) fetches the deployment job's trace and opens it in the editor from `Event::DeploymentLogFetched`. Like `pending_job_trace_editor` on the Jobs tab, `App::pending_deployment_log` records the request: a second `e` while it is in flight starts no second fetch, and `App::take_pending_deployment_log` opens the log only while that deployment is still selected on the Environments tab. GitHub deployments carry no job, so like `start_job` the key is left out of the help overlay and of `build_environment_document`'s action list there, and pressing it is still consumed and raises an error.

### Pipelines Tab
* `Enter` descends into a pipeline, so the inspector is opened with `toggle_preview` (`i`). `build_pipeline_document` ([src/entity_editor.rs](src/entity_editor.rs)) shows the triggering commit, a red `Failed Jobs` count, and `InspectorContent::PipelineStages` lists each failed job with `Job::failure_reason` above the stage list. GitLab fills `failure_reason` from the jobs API (`script_failure` → `script failure`), GitHub from the job's timeout or its first failed step (`github_job_failure_reason`).
* The inspector's failed jobs and the `Stages` dots come from `app.pipeline_jobs`, which the highlight prefetch fills only for pipelines it does not hold. `PipelinesFetched` calls `App::drop_jobs_of_changed_pipelines` (compared against the root level, also inside a descent) to forget the jobs of every pipeline whose status changed, and `spawn_pipeline_retry` forgets the retried ones, so neither shows the previous outcome.
* `r` and `R` share `spawn_pipeline_retry` (`PipelineRetry::Pipeline` / `FailedJobs`) in [src/handlers/tabs.rs](src/handlers/tabs.rs). `Backend::retry_failed_jobs` is `gh run rerun --failed` on GitHub. On GitLab it lists the pipeline's failed jobs and retries each one, because the pipeline retry endpoint also reruns canceled jobs.
* `s` steps the Pipelines `Status` column filter through `PIPELINE_STATUS_FILTER_CYCLE` with `App::cycle_pipeline_status_filter`; it reuses `column_filters`, so the Configure View picker shows and clears the same filter.

### Rate Limiting (`src/backend/rate_limit.rs`)
* `ApiRateLimiter` is a global (`RATE_LIMITER`) that shields every `glab`/`gh` CLI call from HTTP 429 and GitLab GraphQL rate-limit errors.
* Backend command execution delegates to `rate_limit::execute_with_retry` (used by both `GlabBackend` and `GhBackend`), which:
  - acquires a concurrency slot via `RATE_LIMITER.pace_request()` (bounds concurrent subprocesses to `MAX_CONCURRENT_REQUESTS` and enforces a minimum inter-request burst delay) for the duration of one attempt only; the slot is released before any backoff sleep,
  - retries failures that `is_rate_limit_error()` recognizes (`429` as an HTTP status such as `HTTP 429` or `GET <url>: 429`, never bare digits like `#429`; `rate_limit`, abuse-detection, `retry-after`) with exponential backoff plus per-thread jitter up to `MAX_RATE_LIMIT_RETRIES`,
  - bubbles up immediately on non-rate-limit errors so the UI shows the real failure.
* Every `gh`/`glab` API call (the four backend runners, the diff view's comment edit/delete and release creation) runs through `subprocess::output`, so a hung CLI (e.g. a credential helper waiting on a prompt) is killed after `command_timeout_seconds` (config, default 120, reloaded with `config::reload_command_timeout()` on repo switch) instead of holding its slot forever. New API calls must use it too, never `.output()` on a `gh`/`glab` `Command`. Not covered: the self-updater (`perform_self_update`: its `gh release view` calls and the large `gh release download`), the `auth status` probes in `git_helpers` (blocking `std::process` in `spawn_blocking`) and `web--browse`.
* Bulk operations (merging, editing many MRs/issues, etc.) call `pace_bulk_operation()` before each item to avoid tripping secondary rate limits during batch loops.

### Bounded CLI Output, Job Logs & Follow Mode
* The command runners (`run_glab_command`, `run_gh`, `run_git_command`) read output through `bounded_output::output_bounded` ([src/backend/bounded_output.rs](src/backend/bounded_output.rs)). With `Some(limit)` it keeps the first `limit` bytes of stdout and kills the child once it writes more (`CapturedStdout::is_truncated`). Stderr is capped at 64 KiB and drained past that. Plain `run_glab_command` / `run_gh` pass `None` (read everything). Output whose size a third party controls (job logs, diffs) goes through `run_glab_capped` / `run_gh_bounded` with a limit. Only capped reads are killed when their future is dropped: an uncapped run (every mutation, e.g. merge) finishes even if glab-tui quits first.
* Job logs are read up to `JOB_TRACE_MAX_BYTES` (16 MiB) into a `JobTrace` ([src/domain/job_trace.rs](src/domain/job_trace.rs)). It tracks the bytes read (`TraceCursor`), holds a UTF-8 character split across reads until it completes, and records truncation. A truncated log shows `[TRUNCATED: first 16 MiB]` in the Jobs pane title, and follow mode stops polling it.
* Follow mode (`f`) polls `Backend::get_job_trace_since(cursor)` every 5 seconds (`Event::JobTraceFollowed`). `app.job_trace_poll_in_flight` keeps the log on screen meanwhile. A poll moves the pane to the bottom only when it brought text (`TraceUpdate::has_new_text`), so a quiet log can be read further up with follow on. GitLab reads `?byte_offset=&byte_limit=` starting 1 KiB before the end of what was read, checks that the reply starts with those bytes, and loops in chunks of at most 500 KiB (GitLab's `byte_limit` maximum). Each reply is sorted by `classify_ranged_reply` (a pure function with unit tests, so new reply cases go there). A server that ignores the parameters (it predates them) is detected from the reply, remembered per project in `PROJECTS_IGNORING_TRACE_RANGES` (process-wide, since every `GitlabClient` clone builds a fresh backend), and polled with full bounded reads. A log that no longer starts with what was read is replaced (`TraceUpdate::Restart`). GitHub returns nothing new without a CLI call: `gh run view --log` serves only completed jobs, whose logs never change.
* `app.job_trace` is a `JobTraceView` ([src/ui/job_trace.rs](src/ui/job_trace.rs)). It parses only text added since the last frame (plus the still incomplete last line), keeps the wrapped row offset of every line per width, re-parses everything only after a theme change or a restart, and gives the renderer only the visible lines. Never call `parse_ansi_trace` on the whole log from render code.
* Diffs (`get_mr_diff`, `diff_pr_in_checkout`) are read up to `MR_DIFF_MAX_BYTES` (32 MiB). A larger diff fails with an error (`ensure_complete_diff`) instead of showing a partial diff that a review could be submitted against.

### Code Review & Diff System
* **Diff view** supports inline comments, code suggestions, draft reviews, dynamic gutter sizing, and tab expansion:
  - `DiscussionNote` / `NotePosition` structs in [src/domain/mr.rs](src/domain/mr.rs).
  - `list_mr_notes()` fetches notes for an MR via the API.
  - Draft comments are the open diff's working copy in `app.draft_comments: Vec<DraftComment>` (`DraftComment` / `ReviewEvent` live in [src/domain/review.rs](src/domain/review.rs)) and are submitted atomically. Every add, edit, delete, discard and submit calls `App::persist_draft_comments(project_path, mr_iid)` with the open diff's `DiffView::project_path`, which mirrors them into `ProjectCache::draft_comments` (`project_path → mr_iid → Vec<DraftComment>`) and saves the cache, so drafts survive closing the diff and restarting. The project is part of the key because a group-scope cache is shared by every project in the group and the same iid recurs across them.
  - `DiffFetched` restores drafts through `App::restore_draft_comments`, which checks each one with `DiffView::contains_draft_anchor` (its first and, for a range, last line through `contains_anchor`). Drafts whose lines left the diff go to `app.stale_draft_comments` with a toast giving the count: they are never shown or submitted, `persist_draft_comments` writes them back with the rest, and they load again once their lines are back. `Keep drafts` and `Discard` treat them like the other drafts.
  - Every review mutation goes through `Backend::submit_review` / `reply_to_thread` / `set_thread_resolved`; the diff view and the `glab-tui review` subcommand share them. A comment posted outside review mode is a one-comment `submit_review`. Do not shell out to `gh api` / `glab api` for reviews from the event loop.
  - Current (already-pushed) comments live in `app.current_comments: Vec<DiscussionNote>`.
  - Diffs load through `fetch::fetch_diff_view`, which builds the `DiffView` on a blocking thread (parsing and highlighting a large diff takes seconds) and sends `DiffFetched { diff_view, comments }`; the "Fetching Diff" overlay stays up until it arrives. `D` is ignored while `app.diff_loading` is set, so a second press cannot start a second fetch.
  - The parser in `DiffView::new` follows each hunk's `@@` line counts: while a hunk still owes lines, every line is content, so a removed `-- comment` (`--- comment`) or added `++ x` is never read as a file header. `\ No newline at end of file` is a `Meta` row with no line numbers. Paths are read with `parse_diff_git_paths` / `parse_patch_header_path`, which keep spaces and decode git's C-style quoting.
  - Leaving the diff view with pending drafts opens the `SubmitDialog` (`ConfirmAction::SubmitReview(mr_iid)`) with `Submit`, `Keep drafts`, `Discard` and `Cancel`. `Esc`/`Cancel` only close the dialog and return to the diff with drafts and `in_review_mode` untouched. `Keep drafts` closes the diff and leaves the drafts in the cache. `Discard` deletes them (memory and cache) and closes the diff.
  - Open diff key is `D` (remappable via `keybindings.mrs.view_diff`).
* **GitHub diffs over 20,000 lines:** GitHub refuses to serve them (`gh pr diff` fails with HTTP 406 `PullRequest.diff too_large`). `GhBackend::get_mr_diff` then builds the diff with git in the clone `cache::find_local_checkout` finds for the PR's repository (`diff_pr_locally` / `diff_pr_in_checkout` in [src/backend/gh.rs](src/backend/gh.rs)). It only fetches (skipped when both commits are already local) and diffs — no checkout, no branch, no index or working-tree change. Without a known clone the error says a local clone is needed; every other `gh pr diff` error is returned unchanged. Review comments and review submission are unaffected: they go through the API with the same path/line anchors. E2E coverage: `tests/e2e/pr_diff_fallback.rs`.
* **Dynamic line numbers & tab expansion:** Gutter width is dynamically calculated in `DiffView::new` from the widest line number in the diff (floored at 4). Tabs are expanded to spaces at tab stops at diff parse time (`expand_tabs`) so Go/Makefiles maintain indentation without breaking syntax highlighting or search indices.
* **Suggestion rendering:** `format_comment_with_suggestions()` in [src/ui/helpers.rs](src/ui/helpers.rs) parses ` ```suggestion ` blocks from comment bodies and renders them as in-line diff (red for original, green for suggested).
* **Reviewed-file marks:** `m` toggles `DiffView::reviewed_files` (a `HashSet` of diff-relative file paths) for the selected file, or for every file below the selected directory; `M` toggles `DiffView::hide_reviewed`. Both are purely local — neither GitLab nor GitHub's "viewed" state is synced.
  - The tree is flattened through `DiffTreeNode::flatten_ex(depth, prefix, reviewed, hide_reviewed, out)`, which stamps `FlatDiffTreeNode::is_reviewed` (on a directory: every file below it is reviewed) and, when filtering, drops reviewed files plus directories left with no unreviewed file.
  - `DiffTreeNode::sync_expansion_to_review(before, after)` folds a directory when it becomes fully reviewed and unfolds it when it stops being fully reviewed, cascading up through parents.
  - Marks persist in `ProjectCache::reviewed_files` (`mr_iid → Vec<String>`), written via `App::store_reviewed_files_for_mr` + `save_cache` on every toggle and re-seeded by `DiffView::restore_review_state` on `DiffFetched`.
* **Review threads overview:** `T` in the diff view opens `App::review_threads` (`ReviewThreadsOverview`), a snapshot of `current_comments` grouped by `group_threads()` and classified against the diff via `DiffView::contains_anchor`. It is rebuilt on every `DiffFetched`, so resolve/reply/edit results show up after the automatic re-fetch.
  - Anchor matching for inline rendering, the `a` actions and the overview all go through `note_position_anchors()` in [src/app.rs](src/app.rs); do not re-inline the predicate.
  - `Enter` calls `DiffView::jump_to_anchor`, which falls back to the file's first row for outdated anchors. A file out of reach in the tree is revealed first (`DiffTreeNode::expand_path_to` unfolds its directories; a file hidden by the reviewed filter turns `hide_reviewed` off, and the handler syncs `app.hide_reviewed_files`). `a` reuses `Selector::comment_actions` / `Selector::comment_choice`, the same builders the line-level `a` key uses.
  - `j`/`k` wrap around like `Selector` and `StatefulList`; mouse scroll clamps, as the Selector's does.
  - E2E coverage lives in `tests/e2e/review_threads.rs`, driven by the `mr list` / `mr diff` / `mr note list` branches of `tests/mocks/glab` and the `mr_diff.txt` / `mr_notes.json` fixtures.
  - GitHub lists only pull-request review comments (`pulls/{}/comments`); top-level conversation comments are not fetched and threads report no resolved state.

### MR Review State (Approval / Mergeable / Workflow)
* The MR/PR table's `Approval`, `Mergeable`, and `Workflow` columns are derived, not fetched. `ApprovalState` / `MergeabilityState` / `WorkflowStatus` and the display/sort/filter helpers live in [src/domain/mr_state.rs](src/domain/mr_state.rs); cell text uses ALL-CAPS display strings (e.g. `CONFLICT`, `REBASE`, `CLEAN`, `APPROVED`, `AWAITING`) that the column-filter picker also shows.
* **Data sources:** GitLab fills both axes with one bulk `glab api graphql` query over `mergeRequests(iids: [...])` (batched by `api_per_page`); GitHub derives them from the review/merge fields returned by `gh pr list` (`reviewDecision`, `latestReviews`, `mergeable`, `mergeStateStatus`, `reviewRequests`) plus the current login via `gh api user --jq .login`. Either axis may be `None` (unknown) — never a guessed value.
* `MergeRequest` carries `approval` and `mergeability` as `Option<…>` and a `#[serde(skip)]` derived `workflow`. After any load (live fetch or cache read), call `derive_workflow()` in [src/fetch.rs](src/fetch.rs) to recompute `workflow` from approval state — cached rows deserialize with it unset even though the approval state it reads was persisted.
* **Rebase gating:** `rebase_gate()` in [src/domain/mr_state.rs](src/domain/mr_state.rs) decides whether `R` may rebase — `Allowed`, `ResolveLocally` (conflicts), or `NotNeeded` — surfaced as a confirm popup or a user-facing error toast. Revoking approval (`A`) is GitLab-only; `gh pr review` has no revoke path.

### Todos Tab (GitLab Todos / GitHub Notifications)
* The read filter lives on `GitlabClient::show_read_notifications` (default `false`, unread only) so every refresh path fetches with it. `App::toggle_show_read_todos` flips it and `Event::TodosFetched` carries the filter it was requested with, so a list fetched before a toggle is dropped.
* Mutations report `Event::TodoChanged(TodoChange, Result)`. On success `App::apply_todo_change` updates `app.todos` in place (no refetch): read and done items are restated while the tab shows them and dropped otherwise, and GitHub's done notifications always leave the list. On failure the error goes through `App::show_error`.
* GitLab todos have no read state: `mark_notification_as_read` and `mark_notification_as_done` both run `glab todo done`.
* `mark_as_read` and `Enter` share `handlers::todos::open_selected`. The jump resolves the target in the todo's own project (`Scope::contains_project`, `App::focus_issue` / `focus_mr` with `Some(project)`) and fetches it when unlisted. A todo outside the current scope, or whose target has no view in glab-tui, raises an error instead of landing on a same-numbered item, and stays unread: `jump_to_target` returns whether it had something to open, and only then is the todo marked read.
* E2E coverage: `tests/e2e/todos.rs`, driven by `todo list` (`todos.json`) and `api todos?state=done` (`todos_done.json`) in `tests/mocks/glab`.

### Cache & State Persistence
* Cache directory: `~/.cache/glab-tui/` (migrated from `~/.glab-tui-cache`).
* `ProjectCache` stores `enabled_columns`, `group_by_column`, `group_ascending`, `column_filters`, `labels`, `label_colors`, `reviewed_files`, and `draft_comments` in addition to API data.
* Cache is written on every successful data fetch; read on startup.
* **Rule:** cache and config files carry private repository data and custom shell-command bindings, so every write under the cache or config directories (cache JSON, `recent_repos.json`, `recent_groups.json`, `config.toml`, themes, repo-local `.glab-tui/`) goes through `utils::private_fs`, never plain `std::fs::write` / `create_dir_all`, which would inherit the umask (`0644`/`0755` under `022`). Directories glab-tui owns outright (`~/.cache/glab-tui`, and the default `$XDG_CONFIG_HOME/glab-tui` or `~/.config/glab-tui`) go through `private_fs::create_owned_dir`, which also tightens one an older version left at `0755`: the cache path helpers (`prepare_cache_dir`) and `Config::config_path()` run on every startup, so upgraded installs are fixed on first launch. Any other directory goes through `private_fs::create_dir_all` and keeps its mode if it already exists, since the parent of a `GLAB_TUI_CONFIG` file may be user-chosen.

### Config & Theme System
* Config is loaded via `Config::load()` in [src/config.rs](src/config.rs) at startup and stored on `App` as `app.config`.
* `Config` carries an optional `backend: Option<BackendKind>` (deserialized lowercase as `"github"` / `"gitlab"`). When set it overrides automatic backend detection everywhere — startup, repo switcher, diff review submission, and `doctor` — via `git_helpers::detect_backend(remote_url, config.backend)`.
* `Config` exposes both `page_size` (total item budget per tab) and `api_per_page` (items per HTTP request, clamped to GitLab's `1–100` `per_page` range via `api_per_page_clamped()`). Thread both through the `Backend` pagination methods; `_per_request` is a no-op on GitHub, which paginates with `--limit`.
* `fetch_label_colors` (default `true`) selects between the real label colors returned by `glab label list` / `gh label list` and the theme's label palette. The API colors are stored as a `name → Color` map on `app.label_colors` (populated from the cache at startup and refreshed on `RepoAttributesFetched`); light GitHub-style label colors fall back to the theme palette because they are unreadable as foreground text on dark themes (`is_light_color()` luminance check in [src/ui/helpers.rs](src/ui/helpers.rs)).
* `Config::load()` only reads existing config files (global then repo-local) and merges overrides; it **never** writes. `config.toml` is created solely by an explicit save (`save_layout` via the Configure View submit button), targeting either global (`~/.config/glab-tui/config.toml`) or repo-local (`.glab-tui/config.toml`). If no config file exists, the app boots from in-memory defaults.
* Theme selection: `Config` holds a `theme_preset: Option<String>` and optional per-color `ThemeOverrides`. At startup, `App::apply_config()` resolves the final `Theme` and writes it into the global `THEME` `RwLock`. `Theme::default()` derives directly from `src/themes/default.toml` — there is no hardcoded in-code fallback, so the bundled TOML is the single source of truth. Invalid user theme overrides automatically fall back to bundled presets.
* Icons: The global `ICONS` `RwLock` is built from `Config::icons` (`IconSet::Nerd` default, or `IconSet::Ascii` via `icons = "ascii"`) through `IconSet::icons()`, and rebuilt by `reload_icons()` after a repo switch. `Icons::ascii()` keeps every glyph a single ASCII character for terminals without a Nerd Font, whose Private Use Area fallbacks render double-width and break layout. **Rule:** never write a glyph literal in render code; add a field to `Icons` and fill it in both `Icons::nerd()` and `Icons::ascii()`.
* Built-in theme presets are compiled into the binary via `include_str!` in `BUNDLED_THEMES` (38 presets including `oled`, `github-dark-hc`, and the Rosé Pine set). User themes in `~/.config/glab-tui/themes/` take precedence.
* **Semantic tokens & transparent backgrounds:** `group_by` banner styling uses the `badge_group_bg` token, and an empty color string in theme TOMLs maps to `Color::Reset`, letting the terminal's own (possibly transparent) background show through instead of painting a solid fill.
* **Rule:** Never hard-code RGB colors outside `src/themes/*.toml`. Add new semantic tokens (`diff_gutter_bg`, `diff_sep`, etc.) to `Theme` if needed.

### Keybinding System
* All keybinding defaults are defined via the `keybind_defaults!` macro in [src/config.rs](src/config.rs).
* At runtime, every keypress is matched against the config using `keybinding_matches(binding: &str, event: &KeyEvent) -> bool` in [src/keybinding.rs](src/keybinding.rs).
* **Pattern for all new action handlers:**
  ```rust
  _ if (key_event.code == KeyCode::Char('x')
      || keybinding_matches(&app.config.keybindings.tab.action, &key_event)) => { ... }
  ```
* Never add bare `KeyCode::Char('x') =>` match arms for user-facing actions. Always go through `keybinding_matches()` so users can remap.

### Custom Keybindings (`[[custom_keybindings.<pane>]]`)
* User-defined shell commands bound to keys, modelled on gh-dash including its template argument names (`prs` is a serde alias of `mrs`; `normalize_custom_keybindings` renames it per file before the global and repo-local files are merged, since both keys in one merged value is a duplicate field). `Config::custom_keybindings` holds the raw entries; `App::load_custom_commands()` validates them into `App::custom_commands` at startup and after the repo switcher reloads the config, logging every `ConfigProblem` as a failed Terminal-tab entry plus one toast.
* Panes: `universal` (every tab), `issues`, `mrs`, `pipelines` (`PipelineId`, `Ref`, `Sha`), `jobs` (the Jobs tab: `JobId`, `JobName`, `Stage`, `Status`; `JobName` is `Job::api_name`, the name the API reported before `process_pipeline_jobs` split off the matrix; `Stage` is unavailable for GitHub jobs and the repository comes from `App::jobs_project`, then the job's web URL, then the repository scope, through `row_project` like issues and MRs), and `diff` (the MR/PR diff view, dispatched from the catch-all arm of the diff key handler in [src/main.rs](src/main.rs); `FilePath`/`LineNumber` come from `DiffView::cursor_file_position()`). `diff_view_action()` lists the diff view's own keys for shadow reports — it matches key codes regardless of modifiers, so keep it in sync when adding a diff-view key. Template arguments per pane live in `CommandPane::template_arguments()`; row values are resolved in [src/handlers/custom_commands.rs](src/handlers/custom_commands.rs) from the highlighted row. `RepoPath` is `git_helpers::repo_root()` in repository scope and `cache::find_local_checkout()` (recent repos by `origin`) in group scope.
* Dispatch happens in the last arm of the list-view fallback in `handle_active_tab_key`, so every built-in, configured or hard-wired, wins. `builtin_action()` in [src/custom_commands.rs](src/custom_commands.rs) mirrors that precedence to report shadowed keys; when you hard-wire a new list-view key, add it to `LIST_VIEW_CODE_KEYS` / `tab_hardwired_keys()` when the arm compares only `key_event.code` (any modifier is taken), or to `LIST_VIEW_EXACT_KEYS` / `tab_hardwired_exact_keys()` when it goes through `keybinding_matches` or checks modifiers. Pane-specific custom bindings win over universal ones.
* A non-empty `selected_issues` / `selected_mrs` / `selected_pipelines` / `selected_jobs` turns an `issues` / `mrs` / `pipelines` / `jobs` command into a bulk run: `targets()` yields one `Target` per selected item (sorted by project and number, jobs by id), and all renderable runs execute inside one `editor::suspend_while` handoff. `run_all` times each run (`FinishedRun::execution`: start time and duration), and each run is logged with that timing through `App::record_command_outcome` (terminal runs) or `App::settle_command` (background runs); failures raise a single summary toast via `App::raise_error_toast`.
* `background = true` entries skip the handoff: `execute()` runs them on a thread with stdin/stdout detached (`Handoff::Background`) and sends `Event::CustomCommandFinished(RunReport)`; the main loop logs it through `report_runs()`, the same path terminal runs use. Stderr is always collected so a failure names its cause; it is only echoed to the screen for terminal runs. `detach_from_terminal()` starts background runs with `setsid(2)` (Unix): Go/termenv tools open `/dev/tty` and query the background colour even with stdio redirected, and the terminal's reply would otherwise arrive as keypresses. `tests/e2e/custom_keybindings.rs` guards this.
* Feedback while a command works: a background run logs each rendered command as `Running` up front (`App::start_command`) and `report_runs()` settles those rows through their `LogRow` handles (`RunReport::log_rows`, `App::settle_command`); a terminal run prints `glab-tui: running "<label>": <command>` to stderr before starting, so a silent command does not leave a blank screen. Custom-command log rows start with `CUSTOM_COMMAND_LOG_PREFIX` (Terminal-tab re-runs with `RERUN_LOG_PREFIX`), and the generic "most recent `Running` entry" fallbacks (`show_error`, `CommandCompleted`, `TerminalCommandLogged`) go through `App::latest_running_cli_command()`, which skips them so an unrelated completion cannot stamp a background command finished.
* `render()` splices a value into `sh -c` only when it is a single plain word: it refuses values starting with `-` or `+` (vim/nvim/less run `+cmd` as a command, and `FilePath` comes from the PR diff) and values containing whitespace, control characters or a `SHELL_SIGNIFICANT_CHARACTERS` entry (quoting, substitution, separators, redirection, glob, brace/tilde expansion, `#`, `=`). Every value is also exported as `GLAB_TUI_<NAME>`, the only safe way to pass free text such as titles. Keep that guard when adding arguments.
* Keys are parsed with `keybinding::binding_key_event`, the inverse of `keybinding_matches`; a key it rejects is reported as unbindable.

### DatePicker
* `DatePicker` in [src/app.rs](src/app.rs) is a modal widget for selecting dates. It holds `year`, `month`, `day` and a `DatePickerAction` enum identifying which field it's editing.
* Open it by pushing `Some(DatePicker::new(...))` into `app.date_picker`; close it by setting to `None`.
* Navigation: `h`/`l` → previous/next month, `j`/`k` → previous/next day, `Enter` → confirm, `Esc` → cancel.

### Submit Dialog & Confirmations (`SubmitDialog`)
* Mutating and destructive actions (close/reopen issue/MR, merge MR, bulk merge, delete branch/release/milestone/issue/MR, rebase, revoke approval, submit review, cancel pipeline, bulk job cancel, stop environment, roll back deployment, re-run of a Terminal-tab command) open a `SubmitDialog` ([src/app.rs](src/app.rs)). Confirmed bulk job cancels are paced with `pace_bulk_operation()`.
* Pipeline and job actions resolve their project through `App::project_path_for_pipeline(id)` (the listed row's `project_path`, then `active_pipeline_project` for the active pipeline, then the scope) or `App::project_path_for_jobs()` for the Jobs tab. `App::pipeline_project(id)` / `App::jobs_project()` are the same lookups without the scope fallback, and `active_pipeline_project` only ever holds what `pipeline_project` resolved, never the scope. Never pass `app.scope.as_str()` as a pipeline/job project: in group scope it is the group path.
* Includes title, context-aware description body, optional toggleable options (squash, delete branch, auto-merge), and explicit `[ Submit ]` (left, idx 0) and `[ Cancel ]` (right) buttons. `SubmitDialog::extra_buttons` (`DialogChoice`) adds buttons between them; the submit-review dialog uses it for `Keep drafts` / `Discard`.
* Navigation: `h`/`l` / Left/Right move to the neighbouring button, `j`/`k` / Up/Down for vertical option traversal, `Tab`/`BackTab` for full control cycling, `Space` to toggle options, `Enter` to activate focused button, `Esc` to cancel. Cancel never has side effects.
* Destructive actions (close, delete, revoke, cancel pipeline/jobs) default the cursor to Cancel; reversible actions (merge, rebase, review) and re-runs (the user asked for that exact call, shown in full in the body) default to Submit.
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
* **State filter → server state (GitHub repositories only):** the Issues and MRs/PRs State filter picks what `gh issue list` / `gh pr list` fetch. `App::state_filter_query` turns one selected state into a `StateQuery` ([src/domain/state_query.rs](src/domain/state_query.rs)). Two or more states, or no filter, give `All`. `Backend::server_state_query` decides what the server is asked for: GitHub narrows repository listings, while GitLab and GitHub org listings keep fetching every state. Every `spawn_refresh_active_tab` / `spawn_refresh_all_tabs` call passes `app.requested_state_queries()`. `IssuesFetched` / `MrsFetched` carry the query they were listed with. A result that does not cover the current filter is dropped, and `ProjectCache::issues_state_query` / `mrs_state_query` make `App::restore_cached_issues` / `restore_cached_mrs` skip cached rows listed for a narrower state. `fetch::dispatch_state_query_refetch` (on `Event::Tick`) refetches a loaded or loading tab once its requested query changes, whatever path edited the filter. While the server narrows, the State picker offers every state, so the user can switch to one that was not fetched.
* When rendering the MR/PR pipeline status column, check `is_github` to display "Pipeline" (GitLab) or "Action" (GitHub) terminology.
* MR/PR review-state columns (`Approval`, `Mergeable`, `Workflow`) are derived in [src/domain/mr_state.rs](src/domain/mr_state.rs).

## 4. UI & Rendering Guidelines (`ratatui`)

* **Colors & Theming:** Always use the `THEME` global (a `RwLock<Theme>` initialized from `app.config` at startup). Access it as `crate::config::THEME.read().unwrap()`. Do not hard-code raw RGB values; add new semantic color tokens to `src/config.rs` and all theme TOML files if needed. Every surface is theme-driven, including the diff view (`diff_addition_*`/`diff_deletion_*`/`diff_gutter_bg`/`diff_sep`/`comment_bg`/`comment_draft_bg`), markdown rendering, and diff selection/search-match highlights (`highlight_bg`/`yellow_bg`). Pass the resolved theme into render helpers instead of re-locking `THEME` inside them (see `render_markdown`).
* **Fuzzy Matching:** Use `SkimMatcherV2` from the `fuzzy-matcher` crate for filtering tables and selector overlays. The `render_fuzzy_cell` helper in [src/ui/helpers.rs](src/ui/helpers.rs) handles highlighting matched characters in yellow.
* **Untrusted Text:** Titles, descriptions, comments, release notes and job/stage names come from third parties and may carry raw ANSI escape sequences. Pass them through `sanitize_untrusted()` ([src/utils/format.rs](src/utils/format.rs)) before building spans; it strips CSI/OSC/DCS sequences (7-bit and C1) and stray control characters, and borrows the input unchanged when it is clean. `render_markdown`, `render_fuzzy_cell`, `format_comment_with_suggestions` and the inspector field list already do so; `strip_ansi_escapes()` is the owned-`String` form used by the diff parser and job-trace wrapping.
* **Columns:** Table columns are dynamically configurable. `ListTab` tabs draw only the columns enabled in `app.enabled_columns`, which `sync_list_views` feeds into the view, so `ListTab::render` needs no per-cell check. `StatefulTable` tabs must check `app.is_column_visible(tab, "Column Name")` before rendering each cell and header. GitHub-only or GitLab-only columns must also gate on `app.gitlab_client.is_some()` / `is_github`.
* **Layout:** Use `ratatui::layout::Layout` to split screens. Avoid hardcoded fixed sizes where possible, use `Constraint::Percentage` or `Constraint::Fill(1)`. Use `centered_rect_min()` for overlays to ensure minimum readable dimensions on small terminals.

## 5. Adding a New Feature (Workflow)

If asked to add a new Tab (e.g., "Deployments"):
1.  **Update State:** Add the tab to the `Tab` enum in [src/app.rs](src/app.rs) (include it in `ALL`, `title()`, `columns()`, and `default_columns()`). Add a `ListTab<YourColumn>` to `App` with a `ColumnDef` enum in [src/ui/list_tab/](src/ui/list_tab/), and add it to `sync_list_views` / `list_navigation(_mut)`.
2.  **Define Domain Logic:** Create [src/domain/deployments.rs](src/domain/deployments.rs). Define the `Deployment` struct with `serde` traits. Write a `list_deployments` function that delegates to the backend.
3.  **Add Backend Methods:** Add the relevant method to the `Backend` trait in [src/backend/mod.rs](src/backend/mod.rs) and implement it in both [glab.rs](src/backend/glab.rs) and [gh.rs](src/backend/gh.rs). Use native subcommands where available; fall back to `raw_api()` only if no native command exists.
4.  **Create Events:** Add `DeploymentsFetched(Vec<Deployment>)` to the `Event` enum in [src/event.rs](src/event.rs).
5.  **Handle Data Fetching:** In [src/main.rs](src/main.rs), update `spawn_refresh_active_tab` (in [src/fetch.rs](src/fetch.rs)) to fetch data for the new tab.
6.  **Handle UI Updates:** In [src/main.rs](src/main.rs), handle `Event::DeploymentsFetched` with `app.deployments.set_items(..)`, call `app.update_filter_selection()`, copy `app.deployments.items()` into `app.project_cache` and save the cache.
7.  **Handle Navigation:** In [src/handlers/tabs.rs](src/handlers/tabs.rs), add the tab to the `Runners | Releases | Branches | Environments` arms of the `KeyCode::Down`/`Up`/`Home`/`End` handlers, which move the cursor through `app.list_navigation_mut(app.active_tab)`. Mouse scroll and click already go through `list_navigation_mut`.
8.  **Render:** In [src/ui/tabs.rs](src/ui/tabs.rs), add a `render_tab_deployments` that calls `app.deployments.render(f, area, TableContext { .. })` and builds the details preview from `app.deployments.cursor_row()` with `render_list_preview`.

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
| List MR notes | `glab mr note list <iid> --output json -R <repo>` (prints discussions `{id, notes: [...]}`, which `parse_mr_discussions` flattens into notes tagged with their discussion id) | glab fetches every page itself |
| List pipelines | `glab ci list --output json -R <repo> --page N --per-page <api_per_page>` | Loops up to `page_size/api_per_page` pages |
| Current branch's pipeline (Jobs tab) | `glab ci list --output json -R <repo> --ref <branch> --per-page 1` and `glab mr list --output json -R <repo> --source-branch <branch> --per-page 1`, then `glab ci list … --ref refs/merge-requests/<iid>/head --per-page 1` when the branch has an open MR. Newest id wins | Single targeted calls, never a full MR or pipeline listing |
| List runners | `glab runner list --output json -R <repo> --per-page <N>` | Single call (then one GraphQL query fills tags, version, last contact and IP — see Runner details below) |
| List releases | `glab release list --output json -R <repo> --per-page <N>` | Single call |
| List milestones | `glab milestone list --output json --project <repo> --per-page <N>` | Single call (`milestone list` requires `--project`/`--group`, not `-R`) |
| List milestone issues | `glab issue list --milestone <title> --all --output json -R <repo> --per-page <N>` | Single call (`--milestone` filters by milestone **title**, not iid — resolved from the selected milestone at the call site) |
| List todos | `glab todo list --output=json` | Single call (done todos are added from the raw API only while the Todos tab shows read items) |
| List labels | `glab label list --output json -R <repo> --per-page <api_per_page>` | Single call (label colors feed the Labels column) |

#### Mutations — Native Subcommands

| Operation | Command |
|---|---|
| Create release | `glab release create <tag> -R <repo> -n <name> -N <notes>` |
| Update release | `glab release create <tag> -R <repo> -n <name> -N <desc>` (updates the existing release, since `glab release` has no `update` subcommand) |
| Delete release | `glab release delete <tag> -R <repo> -y` |
| Close/reopen milestone | `glab milestone close\|reopen <iid> -R <repo>` |
| Update milestone | `glab milestone update <iid> -R <repo> --title ... --description ...` |
| Delete milestone | `glab milestone delete <iid> -R <repo> -y` |
| Cancel pipeline | `glab ci cancel pipeline <id> -R <repo>` |
| Retry job | `glab ci retry <job_id> -R <repo>` |
| Retry failed jobs (`R`) | `glab ci retry <job_id> -R <repo>` for each job the failed-jobs listing below returns, paced with `pace_bulk_operation()` |
| Cancel job | `glab ci cancel job <id> -R <repo>` |
| Start manual job | `glab ci retry <job_id> -R <repo>` |
| Roll back deployment | `glab ci retry <deployment job id> -R <repo>` (re-runs the selected deployment's `deployable` job) |
| Run pipeline (variables/inputs) | `glab ci run [--branch <ref>] [--mr] [--variables k:v ...] [--input k:v ...]` |
| Mark todo done | `glab todo done <id>` |
| Mark all todos done | `glab todo done --all` |
| Unsubscribe from a todo's issue/MR | `glab issue\|mr unsubscribe <iid> -R <project>` |
| Revoke MR approval | `glab mr revoke <iid> -R <repo>` |
| Rebase MR | `glab mr rebase <iid> -R <repo>` |
| MR diff refs (review anchors) | `glab mr view <iid> --output json -R <repo>` (`diff_refs`) |
| Review summary note | `glab mr note create <iid> -R <repo> -m <body>` |
| Reply to review thread | `glab mr note create <iid> -R <repo> --reply <discussion-id> -m <body>` |
| Resolve/reopen review thread | `glab mr note resolve\|reopen <discussion-id> <iid> -R <repo>` |

#### Data Fetching — Raw API (no native subcommand exists)

| Operation | Endpoint | Why raw API |
|---|---|---|
| List pipeline jobs | `GET /projects/{}/pipelines/{}/jobs?per_page=<N>` (reads `failure_reason` and `commit`) | `glab ci view` is interactive TUI; `glab ci get` returns nested pipeline object with different structure |
| List a pipeline's failed jobs (`R`) | `GET /projects/{}/pipelines/{}/jobs?scope[]=failed&per_page=100&page=<P>` | Same as above. Returns the latest attempt of each failed job. Pages are read until a short one, all before the first retry, since a retried job leaves the listing |
| Get job trace | `GET /projects/{}/jobs/{}/trace` (read up to `JOB_TRACE_MAX_BYTES`) | `glab ci trace` is interactive/streaming; we need programmatic text output |
| Follow job trace (follow mode poll) | `GET /projects/{}/jobs/{}/trace?byte_offset=<N>&byte_limit=<≤500 KiB>` | Only the bytes after what is shown; a server that ignores the parameters is detected and polled with full (bounded) reads |
| List done todos | `GET todos?state=done` | `glab todo list` only shows pending |
| List branches | `GET /projects/{}/repository/branches?per_page=<N>` | No `glab branch` command |
| Create branch | `POST /projects/{}/repository/branches?branch=...&ref=...` | No `glab branch` command |
| Delete branch | `DELETE /projects/{}/repository/branches/{}` | No `glab branch` command |
| List environments | `GET /projects/{}/environments?per_page=<N>` | No native command |
| List deployments | `GET /projects/{}/deployments?per_page=<N>&order_by=id&sort=desc[&environment=<name>]` | No native command. The API sorts oldest first by default, which would cut the newest deployments off the page |
| Stop environment | `POST /projects/{}/environments/{}/stop` | No native command |
| List members | `GET /projects/{}/members/all?per_page=100` | `glab repo members` only has add/remove |
| Runner details (tags, version, last contact, IP) | `glab api graphql` over `project(fullPath:).runners { id tagList contactedAt managers { version ipAddress contactedAt } }`, one query after `glab runner list` | `glab runner list` returns none of these fields and `glab runner` has no view subcommand. GitLab deprecated the runner's own `version` and stopped returning its `ip_address` in 17.0 in favour of the runner managers. A runner the query does not return (a group or project runner the token cannot read) still lists, with those fields unknown |
| Retry pipeline | `POST /projects/{}/pipelines/{}/retry` | `glab ci retry` is job-only; no pipeline retry subcommand |
| MR approval/mergeability state | `glab api graphql` over `mergeRequests(iids: [...])` | `glab mr list` exposes neither axis; one bulk query fills the Approval/Mergeable columns (batched by `api_per_page`) |
| List group issues | `GET /groups/{}/issues?state=...&per_page=<N>&page=<P>` | `glab issue list -g` spans each project via separate requests; a single group endpoint aggregates across the whole group. Group path is `%2F` URL-encoded. |
| List group MRs | `GET /groups/{}/merge_requests?state=...&per_page=<N>&page=<P>` | Same rationale as group issues |
| List group pipelines | `GET /groups/{}/pipelines?per_page=<N>&page=<P>` | Same rationale as group issues |
| List group milestones | `GET /groups/{}/milestones?per_page=<N>` | Native `glab milestone list --group` returns only the group's own milestones per project |
| List group labels | `GET /groups/{}/labels?per_page=<N>` | `glab label list -g` has no pagination control |
| List environments | `GET /projects/{}/environments?per_page=<N>` | No native command |
| List deployments | `GET /projects/{}/deployments?per_page=<N>&order_by=id&sort=desc[&environment=<name>]` | No native command |
| Review draft notes | `POST /projects/{}/merge_requests/{}/draft_notes` per comment, then one `POST .../draft_notes/bulk_publish` | `glab mr note create` posts one note at a time and cannot batch them into a review |

### GhBackend (`src/backend/gh.rs`)

#### Data Fetching — Native Subcommands

| Operation | Command | Pagination |
|---|---|---|
| List issues | `gh issue list --json number,title,state,... -R <repo> --state <s> --limit <N>` | Single `--limit` call (N = page_size × 10). `<s>` is `open` / `closed` when the State filter selects exactly that state, otherwise `all` |
| Get single issue | `gh issue view <iid> --json ... -R <repo>` | N/A |
| List PRs | `gh pr list --json number,title,state,... -R <repo> --state <s> --limit <N>` | Single `--limit` call. `<s>` is `open` / `merged` / `closed` when the State filter selects exactly that state, otherwise `all`. `closed` also returns merged PRs, which the State filter hides. The JSON projection includes `reviewDecision`, `latestReviews`, `mergeable`, `mergeStateStatus`, `reviewRequests` to derive the Approval/Mergeable/Workflow columns |
| Get single PR | `gh pr view <iid> --json ... -R <repo>` | N/A |
| Get PR diff | `gh pr diff <iid> -R <repo>` | N/A |
| Get PR diff over 20,000 lines (only after `gh pr diff` fails with `too_large`) | `gh pr view <iid> -R <repo> --json baseRefOid,headRefOid`, then in the local clone: `git fetch --no-tags origin <baseRefOid> pull/<iid>/head` (skipped when both commits are local) and `git diff <baseRefOid>...<headRefOid>` | N/A |
| List actions/runs | `gh run list --json databaseId,status,... -R <repo> --limit <N>` | Single `--limit` call |
| Current branch's run (Jobs tab) | `gh run list -R <repo> --branch <branch> --limit 1 --json databaseId` | Single call. PR runs carry the head branch |
| List pipeline jobs | `gh run view <id> --json jobs --jq .jobs -R <repo>` (each job's `steps` name its failed step) | Single call |
| Get job trace | `gh run view --job <id> --log -R <repo>` | N/A |
| List releases | `gh release list --json name,tagName,... -R <repo> --limit <N>` | Single call |
| List milestone issues | `gh issue list --milestone <id> --state all --json ... -R <repo> --limit <N>` | Single call |
| List labels | `gh label list --json name,color -R <repo> --limit 100` | Single call (label colors feed the Labels column) |

#### Mutations — Native Subcommands

| Operation | Command |
|---|---|
| Retry run | `gh run rerun <id> -R <repo>` |
| Retry failed jobs (`R`) | `gh run rerun <id> --failed -R <repo>` |
| Cancel run | `gh run cancel <id> -R <repo>` |
| Retry job | `gh run rerun --job <id> -R <repo>` |
| Create release | `gh release create <tag> -R <repo> -t <name> -n <notes>` |
| Update release | `gh release edit <tag> -R <repo> -t <name> -n <desc>` |
| Delete release | `gh release delete <tag> -R <repo> -y` |
| Update milestone state | `gh api -X PATCH repos/{}/milestones/{} -f state=...` |
| Rebase PR | `gh pr update-branch <iid> -R <repo> --rebase` |
| Resolve review thread | not supported — `Backend::set_thread_resolved` errors on GitHub |

#### Data Fetching — Raw API (no native subcommand exists)

| Operation | Endpoint | Why raw API |
|---|---|---|
| List PR review comments | `gh api --paginate /repos/{}/pulls/{}/comments?per_page=100`, plus `gh api graphql --paginate` over `reviewThreads(first:100, after:$endCursor) { pageInfo nodes { isResolved comments(first:1) } }` for the resolution state | `gh pr view --json comments` lacks inline line/position fields needed for diff review, and REST comments carry no resolution state. Every page is read: `page_size` does not bound them. A reply takes its thread's state through `in_reply_to_id`, which always names the thread's first comment |
| Get current user login | `gh api user --jq .login` | Needed to derive "your" workflow/approval state for the MR/PR review columns |
| PR stack positions (Stack column) | `gh api graphql` with one aliased `pullRequest(number: N) { stack stackEntry }` per listed PR | `gh pr list --json` has no stack fields. Runs once per PR-list fetch, only while `App::is_stack_column_needed()` (Stack column shown or grouped by), via `spawn_fetch_pr_stack_summaries` after `MrsFetched` |
| One PR's stack | `gh api graphql` over `pullRequest(number:N) { stackEntry stack { entries(first:50) } }` | No native command. On demand only: the inspector (`dispatch_inspected_pr_stack_fetch`, debounced), `Y` (`view_stack`) and `m` (`merge_mr`), the last two through `with_pr_stack` in `handlers/tabs.rs`, which waits on `App::pending_stack_action`. Results live in `App::pr_stacks` for the session and are re-applied after every list refresh by `App::apply_pr_stacks` |
| Merge a stacked PR (`Backend::merge_pr_stack`) | `PUT /repos/{}/pulls/{}/merge-async` with `merge_method`, then `GET .../merge-async/{uuid}` every 2 s while `pending` | `gh pr merge` calls the GraphQL `mergePullRequest` mutation, which refuses stacked PRs. Merges the PR with every open PR below it, all or nothing (a merge-queue base enqueues them). Single merges of a stacked PR open `ConfirmAction::MergeStack`. Bulk merges look the selection up with `list_pr_stack_summaries` and route stacked PRs here |
| Cancel job | `POST /repos/{}/actions/jobs/{}/cancel` | No per-job cancel in `gh` |
| List runners | `GET /repos/{}/actions/runners?per_page=<N>` (labels become the runner's tags) | No native command |
| List milestones | `GET /repos/{}/milestones?state=all&per_page=<N>` | No `gh milestone` command |
| List notifications | `GET notifications[?all=true]` | No `gh notification` command |
| Mark notification read | `PATCH notifications/threads/{}` | No native command |
| Mark notification done | `DELETE notifications/threads/{}` | No native command |
| Mark all notifications read | `PUT notifications` | No native command |
| Unsubscribe from notification thread | `DELETE notifications/threads/{}/subscription` | No native command |
| List branches | `GET /repos/{}/branches?per_page=<N>` | No native command |
| Create branch | `POST /repos/{}/git/refs` | No native command |
| Delete branch | `DELETE /repos/{}/git/refs/heads/{}` | No native command |
| List environments | `GET /repos/{}/environments?per_page=<N>` | No native command |
| List deployments | `GET /repos/{}/deployments?per_page=<N>[&environment=<name>]` | No native command |
| Stop environment | `GET /repos/{}/deployments?per_page=30&environment=<name>`, then `GET /repos/{}/deployments/{}/statuses?per_page=1` for each, newest first, until one is `success`, then `POST /repos/{}/deployments/{}/statuses` with `state: inactive` on that one | GitHub has no environment stop: deactivating the live deployment (the newest whose latest status is `success`) is its equivalent. The deployment list carries no status, so each one is asked |
| Roll back deployment | `POST /repos/{}/deployments` with `ref: <sha>`, `environment`, `auto_merge: false`, `required_contexts: []` | No native command. Empty `required_contexts` skips GitHub's commit-status check, which would refuse an already deployed SHA whose statuses are not all green |
| List members | `GET /repos/{}/assignees?per_page=100` | No native command |
| Update milestone | `PATCH repos/{}/milestones/{}` | No `gh milestone` command |
| Delete milestone | `DELETE repos/{}/milestones/{}` | No `gh milestone` command |
| List environments | `GET /repos/{}/environments?per_page=<N>` | No native command |
| List deployments | `GET /repos/{}/deployments?per_page=<N>[&environment=<name>]` | No native command |
| List members | `GET /repos/{}/assignees?per_page=100` | No native command |
| Submit review | `POST /repos/{}/pulls/{}/reviews` (`body`, `event`, `comments[]` with `side`/`start_line`) | `gh pr review` has no inline comments; one call carries the whole review |
| Reply to review thread | `POST /repos/{}/pulls/{}/comments` with `in_reply_to` | `gh pr comment` cannot reply to a review comment |

### User-Triggered Mutations

Key handlers in [src/main.rs](src/main.rs) and [src/handlers/](src/handlers/) (`tabs.rs`, `overlays.rs`) run these through `GitlabClient`, which forwards to the matching `Backend` method (`close_issue`, `merge_mr`, `toggle_mr_draft`, `create_release`, `open_in_browser`, …) in `glab.rs` / `gh.rs`. Three calls bypass the backend and are spawned inline in `main.rs` with `tokio::process::Command`:

* `TextInputAction::CreateRelease` builds release notes from `git log` since the last tag, writes them to a temp file and runs `gh|glab release create <tag> -F <file>`.
* "Edit Comment" in the diff view's comment actions runs `gh api repos/{}/pulls/comments/{}` (or `issues/comments/{}`) `-X PATCH` / `glab api projects/{}/merge_requests/{}/notes/{}` `-X PUT`, with the new body passed through `--input`.
* "Delete Comment" runs the same endpoints with `-X DELETE`.

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
| Create milestone | `gh api POST repos/{}/milestones -f title=...` / `glab api POST projects/{}/milestones -f title=...` |
| Create branch | `glab api POST ...repository/branches` / `gh api POST ...git/refs` |
| Delete branch | `glab api DELETE ...repository/branches/{}` / `gh api DELETE ...git/refs/heads/{}` |
| Run pipeline | `gh workflow run` / `glab ci run --mr` |
| Open in browser | `gh issue\|pr\|run view --web` / `glab issue\|mr\|ci view -w` |

> `glab ci run` notes: variables/inputs are passed via the plural `--variables k:v` / `--input k:v` flags (not `--variable`), and `--mr` is only passed when no variables or `workflow_dispatch` inputs are set.

### CLI Command & Flag Reference

The `glab` and `gh` subcommands and flags (`glab v1.114.0` / `gh v2.98.0`) are listed in [docs/cli-reference.md](docs/cli-reference.md). Check it before adding a call, to confirm a native subcommand or flag exists before falling back to `glab api` / `gh api`. It does not list the subcommands of `gh issue`, `gh pr`, `gh release`, `gh repo`, `gh org` or `gh discussion`: confirm those with `gh <command> --help`.

## 7. Development & Quality Standards

* **Error Handling:** Use `anyhow::Result`. Bubble up errors and display them in the UI via `App::show_error(msg)`. Do not `unwrap()` or `panic!()` in UI or event handling code.
* **Test env isolation:** Unit tests that mutate process-global environment variables (config paths via `GLAB_TUI_CONFIG`/`XDG_CONFIG_HOME`, cache dirs) must acquire `config::TEST_ENV_MUTEX` first — env vars are visible to every test thread, and overlapping mutations caused an intermittent Windows CI failure. Never introduce a second ad-hoc mutex for env mutation; reuse the crate-wide one.
* **Secondary-tab smoke matrix:** `tests/e2e/secondary_tabs.rs` drives Runners, Releases, Todos, Milestones, Branches and Environments on GitLab: list every fixture row, filter to one with `/`, run one action on it. The `glab` mock serves `runner list` / `release list` / `todo list` / `milestone list` and the branches / environments / deployments endpoints from `tests/fixtures/{runners,releases,todos,milestones,branches,environments,deployments}.json`. Every list fixture holds two rows so a search visibly narrows the list. `TestSession::press_keys` sends keys one at a time and `TestSession::wait_for_cli_call` waits for a mocked call.
* **Overlay snapshots:** `ui::overlays::tests` renders the merge `SubmitDialog` on a `TestBackend` at 60, 80 and 120 columns and compares it with `src/ui/snapshots/*.txt` (icons normalized to the ASCII set, so the developer's icon config cannot change the result). After an intended layout change, regenerate with `GLAB_TUI_UPDATE_SNAPSHOTS=1 cargo test --bin glab-tui merge_dialog` and review the diff.
* **Dependencies:** Do not add large dependencies (like `reqwest` or `hyper`) for HTTP API calls. The architecture strictly dictates delegating HTTP requests to `gh` and `glab` CLI binaries via `tokio::process::Command` in `GitlabClient`.
* **Format & Lint:** Run `cargo fmt` and `cargo clippy -- -D warnings` before providing code. The CI enforces zero clippy warnings.
* **Tests:** Unit tests live in the library: run `cargo test --lib --bin glab-tui` (`just test`). Running `--bin glab-tui` alone only covers the tests in `main.rs`.
* **MSRV:** The Minimum Supported Rust Version is `1.88`. Edition 2024
  requires Rust 1.85+, but current transitive dependencies raise the floor
  to 1.88 (see `.github/workflows/msrv.yml`). Ensure code is compatible.

## 8. Release Process (Local-First)

Releases are prepared, documented, and distributed from a maintainer's machine via a single orchestrator, `scripts/release.sh`. CI is only responsible for building the cross-platform release binaries. The demo GIFs must be recorded locally because `glab-tui` shells out to `gh`/`glab`, and CI tokens lack the permissions for a realistic recording.

Run `scripts/release.sh [patch|minor|major|nightly]` (default `patch`) and the script walks the full release:

> **Nightly Pre-Releases:** `scripts/release.sh nightly` (or tags matching `vX.Y.Z-nightly[-suffix]`) runs a stripped-down flow that skips `Cargo.toml` bumping, docs regeneration, prepare PRs, demo GIFs, review gates, Homebrew/Scoop manifest syncs, and crates.io publishing, while still building matrix binaries and updating GitHub release notes.

1. **Preflight** — checks `gh`/`opencode`/`cargo`/`jq`/`vhs`/`ttyd`/`ffmpeg`/`unzip`, `gh auth`, JetBrainsMono Nerd Font, and push access to both manifest repos (`rcieri/homebrew-glab-tui`, `rcieri/scoop-glab-tui`); exits non-zero with a clear message if a prerequisite is missing. Long-running steps run under the script's `spinner`/`progress_bar` helpers (animated spinner with captured logs, auto-disabled when not a TTY), and phases are numbered `1/7` … for progress reporting.
2. **Prepare** — computes the next tag from `git describe --tags`, bumps the crate version in `Cargo.toml`, prompts for the AI tool (`agy` or `opencode`) and model unless configured via `AI_TOOL`, `AGY_MODEL`, or `OPENCODE_MODEL`, regenerates `CHANGELOG.md`/`AGENTS.md`/`README.md` via the selected AI tool, rebuilds the demo GIFs against an authenticated `gh`, and opens a `chore: prepare release vX.Y.Z` PR.
3. **Review gate** — pauses for the maintainer to review the PR (CI checks run in the background); the script continues on Enter.
4. **Merge & tag** — squash-merges the PR with `--auto`, tags the merge commit and pushes `vX.Y.Z`. `.github/workflows/release.yml` builds the 11-target binary matrix (8 Linux + 3 non-Linux) and uploads them to the GitHub release.
5. **Wait for build** — polls the release until every required asset exists (timeout: `RELEASE_WAIT_MIN`, default 45 min). The required-asset list in `REQUIRED_ASSETS_STATIC` covers the fixed name patterns; the dynamic Ubuntu-latest build is required to produce at least two `ubuntu-<VERSION_ID>` assets whose VERSION_ID differs from the 22.04 / 24.04 baselines. The script protects against unbound variables and off-by-one asset-count math under `set -u`, and it can resume from a chosen phase (`--phase`) if an earlier step failed or was interrupted.
6. **Post-release** — generates `RELEASE_NOTES.md` via the selected AI tool (entries attribute their contributors as `(thanks @username)` and a `**Contributors**` section lists all `@username` handles since the previous tag), and updates the release body on GitHub.
7. **Publish** — pushes the Homebrew formula (`rcieri/homebrew-glab-tui`), Scoop manifest (`rcieri/scoop-glab-tui`), Docker image to GHCR, and publishes the crate to crates.io (skipped for nightly).

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
- **`src/utils/update.rs`** (`linux_asset_candidates`) — mirrors `install.sh` for the in-app self-updater; queries the GitHub release's asset list through `curl` and picks the first match.
- **Homebrew formula** (`update_homebrew` in `release.sh`) — pins the fully static musl builds for Linux (x86_64 and arm64), which run on any Linux distro regardless of glibc version, and native macOS binaries.

Every install path verifies the downloaded archive against the release's `<asset>.sha256` (generated by `release.yml`) before extracting it, and refuses to install when the checksum asset is missing or does not match: `install.sh` (`verify_checksum`, via `sha256sum` or `shasum`), `install.ps1` (`Get-FileHash`) and the self-updater (`verify_archive_checksum`). Checksum lines are `<hex>  <name>`, or `<hex> *<name>` for the Windows zip.

Scoop's manifest (`rcieri/scoop-glab-tui`) is unaffected — Windows keeps the fixed `glab-tui-windows-amd64.zip` name.

During `Prepare` and `Post-release`, `release.sh` prompts to select the AI tool (`agy` or `opencode`) and model for regenerated docs and release notes; set `AI_TOOL` and `AGY_MODEL` / `OPENCODE_MODEL` to skip prompts.
