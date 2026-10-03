use crate::AppTerminal;
use crate::app::{App, CUSTOM_COMMAND_LOG_PREFIX};
use crate::custom_commands::{CommandPane, CustomCommand, TemplateValues, render, shell_process};
use crate::event::Event;
use crate::scope::Scope;
use crossterm::event::KeyEvent;
use tokio::sync::mpsc::UnboundedSender;

/// Runs the custom command bound to `key_event` on the active tab, handing it
/// the terminal until it exits. Returns whether a custom binding claimed the
/// key; every claimed keypress leaves one entry in the terminal log.
pub fn run_bound_command(
    app: &mut App,
    key_event: &KeyEvent,
    terminal: &mut AppTerminal,
    tx: &UnboundedSender<Event>,
) -> bool {
    let Some(command) = app.custom_commands.find(app.active_tab, key_event).cloned() else {
        return false;
    };
    execute(app, &command, terminal, tx);
    true
}

/// Like `run_bound_command`, for `[[custom_keybindings.diff]]` while the
/// diff view is open.
pub fn run_bound_diff_command(
    app: &mut App,
    key_event: &KeyEvent,
    terminal: &mut AppTerminal,
    tx: &UnboundedSender<Event>,
) -> bool {
    let Some(command) = app.custom_commands.find_in_diff(key_event).cloned() else {
        return false;
    };
    execute(app, &command, terminal, tx);
    true
}

/// One invocation of a command: the row it acts on (`#12`, `!3`, or empty
/// for a command that has none) and its template values.
struct Target {
    row: String,
    values: TemplateValues,
}

/// Outcome of every run of one keypress, reported back to the main loop.
#[derive(Clone, Debug)]
pub struct RunReport {
    label: String,
    total: usize,
    /// Rows refused before running, already logged.
    refused: Vec<String>,
    /// Rendered command and outcome of each run, in order.
    runs: Vec<(String, Result<(), String>)>,
    /// Terminal-log rows shown as running while a background run works,
    /// one per run in the same order; empty for terminal runs.
    log_rows: Vec<usize>,
}

/// Where a command's stderr goes while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handoff {
    /// The command owns the terminal; stderr is shown and also kept.
    Terminal,
    /// The TUI stays on screen; stderr is only kept.
    Background,
}

fn execute(
    app: &mut App,
    command: &CustomCommand,
    terminal: &mut AppTerminal,
    tx: &UnboundedSender<Event>,
) {
    let Selection {
        targets,
        missing_rows,
    } = match targets(app, command.pane) {
        Ok(selection) => selection,
        Err(reason) => {
            app.log_command_outcome(
                format!("{CUSTOM_COMMAND_LOG_PREFIX}{}", command.command),
                Err(format!("\"{}\" not run: {reason}", command.label())),
            );
            return;
        }
    };
    let total = targets.len() + missing_rows.len();

    let mut refused: Vec<String> = Vec::new();
    for row in missing_rows {
        let reason = format!(
            "\"{}\" not run for {row}: it is no longer loaded",
            command.label()
        );
        app.record_command_outcome(
            format!("{CUSTOM_COMMAND_LOG_PREFIX}{}", command.command),
            &Err(reason.clone()),
        );
        refused.push(reason);
    }
    let mut runs: Vec<(String, TemplateValues)> = Vec::new();
    for target in targets {
        match render(&command.command, &target.values) {
            Ok(rendered) => runs.push((rendered, target.values)),
            Err(reason) => {
                let reason = format!(
                    "\"{}\" not run{}: {reason}",
                    command.label(),
                    on_row(&target.row)
                );
                app.record_command_outcome(
                    format!("{CUSTOM_COMMAND_LOG_PREFIX}{}", command.command),
                    &Err(reason.clone()),
                );
                refused.push(reason);
            }
        }
    }

    let mut report = RunReport {
        label: command.label().to_string(),
        total,
        refused,
        runs: Vec::new(),
        log_rows: Vec::new(),
    };
    if runs.is_empty() {
        report_runs(app, report);
        return;
    }

    if command.background {
        report.log_rows = runs
            .iter()
            .map(|(rendered, _)| {
                app.start_command(format!("{CUSTOM_COMMAND_LOG_PREFIX}{rendered}"))
            })
            .collect();
        let command = command.clone();
        let tx = tx.clone();
        std::thread::spawn(move || {
            report.runs = run_all(&command, runs, Handoff::Background);
            let _ = tx.send(Event::CustomCommandFinished(report));
        });
        return;
    }

    let label = command.label().to_string();
    report.runs = match crate::editor::suspend_while(terminal, || {
        run_all(command, runs, Handoff::Terminal)
    }) {
        Ok(outcomes) => outcomes,
        Err(error) => vec![(
            command.command.clone(),
            Err(format!("\"{label}\" not run: {error}")),
        )],
    };
    report_runs(app, report);
}

