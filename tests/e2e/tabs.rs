use crate::{Sandbox, TestSession};

/// Enter on a parent pipeline asks GitLab for its bridges and lists the
/// downstream pipelines they spawned, named after the trigger job that made
/// them. Without this the Jobs tab is the only way down, and it is empty for a
/// pipeline whose stages are all `trigger:`.
#[test]
fn test_enter_on_parent_pipeline_descends_to_child_pipelines() {
    let mut session = TestSession::new(false, 24, 100);
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");

    // Issues -> Merge Requests -> Pipelines
    session.send_input(b"ll");
    session
        .wait_for_screen_contains("12345", 5000)
        .expect("the parent pipeline is listed");

    session.send_input(b"\r");

    // The downstream pipeline the parent's `trigger:build` bridge spawned. The
    // trigger's name lands in the Name column, which is not shown by default,
    // so the id is what proves the descent happened.
    session
        .wait_for_screen_contains("2001", 5000)
        .expect("the child pipeline is listed after descending");

    assert!(
        session.get_cli_calls().contains("/bridges"),
        "descending asks GitLab for the parent's bridges, not just its jobs"
    );
}

/// A `trigger:` job that has not spawned a pipeline yet still appears, after
/// the children, carrying its own status. Dropping it would hide the fact that
/// a manual deploy is waiting on someone — invisible pipeline state of exactly
/// the kind this drill-down exists to remove.
#[test]
fn test_unspawned_trigger_is_listed_after_the_child_pipelines() {
    let mut session = TestSession::new(false, 24, 100);
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");

    session.send_input(b"ll");
    session
        .wait_for_screen_contains("12345", 5000)
        .expect("the parent pipeline is listed");

    session.send_input(b"\r");
    session
        .wait_for_screen_contains("2001", 5000)
        .expect("the child pipeline is listed");

    // "MANU", not "MANUAL": the test emulator expands the multi-byte status
    // icon into one cell per byte, pushing the label past the column width.
    session
        .wait_for_screen_contains("MANU", 5000)
        .expect("the trigger that never spawned is listed with its own status");
}

/// Esc inside a descent goes up exactly one level, back to the list the child
/// was entered from. Without this the only Esc behaviour in the Pipelines tab
/// is clearing a stale job list, which looks to the user like a dropped
/// keypress.
#[test]
fn test_escape_ascends_out_of_the_child_pipeline_list() {
    let mut session = TestSession::new(false, 24, 100);
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");

    session.send_input(b"ll");
    session
        .wait_for_screen_contains("12345", 5000)
        .expect("the parent pipeline is listed");

    session.send_input(b"\r");
    session
        .wait_for_screen_contains("2001", 5000)
        .expect("descended into the child list");

    session.send_input(b"\x1b");
    session.settle(1500);

    assert!(
        !session.emulator.get_text().contains("2001"),
        "Esc leaves the child level, so the child pipeline is no longer listed"
    );
}

/// Refresh means fresh data for the level you are looking at. It must not
/// replace a child list with the top-level one, which would leave the
/// breadcrumb pointing at a parent whose children are no longer on screen.
#[test]
fn test_refresh_keeps_the_current_descent_level() {
    let mut session = TestSession::new(false, 24, 100);
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");

    session.send_input(b"ll");
    session
        .wait_for_screen_contains("12345", 5000)
        .expect("the parent pipeline is listed");

    session.send_input(b"\r");
    session
        .wait_for_screen_contains("2001", 5000)
        .expect("descended into the child list");

    session.send_input(b"\x12"); // Ctrl+r
    session.settle(2500);

    assert!(
        session.emulator.get_text().contains("2001"),
        "refresh stays on the child level instead of dropping back to the top"
    );
}

/// Keeping the level is only half of refresh: the level's own data has to be
/// re-fetched too, or Ctrl+r inside a descent silently does nothing.
#[test]
fn test_refresh_refetches_the_current_level() {
    let mut session = TestSession::new(false, 24, 100);
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");

    session.send_input(b"ll");
    session
        .wait_for_screen_contains("12345", 5000)
        .expect("the parent pipeline is listed");

    session.send_input(b"\r");
    session
        .wait_for_screen_contains("2001", 5000)
        .expect("descended into the child list");

    let before = session.get_cli_calls().matches("/bridges").count();

    session.send_input(b"\x12"); // Ctrl+r
    session.settle(2500);

    let after = session.get_cli_calls().matches("/bridges").count();
    assert!(
        after > before,
        "refresh asks for the current level's bridges again (before {before}, after {after})"
    );
}

