# Changelog

All notable changes to this project will be documented in this file.

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
- **Columns Config Modal Overlay**: Replaced the sidebar panel with a centered columns checkbox toggler popup overlay, triggered by pressing `Tab` or `t`.
- **Hashed Multi-colored Labels**: Implemented individual label coloring based on a hashed color scheme in the Issues and Merge Requests tables, preserving fuzzy-search query highlights.
- **Runner Diagnostics Dashboard**: Integrated simulated performance statistics, utilizing gauges, utilization percentages, queue depths, and average queue wait times.

### Changed
- Expanded the Navigation sidebar pane to take full vertical height when columns config panel is hidden.
- Updated the Keyboard Shortcuts help menu to reflect the new `Tab`/`t` column toggle binding.
- Auto-formatted and cleaned up import structures across all code modules to fix compiler lint warnings.
