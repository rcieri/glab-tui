use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::app::Tab;
use crate::config::{LayoutConfig, NavPosition, PaneSize, PreviewPlacement};
use crate::ui::compaction::{self, PreviewPlacement as Placement};

pub const MIN_TERMINAL_WIDTH: u16 = 54;
pub const MIN_TERMINAL_HEIGHT: u16 = 10;
pub const HEADER_HEIGHT: u16 = 2;
pub const BANNER_MODE_WIDTH: u16 = 12;
/// `preview_width = "auto"`: wider terminals than this give the preview
/// `AUTO_PREVIEW_LARGE_PERCENTAGE` of the workspace.
pub const AUTO_PREVIEW_LARGE_WIDTH_THRESHOLD: u16 = 150;
/// `preview_width = "auto"`: wider terminals than this (up to the large
/// threshold) give the preview `AUTO_PREVIEW_MEDIUM_WIDTH` columns, narrower
/// ones `compaction::MIN_PREVIEW_COLUMNS`.
pub const AUTO_PREVIEW_MEDIUM_WIDTH_THRESHOLD: u16 = 100;
pub const AUTO_PREVIEW_LARGE_PERCENTAGE: u16 = 35;
pub const AUTO_PREVIEW_MEDIUM_WIDTH: u16 = 45;
/// `preview_height = "auto"`: a bottom preview takes this share of the list
/// column.
pub const AUTO_PREVIEW_HEIGHT_PERCENTAGE: u16 = 50;
/// When the sidebar and preview are both open, the list shrinks to this floor
/// before the sidebar is hidden.
pub const MIN_SQUEEZED_LIST_WIDTH: u16 = 30;

/// The pane settings of `[layout]`, everything but the tab list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneSettings {
    pub nav_position: NavPosition,
    pub nav_visible: bool,
    pub nav_width: u16,
    pub nav_min_width: u16,
    pub tab_switch_in_zoom: bool,
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
            nav_position: config.nav_position,
            nav_visible: config.nav_visible,
            nav_width: config.nav_width,
            nav_min_width: config.nav_min_width,
            tab_switch_in_zoom: config.tab_switch_in_zoom,
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
    pub edit_zoomed: bool,
}