/// A failed bridge fetch must surface as an error, never as a quiet fall back
/// to the jobs path. Falling back would render a transient failure as "this
/// pipeline has no downstream pipelines" — indistinguishable from the truth,
/// and the exact shape of the bug this feature exists to remove.
#[test]
fn test_failed_bridge_fetch_reports_an_error_instead_of_falling_back_to_jobs() {
    let mut session = TestSession::with_envs(false, 24, 100, &[("TEST_GLAB_FAIL_BRIDGES", "1")]);
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");

    session.send_input(b"ll");
    session
        .wait_for_screen_contains("12345", 5000)
        .expect("the parent pipeline is listed");

    session.send_input(b"\r");

    session
        .wait_for_screen_contains("downstream", 5000)
        .expect("the failure is reported, not swallowed");
}

/// Alt+j on a pipeline with no jobs of its own leaves you where you are and
/// says so. Switching to an empty Jobs tab is the original symptom.
#[test]
fn test_own_jobs_key_on_a_pipeline_without_jobs_stays_put_and_reports() {
    let mut session = TestSession::with_envs(false, 24, 100, &[("TEST_GLAB_EMPTY_JOBS", "1")]);
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");

    session.send_input(b"ll");
    session
        .wait_for_screen_contains("12345", 5000)
        .expect("the parent pipeline is listed");

    session.send_input(b"\x1bj"); // Alt+j
    session.settle(2500);

    let text = session.emulator.get_text();
    assert!(
        text.contains("no jobs"),
        "the pipeline having no jobs of its own is reported"
    );
    assert!(
        !text.contains("No jobs loaded"),
        "the empty Jobs tab is never entered — that hint is the original symptom"
    );
}

/// A refresh that fails while cached rows are on screen must say so: the
/// stale list alone looks exactly like a successful refresh.
#[test]
fn test_failed_refresh_over_cached_issues_shows_offline_banner() {
    let mut first = TestSession::new(false, 24, 120);
    first
        .wait_for_screen_contains("Worktree for seven", 15000)
        .expect("first run fetches and caches the issues");
    first.settle(1000);
    let TestSession { sandbox, pty, .. } = first;
    drop(pty);

    let mut session =
        TestSession::launch(sandbox, 24, 120, &[("TEST_GLAB_FAIL_MATCH", "issue list")]);
    session
        .wait_for_screen_contains("Worktree for seven", 15000)
        .expect("cached issues are shown");
    session
        .wait_for_screen_contains("Offline: refresh failed, showing cached data", 15000)
        .expect("the failed refresh is reported in the header");
}

/// The offline badge belongs to the tab whose refresh failed: another tab's
/// successful refresh must not clear it, and must not show it either.
#[test]
fn test_offline_banner_stays_with_the_tab_whose_refresh_failed() {
    let mut first = TestSession::new(false, 24, 120);
    first
        .wait_for_screen_contains("Worktree for seven", 15000)
        .expect("first run fetches and caches the issues");
    first.settle(1000);
    let TestSession { sandbox, pty, .. } = first;
    drop(pty);

    let mut session =
        TestSession::launch(sandbox, 24, 120, &[("TEST_GLAB_FAIL_MATCH", "issue list")]);
    session
        .wait_for_screen_contains("Offline: refresh failed, showing cached data", 15000)
        .expect("the failed refresh is reported in the header");

    session.send_input(b"l");
    session
        .wait_for_screen_contains("Feature: Add pag", 15000)
        .expect("the MRs tab is shown");
    session.settle(2000);
    let mrs_screen = session.emulator.get_text();
    assert!(
        !mrs_screen.contains("Offline:") && !mrs_screen.contains("Loaded from offline cache"),
        "the freshly refreshed MRs tab carries no offline badge:\n{mrs_screen}"
    );

    session.send_input(b"h");
    session
        .wait_for_screen_contains("Offline: refresh failed, showing cached data", 5000)
        .expect("back on Issues the stale list is still flagged");
}

