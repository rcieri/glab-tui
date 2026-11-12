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

/// Commands with subcommands that also run on their own. Their help reads
/// like a group's, so they are listed by hand: `gh codespace ports` lists the
/// ports, `glab orbit` runs the Orbit binary.
const RUNNABLE_PARENTS: [&str; 2] = ["gh codespace ports", "glab orbit"];

#[derive(Clone, Default)]
struct Command {
    usage: Option<&'static str>,
    has_subcommands: bool,
    runs_on_its_own: bool,
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
        self.has_subcommands && !self.runs_on_its_own && !own_argument
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
    for path in RUNNABLE_PARENTS {
        if let Some(command) = commands.get_mut(path) {
            command.runs_on_its_own = true;
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
/// when it is a value: `-`, `-1`, `- item`. Every word starting with `--` is
/// a flag, so `---repo` is checked as one and refused.
fn flag_name(arg: &str) -> Option<&str> {
    if arg.starts_with("--") {
        return arg.split('=').next();
    }
    let short = arg.strip_prefix('-')?;
    short
        .starts_with(|c: char| c.is_ascii_alphabetic())
        .then(|| &arg[..2])
}

/// A call sorted into the command it names, its flags and its first
/// positional argument.
struct Invocation<'a> {
    path: String,
    flags: Vec<&'a str>,
    first_positional: Option<&'a str>,
}

impl<'a> Invocation<'a> {
    /// Reads `argv` (program name first) up to `--`. Subcommands come before
    /// the first positional argument. glab's help does not say which flags
    /// take a value, so a word after a flag is its value unless it names a
    /// subcommand: `glab mr -R list view` is misread as `glab mr list`.
    fn read(argv: &[&'a str]) -> Self {
        let mut invocation = Invocation {
            path: argv[0].to_string(),
            flags: Vec::new(),
            first_positional: None,
        };
        let mut after_flag = false;
        for &word in argv[1..].iter().take_while(|word| **word != "--") {
            if let Some(flag) = flag_name(word) {
                invocation.flags.push(flag);
                after_flag = !word.contains('=');
                continue;
            }
            let is_flag_value = std::mem::replace(&mut after_flag, false);
            if invocation.first_positional.is_none() && invocation.descend(word) {
                continue;
            }
            if !is_flag_value {
                invocation.first_positional.get_or_insert(word);
            }
        }
        invocation
    }

    /// Moves to the subcommand `word` names, if the command has one.
    fn descend(&mut self, word: &str) -> bool {
        let child = format!("{} {word}", self.path);
        let found = REFERENCE.contains_key(&child);
        if found {
            self.path = child;
        }
        found
    }

    /// Why the real CLI would refuse this call, or `None`.
    fn refusal(&self) -> Option<String> {
        let path = &self.path;
        let command = &REFERENCE[path];
        if command.help_unavailable {
            return None;
        }
        if command.requires_subcommand() {
            if let Some(arg) = self.first_positional {
                return Some(format!("`{path}` has no subcommand `{arg}`"));
            }
            let asks_for_help = self
                .flags
                .iter()
                .any(|flag| matches!(*flag, "-h" | "--help" | "--version"));
            if !asks_for_help {
                return Some(format!("`{path}` needs a subcommand"));
            }
        }
        self.flags
            .iter()
            .find(|flag| !command.flags.contains(*flag))
            .map(|flag| format!("`{path}` has no flag `{flag}`"))
    }
}

/// Why the real CLI would refuse `argv` (program name first), or `None`.
pub fn violation(argv: &[&str]) -> Option<String> {
    if !matches!(argv.first(), Some(&("glab" | "gh"))) {
        return None;
    }
    Invocation::read(argv)
        .refusal()
        .map(|reason| format!("{}: {reason} (docs/cli-reference.md)", argv.join(" ")))
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

/// Waits until no record has appeared, been renamed or vanished for a
/// moment, at most `timeout`: a mock glab-tui started before it was killed
/// keeps running. A record left under its dot name by a killed mock does not
/// change, so it does not hold the wait up.
pub fn wait_for_recorded_calls(log_path: &Path, timeout: Duration) {
    const QUIET: Duration = Duration::from_millis(50);
    let deadline = Instant::now() + timeout;
    let mut seen = sorted_record_names(log_path);
    while Instant::now() < deadline {
        std::thread::sleep(QUIET);
        let names = sorted_record_names(log_path);
        if names == seen {
            return;
        }
        seen = names;
    }
}

fn sorted_record_names(log_path: &Path) -> Vec<String> {
    let mut names = record_names(log_path);
    names.sort();
    names
}

/// Every call recorded in `<log_path>.argv.d/` the real CLI would refuse.
pub fn violations_in_log(log_path: &Path) -> Vec<String> {
    let dir = argv_dir(log_path);
    let mut refused: Vec<String> = record_names(log_path)
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .filter_map(|name| std::fs::read(dir.join(name)).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
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
        assert_refused(argv, reason);
    }
}

#[test]
fn a_flag_the_command_does_not_have_is_refused() {
    for (argv, reason) in [
        (
            &["gh", "run", "rerun", "1", "--everything"][..],
            "`gh run rerun` has no flag `--everything`",
        ),
        (
            &["glab", "mr", "list", "--fill"],
            "`glab mr list` has no flag `--fill`",
        ),
        (
            &["glab", "alias", "set", "x", "y", "--title"],
            "`glab alias set` has no flag `--title`",
        ),
        (
            &["glab", "repo", "clone", "a/b", "--depth", "1"],
            "`glab repo clone` has no flag `--depth`",
        ),
        (
            &["glab", "mr", "list", "---repo", "o/r"],
            "`glab mr list` has no flag `---repo`",
        ),
    ] {
        assert_refused(argv, reason);
    }
}

fn assert_refused(argv: &[&str], reason: &str) {
    assert_eq!(
        violation(argv),
        Some(format!(
            "{}: {reason} (docs/cli-reference.md)",
            argv.join(" ")
        )),
    );
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
        &["gh", "codespace", "ports", "-c", "x"],
        &["glab", "orbit", "--install"],
    ] {
        assert_eq!(violation(argv), None, "{argv:?}");
    }
}

#[test]
fn the_reference_format_is_read() {
    let commands = parse_reference(
        "### `gh codespace`

**Usage:** `gh codespace [flags]`

**Aliases:** `gh cs`

| `--help` | Show help for command |

#### `gh codespace list`

**Aliases:** `gh codespace ls`, `gh cs ls`

| `-L, --limit int` | Maximum number \\| of codespaces |
| `--help` | Show help for command |

### `glab dependency-firewall`

**Help unavailable:** `not a git repository`
",
    );

    assert!(commands["gh codespace"].requires_subcommand());
    for path in [
        "gh codespace list",
        "gh codespace ls",
        "gh cs list",
        "gh cs ls",
    ] {
        let flags = &commands[path].flags;
        assert!(
            ["-L", "--limit", "--help", "-h"]
                .iter()
                .all(|flag| flags.contains(flag)),
            "{path}: {flags:?}"
        );
    }
    assert!(commands["glab dependency-firewall"].help_unavailable);
}

#[test]
fn a_record_a_killed_mock_left_half_written_does_not_hold_up_the_wait() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("cli_calls.log");
    std::fs::create_dir(argv_dir(&log_path)).unwrap();
    std::fs::write(argv_dir(&log_path).join(".abandoned"), "glab\u{1f}").unwrap();

    let started = Instant::now();
    wait_for_recorded_calls(&log_path, Duration::from_secs(2));

    assert!(started.elapsed() < Duration::from_secs(1));
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
fn a_record_that_is_not_utf8_is_still_checked() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("cli_calls.log");
    std::fs::create_dir(argv_dir(&log_path)).unwrap();
    std::fs::write(
        argv_dir(&log_path).join("a"),
        b"glab\x1fjob\x1fview\x1f\xff\x1f",
    )
    .unwrap();

    let refused = violations_in_log(&log_path);

    assert_eq!(refused.len(), 1, "{refused:?}");
}

#[test]
#[should_panic(expected = "glab job view 1 -w")]
fn a_sandbox_whose_call_log_has_a_refused_call_fails() {
    let sandbox = crate::Sandbox::new(false).unwrap();
    std::fs::create_dir(argv_dir(&sandbox.log_path)).unwrap();
    std::fs::write(
        argv_dir(&sandbox.log_path).join("a"),
        "glab\u{1f}job\u{1f}view\u{1f}1\u{1f}-w\u{1f}",
    )
    .unwrap();
    drop(sandbox);
}
