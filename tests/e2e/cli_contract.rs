//! Checks the `glab` / `gh` calls the CLI mocks record against
//! `docs/cli-reference.md`, which `scripts/gen-cli-reference.sh` writes from
//! the pinned CLIs' `--help`. A mock answers whatever it is asked, so without
//! this a subcommand or flag the real CLI does not have passes every test.
//!
//! glab's help does not say which flags take a value. So after a flag, the
//! next word counts as its value unless it names a subcommand, a value that
//! starts with `-` and a letter is read as a flag, and of combined short flags
//! (`-yR`) only the first is checked, since `-Rowner/repo` looks the same.
//! glab's help lists no aliases either, so `glab mr ls` is refused: call the
//! canonical name.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

/// The mocks end each argument of a recorded call with this byte.
const ARG_END: char = '\u{1f}';

/// Placeholders a usage line writes for the subcommand itself, not for an
/// argument of the command.
const SUBCOMMAND_PLACEHOLDERS: [&str; 4] =
    ["<command>", "[command]", "<subcommand>", "[subcommand]"];

#[derive(Clone, Default)]
struct Command {
    usage: Option<&'static str>,
    has_subcommands: bool,
    /// The generator could not read its help (it needs a git remote, say),
    /// so nothing is known about its flags.
    help_unavailable: bool,
    flags: HashSet<&'static str>,
}

impl Command {
    /// cobra refuses a positional argument here: the command has subcommands
    /// and its usage names no argument of its own (`glab issue [command]`, but
    /// not `glab mr note [command] [<id> | <branch>]`).
    fn requires_subcommand(&self) -> bool {
        let own_argument = self.usage.is_some_and(|usage| {
            SUBCOMMAND_PLACEHOLDERS
                .iter()
                .fold(usage.to_string(), |usage, placeholder| {
                    usage.replace(placeholder, "")
                })
                .contains('<')
        });
        self.has_subcommands && !own_argument
    }
}

/// Every command of `docs/cli-reference.md`, by its full path (`glab ci retry`).
static REFERENCE: LazyLock<HashMap<String, Command>> =
    LazyLock::new(|| parse_reference(include_str!("../../docs/cli-reference.md")));

/// Reads the sections the generator writes: a heading naming the command,
/// then optional `**Usage:**`, `**Aliases:**`, `**Help unavailable:**` and a
/// flag table whose first cell lists the flag's spellings (`-R --repo`,
/// `-j, --job string`).
fn parse_reference(text: &'static str) -> HashMap<String, Command> {
    let mut commands: HashMap<String, Command> = HashMap::new();
    let mut aliases: Vec<(&'static str, &'static str)> = Vec::new();
    let mut current: Option<&'static str> = None;
    for line in text.lines() {
        if line.starts_with('#') {
            current = line
                .trim_start_matches('#')
                .trim()
                .strip_prefix('`')
                .and_then(|rest| rest.strip_suffix('`'))
                .filter(|path| matches!(path.split(' ').next(), Some("glab" | "gh")));
            if let Some(path) = current {
                commands.entry(path.to_string()).or_default();
            }
            continue;
        }
        let Some(path) = current else { continue };
        let command = commands.entry(path.to_string()).or_default();
        if let Some(usage) = line.strip_prefix("**Usage:** ") {
            command.usage = Some(usage.trim_matches('`'));
        } else if line.starts_with("**Help unavailable:**") {
            command.help_unavailable = true;
        } else if let Some(list) = line.strip_prefix("**Aliases:** ") {
            aliases.extend(
                list.split(", ")
                    .map(|alias| (alias.trim_matches('`'), path)),
            );
        } else if let Some(row) = line.strip_prefix("| `") {
            let spec = row.split('`').next().unwrap_or_default();
            let flags = spec
                .split([' ', ','])
                .filter(|word| !word.is_empty())
                .take_while(|word| word.starts_with('-'));
            command.flags.extend(flags);
        }
    }

    let parents: Vec<String> = commands
        .keys()
        .filter_map(|path| path.rsplit_once(' ').map(|(parent, _)| parent.to_string()))
        .collect();
    for parent in parents {
        if let Some(command) = commands.get_mut(&parent) {
            command.has_subcommands = true;
        }
    }

    // gh's help lists only `--help`, but cobra also takes `-h` where no other
    // flag claims it.
    for (path, command) in &mut commands {
        if path.starts_with("gh") && command.flags.contains("--help") {
            command.flags.insert("-h");
        }
    }

    // An alias of a parent also names every command below it: `gh cs list`.
    let mut spelled_by_alias = Vec::new();
    for (alias, target) in aliases {
        for (path, command) in &commands {
            if let Some(below) = path.strip_prefix(target)
                && (below.is_empty() || below.starts_with(' '))
            {
                spelled_by_alias.push((format!("{alias}{below}"), command.clone()));
            }
        }
    }
    for (path, command) in spelled_by_alias {
        commands.entry(path).or_insert(command);
    }
    commands
}

