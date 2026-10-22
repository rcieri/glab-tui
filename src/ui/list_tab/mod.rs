//! A list tab whose columns are typed and whose selection follows the row,
//! not its position.
//!
//! `ListTab` owns the rows, the filtered and sorted view over them, and a
//! cursor holding the selected row's natural key. The view is rebuilt only
//! when the rows or the view inputs (search query, visible columns, column
//! filters, sort) change, so renderers and handlers read it instead of
//! re-running the pipeline.

pub mod branches;
pub mod environments;
pub mod releases;
pub mod runners;

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::Hash;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Row as TableRow, Table, TableState};

use crate::config::{Icons, Theme};
use crate::ui::helpers::sort_header_label;

/// A row with a natural key that survives refreshes, sorts and filters.
pub trait Row {
    type Key: Eq + Hash + Clone + std::fmt::Debug;

    fn key(&self) -> Self::Key;

    /// Override when `key()` allocates, so lookups stay allocation-free.
    fn has_key(&self, key: &Self::Key) -> bool {
        self.key() == *key
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SortKey {
    Number(u64),
    Text(String),
}

/// What a column cell needs from the frame being drawn.
pub struct CellContext<'a> {
    pub theme: &'a Theme,
    pub icons: &'a Icons,
    pub query: &'a str,
    pub is_highlighted: bool,
    /// The style of the whole row: the highlight style on the cursor row.
    pub row_style: Style,
}

/// Values a column filter offers and matches for a cell showing `text`; an
/// empty cell offers none.
fn cell_filter_values(text: Cow<'_, str>) -> Vec<String> {
    if text.is_empty() {
        Vec::new()
    } else {
        vec![text.into_owned()]
    }
}

/// One column of a list tab: its header, cell, search text, filter values
/// and sort key, defined once.
pub trait ColumnDef: Copy + Eq + 'static {
    type Row: Row;

    /// Every column the tab can show, in display order.
    const ALL: &'static [Self];

    /// The header text, which is also the column's name in config files,
    /// the column picker and saved column filters.
    fn label(self) -> &'static str;

    fn is_default(self) -> bool;

    fn width(self) -> Constraint;

    fn header_alignment(self) -> Alignment {
        Alignment::Left
    }

    /// The value the search query, the column filter and the sort read.
    fn text(self, row: &Self::Row) -> Cow<'_, str>;

    fn cell(self, row: &Self::Row, ctx: &CellContext) -> Cell<'static>;

    fn sort_key(self, row: &Self::Row) -> SortKey {
        SortKey::Text(self.text(row).into_owned())
    }

    fn searchable(self) -> bool {
        true
    }

    /// Values the column filter offers and matches; an empty cell offers none.
    fn filter_values(self, row: &Self::Row) -> Vec<String> {
        cell_filter_values(self.text(row))
    }

    fn matches_filter(self, row: &Self::Row, selected: &HashSet<String>) -> bool {
        crate::app::column_filter_matches(self.label(), &self.filter_values(row), selected)
    }

    fn from_label(label: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.label() == label)
    }
}

/// The inputs a view is computed from, borrowed from the app state.
#[derive(Clone, Copy)]
pub struct ViewInputs<'a> {
    pub query: &'a str,
    /// `None` shows every column.
    pub enabled_columns: Option<&'a HashSet<String>>,
    pub column_filters: Option<&'a HashMap<String, HashSet<String>>>,
    /// Column label to sort by, and whether ascending.
    pub sort: Option<(&'a str, bool)>,
}

/// An owned copy of the `ViewInputs` the current view was built from.
#[derive(Debug, Default)]
struct ViewSpec {
    query: String,
    enabled_columns: Option<HashSet<String>>,
    column_filters: HashMap<String, HashSet<String>>,
    sort: Option<(String, bool)>,
}

impl ViewSpec {
    fn from_inputs(inputs: ViewInputs) -> Self {
        Self {
            query: inputs.query.to_string(),
            enabled_columns: inputs.enabled_columns.cloned(),
            column_filters: inputs.column_filters.cloned().unwrap_or_default(),
            sort: inputs.sort.map(|(col, asc)| (col.to_string(), asc)),
        }
    }

