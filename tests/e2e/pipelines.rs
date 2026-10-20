use crate::TestSession;

/// Opens the GitLab Pipelines tab (Issues -> Merge Requests -> Pipelines)
/// and waits for the fixture pipeline.
fn session_on_pipelines(extra: &[(&str, &str)]) -> TestSession {
    let mut session = TestSession::with_envs(false, 40, 200, extra);
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");
    session.press_keys(b"ll");
    session
        .wait_for_screen_contains(PIPELINE_ROW, 5000)
        .expect("the pipeline is listed");
    session
}

/// The fixture pipeline's ID cell. The bare id also shows in the Terminal
/// pane's logged API calls.
const PIPELINE_ROW: &str = "#12345";

/// GitLab lists the commit only with each job, so the inspector must take
/// it from the jobs it fetched, next to the failed job and its reason.
#[test]
fn test_pipeline_inspector_shows_the_commit_and_why_jobs_failed() {
    let mut session = session_on_pipelines(&[("TEST_GLAB_FAILED_JOBS", "1")]);

    session.press_keys(b"i");
    session
        .wait_for_screen_contains("script failure", 5000)
        .expect("the failed job's reason is in the inspector");
    let text = session.emulator.get_text();
    assert!(text.contains("Failed Jobs"), "failure summary:\n{text}");
    assert!(text.contains("Bump parser to 2.1"), "commit title:\n{text}");
    assert!(text.contains("Grace Hopper"), "commit author:\n{text}");
}

/// GitLab's pipeline retry endpoint also reruns canceled jobs, so the
/// failed-jobs retry has to rerun each failed job on its own.
#[test]
fn test_retry_failed_on_gitlab_reruns_only_the_failed_jobs() {
    let mut session = session_on_pipelines(&[("TEST_GLAB_FAILED_JOBS", "1")]);

    session.press_keys(b"R");
    session
        .wait_for_cli_call("ci retry 54322", 5000)
        .expect("the failed job is retried");

    let calls = session.get_cli_calls();
    assert!(
        !calls.contains("ci retry 54321"),
        "the passing job is left alone:\n{calls}"
    );
    assert!(
        !calls.contains("/pipelines/12345/retry"),
        "the whole-pipeline retry is not used:\n{calls}"
    );
}

#[test]
fn retry_failed_on_gitlab_keeps_going_past_a_refused_job_and_reports_it() {
    let mut session = session_on_pipelines(&[
        ("TEST_GLAB_FAILED_JOBS", "1"),
        ("TEST_GLAB_FAIL_MATCH", "ci retry 54322"),
    ]);

    session.press_keys(b"R");
    session
        .wait_for_cli_call("ci retry 54323", 5000)
        .expect("the job after the refused one is still retried");
    session
        .wait_for_screen_contains("retried 1 of 2 failed jobs", 5000)
        .expect("the error says how many jobs were retried");
}

/// A retried job leaves the failed-jobs listing and shifts the pages after
/// it, so every page has to be read before the first retry.
#[test]
fn retry_failed_on_gitlab_reads_every_page_of_failed_jobs_before_retrying() {
    let mut session = session_on_pipelines(&[("TEST_GLAB_FAILED_JOBS_PAGES", "1")]);

    session.press_keys(b"R");
    session
        .wait_for_cli_call("ci retry 60001", 5000)
        .expect("the first failed job is retried");

    let calls = session.get_cli_calls();
    let second_page = calls
        .find("scope[]=failed&per_page=100&page=2")
        .unwrap_or_else(|| panic!("the second page is read:\n{calls}"));
    let first_retry = calls.find("ci retry").expect("a job was retried");
    assert!(
        second_page < first_retry,
        "every page is read before the first retry:\n{calls}"
    );
}

#[test]
fn test_retry_failed_on_github_reruns_the_failed_jobs_of_the_run() {
    let mut session = TestSession::new(true, 40, 200);
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app starts");
    session.press_keys(b"ll");
    session
        .wait_for_screen_contains(PIPELINE_ROW, 5000)
        .expect("the run is listed");

    session.press_keys(b"R");
    session
        .wait_for_cli_call("run rerun 12345 --failed", 5000)
        .expect("gh reruns only the failed jobs");
}

/// The fixture pipeline succeeded, so stepping through Running and Failed
/// hides it, Success shows it again and the next step clears the filter.
#[test]
fn test_status_filter_key_steps_through_running_failed_success_and_all() {
    let mut session = session_on_pipelines(&[]);

    let mut visible_after_each_step = Vec::new();
    for expected_notice in ["RUNNING", "FAILED", "SUCCESS", "ALL"] {
        session.press_keys(b"s");
        session
            .wait_for_screen_contains(&format!("Status filter: {expected_notice}"), 5000)
            .expect("the filter step is announced");
        session.settle(300);
        visible_after_each_step.push(session.emulator.get_text().contains(PIPELINE_ROW));
    }

    assert_eq!(visible_after_each_step, vec![false, false, true, true]);
}