/// The computed geometry for all top-level UI regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutPlan {
    /// Whether the terminal is below [`MIN_TERMINAL_WIDTH`] x [`MIN_TERMINAL_HEIGHT`].
    pub is_too_small: bool,
    /// Top header bar area ([`HEADER_HEIGHT`] rows).
    pub header_area: Rect,
    /// Left portion of the header bar (title, scope, search/filter, header notices).
    pub banner_title: Rect,
    /// Right portion of the header bar (mode indicator, [`BANNER_MODE_WIDTH`] columns).
    pub banner_mode: Rect,
    /// Top horizontal tab navigation strip area (height 1 across workspace top when nav_position is Top and visible).
    pub top_nav_area: Rect,
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
        let raw_workspace_area = chunks[1];

        let header_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(header_area);

        let banner = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(BANNER_MODE_WIDTH)])
            .split(header_rows[0]);

        let state = inputs.state;
        let panes = inputs.panes;
        let is_top_nav = panes.nav_position == NavPosition::Top && panes.nav_visible;

        let show_top_nav =
            is_top_nav && !state.edit_zoomed && (!state.details_zoomed || panes.tab_switch_in_zoom);

        let (top_nav_area, workspace_area) = if show_top_nav {
            let v = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0)])
                .split(raw_workspace_area);
            (v[0], v[1])
        } else {
            (Rect::default(), raw_workspace_area)
        };

        let plan = Self {
            is_too_small: false,
            header_area,
            banner_title: banner[0],
            banner_mode: banner[1],
            top_nav_area,
            workspace_area,
            ..Default::default()
        };

        let is_zoomed =
            (state.details_zoomed && state.active_tab.can_zoom_preview()) || state.edit_zoomed;
        if is_zoomed {
            return plan.zoomed();
        }

        let nav_width = if !is_top_nav && panes.nav_visible && size.width >= panes.nav_min_width {
            panes.nav_width
        } else {
            0
        };
        let term_height = if state.active_tab != Tab::Terminal
            && panes.terminal_pane_visible
            && size.height >= panes.terminal_pane_min_height
        {
            panes.terminal_pane_height
        } else {
            0
        };
        let wants_preview = state.detail_visible && state.active_tab != Tab::Terminal;
        let placement = if wants_preview {
            place_preview(panes, workspace_area, term_height)
        } else {
            Placement::Hidden
        };
        // A tab that cannot zoom has no other way to show its preview, so one
        // the placement leaves out fills the workspace as a zoom would.
        if wants_preview && !state.active_tab.can_zoom_preview() && placement == Placement::Hidden {
            return plan.zoomed();
        }

        let beside_columns = match placement {
            Placement::Beside { columns } => columns,
            Placement::Below { .. } | Placement::Hidden => 0,
        };
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(nav_width),
                Constraint::Min(MIN_SQUEEZED_LIST_WIDTH),
                Constraint::Length(beside_columns),
            ])
            .split(workspace_area);

        let terminal_rows = match placement {
            Placement::Below {
                terminal_log: false,
            } => 0,
            _ => term_height,
        };
        let term_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(compaction::MIN_STACKED_PANE_ROWS),
                Constraint::Length(terminal_rows),
            ])
            .split(columns[1]);
        let bottom_constraint = if matches!(placement, Placement::Below { .. }) {
            preview_height_constraint(panes.preview_height)
        } else {
            Constraint::Length(0)
        };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(compaction::MIN_STACKED_PANE_ROWS),
                bottom_constraint,
            ])
            .split(term_split[0]);

        let detail_rect = match placement {
            Placement::Beside { .. } => columns[2],
            Placement::Below { .. } => rows[1],
            Placement::Hidden => Rect::default(),
        };
        let term_area = if terminal_rows > 0 {
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

    /// This plan with the detail pane filling the workspace, the list and
    /// the sidebar left zero wide.
    fn zoomed(self) -> Self {
        let workspace = self.workspace_area;
        let collapsed = Rect::new(workspace.x, workspace.y, 0, workspace.height);
        Self {
            sidebar_rect: collapsed,
            content_area: collapsed,
            detail_rect: workspace,
            ..self
        }
    }

    pub fn has_top_nav(&self) -> bool {
        self.top_nav_area.height > 0 && self.top_nav_area.width > 0
    }

    pub fn top_nav_area_opt(&self) -> Option<Rect> {
        self.has_top_nav().then_some(self.top_nav_area)
    }

    pub fn has_sidebar(&self) -> bool {
        self.sidebar_rect.width > 0
    }

    pub fn has_detail(&self) -> bool {
        self.detail_rect.width > 0 && self.detail_rect.height > 0
    }

    pub fn has_term(&self) -> bool {
        self.term_area.height > 0
    }

    pub fn sidebar_rect_opt(&self) -> Option<Rect> {
        self.has_sidebar().then_some(self.sidebar_rect)
    }

    pub fn detail_rect_opt(&self) -> Option<Rect> {
        self.has_detail().then_some(self.detail_rect)
    }

    pub fn terminal_area_opt(&self) -> Option<Rect> {
        self.has_term().then_some(self.term_area)
    }
}