    fn matches(&self, inputs: ViewInputs) -> bool {
        self.query == inputs.query
            && self.enabled_columns.as_ref() == inputs.enabled_columns
            && match inputs.column_filters {
                Some(filters) => self.column_filters == *filters,
                None => self.column_filters.is_empty(),
            }
            && self.sort.as_ref().map(|(col, asc)| (col.as_str(), *asc)) == inputs.sort
    }

    fn is_column_visible(&self, label: &str) -> bool {
        self.enabled_columns
            .as_ref()
            .is_none_or(|set| set.contains(label))
    }
}

pub struct ListTab<C: ColumnDef> {
    items: Vec<C::Row>,
    /// Indices into `items`, filtered and sorted.
    view: Vec<usize>,
    cursor: Option<<C::Row as Row>::Key>,
    /// The cursor's position in `view`, kept so a cursor whose row leaves
    /// the view lands on the row now at the same position.
    cursor_pos: Option<usize>,
    /// `None` until built, and again whenever `items` changes or the cursor
    /// is reset.
    spec: Option<ViewSpec>,
    table_state: TableState,
}

impl<C: ColumnDef> Default for ListTab<C> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            view: Vec::new(),
            cursor: None,
            cursor_pos: None,
            spec: None,
            table_state: TableState::default(),
        }
    }
}

impl<C: ColumnDef> ListTab<C> {
    pub fn items(&self) -> &[C::Row] {
        &self.items
    }

    pub fn set_items(&mut self, items: Vec<C::Row>) {
        self.items = items;
        self.view.clear();
        self.spec = None;
    }

    /// Apply `update` to the row with `key`; false when no row has it.
    pub fn update(&mut self, key: &<C::Row as Row>::Key, update: impl FnOnce(&mut C::Row)) -> bool {
        let Some(row) = self.items.iter_mut().find(|r| r.has_key(key)) else {
            return false;
        };
        update(row);
        self.spec = None;
        true
    }

    pub fn retain(&mut self, keep: impl FnMut(&C::Row) -> bool) {
        self.items.retain(keep);
        self.view.clear();
        self.spec = None;
    }

    pub fn get(&self, key: &<C::Row as Row>::Key) -> Option<&C::Row> {
        self.items.iter().find(|r| r.has_key(key))
    }

    /// Rebuild the view if the rows or `inputs` changed since the last build.
    pub fn refresh(&mut self, inputs: ViewInputs) {
        if self.spec.as_ref().is_some_and(|spec| spec.matches(inputs)) {
            return;
        }
        let spec = ViewSpec::from_inputs(inputs);
        self.view = build_view::<C>(&self.items, &spec);
        self.spec = Some(spec);
        self.settle_cursor();
    }

    fn settle_cursor(&mut self) {
        let found = self.cursor.as_ref().and_then(|key| {
            self.view
                .iter()
                .position(|&idx| self.items[idx].has_key(key))
        });
        match found {
            Some(pos) => self.cursor_pos = Some(pos),
            None if self.view.is_empty() => {
                self.cursor = None;
                self.cursor_pos = None;
            }
            None => {
                let pos = self.cursor_pos.unwrap_or(0).min(self.view.len() - 1);
                self.move_cursor_to(pos);
            }
        }
    }

    /// Forget the selection; the next refresh selects the first row.
    pub fn reset_cursor(&mut self) {
        self.cursor = None;
        self.cursor_pos = None;
        self.spec = None;
        self.table_state = TableState::default();
    }

    pub fn cursor_pos(&self) -> Option<usize> {
        self.cursor_pos
    }

    pub fn cursor_row(&self) -> Option<&C::Row> {
        self.cursor.as_ref().and_then(|key| self.get(key))
    }

