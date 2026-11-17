//! End-to-end coverage for following a running job's log: a poll asks GitLab
//! only for what follows the part already shown.
//!
//! With `TEST_GLAB_TRACE_FILE` set, the mock `glab` serves that file as the
//! job log, honouring `byte_offset`/`byte_limit`, and appends `follow line N`
//! after every read, like a job still writing output.
//! `TEST_GLAB_TRACE_IGNORES_RANGES` makes it ignore the range parameters and
//! send the whole log, like a GitLab that predates them.

use crate::{Sandbox, TestSession};

/// Bytes a resumed read requests again ahead of the new content.
const OVERLAP_BYTES: usize = 1024;
/// GitLab's largest accepted `byte_limit`.
const RANGE_MAX_BYTES: usize = 500 * 1024;
const TRACE_ENDPOINT: &str = "glab api /projects/test-owner%2Ftest-repo/jobs/54321/trace";
/// Lines of a log that, with the `follow line N` lines the job writes, fits
/// the 25 rows the zoomed log pane keeps under the job's metadata in a
/// 40-row terminal.
const PANE_FITTING_LINES: usize = 20;
/// Bytes of each of those lines, so the re-requested overlap spans 16 whole
/// lines: appending it again would push lines off screen and show others twice.
const PANE_FITTING_LINE_BYTES: usize = 64;

fn trace_calls(session: &TestSession) -> Vec<String> {
    session
        .get_cli_calls()
        .lines()
        .filter(|line| line.contains("/trace"))
        .map(str::to_string)
        .collect()
}

/// Launches glab-tui with `log` as the job's log and lists the job, its log
/// not fetched yet.
fn list_job(log: &str, extra: &[(&str, &str)]) -> TestSession {
    let sandbox = Sandbox::new(false).unwrap();
    let trace_file = sandbox.temp_dir.path().join("job.log");
    std::fs::write(&trace_file, log).unwrap();
    let trace_path = trace_file.to_str().unwrap().to_string();
    let mut envs = vec![("TEST_GLAB_TRACE_FILE", trace_path.as_str())];
    envs.extend_from_slice(extra);
    let mut session = TestSession::launch(sandbox, 40, 140, &envs);

    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app starts");
    session.send_input(b"ll");
    session
        .wait_for_screen_contains("12345", 15000)
        .expect("the pipeline is listed");
    session.send_input(b"\x1bj"); // Alt+j: the pipeline's own jobs
    session
        .wait_for_screen_contains("rspec", 15000)
        .expect("the pipeline's job is listed");
    session
}

/// Launches glab-tui with `log` as the job's log and opens it.
fn show_job_log(log: &str, extra: &[(&str, &str)]) -> TestSession {
    let mut session = list_job(log, extra);
    session.send_input(b"\r");
    session
        .wait_for_screen_contains("build output line 000", 15000)
        .expect("the job log is shown");
    session
}

/// A log whose one failure, line 80, is more than a log pane away from both
/// its start and its end.
fn failing_log() -> String {
    let mut log: String = (0..80)
        .map(|line| format!("build output line {line:03}\n"))
        .collect();
    log.push_str("ERROR: Job failed: exit code 1\n");
    log.push_str(&"cleanup line\n".repeat(60));
    log
}

fn ranged_read_after(log: &str) -> String {
    format!(
        "{TRACE_ENDPOINT}?byte_offset={}&byte_limit={RANGE_MAX_BYTES}",
        log.len() - OVERLAP_BYTES
    )
}

#[test]
fn follow_mode_appends_what_the_log_gained_from_a_ranged_read() {
    let log: String = (0..PANE_FITTING_LINES)
        .map(|line| {
            let label = format!("build output line {line:03} ");
            format!("{label:.<width$}\n", width = PANE_FITTING_LINE_BYTES - 1)
        })
        .collect();
    let mut session = show_job_log(&log, &[]);

    session.send_input(b"f");
    session
        .wait_for_screen_contains("follow line 1", 15000)
        .expect("the follow poll appends the line the job wrote");

    let calls = trace_calls(&session);
    assert_eq!(
        calls[..2],
        [TRACE_ENDPOINT.to_string(), ranged_read_after(&log)],
        "{calls:?}"
    );
    let screen = session.emulator.get_text();
    for line in 0..PANE_FITTING_LINES {
        let label = format!("build output line {line:03}");
        assert_eq!(screen.matches(&label).count(), 1, "{label} once:\n{screen}");
    }
}

#[test]
fn follow_mode_reads_whole_logs_once_the_server_ignores_ranges() {
    let log: String = (0..100)
        .map(|line| format!("build output line {line:03}\n"))
        .collect();
    let mut session = show_job_log(&log, &[("TEST_GLAB_TRACE_IGNORES_RANGES", "1")]);

    session.send_input(b"f");
    session
        .wait_for_screen_contains("follow line 2", 20000)
        .expect("two follow polls append the lines the job wrote");

    let calls = trace_calls(&session);
    assert_eq!(
        calls[..3],
        [
            TRACE_ENDPOINT.to_string(),
            ranged_read_after(&log),
            TRACE_ENDPOINT.to_string(),
        ],
        "a whole log answering the ranged read makes later polls read whole logs: {calls:?}"
    );
    let screen = session.emulator.get_text();
    assert_eq!(screen.matches("follow line 1").count(), 1, "{screen}");
    assert_eq!(screen.matches("follow line 2").count(), 1, "{screen}");
}

#[test]
fn failure_key_scrolls_the_log_to_its_failure_under_the_job_metadata() {
    let mut session = show_job_log(&failing_log(), &[]);

    let screen = session.emulator.get_text();
    for metadata in [
        "Preview: Job #54321",
        "shared-runner-7",
        "1m 35s",
        "archive, junit",
        "https://gitlab.com/test-owner/test-repo/-/jobs/54321",
    ] {
        assert!(screen.contains(metadata), "{metadata} in\n{screen}");
    }

    session.send_input(b"E");
    session
        .wait_for_screen_contains("ERROR: Job failed: exit code 1", 15000)
        .expect("the failure line is scrolled into view");
    let screen = session.emulator.get_text();
    assert!(
        !screen.contains("build output line 079"),
        "the failure line tops the log pane:\n{screen}"
    );
}

#[test]
fn failure_key_fetches_an_unshown_log_and_scrolls_it_to_its_failure() {
    let mut session = list_job(&failing_log(), &[]);

    session.send_input(b"E");
    session
        .wait_for_screen_contains("ERROR: Job failed: exit code 1", 15000)
        .expect("the fetched log is scrolled to its failure");
    let screen = session.emulator.get_text();
    assert!(
        !screen.contains("build output line 079"),
        "the failure line tops the log pane:\n{screen}"
    );
    assert_eq!(trace_calls(&session), [TRACE_ENDPOINT], "one fetch");
}
