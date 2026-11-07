use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::app::Tab;
use crate::config::UiConfig;

pub const MIN_TERMINAL_WIDTH: u16 = 54;
pub const MIN_TERMINAL_HEIGHT: u16 = 10;
pub const HEADER_HEIGHT: u16 = 2;
pub const BANNER_MODE_WIDTH: u16 = 12;
pub const SIDEBAR_MIN_WIDTH_THRESHOLD: u16 = 80;
pub const DETAIL_MIN_WIDTH_THRESHOLD: u16 = 90;
pub const DETAIL_LARGE_WIDTH_THRESHOLD: u16 = 150;
pub const DETAIL_MEDIUM_WIDTH_THRESHOLD: u16 = 100;
pub const DETAIL_LARGE_PERCENTAGE: u16 = 35;
pub const DETAIL_MEDIUM_WIDTH: u16 = 45;
pub const DETAIL_SMALL_WIDTH: u16 = 30;
pub const TERMINAL_PANE_MIN_HEIGHT_THRESHOLD: u16 = 18;
pub const TERMINAL_PANE_HEIGHT: u16 = 6;

/// Inputs required to compute a [`LayoutPlan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutInputs {
    pub size: Rect,
    pub active_tab: Tab,
    pub sidebar_visible: bool,
    pub sidebar_width: u16,
    pub detail_visible: bool,
    pub details_zoomed: bool,
    pub can_zoom: bool,
    pub edit_zoomed: bool,
    pub terminal_pane_visible: bool,
}

impl LayoutInputs {
    pub fn from_config_and_state(size: Rect, config: &UiConfig, state: &LayoutState) -> Self {
        Self {
            size,
            active_tab: state.active_tab,
            sidebar_visible: config.sidebar_visible,
            sidebar_width: config.sidebar_width,
            detail_visible: state.detail_visible,
            details_zoomed: state.details_zoomed,
            can_zoom: state.can_zoom,
            edit_zoomed: state.edit_zoomed,
            terminal_pane_visible: config.terminal_pane_visible,
        }
    }
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
    /// Detail/preview rectangle (width 0 if hidden or below width threshold).
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
            .constraints([
                Constraint::Length(HEADER_HEIGHT), // Top header bar
                Constraint::Min(0),                // Main workspace
                Constraint::Length(0),             // Reserved
            ])
            .split(size);

        let header_area = chunks[0];
        let workspace_area = chunks[1];