fn run_all(
    command: &CustomCommand,
    runs: Vec<(String, TemplateValues)>,
    handoff: Handoff,
) -> Vec<(String, Result<(), String>)> {
    runs.into_iter()
        .map(|(rendered, values)| {
            if handoff == Handoff::Terminal {
                // The TUI is off screen while a terminal run works; a command
                // that prints nothing would otherwise leave a blank screen.
                eprintln!("glab-tui: running \"{}\": {rendered}", command.label());
            }
            let mut process = shell_process(&rendered);
            process.envs(values.environment());
            let outcome = run_process(command, &mut process, handoff);
            (rendered, outcome)
        })
        .collect()
}

/// Logs every run of a keypress and raises one toast for its failures.
pub fn report_runs(app: &mut App, report: RunReport) {
    let mut failures = report.refused;
    for (index, (rendered, outcome)) in report.runs.into_iter().enumerate() {
        match report.log_rows.get(index) {
            Some(&row) => app.settle_command(row, &outcome),
            None => app
                .record_command_outcome(format!("{CUSTOM_COMMAND_LOG_PREFIX}{rendered}"), &outcome),
        }
        if let Err(reason) = outcome {
            failures.push(reason);
        }
    }
    match failures.as_slice() {
        [] => {}
        [only] if report.total == 1 => app.raise_error_toast(only.clone()),
        _ => app.raise_error_toast(format!(
            "\"{}\" failed for {} of {} selected items; see the Terminal tab",
            report.label,
            failures.len(),
            report.total
        )),
    }
}

fn on_row(row: &str) -> String {
    if row.is_empty() {
        String::new()
    } else {
        format!(" for {row}")
    }
}

/// Characters of a failing command's last stderr line kept for the log.
const MAX_ERROR_DETAIL_CHARS: usize = 200;

/// How long a background run's stderr is still read after the command
/// exits; a descendant it left running may hold the pipe open indefinitely.
const STDERR_DRAIN_AFTER_EXIT: std::time::Duration = std::time::Duration::from_millis(200);

/// Runs `process` to completion. A terminal run inherits every stream, so
/// shells stay interactive and tools see a tty; its failure reports the exit
/// status. A background run keeps its stderr so a failure names its cause.
fn run_process(
    command: &CustomCommand,
    process: &mut std::process::Command,
    handoff: Handoff,
) -> Result<(), String> {
    let (status, stderr) = match handoff {
        Handoff::Terminal => (process.status(), Vec::new()),
        Handoff::Background => {
            process
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped());
            detach_from_terminal(process);
            match process.spawn() {
                Ok(mut child) => {
                    let chunks = child.stderr.take().map(stream_stderr);
                    let status = child.wait();
                    (status, chunks.map(drain_stderr).unwrap_or_default())
                }
                Err(error) => (Err(error), Vec::new()),
            }
        }
    };
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(match last_error_line(&stderr) {
            Some(detail) => format!("\"{}\" failed with {status}: {detail}", command.label()),
            None => format!("\"{}\" failed with {status}", command.label()),
        }),
        Err(error) => Err(format!(
            "\"{}\" could not start {:?}: {error}",
            command.label(),
            process.get_program()
        )),
    }
}