    pub fn view_rows(&self) -> impl Iterator<Item = &C::Row> + '_ {
        self.view.iter().map(|&idx| &self.items[idx])
    }

    /// Columns shown by the last refresh, in display order.
    pub fn visible_columns(&self) -> impl Iterator<Item = C> + '_ {
        C::ALL.iter().copied().filter(|col| {
            self.spec
                .as_ref()
                .is_none_or(|spec| spec.is_column_visible(col.label()))
        })
    }

    fn move_cursor_to(&mut self, pos: usize) {
        let Some(&idx) = self.view.get(pos) else {
            return;
        };
        self.cursor = Some(self.items[idx].key());
        self.cursor_pos = Some(pos);
    }

    /// Distinct column-filter values over every row, not just the view, so
    /// a value that the current filter hides can still be picked.
    pub fn distinct_filter_values(&self, column: &str) -> Vec<String> {
        let Some(col) = C::from_label(column) else {
            return Vec::new();
        };
        let values: BTreeSet<String> = self
            .items
            .iter()
            .flat_map(|row| col.filter_values(row))
            .collect();
        values.into_iter().collect()
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, table: TableContext<'_, C::Row>) {
        let columns: Vec<C> = self.visible_columns().collect();
        let show_checked_column = table.is_checked.is_some();
        let rows = self.view.iter().enumerate().map(|(pos, &idx)| {
            let is_highlighted = self.cursor_pos == Some(pos);
            let row_style = if is_highlighted {
                table.highlight_style
            } else {
                Style::default()
            };
            let ctx = CellContext {
                theme: table.theme,
                icons: table.icons,
                query: table.query,
                is_highlighted,
                row_style,
            };
            let row = &self.items[idx];
            let mut cells: Vec<Cell> = columns.iter().map(|col| col.cell(row, &ctx)).collect();
            if let Some(is_checked_fn) = table.is_checked {
                let is_checked = is_checked_fn(row);
                let bar_cell = Cell::from(Span::styled(
                    " ",
                    if is_checked {
                        Style::default().bg(table.theme.checked_bg)
                    } else {
                        Style::default()
                    },
                ));
                cells.insert(0, bar_cell);
            }
            TableRow::new(cells).style(row_style).height(1)
        });
        let sort = self
            .spec
            .as_ref()
            .and_then(|spec| spec.sort.as_ref())
            .map(|(col, ascending)| (col.as_str(), *ascending));
        let mut header_cells: Vec<Cell> = columns
            .iter()
            .map(|col| {
                let label = sort_header_label(col.label(), sort, table.icons);
                Cell::from(Line::from(label).alignment(col.header_alignment()))
            })
            .collect();
        let mut widths: Vec<Constraint> = columns.iter().map(|col| col.width()).collect();
        if show_checked_column {
            header_cells.insert(0, Cell::from(""));
            widths.insert(0, Constraint::Length(1));
        }
        if widths.is_empty() {
            widths.push(Constraint::Min(0));
        }
        let header = TableRow::new(header_cells)
            .style(table.header_style)
            .height(1);
        let widget = Table::new(rows, widths)
            .header(header)
            .block(table.block)
            .row_highlight_style(Style::default())
            .highlight_symbol(format!(" {} ", table.icons.highlight_arrow));
        self.table_state.select(self.cursor_pos);
        f.render_stateful_widget(widget, area, &mut self.table_state);
    }
}

/// Everything a list-tab table needs from the frame being drawn.
pub struct TableContext<'a, R = ()> {
    pub block: Block<'a>,
    pub header_style: Style,
    pub highlight_style: Style,
    pub theme: &'a Theme,
    pub icons: &'a Icons,
    pub query: &'a str,
    pub is_checked: Option<&'a dyn Fn(&R) -> bool>,
}

