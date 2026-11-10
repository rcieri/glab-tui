use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::app::Tab;
use crate::config::{LayoutConfig, PaneSize, PreviewPlacement};

pub const MIN_TERMINAL_WIDTH: u16 = 54;
pub const MIN_TERMINAL_HEIGHT: u16 = 10;
pub const HEADER_HEIGHT: u16 = 2;
pub const BANNER_MODE_WIDTH: u16 = 12;
/// `preview_width = "auto"`: wider terminals than this give the preview
/// `AUTO_PREVIEW_LARGE_PERCENTAGE` of the workspace.
pub const AUTO_PREVIEW_LARGE_WIDTH_THRESHOLD: u16 = 150;
/// `preview_width = "auto"`: wider terminals than this (up to the large
/// threshold) give the preview `AUTO_PREVIEW_MEDIUM_WIDTH` columns, narrower
/// ones `AUTO_PREVIEW_SMALL_WIDTH`.
pub const AUTO_PREVIEW_MEDIUM_WIDTH_THRESHOLD: u16 = 100;
pub const AUTO_PREVIEW_LARGE_PERCENTAGE: u16 = 35;
pub const AUTO_PREVIEW_MEDIUM_WIDTH: u16 = 45;
pub const AUTO_PREVIEW_SMALL_WIDTH: u16 = 30;
/// `preview_height = "auto"`: a bottom preview takes this share of the list
/// column.
pub const AUTO_PREVIEW_HEIGHT_PERCENTAGE: u16 = 50;

/// The pane settings of `[layout]`, everything but the tab list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneSettings {
    pub nav_visible: bool,
    pub nav_width: u16,
    pub nav_min_width: u16,
    pub preview: PreviewPlacement,
    pub preview_width: PaneSize,
    pub preview_height: PaneSize,
    pub preview_min_width: u16,
    pub terminal_pane_visible: bool,
    pub terminal_pane_height: u16,
    pub terminal_pane_min_height: u16,
}

impl From<&LayoutConfig> for PaneSettings {
    fn from(config: &LayoutConfig) -> Self {
        Self {
            nav_visible: config.nav_visible,
            nav_width: config.nav_width,
            nav_min_width: config.nav_min_width,
            preview: config.preview,
            preview_width: config.preview_width,
            preview_height: config.preview_height,
            preview_min_width: config.preview_min_width,
            terminal_pane_visible: config.terminal_pane_visible,
            terminal_pane_height: config.terminal_pane_height,
            terminal_pane_min_height: config.terminal_pane_min_height,
        }
    }
}

/// Inputs required to compute a [`LayoutPlan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutInputs {
    pub size: Rect,
    pub panes: PaneSettings,
    pub state: LayoutState,
}

/// Dynamic application state influencing layout calculation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutState {
    pub active_tab: Tab,
    pub detail_visible: bool,
    pub details_zoomed: bool,
    pub can_zoom: bool,
    pub edit_zoomed: bool,
}

/// Where the preview ends up for the current terminal size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewSlot {
    Right,
    Bottom,
    Hidden,
}

/// The computed geometry for all top-level UI regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutPlan {
    /// Whether the terminal size is below minimum dimensions (54x10).
    pub is_too_small: bool,
    /// Top header bar area (height 2).
    pub header_area: Rect,
    /// Left portion of the header bar (title, scope, search/filter, header notices).
    pub banner_title: Rect,
    /// Right portion of the header bar (mode indicator, width 12).
    pub banner_mode: Rect,
    /// Middle workspace area (between header and bottom).
    pub workspace_area: Rect,
    /// Sidebar navigation rectangle (width 0 if hidden or below width threshold).
    pub sidebar_rect: Rect,
    /// Main content rectangle (the table/list area, excluding the compact terminal pane).
    pub content_area: Rect,
    /// Detail/preview rectangle, right of or below the list (empty if hidden).
    pub detail_rect: Rect,
    /// Compact terminal pane rectangle at bottom of content column (height 0 if hidden).
    pub term_area: Rect,
}

impl LayoutPlan {
    /// Computes the pure layout geometry given the terminal dimensions, config, and state.
    pub fn compute(inputs: LayoutInputs) -> Self {
        let size = inputs.size;
        if size.width < MIN_TERMINAL_WIDTH || size.height < MIN_TERMINAL_HEIGHT {
            return Self {
                is_too_small: true,
                ..Default::default()
            };
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(HEADER_HEIGHT), Constraint::Min(0)])
            .split(size);
        let header_area = chunks[0];
        let workspace_area = chunks[1];

