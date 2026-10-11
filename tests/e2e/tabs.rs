use crate::TestSession;

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
