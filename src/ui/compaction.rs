//! Ordered compaction: how a screen gives up layout when the terminal is too
//! small for its full form.
//!
//! A screen lists the steps it can take, most expendable first, and [`compact`]
//! applies them one at a time until the layout fits. Every size-dependent
//! layout decision goes through it, so screens share one algorithm instead of
//! each inventing its own thresholds.

use ratatui::layout::Rect;

/// Applies `rules` to `full` in order, stopping at the first layout that
/// `fits`. When none does, the layout after the last rule is returned: it is
/// the most compact one the screen has.
pub fn compact<L: Copy, R: Copy>(
    full: L,
    rules: &[R],
    fits: impl Fn(L) -> bool,
    apply: impl Fn(L, R) -> L,
) -> L {
    let mut layout = full;
    for &rule in rules {
        if fits(layout) {
            break;
        }
        layout = apply(layout, rule);
    }
    layout
}

/// Columns the list keeps beside the preview pane. With the default sidebar
/// and the 30-column preview of a narrow terminal, the preview stays beside the
/// list from 90 columns up, the width it always needed.
const MIN_LIST_COLUMNS: u16 = 38;

/// Rows each of the list and the preview keeps when they are stacked: borders,
/// a table header and a few rows.
const MIN_STACKED_PANE_ROWS: u16 = 6;

/// Where the detail/preview pane goes relative to the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewPlacement {
    Beside,
    Below,
    Hidden,
}

#[derive(Debug, Clone, Copy)]
enum WorkspaceRule {
    StackPreview,
    HidePreview,
}

const WORKSPACE_RULES: &[WorkspaceRule] =
    &[WorkspaceRule::StackPreview, WorkspaceRule::HidePreview];

/// Where the preview goes in `area`, the space shared by the list and the
/// preview, when the preview wants `preview_columns` beside the list.
pub fn place_preview(area: Rect, preview_columns: u16) -> PreviewPlacement {
    compact(
        PreviewPlacement::Beside,
        WORKSPACE_RULES,
        |placement| match placement {
            PreviewPlacement::Beside => {
                area.width >= preview_columns.saturating_add(MIN_LIST_COLUMNS)
            }
            PreviewPlacement::Below => area.height >= 2 * MIN_STACKED_PANE_ROWS,
            PreviewPlacement::Hidden => true,
        },
        |_, rule| match rule {
            WorkspaceRule::StackPreview => PreviewPlacement::Below,
            WorkspaceRule::HidePreview => PreviewPlacement::Hidden,
        },
    )
}

/// Columns each side of a side-by-side diff needs: the line-number gutter,
/// the diff marker and enough code to read without most lines being cut.
const MIN_SIDE_BY_SIDE_PANE_COLUMNS: u16 = 60;

#[derive(Debug, Clone, Copy)]
enum DiffRule {
    Unify,
}

const DIFF_RULES: &[DiffRule] = &[DiffRule::Unify];

/// Whether a diff pane `columns` wide (inside its borders) shows its rows side
/// by side rather than unified.
pub fn fits_side_by_side_diff(columns: u16) -> bool {
    compact(
        true,
        DIFF_RULES,
        |side_by_side| !side_by_side || columns >= 2 * MIN_SIDE_BY_SIDE_PANE_COLUMNS,
        |_, DiffRule::Unify| false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compaction_stops_at_the_first_rule_that_makes_the_layout_fit() {
        let applied = compact(
            0,
            &[1, 2, 3],
            |layout| layout >= 3,
            |layout, rule| layout + rule,
        );

        assert_eq!(applied, 3);
    }

    #[test]
    fn compaction_returns_the_most_compact_layout_when_nothing_fits() {
        let applied = compact(0, &[1, 2], |_| false, |layout, rule| layout + rule);

        assert_eq!(applied, 3);
    }

    #[test]
    fn a_layout_that_already_fits_applies_no_rule() {
        let applied = compact(0, &[1], |_| true, |layout, rule| layout + rule);

        assert_eq!(applied, 0);
    }

    #[test]
    fn preview_stays_beside_the_list_while_the_list_keeps_its_minimum_width() {
        assert_eq!(
            place_preview(Rect::new(0, 0, 30 + MIN_LIST_COLUMNS, 20), 30),
            PreviewPlacement::Beside
        );
    }

    #[test]
    fn preview_moves_below_the_list_one_column_short_of_the_minimum() {
        assert_eq!(
            place_preview(Rect::new(0, 0, 30 + MIN_LIST_COLUMNS - 1, 20), 30),
            PreviewPlacement::Below
        );
    }

    #[test]
    fn preview_is_hidden_when_stacking_leaves_too_few_rows() {
        assert_eq!(
            place_preview(Rect::new(0, 0, 50, 2 * MIN_STACKED_PANE_ROWS - 1), 30),
            PreviewPlacement::Hidden
        );
        assert_eq!(
            place_preview(Rect::new(0, 0, 50, 2 * MIN_STACKED_PANE_ROWS), 30),
            PreviewPlacement::Below
        );
    }

    #[test]
    fn diff_goes_unified_below_two_minimum_panes() {
        assert!(fits_side_by_side_diff(2 * MIN_SIDE_BY_SIDE_PANE_COLUMNS));
        assert!(!fits_side_by_side_diff(
            2 * MIN_SIDE_BY_SIDE_PANE_COLUMNS - 1
        ));
    }
}
