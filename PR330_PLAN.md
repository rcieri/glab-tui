# PR #330 Plan: Full Group/Org-Level Browsing

This document captures the implementation plan for landing full
group/org-level browsing in `glab-tui`, building on the backend foundation
laid by PR [#330](https://github.com/rcieri/glab-tui/pull/330), which
closes [#179](https://github.com/rcieri/glab-tui/issues/179).

The plan is broken into nine phases. **Phase 0 is a hard prerequisite** for
every other phase — without it the PR does not merge cleanly.

## Phase 0 — Merge main into PR branch (PREREQUISITE)

**Why first:** PR #330 currently reports `mergeable: CONFLICTING`,
`mergeStateStatus: DIRTY`. Its HEAD (`6d319f2`) is based on `cebb461`;
main has moved to `4e811c9`. Eleven commits on main since fork touch
files this PR also touches (`src/backend/mod.rs`, `src/backend/glab.rs`,
`src/backend/gh.rs`, `src/main.rs`, `src/app.rs`, `src/config.rs`,
`src/domain/client.rs`, `src/domain/mr.rs`, `src/entity_editor.rs`,
`src/handlers/overlays.rs`, `src/handlers/tabs.rs`).

**Steps:**

1. `git fetch origin feat/179-gitlab-group-mode`
2. `git checkout feat/179-gitlab-group-mode`
3. `git merge --no-ff origin/main` (expect conflicts; never rebase a
   shared branch)
4. Resolve the four files PR #330 modifies vs main moved:
   - **`src/backend/mod.rs`** — most likely to conflict; main
     added/changed trait methods (`bulk_*`, rate-limit, `IssueUpdate` /
     `MrUpdate` shapes).
   - **`src/backend/glab.rs`** — main's #382 added rate-limit pacing
     + batched edits; PR #330 inserts `list_group_*` stubs in the same
     neighborhoods.
   - **`src/backend/gh.rs`** — same risk; main has #348 / #384
     batched-edits changes.
   - **`src/domain/client.rs`**, **`src/domain/mr.rs`** — main added
     bulk wrappers + MR fields; PR #330 doesn't touch these but
     adjacency may shift line numbers (no conflict in `diff3` terms).
5. After resolving: `cargo fmt`, `cargo clippy -- -D warnings`,
   `cargo test`. Full test suite on ubuntu/macos/windows runners.
6. Push and let CI re-run on the merge commit.

**Conflict-resolution principle:** keep PR #330's `list_group_*` trait
methods + `parse_group` helper exactly as written; for every other
hunk, prefer main. Do NOT rename `list_group_*` into `list_x(&Scope)`
here — that comes in Phase 1.

## Phase 1 — `Scope` enum + Backend trait refactor (foundation)

**New file:** `src/scope.rs`

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    Repository(String),  // "group/project" (or nested subgroups)
    Group(String),       // "group" or "group/subgroup"
}
impl Scope {
    pub fn as_str(&self) -> &str;          // unwraps to the inner String
    pub fn is_group(&self) -> bool;
    pub fn is_repository(&self) -> bool;
    pub fn cli_repo_arg(&self) -> Option<(&str, &str)>;   // Some(("-R", repo))
    pub fn cli_group_arg(&self) -> Option<(&str, &str)>;  // Some(("--group", g))
    pub fn api_path_prefix(&self) -> String; // "projects/{url-encoded}" | "groups/{url-encoded}"
    pub fn display(&self) -> String;       // "group/project" | "Group: group"
}
```

**Refactor `Backend` trait** (`src/backend/mod.rs`): change every
`project: &str` parameter on ~40 methods to `scope: &Scope`. Provide
blanket default impls where the operation makes sense in both scopes;
force overrides where it doesn't (e.g. `create_branch`).

**Implement scope arms in `glab.rs`:** existing methods switch on
`Scope::Repository` → `-R repo`, `Scope::Group` → `--group group` (where
supported) or raw API path otherwise. New `list_group_*` impls from #330
either fold into `list_x(scope)` or stay as private helpers.

**Implement scope arms in `gh.rs`:** repo arm unchanged; group arm
issues `gh api orgs/{org}/issues?filter=all&state=...&per_page=...`
and `gh api orgs/{org}/pulls?state=...&per_page=...`. For Actions:
aggregate per repo via
`gh api repos/{org}/{repo}/actions/runs?per_page=...` after
`gh api orgs/{org}/repos --jq '.[].full_name'` (cached).

**Domain wrappers** (`src/domain/{client,issues,mr,pipelines,...}.rs`):
every wrapper that takes `project_path: &str` takes `scope: &Scope`
instead. 10 files affected.

## Phase 2 — Per-item project path + drill-down

**Embed `project_path: Option<String>` on:**

- `Issue` (`src/domain/issues.rs`) — populated when scope is `Group`
- `MergeRequest` (`src/domain/mr.rs`) — `#[serde(default)]`, populated
  from `references.full` (GitLab) or `head.repo.full_name` (GitHub)
- `Pipeline` (`src/domain/pipelines.rs`) — populated from
  `project.path_with_namespace` (GitLab) or `repository.full_name`
  (GitHub)

**New `App` method:** `fn drill_into(scope: Scope)` — switch active
scope to a `Scope::Repository(path)` from the selected item, then
re-fetch the active tab. `Enter` key in group scope → drill.

## Phase 3 — App state, cache, CLI

**`src/app.rs`:**

- Replace `pub project_context: String` with `pub scope: Scope`
- Add `pub prev_scope: Option<Scope>` so `Esc` returns from drill-down
- New helper `App::scope_label() -> String` for header rendering
- New helper `App::tab_supported_in_scope(tab) -> bool`

**`src/config.rs`:**

- Add `default_scope: Option<String>` to `Config` (parsed as
  `Scope::Group`)
- Persist on save, apply on load

**`src/cli.rs`:**

- Add `#[arg(short = 'g', long = "group")] group: Option<String>` to
  `Cli`
- Wire to `Scope::Group` in `main.rs`
- `run_open_in_browser` adjustments (no group-level browser open in
  v1; per-item only)

**`src/utils/cache.rs`:**

- `cache_file_name(scope)` produces `<group_or_repo>.json` from
  `Scope::as_str()`
- `load_cache(scope)` and `save_cache(scope, &cache)` take `&Scope`
- Per design choice: shared cache across scopes — same filename when
  same path
- `clean_cache` updates to enumerate both

**`src/domain/client.rs`:**

- `GitlabClient` field `scope: Scope`; constructor takes `&Scope`
- All wrappers forward `&self.scope`

## Phase 4 — Fetch + tab gating

**`src/fetch.rs`:**

- `spawn_refresh_active_tab(client, scope, tab, tx)` takes `&Scope`
- For each tab, branch on scope:
  - **Issues / MRs / Pipelines / Milestones / Releases:** use group
    endpoint
  - **Branches / Environments / Deployments / Runners / Jobs:** send
    `Event::FetchFailed(tab, "Not available in group scope")` and skip
  - **Todos:** unchanged (user-global)
- `spawn_fetch_repo_attributes` becomes a no-op in group scope (no
  project-level labels/members)

**`src/event.rs`:** add `Event::GroupScopeUnavailable(Tab, String)` for
the disabled tabs (reuses existing `FetchFailed` shape — no new variant
needed).

## Phase 5 — UI: header badge, scope overlay, tab rendering

**`src/ui/mod.rs` (header):** replace `app.project_context` span with
`app.scope_label()`; add `[GROUP]` mode badge in `render_mode_indicator`
(mirrors the existing `[SELECT]` / `[NORMAL]` badges; new theme token
`badge_group_bg`).

**`src/ui/tabs.rs`:** for each render function, check
`App::tab_supported_in_scope`. If not, render a centered message
("`{Tab}` not available in group scope. Press `Ctrl+s` to switch back
to a repository.") instead of the empty table.

**Group-listed tables** show a leading `Project` column (always visible
in group scope) so users see which project each item belongs to —
values come from the new `project_path` field.

**`src/handlers/overlays.rs` (`handle_switch_repo`):** rename to
`handle_switch_scope`. Overlay now has two sections — "Repositories"
and "Groups". Groups section sourced from a new `recent_groups.json`
cache (mirrors `recent_repos.json`), seeded by `--group`/config/this
session. `Enter` picks the highlighted entry; `Tab` jumps sections.

**Drill-down handlers** (`src/handlers/tabs.rs`): on `Enter` in group
scope, call `app.drill_into(Scope::Repository(item.project_path.clone()
.unwrap()))`. On `Esc` (when `prev_scope == Some(Group)`), pop back to
group scope.

## Phase 6 — GitHub parity

- `GhBackend::list_issues(Scope::Group)` →
  `gh api orgs/{org}/issues?filter=all&state=open&per_page={n}`
  - Filter out items where `pull_request` is non-null (those are PRs)
  - Each response item carries `repository.full_name` → stash in
    `Issue.project_path`
- `GhBackend::list_mrs(Scope::Group)` →
  `gh api orgs/{org}/pulls?state=open&per_page={n}`
  - Each item's `head.repo.full_name` → `MergeRequest.project_path`
- `GhBackend::list_pipelines(Scope::Group)`:
  1. `gh api orgs/{org}/repos --paginate --jq '.[].full_name'` (one
     call, cached in `recent_org_repos`)
  2. For each repo:
     `gh api repos/{owner}/{repo}/actions/runs?per_page={n}`
  3. Merge + tag each `Pipeline` with `repository.full_name`
  4. Use `tokio::join_all` +
     `crate::backend::rate_limit::pace_bulk_operation` to respect
     #382's pacing

## Phase 7 — Keybindings + docs

**`src/config.rs`:** new `def_switch_scope = "Ctrl+s"`
(replaces/aliases `switch_repo`); keep `def_save_view = "s"` separate.
Add `def_drill_into = "Enter"` and `def_pop_scope = "Esc"` to
`KeybindingGlobal`.

**`AGENTS.md`:** new section "Group/Org Scope" documenting `Scope`
enum, entry methods, supported tabs, drill-down behavior, GitHub
parity caveat. Update `src/scope.rs` in directory structure. Update
CLI command tables.

**`README.md`:** new "Group/Org browsing" subsection with `--group`
examples.

## Phase 8 — Tests

- `src/git_helpers.rs::parse_group` — round-trip tests for
  `https://host/group`, `git@host:group`, paths with nested subgroups
- `src/scope.rs` — `as_str()`, `is_group()`, `api_path_prefix()`
  URL-encoding tests (especially
  `group/sub/project` → `group%2Fsub%2Fproject`)
- `src/backend/glab.rs` — extend existing tests with `Scope::Group`
  arms where feasible; assert arg ordering for `--group` flag
- `src/backend/gh.rs` — fixture JSON for `/orgs/{org}/issues`
  response, verify `project_path` is populated
- Cache round-trip with `Scope::Group`
- `TEST_ENV_MUTEX` acquired in any env-mutating tests (per
  AGENTS.md §7)

## Phase 9 — Verification

- `cargo fmt`
- `cargo clippy -- -D warnings` (CI enforces zero warnings)
- `cargo test --all-features`
- Manual smoke test against a real GitLab group (issues, MRs,
  pipelines) and a real GitHub org
- `scripts/release.sh` preflight (no CI permissions for demo
  recording; release doc note: group screenshots added to demo tape
  list)

## Out of scope for this PR (deferred)

- Per-tab per-group permission filtering (GitLab group visibility)
- Group-level milestone creation / issue creation / MR creation
- Cross-group issues/MRs
- GitHub Enterprise custom hosts in group mode
- Caching the org's repo list for Actions aggregation across sessions
  (today: per-session)