/// Forwards `stderr` chunk by chunk from a reader thread.
fn stream_stderr(mut stderr: std::process::ChildStderr) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buffer = [0u8; 4096];
        while let Ok(read) = stderr.read(&mut buffer) {
            if read == 0 || sender.send(buffer[..read].to_vec()).is_err() {
                break;
            }
        }
    });
    receiver
}

/// Collects what `chunks` delivers until the pipe closes or
/// `STDERR_DRAIN_AFTER_EXIT` passes.
fn drain_stderr(chunks: std::sync::mpsc::Receiver<Vec<u8>>) -> Vec<u8> {
    let deadline = std::time::Instant::now() + STDERR_DRAIN_AFTER_EXIT;
    let mut stderr = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match chunks.recv_timeout(remaining) {
            Ok(chunk) => stderr.extend_from_slice(&chunk),
            Err(_) => return stderr,
        }
    }
}

/// Starts `process` in a new session without a controlling terminal. Tools
/// built on Go's termenv (lazyworktree, gh) open /dev/tty even with every
/// stdio redirected and query the background colour; the terminal's reply
/// then arrives as keypresses in the TUI (`r` of `rgb:` reopening an issue).
#[cfg(unix)]
fn detach_from_terminal(process: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the hook only calls setsid(2), which is async-signal-safe and
    // touches no memory shared with the parent.
    unsafe {
        process.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn detach_from_terminal(_process: &mut std::process::Command) {}

/// Last non-blank line of `output`, without control characters.
fn last_error_line(output: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(output);
    let line = text.lines().rev().find(|line| !line.trim().is_empty())?;
    let cleaned: String = line
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_ERROR_DETAIL_CHARS)
        .collect();
    Some(cleaned)
}

/// Local checkout per project, looked up once per keypress: in group scope
/// each lookup runs git once per recent repository.
type CheckoutCache = std::collections::HashMap<String, Result<String, String>>;

/// What one keypress acts on, plus the selected rows that are no longer
/// loaded and so are skipped.
struct Selection {
    targets: Vec<Target>,
    missing_rows: Vec<String>,
}

/// What a command acts on: every selected issue or MR/PR when there is a
/// selection (bulk run), otherwise the highlighted row, the diff cursor, or
/// the repository for a universal command.
fn targets(app: &App, pane: CommandPane) -> Result<Selection, String> {
    let mut checkouts = CheckoutCache::new();
    let mut missing_rows = Vec::new();
    let targets = match pane {
        CommandPane::Universal => {
            let mut values = TemplateValues::default();
            let project = app
                .scope
                .is_repository()
                .then(|| app.scope.as_str().to_string());
            insert_repo_values(&mut values, app, project, &mut checkouts);
            vec![Target {
                row: String::new(),
                values,
            }]
        }
        CommandPane::Issues => {
            let issues: Vec<&crate::domain::issues::Issue> = if app.selected_issues.is_empty() {
                let issue = app
                    .issues
                    .state
                    .selected()
                    .and_then(|index| app.filtered_issues().get(index).copied())
                    .ok_or_else(|| "no issue is selected".to_string())?;
                vec![issue]
            } else {
                let loaded: Vec<_> = app
                    .issues
                    .items
                    .iter()
                    .filter(|i| {
                        app.selected_issues
                            .contains(&(i.project_path.clone(), i.iid))
                    })
                    .collect();
                missing_rows = unloaded_rows(
                    &app.selected_issues,
                    loaded.iter().map(|i| (i.project_path.as_str(), i.iid)),
                    "#",
                );
                sorted_by_project_and_number(loaded, |i| (i.project_path.as_str(), i.iid))
            };
            if issues.is_empty() {
                return Err("the selected issues are no longer loaded".to_string());
            }
            issues
                .into_iter()
                .map(|issue| issue_target(app, issue, &mut checkouts))
                .collect()
        }
        CommandPane::MergeRequests => {
            let mrs: Vec<&crate::domain::mr::MergeRequest> = if app.selected_mrs.is_empty() {
                let mr = app
                    .mrs
                    .state
                    .selected()
                    .and_then(|index| app.filtered_mrs().get(index).copied())
                    .ok_or_else(|| format!("no {} is selected", app.kind().term("mr_short")))?;
                vec![mr]
            } else {
                let loaded: Vec<_> = app
                    .mrs
                    .items
                    .iter()
                    .filter(|m| app.selected_mrs.contains(&(m.project_path.clone(), m.iid)))
                    .collect();
                missing_rows = unloaded_rows(
                    &app.selected_mrs,
                    loaded.iter().map(|m| (m.project_path.as_str(), m.iid)),
                    mr_row_prefix(app),
                );
                sorted_by_project_and_number(loaded, |m| (m.project_path.as_str(), m.iid))
            };
            if mrs.is_empty() {
                return Err(format!(
                    "the selected {} are no longer loaded",
                    app.kind().term("mr_plural")
                ));
            }
            mrs.into_iter()
                .map(|mr| mr_target(app, mr, &mut checkouts))
                .collect()
        }
        CommandPane::Diff => vec![Target {
            row: String::new(),
            values: diff_values(app, &mut checkouts)?,
        }],
    };
    Ok(Selection {
        targets,
        missing_rows,
    })
}

fn sorted_by_project_and_number<'a, T>(
    mut rows: Vec<&'a T>,
    key: impl Fn(&T) -> (&str, u64),
) -> Vec<&'a T> {
    rows.sort_by(|a, b| key(a).cmp(&key(b)));
    rows
}

