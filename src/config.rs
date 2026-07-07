use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock as Lazy;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub bg: Color,
    pub border: Color,
    pub border_focused: Color,
    pub header_fg: Color,
    pub highlight_bg: Color,
    pub inactive_bg: Color,
    pub text_normal: Color,
    pub text_muted: Color,
    pub checked_bg: Color,
    pub green: Color,
    pub green_bg: Color,
    pub red: Color,
    pub red_bg: Color,
    pub blue: Color,
    pub blue_bg: Color,
    pub yellow: Color,
    pub yellow_bg: Color,
    pub purple: Color,
    pub purple_bg: Color,
}

fn hex_to_color(s: &str) -> Option<Color> {
    let s = s.trim_start_matches('#');
    if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Color::Rgb(r, g, b))
    } else {
        None
    }
}

fn color_to_hex(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{:02x}{:02x}{:02x}", r, g, b),
        _ => "#000000".to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ThemeToml {
    bg: String,
    border: String,
    border_focused: String,
    header_fg: String,
    highlight_bg: String,
    inactive_bg: String,
    text_normal: String,
    text_muted: String,
    checked_bg: String,
    green: String,
    green_bg: String,
    red: String,
    red_bg: String,
    blue: String,
    blue_bg: String,
    yellow: String,
    yellow_bg: String,
    purple: String,
    purple_bg: String,
}

impl ThemeToml {
    fn to_theme(&self) -> Option<Theme> {
        Some(Theme {
            bg: hex_to_color(&self.bg)?,
            border: hex_to_color(&self.border)?,
            border_focused: hex_to_color(&self.border_focused)?,
            header_fg: hex_to_color(&self.header_fg)?,
            highlight_bg: hex_to_color(&self.highlight_bg)?,
            inactive_bg: hex_to_color(&self.inactive_bg)?,
            text_normal: hex_to_color(&self.text_normal)?,
            text_muted: hex_to_color(&self.text_muted)?,
            checked_bg: hex_to_color(&self.checked_bg)?,
            green: hex_to_color(&self.green)?,
            green_bg: hex_to_color(&self.green_bg)?,
            red: hex_to_color(&self.red)?,
            red_bg: hex_to_color(&self.red_bg)?,
            blue: hex_to_color(&self.blue)?,
            blue_bg: hex_to_color(&self.blue_bg)?,
            yellow: hex_to_color(&self.yellow)?,
            yellow_bg: hex_to_color(&self.yellow_bg)?,
            purple: hex_to_color(&self.purple)?,
            purple_bg: hex_to_color(&self.purple_bg)?,
        })
    }
}

const BUNDLED_THEMES: &[(&str, &str)] = &[
    ("default", include_str!("themes/default.toml")),
    ("tokyo-night", include_str!("themes/tokyo-night.toml")),
    ("gruvbox", include_str!("themes/gruvbox.toml")),
    ("nord", include_str!("themes/nord.toml")),
    (
        "catppuccin-mocha",
        include_str!("themes/catppuccin-mocha.toml"),
    ),
    ("dracula", include_str!("themes/dracula.toml")),
];

fn config_dir() -> PathBuf {
    if let Ok(path) = std::env::var("GLAB_TUI_CONFIG") {
        let mut p = PathBuf::from(path);
        p.pop();
        return p;
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let xdg_config = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let mut p = PathBuf::from(&home);
            p.push(".config");
            p
        });
    let mut path = xdg_config;
    path.push("glab-tui");
    path
}

fn themes_dir() -> PathBuf {
    let mut path = config_dir();
    path.push("themes");
    path
}

fn ensure_themes() {
    let dir = themes_dir();
    let _ = std::fs::create_dir_all(&dir);
    for (name, toml_str) in BUNDLED_THEMES {
        let theme_path = dir.join(format!("{}.toml", name));
        if !theme_path.exists() {
            let _ = std::fs::write(&theme_path, toml_str);
        }
    }
}