        let banner = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(BANNER_MODE_WIDTH)])
            .split(header_area);

        let plan = Self {
            is_too_small: false,
            header_area,
            banner_title: banner[0],
            banner_mode: banner[1],
            workspace_area,
            ..Default::default()
        };

        let state = inputs.state;
        let is_zoomed = (state.details_zoomed && state.can_zoom) || state.edit_zoomed;
        if is_zoomed {
            return Self {
                sidebar_rect: Rect::new(
                    workspace_area.x,
                    workspace_area.y,
                    0,
                    workspace_area.height,
                ),
                content_area: Rect::new(
                    workspace_area.x,
                    workspace_area.y,
                    0,
                    workspace_area.height,
                ),
                detail_rect: workspace_area,
                ..plan
            };
        }

        let panes = inputs.panes;
        let nav_width = if panes.nav_visible && size.width >= panes.nav_min_width {
            panes.nav_width
        } else {
            0
        };
        let slot = if state.active_tab == Tab::Terminal {
            PreviewSlot::Hidden
        } else {
            preview_slot(panes, size.width)
        };
        let shows_preview = state.detail_visible && state.active_tab != Tab::Terminal;

        let right_constraint = if shows_preview && slot == PreviewSlot::Right {
            preview_width_constraint(panes.preview_width, size.width)
        } else {
            Constraint::Length(0)
        };
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(nav_width),
                Constraint::Min(0),
                right_constraint,
            ])
            .split(workspace_area);
        let list_column = columns[1];

        let term_height = if state.active_tab != Tab::Terminal
            && panes.terminal_pane_visible
            && size.height >= panes.terminal_pane_min_height
        {
            panes.terminal_pane_height
        } else {
            0
        };
        let term_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(term_height)])
            .split(list_column);
        let bottom_constraint = if shows_preview && slot == PreviewSlot::Bottom {
            preview_height_constraint(panes.preview_height)
        } else {
            Constraint::Length(0)
        };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), bottom_constraint])
            .split(term_split[0]);

        let detail_rect = match slot {
            _ if !state.detail_visible => Rect::default(),
            PreviewSlot::Right => columns[2],
            PreviewSlot::Bottom => rows[1],
            PreviewSlot::Hidden => Rect::new(columns[2].x, columns[2].y, 0, columns[2].height),
        };
        let term_area = if term_height > 0 {
            term_split[1]
        } else {
            Rect::default()
        };

        Self {
            sidebar_rect: columns[0],
            content_area: rows[0],
            detail_rect,
            term_area,
            ..plan
        }
    }

    /// Whether the sidebar is visible and has positive width.
    pub fn has_sidebar(&self) -> bool {
        self.sidebar_rect.width > 0
    }

    /// Whether the detail pane is visible and has positive width.
    pub fn has_detail(&self) -> bool {
        self.detail_rect.width > 0 && self.detail_rect.height > 0
    }

    /// Whether the compact terminal pane is visible and has positive height.
    pub fn has_term(&self) -> bool {
        self.term_area.height > 0
    }

    /// Returns `Some(sidebar_rect)` if visible, or `None`.
    pub fn sidebar_rect_opt(&self) -> Option<Rect> {
        self.has_sidebar().then_some(self.sidebar_rect)
    }

    /// Returns `Some(detail_rect)` if visible, or `None`.
    pub fn detail_rect_opt(&self) -> Option<Rect> {
        self.has_detail().then_some(self.detail_rect)
    }

    /// Returns `Some(term_area)` if visible, or `None`.
    pub fn terminal_area_opt(&self) -> Option<Rect> {
        self.has_term().then_some(self.term_area)
    }
}

fn preview_slot(panes: PaneSettings, terminal_width: u16) -> PreviewSlot {
    let is_wide = terminal_width >= panes.preview_min_width;
    match panes.preview {
        PreviewPlacement::Right if is_wide => PreviewSlot::Right,
        PreviewPlacement::Auto if is_wide => PreviewSlot::Right,
        PreviewPlacement::Auto | PreviewPlacement::Bottom => PreviewSlot::Bottom,
        PreviewPlacement::Right | PreviewPlacement::Off => PreviewSlot::Hidden,
    }
}