/// Selected `(project, number)` keys with no loaded row, as `#12` / `!12`.
fn unloaded_rows<'a>(
    selected: &std::collections::HashSet<(String, u64)>,
    loaded: impl Iterator<Item = (&'a str, u64)>,
    prefix: &str,
) -> Vec<String> {
    let loaded: std::collections::HashSet<(&str, u64)> = loaded.collect();
    let mut missing: Vec<&(String, u64)> = selected
        .iter()
        .filter(|(project, number)| !loaded.contains(&(project.as_str(), *number)))
        .collect();
    missing.sort();
    missing
        .into_iter()
        .map(|(_, number)| format!("{prefix}{number}"))
        .collect()
}

/// GitHub numbers pull requests like issues (`#12`); GitLab uses `!12`.
fn mr_row_prefix(app: &App) -> &'static str {
    if app.is_github() { "#" } else { "!" }
}

fn issue_target(
    app: &App,
    issue: &crate::domain::issues::Issue,
    checkouts: &mut CheckoutCache,
) -> Target {
    let mut values = TemplateValues::default();
    values.insert("IssueNumber", issue.iid.to_string());
    values.insert("IssueTitle", issue.title.clone());
    values.insert("Author", issue.author.username.clone());
    let project = row_project(&issue.project_path, Some(&issue.web_url), &app.scope);
    insert_repo_values(&mut values, app, project, checkouts);
    Target {
        row: format!("#{}", issue.iid),
        values,
    }
}

fn mr_target(
    app: &App,
    mr: &crate::domain::mr::MergeRequest,
    checkouts: &mut CheckoutCache,
) -> Target {
    let mut values = TemplateValues::default();
    insert_mr_values(&mut values, mr);
    let project = row_project(&mr.project_path, mr.web_url.as_deref(), &app.scope);
    insert_repo_values(&mut values, app, project, checkouts);
    Target {
        row: format!("{}{}", mr_row_prefix(app), mr.iid),
        values,
    }
}