/// The flag `arg` names (`--repo` for `--repo=x`, `-R` for `-Rx`), or `None`
/// when it is a value: `-`, `-1`, `- item`.
fn flag_name(arg: &str) -> Option<&str> {
    if let Some(long) = arg.strip_prefix("--") {
        long.starts_with(|c: char| c.is_ascii_alphabetic())
            .then(|| arg.split('=').next().unwrap_or(arg))
    } else {
        let short = arg.strip_prefix('-')?;
        short
            .starts_with(|c: char| c.is_ascii_alphabetic())
            .then(|| &arg[..2])
    }
}

/// Why the real CLI would refuse `argv` (program name first), or `None`.
pub fn violation(argv: &[&str]) -> Option<String> {
    if !matches!(argv.first(), Some(&("glab" | "gh"))) {
        return None;
    }
    let refused = |reason: String| {
        Some(format!(
            "{}: {reason} (docs/cli-reference.md)",
            argv.join(" ")
        ))
    };

    let mut path = argv[0].to_string();
    let mut flags = Vec::new();
    let mut first_positional = None;
    let mut may_be_flag_value = false;
    for &arg in &argv[1..] {
        if arg == "--" {
            break;
        }
        if let Some(flag) = flag_name(arg) {
            flags.push(flag);
            may_be_flag_value = !arg.contains('=');
            continue;
        }
        if first_positional.is_none() {
            let child = format!("{path} {arg}");
            if REFERENCE.contains_key(&child) {
                path = child;
                may_be_flag_value = false;
                continue;
            }
        }
        if std::mem::take(&mut may_be_flag_value) {
            continue;
        }
        first_positional.get_or_insert(arg);
    }

    let command = &REFERENCE[&path];
    if command.help_unavailable {
        return None;
    }
    if command.requires_subcommand() {
        if let Some(arg) = first_positional {
            return refused(format!("`{path}` has no subcommand `{arg}`"));
        }
        let asks_for_help = flags
            .iter()
            .any(|flag| matches!(*flag, "-h" | "--help" | "--version"));
        if !asks_for_help {
            return refused(format!("`{path}` needs a subcommand"));
        }
    }
    flags
        .iter()
        .find(|flag| !command.flags.contains(*flag))
        .and_then(|flag| refused(format!("`{path}` has no flag `{flag}`")))
}

fn argv_dir(log_path: &Path) -> PathBuf {
    let mut dir = log_path.as_os_str().to_owned();
    dir.push(".argv.d");
    PathBuf::from(dir)
}

/// The names of the records in `<log_path>.argv.d/`. A mock writes its record
/// under a dot name and renames it once complete.
fn record_names(log_path: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(argv_dir(log_path)) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect()
}

/// Waits until no mock is still writing its record and none has started one
/// for a moment, at most `timeout`: a mock glab-tui started before it was
/// killed keeps running.
pub fn wait_for_recorded_calls(log_path: &Path, timeout: Duration) {
    const QUIET: Duration = Duration::from_millis(50);
    let deadline = Instant::now() + timeout;
    let mut seen = record_names(log_path).len();
    while Instant::now() < deadline {
        std::thread::sleep(QUIET);
        let names = record_names(log_path);
        let pending = names.iter().any(|name| name.starts_with('.'));
        if !pending && names.len() == seen {
            return;
        }
        seen = names.len();
    }
}

/// Every call recorded in `<log_path>.argv.d/` the real CLI would refuse.
pub fn violations_in_log(log_path: &Path) -> Vec<String> {
    let dir = argv_dir(log_path);
    let mut refused: Vec<String> = record_names(log_path)
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .filter_map(|name| std::fs::read_to_string(dir.join(name)).ok())
        .filter_map(|record| {
            let argv: Vec<&str> = record
                .strip_suffix(ARG_END)
                .unwrap_or(&record)
                .split(ARG_END)
                .collect();
            violation(&argv)
        })
        .collect();
    refused.sort();
    refused.dedup();
    refused
}

