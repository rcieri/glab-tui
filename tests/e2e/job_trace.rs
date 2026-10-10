//! End-to-end coverage for following a running job's log: a poll asks GitLab
//! only for what follows the part already shown.
//!
//! With `TEST_GLAB_TRACE_FILE` set, the mock `glab` serves that file as the
//! job log, honouring `byte_offset`/`byte_limit`, and appends `follow line N`
//! after every read, like a job still writing output.

use crate::{Sandbox, TestSession};

/// Bytes a resumed read requests again ahead of the new content.
const OVERLAP_BYTES: usize = 1024;
/// GitLab's largest accepted `byte_limit`.
const RANGE_MAX_BYTES: usize = 500 * 1024;

fn trace_calls(session: &TestSession) -> Vec<String> {
    session
        .get_cli_calls()
        .lines()
        .filter(|line| line.contains("/trace"))
        .map(str::to_string)
        .collect()
}

#[test]
fn follow_mode_appends_what_the_log_gained_from_a_ranged_read() {
    let sandbox = Sandbox::new(false).unwrap();
    let trace_file = sandbox.temp_dir.path().join("job.log");
    let log: String = (0..100)
        .map(|line| format!("build output line {line:03}\n"))
        .collect();
    std::fs::write(&trace_file, &log).unwrap();
    let mut session = TestSession::launch(
        sandbox,
        40,
        140,
        &[("TEST_GLAB_TRACE_FILE", trace_file.to_str().unwrap())],
    );

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
    session.send_input(b"\r");
    session
        .wait_for_screen_contains("build output line 000", 15000)
        .expect("the job log is shown");

    session.send_input(b"f");
    session
        .wait_for_screen_contains("follow line 1", 15000)
        .expect("the follow poll appends the line the job wrote");

    let endpoint = "glab api /projects/test-owner%2Ftest-repo/jobs/54321/trace";
    let calls = trace_calls(&session);
    assert_eq!(
        calls[..2],
        [
            endpoint.to_string(),
            format!(
                "{endpoint}?byte_offset={}&byte_limit={RANGE_MAX_BYTES}",
                log.len() - OVERLAP_BYTES
            ),
        ],
        "{calls:?}"
    );
    assert!(
        session
            .emulator
            .get_text()
            .contains("build output line 099"),
        "the lines read before stay on screen:\n{}",
        session.emulator.get_text()
    );
}
