//! The rows of the Configure View, in display order. Its key handler, its
//! renderer and its mouse handler all walk the same list, so a row's index
//! means the same thing to each of them.

use crate::app::{App, Tab};
use crate::config::{LayoutConfig, PaneSize, PreviewPlacement};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigureSection {
    Columns,
    GroupBy,
    Order,
    View,
    Tabs,
    Layout,
    Save,
}

impl ConfigureSection {
    /// Heading drawn above the section, `None` for the untitled ones.
    pub fn heading(self) -> Option<&'static str> {
        match self {
            ConfigureSection::Columns => Some("COLUMNS"),
            ConfigureSection::GroupBy => Some("GROUP BY"),
            ConfigureSection::Order => Some("ORDER"),
            ConfigureSection::Tabs => Some("TABS (all tabs)"),
            ConfigureSection::Layout => Some("LAYOUT (all tabs)"),
            ConfigureSection::View | ConfigureSection::Save => None,
        }
    }
}

/// A `[layout]` pane setting the LAYOUT section edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutSetting {
    NavVisible,
    NavWidth,
    Preview,
    PreviewWidth,
    PreviewHeight,
    TerminalPaneVisible,
    TerminalPaneHeight,
}

const PREVIEW_WIDTH_CHOICES: [PaneSize; 7] = [
    PaneSize::Auto,
    PaneSize::Percent(25),
    PaneSize::Percent(30),
    PaneSize::Percent(35),
    PaneSize::Percent(40),
    PaneSize::Percent(50),
    PaneSize::Percent(60),
];
const PREVIEW_HEIGHT_CHOICES: [PaneSize; 6] = [
    PaneSize::Auto,
    PaneSize::Percent(30),
    PaneSize::Percent(40),
    PaneSize::Percent(50),
    PaneSize::Percent(60),
    PaneSize::Percent(70),
];
const NAV_WIDTH_RANGE: std::ops::RangeInclusive<u16> = 12..=40;
const NAV_WIDTH_STEP: u16 = 2;
const TERMINAL_PANE_HEIGHT_RANGE: std::ops::RangeInclusive<u16> = 3..=20;

impl LayoutSetting {
    pub const ALL: [LayoutSetting; 7] = [
        LayoutSetting::NavVisible,
        LayoutSetting::NavWidth,
        LayoutSetting::Preview,
        LayoutSetting::PreviewWidth,
        LayoutSetting::PreviewHeight,
        LayoutSetting::TerminalPaneVisible,
        LayoutSetting::TerminalPaneHeight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LayoutSetting::NavVisible => "Navigation",
            LayoutSetting::NavWidth => "Navigation width",
            LayoutSetting::Preview => "Preview",
            LayoutSetting::PreviewWidth => "Preview width",
            LayoutSetting::PreviewHeight => "Preview height",
            LayoutSetting::TerminalPaneVisible => "Terminal pane",
            LayoutSetting::TerminalPaneHeight => "Terminal pane height",
        }
    }

    /// The longest label, so every setting's value starts in one column.
    pub fn label_width() -> usize {
        Self::ALL
            .iter()
            .map(|setting| setting.label().len())
            .max()
            .unwrap_or(0)
    }

    pub fn value(self, layout: &LayoutConfig) -> String {
        let shown = |is_shown: bool| if is_shown { "shown" } else { "hidden" }.to_string();
        match self {
            LayoutSetting::NavVisible => shown(layout.nav_visible),
            LayoutSetting::NavWidth => layout.nav_width.to_string(),
            LayoutSetting::Preview => layout.preview.label().to_string(),
            LayoutSetting::PreviewWidth => layout.preview_width.label(),
            LayoutSetting::PreviewHeight => layout.preview_height.label(),
            LayoutSetting::TerminalPaneVisible => shown(layout.terminal_pane_visible),
            LayoutSetting::TerminalPaneHeight => layout.terminal_pane_height.to_string(),
        }
    }

    /// Moves the setting to its next value (`forward`) or its previous one.
    /// Switches flip, choices wrap around, numbers stop at their range.
    pub fn step(self, layout: &mut LayoutConfig, forward: bool) {
        match self {
            LayoutSetting::NavVisible => layout.nav_visible = !layout.nav_visible,
            LayoutSetting::TerminalPaneVisible => {
                layout.terminal_pane_visible = !layout.terminal_pane_visible;
            }
            LayoutSetting::Preview => {
                layout.preview = cycle(&PreviewPlacement::ALL, layout.preview, forward);
            }
            LayoutSetting::PreviewWidth => {
                layout.preview_width = cycle(&PREVIEW_WIDTH_CHOICES, layout.preview_width, forward);
            }
            LayoutSetting::PreviewHeight => {
                layout.preview_height =
                    cycle(&PREVIEW_HEIGHT_CHOICES, layout.preview_height, forward);
            }
            LayoutSetting::NavWidth => {
                layout.nav_width =
                    step_within(layout.nav_width, NAV_WIDTH_STEP, NAV_WIDTH_RANGE, forward);
            }
            LayoutSetting::TerminalPaneHeight => {
                layout.terminal_pane_height = step_within(
                    layout.terminal_pane_height,
                    1,
                    TERMINAL_PANE_HEIGHT_RANGE,
                    forward,
                );
            }
        }
    }
}

