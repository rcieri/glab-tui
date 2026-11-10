//! The `[layout]` config section: which tabs show and in what order, where
//! the navigation, preview and terminal panes go, and how big they are.

use crate::app::Tab;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Where the preview pane sits next to the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreviewPlacement {
    /// Right of the list, hidden while the workspace is narrower than
    /// `preview_min_width`.
    #[default]
    Right,
    /// Below the list, at any width.
    Bottom,
    /// Right of the list, moving below it while the workspace is narrower
    /// than `preview_min_width`.
    Auto,
    /// Never beside the list. A zoomed preview or an edit form still fills
    /// the workspace.
    Off,
}

impl PreviewPlacement {
    pub const ALL: [PreviewPlacement; 4] = [
        PreviewPlacement::Right,
        PreviewPlacement::Bottom,
        PreviewPlacement::Auto,
        PreviewPlacement::Off,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PreviewPlacement::Right => "right",
            PreviewPlacement::Bottom => "bottom",
            PreviewPlacement::Auto => "auto",
            PreviewPlacement::Off => "off",
        }
    }
}

/// The size of a pane along the axis it is split on: `"auto"`, a number of
/// terminal cells (`40`) or a share of the room it is split from (`"35%"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaneSize {
    /// glab-tui picks the size from the terminal size.
    #[default]
    Auto,
    Cells(u16),
    Percent(u16),
}

impl PaneSize {
    pub fn label(self) -> String {
        match self {
            PaneSize::Auto => "auto".to_string(),
            PaneSize::Cells(cells) => cells.to_string(),
            PaneSize::Percent(percent) => format!("{percent}%"),
        }
    }
}

const PANE_SIZE_HINT: &str =
    "use \"auto\", a number of cells such as 40, or a percentage from 1% to 100% such as \"35%\"";

impl std::str::FromStr for PaneSize {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("auto") {
            return Ok(PaneSize::Auto);
        }
        let parsed = match text.strip_suffix('%') {
            Some(percent) => percent
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|p| (1..=100).contains(p))
                .map(PaneSize::Percent),
            None => text
                .parse::<u16>()
                .ok()
                .filter(|&cells| cells > 0)
                .map(PaneSize::Cells),
        };
        parsed.ok_or_else(|| format!("invalid pane size {text:?}; {PANE_SIZE_HINT}"))
    }
}

impl Serialize for PaneSize {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            PaneSize::Cells(cells) => serializer.serialize_u16(*cells),
            other => serializer.serialize_str(&other.label()),
        }
    }
}

impl<'de> Deserialize<'de> for PaneSize {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Cells(i64),
            Text(String),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Cells(cells) => u16::try_from(cells)
                .ok()
                .filter(|&cells| cells > 0)
                .map(PaneSize::Cells)
                .ok_or_else(|| {
                    serde::de::Error::custom(format!("invalid pane size {cells}; {PANE_SIZE_HINT}"))
                }),
            Raw::Text(text) => text.parse().map_err(serde::de::Error::custom),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    /// Tabs to show, in order. A tab left out is hidden. `None` shows every
    /// tab in the built-in order.
    #[serde(with = "tab_names", skip_serializing_if = "Option::is_none")]
    pub tabs: Option<Vec<Tab>>,
    pub nav_visible: bool,
    pub nav_width: u16,
    /// The navigation sidebar hides while the terminal is narrower.
    pub nav_min_width: u16,
    pub preview: PreviewPlacement,
    /// Width of a preview right of the list.
    pub preview_width: PaneSize,
    /// Height of a preview below the list. `auto` is half the list column.
    pub preview_height: PaneSize,
    /// Below this terminal width a `right` preview hides and an `auto`
    /// preview moves below the list.
    pub preview_min_width: u16,
    pub terminal_pane_visible: bool,
    pub terminal_pane_height: u16,
    /// The terminal pane hides while the terminal is shorter.
    pub terminal_pane_min_height: u16,
}

pub const DEFAULT_NAV_WIDTH: u16 = 22;
pub const DEFAULT_NAV_MIN_WIDTH: u16 = 80;
pub const DEFAULT_PREVIEW_MIN_WIDTH: u16 = 90;
pub const DEFAULT_TERMINAL_PANE_HEIGHT: u16 = 6;
pub const DEFAULT_TERMINAL_PANE_MIN_HEIGHT: u16 = 18;

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            tabs: None,
            nav_visible: true,
            nav_width: DEFAULT_NAV_WIDTH,
            nav_min_width: DEFAULT_NAV_MIN_WIDTH,
            preview: PreviewPlacement::default(),
            preview_width: PaneSize::Auto,
            preview_height: PaneSize::Auto,
            preview_min_width: DEFAULT_PREVIEW_MIN_WIDTH,
            terminal_pane_visible: true,
            terminal_pane_height: DEFAULT_TERMINAL_PANE_HEIGHT,
            terminal_pane_min_height: DEFAULT_TERMINAL_PANE_MIN_HEIGHT,
        }
    }
}

