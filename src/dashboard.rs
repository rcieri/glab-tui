//! Where the Dashboard's selection is, whose work it shows, and what each
//! selectable row lists.

use crate::app::{App, Tab};
use crate::domain::contributions::{Contribution, ContributionHistory, TargetKind};
use crate::domain::dashboard::{
    self, Activity, ActivityWindow, IssueQueue, MilestoneProgress, MrQueue, Now, Person,
    recent_activity,
};
use crate::domain::pipelines::Pipeline;
use crate::domain::releases::Release;
use chrono::{DateTime, Utc};
use std::cell::Cell;
use std::collections::HashMap;

/// The repository's activity feed as the Dashboard last fetched it.
#[derive(Debug, Default, Clone)]
pub enum ContributionFeed {
    /// Not fetched yet, or not available in the current scope.
    #[default]
    NotLoaded,
    Loaded(ContributionHistory),
    Failed(String),
}

impl ContributionFeed {
    pub fn contributions(&self) -> &[Contribution] {
        match self {
            ContributionFeed::Loaded(history) => &history.contributions,
            ContributionFeed::NotLoaded | ContributionFeed::Failed(_) => &[],
        }
    }

    /// The oldest event read when the feed stopped short of `since`.
    pub fn cut_off_after(&self, since: DateTime<Utc>) -> Option<&str> {
        let ContributionFeed::Loaded(history) = self else {
            return None;
        };
        let cut_off_at = history.cut_off_at.as_deref()?;
        DateTime::parse_from_rfc3339(cut_off_at)
            .is_ok_and(|at| at.with_timezone(&Utc) > since)
            .then_some(cut_off_at)
    }
}

/// What the Contributors card counts per person.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ContributorsView {
    /// Open MRs/PRs, pending reviews, assigned and overdue issues.
    #[default]
    OpenWork,
    /// Commits, comments, thread comments and reviews in the window.
    Activity(ActivityWindow),
}

impl ContributorsView {
    pub const ALL: [ContributorsView; 4] = [
        ContributorsView::OpenWork,
        ContributorsView::Activity(ActivityWindow::Day),
        ContributorsView::Activity(ActivityWindow::Week),
        ContributorsView::Activity(ActivityWindow::Month),
    ];
    pub const COLUMN_COUNT: usize = 4;

    pub fn next(self) -> Self {
        let current = Self::ALL.iter().position(|view| *view == self).unwrap_or(0);
        Self::ALL[(current + 1) % Self::ALL.len()]
    }

    pub fn columns(self) -> [&'static str; Self::COLUMN_COUNT] {
        match self {
            ContributorsView::OpenWork => ["MRs", "rev", "iss", "ovd"],
            ContributorsView::Activity(_) => ["cmt", "com", "thr", "rvw"],
        }
    }

    /// The start of the window whose contributions the item list shows.
    pub fn activity_since(self, now: Now) -> DateTime<Utc> {
        match self {
            ContributorsView::OpenWork => ActivityWindow::Month.starts_at(now),
            ContributorsView::Activity(window) => window.starts_at(now),
        }
    }
}

