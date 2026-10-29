//! Where the Dashboard's selection is, whose work it shows, and what each
//! selectable row lists.

use crate::app::{App, Tab};
use crate::domain::contributions::{Contribution, TargetKind};
use crate::domain::dashboard::{self, Activity, IssueQueue, MrQueue, Now, Person, recent_activity};

/// The repository's activity feed as the Dashboard last fetched it.
#[derive(Debug, Default, Clone)]
pub enum ContributionFeed {
    /// Not fetched yet, or not available in the current scope.
    #[default]
    NotLoaded,
    Loaded(Vec<Contribution>),
    Failed(String),
}

impl ContributionFeed {
    pub fn contributions(&self) -> &[Contribution] {
        match self {
            ContributionFeed::Loaded(contributions) => contributions,
            ContributionFeed::NotLoaded | ContributionFeed::Failed(_) => &[],
        }
    }
}

/// Whose work the Dashboard shows.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum DashboardPerson {
    /// The signed-in user.
    #[default]
    Me,
    Everyone,
    User(String),
}

impl DashboardPerson {
    /// `Me` is the signed-in user, and everyone while their login is unknown.
    pub fn resolve<'a>(&'a self, signed_in: Option<&'a str>) -> Person<'a> {
        match self {
            DashboardPerson::Everyone => Person::Everyone,
            DashboardPerson::Me => signed_in.map_or(Person::Everyone, Person::User),
            DashboardPerson::User(login) => Person::User(login),
        }
    }
}

/// A card on the Dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashboardCard {
    MergeRequests,
    Issues,
    Contributors,
    Pipelines,
    Milestones,
    Releases,
}

impl DashboardCard {
    /// The card summarizing `tab`'s list.
    pub fn for_tab(tab: Tab) -> Option<Self> {
        match tab {
            Tab::MergeRequests => Some(Self::MergeRequests),
            Tab::Issues => Some(Self::Issues),
            Tab::Pipelines => Some(Self::Pipelines),
            Tab::Milestones => Some(Self::Milestones),
            Tab::Releases => Some(Self::Releases),
            _ => None,
        }
    }

    /// The tab whose list the card summarizes. Contributors draws on several.
    pub fn tab(self) -> Option<Tab> {
        match self {
            Self::MergeRequests => Some(Tab::MergeRequests),
            Self::Issues => Some(Tab::Issues),
            Self::Contributors => None,
            Self::Pipelines => Some(Tab::Pipelines),
            Self::Milestones => Some(Tab::Milestones),
            Self::Releases => Some(Tab::Releases),
        }
    }
}

/// One selectable line of the Dashboard: a queue, a person, or a whole card
/// that has no lines of its own to pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashboardRow {
    Mr(MrQueue),
    Issue(IssueQueue),
    Contributor(DashboardPerson),
    Card(DashboardCard),
}

impl DashboardRow {
    pub fn card(&self) -> DashboardCard {
        match self {
            DashboardRow::Mr(_) => DashboardCard::MergeRequests,
            DashboardRow::Issue(_) => DashboardCard::Issues,
            DashboardRow::Contributor(_) => DashboardCard::Contributors,
            DashboardRow::Card(card) => *card,
        }
    }
}

/// What `Enter` on the Dashboard leads to outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashboardAction {
    Stay,
    OpenTab(Tab),
    OpenIssue { project: String, iid: u64 },
    OpenMr { project: String, iid: u64 },
}

#[derive(Debug, Default, Clone)]
pub struct DashboardState {
    pub person: DashboardPerson,
    /// `None` until the first move, which leaves the first row selected.
    row: Option<DashboardRow>,
    /// Whether the keys move through the item list below the cards.
    pub is_list_focused: bool,
    pub list_cursor: usize,
    pub contributions: ContributionFeed,
}

impl DashboardState {
    /// The selected row among `rows`, the first one when the selection left
    /// them (a contributor with nothing open any more).
    pub fn selected_row<'a>(&'a self, rows: &'a [DashboardRow]) -> Option<&'a DashboardRow> {
        self.row
            .as_ref()
            .filter(|row| rows.contains(row))
            .or(rows.first())
    }

    /// Move one row forward or back, wrapping.
    pub fn step_row(&mut self, rows: &[DashboardRow], forward: bool) {
        if rows.is_empty() {
            return;
        }
        let current = self
            .selected_row(rows)
            .and_then(|row| rows.iter().position(|r| r == row))
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % rows.len()
        } else {
            (current + rows.len() - 1) % rows.len()
        };
        self.row = Some(rows[next].clone());
    }

    pub fn select_row(&mut self, row: Option<&DashboardRow>) {
        if let Some(row) = row {
            self.row = Some(row.clone());
        }
    }

    /// Move the list cursor one item, staying within `len` items.
    pub fn step_list(&mut self, len: usize, forward: bool) {
        self.list_cursor = if forward {
            (self.list_cursor + 1).min(len.saturating_sub(1))
        } else {
            self.list_cursor.saturating_sub(1)
        };
    }

    pub fn focus_list(&mut self) {
        self.is_list_focused = true;
        self.list_cursor = 0;
    }
}

