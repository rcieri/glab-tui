//! End-to-end coverage for how configuration turns into paged CLI requests.
//!
//! These assert on the arguments the app actually hands to `glab`, captured by
//! the mock CLI in `tests/mocks/glab`. That is the whole chain — config file →
//! `Config` → backend → command line — which unit tests cannot cover.
//!
//! Pages are requested concurrently, so the log order is nondeterministic.
//! Every assertion here is on the *set* of calls, never their sequence.

use crate::TestSession;
use std::time::{Duration, Instant};

/// Launch with `config_toml` and wait until the first screen has rendered.
fn session_with(config_toml: Option<&str>) -> TestSession {
    let mut session = TestSession::with_config(false, 24, 80, config_toml);
    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app should reach the Issues tab");
    session
}

/// Every `glab issue list` invocation recorded so far.
///
/// `issue list` is the paged fetch the app makes for its default tab. Other
/// calls (`label list`, `api .../members/all`) are filtered out: they are
/// unpaged and would otherwise make these assertions timing-dependent.
fn issue_list_calls(session: &TestSession) -> Vec<String> {
    session
        .get_cli_calls()
        .lines()
        .filter(|line| line.contains("issue list"))
        .map(|line| line.to_string())
        .collect()
}

/// Wait until `expected` issue-list calls have been logged, then confirm no
/// further ones arrive. Polling rather than sleeping a fixed interval keeps the
/// test fast when the app is quick; the settle window is what makes
/// "exactly N" a real assertion rather than "at least N".
fn wait_for_issue_list_calls(session: &TestSession, expected: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_millis(15000);
    while Instant::now() < deadline && issue_list_calls(session).len() < expected {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(750));
    let calls = issue_list_calls(session);
    assert_eq!(
        calls.len(),
        expected,
        "expected exactly {} issue-list request(s), got {}:\n{}",
        expected,
        calls.len(),
        calls.join("\n")
    );
    calls
}

/// The `--page N` values across the given calls, sorted.
fn pages_requested(calls: &[String]) -> Vec<u32> {
    let mut pages: Vec<u32> = calls
        .iter()
        .map(|call| {
            let tokens: Vec<&str> = call.split_whitespace().collect();
            let idx = tokens
                .iter()
                .position(|t| *t == "--page")
                .unwrap_or_else(|| panic!("no --page in: {}", call));
            tokens[idx + 1]
                .parse()
                .unwrap_or_else(|_| panic!("unparseable --page in: {}", call))
        })
        .collect();
    pages.sort_unstable();
    pages
}

/// The `--per-page N` value of a single call.
///
/// Parsed as a whole token rather than matched as a substring: `--per-page 100`
/// *contains* the text `--per-page 1`, so substring matching would silently
/// accept a wrong page size.
fn per_page_of(call: &str) -> u32 {
    let tokens: Vec<&str> = call.split_whitespace().collect();
    let idx = tokens
        .iter()
        .position(|t| *t == "--per-page")
        .unwrap_or_else(|| panic!("no --per-page in: {}", call));
    tokens[idx + 1]
        .parse()
        .unwrap_or_else(|_| panic!("unparseable --per-page in: {}", call))
}

fn assert_all_use_per_page(calls: &[String], per_page: u32) {
    for call in calls {
        assert_eq!(per_page_of(call), per_page, "wrong --per-page in: {}", call);
    }
}

// --- Tier 1: Feature Coverage (5 cases) ---

/// With no configuration, one request covers the default 100-item budget.
#[test]
fn test_pagination_normal_limit() {
    let session = session_with(None);
    let calls = wait_for_issue_list_calls(&session, 1);
    assert_eq!(pages_requested(&calls), vec![1]);
    assert_all_use_per_page(&calls, 100);
}

/// A budget larger than one request spans several pages, each still asking for
/// the default 100 items.
#[test]
fn test_pagination_large_limit() {
    let session = session_with(Some("page_size = 250\n"));
    let calls = wait_for_issue_list_calls(&session, 3);
    assert_eq!(pages_requested(&calls), vec![1, 2, 3]);
    assert_all_use_per_page(&calls, 100);
}