/// Where the preview goes in `workspace` for the configured placement. Both
/// `preview_min_width` and the panes' fit are measured on the room the
/// configured sidebar leaves: toggling the sidebar off frees its columns,
/// while the sidebar leaving on its own under `nav_min_width` never moves
/// the preview back beside the list.
fn place_preview(panes: PaneSettings, workspace: Rect, terminal_rows: u16) -> Placement {
    let nav_width = if panes.nav_position == NavPosition::Left && panes.nav_visible {
        panes.nav_width
    } else {
        0
    };
    let room = Rect {
        width: workspace.width.saturating_sub(nav_width),
        ..workspace
    };
    let is_wide = room.width >= panes.preview_min_width;
    let columns = preview_columns(panes.preview_width, workspace.width, room.width);
    match panes.preview {
        PreviewPlacement::Right if is_wide => {
            compaction::place_preview_beside(room, columns, terminal_rows)
        }
        PreviewPlacement::Auto if is_wide => {
            compaction::place_preview(room, columns, terminal_rows)
        }
        PreviewPlacement::Auto | PreviewPlacement::Bottom => {
            compaction::place_preview_below(room, terminal_rows)
        }
        PreviewPlacement::Right | PreviewPlacement::Off => Placement::Hidden,
    }
}

/// Columns the preview asks for beside the list in a workspace `width` wide,
/// before compaction narrows it to keep the list readable. A configured
/// width is capped at what fits beside the list in `room_width`, so it
/// stays as wide as fits instead of dropping to [`compaction::MIN_PREVIEW_COLUMNS`].
fn preview_columns(size: PaneSize, width: u16, room_width: u16) -> u16 {
    let share = |percent: u16| (u32::from(width) * u32::from(percent) / 100) as u16;
    let fits_beside = room_width
        .saturating_sub(compaction::MIN_LIST_COLUMNS)
        .max(compaction::MIN_PREVIEW_COLUMNS);
    match size {
        PaneSize::Cells(cells) => cells.min(fits_beside),
        PaneSize::Percent(percent) => share(percent).min(fits_beside),
        PaneSize::Auto if width > AUTO_PREVIEW_LARGE_WIDTH_THRESHOLD => {
            share(AUTO_PREVIEW_LARGE_PERCENTAGE)
        }
        PaneSize::Auto if width > AUTO_PREVIEW_MEDIUM_WIDTH_THRESHOLD => AUTO_PREVIEW_MEDIUM_WIDTH,
        PaneSize::Auto => compaction::MIN_PREVIEW_COLUMNS,
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
        assert_eq!(plan.banner_mode, Rect::new(88, 0, 12, 1));
        assert_eq!(plan.banner_title, Rect::new(0, 0, 88, 1));
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
    fn top_nav_geometry() {
        let mut inputs = base_inputs(120, 30);
        inputs.panes.nav_position = NavPosition::Top;
        let plan = LayoutPlan::compute(inputs);

        assert!(plan.has_top_nav());
        assert_eq!(plan.top_nav_area, Rect::new(0, 2, 120, 1));
        assert!(!plan.has_sidebar());
        assert_eq!(plan.sidebar_rect.width, 0);
        // Table content starts below top nav (y = 3)
        assert_eq!(plan.content_area.y, 3);
        assert_eq!(plan.content_area.x, 0);

        // When nav is hidden, top nav is not computed
        inputs.panes.nav_visible = false;
        let plan_hidden = LayoutPlan::compute(inputs);
        assert!(!plan_hidden.has_top_nav());
        assert_eq!(plan_hidden.top_nav_area, Rect::default());
        assert_eq!(plan_hidden.content_area.y, 2);
    }

    #[test]
    fn top_nav_in_zoomed_preview() {
        let mut inputs = base_inputs(120, 30);
        inputs.panes.nav_position = NavPosition::Top;
        inputs.state.details_zoomed = true;

        let plan = LayoutPlan::compute(inputs);
        assert!(plan.has_top_nav());
        assert_eq!(plan.top_nav_area, Rect::new(0, 2, 120, 1));
        assert_eq!(plan.detail_rect, Rect::new(0, 3, 120, 27));

        // When tab switching in zoom is disabled, top nav is hidden during zoom
        inputs.panes.tab_switch_in_zoom = false;
        let plan_no_switch = LayoutPlan::compute(inputs);
        assert!(!plan_no_switch.has_top_nav());
        assert_eq!(plan_no_switch.top_nav_area, Rect::default());
        assert_eq!(plan_no_switch.detail_rect, Rect::new(0, 2, 120, 28));

        // When edit is zoomed, top nav is always hidden
        inputs.panes.tab_switch_in_zoom = true;
        inputs.state.edit_zoomed = true;
        let plan_edit = LayoutPlan::compute(inputs);
        assert!(!plan_edit.has_top_nav());
        assert_eq!(plan_edit.top_nav_area, Rect::default());
        assert_eq!(plan_edit.detail_rect, Rect::new(0, 2, 120, 28));
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

        let plan_89 = LayoutPlan::compute(base_inputs(89, 30));
        assert!(plan_89.has_detail());
        assert_eq!(plan_89.detail_rect.y, plan_89.content_area.bottom());

        assert_eq!(
            LayoutPlan::compute(base_inputs(95, 30)).detail_rect.width,
            30
        );
        assert_eq!(
            LayoutPlan::compute(base_inputs(101, 30)).detail_rect.width,
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
    fn a_configured_preview_width_that_does_not_fit_shrinks_to_what_fits() {
        let mut percent = base_inputs(140, 30);
        percent.panes.preview_width = PaneSize::Percent(60);
        assert_eq!(LayoutPlan::compute(percent).detail_rect.width, 80);

        let mut cells = base_inputs(100, 30);
        cells.panes.preview_width = PaneSize::Cells(60);
        assert_eq!(LayoutPlan::compute(cells).detail_rect.width, 40);
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
    fn preview_min_width_is_measured_beside_the_sidebar() {
        let mut no_sidebar = base_inputs(85, 30);
        no_sidebar.panes.nav_visible = false;
        assert_eq!(
            LayoutPlan::compute(no_sidebar).detail_rect,
            Rect::new(55, 2, 30, 28)
        );

        let mut raised = base_inputs(110, 30);
        raised.panes.preview_min_width = 90;
        let plan = LayoutPlan::compute(raised);
        assert_eq!(plan.detail_rect.y, plan.content_area.bottom());
    }

    #[test]
    fn a_preview_that_cannot_zoom_fills_the_workspace_when_it_has_no_place() {
        let mut off = with_preview(120, PreviewPlacement::Off);
        off.state.active_tab = Tab::Pipelines;
        assert_eq!(
            LayoutPlan::compute(off).detail_rect,
            Rect::new(0, 2, 120, 28)
        );

        let mut narrow_right = with_preview(80, PreviewPlacement::Right);
        narrow_right.state.active_tab = Tab::Pipelines;
        let plan = LayoutPlan::compute(narrow_right);
        assert_eq!(plan.detail_rect, Rect::new(0, 2, 80, 28));
        assert_eq!(plan.content_area.width, 0);

        let mut closed = with_preview(120, PreviewPlacement::Off);
        closed.state.active_tab = Tab::Pipelines;
        closed.state.detail_visible = false;
        assert!(!LayoutPlan::compute(closed).has_detail());
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

        inputs.state.active_tab = Tab::Pipelines;
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

    #[test]
    fn oversized_panes_shrink_before_the_list_does() {
        let mut wide = base_inputs(90, 30);
        wide.panes.nav_width = 40;
        wide.panes.preview = PreviewPlacement::Off;
        wide.size.width = 60;
        wide.panes.nav_min_width = 60;
        assert_eq!(
            LayoutPlan::compute(wide).content_area.width,
            MIN_SQUEEZED_LIST_WIDTH
        );

        let mut tall = base_inputs(100, 20);
        tall.panes.terminal_pane_height = 20;
        tall.panes.preview = PreviewPlacement::Off;
        assert_eq!(
            LayoutPlan::compute(tall).content_area.height,
            compaction::MIN_STACKED_PANE_ROWS
        );
    }

    #[test]
    fn default_preview_min_width_matches_pane_minimums() {
        assert_eq!(
            crate::config::layout::DEFAULT_PREVIEW_MIN_WIDTH,
            compaction::MIN_PREVIEW_COLUMNS + compaction::MIN_LIST_COLUMNS
        );
    }
}