#[rustfmt::skip]
impl Theme {
    pub fn default() -> Self {
        Self {
            bg:               Color::Rgb(18, 18, 20),
            border:           Color::Rgb(80, 80, 88),
            border_focused:   Color::Rgb(49, 191, 103),
            header_fg:        Color::Rgb(49, 191, 103),
            highlight_bg:     Color::Rgb(43, 43, 57),
            inactive_bg:      Color::Rgb(49, 50, 68),
            text_normal:      Color::Rgb(216, 222, 233),
            text_muted:       Color::Rgb(130, 130, 138),
            checked_bg:       Color::Rgb(28, 38, 55),
            green:            Color::Rgb(49, 191, 103),
            green_bg:         Color::Rgb(20, 45, 28),
            red:              Color::Rgb(224, 73, 83),
            red_bg:           Color::Rgb(50, 20, 25),
            blue:             Color::Rgb(61, 139, 255),
            blue_bg:          Color::Rgb(15, 35, 60),
            yellow:           Color::Rgb(235, 180, 50),
            yellow_bg:        Color::Rgb(45, 35, 15),
            purple:           Color::Rgb(168, 122, 243),
            purple_bg:        Color::Rgb(38, 25, 55),
        }
    }

    pub fn preset(name: &str) -> Option<Self> {
        // Check user's themes directory first
        let theme_path = themes_dir().join(format!("{}.toml", name));
        if theme_path.exists() {
            if let Ok(contents) = std::fs::read_to_string(&theme_path) {
                if let Ok(tf) = toml::from_str::<ThemeToml>(&contents) {
                    return tf.to_theme();
                }
            }
        }
        // Fall back to bundled theme
        BUNDLED_THEMES
            .iter()
            .find(|(n, _)| *n == name)
            .and_then(|(_, toml_str)| toml::from_str::<ThemeToml>(toml_str).ok())
            .and_then(|tf| tf.to_theme())
            .or_else(|| (name == "default").then(Self::default))
    }
}

fn apply_color(field: &mut Color, override_val: &Option<String>) {
    if let Some(s) = override_val {
        if let Some(c) = hex_to_color(s) {
            *field = c;
        }
    }
}

