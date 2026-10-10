//! End-to-end coverage for the Environments detail pane: deployments fetched
//! for one environment never stay on screen once another environment is
//! highlighted.
//!
//! The mock `glab` serves `tests/fixtures/environments.json` (`production`,
//! then `staging`) for the environments list and
//! `tests/fixtures/deployments.json` for every deployments request. Its only
//! deployment has SHA `f00dcafe`, which nothing else on the Environments tab
//! shows.

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

    session.send_input(b"j");
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
