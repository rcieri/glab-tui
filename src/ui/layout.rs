use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::app::Tab;
use crate::config::UiConfig;
use crate::ui::compaction::{self, PreviewPlacement};

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

/// The panes of the main workspace below the header bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Workspace {
    pub sidebar: Rect,
    pub list: Rect,
    /// The compact terminal log under the list, or under the list and the stacked preview.
    pub terminal: Rect,
    pub detail: Rect,
}

impl Workspace {
    /// Lays out the sidebar (left out on a terminal narrower than
    /// `SIDEBAR_MIN_WIDTH_THRESHOLD`), the list with the terminal log under
    /// it, and, when `preview_columns` is set, the preview where
    /// `compaction::place_preview` puts it: beside the list at that width or
    /// narrowed, below the list (giving up the terminal log when both do not
    /// fit), or hidden when neither fits.
    pub fn split(
        area: Rect,
        sidebar_width: u16,
        terminal_height: u16,
        preview_columns: Option<u16>,
    ) -> Self {
        let shown_sidebar_width = if area.width >= SIDEBAR_MIN_WIDTH_THRESHOLD {
            sidebar_width
        } else {
            0
        };
        let [sidebar, main] =
            Layout::horizontal([Constraint::Length(shown_sidebar_width), Constraint::Min(0)])
                .areas(area);
        let placement = match preview_columns {
            Some(preferred_columns) => {
                // Measured as if the sidebar were shown, so leaving it out on a
                // narrow terminal never moves the preview back beside the list.
                compaction::place_preview(
                    Rect {
                        width: area.width.saturating_sub(sidebar_width),
                        ..main
                    },
                    preferred_columns,
                    terminal_height,
                )
            }
            None => PreviewPlacement::Hidden,
        };
        let split_terminal = |column: Rect, terminal_rows: u16| -> [Rect; 2] {
            Layout::vertical([Constraint::Min(0), Constraint::Length(terminal_rows)]).areas(column)
        };
        match placement {
            PreviewPlacement::Beside { columns } => {
                let [column, detail] =
                    Layout::horizontal([Constraint::Min(0), Constraint::Length(columns)])
                        .areas(main);
                let [list, terminal] = split_terminal(column, terminal_height);
                Self {
                    sidebar,
                    list,
                    terminal,
                    detail,
                }
            }
            PreviewPlacement::Below { terminal_log } => {
                let terminal_rows = if terminal_log { terminal_height } else { 0 };
                let [column, terminal] = split_terminal(main, terminal_rows);
                let [list, detail] =
                    Layout::vertical([Constraint::Fill(1), Constraint::Fill(1)]).areas(column);
                Self {
                    sidebar,
                    list,
                    terminal,
                    detail,
                }
            }
            PreviewPlacement::Hidden => {
                let [list, terminal] = split_terminal(main, terminal_height);
                Self {
                    sidebar,
                    list,
                    terminal,
                    detail: Rect::default(),
                }
            }
        }
    }
}

/// Columns the preview asks for beside the list in a terminal `width` wide,
/// before `compaction::place_preview` narrows it to keep the list readable.
pub fn preview_columns(width: u16) -> u16 {
    if width > DETAIL_LARGE_WIDTH_THRESHOLD {
        (u32::from(width) * u32::from(DETAIL_LARGE_PERCENTAGE) / 100) as u16
    } else if width > DETAIL_MEDIUM_WIDTH_THRESHOLD {
        DETAIL_MEDIUM_WIDTH
    } else {
        compaction::MIN_PREVIEW_COLUMNS
    }
}

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

        let sidebar_width = if inputs.sidebar_visible {
            inputs.sidebar_width
        } else {
            0
        };

        let term_height = if inputs.active_tab != Tab::Terminal
            && size.height >= TERMINAL_PANE_MIN_HEIGHT_THRESHOLD
            && inputs.terminal_pane_visible
        {
            TERMINAL_PANE_HEIGHT
        } else {
            0
        };

        let is_zoomed = (inputs.details_zoomed && inputs.can_zoom) || inputs.edit_zoomed;

        let workspace = if is_zoomed {
            Workspace {
                sidebar: Rect::default(),
                list: Rect::default(),
                terminal: Rect::default(),
                detail: workspace_area,
            }
        } else {
            let show_preview = inputs.detail_visible && inputs.active_tab != Tab::Terminal;
            let preview_cols = show_preview.then(|| preview_columns(workspace_area.width));
            Workspace::split(workspace_area, sidebar_width, term_height, preview_cols)
        };

        let sidebar_rect = workspace.sidebar;
        let content_area = workspace.list;
        let detail_rect = workspace.detail;
        let term_area = workspace.terminal;

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

        // Below 90 columns with sidebar: preview placed below list
        let plan_89 = LayoutPlan::compute(base_inputs(89, 30));
        assert!(plan_89.has_detail());
        assert_eq!(plan_89.detail_rect.y, plan_89.content_area.bottom());

        // 90..=100 columns: 30 columns
        let plan_95 = LayoutPlan::compute(base_inputs(95, 30));
        assert!(plan_95.has_detail());
        assert_eq!(plan_95.detail_rect.width, 30);

        // 101..=104 columns: narrowed to 30 columns to keep list readable
        let plan_101 = LayoutPlan::compute(base_inputs(101, 30));
        assert_eq!(plan_101.detail_rect.width, 30);

        // 105..=150 columns: 45 columns
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

        // Cannot zoom when can_zoom is false (e.g. Pipelines)
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