/// The choice after (or before) `current`; a value that is not one of the
/// choices (set by hand in `config.toml`) goes to the first one.
fn cycle<T: Copy + PartialEq>(choices: &[T], current: T, forward: bool) -> T {
    let Some(pos) = choices.iter().position(|c| *c == current) else {
        return choices[0];
    };
    let next = if forward {
        (pos + 1) % choices.len()
    } else {
        (pos + choices.len() - 1) % choices.len()
    };
    choices[next]
}

/// `value` moved by `step` and kept within `range`, never moving the wrong
/// way: a value set outside the range by hand stays put on a step away from
/// the range.
fn step_within(value: u16, step: u16, range: std::ops::RangeInclusive<u16>, forward: bool) -> u16 {
    let stepped = if forward {
        value.saturating_add(step)
    } else {
        value.saturating_sub(step)
    };
    let clamped = stepped.clamp(*range.start(), *range.end());
    if forward {
        clamped.max(value)
    } else {
        clamped.min(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigureRow {
    Column(&'static str),
    GroupBy(&'static str),
    Order { ascending: bool },
    PageSize,
    PrefetchTabs,
    Theme,
    Tab(Tab),
    Layout(LayoutSetting),
    Save,
}

impl ConfigureRow {
    pub fn section(self) -> ConfigureSection {
        match self {
            ConfigureRow::Column(_) => ConfigureSection::Columns,
            ConfigureRow::GroupBy(_) => ConfigureSection::GroupBy,
            ConfigureRow::Order { .. } => ConfigureSection::Order,
            ConfigureRow::PageSize | ConfigureRow::PrefetchTabs | ConfigureRow::Theme => {
                ConfigureSection::View
            }
            ConfigureRow::Tab(_) => ConfigureSection::Tabs,
            ConfigureRow::Layout(_) => ConfigureSection::Layout,
            ConfigureRow::Save => ConfigureSection::Save,
        }
    }
}

/// Every row of the Configure View for the active tab, top to bottom.
pub fn configure_rows(app: &App) -> Vec<ConfigureRow> {
    let tab = app.active_tab;
    let mut rows: Vec<ConfigureRow> = app
        .ordered_columns(tab)
        .into_iter()
        .map(ConfigureRow::Column)
        .collect();
    rows.extend(
        tab.columns(app.kind(), app.scope.is_group())
            .into_iter()
            .map(ConfigureRow::GroupBy),
    );
    rows.extend([
        ConfigureRow::Order { ascending: true },
        ConfigureRow::Order { ascending: false },
        ConfigureRow::PageSize,
        ConfigureRow::PrefetchTabs,
        ConfigureRow::Theme,
    ]);
    rows.extend(
        app.config
            .layout
            .tab_order()
            .into_iter()
            .filter(|(tab, _)| tab.is_enabled_in(&app.config))
            .map(|(tab, _)| ConfigureRow::Tab(tab)),
    );
    rows.extend(LayoutSetting::ALL.map(ConfigureRow::Layout));
    rows.push(ConfigureRow::Save);
    rows
}

/// Shows or hides `tab`. The last shown tab `is_enabled` accepts cannot be
/// hidden: returns false and changes nothing. A tab shown again goes after
/// the last shown tab that precedes it in the built-in order, so hiding and
/// showing a tab of the default order stores no tab list.
pub fn toggle_tab(layout: &mut LayoutConfig, tab: Tab, is_enabled: impl Fn(Tab) -> bool) -> bool {
    let mut order = layout.tab_order();
    let shown_count = order
        .iter()
        .filter(|&&(t, shown)| shown && is_enabled(t))
        .count();
    let Some(index) = order.iter().position(|(t, _)| *t == tab) else {
        return false;
    };
    if order[index].1 {
        if shown_count == 1 {
            return false;
        }
        order[index].1 = false;
    } else {
        order.remove(index);
        let built_in_rank = |t: Tab| Tab::ALL.iter().position(|all| *all == t);
        let insert_at = order
            .iter()
            .rposition(|&(t, shown)| shown && built_in_rank(t) < built_in_rank(tab))
            .map_or(0, |pos| pos + 1);
        order.insert(insert_at, (tab, true));
    }
    layout.set_tab_order(&order);
    true
}

/// Swaps a shown `tab` with the next (`towards_end`) or previous shown tab
/// `is_enabled` accepts. Shown tabs it rejects keep their stored place, and
/// hidden tabs have no position to move. Returns whether it moved.
pub fn move_tab(
    layout: &mut LayoutConfig,
    tab: Tab,
    towards_end: bool,
    is_enabled: impl Fn(Tab) -> bool,
) -> bool {
    let mut shown: Vec<Tab> = layout
        .tab_order()
        .into_iter()
        .filter(|(_, shown)| *shown)
        .map(|(tab, _)| tab)
        .collect();
    let Some(from) = shown.iter().position(|t| *t == tab) else {
        return false;
    };
    let neighbour = if towards_end {
        (from + 1..shown.len()).find(|&i| is_enabled(shown[i]))
    } else {
        (0..from).rev().find(|&i| is_enabled(shown[i]))
    };
    let Some(to) = neighbour else {
        return false;
    };
    shown.swap(from, to);
    let order: Vec<(Tab, bool)> = shown.into_iter().map(|tab| (tab, true)).collect();
    layout.set_tab_order(&order);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout_with(tabs: &[Tab]) -> LayoutConfig {
        LayoutConfig {
            tabs: Some(tabs.to_vec()),
            ..LayoutConfig::default()
        }
    }

    fn every_tab(_: Tab) -> bool {
        true
    }

    fn without_dashboard(tab: Tab) -> bool {
        tab != Tab::Dashboard
    }

    #[test]
    fn toggling_a_tab_hides_it_and_shows_it_again_in_its_built_in_place() {
        let mut layout = layout_with(&[Tab::Issues, Tab::MergeRequests, Tab::Pipelines]);

        assert!(toggle_tab(&mut layout, Tab::Issues, every_tab));
        assert_eq!(layout.tabs, Some(vec![Tab::MergeRequests, Tab::Pipelines]));
        assert!(toggle_tab(&mut layout, Tab::Issues, every_tab));
        assert_eq!(
            layout.tabs,
            Some(vec![Tab::Issues, Tab::MergeRequests, Tab::Pipelines])
        );
    }

    #[test]
    fn hiding_and_showing_a_tab_of_the_default_order_stores_no_tab_list() {
        for tab in Tab::ALL {
            let mut layout = LayoutConfig::default();

            assert!(toggle_tab(&mut layout, tab, every_tab));
            assert!(toggle_tab(&mut layout, tab, every_tab));
            assert_eq!(layout.tabs, None, "{tab:?}");
        }
    }

    #[test]
    fn a_tab_shown_again_follows_the_last_shown_tab_before_it_in_built_in_order() {
        let mut layout = layout_with(&[Tab::Terminal, Tab::MergeRequests]);

        assert!(toggle_tab(&mut layout, Tab::Pipelines, every_tab));
        assert_eq!(
            layout.tabs,
            Some(vec![Tab::Terminal, Tab::MergeRequests, Tab::Pipelines])
        );
    }

    #[test]
    fn the_last_shown_tab_cannot_be_hidden() {
        let mut layout = layout_with(&[Tab::Todos]);

        assert!(!toggle_tab(&mut layout, Tab::Todos, every_tab));
        assert_eq!(layout.tabs, Some(vec![Tab::Todos]));
    }

    #[test]
    fn with_the_dashboard_off_the_last_other_shown_tab_cannot_be_hidden() {
        let mut layout = LayoutConfig::default();
        let others: Vec<Tab> = Tab::ALL
            .into_iter()
            .filter(|&tab| without_dashboard(tab))
            .collect();

        for &tab in &others[1..] {
            assert!(toggle_tab(&mut layout, tab, without_dashboard), "{tab:?}");
        }
        assert!(!toggle_tab(&mut layout, others[0], without_dashboard));
        assert_eq!(layout.tabs, Some(vec![Tab::Dashboard, others[0]]));
    }

    #[test]
    fn moving_a_tab_swaps_it_among_the_shown_tabs_only() {
        let mut layout = layout_with(&[Tab::Issues, Tab::MergeRequests]);

        assert!(move_tab(&mut layout, Tab::Issues, true, every_tab));
        assert_eq!(layout.tabs, Some(vec![Tab::MergeRequests, Tab::Issues]));
        assert!(!move_tab(&mut layout, Tab::Issues, true, every_tab));
        assert!(!move_tab(&mut layout, Tab::Runners, false, every_tab));
    }

    #[test]
    fn moving_a_tab_skips_a_shown_tab_that_is_not_enabled() {
        let mut layout = layout_with(&[Tab::Dashboard, Tab::Issues, Tab::MergeRequests]);

        assert!(!move_tab(
            &mut layout,
            Tab::Issues,
            false,
            without_dashboard
        ));
        assert!(move_tab(
            &mut layout,
            Tab::MergeRequests,
            false,
            without_dashboard
        ));
        assert_eq!(
            layout.tabs,
            Some(vec![Tab::Dashboard, Tab::MergeRequests, Tab::Issues])
        );

        let mut layout = layout_with(&[Tab::Issues, Tab::Dashboard, Tab::MergeRequests]);
        assert!(move_tab(&mut layout, Tab::Issues, true, without_dashboard));
        assert_eq!(
            layout.tabs,
            Some(vec![Tab::MergeRequests, Tab::Dashboard, Tab::Issues])
        );
    }

    #[test]
    fn layout_settings_cycle_choices_and_stop_numbers_at_their_range() {
        let mut layout = LayoutConfig::default();

        LayoutSetting::Preview.step(&mut layout, false);
        assert_eq!(layout.preview, PreviewPlacement::Off);
        LayoutSetting::Preview.step(&mut layout, true);
        assert_eq!(layout.preview, PreviewPlacement::Auto);

        layout.preview_width = PaneSize::Cells(44);
        LayoutSetting::PreviewWidth.step(&mut layout, true);
        assert_eq!(layout.preview_width, PaneSize::Auto);

        layout.nav_width = 40;
        LayoutSetting::NavWidth.step(&mut layout, true);
        assert_eq!(layout.nav_width, 40);
        layout.terminal_pane_height = 3;
        LayoutSetting::TerminalPaneHeight.step(&mut layout, false);
        assert_eq!(layout.terminal_pane_height, 3);
    }

    #[test]
    fn a_number_set_outside_its_range_never_steps_further_away() {
        let mut layout = LayoutConfig {
            nav_width: 50,
            ..LayoutConfig::default()
        };

        LayoutSetting::NavWidth.step(&mut layout, true);
        assert_eq!(layout.nav_width, 50);
        LayoutSetting::NavWidth.step(&mut layout, false);
        assert_eq!(layout.nav_width, *NAV_WIDTH_RANGE.end());
    }
}