fn build_view<C: ColumnDef>(items: &[C::Row], spec: &ViewSpec) -> Vec<usize> {
    let query = spec.query.trim();
    let searchable: Vec<C> = C::ALL
        .iter()
        .copied()
        .filter(|col| col.searchable() && spec.is_column_visible(col.label()))
        .collect();
    let filters: Vec<(C, &HashSet<String>)> = spec
        .column_filters
        .iter()
        .filter(|(_, selected)| !selected.is_empty())
        .filter_map(|(label, selected)| C::from_label(label).map(|col| (col, selected)))
        .collect();
    let matcher = &*crate::app::FUZZY_MATCHER;
    let mut scored: Vec<(usize, Option<i64>)> = items
        .iter()
        .enumerate()
        .filter_map(|(idx, row)| {
            if query.is_empty() {
                Some((idx, None))
            } else {
                let mut best: Option<i64> = None;
                for col in &searchable {
                    if let Some(score) =
                        fuzzy_matcher::FuzzyMatcher::fuzzy_match(matcher, &col.text(row), query)
                    {
                        best = Some(best.map_or(score, |b| b.max(score)));
                    }
                }
                best.map(|score| (idx, Some(score)))
            }
        })
        .filter(|(idx, _)| {
            let row = &items[*idx];
            filters
                .iter()
                .all(|(col, selected)| col.matches_filter(row, selected))
        })
        .collect();
    if spec.sort.is_none() && !query.is_empty() {
        scored.sort_by(|a, b| b.1.cmp(&a.1));
    }
    let mut view: Vec<usize> = scored.into_iter().map(|(idx, _)| idx).collect();
    let sort = spec
        .sort
        .as_ref()
        .and_then(|(label, asc)| C::from_label(label).map(|col| (col, *asc)));
    if let Some((col, ascending)) = sort {
        let mut keyed: Vec<(SortKey, usize)> = view
            .into_iter()
            .map(|idx| (col.sort_key(&items[idx]), idx))
            .collect();
        keyed.sort_by(|(a, _), (b, _)| {
            let order = a.cmp(b);
            if ascending { order } else { order.reverse() }
        });
        view = keyed.into_iter().map(|(_, idx)| idx).collect();
    }
    view
}

/// Navigation and column-filter lookups shared by every list tab, so the
/// app can dispatch on the active tab without knowing its row type.
pub trait ListNavigation {
    fn next(&mut self);
    fn previous(&mut self);
    fn first(&mut self);
    fn last(&mut self);
    /// Select the row drawn `row` lines below the first visible one.
    fn select_visible_row(&mut self, row: usize);
    /// Move one row without wrapping.
    fn step(&mut self, down: bool);
    /// Move `rows` rows without wrapping.
    fn move_by(&mut self, down: bool, rows: usize);
    fn distinct_filter_values(&self, column: &str) -> Vec<String>;
}

impl<C: ColumnDef> ListNavigation for ListTab<C> {
    fn next(&mut self) {
        let len = self.view.len();
        if len == 0 {
            return;
        }
        let pos = match self.cursor_pos {
            Some(pos) if pos + 1 < len => pos + 1,
            _ => 0,
        };
        self.move_cursor_to(pos);
    }

    fn previous(&mut self) {
        let len = self.view.len();
        if len == 0 {
            return;
        }
        let pos = match self.cursor_pos {
            Some(0) => len - 1,
            Some(pos) => pos - 1,
            None => 0,
        };
        self.move_cursor_to(pos);
    }

    fn first(&mut self) {
        self.move_cursor_to(0);
    }

    fn last(&mut self) {
        if let Some(pos) = self.view.len().checked_sub(1) {
            self.move_cursor_to(pos);
        }
    }

    fn select_visible_row(&mut self, row: usize) {
        self.move_cursor_to(self.table_state.offset() + row);
    }

    fn step(&mut self, down: bool) {
        let Some(last) = self.view.len().checked_sub(1) else {
            return;
        };
        let pos = self.cursor_pos.unwrap_or(0);
        let pos = if down {
            (pos + 1).min(last)
        } else {
            pos.saturating_sub(1)
        };
        self.move_cursor_to(pos);
    }

    fn move_by(&mut self, down: bool, rows: usize) {
        if self.view.is_empty() {
            return;
        }
        let pos = crate::utils::ui::step_clamped(
            self.cursor_pos.unwrap_or(0),
            self.view.len(),
            down,
            rows,
        );
        self.move_cursor_to(pos);
    }