impl LayoutConfig {
    /// Every tab in display order with whether it is shown: the listed tabs
    /// first, in list order, then the hidden ones in the built-in order.
    pub fn tab_order(&self) -> Vec<(Tab, bool)> {
        let Some(listed) = &self.tabs else {
            return Tab::ALL.iter().map(|&tab| (tab, true)).collect();
        };
        let mut order: Vec<(Tab, bool)> = Vec::with_capacity(Tab::ALL.len());
        for &tab in listed {
            if !order.iter().any(|(seen, _)| *seen == tab) {
                order.push((tab, true));
            }
        }
        for tab in Tab::ALL {
            if !listed.contains(&tab) {
                order.push((tab, false));
            }
        }
        order
    }

    /// Stores `order` as the tab list, keeping only the shown tabs.
    pub fn set_tab_order(&mut self, order: &[(Tab, bool)]) {
        self.tabs = Some(
            order
                .iter()
                .filter(|(_, shown)| *shown)
                .map(|(tab, _)| *tab)
                .collect(),
        );
    }

    /// The keys of `self` whose values differ from `base`, as a TOML table.
    pub fn diff_from(&self, base: &LayoutConfig) -> toml::Table {
        let (Ok(toml::Value::Table(mine)), Ok(toml::Value::Table(theirs))) =
            (toml::Value::try_from(self), toml::Value::try_from(base))
        else {
            return toml::Table::new();
        };
        mine.into_iter()
            .filter(|(key, value)| theirs.get(key) != Some(value))
            .collect()
    }
}

/// Reads a tab name the way `--tab` and `active_tab` do, also accepting the
/// sidebar title (`Merge Requests`, `Notifications`).
pub fn parse_tab_name(name: &str) -> Option<Tab> {
    let trimmed = name.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
    Tab::from_str(trimmed).or_else(|| {
        let joined: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
        Tab::from_str(&joined)
    })
}