/// Values for a diff-view command: the open MR/PR plus the file and line
/// under the cursor.
fn diff_values(app: &App, checkouts: &mut CheckoutCache) -> Result<TemplateValues, String> {
    let mut values = TemplateValues::default();
    let diff_view = app
        .diff_view
        .as_ref()
        .ok_or_else(|| "the diff view is not open".to_string())?;
    let (file_path, line_number) = diff_view
        .cursor_file_position()
        .ok_or_else(|| "the cursor is not on a file".to_string())?;
    values.insert("FilePath", file_path);
    match line_number {
        Some(line) => values.insert("LineNumber", line.to_string()),
        None => values.insert_unavailable(
            "LineNumber",
            "the change leaves no line of this file".to_string(),
        ),
    }
    let mr = app.mrs.items.iter().find(|mr| {
        mr.iid == diff_view.mr_iid
            && (mr.project_path.is_empty()
                || diff_view.project_path.is_empty()
                || mr.project_path == diff_view.project_path)
    });
    match mr {
        Some(mr) => insert_mr_values(&mut values, mr),
        None => {
            values.insert("PrNumber", diff_view.mr_iid.to_string());
            for argument in ["HeadRefName", "BaseRefName", "Author"] {
                values.insert_unavailable(
                    argument,
                    "the MR/PR is not in the loaded list".to_string(),
                );
            }
        }
    }
    let project = if diff_view.project_path.is_empty() {
        mr.and_then(|mr| row_project(&mr.project_path, mr.web_url.as_deref(), &app.scope))
            .or_else(|| {
                app.scope
                    .is_repository()
                    .then(|| app.scope.as_str().to_string())
            })
    } else {
        Some(diff_view.project_path.clone())
    };
    insert_repo_values(&mut values, app, project, checkouts);
    Ok(values)
}

fn insert_repo_values(
    values: &mut TemplateValues,
    app: &App,
    project: Option<String>,
    checkouts: &mut CheckoutCache,
) {
    match project {
        Some(project) => {
            let checkout = checkouts
                .entry(project.clone())
                .or_insert_with(|| local_checkout(&app.scope, &project));
            match checkout {
                Ok(path) => values.insert("RepoPath", path.clone()),
                Err(reason) => values.insert_unavailable("RepoPath", reason.clone()),
            }
            values.insert("RepoName", project);
        }
        None => {
            let reason = format!("the group {} has no single repository", app.scope.as_str());
            values.insert_unavailable("RepoName", reason.clone());
            values.insert_unavailable("RepoPath", reason);
        }
    }
}

fn insert_mr_values(values: &mut TemplateValues, mr: &crate::domain::mr::MergeRequest) {
    values.insert("PrNumber", mr.iid.to_string());
    values.insert("HeadRefName", mr.source_branch.clone());
    values.insert("BaseRefName", mr.target_branch.clone());
    values.insert("Author", mr.author.username.clone());
}

/// `namespace/project` of a row: its own project path, else the one in its
/// web URL, else the repository being browsed.
fn row_project(project_path: &str, web_url: Option<&str>, scope: &Scope) -> Option<String> {
    if !project_path.is_empty() {
        return Some(project_path.to_string());
    }
    web_url
        .and_then(crate::git_helpers::parse_project_path_from_web_url)
        .filter(|project| !project.is_empty())
        .or_else(|| scope.is_repository().then(|| scope.as_str().to_string()))
}