fn apply_overrides(base: &mut Theme, overrides: &ThemeOverrides) {
    apply_color(&mut base.bg, &overrides.bg);
    apply_color(&mut base.border, &overrides.border);
    apply_color(&mut base.border_focused, &overrides.border_focused);
    apply_color(&mut base.header_fg, &overrides.header_fg);
    apply_color(&mut base.highlight_bg, &overrides.highlight_bg);
    apply_color(&mut base.inactive_bg, &overrides.inactive_bg);
    apply_color(&mut base.text_normal, &overrides.text_normal);
    apply_color(&mut base.text_muted, &overrides.text_muted);
    apply_color(&mut base.checked_bg, &overrides.checked_bg);
    apply_color(&mut base.green, &overrides.green);
    apply_color(&mut base.green_bg, &overrides.green_bg);
    apply_color(&mut base.red, &overrides.red);
    apply_color(&mut base.red_bg, &overrides.red_bg);
    apply_color(&mut base.blue, &overrides.blue);
    apply_color(&mut base.blue_bg, &overrides.blue_bg);
    apply_color(&mut base.yellow, &overrides.yellow);
    apply_color(&mut base.yellow_bg, &overrides.yellow_bg);
    apply_color(&mut base.purple, &overrides.purple);
    apply_color(&mut base.purple_bg, &overrides.purple_bg);
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeOverrides {
    bg: Option<String>,
    border: Option<String>,
    border_focused: Option<String>,
    header_fg: Option<String>,
    highlight_bg: Option<String>,
    inactive_bg: Option<String>,
    text_normal: Option<String>,
    text_muted: Option<String>,
    checked_bg: Option<String>,
    green: Option<String>,
    green_bg: Option<String>,
    red: Option<String>,
    red_bg: Option<String>,
    blue: Option<String>,
    blue_bg: Option<String>,
    yellow: Option<String>,
    yellow_bg: Option<String>,
    purple: Option<String>,
    purple_bg: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindingGlobal {
    #[serde(default)]
    pub quit: String,
    #[serde(default)]
    pub help: String,
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub refresh: String,
    #[serde(default)]
    pub configure: String,
    #[serde(default)]
    pub next_tab: String,
    #[serde(default)]
    pub prev_tab: String,
    #[serde(default)]
    pub scroll_down: String,
    #[serde(default)]
    pub scroll_up: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindingIssues {
    #[serde(default)]
    pub create_issue: String,
    #[serde(default)]
    pub edit_entity: String,
    #[serde(default)]
    pub close_entity: String,
    #[serde(default)]
    pub reopen_entity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindingMrs {
    #[serde(default)]
    pub create_mr: String,
    #[serde(default)]
    pub approve_mr: String,
    #[serde(default)]
    pub merge_mr: String,
    #[serde(default)]
    pub toggle_draft: String,
    #[serde(default)]
    pub view_diff: String,
    #[serde(default)]
    pub edit_entity: String,
    #[serde(default)]
    pub close_entity: String,
    #[serde(default)]
    pub reopen_entity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindingPipelines {
    #[serde(default)]
    pub trigger_pipeline: String,
    #[serde(default)]
    pub retry: String,
    #[serde(default)]
    pub cancel: String,
    #[serde(default)]
    pub download_artifact: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindingReleases {
    #[serde(default)]
    pub create_release: String,
    #[serde(default)]
    pub edit_release: String,
    #[serde(default)]
    pub delete_release: String,
    #[serde(default)]
    pub open_in_browser: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindingMilestones {
    #[serde(default)]
    pub create_milestone: String,
    #[serde(default)]
    pub edit_milestone: String,
    #[serde(default)]
    pub close_milestone: String,
    #[serde(default)]
    pub reopen_milestone: String,
    #[serde(default)]
    pub delete_milestone: String,
    #[serde(default)]
    pub open_in_browser: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindingJobs {
    #[serde(default)]
    pub enter_pipeline: String,
    #[serde(default)]
    pub select_job: String,
    #[serde(default)]
    pub retry: String,
    #[serde(default)]
    pub select_stage: String,
    #[serde(default)]
    pub cancel: String,
    #[serde(default)]
    pub download_artifact: String,
    #[serde(default)]
    pub open_in_browser: String,
    #[serde(default)]
    pub view_trace_editor: String,
    #[serde(default)]
    pub view_trace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindingRunners {
    #[serde(default)]
    pub pause: String,
    #[serde(default)]
    pub resume: String,
    #[serde(default)]
    pub edit_description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindingTodos {
    #[serde(default)]
    pub mark_as_read: String,
    #[serde(default)]
    pub open_in_browser: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindingConfig {
    #[serde(default)]
    pub global: KeybindingGlobal,
    #[serde(default)]
    pub issues: KeybindingIssues,
    #[serde(default)]
    pub mrs: KeybindingMrs,
    #[serde(default)]
    pub pipelines: KeybindingPipelines,
    #[serde(default)]
    pub releases: KeybindingReleases,
    #[serde(default)]
    pub milestones: KeybindingMilestones,
    #[serde(default)]
    pub jobs: KeybindingJobs,
    #[serde(default)]
    pub runners: KeybindingRunners,
    #[serde(default)]
    pub todos: KeybindingTodos,
}

macro_rules! keybind_defaults {
    ( $( $name:ident = $val:expr ),+ $(,)? ) => {
        $(
            fn $name() -> String { $val.to_string() }
        )+
    };
}

keybind_defaults! {
    def_quit = "q",
    def_help = "?",
    def_search = "/",
    def_refresh = "Ctrl+r",
    def_configure = "Tab",
    def_next_tab = "l",
    def_prev_tab = "h",
    def_scroll_down = "J",
    def_scroll_up = "K",
    def_create_issue = "n",
    def_edit_entity = "e",
    def_close_entity = "c",
    def_reopen_entity = "r",
    def_create_mr = "n",
    def_approve_mr = "a",
    def_merge_mr = "m",
    def_toggle_draft = "s",
    def_view_diff = "v",
    def_trigger_pipeline = "p",
    def_retry = "r",
    def_cancel = "d",
    def_download_artifact = "d",
    def_create_release = "n",
    def_edit_release = "e",
    def_delete_release = "d",
    def_create_milestone = "n",
    def_edit_milestone = "e",
    def_close_milestone = "c",
    def_reopen_milestone = "r",
    def_delete_milestone = "d",
    def_open_in_browser = "o",
    def_enter_pipeline = "p",
    def_select_job = "Space",
    def_retry_job = "r",
    def_select_stage = "s",
    def_cancel_job = "c",
    def_download_artifact_job = "d",
    def_open_in_browser_job = "o",
    def_view_trace_editor = "e",
    def_view_trace = "Enter",
    def_pause_runner = "p",
    def_resume_runner = "r",
    def_edit_description = "e",
    def_mark_as_read = "Enter",
    def_open_in_browser_todo = "o",
}

impl Default for KeybindingGlobal {
    fn default() -> Self {
        Self {
            quit: def_quit(),
            help: def_help(),
            search: def_search(),
            refresh: def_refresh(),
            configure: def_configure(),
            next_tab: def_next_tab(),
            prev_tab: def_prev_tab(),
            scroll_down: def_scroll_down(),
            scroll_up: def_scroll_up(),
        }
    }
}

impl Default for KeybindingIssues {
    fn default() -> Self {
        Self {
            create_issue: def_create_issue(),
            edit_entity: def_edit_entity(),
            close_entity: def_close_entity(),
            reopen_entity: def_reopen_entity(),
        }
    }
}

impl Default for KeybindingMrs {
    fn default() -> Self {
        Self {
            create_mr: def_create_mr(),
            approve_mr: def_approve_mr(),
            merge_mr: def_merge_mr(),
            toggle_draft: def_toggle_draft(),
            view_diff: def_view_diff(),
            edit_entity: def_edit_entity(),
            close_entity: def_close_entity(),
            reopen_entity: def_reopen_entity(),
        }
    }
}

impl Default for KeybindingPipelines {
    fn default() -> Self {
        Self {
            trigger_pipeline: def_trigger_pipeline(),
            retry: def_retry(),
            cancel: def_cancel(),
            download_artifact: def_download_artifact(),
        }
    }
}

impl Default for KeybindingReleases {
    fn default() -> Self {
        Self {
            create_release: def_create_release(),
            edit_release: def_edit_release(),
            delete_release: def_delete_release(),
            open_in_browser: def_open_in_browser(),
        }
    }
}

impl Default for KeybindingMilestones {
    fn default() -> Self {
        Self {
            create_milestone: def_create_milestone(),
            edit_milestone: def_edit_milestone(),
            close_milestone: def_close_milestone(),
            reopen_milestone: def_reopen_milestone(),
            delete_milestone: def_delete_milestone(),
            open_in_browser: def_open_in_browser(),
        }
    }
}

impl Default for KeybindingJobs {
    fn default() -> Self {
        Self {
            enter_pipeline: def_enter_pipeline(),
            select_job: def_select_job(),
            retry: def_retry_job(),
            select_stage: def_select_stage(),
            cancel: def_cancel_job(),
            download_artifact: def_download_artifact_job(),
            open_in_browser: def_open_in_browser_job(),
            view_trace_editor: def_view_trace_editor(),
            view_trace: def_view_trace(),
        }
    }
}

impl Default for KeybindingRunners {
    fn default() -> Self {
        Self {
            pause: def_pause_runner(),
            resume: def_resume_runner(),
            edit_description: def_edit_description(),
        }
    }
}

impl Default for KeybindingTodos {
    fn default() -> Self {
        Self {
            mark_as_read: def_mark_as_read(),
            open_in_browser: def_open_in_browser_todo(),
        }
    }
}

impl Default for KeybindingConfig {
    fn default() -> Self {
        Self {
            global: KeybindingGlobal::default(),
            issues: KeybindingIssues::default(),
            mrs: KeybindingMrs::default(),
            pipelines: KeybindingPipelines::default(),
            releases: KeybindingReleases::default(),
            milestones: KeybindingMilestones::default(),
            jobs: KeybindingJobs::default(),
            runners: KeybindingRunners::default(),
            todos: KeybindingTodos::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneConfig {
    pub columns: Option<Vec<String>>,
    pub column_filters: HashMap<String, Vec<String>>,
    pub group_by_column: Option<String>,
    pub group_ascending: bool,
}

impl Default for PaneConfig {
    fn default() -> Self {
        Self {
            columns: None,
            column_filters: HashMap::new(),
            group_by_column: None,
            group_ascending: true,
        }
    }
}

fn def_page_size() -> usize {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub theme_preset: Option<String>,
    pub theme: ThemeOverrides,
    pub keybindings: KeybindingConfig,
    #[serde(default = "def_page_size")]
    pub page_size: usize,
    pub disabled_tabs: Option<Vec<String>>,
    pub issues: PaneConfig,
    pub mrs: PaneConfig,
    pub pipelines: PaneConfig,
    pub jobs: PaneConfig,
    pub runners: PaneConfig,
    pub releases: PaneConfig,
    pub todos: PaneConfig,
    pub milestones: PaneConfig,
    pub terminal: PaneConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme_preset: Some("default".to_string()),
            theme: ThemeOverrides::default(),
            keybindings: KeybindingConfig::default(),
            page_size: def_page_size(),
            disabled_tabs: None,
            issues: PaneConfig::default(),
            mrs: PaneConfig::default(),
            pipelines: PaneConfig::default(),
            jobs: PaneConfig::default(),
            runners: PaneConfig::default(),
            releases: PaneConfig::default(),
            todos: PaneConfig::default(),
            milestones: PaneConfig::default(),
            terminal: PaneConfig::default(),
        }
    }
}

impl Config {
    fn config_path() -> PathBuf {
        let mut path = config_dir();
        let _ = std::fs::create_dir_all(&path);
        path.push("config.toml");
        path
    }

    fn generate_default_toml() -> String {
        let theme = Theme::default();
        format!(
            r##"# glab-tui configuration
# See https://github.com/rcieri/glab-tui for documentation

# Theme preset: "default", "tokyo-night", "gruvbox", "nord", "catppuccin-mocha", "dracula"
theme_preset = "default"

# Default request page size
page_size = 100

# Per-color overrides (takes precedence over theme_preset).
# Uncomment the [theme] line and any colors you want to override.
# [theme]
# bg = "{bg}"
# border = "{border}"
# border_focused = "{border_focused}"
# header_fg = "{header_fg}"
# highlight_bg = "{highlight_bg}"
# inactive_bg = "{inactive_bg}"
# text_normal = "{text_normal}"
# text_muted = "{text_muted}"
# checked_bg = "{checked_bg}"
# green = "{green}"
# green_bg = "{green_bg}"
# red = "{red}"
# red_bg = "{red_bg}"
# blue = "{blue}"
# blue_bg = "{blue_bg}"
# yellow = "{yellow}"
# yellow_bg = "{yellow_bg}"
# purple = "{purple}"
# purple_bg = "{purple_bg}"

[keybindings.global]
quit = "q"
help = "?"
search = "/"
refresh = "Ctrl+r"
configure = "Tab"
next_tab = "l"
prev_tab = "h"
scroll_down = "J"
scroll_up = "K"

[keybindings.issues]
create_issue = "n"
edit_entity = "e"
close_entity = "c"
reopen_entity = "r"

[keybindings.mrs]
create_mr = "n"
approve_mr = "a"
merge_mr = "m"
toggle_draft = "s"
view_diff = "v"
edit_entity = "e"
close_entity = "c"
reopen_entity = "r"

[keybindings.pipelines]
trigger_pipeline = "p"
retry = "r"
cancel = "d"
download_artifact = "d"

[keybindings.releases]
create_release = "n"
edit_release = "e"
delete_release = "d"
open_in_browser = "o"

[keybindings.milestones]
create_milestone = "n"
edit_milestone = "e"
close_milestone = "c"
reopen_milestone = "r"
delete_milestone = "d"
open_in_browser = "o"

[keybindings.jobs]
enter_pipeline = "p"
select_job = "Space"
retry = "r"
select_stage = "s"
cancel = "c"
download_artifact = "d"
open_in_browser = "o"
view_trace_editor = "e"
view_trace = "Enter"

[keybindings.runners]
pause = "p"
resume = "r"
edit_description = "e"

[keybindings.todos]
mark_as_read = "Enter"
open_in_browser = "o"

# Tabs to disable/hide from the sidebar.
# Uncomment to disable specific tabs:
# disabled_tabs = ["Runners", "Terminal"]

# Per-pane column config (unset = show all columns)
# [issues]
# columns = ["ID", "State", "Title", "Labels"]
# [issues.column_filters]
# State = ["opened"]
# group_by_column = "State"
# group_ascending = true

# [mrs]
# columns = ["ID", "State", "Status", "Title", "Labels"]
"##,
            bg = color_to_hex(theme.bg),
            border = color_to_hex(theme.border),
            border_focused = color_to_hex(theme.border_focused),
            header_fg = color_to_hex(theme.header_fg),
            highlight_bg = color_to_hex(theme.highlight_bg),
            inactive_bg = color_to_hex(theme.inactive_bg),
            text_normal = color_to_hex(theme.text_normal),
            text_muted = color_to_hex(theme.text_muted),
            checked_bg = color_to_hex(theme.checked_bg),
            green = color_to_hex(theme.green),
            green_bg = color_to_hex(theme.green_bg),
            red = color_to_hex(theme.red),
            red_bg = color_to_hex(theme.red_bg),
            blue = color_to_hex(theme.blue),
            blue_bg = color_to_hex(theme.blue_bg),
            yellow = color_to_hex(theme.yellow),
            yellow_bg = color_to_hex(theme.yellow_bg),
            purple = color_to_hex(theme.purple),
            purple_bg = color_to_hex(theme.purple_bg),
        )
    }

    pub fn load() -> Self {
        ensure_themes();
        let path = Self::config_path();
        if !path.exists() {
            let toml_str = Self::generate_default_toml();
            let _ = std::fs::write(&path, &toml_str);
        }

        let global_contents = std::fs::read_to_string(&path).unwrap_or_default();
        let mut merged_value: toml::Value = toml::from_str(&global_contents)
            .unwrap_or_else(|_| toml::Value::Table(toml::Table::new()));

        fn find_git_root() -> Option<PathBuf> {
            let mut current = std::env::current_dir().ok()?;
            loop {
                let git_dir = current.join(".git");
                if git_dir.exists() {
                    return Some(current);
                }
                if !current.pop() {
                    break;
                }
            }
            None
        }

        fn merge_toml_values(base: &mut toml::Value, overrides: toml::Value) {
            match (base, overrides) {
                (toml::Value::Table(base_table), toml::Value::Table(overrides_table)) => {
                    for (key, val) in overrides_table {
                        match base_table.entry(key) {
                            toml::map::Entry::Occupied(mut entry) => {
                                merge_toml_values(entry.get_mut(), val);
                            }
                            toml::map::Entry::Vacant(entry) => {
                                entry.insert(val);
                            }
                        }
                    }
                }
                (base, overrides) => {
                    *base = overrides;
                }
            }
        }

        if let Some(root) = find_git_root() {
            let paths = [
                root.join(".glab-tui").join("config.toml"),
                root.join(".config").join("glab-tui").join("config.toml"),
            ];
            for p in &paths {
                if p.exists() {
                    if let Ok(workspace_contents) = std::fs::read_to_string(p) {
                        if let Ok(workspace_val) =
                            toml::from_str::<toml::Value>(&workspace_contents)
                        {
                            merge_toml_values(&mut merged_value, workspace_val);
                        }
                    }
                }
            }
        }

        match Config::deserialize(merged_value) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("Error deserializing merged config: {}. Using defaults.", e);
                Config::default()
            }
        }
    }

    pub fn save(&self) {
        let path = Self::config_path();
        match toml::to_string(self) {
            Ok(toml_str) => {
                let _ = std::fs::write(&path, &toml_str);
            }
            Err(e) => {
                eprintln!("Error serializing config: {}", e);
            }
        }
    }

    pub fn resolve_theme(&self) -> Theme {
        let mut theme = if let Some(ref preset) = self.theme_preset {
            Theme::preset(preset).unwrap_or_else(Theme::default)
        } else {
            Theme::default()
        };
        apply_overrides(&mut theme, &self.theme);
        theme
    }
}

pub static THEME: Lazy<RwLock<Theme>> = Lazy::new(|| RwLock::new(Config::load().resolve_theme()));

pub const THEME_PRESETS: &[&str] = &[
    "default",
    "tokyo-night",
    "gruvbox",
    "nord",
    "catppuccin-mocha",
    "dracula",
];

pub fn reload_theme() {
    if let Ok(mut theme) = THEME.write() {
        *theme = Config::load().resolve_theme();
    }
}

pub fn set_theme_preset(name: &str) {
    if let Some(preset) = Theme::preset(name) {
        if let Ok(mut theme) = THEME.write() {
            *theme = preset;
        }
    }
}
