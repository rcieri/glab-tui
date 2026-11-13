//! The user's column order: a tab's columns sorted by the saved column list
//! (`columns = [...]` in a pane's config, or the Configure View's `J`/`K`).

/// Column names that mean the same column on GitLab and GitHub, so an order
/// saved against one host keeps its place on the other.
const SAME_COLUMN: &[&[&str]] = &[
    &["MRs", "PRs", "Related MRs", "Related PRs"],
    &["Closes", "Linked Issues"],
    &["Pipeline", "Action"],
];

/// Whether `a` and `b` name the same column, directly or as the GitLab and
/// GitHub names of one column.
pub fn same_column(a: &str, b: &str) -> bool {
    a == b
        || SAME_COLUMN
            .iter()
            .any(|names| names.contains(&a) && names.contains(&b))
}

fn rank(order: &[String], label: &str) -> Option<usize> {
    order.iter().position(|name| same_column(name, label))
}

/// `columns`, given in built-in order, sorted into `order`: the listed
/// columns first, in list order, then the others in built-in order.
pub fn order_columns<'a>(columns: &[&'a str], order: Option<&[String]>) -> Vec<&'a str> {
    let mut sorted = columns.to_vec();
    if let Some(order) = order {
        sorted.sort_by_key(|label| rank(order, label).unwrap_or(usize::MAX));
    }
    sorted
}

/// For the columns a table draws (`visible`, in built-in order), the
/// position in `visible` of each column to draw, left to right.
pub fn display_permutation(visible: &[&str], order: Option<&[String]>) -> Vec<usize> {
    order_columns(visible, order)
        .iter()
        .filter_map(|label| visible.iter().position(|v| v == label))
        .collect()
}

/// `cells`, built in `display_permutation`'s built-in order, rearranged into
/// display order. Left as they are when their count does not match.
pub fn permute<T>(cells: Vec<T>, permutation: &[usize]) -> Vec<T> {
    if cells.len() != permutation.len() {
        return cells;
    }
    let mut slots: Vec<Option<T>> = cells.into_iter().map(Some).collect();
    permutation
        .iter()
        .filter_map(|&idx| slots.get_mut(idx).and_then(Option::take))
        .collect()
}

/// `cells` with the first `fixed` left in place (a table's selection bar)
/// and the rest rearranged by `permute`.
pub fn permute_after<T>(mut cells: Vec<T>, fixed: usize, permutation: &[usize]) -> Vec<T> {
    if fixed > cells.len() {
        return cells;
    }
    let rest = cells.split_off(fixed);
    cells.extend(permute(rest, permutation));
    cells
}

/// `items` with `item` moved one place towards the end when `towards_end`,
/// else towards the start. `None` when `item` is already at that end or not
/// among `items`.
pub fn move_item<T: Copy + PartialEq>(items: &[T], item: T, towards_end: bool) -> Option<Vec<T>> {
    let from = items.iter().position(|c| *c == item)?;
    let to = if towards_end {
        from.checked_add(1).filter(|&to| to < items.len())?
    } else {
        from.checked_sub(1)?
    };
    let mut moved = items.to_vec();
    moved.swap(from, to);
    Some(moved)
}

/// `display` (the current columns in display order) with every name of
/// `stored` that `display` lacks put back right after the stored name before
/// it. Keeps the place of a column this host or scope does not have (a
/// group-scope `Project` while browsing a repository) when the order is
/// stored again.
pub fn merge_order(display: &[&str], stored: &[String]) -> Vec<String> {
    let mut merged: Vec<String> = display.iter().map(|label| label.to_string()).collect();
    let mut insert_at = 0;
    for name in stored {
        match merged.iter().position(|label| same_column(label, name)) {
            Some(pos) => insert_at = pos + 1,
            None => {
                merged.insert(insert_at, name.clone());
                insert_at += 1;
            }
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn listed_columns_come_first_in_list_order_then_the_rest_in_built_in_order() {
        let columns = ["ID", "State", "Title", "Labels", "Author"];
        let saved = order(&["Title", "ID"]);

        assert_eq!(
            order_columns(&columns, Some(&saved)),
            vec!["Title", "ID", "State", "Labels", "Author"]
        );
        assert_eq!(order_columns(&columns, None), columns.to_vec());
    }

    #[test]
    fn an_order_saved_on_one_host_places_the_other_hosts_column_name() {
        let columns = ["ID", "Title", "PRs"];
        let saved = order(&["MRs", "ID"]);

        assert_eq!(
            order_columns(&columns, Some(&saved)),
            vec!["PRs", "ID", "Title"]
        );
    }

    #[test]
    fn cells_follow_the_display_permutation() {
        let visible = ["ID", "State", "Title"];
        let permutation = display_permutation(&visible, Some(&order(&["Title", "ID", "State"])));

        assert_eq!(permutation, vec![2, 0, 1]);
        assert_eq!(
            permute(vec!['i', 's', 't'], &permutation),
            vec!['t', 'i', 's']
        );
        assert_eq!(permute(vec!['i', 's'], &permutation), vec!['i', 's']);
    }

    #[test]
    fn moving_an_item_swaps_it_with_its_neighbour_and_stops_at_the_ends() {
        let columns = ["ID", "State", "Title"];

        assert_eq!(
            move_item(&columns, "State", true),
            Some(vec!["ID", "Title", "State"])
        );
        assert_eq!(
            move_item(&columns, "State", false),
            Some(vec!["State", "ID", "Title"])
        );
        assert_eq!(move_item(&columns, "Title", true), None);
        assert_eq!(move_item(&columns, "ID", false), None);
    }

    #[test]
    fn merging_keeps_the_place_of_stored_columns_the_display_lacks() {
        let display = ["Title", "ID", "State"];
        let stored = order(&["Project", "ID", "Title", "Due Date", "State"]);

        assert_eq!(
            merge_order(&display, &stored),
            order(&["Project", "Title", "Due Date", "ID", "State"])
        );
    }

    #[test]
    fn merging_matches_the_other_hosts_column_name() {
        let display = ["ID", "PRs", "Title"];
        let stored = order(&["ID", "MRs", "Title"]);

        assert_eq!(
            merge_order(&display, &stored),
            order(&["ID", "PRs", "Title"])
        );
    }
}
