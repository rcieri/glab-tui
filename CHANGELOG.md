# Changelog

All notable changes to this project will be documented in this file.

## [2.3.1] - 2026-07-03

### Fixed
- **Multi-byte label panic** — `render_labels_cell` no longer panics when truncating labels containing multi-byte characters such as emojis (👕, 🌟) or accented Unicode. Introduced `floor_char_boundary()` to safely snap slice indices to valid UTF-8 boundaries (#90, #93).

### Maintenance
- **CI release automation** — added automated release workflow to streamline tagging and publishing.
- **opencode agent setup** — configured `opencode` CI agent for assisted code reviews via `pull_request_review_comment` triggers; upgraded `actions/checkout` to v7 with `persist-credentials: false`.

## [2.3.0] - 2026-07-02

### Added
- **TOML config file** — `~/.config/glab-tui/config.toml` (or `$GLAB_TUI_CONFIG`) auto-generated on first run with all options documented inline.
- **Theme system** — choose from six bundled presets (`default`, `tokyo-night`, `gruvbox`, `nord`, `catppuccin-mocha`, `dracula`) via `theme_preset` in config; full per-color overrides supported under `[theme]`.
- **Custom theme files** — place additional `<name>.toml` files in `~/.config/glab-tui/themes/` to create and share your own themes.
- **Fully configurable keybindings** — every action across all panes is remappable in `config.toml` under `[keybindings.global]`, `[keybindings.issues]`, `[keybindings.mrs]`, `[keybindings.pipelines]`, and `[keybindings.releases]`.
- **Interactive calendar date picker** — press `Enter` on Due Date / Start Date in the edit menu to open an inline calendar widget; navigate with `h`/`l` (month) and `j`/`k` (day).
- **Due Date column in Issues** — new `Due Date` column in the issues table; hidden automatically when connected to GitHub.
- **Start Date column in Milestones** — new `Start Date` column; hidden automatically when connected to GitHub.
- **Runner details panel** — selecting a runner now opens a structured side-panel showing Runner ID, description, status, tags, and live job/queue metrics.
- **Per-pane column config in TOML** — set default visible columns, column filters, and group-by column persistently via `[issues]`, `[mrs]`, etc. sections in `config.toml`.

### Fixed
- **Small terminal handling** — gracefully degrade layout when the terminal is too small rather than panicking.
- **Pipeline job cache persistence** — pipeline jobs are now saved to and restored from disk cache.
- **Selector "Create New" entry** — always appears at the top of the list even when a filter is active.
- **Empty description on GitHub** — creating issues/MRs on GitHub no longer inserts a blank description field.
- **GitLab-only fields hidden on GitHub** — due date, weight, confidential, and start-date fields are excluded from GitHub issue/MR forms.
- **`Ctrl+E` to open editor** — unified shortcut to open `$EDITOR` for description fields across all edit menus.

### Changed
- **Config architecture refactor** — keybindings, column visibility, and themes were extracted from hard-coded constants in `ui.rs` into a dedicated `config.rs` module; `FormattingConfig` struct removed.
- **Keybinding matching** — all hardcoded `KeyCode::Char` match arms replaced with `keybinding_matches()` helper, enabling full runtime override from `config.toml`.
- **Edit menu UI polish** — edit popup border and title rendered in focused accent color; field values colored to match the details pane; date picker styled to match the details pane theme.
- **`cancel` pipeline keybinding** — default changed from `c` to `d` (resolves conflict with `download_artifact`, which was also `d`).
- **Runner tab layout** — rebuilt runner details rendering: removed old flat list in favor of a structured two-pane layout (table + details panel).

### Dependencies
- Bump `anyhow` from `1.0.98` to `1.0.103`
- Bump `ratatui` from `0.30.1` to `0.30.2`
- Bump `actions/checkout` from 4 to 7 (CI)
- Bump `actions/stale` from 9 to 10 (CI)

---

## [2.2.0] - 2026-06-13

### Added
- **Code review system** with draft comments, multi-line comments, and code suggestions in diff view.
- **Syntax highlighting** in diff/patch viewer using `syntect` (`base16-eighties.dark` theme).
- **Side-by-side diff layout** — toggle between unified and side-by-side with `d` in diff view.
- **Value-based column filtering** — filter table rows by specific column values via configure popup.
- **Column grouping & ordering** — merge grouping into configure view with ascending/descending sort.
- **Show read notifications** — toggleable via `show_read` parameter on todos/notifications tab.

### Fixed
- **ID sorting** — compare ID columns numerically instead of lexicographically.
- **Diff contextual naming** — show "Pull Request" or "Merge Request" based on host, not both.
- **Review pane focus** — focus files pane on Esc, confirm drafts when closing diff.
- **Line range selection** — correct line range and comment target on side-by-side diff.
- **UI rendering alignment** — align with sorted lists, resolve borrow checker conflict.
- **Row selection in grouping view** — restore normal selection, editing, and column toggling.
- **Group map rebuild** — rebuild group map and update filters when toggling columns.
- **Layout scaling** — fix layout scaling issues (#71).
- **POST for retry/cancel** — use `-X POST` for retry and cancel endpoints (#49).
- **Editor-based comments** — fix comment creation via editor (#38).
- **`--file-path` flag** — use for `glab mr note create`.
- **Description template** — hide from EditMenu, load on demand when editing.
- **Notification command args** — fix `gh api notifications?all=true` argument passing.

### Changed
- **Refactored column configure popup** — replaced old FILTERS section with unified COLUMNS, GROUP BY, and ORDER sections.
- **Contextual column renaming** — milestones: rename `IID` column to `ID`.
- **Cache directory migration** — moved from `~/.glab-tui-cache` to `~/.cache/glab-tui`.
- **Extended cache persistence** — now saves `enabled_columns`, `group_by_column`, `group_ascending`, `column_filters`.
- **Event refactoring** — `DiffFetched` changed from tuple struct to named fields with `comments` payload.
- **GitHub endpoint translation** — added `/retry`→`/rerun`, `/notes`→`/comments` maps; pull request comment JSON translation.

### Dependencies
- Bump `ratatui` from `0.29.0` to `0.30.1`
- Bump `crossterm` from `0.28.1` to `0.29.0`
- Bump `chrono` from `0.4.44` to `0.4.45`
- Add `syntect` v5 with `default-fancy` features

### CI/CD
- Bump `codecov/codecov-action` from v4 to v7
- Bump `actions/upload-artifact` from v4 to v7
- Bump `actions/labeler` from v5 to v6
- Bump `amannn/action-semantic-pull-request` from v5 to v6
- Bump `softprops/action-gh-release` from v2 to v3

## [0.2.1] - 2026-06-07

### Added
- **New MR creation from issue**: Branch selector with auto-create, slug-based source branch, auto-push before PR creation.
- **Reopen/close issues and MRs.**
- **Persistent offline caching** for all data tabs (issues, MRs, pipelines, runners, releases, todos, milestones).
- **1-minute auto-refresh** of the active tab.
- **Inline command logs** and a scrollable **Terminal tab** showing CLI command history.
- **Creation forms** for issues, MRs, and pipeline triggers.
- **Edit menus** with `$EDITOR` integration for descriptions and freeform fields.
- **Pipeline/JD job trace viewer** with scroll support and open-in-editor.
- **Self-updater** via `--update` / `-u` flag (GitHub releases).
- **Security audit** CI workflow (`cargo audit`).

### Fixed
- UI table overflow: main content pane now respects the terminal pane's reserved height.
- Windows: `NamedTempFile` handle locking — editor temp files use `into_temp_path()` to release the handle before spawning.
- Windows: removed `cmd /c` wrapper from editor spawn — Rust's command-line builder was double-escaping path quotes.
- GitHub mode: labels, milestones, description editing, and PR-from-issue creation.
- Fuzzy search: disabled fuzzy matching on all tabs except MRs; "Create New" option moved to top of selector.
- Self-updater: works correctly on both Linux and Windows.
- Various UI panics on empty lists, ellipsis padding, and rendering edge cases.

### Changed
- Refactored editor integration: extracted `Cli` / `UpdateCmd` helper structs for clean GitHub/GitLab CLI flag mapping.
- CI workflows now trigger only on `main` (dev branch triggers removed post-merge).

## [0.2.0] - 2026-06-03

### Added
- **Dual-Engine GitHub & GitLab Support**: glab-tui now automatically detects if a project is hosted on GitHub or GitLab, translating TUI views and actions to `gh` or `glab` CLI commands under the hood.
- **CLI Configuration Options**: Added option flags `--repo <namespace>` (to override project context) and `--dir <path>` (to target a custom repository directory) on launch.
- **Columns Config Modal Overlay**: Replaced the sidebar panel with a centered columns checkbox toggler popup overlay, triggered by pressing `Tab` or `,`.
- **Hashed Multi-colored Labels**: Implemented individual label coloring based on a hashed color scheme in the Issues and Merge Requests tables, preserving fuzzy-search query highlights.
- **Runner Diagnostics Dashboard**: Integrated simulated performance statistics, utilizing gauges, utilization percentages, queue depths, and average queue wait times.

### Changed
- Expanded the Navigation sidebar pane to take full vertical height when columns config panel is hidden.
- Updated the Keyboard Shortcuts help menu to reflect the new `Tab` / `,` column toggle binding.
- Auto-formatted and cleaned up import structures across all code modules to fix compiler lint warnings.
