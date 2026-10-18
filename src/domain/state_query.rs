use serde::{Deserialize, Serialize};

/// Which issue or MR/PR states a list request asks the server for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StateQuery {
    #[default]
    All,
    Open,
    Closed,
    Merged,
}

impl StateQuery {
    /// Whether rows listed with `self` contain every row a listing with
    /// `narrower` would return.
    pub fn includes(self, narrower: StateQuery) -> bool {
        self == StateQuery::All || self == narrower
    }
}

/// The state query for each tab whose list the server can narrow by state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StateQueries {
    pub issues: StateQuery,
    pub mrs: StateQuery,
}

#[cfg(test)]
mod tests {
    use super::StateQuery;

    #[test]
    fn all_includes_every_state_and_a_single_state_only_itself() {
        let states = [
            StateQuery::All,
            StateQuery::Open,
            StateQuery::Closed,
            StateQuery::Merged,
        ];
        for narrower in states {
            assert!(StateQuery::All.includes(narrower), "All ⊇ {narrower:?}");
        }
        for wider in [StateQuery::Open, StateQuery::Closed, StateQuery::Merged] {
            for narrower in states {
                assert_eq!(
                    wider.includes(narrower),
                    wider == narrower,
                    "{wider:?} ⊇ {narrower:?}"
                );
            }
        }
    }
}
