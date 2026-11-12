use crate::Sandbox;

/// Runs the sandbox's `tool` mock the way glab-tui would, with `args`.
fn call_mock(sandbox: &Sandbox, tool: &str, args: &[&str]) {
    std::process::Command::new(sandbox.bin_dir.join(tool))
        .args(args)
        .envs(sandbox.envs())
        .output()
        .unwrap();
}

#[test]
#[should_panic(expected = "glab no-such-command --flag")]
fn unknown_glab_call_fails_the_test() {
    let sandbox = Sandbox::new(false).unwrap();
    call_mock(&sandbox, "glab", &["no-such-command", "--flag"]);
}

#[test]
#[should_panic(expected = "gh api no/such/endpoint")]
fn unknown_gh_api_call_fails_the_test() {
    let sandbox = Sandbox::new(true).unwrap();
    call_mock(&sandbox, "gh", &["api", "no/such/endpoint"]);
}

#[test]
fn known_and_injected_failures_pass() {
    let sandbox = Sandbox::new(false).unwrap();
    call_mock(&sandbox, "glab", &["todo", "list", "--output=json"]);
    std::process::Command::new(sandbox.bin_dir.join("glab"))
        .args(["check-update"])
        .envs(sandbox.envs())
        .env("TEST_GLAB_FAIL_MATCH", "check-update")
        .output()
        .unwrap();
}