    fn distinct_filter_values(&self, column: &str) -> Vec<String> {
        ListTab::distinct_filter_values(self, column)
    }
}

/// Column labels in display order, for `Tab::columns`.
pub fn column_labels<C: ColumnDef>() -> Vec<&'static str> {
    C::ALL.iter().map(|col| col.label()).collect()
}

/// Labels of the columns shown before the user configures any.
pub fn default_column_labels<C: ColumnDef>() -> Vec<&'static str> {
    C::ALL
        .iter()
        .filter(|col| col.is_default())
        .map(|col| col.label())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell as CallCount;
    use std::collections::{HashMap, HashSet};

    use super::branches::BranchColumn;
    use super::releases::ReleaseColumn;
    use super::*;
    use crate::domain::branches::Branch;

    fn releases(json: serde_json::Value) -> ListTab<ReleaseColumn> {
        let mut list = ListTab::default();
        list.set_items(serde_json::from_value(json).expect("release fixture"));
        list
    }

    fn release_fixture() -> serde_json::Value {
        serde_json::json!([
            {"name": "Beta", "tag_name": "v2.0.0", "released_at": "2026-02-01T00:00:00Z",
             "description": "adds widgets", "author_name": "bob"},
            {"name": "Alpha", "tag_name": "v1.0.0", "released_at": "2026-01-01T00:00:00Z",
             "description": "first cut", "author_name": "alice"},
            {"name": "Gamma", "tag_name": "v3.0.0", "released_at": "2026-03-01T00:00:00Z",
             "description": "removes widgets", "author_name": "carol"}
        ])
    }

    fn inputs<'a>(
        query: &'a str,
        filters: Option<&'a HashMap<String, HashSet<String>>>,
        sort: Option<(&'a str, bool)>,
    ) -> ViewInputs<'a> {
        ViewInputs {
            query,
            enabled_columns: None,
            column_filters: filters,
            sort,
        }
    }

    fn view_tags(list: &ListTab<ReleaseColumn>) -> Vec<&str> {
        list.view_rows().map(|r| r.tag_name.as_str()).collect()
    }

    fn cursor_tag(list: &ListTab<ReleaseColumn>) -> Option<&str> {
        list.cursor_row().map(|r| r.tag_name.as_str())
    }

    #[test]
    fn selection_stays_on_the_same_release_when_the_sort_reorders_rows() {
        let mut list = releases(release_fixture());
        list.refresh(inputs("", None, None));
        list.next();
        assert_eq!(cursor_tag(&list), Some("v1.0.0"));

        list.refresh(inputs("", None, Some(("Tag", false))));

        assert_eq!(view_tags(&list), ["v3.0.0", "v2.0.0", "v1.0.0"]);
        assert_eq!(cursor_tag(&list), Some("v1.0.0"));
        list.previous();
        assert_eq!(cursor_tag(&list), Some("v2.0.0"));
    }

    #[test]
    fn selection_survives_a_refresh_that_reorders_the_rows() {
        let mut list = releases(release_fixture());
        list.refresh(inputs("", None, None));
        list.last();
        assert_eq!(cursor_tag(&list), Some("v3.0.0"));

        list.set_items(
            serde_json::from_value(serde_json::json!([
                {"name": "Gamma", "tag_name": "v3.0.0", "released_at": "2026-03-01T00:00:00Z"},
                {"name": "Delta", "tag_name": "v4.0.0", "released_at": "2026-04-01T00:00:00Z"}
            ]))
            .expect("refreshed releases"),
        );
        list.refresh(inputs("", None, None));

        assert_eq!(cursor_tag(&list), Some("v3.0.0"));
    }

    #[test]
    fn selection_filtered_out_lands_on_the_row_now_at_its_position() {
        let mut list = releases(release_fixture());
        list.refresh(inputs("", None, None));
        list.next();
        assert_eq!(cursor_tag(&list), Some("v1.0.0"));

        list.refresh(inputs("widgets", None, None));
        assert_eq!(view_tags(&list), ["v2.0.0", "v3.0.0"]);
        assert_eq!(cursor_tag(&list), Some("v3.0.0"));

        list.refresh(inputs("no such release", None, None));
        assert_eq!(cursor_tag(&list), None);

        list.refresh(inputs("", None, None));
        assert_eq!(cursor_tag(&list), Some("v2.0.0"));
    }

    #[test]
    fn reset_cursor_selects_the_first_row_on_a_refresh_with_unchanged_inputs() {
        let mut list = releases(release_fixture());
        list.refresh(inputs("", None, None));
        list.last();
        assert_eq!(cursor_tag(&list), Some("v3.0.0"));

        list.reset_cursor();
        list.refresh(inputs("", None, None));

        assert_eq!(cursor_tag(&list), Some("v2.0.0"));
    }

    #[test]
    fn release_notes_column_is_searched_and_filtered_under_its_own_label() {
        let mut list = releases(release_fixture());
        let filters = HashMap::from([(
            "Release Notes".to_string(),
            HashSet::from(["first cut".to_string()]),
        )]);
        list.refresh(inputs("", Some(&filters), None));
        assert_eq!(view_tags(&list), ["v1.0.0"]);

        list.refresh(inputs("first cut", None, None));
        assert_eq!(view_tags(&list), ["v1.0.0"]);
        assert_eq!(
            list.distinct_filter_values("Release Notes"),
            ["adds widgets", "first cut", "removes widgets"]
        );
    }

    #[test]
    fn branch_flag_filters_offer_the_shown_text_and_keep_saved_raw_values() {
        let mut list: ListTab<BranchColumn> = ListTab::default();
        let rows: Vec<Branch> = serde_json::from_value(serde_json::json!([
            {"name": "main", "default": true, "protected": true, "web_url": ""},
            {"name": "feature", "default": false, "protected": false, "web_url": ""}
        ]))
        .expect("branch fixture");
        list.set_items(rows);
        assert_eq!(list.distinct_filter_values("Default"), ["NO", "YES"]);

        for saved in ["YES", "true"] {
            let filters =
                HashMap::from([("Default".to_string(), HashSet::from([saved.to_string()]))]);
            list.refresh(inputs("", Some(&filters), None));
            let names: Vec<&str> = list.view_rows().map(|b| b.name.as_str()).collect();
            assert_eq!(names, ["main"], "saved filter value {saved}");
        }
    }

    thread_local! {
        static TEXT_CALLS: CallCount<usize> = const { CallCount::new(0) };
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct CountingColumn;

    struct Numbered(u64);

    impl Row for Numbered {
        type Key = u64;

        fn key(&self) -> u64 {
            self.0
        }
    }

    impl ColumnDef for CountingColumn {
        type Row = Numbered;

        const ALL: &'static [Self] = &[Self];

        fn label(self) -> &'static str {
            "N"
        }

        fn is_default(self) -> bool {
            true
        }

        fn width(self) -> Constraint {
            Constraint::Fill(1)
        }

        fn text(self, row: &Numbered) -> Cow<'_, str> {
            TEXT_CALLS.with(|calls| calls.set(calls.get() + 1));
            Cow::Owned(row.0.to_string())
        }

        fn cell(self, row: &Numbered, _ctx: &CellContext) -> Cell<'static> {
            Cell::from(row.0.to_string())
        }
    }

    #[test]
    fn view_is_rebuilt_only_when_its_inputs_or_rows_change() {
        let mut list: ListTab<CountingColumn> = ListTab::default();
        list.set_items(vec![Numbered(1), Numbered(2)]);
        let calls = || TEXT_CALLS.with(CallCount::get);

        list.refresh(inputs("1", None, None));
        assert_eq!(calls(), 2, "first build reads each row once");
        list.refresh(inputs("1", None, None));
        assert_eq!(calls(), 2, "unchanged inputs reuse the view");

        list.refresh(inputs("2", None, None));
        assert_eq!(calls(), 4, "a new query rebuilds the view");

        list.update(&2, |row| row.0 = 2);
        list.refresh(inputs("2", None, None));
        assert_eq!(calls(), 6, "a changed row rebuilds the view");
    }
}
