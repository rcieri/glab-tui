# Changelog

All notable changes to this project will be documented in this file.

## [0.7.1] - 2026-07-26

### Added
- **Mouse support** — Full mouse interaction across the entire UI: sidebar tab switching, table row selection, and scroll-based navigation. All overlays and modals (confirm popup, selector, edit menu, date picker, column configure, help, save submenu) respond to click and scroll events. Z-order overlay dispatch via new `OverlayKind` enum and `overlay_stack` tracking (#205, #216, #228).
- **CLI subcommands** — New `doctor`, `clean-cache`, `cache`, `open`, and `repos` subcommands via `clap` derive parser (#178, #237):
  - `doctor` — validates glab/gh/git availability, config, cache, repo context, and terminal.
  - `clean-cache [--dry-run]` — prunes stale cache entries.
  - `cache` — lists cached files with sizes and timestamps.
  - `open <entity> <id>` — opens entities in the browser via glab/gh.
  - `repos` — lists recently-used repositories with validity markers.
  - All output color-coded with ANSI SGR sequences.
- **Bulk editing** for issues and merge requests — Multi-select via `Space` (default), then press `e` to bulk-edit labels, assignees, and milestone across all selected items. Selection state tracked per-tab with `HashSet<u64>`, cleared on tab switch or `Esc`. Cursor item automatically included in selection for intuitive single/bulk flow (#215, #230).
- **Cache repo attributes** — Labels, members, and milestones cached with typed fields in `ProjectCache`, pre-fetched at startup and on explicit refresh. Selectors populate from cache immediately, eliminating loading spinners. Auto-refresh gated to high-churn tabs only (Issues, MRs, Pipelines, Jobs, Todos) (#174, #240).
- **Create MR/PR from issue** — Press `m` in the Issues tab to open an EditMenu pre-filled with issue data. Source branch auto-generated from issue IID and slugified title; description includes `Closes #N` reference (#185, #229).
- **`UiConfig`** — Configurable `sidebar_width` (default 22), `sidebar_visible` (default true), and `terminal_pane_visible` (default true) in config TOML (#202, #223).
- **`BackendKind` enum** — `BackendKind::{GitLab, GitHub}` with `App::backend_kind()` helper for cleaner host-specific branching (#206).
- **Semantic theme tokens** — New label palette tokens (`label_text`, `label_bg`) and `clean` preset theme (`src/themes/clean.toml`) (#207).
- **Unified `Modal` component** — `modal_block()` and `modal_area()` helpers in `src/ui/modal.rs` providing standard double-border styling and centered sizing. Edit menu and selector overlays migrated (#193, #210).
- **Auto-dismiss error toast** — `app.error_message` now renders as a red-bg bottom-centered toast, auto-dismissing after 4 seconds (#211).
- **`FieldType` enum** — Typed `Field` constructors (`Text`, `MultiSelect`, `Date`, `Toggle`, `Ref`) for EditMenu migration. Edit menu titles use backend-specific labels (MR/PR) (#194, #225).
- **Edit menu mnemonics** — Footer hints show mnemonic keybindings per field (e.g., `t` for Title, `d` for Description) (#201, #220).
- **Pipeline dialog gating** — "Merge Request Pipeline" shown only for GitLab, "Workflow File" only for GitHub (#199, #222).
- **Pipeline search/group-by** — Name, Event, SHA, and Actor columns now support fuzzy search and group-by in the pipelines tab (#198, #221).
- **View related pipelines** — Press `P` in the MR tab to switch to Pipelines tab and auto-select the head pipeline (#209, #212).
- **Start manual jobs** — Press `S` (GitLab only) to start manual pipeline jobs via `POST /projects/:id/jobs/:job_id/play` (#208, #214).
- **Todos tab enhancements** — Badges, `Updated` column, `time_ago` formatting, fuzzy search support (#158, #239).
- **Milestone progress bar** — Visual progress bar replaces percentage text in milestones table column (#161, #238).
- **`clap` dependency** — Added `clap = { version = "4", features = ["derive"] }` for CLI argument parsing.

### Fixed
- **Help overlay completeness** — Added all missing keybindings (global_search, save_view, enter_pipeline, milestones edit/close/reopen/delete, releases edit/delete, terminal toggle_wrap). Upgraded hardcoded key references to config-backed lookups (#236).
- **Milestone cache staleness** — `milestone_issues_cache` cleared on `MilestonesFetched` event so completed/closed counts are re-fetched after refresh (#231, #235).
- **GitHub branch metadata** — `list_members` now fetches the real `default_branch` from repo info API instead of marking the first branch as default; `web_url` populated for GitHub branches (#171, #235).
- **Quote extraction** — `extract_quotes()` now only strips matching outer quote pairs, preserving natural quotes inside titles (e.g., `Fix "bug" in parser`) (#232, #234).
- **Mouse click targeting** — Selector click handler parameterized for search/footer height differences; search bar presence correctly computed from `field_type` for ColumnFilter overlay.
- **Confirm popup mouse integration** — Raw shell commands replaced with `GitlabClient` backend methods; unconditional event-sending gated behind `Ok` branches; optimistic UI updates added before async calls.
- **Shift+P/S keybindings** — Added bare `KeyCode::Char('P'/'S')` checks before `keybinding_matches` to handle crossterm's Shift-modifier reporting (#227).
- **Responsive column widths** — `col_w()` helper caps widths for narrow terminals: 80-col terminals now get readable table columns (#203, #224).
- **GitHub adaptation** — Confirm popup labels use backend-specific terms (MR/PR); pipeline detail labels adapted for GitHub (Run ID, Jobs Status) (#200, #219).
- **Pipeline keybindings** — `view_related_pipelines` and `start_job` properly wired; `start_job` default changed from `p` to `S` to avoid shadowing `enter_pipeline` (#227).
- **GitHub job stages** — Pipeline stage column uses workflow name for GitHub instead of stage ID (#213).
- **Terminal output consistency** — Milestone close/reopen operations now use `CLOSING MILESTONE` / `REOPENING MILESTONE` labels matching the convention used by all other mutation operations (#217, #218).
- **GitLab milestone fixes** — Various milestone command parsing and state sync corrections.

### Changed
- **Hints removed** — All inline edit menu hints, modal footer bars, and inline command hints removed. Users now rely on the help overlay (`?`). Removed unused `hint_text` theme token (#180, #226).
- **Selector overlay** — Refactored to use new `Modal` component; footer adapted for multi-select vs single-select.
- **Pipeline start job** — Default keybinding changed from `p` to `S` (was shadowing `enter_pipeline`).
- **Auto-refresh scope** — 60-second timer gated to high-churn tabs only (Issues, MRs, Pipelines, Jobs, Todos); other tabs skip the timer.
- **Optimistic local updates** — Milestone close/reopen and delete operations update local state immediately, skipping full re-fetches where possible.

### Dependencies
- Add `clap` `4.6` (with `derive` feature)
- Bump transitive `anstream`, `anstyle`, `clap_builder`, `clap_derive`, `clap_lex` (via clap addition)

---

## [0.7.0] - 2026-07-20

### Added
- **Backend trait system** — Extracted a unified `Backend` trait (`src/backend/mod.rs`) with dedicated `GlabBackend` (`src/backend/glab.rs`) and `GhBackend` (`src/backend/gh.rs`) implementations, replacing the old `src/gitlab/` translation layer. The trait provides ~40 methods covering all API interactions with proper async dispatch (#165).
- **Domain model layer** — Consolidated domain types into `src/domain/` with clean modules for `branches`, `deployments` (Environment & Deployment), `issues`, `milestones`, `mr`, `notifications`, `pipelines`, `releases`, and `runners`, each with serde-powered structs and dedicated list/get helpers.
- **crates.io publishing** — Package renamed to `glab-tui-crate` (binary stays `glab-tui`) for publishing on crates.io; `cargo install glab-tui-crate` now works.
- **Homebrew & Scoop distribution** — Added `.gitmodules` for `homebrew-glab-tui` and `scoop-glab-tui` manifest repos, with CI automation to update formulas on release.
- **`async-trait` dependency** — Added `async-trait = "0.1.89"` to support the new async `Backend` trait.

### Fixed
- **is_draft detection** — Fixed draft status not being correctly parsed from GitLab MR responses.
- **GitLab nerd font icons** — Replaced custom nerd font GitLab icon with standard FontAwesome icon for better cross-terminal compatibility.
- **Repository argument encoding** — Removed spurious URL encoding from `glab` native subcommand `-R` arguments, fixing entity fetch failures when project paths contain special characters (#181).
- **Non-blocking CLI commands** — Reverted to non-blocking subprocess spawning to prevent UI freeze during CLI calls (#183).
- **Diff rename handling** — Fixed diff parsing when files are renamed, ensuring renamed file diffs are displayed correctly (#184).
- **Terminal output corruption** — Resolved extraneous printouts corrupting the terminal display (#173).
- **Trace view regression** — Fixed the job trace viewer that was not displaying output (#172).
- **Code injection mitigation** — Applied escaping fixes for CodeQL security alert #25 (shell argument injection) (#168).
- **macOS CI hangs** — Prevented `cargo test` from hanging on macOS runners by fixing PTY lifecycle.
- **Duplicate release notes** — CI now generates release notes only once in the matrix build.

### Changed
- **Architecture overhaul** — Removed the entire `src/gitlab/` module tree (client.rs, issues.rs, mr.rs, pipelines.rs, runners.rs, releases.rs, milestones.rs, notifications.rs, branches.rs, deployments.rs). Replaced with `src/backend/` (trait + per-host impls) and `src/domain/` (data types + logic). The `GitlabClient` now lives in `src/domain/client.rs` and delegates to the backend trait.
- **Package identity** — Cargo package renamed from `glab-tui` to `glab-tui-crate` to free the `glab-tui` name for the binary. The install command changes to `cargo install glab-tui-crate`.
- **Release automation** — CI `release.yml` now triggers manifest updates on Homebrew and Scoop submodules after a release publish.

### Dependencies
- Bump `toml` from `0.8` to `1.1.3+spec-1.1.0`
- Bump `tokio` from `1.52.3` to `1.53.0` (minor-updates group)
- Add `async-trait` `0.1.89`
- Bump `the patch-updates group` with 4 dependency updates
- Bump `docker/login-action`, `docker/build-push-action`, `actions/checkout` (CI)

---

## [0.6.0] - 2026-07-11

### Added
- **Nerd Font icon system** — All tab titles, status badges, labels, and UI indicators can now render nerd font icons. Uses hardcoded nerd font defaults that are not user-configurable. (original #156)
- **Pipeline / Action status in MR/PR pane** — The MR/PR details panel now displays the pipeline (GitLab) or workflow action (GitHub) status graphically with stage dots, adapting terminology to the remote host (#144, #126).
- **Confirmation prompts for destructive actions** — Closing issues, closing MRs, merging MRs, and deleting branches/releases/milestones now show a confirmation dialog before executing. Reduces accidental destructive operations (#141, #146).
- **Entity deletion** — Issues and merge requests can now be deleted directly from the TUI. New `delete_entity` keybinding added to the issues and MRs keybinding tables (#150).
- **Fetchable selectors for free-form fields** — Branch inputs, environment selectors, and other free-form fields were upgraded to fetchable `Selector` lists with fuzzy matching, matching the selector UX used elsewhere (#145).
- **Improved cache persistence** — Selector items and milestone issues are now persisted to disk cache alongside API payload data, reducing redundant network fetches on tab switches (#147).

### Fixed
- **Column widths bounded** — All table columns now use fixed `Length` constraints instead of `Fill`, guaranteeing every column stays within the terminal viewport. Affects issues, MRs, pipelines, jobs, runners, releases, todos, milestones, branches, and environments (#125, #155).
- **Config auto-create removed** — `Config::load()` no longer writes `config.toml` on startup when missing. The file is now created only by an explicit `save_view` / `save_layout` action, aligning with the documented behavior (#74daa1b).
- **Homebrew installation** — Fixed wrong version pinning and corrected the installation method in the Homebrew formula (#148).
- **E2E test deadlocks** — Resolved deadlocks in parallel PTY spawning by preparing process allocations before forking; refactored `test_cascading_repo_override` to use `Pty::spawn` (#fcb16b3, #4118699).
- **Install script asset matching** — `install.sh` now matches exact asset names to avoid downloading multiple release URLs (#c6acf24).

### Changed
- **Tab titles** — Now include nerd font icons (e.g., ` Issues`, ` PRs`, ` Pipelines`/` Actions`). Falls back gracefully on non-nerd-font terminals via config override.
- **Pipeline column in MRs** — Renamed to "Pipeline" on GitLab, "Action" on GitHub, gated by host detection.
- **Confirmation UX** — `ConfirmAction` enum expanded with `DeleteBranch`, `DeleteIssue`, `DeleteMr`, `CloseIssue`, `CloseMr`, `MergeMr` variants; new `confirm_popup_selected_yes` state field.

### Dependencies
- Bump `docker/login-action` from 3 to 4 (CI)
- Bump `docker/build-push-action` from 6 to 7 (CI)

---

## [0.5.0] - 2026-07-07

### Added
- **Save view configurations** — Inline page size editing, multi-page fetching, and config persistence validation in the configuration view (#142).
- **Milestone tracker & editing** — Support editing milestone fields, color-coded progress bars, caching milestone issues to avoid redundant network fetches, and dynamically rendering milestones column headers (#106, #110, #140).
- **Release creation & editing** — Support structured release creation and editing via `EditMenu`, along with commit metadata and assets link rendering in the release preview (#106, #110).
- **Issue, MR, and PR description templates** — Choose from description templates when creating new issues or merge/pull requests (#123).
- **Fuzzy matching improvements** — Upgrade pipelines, jobs, and branch/workflow selectors to use `SkimMatcherV2` fuzzy matching, matching the merge request list (#103).
- **Run pipeline workflow/branch selectors** — Autocomplete and search local/remote branches and CI configuration files when triggering pipelines (#103).
- **Packaging and manifests** — Add Docker container support, Scoop, and Homebrew formula packages with manifest auto-bumping utilities (#107).

### Fixed
- **GitHub PR Ready** — Use correct `gh pr ready` subcommand instead of the invalid `gh pr edit --ready` flag when marking GitHub PRs ready (#103).
- **Runner details panel** — Hide details pane if not applicable/empty (#109).
- **UTF-8 characters in labels** — Prevent panic on label truncation with multi-byte characters by ensuring truncation snaps down to character boundaries (#93).

### Changed
- Reordered Date column to the left of Release Name in the releases table.
- Moved collapse/expand matrix hint from jobs pane to help view.

---

## [0.4.0] - 2026-07-02

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

## [0.3.0] - 2026-06-13

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