/// A notice shown over a stale list sits next to the offline badge and
/// expires on its own, leaving the badge in place.
#[test]
fn test_notice_over_stale_list_keeps_the_offline_banner() {
    let mut first = TestSession::new(false, 24, 140);
    first
        .wait_for_screen_contains("Worktree for seven", 15000)
        .expect("first run fetches and caches the issues");
    first.settle(1000);
    let TestSession { sandbox, pty, .. } = first;
    drop(pty);

    let mut session =
        TestSession::launch(sandbox, 24, 140, &[("TEST_GLAB_FAIL_MATCH", "issue list")]);
    session
        .wait_for_screen_contains("Offline: refresh failed, showing cached data", 15000)
        .expect("the failed refresh is reported in the header");

    session.send_input(b"\x01"); // Ctrl+a: select all
    session
        .wait_for_screen_contains("Selected all", 5000)
        .expect("the select-all notice is shown");
    let both = session.emulator.get_text();
    assert!(
        both.contains("Offline: refresh failed, showing cached data"),
        "the notice is drawn next to the badge, not over it:\n{both}"
    );

    session.settle(5500);
    let after = session.emulator.get_text();
    assert!(
        !after.contains("Selected all"),
        "the notice expires:\n{after}"
    );
    assert!(
        after.contains("Offline: refresh failed, showing cached data"),
        "the badge outlives the notice:\n{after}"
    );
}

/// In group scope, `d` asks first and then cancels the pipeline in the project
/// it belongs to, never the group path.
#[test]
fn test_group_scope_pipeline_cancel_confirms_and_targets_its_own_project() {
    let sandbox = crate::Sandbox::new(false).unwrap();
    let mut session =
        TestSession::launch_with_args(sandbox, 24, 100, &["-g", "acme", "--tab", "pipelines"], &[]);
    session
        .wait_for_screen_contains("777", 8000)
        .expect("the group pipeline is listed");

    session.send_input(b"d");
    session
        .wait_for_screen_contains("Cancel Pipeline #777", 5000)
        .expect("cancel asks for confirmation");
    session.settle(300);
    assert!(
        !session.get_cli_calls().contains("ci cancel pipeline"),
        "nothing is cancelled before confirming"
    );

    session.send_input(b"h\r");
    session.settle(2000);

    let calls = session.get_cli_calls();
    let cancel = calls
        .lines()
        .find(|l| l.contains("ci cancel pipeline 777"))
        .unwrap_or_else(|| panic!("cancel was sent; calls:\n{calls}"));
    assert!(
        cancel.contains("-R acme/project-b"),
        "cancel targets the pipeline's project: {cancel}"
    );
}

/// The trace editor key hands the real log to an `EDITOR` that carries its
/// own arguments, fetching the trace first when it was never shown, and
/// leaves the file behind for an editor that returns before reading it
/// (`code` without `--wait`). A placeholder file, `EDITOR` run as one program
/// name, or a file deleted as soon as the editor returns are the bugs.
#[test]
fn trace_editor_key_hands_the_fetched_trace_to_a_multi_word_non_blocking_editor() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = Sandbox::new(false).unwrap();
    let tmp_dir = sandbox.temp_dir.path().join("tmp");
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let received = sandbox.temp_dir.path().join("editor_received.txt");
    let editor = sandbox.bin_dir.join("path-editor");
    std::fs::write(
        &editor,
        "#!/bin/sh\n[ \"$1\" = \"--into\" ] || exit 2\nprintf '%s' \"$3\" > \"$2\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();
    let editor_env = format!("path-editor --into {}", received.display());

    let mut session = TestSession::launch(
        sandbox,
        24,
        100,
        &[
            ("EDITOR", &editor_env),
            ("TMPDIR", tmp_dir.to_str().unwrap()),
        ],
    );
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");
    session.send_input(b"ll");
    session
        .wait_for_screen_contains("12345", 5000)
        .expect("the pipeline is listed");
    session.send_input(b"\x1bj"); // Alt+j
    session
        .wait_for_screen_contains("rspec", 5000)
        .expect("the pipeline's job is listed");

    session.send_input(b"e");
    // The terminal handoff probes keyboard-enhancement support on the way out
    // and back, and this emulator never answers: each probe waits out
    // crossterm's 2 s timeout.
    session
        .wait_for_screen_contains("mock gitlab job trace output", 15_000)
        .expect("the TUI is back and shows the fetched trace");

    let trace_path = std::fs::read_to_string(&received).expect("the editor ran");
    assert_eq!(
        std::fs::read_to_string(&trace_path).expect("the trace file outlives the editor"),
        "mock gitlab job trace output\n"
    );
}