fn preview_width_constraint(width: PaneSize, terminal_width: u16) -> Constraint {
    match width {
        PaneSize::Cells(cells) => Constraint::Length(cells),
        PaneSize::Percent(percent) => Constraint::Percentage(percent),
        PaneSize::Auto if terminal_width > AUTO_PREVIEW_LARGE_WIDTH_THRESHOLD => {
            Constraint::Percentage(AUTO_PREVIEW_LARGE_PERCENTAGE)
        }
        PaneSize::Auto if terminal_width > AUTO_PREVIEW_MEDIUM_WIDTH_THRESHOLD => {
            Constraint::Length(AUTO_PREVIEW_MEDIUM_WIDTH)
        }
        PaneSize::Auto => Constraint::Length(AUTO_PREVIEW_SMALL_WIDTH),
    }
}

fn preview_height_constraint(height: PaneSize) -> Constraint {
    match height {
        PaneSize::Cells(cells) => Constraint::Length(cells),
        PaneSize::Percent(percent) => Constraint::Percentage(percent),
        PaneSize::Auto => Constraint::Percentage(AUTO_PREVIEW_HEIGHT_PERCENTAGE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_inputs(width: u16, height: u16) -> LayoutInputs {
        LayoutInputs {
            size: Rect::new(0, 0, width, height),
            panes: PaneSettings::from(&LayoutConfig::default()),
            state: LayoutState {
                active_tab: Tab::Issues,
                detail_visible: true,
                details_zoomed: false,
                can_zoom: true,
                edit_zoomed: false,
            },
        }
    }

    fn with_preview(width: u16, preview: PreviewPlacement) -> LayoutInputs {
        let mut inputs = base_inputs(width, 30);
        inputs.panes.preview = preview;
        inputs
    }

    #[test]
    fn terminal_too_small_guard() {
        let plan_narrow = LayoutPlan::compute(base_inputs(53, 24));
        assert!(plan_narrow.is_too_small);

        let plan_short = LayoutPlan::compute(base_inputs(80, 9));
        assert!(plan_short.is_too_small);

        let plan_exact = LayoutPlan::compute(base_inputs(54, 10));
        assert!(!plan_exact.is_too_small);
    }

    #[test]
    fn header_and_banner_geometry() {
        let plan = LayoutPlan::compute(base_inputs(100, 30));
        assert_eq!(plan.header_area, Rect::new(0, 0, 100, 2));
        assert_eq!(plan.banner_mode, Rect::new(88, 0, 12, 2));
        assert_eq!(plan.banner_title, Rect::new(0, 0, 88, 2));
        assert_eq!(plan.workspace_area, Rect::new(0, 2, 100, 28));
    }

    #[test]
    fn sidebar_visibility_and_breakpoints() {
        let plan_79 = LayoutPlan::compute(base_inputs(79, 30));
        assert!(!plan_79.has_sidebar());
        assert_eq!(plan_79.sidebar_rect_opt(), None);

        let plan_80 = LayoutPlan::compute(base_inputs(80, 30));
        assert_eq!(plan_80.sidebar_rect_opt(), Some(Rect::new(0, 2, 22, 28)));

        let mut inputs_hidden = base_inputs(120, 30);
        inputs_hidden.panes.nav_visible = false;
        assert!(!LayoutPlan::compute(inputs_hidden).has_sidebar());
    }

    #[test]
    fn configured_nav_width_and_threshold_replace_the_defaults() {
        let mut inputs = base_inputs(70, 30);
        inputs.panes.nav_width = 30;
        inputs.panes.nav_min_width = 60;

        assert_eq!(
            LayoutPlan::compute(inputs).sidebar_rect_opt(),
            Some(Rect::new(0, 2, 30, 28))
        );
    }

    #[test]
    fn detail_width_breakpoints() {
        let mut inputs = base_inputs(120, 30);
        inputs.state.detail_visible = false;
        let plan = LayoutPlan::compute(inputs);
        assert_eq!(plan.detail_rect_opt(), None);

        assert!(!LayoutPlan::compute(base_inputs(89, 30)).has_detail());
        assert_eq!(
            LayoutPlan::compute(base_inputs(95, 30)).detail_rect.width,
            30
        );
        assert_eq!(
            LayoutPlan::compute(base_inputs(120, 30)).detail_rect.width,
            45
        );
        assert_eq!(
            LayoutPlan::compute(base_inputs(200, 30)).detail_rect.width,
            70
        );
    }

    #[test]
    fn configured_preview_width_overrides_the_auto_breakpoints() {
        let mut cells = base_inputs(120, 30);
        cells.panes.preview_width = PaneSize::Cells(50);
        assert_eq!(LayoutPlan::compute(cells).detail_rect.width, 50);

        let mut percent = base_inputs(200, 30);
        percent.panes.preview_width = PaneSize::Percent(50);
        assert_eq!(LayoutPlan::compute(percent).detail_rect.width, 100);
    }

    #[test]
    fn bottom_preview_sits_between_the_list_and_the_terminal_pane() {
        let plan = LayoutPlan::compute(with_preview(120, PreviewPlacement::Bottom));

        assert_eq!(plan.content_area, Rect::new(22, 2, 98, 11));
        assert_eq!(plan.detail_rect, Rect::new(22, 13, 98, 11));
        assert_eq!(plan.term_area, Rect::new(22, 24, 98, 6));
    }

    #[test]
    fn auto_preview_moves_below_the_list_on_narrow_terminals() {
        let wide = LayoutPlan::compute(with_preview(120, PreviewPlacement::Auto));
        assert_eq!(wide.detail_rect, Rect::new(75, 2, 45, 28));

        let narrow = LayoutPlan::compute(with_preview(80, PreviewPlacement::Auto));
        assert_eq!(narrow.detail_rect, Rect::new(22, 13, 58, 11));
        assert_eq!(narrow.content_area.width, 58);
    }

    #[test]
    fn preview_off_keeps_the_list_full_width_but_zoom_still_fills_the_workspace() {
        let mut inputs = with_preview(120, PreviewPlacement::Off);
        let plan = LayoutPlan::compute(inputs);
        assert!(!plan.has_detail());
        assert_eq!(plan.content_area.width, 98);

        inputs.state.details_zoomed = true;
        assert_eq!(
            LayoutPlan::compute(inputs).detail_rect,
            Rect::new(0, 2, 120, 28)
        );
    }

    #[test]
    fn terminal_tab_omits_detail_and_compact_terminal_pane() {
        let mut inputs = base_inputs(120, 30);
        inputs.state.active_tab = Tab::Terminal;
        let plan = LayoutPlan::compute(inputs);
        assert!(!plan.has_detail());
        assert!(!plan.has_term());
        assert_eq!(plan.content_area.width, 120 - 22);
    }

    #[test]
    fn zoom_expands_detail_to_full_workspace() {
        let mut inputs = base_inputs(120, 30);
        inputs.state.details_zoomed = true;
        let plan = LayoutPlan::compute(inputs);

        assert!(!plan.has_sidebar());
        assert_eq!(plan.content_area.width, 0);
        assert_eq!(plan.detail_rect, Rect::new(0, 2, 120, 28));

        inputs.state.can_zoom = false;
        let plan_no_zoom = LayoutPlan::compute(inputs);
        assert!(plan_no_zoom.has_sidebar());
        assert!(plan_no_zoom.content_area.width > 0);
    }

    #[test]
    fn edit_zoomed_forces_detail_full_workspace() {
        let mut inputs = base_inputs(120, 30);
        inputs.state.edit_zoomed = true;
        let plan = LayoutPlan::compute(inputs);

        assert!(!plan.has_sidebar());
        assert_eq!(plan.content_area.width, 0);
        assert_eq!(plan.detail_rect, Rect::new(0, 2, 120, 28));
    }

    #[test]
    fn terminal_pane_splits_content_vertically() {
        assert!(!LayoutPlan::compute(base_inputs(100, 17)).has_term());

        let plan_18 = LayoutPlan::compute(base_inputs(100, 18));
        assert_eq!(plan_18.term_area.height, 6);
        assert_eq!(plan_18.content_area.height, 18 - 2 - 6);

        let mut inputs_no_term = base_inputs(100, 24);
        inputs_no_term.panes.terminal_pane_visible = false;
        let plan_no_term = LayoutPlan::compute(inputs_no_term);
        assert!(!plan_no_term.has_term());
        assert_eq!(plan_no_term.content_area.height, 24 - 2);
    }

    #[test]
    fn configured_terminal_pane_height_and_threshold_replace_the_defaults() {
        let mut inputs = base_inputs(100, 14);
        inputs.panes.terminal_pane_height = 4;
        inputs.panes.terminal_pane_min_height = 12;

        assert_eq!(LayoutPlan::compute(inputs).term_area.height, 4);
    }
}