/// Lowering `api_per_page` splits the same budget into more, smaller requests —
/// the point of the setting, for servers that truncate large responses.
#[test]
fn test_pagination_small_limit() {
    let session = session_with(Some("page_size = 100\napi_per_page = 20\n"));
    let calls = wait_for_issue_list_calls(&session, 5);
    assert_eq!(pages_requested(&calls), vec![1, 2, 3, 4, 5]);
    assert_all_use_per_page(&calls, 20);
}

/// A value of the wrong type makes config loading fall back to defaults rather
/// than failing to start.
#[test]
fn test_pagination_fallback_invalid() {
    let session = session_with(Some("page_size = 100\napi_per_page = \"twenty\"\n"));
    let calls = wait_for_issue_list_calls(&session, 1);
    assert_all_use_per_page(&calls, 100);
}

/// `api_per_page` is passed through to all list endpoints, including labels.
/// Lowering it from the default 100 shrinks every page, including the label
/// fetch that populates the edit-menu selector.
#[test]
fn test_pagination_custom_per_endpoint() {
    let session = session_with(Some("page_size = 100\napi_per_page = 20\n"));
    let calls = wait_for_issue_list_calls(&session, 5);
    assert_all_use_per_page(&calls, 20);

    let label_calls: Vec<String> = session
        .get_cli_calls()
        .lines()
        .filter(|line| line.contains("label list"))
        .map(|line| line.to_string())
        .collect();
    assert!(
        !label_calls.is_empty(),
        "expected a label-list call to compare against"
    );
    // fetch_labels now honours api_per_page (was previously hard-coded to 100).
    for call in &label_calls {
        assert!(
            call.contains("--per-page 20"),
            "label fetch should use api_per_page=20, got: {}",
            call
        );
    }
}

// --- Tier 2: Boundary & Corner Cases (5 cases) ---

/// Zero would make the page-count arithmetic divide by zero; it is clamped to
/// one item per request instead of panicking.
#[test]
fn test_pagination_zero() {
    let session = session_with(Some("page_size = 3\napi_per_page = 0\n"));
    let calls = wait_for_issue_list_calls(&session, 3);
    assert_eq!(pages_requested(&calls), vec![1, 2, 3]);
    assert_all_use_per_page(&calls, 1);
}

/// A negative value cannot deserialize into the unsigned field, so the app
/// starts on defaults rather than refusing to run.
#[test]
fn test_pagination_negative() {
    let session = session_with(Some("page_size = 100\napi_per_page = -5\n"));
    let calls = wait_for_issue_list_calls(&session, 1);
    assert_all_use_per_page(&calls, 100);
}

/// GitLab rejects `per_page` above 100, so larger values are clamped rather
/// than sent through and refused by the server.
#[test]
fn test_pagination_max_bounds() {
    let session = session_with(Some("page_size = 100\napi_per_page = 250\n"));
    let calls = wait_for_issue_list_calls(&session, 1);
    assert_eq!(pages_requested(&calls), vec![1]);
    assert_all_use_per_page(&calls, 100);
}

/// The mock returns an empty list for every page. Because the pages are issued
/// concurrently there is no last-page signal to stop on, so the full budget is
/// still requested — and an empty result must not crash the app.
#[test]
fn test_pagination_empty_response() {
    let mut session = session_with(Some("page_size = 60\napi_per_page = 20\n"));
    let calls = wait_for_issue_list_calls(&session, 3);
    assert_eq!(pages_requested(&calls), vec![1, 2, 3]);
    session
        .wait_for_screen_contains("Issues", 5000)
        .expect("app should still be running after empty pages");
}

/// A single-item budget is one request for one item, not a rounded-up page.
#[test]
fn test_pagination_single_item() {
    let session = session_with(Some("page_size = 1\napi_per_page = 1\n"));
    let calls = wait_for_issue_list_calls(&session, 1);
    assert_eq!(pages_requested(&calls), vec![1]);
    assert_all_use_per_page(&calls, 1);
}