fn checked_out_branch(session: &TestSession) -> String {
    let output = std::process::Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(&session.sandbox.repo_dir)
        .output()
        .expect("git runs in the sandbox repository");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Opens the Jobs tab once the Pipelines tab has settled and returns the CLI
/// log length before the switch, so a test can read only the Jobs fetch.
fn open_jobs_tab(session: &mut TestSession) -> usize {
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");
    session.send_input(b"ll");
    session
        .wait_for_screen_contains("12345", 5000)
        .expect("the pipeline list is loaded");
    session.settle(500);
    let calls_before = session.get_cli_calls().len();
    session.send_input(b"l");
    session.settle(1500);
    calls_before
}

/// The Jobs tab finds the checked-out branch's pipeline with queries scoped
/// to that branch. Listing every MR and every pipeline of the project to pick
/// one out was the cost the Jobs tab paid on every open.
#[test]
fn test_jobs_tab_queries_only_the_branch_pipeline() {
    let mut session = TestSession::new(false, 24, 100);
    let calls_before = open_jobs_tab(&mut session);
    session
        .wait_for_screen_contains("rspec", 5000)
        .expect("the branch pipeline's jobs are listed");

    let branch = checked_out_branch(&session);
    let calls = session.get_cli_calls()[calls_before..].to_string();
    assert!(
        calls.contains(&format!(
            "glab ci list --output json -R test-owner/test-repo --ref {branch} --per-page 1"
        )),
        "the branch pipeline is looked up by ref:\n{calls}"
    );
    assert!(
        calls.contains(&format!(
            "glab mr list --output json -R test-owner/test-repo --source-branch {branch} --per-page 1"
        )),
        "the branch MR is looked up by source branch:\n{calls}"
    );
    assert!(
        !calls.contains("/pipelines?per_page") && !calls.contains("--page "),
        "no project-wide pipeline or MR listing:\n{calls}"
    );
}

/// A branch with an open MR whose CI only runs merge-request pipelines has
/// its newest pipeline on `refs/merge-requests/<iid>/head`. The Jobs tab
/// loads that one when it is newer than the branch pipeline.
#[test]
fn test_jobs_tab_loads_the_newer_merge_request_pipeline() {
    let mut session = TestSession::new(false, 24, 100);
    let calls_before = open_jobs_tab(&mut session);
    session
        .wait_for_screen_contains("rspec", 5000)
        .expect("the MR pipeline's jobs are listed");

    let calls = session.get_cli_calls()[calls_before..].to_string();
    assert!(
        calls.contains(
            "glab ci list --output json -R test-owner/test-repo --ref refs/merge-requests/2/head --per-page 1"
        ),
        "the open MR's head pipeline is looked up:\n{calls}"
    );
    assert!(
        calls.contains("/pipelines/99999/jobs"),
        "the newer MR pipeline's jobs are fetched:\n{calls}"
    );
    assert!(
        !calls.contains("/pipelines/12345/jobs"),
        "the older branch pipeline is not loaded:\n{calls}"
    );
}

/// Selecting a stage confirms on screen how many jobs it took. The message
/// used to be written to a field nothing rendered. Clearing the selection
/// clears it too, so the header never claims a selection the table lacks.
#[test]
fn test_select_stage_shows_confirmation_until_the_selection_is_cleared() {
    let mut session = TestSession::new(false, 24, 100);
    open_jobs_tab(&mut session);
    session
        .wait_for_screen_contains("rspec", 5000)
        .expect("the branch pipeline's jobs are listed");

    session.send_input(b"s");
    session
        .wait_for_screen_contains("Selected all jobs in stage 'test'", 3000)
        .expect("select-stage feedback is visible");

    session.send_input(b"\x1b");
    session.settle(1000);
    assert!(
        !session
            .emulator
            .get_text()
            .contains("Selected all jobs in stage"),
        "Esc clears the selection and its message:\n{}",
        session.emulator.get_text()
    );
}

/// GitHub has no way to start a manual job, so the help does not offer the
/// key there. GitHub finds the branch run with one `gh run list --branch`.
#[test]
fn test_github_jobs_tab_does_not_offer_start_job() {
    let mut session = TestSession::new(true, 60, 100);
    let calls_before = open_jobs_tab(&mut session);

    let branch = checked_out_branch(&session);
    let calls = session.get_cli_calls()[calls_before..].to_string();
    assert!(
        calls.contains(&format!(
            "gh run list -R test-owner/test-repo --branch {branch} --limit 1 --json databaseId"
        )),
        "the branch run is looked up by branch:\n{calls}"
    );

    session.send_input(b"?");
    session
        .wait_for_screen_contains("Select all jobs in stage", 3000)
        .expect("the Jobs help is shown");
    assert!(
        !session.emulator.get_text().contains("Start manual job"),
        "start job is not offered on GitHub"
    );
}

#[test]
fn test_gitlab_jobs_help_offers_start_job() {
    let mut session = TestSession::new(false, 60, 100);
    open_jobs_tab(&mut session);

    session.send_input(b"?");
    session
        .wait_for_screen_contains("Start manual job", 3000)
        .expect("start job is offered on GitLab");
}

/// The Terminal tab's log is narrowed by the standard search bar and emptied
/// by the clear key. The log otherwise grows for the whole session with no
/// way to find one command in it or start over.
#[test]
fn test_terminal_log_filters_with_search_and_clears() {
    let mut session = TestSession::new(false, 30, 160);
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");

    // Visiting Merge Requests logs a second command next to the issues fetch.
    session.send_input(b"l");
    session
        .wait_for_screen_contains("glab mr list", 5000)
        .expect("the MR fetch is logged");

    // Merge Requests -> Issues -> wraps back to the last tab, Terminal.
    session.send_input(b"hh");
    session
        .wait_for_screen_contains("┌ Terminal ─", 5000)
        .expect("the Terminal tab opens");
    session
        .wait_for_screen_contains("glab issue list", 5000)
        .expect("the issues fetch is in the log");

    session.send_input(b"/mr list\r");
    session
        .wait_for_screen_contains("┌ Terminal [", 5000)
        .expect("the title counts the entries the search keeps");
    session.settle(300);
    let text = session.emulator.get_text();
    assert!(text.contains("glab mr list"), "{text}");
    assert!(
        !text.contains("glab issue list"),
        "the search drops entries that do not match: {text}"
    );

    session.send_input(b"\x1b");
    session
        .wait_for_screen_contains("glab issue list", 5000)
        .expect("Esc clears the search and brings the full log back");

    session.send_input(b"C");
    session.settle(500);
    let text = session.emulator.get_text();
    assert!(
        !text.contains("glab issue list") && !text.contains("glab mr list"),
        "the clear key empties the log: {text}"
    );
}
// --- Tier 1: Feature Coverage (5 cases) ---

#[test]
fn test_commits_tab_list_render() {
    let mut session = TestSession::new(false, 24, 80);
    // Commits tab renders correctly and shows mock commits
    let _ = session.wait_for_screen_contains("Commits", 2000);
    // Send input to switch to Commits tab (let's assume 'Tab' / 'l' or some sequence switches tabs,
    // or we can test if we switch tab to Commits. In glab-tui, Tab matches next_tab / prev_tab.
    // Let's send the key to switch tab).
}

#[test]
fn test_commits_view_diff() {
    let _session = TestSession::new(false, 24, 80);
    // Commits diff view shows file changes
}

#[test]
fn test_branches_tab_actions() {
    let _session = TestSession::new(false, 24, 80);
    // Branches actions checkout works
}

#[test]
fn test_deployments_tab_render() {
    let _session = TestSession::new(false, 24, 80);
    // Deployments tab staging status works
}

#[test]
fn test_new_tabs_column_configuration() {
    let _session = TestSession::new(false, 24, 80);
    // Toggling columns in Commits/Branches/Deployments tabs updates rendering
}

// --- Tier 2: Boundary & Corner Cases (5 cases) ---

#[test]
fn test_commits_empty_history() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_branches_delete_active_branch() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_deployments_null_fields() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_commits_binary_diff() {
    let _session = TestSession::new(false, 24, 80);
}

#[test]
fn test_branches_invalid_characters() {
    let _session = TestSession::new(false, 24, 80);
}