/// Local checkout of `project`. In repository scope that is the checkout
/// glab-tui runs in, even when `gh` points it at an upstream of the fork;
/// in group scope it is the recently used checkout whose remote matches.
fn local_checkout(scope: &Scope, project: &str) -> Result<String, String> {
    if scope.is_repository() {
        return crate::git_helpers::repo_root()
            .ok_or_else(|| "the working directory is not inside a git checkout".to_string());
    }
    crate::utils::cache::find_local_checkout(project).ok_or_else(|| {
        format!(
            "no local checkout of {project} is known; open it once with the repository switcher"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mr(json: &str) -> crate::domain::mr::MergeRequest {
        serde_json::from_str(json).expect("merge request fixture")
    }

    fn issue(iid: u64, title: &str) -> crate::domain::issues::Issue {
        serde_json::from_value(serde_json::json!({
            "iid": iid, "title": title, "state": "opened", "updated_at": "",
            "author": {"username": "erin"}
        }))
        .expect("issue fixture")
    }

    /// The values of a command that acts on exactly one target.
    fn single_values(app: &App, pane: CommandPane) -> Result<TemplateValues, String> {
        let mut selection = targets(app, pane)?;
        assert_eq!(selection.targets.len(), 1, "expected a single target");
        Ok(selection.targets.remove(0).values)
    }

    #[cfg(unix)]
    #[test]
    fn failing_command_reports_its_last_stderr_line() {
        let command = CustomCommand {
            pane: CommandPane::Universal,
            key: "X".to_string(),
            name: Some("worktree".to_string()),
            command: String::new(),
            background: false,
            shadowed_on: Vec::new(),
        };
        let mut process = shell_process(
            "echo progress >&2; echo 'failed to fetch issue: issue #475 is not open' >&2; exit 1",
        );

        let error = run_process(&command, &mut process, Handoff::Background).unwrap_err();
        assert!(
            error.ends_with(": failed to fetch issue: issue #475 is not open"),
            "{error}"
        );
        assert!(
            run_process(
                &command,
                &mut shell_process("echo fine >&2"),
                Handoff::Background
            )
            .is_ok()
        );
    }

    #[cfg(unix)]
    #[test]
    fn background_process_runs_in_its_own_session() {
        let mut process = std::process::Command::new("sleep");
        process.arg("1");
        detach_from_terminal(&mut process);
        let mut child = process.spawn().unwrap();
        let pid = child.id() as libc::pid_t;

        let session = unsafe { libc::getsid(pid) };
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(
            session, pid,
            "the child must lead a new session, detached from the tty"
        );
    }

    /// A descendant that keeps stderr open (`tool &`, a daemon) must not hold
    /// the report back until it exits.
    #[cfg(unix)]
    #[test]
    fn background_run_does_not_wait_for_descendants_holding_stderr() {
        let command = CustomCommand {
            pane: CommandPane::Universal,
            key: "X".to_string(),
            name: None,
            command: String::new(),
            background: true,
            shadowed_on: Vec::new(),
        };
        let started = std::time::Instant::now();
        let outcome = run_process(
            &command,
            &mut shell_process("sleep 5 & echo 'boom' >&2; exit 1"),
            Handoff::Background,
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "took {:?}",
            started.elapsed()
        );
        assert!(outcome.unwrap_err().ends_with(": boom"));
    }

    #[test]
    fn selected_issues_each_get_a_run_in_number_order() {
        let mut app = App::default();
        app.scope = Scope::Repository("owner/repo".to_string());
        app.issues.items = vec![issue(30, "c"), issue(10, "a"), issue(20, "b")];
        app.issues.state.select(Some(0));
        app.selected_issues = [(String::new(), 30), (String::new(), 10)]
            .into_iter()
            .collect();

        let targets = targets(&app, CommandPane::Issues).unwrap().targets;
        let rendered: Vec<String> = targets
            .iter()
            .map(|target| render("wt {{.IssueNumber}}", &target.values).unwrap())
            .collect();
        assert_eq!(
            rendered,
            vec!["wt 10", "wt 30"],
            "selection wins over the highlighted #20"
        );
        assert_eq!(
            targets.iter().map(|t| t.row.as_str()).collect::<Vec<_>>(),
            vec!["#10", "#30"]
        );
    }

    #[test]
    fn selection_that_is_no_longer_loaded_is_refused() {
        let mut app = App::default();
        app.scope = Scope::Repository("owner/repo".to_string());
        app.issues.items = vec![issue(1, "a")];
        app.selected_issues = [(String::new(), 99)].into_iter().collect();

        assert_eq!(
            targets(&app, CommandPane::Issues).err(),
            Some("the selected issues are no longer loaded".to_string())
        );

        app.selected_issues.insert((String::new(), 1));
        let selection = targets(&app, CommandPane::Issues).unwrap();
        assert_eq!(selection.targets.len(), 1);
        assert_eq!(
            selection.missing_rows,
            vec!["#99"],
            "vanished rows are reported"
        );
    }

    #[test]
    fn mr_values_come_from_the_highlighted_row() {
        let mut app = App::default();
        app.scope = Scope::Repository("owner/repo".to_string());
        app.mrs.items = vec![
            mr(
                r#"{"iid": 1, "title": "one", "state": "opened", "updated_at": "",
                   "author": {"username": "alice"}, "target_branch": "main",
                   "source_branch": "feat/one", "draft": false}"#,
            ),
            mr(
                r#"{"iid": 2, "title": "two", "state": "opened", "updated_at": "",
                   "author": {"username": "bob"}, "target_branch": "release",
                   "source_branch": "fix/two", "draft": false}"#,
            ),
        ];
        app.mrs.state.select(Some(1));

        let values = single_values(&app, CommandPane::MergeRequests).unwrap();
        assert_eq!(
            render(
                "{{.RepoName}} !{{.PrNumber}} {{.BaseRefName}}...{{.HeadRefName}} by {{.Author}}",
                &values
            )
            .as_deref(),
            Ok("owner/repo !2 release...fix/two by bob")
        );
    }

    #[test]
    fn diff_values_come_from_the_cursor_and_the_open_mr() {
        let mut app = App::default();
        app.scope = Scope::Repository("owner/repo".to_string());
        app.mrs.items = vec![mr(r#"{"iid": 7, "title": "t", "state": "opened",
            "updated_at": "", "author": {"username": "dan"}, "target_branch": "main",
            "source_branch": "feat/x", "draft": false}"#)];
        let mut view = crate::app::DiffView::new(
            7,
            "owner/repo".to_string(),
            "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n\
             @@ -4,2 +4,3 @@\n four\n+five\n six\n"
                .to_string(),
        );
        view.focus_on_files = false;
        view.cursor_idx = view
            .lines
            .iter()
            .position(|l| l.content == "+five")
            .unwrap();
        app.diff_view = Some(view);

        let values = single_values(&app, CommandPane::Diff).unwrap();
        assert_eq!(
            render(
                "{{.RepoName}} !{{.PrNumber}} {{.HeadRefName}} {{.FilePath}}:{{.LineNumber}}",
                &values
            )
            .as_deref(),
            Ok("owner/repo !7 feat/x src/a.rs:5")
        );
    }

    #[test]
    fn row_command_without_a_selected_row_is_refused() {
        let mut app = App::default();
        app.scope = Scope::Repository("owner/repo".to_string());
        app.mrs.items.clear();
        app.mrs.state.select(None);

        assert_eq!(
            single_values(&app, CommandPane::MergeRequests),
            Err("no MR is selected".to_string())
        );
    }

    #[test]
    fn group_scope_issue_takes_its_repo_name_from_the_web_url() {
        let mut app = App::default();
        app.scope = Scope::Group("acme".to_string());
        app.issues.items = vec![
            serde_json::from_str(
                r#"{"iid": 9, "title": "Crash", "state": "opened", "updated_at": "",
                    "author": {"username": "carol"},
                    "web_url": "https://gitlab.com/acme/sub/tool/-/issues/9"}"#,
            )
            .unwrap(),
        ];
        app.issues.state.select(Some(0));

        let values = single_values(&app, CommandPane::Issues).unwrap();
        assert_eq!(
            render("{{.RepoName}}#{{.IssueNumber}} {{.IssueTitle}}", &values).as_deref(),
            Ok("acme/sub/tool#9 Crash")
        );
    }

    #[test]
    fn universal_command_in_group_scope_has_no_repository() {
        let mut app = App::default();
        app.scope = Scope::Group("acme".to_string());

        let values = single_values(&app, CommandPane::Universal).unwrap();
        assert_eq!(
            render("cd {{.RepoPath}}", &values),
            Err(
                "{{.RepoPath}} is unavailable: the group acme has no single repository".to_string()
            )
        );
    }
}