impl App {
    /// The Dashboard's cards in the current scope, in order, without the
    /// tabs the config disables. Contributors follows the MR/PR and issue
    /// cards it draws on.
    pub fn dashboard_cards(&self) -> Vec<DashboardCard> {
        let available = self.available_tabs();
        let mut cards: Vec<DashboardCard> = Tab::dashboard_sources(&self.scope, self.kind())
            .iter()
            .filter(|tab| available.contains(tab))
            .filter_map(|tab| DashboardCard::for_tab(*tab))
            .collect();
        if let Some(last_source) = cards
            .iter()
            .rposition(|card| matches!(card, DashboardCard::MergeRequests | DashboardCard::Issues))
        {
            cards.insert(last_source + 1, DashboardCard::Contributors);
        }
        cards
    }

    pub fn dashboard_signed_in_user(&self) -> Option<&str> {
        dashboard::signed_in_user(&self.mrs.items)
    }

    /// Whose work the MR/PR and issue cards count.
    pub fn dashboard_person(&self) -> Person<'_> {
        self.dashboard
            .person
            .resolve(self.dashboard_signed_in_user())
    }

    /// Every selectable row, card by card.
    pub fn dashboard_rows(&self, now: Now) -> Vec<DashboardRow> {
        let shows_due_dates = self.kind().is_gitlab();
        let mut rows = Vec::new();
        for card in self.dashboard_cards() {
            match card {
                DashboardCard::MergeRequests => rows.extend(MrQueue::ALL.map(DashboardRow::Mr)),
                DashboardCard::Issues => rows.extend(
                    IssueQueue::ALL
                        .into_iter()
                        .filter(|queue| shows_due_dates || !queue.needs_due_dates())
                        .map(DashboardRow::Issue),
                ),
                DashboardCard::Contributors => {
                    rows.push(DashboardRow::Contributor(DashboardPerson::Everyone));
                    rows.push(DashboardRow::Contributor(DashboardPerson::Me));
                    let contributors = dashboard::contributors(
                        &self.issues.items,
                        &self.mrs.items,
                        self.dashboard.contributions.contributions(),
                        self.dashboard_signed_in_user(),
                        now,
                    );
                    rows.extend(contributors.into_iter().map(|(login, _)| {
                        DashboardRow::Contributor(DashboardPerson::User(login.to_string()))
                    }));
                }
                card => rows.push(DashboardRow::Card(card)),
            }
        }
        rows
    }

    /// The issues and MRs/PRs listed below the cards for the selected row.
    pub fn dashboard_items(&self, now: Now) -> Vec<Activity<'_>> {
        let rows = self.dashboard_rows(now);
        self.dashboard_items_for(self.dashboard.selected_row(&rows), now)
    }

    /// What `row` lists: a queue's items, a contributor's activity, or the
    /// shown person's activity.
    pub fn dashboard_items_for(&self, row: Option<&DashboardRow>, now: Now) -> Vec<Activity<'_>> {
        let signed_in = self.dashboard_signed_in_user();
        let person = self.dashboard_person();
        let (issues, mrs) = (&self.issues.items, &self.mrs.items);
        let contributions = self.dashboard.contributions.contributions();
        match row {
            Some(DashboardRow::Mr(queue)) => queue
                .list(mrs, person, signed_in, now)
                .into_iter()
                .map(Activity::MergeRequest)
                .collect(),
            Some(DashboardRow::Issue(queue)) => queue
                .list(issues, person, now)
                .into_iter()
                .map(Activity::Issue)
                .collect(),
            Some(DashboardRow::Contributor(contributor)) => {
                recent_activity(issues, mrs, contributions, contributor.resolve(signed_in))
            }
            Some(DashboardRow::Card(_)) | None => {
                recent_activity(issues, mrs, contributions, person)
            }
        }
    }

    pub fn dashboard_selected_row(&self, now: Now) -> Option<DashboardRow> {
        let rows = self.dashboard_rows(now);
        self.dashboard.selected_row(&rows).cloned()
    }

    /// Move to the next or previous row, or item while the list is focused.
    pub fn step_dashboard(&mut self, forward: bool) {
        let now = Now::current();
        if self.dashboard.is_list_focused {
            let len = self.dashboard_items(now).len();
            self.dashboard.step_list(len, forward);
        } else {
            let rows = self.dashboard_rows(now);
            self.dashboard.step_row(&rows, forward);
        }
    }

    pub fn select_first_on_dashboard(&mut self) {
        if self.dashboard.is_list_focused {
            self.dashboard.list_cursor = 0;
        } else {
            let rows = self.dashboard_rows(Now::current());
            self.dashboard.select_row(rows.first());
        }
    }

    pub fn select_last_on_dashboard(&mut self) {
        let now = Now::current();
        if self.dashboard.is_list_focused {
            self.dashboard.list_cursor = self.dashboard_items(now).len().saturating_sub(1);
        } else {
            let rows = self.dashboard_rows(now);
            self.dashboard.select_row(rows.last());
        }
    }

    /// `Enter` on the Dashboard. A queue or contributor opens its list (a
    /// contributor also becomes the person shown), a card its tab, a listed
    /// item its issue or MR/PR.
    pub fn enter_dashboard(&mut self) -> DashboardAction {
        let now = Now::current();
        if self.dashboard.is_list_focused {
            let items = self.dashboard_items(now);
            let cursor = self
                .dashboard
                .list_cursor
                .min(items.len().saturating_sub(1));
            let action = match items.get(cursor) {
                Some(Activity::Issue(issue)) => DashboardAction::OpenIssue {
                    project: self.scope.project_of(&issue.project_path).to_string(),
                    iid: issue.iid,
                },
                Some(Activity::MergeRequest(mr)) => DashboardAction::OpenMr {
                    project: self.scope.project_of(&mr.project_path).to_string(),
                    iid: mr.iid,
                },
                Some(Activity::Contribution(contribution)) => match &contribution.target {
                    Some(target) => {
                        let project = self.scope.as_str().to_string();
                        let iid = target.iid;
                        match target.kind {
                            TargetKind::Issue => DashboardAction::OpenIssue { project, iid },
                            TargetKind::MergeRequest => DashboardAction::OpenMr { project, iid },
                        }
                    }
                    None => DashboardAction::Stay,
                },
                None => DashboardAction::Stay,
            };
            let is_commit = matches!(
                items.get(cursor),
                Some(Activity::Contribution(contribution)) if contribution.target.is_none()
            );
            if is_commit {
                self.show_notice("A commit has no issue or MR/PR to open".to_string());
            }
            return action;
        }
        let Some(row) = self.dashboard_selected_row(now) else {
            return DashboardAction::Stay;
        };
        match &row {
            DashboardRow::Card(card) => {
                return card
                    .tab()
                    .map_or(DashboardAction::Stay, DashboardAction::OpenTab);
            }
            DashboardRow::Contributor(person) => {
                self.dashboard.person = person.clone();
                self.dashboard.select_row(Some(&row));
            }
            DashboardRow::Mr(_) | DashboardRow::Issue(_) => {}
        }
        if self.dashboard_items(now).is_empty() {
            self.show_notice("Nothing to list here".to_string());
        } else {
            self.dashboard.focus_list();
        }
        DashboardAction::Stay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<DashboardRow> {
        vec![
            DashboardRow::Mr(MrQueue::Open),
            DashboardRow::Contributor(DashboardPerson::User("ada".to_string())),
            DashboardRow::Card(DashboardCard::Releases),
        ]
    }

    #[test]
    fn stepping_wraps_around_both_ends_and_starts_from_the_first_row() {
        let rows = rows();
        let mut state = DashboardState::default();
        assert_eq!(state.selected_row(&rows), Some(&rows[0]));

        state.step_row(&rows, false);
        assert_eq!(state.selected_row(&rows), Some(&rows[2]));
        state.step_row(&rows, true);
        assert_eq!(state.selected_row(&rows), Some(&rows[0]));
    }

    #[test]
    fn a_selected_contributor_who_left_the_rows_falls_back_to_the_first_row() {
        let mut rows = rows();
        let mut state = DashboardState::default();
        state.step_row(&rows, true);
        rows.remove(1);

        assert_eq!(state.selected_row(&rows), Some(&rows[0]));
        state.step_row(&rows, true);
        assert_eq!(state.selected_row(&rows), Some(&rows[1]));
    }

    #[test]
    fn the_list_cursor_stays_on_the_listed_items() {
        let mut state = DashboardState::default();
        state.focus_list();
        state.step_list(2, true);
        state.step_list(2, true);
        assert_eq!(state.list_cursor, 1);
        state.step_list(2, false);
        state.step_list(2, false);
        assert_eq!(state.list_cursor, 0);
    }

    #[test]
    fn me_stands_for_everyone_until_the_signed_in_login_is_known() {
        assert_eq!(DashboardPerson::Me.resolve(None), Person::Everyone);
        assert_eq!(
            DashboardPerson::Me.resolve(Some("ada")),
            Person::User("ada")
        );
    }
}