        let banner = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(BANNER_MODE_WIDTH)])
            .split(header_area);
        let banner_title = banner[0];
        let banner_mode = banner[1];

        let sidebar_constraint =
            if size.width >= SIDEBAR_MIN_WIDTH_THRESHOLD && inputs.sidebar_visible {
                Constraint::Length(inputs.sidebar_width)
            } else {
                Constraint::Length(0)
            };

        let details_constraint =
            if !inputs.detail_visible || size.width < DETAIL_MIN_WIDTH_THRESHOLD {
                Constraint::Max(0)
            } else if size.width > DETAIL_LARGE_WIDTH_THRESHOLD {
                Constraint::Percentage(DETAIL_LARGE_PERCENTAGE)
            } else if size.width > DETAIL_MEDIUM_WIDTH_THRESHOLD {
                Constraint::Length(DETAIL_MEDIUM_WIDTH)
            } else {
                Constraint::Length(DETAIL_SMALL_WIDTH)
            };

        let is_zoomed = (inputs.details_zoomed && inputs.can_zoom) || inputs.edit_zoomed;

        let middle_chunks = if is_zoomed {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(0),
                    Constraint::Length(0),
                    Constraint::Min(0),
                ])
                .split(workspace_area)
        } else if inputs.active_tab == Tab::Terminal {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    sidebar_constraint,
                    Constraint::Min(0),
                    Constraint::Length(0),
                ])
                .split(workspace_area)
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([sidebar_constraint, Constraint::Min(0), details_constraint])
                .split(workspace_area)
        };

        let sidebar_rect = middle_chunks[0];
        let content_rect_raw = middle_chunks[1];
        let mut detail_rect = middle_chunks[2];

        if !inputs.detail_visible && !inputs.details_zoomed && !inputs.edit_zoomed {
            detail_rect = Rect::default();
        }

        let term_height = if inputs.active_tab != Tab::Terminal
            && size.height >= TERMINAL_PANE_MIN_HEIGHT_THRESHOLD
            && inputs.terminal_pane_visible
        {
            TERMINAL_PANE_HEIGHT
        } else {
            0
        };

        let (content_area, term_area) = if term_height > 0 {
            let tc = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(term_height)])
                .split(content_rect_raw);
            (tc[0], tc[1])
        } else {
            (content_rect_raw, Rect::default())
        };

        Self {
            is_too_small: false,
            header_area,
            banner_title,
            banner_mode,
            workspace_area,
            sidebar_rect,
            content_area,
            detail_rect,
            term_area,
        }
    }

    /// Whether the sidebar is visible and has positive width.
    pub fn has_sidebar(&self) -> bool {
        self.sidebar_rect.width > 0
    }

    /// Whether the detail pane is visible and has positive width.
    pub fn has_detail(&self) -> bool {
        self.detail_rect.width > 0
    }

    /// Whether the compact terminal pane is visible and has positive height.
    pub fn has_term(&self) -> bool {
        self.term_area.height > 0
    }

    /// Returns `Some(sidebar_rect)` if visible, or `None`.
    pub fn sidebar_rect_opt(&self) -> Option<Rect> {
        if self.has_sidebar() {
            Some(self.sidebar_rect)
        } else {
            None
        }
    }

    /// Returns `Some(detail_rect)` if visible, or `None`.
    pub fn detail_rect_opt(&self) -> Option<Rect> {
        if self.has_detail() {
            Some(self.detail_rect)
        } else {
            None
        }
    }

    /// Returns `Some(term_area)` if visible, or `None`.
    pub fn terminal_area_opt(&self) -> Option<Rect> {
        if self.has_term() {
            Some(self.term_area)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_inputs(width: u16, height: u16) -> LayoutInputs {
        LayoutInputs {
            size: Rect::new(0, 0, width, height),
            active_tab: Tab::Issues,
            sidebar_visible: true,
            sidebar_width: 22,
            detail_visible: true,
            details_zoomed: false,
            can_zoom: true,
            edit_zoomed: false,
            terminal_pane_visible: true,
        }
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
        // Below 80 columns: sidebar width is 0
        let plan_79 = LayoutPlan::compute(base_inputs(79, 30));
        assert!(!plan_79.has_sidebar());
        assert_eq!(plan_79.sidebar_rect_opt(), None);

        // At 80 columns: sidebar is shown with configured width
        let plan_80 = LayoutPlan::compute(base_inputs(80, 30));
        assert!(plan_80.has_sidebar());
        assert_eq!(plan_80.sidebar_rect, Rect::new(0, 2, 22, 28));
        assert_eq!(plan_80.sidebar_rect_opt(), Some(Rect::new(0, 2, 22, 28)));

        // Config says sidebar_visible: false -> width is 0 even at 120 columns
        let mut inputs_hidden = base_inputs(120, 30);
        inputs_hidden.sidebar_visible = false;
        let plan_hidden = LayoutPlan::compute(inputs_hidden);
        assert!(!plan_hidden.has_sidebar());
    }

    #[test]
    fn detail_width_breakpoints() {
        // Detail hidden if detail_visible: false
        let mut inputs = base_inputs(120, 30);
        inputs.detail_visible = false;
        let plan = LayoutPlan::compute(inputs);
        assert!(!plan.has_detail());
        assert_eq!(plan.detail_rect_opt(), None);

        // Below 90 columns: detail hidden
        let plan_89 = LayoutPlan::compute(base_inputs(89, 30));
        assert!(!plan_89.has_detail());

        // 90..=100 columns: 30 columns
        let plan_95 = LayoutPlan::compute(base_inputs(95, 30));
        assert!(plan_95.has_detail());
        assert_eq!(plan_95.detail_rect.width, 30);

        // 101..=150 columns: 45 columns
        let plan_120 = LayoutPlan::compute(base_inputs(120, 30));
        assert_eq!(plan_120.detail_rect.width, 45);

        // > 150 columns: 35% of workspace width (200 * 35% = 70)
        let plan_200 = LayoutPlan::compute(base_inputs(200, 30));
        assert_eq!(plan_200.detail_rect.width, 70);
    }

    #[test]
    fn terminal_tab_omits_detail_and_compact_terminal_pane() {
        let mut inputs = base_inputs(120, 30);
        inputs.active_tab = Tab::Terminal;
        let plan = LayoutPlan::compute(inputs);
        assert!(!plan.has_detail());
        assert!(!plan.has_term());
        assert_eq!(plan.content_area.width, 120 - 22);
    }

    #[test]
    fn zoom_expands_detail_to_full_workspace() {
        let mut inputs = base_inputs(120, 30);
        inputs.details_zoomed = true;
        inputs.can_zoom = true;
        let plan = LayoutPlan::compute(inputs);

        assert!(!plan.has_sidebar());
        assert_eq!(plan.content_area.width, 0);
        assert_eq!(plan.detail_rect, Rect::new(0, 2, 120, 28));

        // Cannot zoom when can_zoom is false (e.g. Pipelines with no jobs)
        inputs.can_zoom = false;
        let plan_no_zoom = LayoutPlan::compute(inputs);
        assert!(plan_no_zoom.has_sidebar());
        assert!(plan_no_zoom.content_area.width > 0);
    }

    #[test]
    fn edit_zoomed_forces_detail_full_workspace() {
        let mut inputs = base_inputs(120, 30);
        inputs.edit_zoomed = true;
        let plan = LayoutPlan::compute(inputs);

        assert!(!plan.has_sidebar());
        assert_eq!(plan.content_area.width, 0);
        assert_eq!(plan.detail_rect, Rect::new(0, 2, 120, 28));
    }

    #[test]
    fn terminal_pane_splits_content_vertically() {
        // Below 18 height: terminal pane hidden
        let plan_17 = LayoutPlan::compute(base_inputs(100, 17));
        assert!(!plan_17.has_term());

        // At 18 height with terminal_pane_visible: true
        let plan_18 = LayoutPlan::compute(base_inputs(100, 18));
        assert!(plan_18.has_term());
        assert_eq!(plan_18.term_area.height, 6);
        assert_eq!(plan_18.content_area.height, 18 - 2 - 6);

        // terminal_pane_visible: false -> hidden
        let mut inputs_no_term = base_inputs(100, 24);
        inputs_no_term.terminal_pane_visible = false;
        let plan_no_term = LayoutPlan::compute(inputs_no_term);
        assert!(!plan_no_term.has_term());
        assert_eq!(plan_no_term.content_area.height, 24 - 2);
    }
}