mod tab_names {
    use super::parse_tab_name;
    use crate::app::Tab;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        tabs: &Option<Vec<Tab>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match tabs {
            Some(tabs) => serializer.collect_seq(tabs.iter().map(|tab| tab.to_str())),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Vec<Tab>>, D::Error> {
        let names = Vec::<String>::deserialize(deserializer)?;
        names
            .iter()
            .map(|name| {
                parse_tab_name(name).ok_or_else(|| {
                    serde::de::Error::custom(format!(
                        "unknown tab {name:?} in layout.tabs; use issues, mrs, pipelines, jobs, runners, releases, todos, milestones, branches, environments or terminal"
                    ))
                })
            })
            .collect::<Result<Vec<Tab>, _>>()
            .map(Some)
    }
}

/// Top-level keys that `[layout]` replaced. They are still read, for one
/// release, from a file whose `[layout]` does not set the same thing.
const LEGACY_DISABLED_TABS: &str = "disabled_tabs";
const LEGACY_UI: &str = "ui";
const LEGACY_UI_KEYS: [(&str, &str); 3] = [
    ("sidebar_visible", "nav_visible"),
    ("sidebar_width", "nav_width"),
    ("terminal_pane_visible", "terminal_pane_visible"),
];

/// Moves the deprecated `disabled_tabs` and `[ui]` keys of one parsed config
/// file into its `[layout]` table. Done per file before merging, so a
/// repo-local `disabled_tabs` still overrides a global `layout.tabs`.
pub(super) fn migrate_legacy_keys(file: &mut toml::Value) {
    let Some(root) = file.as_table_mut() else {
        return;
    };
    let disabled_tabs = root.remove(LEGACY_DISABLED_TABS);
    let ui = root.remove(LEGACY_UI);
    if disabled_tabs.is_none() && ui.is_none() {
        return;
    }
    let layout = root
        .entry("layout")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let Some(layout) = layout.as_table_mut() else {
        return;
    };
    if let Some(toml::Value::Array(disabled)) = disabled_tabs
        && !layout.contains_key("tabs")
    {
        let disabled: Vec<Tab> = disabled
            .iter()
            .filter_map(toml::Value::as_str)
            .filter_map(parse_tab_name)
            .collect();
        let shown = Tab::ALL
            .iter()
            .filter(|tab| !disabled.contains(tab))
            .map(|tab| toml::Value::String(tab.to_str().to_string()))
            .collect();
        layout.insert("tabs".to_string(), toml::Value::Array(shown));
    }
    if let Some(toml::Value::Table(mut ui)) = ui {
        for (legacy, current) in LEGACY_UI_KEYS {
            if let Some(value) = ui.remove(legacy)
                && !layout.contains_key(current)
            {
                layout.insert(current.to_string(), value);
            }
        }
    }
}

/// Removes the deprecated keys from a file `save_layout` rewrites, since the
/// `[layout]` table written next to them carries the whole effective layout.
pub(super) fn remove_legacy_keys(file: &mut toml::Table) {
    file.remove(LEGACY_DISABLED_TABS);
    file.remove(LEGACY_UI);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout_from(toml: &str) -> LayoutConfig {
        let mut value: toml::Value = toml::from_str(toml).unwrap();
        migrate_legacy_keys(&mut value);
        value
            .get("layout")
            .cloned()
            .map(|layout| layout.try_into().unwrap())
            .unwrap_or_default()
    }

    #[test]
    fn tab_list_sets_order_and_hides_unlisted_tabs() {
        let layout = layout_from(
            r#"
            [layout]
            tabs = ["MergeRequests", "Issues", "Pipelines", "Todos", "Terminal"]
            "#,
        );

        assert_eq!(
            layout.tabs,
            Some(vec![
                Tab::MergeRequests,
                Tab::Issues,
                Tab::Pipelines,
                Tab::Todos,
                Tab::Terminal
            ])
        );
        let order = layout.tab_order();
        assert_eq!(order[0], (Tab::MergeRequests, true));
        assert_eq!(order[5], (Tab::Jobs, false));
        assert_eq!(order.len(), Tab::ALL.len());
    }

    #[test]
    fn unknown_tab_name_is_rejected_with_the_name() {
        let error = toml::from_str::<LayoutConfig>(r#"tabs = ["issues", "wiki"]"#).unwrap_err();
        assert!(
            error.to_string().contains("unknown tab \"wiki\""),
            "{error}"
        );
    }

    #[test]
    fn disabled_tabs_become_the_tab_list_without_them() {
        let layout = layout_from(r#"disabled_tabs = ["Runners", "Merge Requests"]"#);

        let expected: Vec<Tab> = Tab::ALL
            .into_iter()
            .filter(|tab| !matches!(tab, Tab::Runners | Tab::MergeRequests))
            .collect();
        assert_eq!(layout.tabs, Some(expected));
    }

    #[test]
    fn layout_tabs_win_over_disabled_tabs_in_the_same_file() {
        let layout = layout_from(
            r#"
            disabled_tabs = ["Runners"]
            [layout]
            tabs = ["issues"]
            "#,
        );

        assert_eq!(layout.tabs, Some(vec![Tab::Issues]));
    }

    #[test]
    fn legacy_ui_keys_fill_unset_layout_keys() {
        let layout = layout_from(
            r#"
            [ui]
            sidebar_visible = false
            sidebar_width = 30
            terminal_pane_visible = false
            [layout]
            nav_width = 26
            "#,
        );

        assert!(!layout.nav_visible);
        assert_eq!(layout.nav_width, 26);
        assert!(!layout.terminal_pane_visible);
    }

    #[test]
    fn pane_sizes_read_auto_cells_and_percentages() {
        let layout: LayoutConfig = toml::from_str(
            r#"
            preview_width = 40
            preview_height = "35%"
            "#,
        )
        .unwrap();

        assert_eq!(layout.preview_width, PaneSize::Cells(40));
        assert_eq!(layout.preview_height, PaneSize::Percent(35));
        assert_eq!("auto".parse::<PaneSize>(), Ok(PaneSize::Auto));
        assert_eq!(
            "0%".parse::<PaneSize>(),
            Err(format!("invalid pane size \"0%\"; {PANE_SIZE_HINT}"))
        );
    }

    #[test]
    fn diff_keeps_only_changed_keys_and_round_trips() {
        let layout = LayoutConfig {
            tabs: Some(vec![Tab::MergeRequests, Tab::Issues]),
            preview: PreviewPlacement::Bottom,
            preview_width: PaneSize::Percent(40),
            ..LayoutConfig::default()
        };

        let diff = layout.diff_from(&LayoutConfig::default());

        let keys: Vec<&str> = diff.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["preview", "preview_width", "tabs"]);
        let reloaded: LayoutConfig = toml::Value::Table(diff).try_into().unwrap();
        assert_eq!(reloaded, layout);
    }
}