/// The Contributors column people are ordered by. Unsorted, they are listed
/// by login, so the card is not a ranking unless asked to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContributorSort {
    pub column: usize,
    pub is_descending: bool,
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
    /// `me`, `everyone` or a login (with or without its `@`), as
    /// `dashboard.person` takes it.
    pub fn from_config(value: &str) -> Self {
        let value = value.trim();
        if value.is_empty() || value.eq_ignore_ascii_case("me") {
            DashboardPerson::Me
        } else if value.eq_ignore_ascii_case("everyone") {
            DashboardPerson::Everyone
        } else {
            DashboardPerson::User(value.trim_start_matches('@').to_string())
        }
    }

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

    /// Whether the card has lines of its own to pick before its item list.
    pub fn has_rows(self) -> bool {
        matches!(
            self,
            Self::MergeRequests | Self::Issues | Self::Contributors
        )
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

/// One entry of the list below the cards.
#[derive(Debug, Clone, Copy)]
pub enum DashboardItem<'a> {
    Activity(Activity<'a>),
    Pipeline(&'a Pipeline),
    Milestone(MilestoneProgress<'a>),
    Release(&'a Release),
}

/// Where the Dashboard's keys go. Each `Enter` goes one level down and each
/// `Esc` one level up.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DashboardFocus {
    /// The tab itself: `h`/`l` still switch tabs.
    #[default]
    Tab,
    /// `h`/`j`/`k`/`l` move between cards.
    Cards,
    /// `j`/`k` pick a queue or a person of the selected card.
    Rows,
    /// `j`/`k` walk the item list.
    List,
}

/// A move between cards on the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardStep {
    Left,
    Right,
    Up,
    Down,
}

/// What `Enter` on the Dashboard leads to outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashboardAction {
    Stay,
    OpenIssue { project: String, iid: u64 },
    OpenMr { project: String, iid: u64 },
    OpenPipeline(u64),
    OpenMilestone(u64),
    OpenRelease(String),
}

#[derive(Debug, Default, Clone)]
pub struct DashboardState {
    pub person: DashboardPerson,
    pub focus: DashboardFocus,
    /// `None` until the first move, which leaves the first card selected.
    card: Option<DashboardCard>,
    /// The row last picked on each card.
    rows_by_card: HashMap<DashboardCard, DashboardRow>,
    pub list_cursor: usize,
    /// Cards per grid line in the last draw, the stride of `j`/`k` between
    /// cards. Only the draw knows the width that decides it.
    pub grid_columns: Cell<usize>,
    pub contributions: ContributionFeed,
    pub contributors_view: ContributorsView,
    pub contributors_sort: Option<ContributorSort>,
}

/// The cards `rows` belong to, in order.
fn cards_of(rows: &[DashboardRow]) -> Vec<DashboardCard> {
    let mut cards: Vec<DashboardCard> = Vec::new();
    for row in rows {
        if cards.last() != Some(&row.card()) {
            cards.push(row.card());
        }
    }
    cards
}

impl DashboardState {
    /// The selected card among the ones `rows` belong to, the first one when
    /// the selection left them.
    pub fn selected_card(&self, rows: &[DashboardRow]) -> Option<DashboardCard> {
        self.card
            .filter(|card| rows.iter().any(|row| row.card() == *card))
            .or_else(|| rows.first().map(DashboardRow::card))
    }

    /// The selected card's picked row, its first one when the pick left the
    /// rows (a contributor with nothing open any more).
    pub fn selected_row<'a>(&'a self, rows: &'a [DashboardRow]) -> Option<&'a DashboardRow> {
        let card = self.selected_card(rows)?;
        self.rows_by_card
            .get(&card)
            .filter(|row| rows.contains(row))
            .or_else(|| rows.iter().find(|row| row.card() == card))
    }

    /// Move one row forward or back within the selected card, wrapping.
    pub fn step_row(&mut self, rows: &[DashboardRow], forward: bool) {
        let Some(card) = self.selected_card(rows) else {
            return;
        };
        let in_card: Vec<&DashboardRow> = rows.iter().filter(|row| row.card() == card).collect();
        let current = self
            .selected_row(rows)
            .and_then(|row| in_card.iter().position(|r| *r == row))
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % in_card.len()
        } else {
            (current + in_card.len() - 1) % in_card.len()
        };
        let row = in_card[next].clone();
        self.rows_by_card.insert(card, row);
    }

    /// Select `row` and the card it belongs to.
    pub fn select_row(&mut self, row: Option<&DashboardRow>) {
        if let Some(row) = row {
            self.card = Some(row.card());
            self.rows_by_card.insert(row.card(), row.clone());
        }
    }

    /// Move to the neighbouring card on a grid `grid_columns` wide. A step
    /// off the grid stays put; down from a line whose card below is missing
    /// lands on the last card.
    pub fn step_card(&mut self, rows: &[DashboardRow], step: CardStep) {
        let cards = cards_of(rows);
        let Some(last) = cards.len().checked_sub(1) else {
            return;
        };
        let columns = self.grid_columns.get().max(1);
        let current = self
            .selected_card(rows)
            .and_then(|card| cards.iter().position(|c| *c == card))
            .unwrap_or(0);
        let next = match step {
            CardStep::Left => (current % columns != 0).then(|| current - 1),
            CardStep::Right => {
                (current % columns + 1 < columns && current < last).then(|| current + 1)
            }
            CardStep::Up => current.checked_sub(columns),
            CardStep::Down if current + columns <= last => Some(current + columns),
            CardStep::Down => (current / columns < last / columns).then_some(last),
        };
        if let Some(next) = next {
            self.card = Some(cards[next]);
        }
    }

    fn select_edge_card(&mut self, rows: &[DashboardRow], is_last: bool) {
        let cards = cards_of(rows);
        let card = if is_last { cards.last() } else { cards.first() };
        if let Some(card) = card {
            self.card = Some(*card);
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
        self.focus = DashboardFocus::List;
        self.list_cursor = 0;
    }
}

impl App {
    /// The current moment, with the configured idle thresholds.
    pub fn dashboard_now(&self) -> Now {
        Now::current(self.config.dashboard.idle_days())
    }

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
                    rows.extend(self.sorted_contributors(now).into_iter().map(|login| {
                        DashboardRow::Contributor(DashboardPerson::User(login.to_string()))
                    }));
                }
                card => rows.push(DashboardRow::Card(card)),
            }
        }
        rows
    }

    /// What the list below the cards shows for the selected row.
    pub fn dashboard_items(&self, now: Now) -> Vec<DashboardItem<'_>> {
        let rows = self.dashboard_rows(now);
        self.dashboard_items_for(self.dashboard.selected_row(&rows), now)
    }

    /// What `row` lists: a queue's items, a contributor's activity, a
    /// card's pipelines, milestones or releases.
    pub fn dashboard_items_for(
        &self,
        row: Option<&DashboardRow>,
        now: Now,
    ) -> Vec<DashboardItem<'_>> {
        let signed_in = self.dashboard_signed_in_user();
        let person = self.dashboard_person();
        let (issues, mrs) = (&self.issues.items, &self.mrs.items);
        let contributions = self.dashboard.contributions.contributions();
        let since = self.dashboard.contributors_view.activity_since(now);
        match row {
            Some(DashboardRow::Mr(queue)) => queue
                .list(mrs, person, signed_in, now)
                .into_iter()
                .map(|mr| DashboardItem::Activity(Activity::MergeRequest(mr)))
                .collect(),
            Some(DashboardRow::Issue(queue)) => queue
                .list(issues, person, now)
                .into_iter()
                .map(|issue| DashboardItem::Activity(Activity::Issue(issue)))
                .collect(),
            Some(DashboardRow::Contributor(contributor)) => recent_activity(
                issues,
                mrs,
                contributions,
                contributor.resolve(signed_in),
                since,
            )
            .into_iter()
            .map(DashboardItem::Activity)
            .collect(),
            Some(DashboardRow::Card(DashboardCard::Pipelines)) => self
                .pipelines
                .items
                .iter()
                .map(DashboardItem::Pipeline)
                .collect(),
            Some(DashboardRow::Card(DashboardCard::Milestones)) => dashboard::upcoming_milestones(
                &self.milestones.items,
                |iid| self.milestone_progress(iid),
                now.today,
                usize::MAX,
            )
            .into_iter()
            .map(DashboardItem::Milestone)
            .collect(),
            Some(DashboardRow::Card(DashboardCard::Releases)) => self
                .releases
                .items()
                .iter()
                .map(DashboardItem::Release)
                .collect(),
            Some(DashboardRow::Card(_)) | None => {
                recent_activity(issues, mrs, contributions, person, since)
                    .into_iter()
                    .map(DashboardItem::Activity)
                    .collect()
            }
        }
    }

    /// The Contributors card's numbers for `person` in its current view.
    /// `None` for the signed-in user while their login is unknown.
    pub fn contributor_stats(
        &self,
        person: &DashboardPerson,
        now: Now,
    ) -> Option<[usize; ContributorsView::COLUMN_COUNT]> {
        let signed_in = self.dashboard_signed_in_user();
        if *person == DashboardPerson::Me && signed_in.is_none() {
            return None;
        }
        let person = person.resolve(signed_in);
        Some(match self.dashboard.contributors_view {
            ContributorsView::OpenWork => {
                let load = dashboard::workload(
                    &self.issues.items,
                    &self.mrs.items,
                    person,
                    signed_in,
                    now,
                );
                [
                    load.open_mrs,
                    load.reviews,
                    load.open_issues,
                    load.overdue_issues,
                ]
            }
            ContributorsView::Activity(window) => {
                let counts = dashboard::activity_counts(
                    self.dashboard.contributions.contributions(),
                    person,
                    window.starts_at(now),
                );
                [
                    counts.commits,
                    counts.comments,
                    counts.threads,
                    counts.reviews,
                ]
            }
        })
    }

    /// Contributors other than the signed-in user, by login, or by the
    /// sort column with the login breaking ties.
    fn sorted_contributors(&self, now: Now) -> Vec<&str> {
        let mut logins = dashboard::contributor_logins(
            &self.issues.items,
            &self.mrs.items,
            self.dashboard.contributions.contributions(),
            self.dashboard_signed_in_user(),
        );
        let Some(sort) = self.dashboard.contributors_sort else {
            return logins;
        };
        logins.sort_by_cached_key(|login| {
            let value = self
                .contributor_stats(&DashboardPerson::User(login.to_string()), now)
                .map_or(0, |stats| stats[sort.column]);
            let key = if sort.is_descending {
                usize::MAX - value
            } else {
                value
            };
            (key, login.to_string())
        });
        logins
    }

    /// `cycle_window`: count open work, then the last day, week and month
    /// of activity.
    pub fn cycle_contributors_view(&mut self) {
        self.dashboard.contributors_view = self.dashboard.contributors_view.next();
    }

    /// `>` / `<` on the Dashboard: sort contributors by the next or previous
    /// column, highest first, with "by login" between the last and the first.
    pub fn cycle_contributors_sort(&mut self, forward: bool) {
        let positions = ContributorsView::COLUMN_COUNT + 1;
        let current = self
            .dashboard
            .contributors_sort
            .map_or(0, |sort| sort.column + 1);
        let next = if forward {
            (current + 1) % positions
        } else {
            (current + positions - 1) % positions
        };
        self.dashboard.contributors_sort = next.checked_sub(1).map(|column| ContributorSort {
            column,
            is_descending: true,
        });
    }

    /// `=` on the Dashboard: flip the contributor sort, if any.
    pub fn reverse_contributors_sort(&mut self) {
        if let Some(sort) = &mut self.dashboard.contributors_sort {
            sort.is_descending = !sort.is_descending;
        }
    }

    pub fn dashboard_selected_row(&self, now: Now) -> Option<DashboardRow> {
        let rows = self.dashboard_rows(now);
        self.dashboard.selected_row(&rows).cloned()
    }

    fn dashboard_selected_card(&self, now: Now) -> Option<DashboardCard> {
        let rows = self.dashboard_rows(now);
        self.dashboard.selected_card(&rows)
    }

    /// `j`/`k`: the card below or above, the next or previous row of the
    /// card, or item of the list, depending on the focus.
    pub fn step_dashboard(&mut self, forward: bool) {
        let now = self.dashboard_now();
        match self.dashboard.focus {
            DashboardFocus::Tab => {}
            DashboardFocus::Cards => {
                let rows = self.dashboard_rows(now);
                let step = if forward {
                    CardStep::Down
                } else {
                    CardStep::Up
                };
                self.dashboard.step_card(&rows, step);
            }
            DashboardFocus::Rows => {
                let rows = self.dashboard_rows(now);
                self.dashboard.step_row(&rows, forward);
            }
            DashboardFocus::List => {
                let len = self.dashboard_items(now).len();
                self.dashboard.step_list(len, forward);
            }
        }
    }

    /// `h`/`l` while the cards have the keys. Returns whether the Dashboard
    /// took the key: everywhere below the tab level, so a drill-down is not
    /// left for another tab by accident.
    pub fn step_dashboard_sideways(&mut self, step: CardStep) -> bool {
        match self.dashboard.focus {
            DashboardFocus::Tab => false,
            DashboardFocus::Cards => {
                let rows = self.dashboard_rows(self.dashboard_now());
                self.dashboard.step_card(&rows, step);
                true
            }
            DashboardFocus::Rows | DashboardFocus::List => true,
        }
    }

    pub fn select_first_on_dashboard(&mut self) {
        self.select_edge_on_dashboard(false);
    }

    pub fn select_last_on_dashboard(&mut self) {
        self.select_edge_on_dashboard(true);
    }

    fn select_edge_on_dashboard(&mut self, is_last: bool) {
        let now = self.dashboard_now();
        match self.dashboard.focus {
            DashboardFocus::Tab => {}
            DashboardFocus::Cards => {
                let rows = self.dashboard_rows(now);
                self.dashboard.select_edge_card(&rows, is_last);
            }
            DashboardFocus::Rows => {
                let rows = self.dashboard_rows(now);
                let Some(card) = self.dashboard.selected_card(&rows) else {
                    return;
                };
                let mut in_card = rows.iter().filter(|row| row.card() == card);
                let row = if is_last {
                    in_card.next_back()
                } else {
                    in_card.next()
                };
                self.dashboard.select_row(row);
            }
            DashboardFocus::List => {
                self.dashboard.list_cursor = if is_last {
                    self.dashboard_items(now).len().saturating_sub(1)
                } else {
                    0
                };
            }
        }
    }

    /// `Enter` on the Dashboard: from the tab into the cards, from a card
    /// into its queues or people (or straight to its list when it has none),
    /// from a queue or person into its list, and from a listed item out to
    /// it in its tab. A person also becomes the one the cards count for.
    pub fn enter_dashboard(&mut self) -> DashboardAction {
        let now = self.dashboard_now();
        match self.dashboard.focus {
            DashboardFocus::Tab => {
                self.dashboard.focus = DashboardFocus::Cards;
                DashboardAction::Stay
            }
            DashboardFocus::Cards => {
                if self
                    .dashboard_selected_card(now)
                    .is_some_and(DashboardCard::has_rows)
                {
                    self.dashboard.focus = DashboardFocus::Rows;
                } else {
                    self.open_dashboard_list(now);
                }
                DashboardAction::Stay
            }
            DashboardFocus::Rows => {
                if let Some(row) = self.dashboard_selected_row(now)
                    && let DashboardRow::Contributor(person) = &row
                {
                    self.dashboard.person = person.clone();
                    self.dashboard.select_row(Some(&row));
                }
                self.open_dashboard_list(now);
                DashboardAction::Stay
            }
            DashboardFocus::List => self.open_dashboard_item(now),
        }
    }

    fn open_dashboard_list(&mut self, now: Now) {
        if self.dashboard_items(now).is_empty() {
            self.show_notice("Nothing to list here".to_string());
        } else {
            self.dashboard.focus_list();
        }
    }

    fn open_dashboard_item(&mut self, now: Now) -> DashboardAction {
        let items = self.dashboard_items(now);
        let cursor = self
            .dashboard
            .list_cursor
            .min(items.len().saturating_sub(1));
        let action = match items.get(cursor) {
            Some(DashboardItem::Activity(Activity::Issue(issue))) => DashboardAction::OpenIssue {
                project: self.scope.project_of(&issue.project_path).to_string(),
                iid: issue.iid,
            },
            Some(DashboardItem::Activity(Activity::MergeRequest(mr))) => DashboardAction::OpenMr {
                project: self.scope.project_of(&mr.project_path).to_string(),
                iid: mr.iid,
            },
            Some(DashboardItem::Activity(Activity::Contribution(contribution))) => {
                match &contribution.target {
                    Some(target) => {
                        let project = self.scope.as_str().to_string();
                        let iid = target.iid;
                        match target.kind {
                            TargetKind::Issue => DashboardAction::OpenIssue { project, iid },
                            TargetKind::MergeRequest => DashboardAction::OpenMr { project, iid },
                        }
                    }
                    None => DashboardAction::Stay,
                }
            }
            Some(DashboardItem::Pipeline(pipeline)) => DashboardAction::OpenPipeline(pipeline.id),
            Some(DashboardItem::Milestone(progress)) => {
                DashboardAction::OpenMilestone(progress.milestone.iid)
            }
            Some(DashboardItem::Release(release)) => {
                DashboardAction::OpenRelease(release.tag_name.clone())
            }
            None => DashboardAction::Stay,
        };
        let is_commit = matches!(
            items.get(cursor),
            Some(DashboardItem::Activity(Activity::Contribution(contribution)))
                if contribution.target.is_none()
        );
        if is_commit {
            self.show_notice("A commit has no issue or MR/PR to open".to_string());
        }
        action
    }

    /// `Esc` on the Dashboard: one level up. Returns false at the tab level,
    /// where `Esc` keeps its other meanings.
    pub fn leave_dashboard_level(&mut self) -> bool {
        self.dashboard.focus = match self.dashboard.focus {
            DashboardFocus::Tab => return false,
            DashboardFocus::Cards => DashboardFocus::Tab,
            DashboardFocus::Rows => DashboardFocus::Cards,
            DashboardFocus::List => {
                let now = self.dashboard_now();
                if self
                    .dashboard_selected_card(now)
                    .is_some_and(DashboardCard::has_rows)
                {
                    DashboardFocus::Rows
                } else {
                    DashboardFocus::Cards
                }
            }
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Five cards on a grid two wide: MRs and Contributors with rows, then
    /// Pipelines, Milestones and Releases.
    fn rows() -> Vec<DashboardRow> {
        vec![
            DashboardRow::Mr(MrQueue::Open),
            DashboardRow::Mr(MrQueue::Drafts),
            DashboardRow::Contributor(DashboardPerson::Everyone),
            DashboardRow::Contributor(DashboardPerson::User("ada".to_string())),
            DashboardRow::Card(DashboardCard::Pipelines),
            DashboardRow::Card(DashboardCard::Milestones),
            DashboardRow::Card(DashboardCard::Releases),
        ]
    }

    fn grid_state() -> DashboardState {
        let state = DashboardState::default();
        state.grid_columns.set(2);
        state
    }

    #[test]
    fn row_steps_stay_inside_the_selected_card_and_wrap() {
        let rows = rows();
        let mut state = grid_state();
        assert_eq!(state.selected_row(&rows), Some(&rows[0]));

        state.step_row(&rows, false);
        assert_eq!(state.selected_row(&rows), Some(&rows[1]));
        state.step_row(&rows, true);
        assert_eq!(state.selected_row(&rows), Some(&rows[0]));
    }

    #[test]
    fn card_steps_follow_the_grid_and_stop_at_its_edges() {
        let rows = rows();
        let mut state = grid_state();
        let card = |state: &DashboardState| state.selected_card(&rows);

        state.step_card(&rows, CardStep::Left);
        state.step_card(&rows, CardStep::Up);
        assert_eq!(card(&state), Some(DashboardCard::MergeRequests));
        state.step_card(&rows, CardStep::Right);
        assert_eq!(card(&state), Some(DashboardCard::Contributors));
        state.step_card(&rows, CardStep::Right);
        assert_eq!(card(&state), Some(DashboardCard::Contributors));
        state.step_card(&rows, CardStep::Down);
        assert_eq!(card(&state), Some(DashboardCard::Milestones));
        state.step_card(&rows, CardStep::Down);
        assert_eq!(
            card(&state),
            Some(DashboardCard::Releases),
            "down from a line with no card below lands on the last card"
        );
        state.step_card(&rows, CardStep::Up);
        assert_eq!(card(&state), Some(DashboardCard::Pipelines));
    }

    #[test]
    fn each_card_remembers_its_row_across_card_moves() {
        let rows = rows();
        let mut state = grid_state();
        state.step_row(&rows, true);
        state.step_card(&rows, CardStep::Right);
        assert_eq!(state.selected_row(&rows), Some(&rows[2]));
        state.step_card(&rows, CardStep::Left);
        assert_eq!(state.selected_row(&rows), Some(&rows[1]));
    }

    #[test]
    fn a_picked_contributor_who_left_the_rows_falls_back_to_the_cards_first_row() {
        let mut rows = rows();
        let mut state = grid_state();
        state.select_row(Some(&rows[3].clone()));
        rows.remove(3);

        assert_eq!(state.selected_row(&rows), Some(&rows[2]));
    }

    #[test]
    fn enter_drills_down_level_by_level_and_esc_climbs_back() {
        let mut app = App::default();
        app.config.dashboard.enabled = true;
        app.dashboard.grid_columns.set(3);
        assert_eq!(app.dashboard.focus, DashboardFocus::Tab);
        assert!(!app.leave_dashboard_level());

        app.enter_dashboard();
        assert_eq!(app.dashboard.focus, DashboardFocus::Cards);
        app.enter_dashboard();
        assert_eq!(app.dashboard.focus, DashboardFocus::Rows);

        assert!(app.leave_dashboard_level());
        assert_eq!(app.dashboard.focus, DashboardFocus::Cards);
        assert!(app.leave_dashboard_level());
        assert_eq!(app.dashboard.focus, DashboardFocus::Tab);
    }

    #[test]
    fn a_card_without_rows_lists_its_own_items() {
        use crate::domain::pipelines::Pipeline;
        let mut app = App::default();
        app.config.dashboard.enabled = true;
        app.pipelines.items = vec![
            serde_json::from_value::<Pipeline>(serde_json::json!({
                "id": 41, "status": "failed", "ref": "main", "updated_at": "2026-10-28T00:00:00Z"
            }))
            .expect("valid pipeline"),
        ];
        let now = app.dashboard_now();
        let pipelines = DashboardRow::Card(DashboardCard::Pipelines);
        app.dashboard.select_row(Some(&pipelines));
        app.dashboard.focus = DashboardFocus::Cards;

        app.enter_dashboard();
        assert_eq!(app.dashboard.focus, DashboardFocus::List);
        assert!(matches!(
            app.dashboard_items(now).as_slice(),
            [DashboardItem::Pipeline(pipeline)] if pipeline.id == 41
        ));
        assert_eq!(app.enter_dashboard(), DashboardAction::OpenPipeline(41));
        app.dashboard.focus = DashboardFocus::List;
        assert!(app.leave_dashboard_level());
        assert_eq!(app.dashboard.focus, DashboardFocus::Cards);
    }

    #[test]
    fn the_configured_person_accepts_me_everyone_and_a_login() {
        assert_eq!(DashboardPerson::from_config("me"), DashboardPerson::Me);
        assert_eq!(
            DashboardPerson::from_config("Everyone"),
            DashboardPerson::Everyone
        );
        assert_eq!(
            DashboardPerson::from_config("@ada"),
            DashboardPerson::User("ada".to_string())
        );
    }

    #[test]
    fn the_dashboard_tab_is_listed_only_once_enabled() {
        let mut app = App::default();
        assert!(!app.available_tabs().contains(&Tab::Dashboard));
        app.config.dashboard.enabled = true;
        assert!(app.available_tabs().contains(&Tab::Dashboard));
        app.config.disabled_tabs = Some(vec!["dashboard".to_string()]);
        assert!(!app.available_tabs().contains(&Tab::Dashboard));
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
    fn contributors_list_by_login_until_sorted_by_a_column_highest_first() {
        use crate::domain::contributions::ContributionKind;
        let now = Now {
            instant: DateTime::parse_from_rfc3339("2026-10-29T12:00:00Z")
                .expect("valid time")
                .with_timezone(&Utc),
            today: chrono::NaiveDate::from_ymd_opt(2026, 10, 29).expect("valid date"),
            idle: dashboard::IdleDays::default(),
        };
        let commit = |author: &str| Contribution {
            author: author.to_string(),
            kind: ContributionKind::Commit,
            created_at: "2026-10-28T00:00:00Z".to_string(),
            summary: String::new(),
            target: None,
            location: None,
        };
        let mut app = App::default();
        app.dashboard.contributions = ContributionFeed::Loaded(ContributionHistory {
            contributions: vec![commit("bob"), commit("cy"), commit("cy")],
            cut_off_at: None,
        });
        app.dashboard.contributors_view = ContributorsView::Activity(ActivityWindow::Week);
        let contributors = |app: &App| -> Vec<String> {
            app.dashboard_rows(now)
                .into_iter()
                .filter_map(|row| match row {
                    DashboardRow::Contributor(DashboardPerson::User(login)) => Some(login),
                    _ => None,
                })
                .collect()
        };

        assert_eq!(contributors(&app), vec!["bob", "cy"]);
        app.cycle_contributors_sort(true);
        assert_eq!(
            app.contributor_stats(&DashboardPerson::User("cy".into()), now),
            Some([2, 0, 0, 0])
        );
        assert_eq!(contributors(&app), vec!["cy", "bob"]);
        app.reverse_contributors_sort();
        assert_eq!(contributors(&app), vec!["bob", "cy"]);
        app.cycle_contributors_sort(false);
        assert_eq!(app.dashboard.contributors_sort, None);
    }

    #[test]
    fn the_window_key_steps_from_open_work_through_day_week_and_month() {
        let mut view = ContributorsView::default();
        let mut seen = Vec::new();
        for _ in 0..ContributorsView::ALL.len() {
            view = view.next();
            seen.push(view);
        }
        assert_eq!(
            seen,
            vec![
                ContributorsView::Activity(ActivityWindow::Day),
                ContributorsView::Activity(ActivityWindow::Week),
                ContributorsView::Activity(ActivityWindow::Month),
                ContributorsView::OpenWork,
            ]
        );
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
