//! The user's column order: a tab's columns sorted by the saved column list
//! (`columns = [...]` in a pane's config, or the Configure View's `J`/`K`).

/// Column names that mean the same column on GitLab and GitHub, so an order
/// saved against one host keeps its place on the other.
const SAME_COLUMN: &[&[&str]] = &[
    &["MRs", "PRs", "Related MRs", "Related PRs"],
    &["Closes", "Linked Issues"],
    &["Pipeline", "Action"],
];

fn rank(order: &[String], label: &str) -> Option<usize> {
    let aliases: &[&str] = SAME_COLUMN
        .iter()
        .find(|names| names.contains(&label))
        .copied()
        .unwrap_or(&[]);
    order
        .iter()
        .position(|name| name == label || aliases.contains(&name.as_str()))
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

/// `columns` (in display order) with `label` moved one place towards the end
/// when `towards_end`, else towards the start, as the order to store.
/// `None` when `label` is already at that end or not among `columns`.
pub fn move_column(columns: &[&str], label: &str, towards_end: bool) -> Option<Vec<String>> {
    let from = columns.iter().position(|c| *c == label)?;
    let to = if towards_end {
        from.checked_add(1).filter(|&to| to < columns.len())?
    } else {
        from.checked_sub(1)?
    };
    let mut moved: Vec<String> = columns.iter().map(|c| c.to_string()).collect();
    moved.swap(from, to);
    Some(moved)
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
    fn moving_a_column_swaps_it_with_its_neighbour_and_stops_at_the_ends() {
        let columns = ["ID", "State", "Title"];

        assert_eq!(
            move_column(&columns, "State", true),
            Some(order(&["ID", "Title", "State"]))
        );
        assert_eq!(
            move_column(&columns, "State", false),
            Some(order(&["State", "ID", "Title"]))
        );
        assert_eq!(move_column(&columns, "Title", true), None);
        assert_eq!(move_column(&columns, "ID", false), None);
    }
}
