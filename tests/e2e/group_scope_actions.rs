use crate::{Sandbox, TestSession};

/// In group scope two projects can each have issue/MR #5. Confirming an
/// action on the second row must reach that row's project, not the first
/// project listed with the same iid.
fn confirm_on_second_row(tab_keys: &[u8], first_title: &str, close_key: &[u8]) -> TestSession {
    let mut session = TestSession::launch_with_args(
        Sandbox::new(false).unwrap(),
        30,
        140,
        &["-g", "test-group"],
        &[],
    );
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");
    session.send_input(tab_keys);
    session
        .wait_for_screen_contains(first_title, 5000)
        .expect("both rows are listed");

    session.send_input(b"j");
    session.settle(200);
    session.send_input(close_key);
    session
        .wait_for_screen_contains("Close", 5000)
        .expect("the confirm dialog opens");
    // Destructive dialogs open on Cancel; move to Submit and confirm.
    session.send_input(b"h");
    session.send_input(b"\r");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !session.get_cli_calls().contains(" close 5 -R ") {
        assert!(
            std::time::Instant::now() < deadline,
            "no close call; calls:\n{}",
            session.get_cli_calls()
        );
        session.settle(20);
    }
    session
}

#[test]
fn closing_an_issue_in_group_scope_targets_the_selected_rows_project() {
    let session = confirm_on_second_row(b"", "Alpha crash", b"c");
    let calls = session.get_cli_calls();
    assert!(
        calls.contains("glab issue close 5 -R test-group/beta"),
        "close goes to the selected row's project; calls:\n{calls}"
    );
    assert!(
        !calls.contains("glab issue close 5 -R test-group/alpha"),
        "the other project's issue #5 is untouched; calls:\n{calls}"
    );
}

#[test]
fn closing_an_mr_in_group_scope_targets_the_selected_rows_project() {
    let session = confirm_on_second_row(b"l", "Alpha feature", b"c");
    let calls = session.get_cli_calls();
    assert!(
        calls.contains("glab mr close 5 -R test-group/beta"),
        "close goes to the selected row's project; calls:\n{calls}"
    );
    assert!(
        !calls.contains("glab mr close 5 -R test-group/alpha"),
        "the other project's MR !5 is untouched; calls:\n{calls}"
    );
}

#[test]
fn gitlab_group_scope_pipelines_tab_shows_placeholder_and_requests_no_pipelines() {
    let mut session = TestSession::launch_with_args(
        Sandbox::new(false).unwrap(),
        30,
        140,
        &["-g", "test-group", "--tab", "pipelines"],
        &[],
    );
    session
        .wait_for_screen_contains("Pipelines tab is not available in group scope", 8000)
        .expect("the tab shows the group-scope placeholder");
    session.settle(1500);

    let screen = session.emulator.get_text();
    assert!(
        !screen.contains("Group-level pipelines not supported")
            && !screen.contains("Failed to fetch pipelines"),
        "no error toast: {screen}"
    );
    let calls = session.get_cli_calls();
    assert!(
        !calls
            .lines()
            .any(|l| l.contains("ci list") || l.contains("/pipelines")),
        "no pipeline is requested; calls:\n{calls}"
    );
}

/// Opens `tab` in the group or organization `group` and returns the calls
/// made once its placeholder is drawn.
fn calls_behind_group_scope_placeholder(is_github: bool, group: &str, tab: &str) -> String {
    let mut session = TestSession::launch_with_args(
        Sandbox::new(is_github).unwrap(),
        30,
        140,
        &["-g", group, "--tab", tab],
        &[],
    );
    let title = format!("{}{}", tab[..1].to_uppercase(), &tab[1..]);
    session
        .wait_for_screen_contains(
            &format!("{title} tab is not available in group scope"),
            8000,
        )
        .expect("the tab shows the group-scope placeholder");
    session.settle(1500);
    let screen = session.emulator.get_text();
    assert!(
        !screen.contains("not supported") && !screen.contains("Failed to fetch"),
        "no error toast: {screen}"
    );
    session.get_cli_calls()
}

#[test]
fn gitlab_group_scope_branches_tab_requests_no_branches() {
    let calls = calls_behind_group_scope_placeholder(false, "test-group", "branches");
    assert!(
        !calls.lines().any(|l| l.contains("/branches")),
        "no branch is requested; calls:\n{calls}"
    );
}

#[test]
fn github_org_scope_milestones_tab_requests_no_milestones() {
    let calls = calls_behind_group_scope_placeholder(true, "acme", "milestones");
    assert!(
        !calls.lines().any(|l| l.contains("/milestones")),
        "no milestone is requested; calls:\n{calls}"
    );
}

#[test]
fn own_jobs_key_in_github_org_scope_fetches_from_the_runs_repository() {
    let mut session = TestSession::launch_with_args(
        Sandbox::new(true).unwrap(),
        30,
        140,
        &["-g", "acme", "--tab", "pipelines"],
        &[],
    );
    session
        .wait_for_screen_contains("12345", 5000)
        .expect("the org's run is listed");
    // Highlighting a run already fetches its jobs for the preview, so only
    // the calls made after Alt+j count.
    let row_project_jobs = "run view 12345 --json jobs --jq .jobs -R acme/project-b";
    session
        .wait_for_cli_call(row_project_jobs, 5000)
        .expect("the preview fetches the highlighted run's jobs");
    let before = session.get_cli_calls().matches(row_project_jobs).count();

    session.send_input(b"\x1bj"); // Alt+j
    session
        .wait_for_cli_call_count(row_project_jobs, before + 1, 5000)
        .expect("Alt+j fetches jobs from the run's repository");
    session.settle(300);

    let calls = session.get_cli_calls();
    assert!(
        !calls
            .lines()
            .any(|l| l.contains("--jq .jobs") && l.ends_with("-R acme")),
        "the org is not a repository; calls:\n{calls}"
    );
}

/// Group milestones live under `/groups/<group>/-/milestones/`, which only
/// the milestone's `web_url` names.
#[test]
fn opening_a_group_milestone_uses_its_web_url() {
    let mut session = TestSession::launch_with_args(
        Sandbox::new(false).unwrap(),
        30,
        140,
        &["-g", "test-group"],
        &[],
    );
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");
    session.press_keys(&b"l".repeat(7));
    session
        .wait_for_screen_contains("Group Roadmap", 5000)
        .expect("the group milestone is listed");

    session.press_keys(b"o");
    session
        .wait_for_cli_call(
            "browser https://gitlab.com/groups/test-group/-/milestones/3\n",
            5000,
        )
        .expect("the group milestone page should open in the browser");
}

/// GitHub lists an org's issues and PRs through the search API, whose reply
/// wraps them in `items`.
#[test]
fn github_org_scope_lists_issues_and_prs_from_search() {
    let mut session =
        TestSession::launch_with_args(Sandbox::new(true).unwrap(), 30, 140, &["-g", "acme"], &[]);
    session
        .wait_for_screen_contains("Org issue in project-b", 5000)
        .expect("the org's issue is listed");
    session.send_input(b"l");
    session
        .wait_for_screen_contains("Org PR in project-b", 5000)
        .expect("the org's PR is listed");
}
