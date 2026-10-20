//! End-to-end coverage for the Environments detail pane: deployments fetched
//! for one environment never stay on screen once another environment is
//! highlighted, the listed deployments can be walked and inspected, and the
//! rollback and stop actions reach the CLI only after a confirmation.
//!
//! The mock `glab` serves `tests/fixtures/environments.json` (`production`,
//! then `staging`) for the environments list and
//! `tests/fixtures/deployments.json` for every deployments request: the
//! newest deployment has SHA `f00dcafe`, which nothing else on the
//! Environments tab shows, and the older one, run by job 500, carries the
//! note `Hotfix rollout for the login outage`.
//!
//! On GitHub the mock `gh` serves `tests/fixtures/gh_environments.json` and
//! `tests/fixtures/gh_deployments.json` (deployment 100, then 99), and the
//! newest status of deployment 100 is `failure` while 99's is `success`.

use crate::TestSession;

const DEPLOYMENT_SHA: &str = "f00dcafe";
/// Long enough for several redraws, so a stale deployments table would have
/// been painted by the time the screen is checked for its absence.
const REDRAW_WINDOW_MS: u64 = 500;

fn session_showing_production_deployments() -> TestSession {
    let mut session =
        TestSession::with_config(false, 40, 160, Some("active_tab = \"environments\"\n"));
    session
        .wait_for_screen_contains("staging", 30000)
        .expect("the Environments tab lists the fixture environments");
    session.send_input(b"\r");
    session
        .wait_for_screen_contains(DEPLOYMENT_SHA, 15000)
        .expect("Enter shows the deployments of the highlighted environment");
    session
}

fn assert_deployments_hidden(session: &mut TestSession) {
    session.settle(REDRAW_WINDOW_MS);
    let screen = session.emulator.get_text();
    assert!(
        !screen.contains(DEPLOYMENT_SHA),
        "deployments of production are still shown:\n{screen}"
    );
}

#[test]
fn selecting_another_environment_clears_the_shown_deployments() {
    let mut session = session_showing_production_deployments();

    // Enter focused the deployments; Esc hands j back to the environment list.
    session.press_keys(b"\x1b");
    session.press_keys(b"j");
    session
        .wait_for_screen_contains("https://staging.example.com", 5000)
        .expect("the detail pane falls back to the staging environment's details");
    assert_deployments_hidden(&mut session);

    session.send_input(b"k");
    session
        .wait_for_screen_contains("https://production.example.com", 5000)
        .expect("returning to production shows its details, not its old deployments");
    assert_deployments_hidden(&mut session);
}

const OLDER_DEPLOYMENT_NOTE: &str = "Hotfix rollout for the login outage";

fn confirm_dialog(session: &mut TestSession, title: &str) {
    session
        .wait_for_screen_contains(title, 5000)
        .expect("the action should ask for confirmation");
    session.press_keys(b"h\r");
}

#[test]
fn focused_deployments_select_an_older_deployment_and_show_its_details() {
    let mut session = session_showing_production_deployments();

    session.press_keys(b"j");
    session
        .wait_for_screen_contains(OLDER_DEPLOYMENT_NOTE, 5000)
        .expect("j moves to the older deployment and the pane shows its notes");
    let screen = session.emulator.get_text();
    assert!(
        screen.contains(DEPLOYMENT_SHA),
        "the deployments of production stay listed:\n{screen}"
    );
}

#[test]
fn rollback_reruns_the_job_of_the_selected_deployment() {
    let mut session = session_showing_production_deployments();

    session.press_keys(b"j");
    session
        .wait_for_screen_contains(OLDER_DEPLOYMENT_NOTE, 5000)
        .expect("j selects the older deployment");
    session.press_keys(b"R");
    confirm_dialog(&mut session, "Roll Back 'production' to 0ldc0de1");
    session
        .wait_for_cli_call("glab ci retry 500 -R test-owner/test-repo", 5000)
        .expect("confirming re-runs the selected deployment's job");
}

