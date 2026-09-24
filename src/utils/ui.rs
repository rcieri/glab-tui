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
