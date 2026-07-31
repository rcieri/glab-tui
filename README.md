# glab-tui <img src="assets/terminal_trove_tool_of_the_week_green_on_black_bg.png" alt="Terminal Trove — Tool of the Week" width="180" align="right">

<p align="center">
<img src="assets/glab-tui-banner-v2.svg" alt="glab-tui" width="560">
</p>

A terminal user interface (TUI) for GitLab and GitHub, built on top of [`glab`](https://gitlab.com/gitlab-org/cli) and [`gh`](https://cli.github.com/). Browse issues, pull requests / merge requests, pipelines, runners, and releases without leaving your terminal.

---

## Features

- **GitHub & GitLab Dual Support** — Automatic detection of repository host, dynamically translating TUI actions and metadata updates to `gh` or `glab` CLI commands.
- **Mouse support** — click to navigate tabs, scroll tables, and interact with all overlays and modals
- **Bulk editing** — select multiple issues or merge requests with `Space`, then press `e` to apply labels, assignees, or milestone across all selected items at once
- **Issues** — list, filter, create, and edit issues (title, labels, assignees, milestone, due date, weight, confidentiality, description)
- **Merge Requests / Pull Requests** — list, filter, create MRs from issues, approve, merge, view diffs in terminal with code reviews, and edit MR/PR metadata
- **Code Reviews** — draft inline comments, multi-line selections, code suggestions with syntax highlighting, and atomic review submission
- **Side-by-Side Diff** — toggle between unified and side-by-side diff layouts with syntax highlighting
- **Pipelines / Actions** — inspect pipelines and their jobs, retry/cancel pipelines/actions and individual jobs, stream build traces; trigger pipelines with `workflow_dispatch` input prompts
- **Runners** — list runners with structured details panel; pause/resume, edit descriptions, and monitor live performance/queue metrics
- **Releases** — browse project releases and view details in the terminal
- **Todos / Notifications** — tab with badges, relative timestamps, fuzzy search, and an Updated column
- **Milestones** — progress bar column, inline editing, and milestone issue caching
- **Branches** — browse branches with default/protected markers; create and delete branches inline
- **Environments & Deployments** — browse environments and their deployment status, drilling into deployment history with `Enter`
- **Terminal** — live log of every `glab`/`gh` command the TUI executes, with success/failure status
- **Multi-colored Labels** — table columns render labels with their individual unique hashed colors, preserving search highlights
- **Columns Config Modal** — press `Tab` / `,` to open a centered popup overlay to toggle column visibility (`Space`), group by any column, set sort order, page size, and theme
- **Value-based Column Filtering** — press `Enter` on any column inside the configure popup to filter rows by that column's values (e.g. Issues → `State` → `opened`); multi-select supports multiple values per column
- **Live Search** — fuzzy-filter across all visible columns by pressing `/`
- **Global Search** — press `Ctrl+P` to fuzzy-search across all loaded issues and MRs from any tab
- **Switch Repository** — press `Ctrl+S` to switch to another local repository without restarting
- **Inline editing** — full edit menus with searchable multi-select selectors for labels, assignees, reviewers, and milestones
- **Interactive Date Picker** — calendar widget for Due Date / Start Date fields in edit menus
- **External editor** — descriptions and freeform fields open in your `$EDITOR` / `$VISUAL` (also via `Ctrl+E`)
- **Self-update** — press `u` in the TUI (or run `glab-tui --update`) to check for and install updates
- **CLI subcommands** — `doctor` (system diagnostics), `clean-cache` (stale cache cleanup), `cache` (list cached data), `open` (open entity in browser), `repos` (list recent repositories)
- **Lazy-load tabs** — data for each tab is only fetched the first time you switch to it; refresh with `F5` / `Ctrl+R`
- **Themes** — 13 built-in color themes; fully customizable via `config.toml` or custom `.toml` files
- **Configurable keybindings** — every action is remappable in `~/.config/glab-tui/config.toml`

---

![Overview](assets/demo-overview.gif)
![Search & Configure](assets/demo-search.gif)
![Navigation & Selection](assets/demo-selection.gif)

## Prerequisites

| Requirement | Notes |
|---|---|
| **Rust** (stable, edition 2024) | Install via [rustup](https://rustup.rs/) |
| **[`glab`](https://gitlab.com/gitlab-org/cli)** / **[`gh`](https://cli.github.com/)** | Either `glab` (for GitLab repos, authenticated via `glab auth login`) or `gh` (for GitHub repos, authenticated via `gh auth login`) must be on `$PATH`. You only need the CLI for the service you use. |
| **`git`** | Used to auto-detect the current project from `git remote get-url origin` |
| **A terminal emulator** | Any terminal that supports 256 colours and Unicode |

> **Windows note:** the binary works on Windows. Editor integration uses `cmd /c` automatically when `$OS` is Windows.

---

## Installation

### From source

```sh
git clone https://github.com/rcieri/glab-tui
cd glab-tui
cargo build --release
# The binary is at ./target/release/glab-tui
```

Copy the binary somewhere on your `$PATH`, e.g.:

* **Linux / macOS**:
  ```sh
  cp target/release/glab-tui ~/.local/bin/
  ```
* **Windows (PowerShell)**:
  ```powershell
  Copy-Item target\release\glab-tui.exe $env:USERPROFILE\.local\bin\
  ```

### With `cargo install` (from crates.io)

```sh
cargo install glab-tui-crate
```

### With `cargo install` (from the repo root)

```sh
cargo install --path .
```

### Install script (Linux / macOS)

```sh
curl -sSfL https://raw.githubusercontent.com/rcieri/glab-tui/main/install.sh | sh
```

Or with `wget`:

```sh
wget -qO- https://raw.githubusercontent.com/rcieri/glab-tui/main/install.sh | sh
```

The binary is installed to `~/.local/bin/` (configurable via `PREFIX` environment variable).

### Install script (Windows)

```powershell
iwr -useb https://raw.githubusercontent.com/rcieri/glab-tui/main/install.ps1 | iex
```

The binary is installed to `$env:USERPROFILE\.local\bin\` (configurable via `-Prefix` parameter).

### Docker

```sh
docker run --rm -it -v "$PWD:/workspace" ghcr.io/rcieri/glab-tui:latest
```

The image includes both `glab` and `gh` CLIs and expects a Git repository mounted at `/workspace`.

### Homebrew

```sh
brew tap rcieri/glab-tui
brew install glab-tui
```

Installs the `glab-tui` binary on macOS (Intel and Apple Silicon) and Linux (x86_64 and ARM64) from the [homebrew-glab-tui](https://github.com/rcieri/homebrew-glab-tui) tap. Requires either `gh` or `glab` (only the CLI matching the repository hosting service you use is needed).

### Scoop (Windows)

```powershell
scoop bucket add glab-tui https://github.com/rcieri/scoop-glab-tui.git
scoop install glab-tui
```

Installs `glab-tui` from the [scoop-glab-tui](https://github.com/rcieri/scoop-glab-tui) bucket. The manifest uses Scoop's `autoupdate` — version bumps are pulled automatically from GitHub releases.

---

## Configuration

### Authentication

`glab-tui` delegates all API calls to the `glab` or `gh` CLI depending on the hosting service (you only need to authenticate the one you use):

```sh
glab auth login   # for GitLab repos
gh auth login     # for GitHub repos
```

The active project is detected automatically from the `origin` remote in the current working directory.

### Config file

The config file is optional; the app boots from in-memory defaults when none exists. To create one, press the **save view** keybinding (default `s`), which writes the current view layout to either `~/.config/glab-tui/config.toml` (global) or `.glab-tui/config.toml` (repo-local, when inside a git repo). Locations:

```
~/.config/glab-tui/config.toml          # Linux / macOS (XDG)
$GLAB_TUI_CONFIG                         # override: set to the full file path
```

The generated file is fully annotated. Key sections:

```toml
# Pick a built-in theme preset
theme_preset = "default"   # default | tokyo-night | gruvbox | nord | catppuccin-mocha | dracula

# Override individual colors (takes precedence over theme_preset)
# [theme]
# bg = "#121214"
# border_focused = "#31bf67"
# ...color tokens (see bundled themes for the full list)

# Remap any keybinding
[keybindings.global]
next_tab = "l"
# ...

[keybindings.issues]
create_issue = "n"
edit_entity = "e"
# ...

# Persist default column visibility / grouping / filters per pane
# [issues]
# columns = ["ID", "State", "Title", "Labels"]
# group_by_column = "State"
# group_ascending = true
# [issues.column_filters]
# State = ["opened"]

# [mrs]
# columns = ["ID", "State", "Status", "Title", "Labels"]
# [mrs.column_filters]
# State = ["opened"]
```

### Custom themes

Drop any `<name>.toml` file into `~/.config/glab-tui/themes/` and set `theme_preset = "<name>"` in `config.toml`. The file must define the same 19 color tokens as the bundled themes.

### Editor

Set `$EDITOR` or `$VISUAL` to control which editor opens for description and freeform fields:

```sh
export EDITOR=nvim   # or vim, nano, hx, code, etc.
```

The default fallback is `helix` (`hx`). Inside any edit menu you can also press `Ctrl+E` to open the editor directly.

---

## Usage

```sh
# Run from inside a GitLab or GitHub repository:
cd /path/to/your/repo
glab-tui

# Specifying optional flags:
glab-tui --repo organization/project-name
glab-tui --dir /path/to/other/repo
```

### Options

| Flag / Subcommand | Argument | Description |
|---|---|---|
| `-r`, `--repo` | `owner/repo` | Launch glab-tui for a custom remote repository |
| `-d`, `--dir` | `/path/to/dir` | Launch glab-tui in a custom repository directory |
| `-u`, `--update` | | Check for and install updates |
| `-h`, `--help` | | Print usage help details |
| `-V`, `--version` | | Print version information |
| `doctor` | *(subcommand)* | Check system health — backend CLI availability, config integrity, cache status |
| `clean-cache` | `[-n, --dry-run]` | Remove stale cache entries for repos that no longer exist (preview with `--dry-run`) |
| `cache` | *(subcommand)* | List cached data files with sizes |
| `open` | `<entity> <id>` | Open an entity in the browser **without launching the TUI** — valid entities: `issue`, `mr`, `pr`, `pipeline`, `job`, `milestone` |
| `repos` | *(subcommand)* | List recently-used and sibling repositories |

The TUI will launch in the terminal, auto-detecting the project context and fetching the Issues tab immediately.

### CLI subcommand examples

```sh
glab-tui doctor                     # run system diagnostics
glab-tui clean-cache --dry-run      # preview stale-cache cleanup
glab-tui clean-cache                # actually remove stale cache entries
glab-tui cache                      # list cached data files with sizes
glab-tui open issue 42              # open issue #42 in your browser
glab-tui open mr 7                  # open MR/PR #7 in your browser
glab-tui repos                      # list recently-used repositories
```

---

## Filtering, Grouping & Columns

Every table tab (Issues, MRs/PRs, Pipelines, Jobs, Runners, Releases, Todos, Milestones, Branches, Environments) can be tailored with column visibility, value-based filters, grouping, and sort order — all from a single **Configure View** popup.

### Column configuration & value-based filtering

1. Press **`Tab`** (or **`,`**) to open the **Configure View** popup.
2. The **COLUMNS** section lists every available column for the active tab. Use `j`/`k` (or arrows) to move through it.
   - **`Space`** toggles whether a column is shown in the table.
   - **`Enter`** opens a **value-based filter** for that column: a searchable multi-select of the distinct values currently loaded. For example, on the Issues tab, `Enter` on the `State` column lets you filter to just `opened` issues — or on the `Labels` column, to specific labels.
3. Inside the filter selector: `Space` toggles values on/off, `/` or `f` fuzzy-searches the values, `Enter` applies the filter, `Esc` cancels. Selecting multiple values is supported (e.g. `opened` **and** `closed`).
4. Applied filters are shown as a count next to the column, e.g. `[x] State (1)`. Re-open the column and uncheck values to widen or clear the filter.

### Grouping & sort order

- The **GROUP BY** section lets you group rows by any column: move to a column and press **`Space`** or **`Enter`** to toggle grouping. Grouped rows are visually separated by headers.
- The **ORDER** section toggles between **Ascending** and **Descending** sort order for the current group-by column (or the default ordering when no group is set).

### Page size & theme

- **PAGE SIZE** controls how many items are fetched per tab. **`Enter`** on it puts it into edit mode.
- **THEME** lets you switch the color theme on the fly; selections persist via **Save View** below.

### Saving & persistence

- The **Save View** button at the bottom of the popup writes the current layout — enabled columns, group-by, order, filters, and page size — to `config.toml` (repo-local `.glab-tui/config.toml` or global `~/.config/glab-tui/config.toml`).

### A note on filtering

> Column filters are applied **client-side, after data is fetched** — they only ever see the rows that were loaded. If you have many closed/merged items and want more open ones in the list, raise the **PAGE SIZE** in the Configure View popup (or set `page_size` in `config.toml`) so more rows are fetched to filter across.

---

## Key Bindings

> All tables below show the **default** keys. The **Config** column shows the key name to remap in `config.toml` (e.g. under `[keybindings.issues]`, `create_issue = "n"`). `—` means the binding is fixed and **not** remappable.

### Global

> Remappable via `[keybindings.global]` in `config.toml`.

| Key | Action | Config |
|---|---|---|
| `l` / `→` | Next tab | `next_tab` |
| `h` / `←` | Previous tab | `prev_tab` |
| `Tab` / `,` | Open column configure popup (`Space` toggle column, `Enter` filter by column values) | `configure` |
| `Esc` | Close configure popup / overlay | — |
| `j` / `↓` | Move selection down | — |
| `k` / `↑` | Move selection up | — |
| `J` | Scroll description panel down | `scroll_down` |
| `K` | Scroll description panel up | `scroll_up` |
| `f` / `/` | Open search / filter bar | `search` |
| `Enter` / `Esc` (in search) | Close search bar | — |
| `?` / `F1` | Show help | `help` |
| `Ctrl+P` | Global search across all loaded issues & MRs | `global_search` |
| `Ctrl+S` | Switch repository | — |
| `F5` / `Ctrl+R` | Refresh current tab | `refresh` |
| `s` | Save view layout to config | `save_view` |
| `u` | Check for updates | — |
| `q` / `Esc` | Quit (or close current overlay) | `quit` |

---

### Issues tab

> Remappable via `[keybindings.issues]` in `config.toml`.

| Key | Action | Config |
|---|---|---|
| `n` | Create new issue (prompts for title) | `create_issue` |
| `e` | Open edit menu for selected issue (opens bulk edit menu when multiple are selected) | `edit_entity` |
| `m` | Create MR/PR from selected issue | `create_mr` |
| `c` | Close selected issue | `close_entity` |
| `r` | Reopen selected issue | `reopen_entity` |
| `d` | Delete selected issue (with confirmation) | `delete_entity` |
| `o` | Open selected issue in browser | — |
| `Space` | Select issue for bulk editing | `select_issue` |
| `J` | Scroll description panel down | `scroll_down` |
| `K` | Scroll description panel up | `scroll_up` |

**Issue edit menu fields**

| Field | Input method |
|---|---|
| Title | Inline text input |
| Labels | Searchable multi-select (fetched from GitLab/GitHub) |
| Assignees | Searchable multi-select (fetched from project members) |
| Milestone | Searchable single-select (fetched from project) |
| Confidential | Single-select: Public / Confidential *(GitLab only)* |
| Due Date | Interactive calendar date picker (`Enter` to open; `h`/`l` month, `j`/`k` day) *(GitLab only)* |
| Weight | Inline text input (integer) *(GitLab only)* |
| Description | Opens `$EDITOR` (or press `Ctrl+E`) |

---

### Merge Requests tab

> Remappable via `[keybindings.mrs]` in `config.toml`.

| Key | Action | Config |
|---|---|---|
| `n` | Create MR from issue ID (prompts for issue IID) | `create_mr` |
| `e` | Open edit menu for selected MR (opens bulk edit menu when multiple are selected) | `edit_entity` |
| `a` | Approve selected MR | `approve_mr` |
| `m` | Merge selected MR (squash + remove source branch) | `merge_mr` |
| `v` | View diff of selected MR in terminal | `view_diff` |
| `P` | View related pipelines from MR detail | `view_related_pipelines` |
| `Space` | Select MR for bulk editing | `select_mr` |
| `o` | Open selected MR in browser | — |
| `s` | Toggle Draft / Ready status | `toggle_draft` |
| `c` | Close selected MR | `close_entity` |
| `r` | Reopen selected MR | `reopen_entity` |
| `d` | Delete selected MR (with confirmation) | `delete_entity` |
| `J` | Scroll description panel down | `scroll_down` |
| `K` | Scroll description panel up | `scroll_up` |

**MR edit menu fields**

| Field | Input method |
|---|---|
| Title | Inline text input |
| Labels | Searchable multi-select |
| Assignees | Searchable multi-select |
| Reviewers | Searchable multi-select |
| Milestone | Searchable single-select |
| Target Branch | Inline text input |
| Status (Draft/Ready) | Single-select |
| Description | Opens `$EDITOR` (or press `Ctrl+E`) |

---

### Diff View

Press `v` on an MR/PR to open its diff. Use `Tab` to move focus between the **file tree** and the **diff pane**.

> Diff View keys are **fixed** and not remappable in `config.toml`.

| Key | Action |
|---|---|
| `q` / `Esc` | Exit diff view (or cancel current selection / search) |
| `Tab` | Toggle focus between file tree and diff pane |
| `h` / `←` | In file tree: collapse directory; in diff: focus file tree |
| `l` / `→` | In file tree: expand directory / open file |
| `j` / `↓` | Move down (file tree or diff lines) |
| `k` / `↑` | Move up (file tree or diff lines) |
| `J` / `K` | Scroll 10 lines / jump 10 files |
| `Enter` / `Space` | In file tree: open file; in diff: toggle zoom (hide/show file tree) |
| `[` / `]` | Previous / next hunk |
| `z` / `Z` | Collapse / expand all files |
| `d` | Toggle unified / side-by-side layout |
| `v` / `V` | Start / stop multi-line selection for comments |
| `c` | Add comment on current line / selection |
| `C` | Add comment via external `$EDITOR` |
| `e` | Add code suggestion via `$EDITOR` |
| `a` | Open comment actions menu (reply, resolve, edit, delete) |
| `r` | Submit review (Approve / Request Changes / Comment) |
| `/` / `f` | Search within diff |
| `Ctrl+N` | Next search match |
| `Ctrl+Shift+N` | Previous search match |
| `?` / `F1` | Show help |

---

### Pipelines tab

> Remappable via `[keybindings.pipelines]` in `config.toml`.

| Key | Action | Config |
|---|---|---|
| `Enter` | Drill into selected pipeline (show its jobs) | — |
| `Esc` / `Backspace` | Go back (jobs → pipelines, trace → jobs) | — |
| `n` | Create / run a pipeline with an interactive form (branch/ref, workflow inputs, variables) | — |
| `p` | Trigger a new pipeline from the current branch (`glab ci run --mr`) | `trigger_pipeline` |
| `r` | Retry selected pipeline (or all checked pipelines) | `retry` |
| `d` | Cancel selected pipeline | `cancel` |
| `o` | Open pipeline in browser | — |
| `Space` | Check/uncheck pipeline for bulk retry | — |
| `j` / `↓` | (in job view) move down | — |
| `k` / `↑` | (in job view) move up | — |

**Inside a pipeline (job view)**

> Remappable via `[keybindings.jobs]` in `config.toml`.

| Key | Action | Config |
|---|---|---|
| `Enter` | Fetch and display job trace (toggle zoom when trace is open) | `view_trace` |
| `r` | Retry selected job (or all checked jobs) | `retry` |
| `S` | Start manual (blocked) GitLab CI job | `start_job` |
| `c` | Cancel selected job (or all checked jobs) | `cancel` |
| `d` | Download job artifact | `download_artifact` |
| `o` | Open job in browser | `open_in_browser` |
| `e` | Open job trace in `$EDITOR` | `view_trace_editor` |
| `p` | Switch to pipeline selector | `enter_pipeline` |
| `s` | Select all jobs in the current stage | `select_stage` |
| `w` | Toggle trace word wrap | `toggle_trace_wrap` |
| `m` | Collapse / expand matrix jobs | — |
| `Space` | Check/uncheck job for bulk retry/cancel | `select_job` |
| `Esc` / `Backspace` | Go back (trace → jobs → pipelines) | — |
| `j` / `↓` | (in trace view) scroll down | — |
| `k` / `↑` | (in trace view) scroll up | — |

---

### Runners tab

> Remappable via `[keybindings.runners]` in `config.toml`.

| Key | Action | Config |
|---|---|---|
| `p` | Pause selected runner | `pause` |
| `r` | Resume (un-pause) selected runner | `resume` |
| `e` | Edit runner description (inline text input) | `edit_description` |

---

### Releases tab

> Remappable via `[keybindings.releases]` in `config.toml`.

| Key | Action | Config |
|---|---|---|
| `Enter` | View release details in terminal | — |
| `n` | Create a new release (tag, name, description) | `create_release` |
| `e` | Edit selected release | `edit_release` |
| `d` | Delete selected release (with confirmation) | `delete_release` |
| `o` | Open release in browser | `open_in_browser` |

---

### Milestones tab

> Remappable via `[keybindings.milestones]` in `config.toml`.

| Key | Action | Config |
|---|---|---|
| `n` | Create new milestone (title, description, start & due date) | `create_milestone` |
| `e` | Edit selected milestone | `edit_milestone` |
| `c` | Close selected milestone | `close_milestone` |
| `r` | Reopen selected milestone | `reopen_milestone` |
| `d` | Delete selected milestone (with confirmation) | `delete_milestone` |
| `o` | Open milestone in browser | `open_in_browser` |

---

### Todos tab

On GitLab this tab shows **Todos**; on GitHub it shows **Notifications**.

> Remappable via `[keybindings.todos]` in `config.toml`.

| Key | Action | Config |
|---|---|---|
| `Enter` | Mark item as read and jump to its target (issue / MR) | `mark_as_read` |
| `o` | Open item in browser | `open_in_browser` |

---

### Branches tab

> Remappable via `[keybindings.branches]` in `config.toml`.

| Key | Action | Config |
|---|---|---|
| `n` | Create a new branch (prompts for name; based on the selected branch) | `create_branch` |
| `d` | Delete selected branch (with confirmation) | `delete_branch` |

---

### Environments tab

> Remappable via `[keybindings.environments]` in `config.toml`.

| Key | Action | Config |
|---|---|---|
| `Enter` | Fetch and view the deployments list for the selected environment | `view_deployments` |

---

### Terminal tab

Logs every `glab` / `gh` command the TUI executes, with success/failure status.

> Remappable via `[keybindings.terminal]` in `config.toml`.

| Key | Action | Config |
|---|---|---|
| `j` / `↓` | Scroll log down | — |
| `k` / `↑` | Scroll log up | — |
| `w` | Toggle line wrapping | `toggle_wrap` |

---

### Selector overlays (labels, assignees, etc.)

Searchable multi-select popups are used for choosing labels, assignees, reviewers, milestones, and for **value-based column filtering** (see [Filtering, Grouping & Columns](#filtering-grouping--columns)).

> Selector keys are **fixed** and not remappable in `config.toml`.

| Key | Action |
|---|---|
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `Space` | Toggle selection |
| `f` / `/` / `i` | Enter filter/search mode |
| `Backspace` | Delete last character in filter |
| `Enter` | Confirm selection and apply |
| `Esc` | Cancel and return to edit menu |

> If you type a value that doesn't exist in the list, a **`+ Create "…"`** option appears at the top, letting you create a new label inline.

---

## Dependencies

| Crate | Version | Purpose |
|---|---|---|
| [`ratatui`](https://crates.io/crates/ratatui) | 0.30.2 | TUI rendering framework |
| [`crossterm`](https://crates.io/crates/crossterm) | 0.29.0 | Cross-platform terminal I/O and event streaming |
| [`tokio`](https://crates.io/crates/tokio) | 1.53 (full) | Async runtime for concurrent data fetching |
| [`serde`](https://crates.io/crates/serde) | 1.0 (derive) | Serialization / deserialization |
| [`serde_json`](https://crates.io/crates/serde_json) | 1.0 | Parsing JSON responses from `glab api` |
| [`toml`](https://crates.io/crates/toml) | 1.1 | Parsing `config.toml` and theme files |
| [`anyhow`](https://crates.io/crates/anyhow) | 1.0 | Ergonomic error handling |
| [`async-trait`](https://crates.io/crates/async-trait) | 0.1 | Async trait support for Backend trait |
| [`clap`](https://crates.io/crates/clap) | 4 (derive) | CLI argument parsing for subcommands |
| [`serde_yaml`](https://crates.io/crates/serde_yaml) | 0.9 | YAML output for `doctor` diagnostics |
| [`chrono`](https://crates.io/crates/chrono) | 0.4 | Timestamp formatting ("2 hours ago") |
| [`tempfile`](https://crates.io/crates/tempfile) | 3.10 | Temporary files for editor integration |
| [`fuzzy-matcher`](https://crates.io/crates/fuzzy-matcher) | 0.3 | Fuzzy search/filter across table columns |
| [`syntect`](https://crates.io/crates/syntect) | 5 | Syntax highlighting in diff and preview panes |

All API calls are made by shelling out to `gh api` or `glab api` (depending on the repository host; you only need the CLI matching the service you use) — no personal access token or direct HTTP client is required inside the binary.

---

## Project Structure

```
src/
├── main.rs          # Entry point, event loop, all key-binding handlers
├── app.rs           # App state, Tab enum, DiffView, DatePicker, filtering logic
├── config.rs        # Config/Theme loading, keybinding structs, TOML generation
├── event.rs         # Async event handler (keyboard, tick, async data events)
├── fetch.rs         # Per-tab data-fetching dispatch
├── editor.rs        # External editor integration ($EDITOR)
├── entity_editor.rs # Edit-menu field change logic
├── templates.rs     # Default issue/MR/PR description templates
├── cli.rs           # CLI subcommands (doctor, clean-cache, cache, open, repos)
├── themes/          # Bundled theme TOML files
├── backend/         # CLI backend layer
│   ├── mod.rs       # Backend trait (~40 methods)
│   ├── glab.rs      # GlabBackend — shells out to glab CLI
│   └── gh.rs        # GhBackend — shells out to gh CLI
├── domain/          # Domain models + API logic
│   ├── mod.rs       # Module declarations
│   ├── client.rs    # GitlabClient wrapper (backend + page_size + event tx)
│   ├── issues.rs    # Issue struct + list/get/create/edit
│   ├── mr.rs        # MergeRequest/PR, DiscussionNote, NotePosition
│   ├── pipelines.rs # Pipeline + Job types, dedup, retry logic, unit tests
│   ├── runners.rs   # Runner type + list/edit logic
│   ├── releases.rs  # Release type + list/create/edit
│   ├── milestones.rs# Milestone type + list/edit
│   ├── notifications.rs # Todo/notification type + list
│   ├── branches.rs  # Branch type + list/create/delete
│   └── deployments.rs # Environment + Deployment types
├── handlers/        # Keypress handlers
│   ├── mod.rs
│   ├── tabs.rs      # Per-tab keybinding handlers
│   └── overlays.rs  # Overlay keybinding handlers
├── ui/              # Ratatui render functions
│   ├── mod.rs       # Re-exports
│   ├── tabs.rs      # Tab-specific render functions
│   ├── overlays.rs  # Overlay render functions
│   ├── helpers.rs   # Shared UI helpers
│   ├── diff.rs      # Diff view render functions
└── modal.rs     # Unified modal component
└── utils/
    ├── mod.rs
    ├── cache.rs     # Offline caching
    ├── format.rs    # Time formatting, markdown, truncation
    ├── ui.rs        # StatefulTable generic helper
    └── update.rs    # GitHub releases self-updater
```

---

## Running Tests

```sh
cargo test
```

Unit tests live in several modules:
- [`src/domain/pipelines.rs`](src/domain/pipelines.rs) — pipeline job deduplication and stage-ordering logic.
- [`src/domain/mr.rs`](src/domain/mr.rs) — discussion note and review comment logic.
- [`src/app.rs`](src/app.rs) — selector fuzzy-matching and filter logic.

---

## Contributing

1. Fork the repo and create a feature branch.
2. Keep commits atomic and follow [Conventional Commits](https://www.conventionalcommits.org/).
3. Run `cargo fmt` and `cargo clippy -- -D warnings` before opening a PR.
4. Add or update tests where relevant.

---

## License

[`MIT`](LICENSE.md)