#[test]
fn a_subcommand_glab_does_not_have_is_refused() {
    for (argv, reason) in [
        (
            &["glab", "job", "view", "54321", "-R", "x", "-w"][..],
            "`glab job` has no subcommand `view`",
        ),
        (
            &["glab", "issue", "frobnicate", "1"],
            "`glab issue` has no subcommand `frobnicate`",
        ),
        (
            &["glab", "repo", "remote", "frob"],
            "`glab repo remote` has no subcommand `frob`",
        ),
        (
            &["glab", "mr", "-R", "o/r", "bogus"],
            "`glab mr` has no subcommand `bogus`",
        ),
        (&["gh", "cs", "bogus"], "`gh cs` has no subcommand `bogus`"),
    ] {
        let refused = violation(argv);
        assert!(
            refused.as_deref().is_some_and(|text| text.contains(reason)),
            "{argv:?}: {refused:?}"
        );
    }
}

#[test]
fn a_flag_the_command_does_not_have_is_refused() {
    for (argv, flag) in [
        (
            &["gh", "run", "rerun", "1", "--everything"][..],
            "--everything",
        ),
        (&["glab", "mr", "list", "--fill"], "--fill"),
        (&["glab", "alias", "set", "x", "y", "--title"], "--title"),
        (&["glab", "repo", "clone", "a/b", "--depth", "1"], "--depth"),
    ] {
        let refused = violation(argv);
        assert!(
            refused
                .as_deref()
                .is_some_and(|text| text.contains(&format!("no flag `{flag}`"))),
            "{argv:?}: {refused:?}"
        );
    }
}

#[test]
fn calls_the_cli_accepts_pass() {
    for argv in [
        &["glab", "ci", "retry", "500", "-R", "test-owner/test-repo"][..],
        &["glab", "mr", "list", "--output=json", "-R", "o/r"],
        &["glab", "mr", "-R", "o/r", "list", "--output", "json"],
        &["glab", "mr", "note", "1", "-R", "o/r"],
        &["glab", "mr", "note", "create", "1", "-m", "- item one"],
        &["glab", "alias", "set", "x", "y"],
        &["glab", "runner", "update", "1", "--pause", "-R", "o/r"],
        &["glab", "repo", "clone", "a/b", "--", "--depth", "1"],
        &["glab", "dependency-firewall", "npm", "--anything"],
        &["gh", "run", "rerun", "1", "--failed", "-R", "o/r"],
        &["gh", "api", "graphql", "-f", "query=x", "--input", "-"],
        &["gh", "pr", "list", "-h"],
        &["gh", "pr", "ls", "--state=all"],
        &["gh", "cs", "list"],
        &["gh", "--version"],
    ] {
        assert_eq!(violation(argv), None, "{argv:?}");
    }
}

#[test]
fn a_recorded_call_glab_does_not_have_is_reported_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("cli_calls.log");
    std::fs::create_dir(argv_dir(&log_path)).unwrap();
    let record =
        |name: &str, argv: &str| std::fs::write(argv_dir(&log_path).join(name), argv).unwrap();
    record("a", "glab\u{1f}job\u{1f}view\u{1f}1\u{1f}-w\u{1f}");
    record("b", "glab\u{1f}ci\u{1f}retry\u{1f}1\u{1f}");
    // Still being written: not read.
    record(".c", "glab\u{1f}bogus\u{1f}");

    let refused = violations_in_log(&log_path);

    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0].starts_with("glab job view 1 -w: "),
        "{refused:?}"
    );
}

#[test]
#[should_panic(expected = "glab job view 1 -w")]
fn a_test_session_whose_call_log_has_a_refused_call_fails() {
    let sandbox = crate::Sandbox::new(false).unwrap();
    std::fs::create_dir(argv_dir(&sandbox.log_path)).unwrap();
    std::fs::write(
        argv_dir(&sandbox.log_path).join("a"),
        "glab\u{1f}job\u{1f}view\u{1f}1\u{1f}-w\u{1f}",
    )
    .unwrap();
    drop(sandbox);
}
