use ratatui::widgets::{ListState, TableState};

pub struct StatefulList<T> {
    pub state: ListState,
    pub items: Vec<T>,
}

impl<T> StatefulList<T> {
    pub fn with_items(items: Vec<T>) -> StatefulList<T> {
        StatefulList {
            state: ListState::default(),
            items,
        }
    }

    pub fn next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.items.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn previous(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn first(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.state.select(Some(0));
    }

    pub fn last(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.state.select(Some(self.items.len() - 1));
    }

    pub fn unselect(&mut self) {
        self.state.select(None);
    }
}

pub struct StatefulTable<T> {
    pub state: TableState,
    pub items: Vec<T>,
}

impl<T> StatefulTable<T> {
    pub fn with_items(items: Vec<T>) -> StatefulTable<T> {
        StatefulTable {
            state: TableState::default(),
            items,
        }
    }

    pub fn next(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i >= len - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn next_item(&mut self) {
        let len = self.items.len();
        self.next(len);
    }

    pub fn previous(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    len - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn previous_item(&mut self) {
        let len = self.items.len();
        self.previous(len);
    }

    pub fn first(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        self.state.select(Some(0));
    }

    pub fn first_item(&mut self) {
        let len = self.items.len();
        self.first(len);
    }

    pub fn last(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        self.state.select(Some(len - 1));
    }

    pub fn last_item(&mut self) {
        let len = self.items.len();
        self.last(len);
    }

    pub fn unselect(&mut self) {
        self.state.select(None);
    }

    /// Moves the selection `rows` down or up, stopping at the first and last row.
    pub fn move_by(&mut self, len: usize, down: bool, rows: usize) {
        if len == 0 {
            return;
        }
        let idx = step_clamped(self.state.selected().unwrap_or(0), len, down, rows);
        self.state.select(Some(idx));
    }
}

/// `idx` moved `rows` down or up inside `0..len`. Unlike `next`/`previous`,
/// a page move stops at the ends instead of wrapping.
pub fn step_clamped(idx: usize, len: usize, down: bool, rows: usize) -> usize {
    let idx = if down {
        idx.saturating_add(rows)
    } else {
        idx.saturating_sub(rows)
    };
    idx.min(len.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stateful_table_first_and_last() {
        let mut table: StatefulTable<i32> = StatefulTable::with_items(vec![10, 20, 30]);
        // Empty len
        table.first(0);
        assert_eq!(table.state.selected(), None);
        table.last(0);
        assert_eq!(table.state.selected(), None);

        // N items
        table.first(3);
        assert_eq!(table.state.selected(), Some(0));
        table.last(3);
        assert_eq!(table.state.selected(), Some(2));

        // 1 item
        table.first(1);
        assert_eq!(table.state.selected(), Some(0));
        table.last(1);
        assert_eq!(table.state.selected(), Some(0));
    }

    #[test]
    fn step_clamped_stops_at_both_ends_instead_of_wrapping() {
        assert_eq!(step_clamped(2, 10, true, 5), 7);
        assert_eq!(step_clamped(7, 10, true, 5), 9);
        assert_eq!(step_clamped(7, 10, false, 5), 2);
        assert_eq!(step_clamped(2, 10, false, 5), 0);
        assert_eq!(step_clamped(0, 0, true, 5), 0);
    }

    #[test]
    fn stateful_table_move_by_clamps_and_ignores_empty_lists() {
        let mut table: StatefulTable<i32> = StatefulTable::with_items(vec![]);
        table.move_by(0, true, 3);
        assert_eq!(table.state.selected(), None);

        table.state.select(Some(1));
        table.move_by(5, true, 3);
        assert_eq!(table.state.selected(), Some(4));
        table.move_by(5, true, 3);
        assert_eq!(table.state.selected(), Some(4));
        table.move_by(5, false, 3);
        assert_eq!(table.state.selected(), Some(1));
        table.move_by(5, false, 3);
        assert_eq!(table.state.selected(), Some(0));
    }

    #[test]
    fn stateful_list_first_and_last() {
        let mut list: StatefulList<i32> = StatefulList::with_items(vec![]);
        list.first();
        assert_eq!(list.state.selected(), None);
        list.last();
        assert_eq!(list.state.selected(), None);

        let mut list_populated: StatefulList<i32> = StatefulList::with_items(vec![1, 2, 3]);
        list_populated.first();
        assert_eq!(list_populated.state.selected(), Some(0));
        list_populated.last();
        assert_eq!(list_populated.state.selected(), Some(2));
    }
}