#[test]
fn stop_stops_the_highlighted_environment_after_confirmation() {
    let mut session =
        TestSession::with_config(false, 40, 160, Some("active_tab = \"environments\"\n"));
    session
        .wait_for_screen_contains("staging", 30000)
        .expect("the Environments tab lists the fixture environments");

    session.press_keys(b"d");
    confirm_dialog(&mut session, "Stop Environment 'production'");
    session
        .wait_for_cli_call(
            "glab api -X POST /projects/test-owner%2Ftest-repo/environments/1/stop",
            5000,
        )
        .expect("confirming stops the highlighted environment");
}

#[test]
fn github_stop_deactivates_the_live_deployment_not_a_newer_failed_one() {
    let mut session =
        TestSession::with_config(true, 40, 160, Some("active_tab = \"environments\"\n"));
    session
        .wait_for_screen_contains("staging", 30000)
        .expect("the Environments tab lists the fixture environments");

    session.press_keys(b"d");
    confirm_dialog(&mut session, "Stop Environment 'production'");
    session
        .wait_for_cli_call(
            "gh api -X POST --input - /repos/test-owner/test-repo/deployments/99/statuses",
            5000,
        )
        .expect("confirming deactivates the deployment whose latest status is success");
    let calls = session.get_cli_calls();
    assert!(
        !calls.contains("/repos/test-owner/test-repo/deployments/100/statuses\n"),
        "the newer failed deployment was marked inactive:\n{calls}"
    );
}

const DEPLOYMENT_LOG_ACTION: &str = "opens the selected deployment's job log";
const DEPLOYMENT_LOG_HELP: &str = "Open the selected deployment's job log";

/// Opens the detail pane on `staging`'s environment document, the one that
/// lists the action keys: Enter shows production's deployments, Esc and j
/// move on to staging, whose deployments are not listed.
fn session_showing_staging_actions(is_github: bool) -> TestSession {
    let mut session =
        TestSession::with_config(is_github, 40, 160, Some("active_tab = \"environments\"\n"));
    session
        .wait_for_screen_contains("staging", 30000)
        .expect("the Environments tab lists the fixture environments");
    session.send_input(b"\r");
    session
        .wait_for_screen_contains(DEPLOYMENT_SHA, 15000)
        .expect("Enter shows the deployments of the highlighted environment");
    session.press_keys(b"\x1bj");
    session
        .wait_for_screen_contains("stops the environment", 5000)
        .expect("the detail pane lists staging's action keys");
    session
}

fn open_environments_help(session: &mut TestSession) {
    session.press_keys(b"?");
    session
        .wait_for_screen_contains("List and focus the environment's deployments", 3000)
        .expect("the Environments help is shown");
}

/// GitHub deployments carry no job, so neither the environment's action list
/// nor the help offers the key that opens one's log.
#[test]
fn github_environments_do_not_offer_the_deployment_log() {
    let mut session = session_showing_staging_actions(true);
    let screen = session.emulator.get_text();
    assert!(
        !screen.contains(DEPLOYMENT_LOG_ACTION),
        "the deployment log is offered on GitHub:\n{screen}"
    );

    open_environments_help(&mut session);
    let screen = session.emulator.get_text();
    assert!(
        !screen.contains(DEPLOYMENT_LOG_HELP),
        "the help offers the deployment log on GitHub:\n{screen}"
    );
}

#[test]
fn gitlab_environments_offer_the_deployment_log() {
    let mut session = session_showing_staging_actions(false);
    assert!(
        session.emulator.get_text().contains(DEPLOYMENT_LOG_ACTION),
        "the detail pane offers the deployment log on GitLab"
    );

    open_environments_help(&mut session);
    session
        .wait_for_screen_contains(DEPLOYMENT_LOG_HELP, 3000)
        .expect("the help offers the deployment log on GitLab");
}
