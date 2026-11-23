use crate::ui::compaction::compact;
use crate::ui::render_ctx::RenderCtx;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, HighlightSpacing, Paragraph, Row, Table, TableState, Wrap,
    },
};

use super::helpers::{
    PROGRESS_BAR_WIDTH, badge_style_for, progress_bar, sort_header_label, status_dot,
};
use crate::app::{App, StaleData, Tab};
use crate::backend::BackendKind;
use crate::config::{Icons, Theme};
use crate::dashboard::{
    ContributionFeed, ContributorsView, CountCut, DashboardCard, DashboardFocus, DashboardItem,
    DashboardPerson, DashboardRow,
};
use crate::domain::contributions::{
    Contribution, ContributionKind, ContributionTarget, TargetKind,
};
use crate::domain::dashboard::{
    self, Activity, ActivityWindow, IdleDays, IssueQueue, MilestoneProgress, MrQueue, Now, Person,
    PipelineOutcome,
};
use crate::domain::issues::Issue;
use crate::domain::mr::MergeRequest;
use crate::domain::pipelines::Pipeline;
use crate::domain::releases::Release;
use crate::scope::Scope;
use crate::utils::format::{format_ref, sanitize_untrusted, time_ago, truncate};
use std::ops::Range;

/// Rows of one card, borders included.
const CARD_HEIGHT: u16 = 9;
const CARD_BODY_LINES: usize = CARD_HEIGHT as usize - 2;
/// Below this many rows the item list is left out.
const LIST_MIN_HEIGHT: u16 = 3;
const COUNT_WIDTH: usize = 3;
/// One of the two longest fixed queue labels (with "opened by you,
/// unassigned"), which sets how narrow a card may get.
const BLOCKED_QUEUE_LABEL: &str = "conflict or rebase needed";
/// The marker, the count and the space-padded icon before a queue label.
const QUEUE_LABEL_INDENT: usize = 2 + COUNT_WIDTH + 4;
/// Columns a card needs to show `BLOCKED_QUEUE_LABEL`'s line inside its
/// borders.
const CARD_MIN_WIDTH: u16 = (QUEUE_LABEL_INDENT + BLOCKED_QUEUE_LABEL.len() + 2) as u16;
/// Each milestone takes a title line and a progress line.
const MILESTONES_SHOWN: usize = CARD_BODY_LINES / 2;
/// Width of one number column of the Contributors card.
const CONTRIBUTOR_COLUMN_CHARS: usize = 5;
const LIST_REF_WIDTH: u16 = 7;
/// GitHub run ids run to 11 digits and release tags can be long.
const LIST_REF_MAX_WIDTH: u16 = 14;
const LIST_AUTHOR_WIDTH: u16 = 16;
const LIST_PEOPLE_WIDTH: u16 = 20;
const LIST_NOTE_WIDTH: u16 = 24;
const LIST_UPDATED_WIDTH: u16 = 14;
const LIST_ICON_WIDTH: u16 = 2;
/// The item list's column gap.
const LIST_COLUMN_SPACING: u16 = 1;
/// The title keeps at least this many cells. To protect it, `updated`,
/// `note`, `people` and `author` are dropped, least useful first.
const LIST_TITLE_MIN_WIDTH: u16 = 20;

/// Who a label speaks about.
#[derive(Clone, Copy)]
enum Who<'a> {
    Everyone,
    You,
    Someone(&'a str),
}

impl<'a> Who<'a> {
    fn of(person: Person<'a>, signed_in: Option<&str>) -> Self {
        match person {
            Person::Everyone => Who::Everyone,
            Person::User(login) if Some(login) == signed_in => Who::You,
            Person::User(login) => Who::Someone(login),
        }
    }

    fn card_suffix(self) -> String {
        match self {
            Who::Everyone => "everyone".to_string(),
            Who::You => "you".to_string(),
            Who::Someone(login) => format!("@{}", sanitize_untrusted(login)),
        }
    }
}

struct Context<'a> {
    app: &'a App,
    theme: &'a Theme,
    icons: &'a Icons,
    kind: BackendKind,
    now: Now,
    signed_in: Option<&'a str>,
    person: Person<'a>,
    rows: Vec<DashboardRow>,
    selected: Option<DashboardRow>,
    selected_card: Option<DashboardCard>,
    focus: DashboardFocus,
}

impl Context<'_> {
    fn who(&self) -> Who<'_> {
        Who::of(self.person, self.signed_in)
    }

    /// Whether `row` has the keys: picked while its card's rows are walked.
    fn has_keys(&self, row: &DashboardRow) -> bool {
        self.is_selected(row) && self.focus == DashboardFocus::Rows
    }

    /// Whether `card` has the keys: selected while the cards or its rows
    /// are walked.
    fn is_card_focused(&self, card: DashboardCard) -> bool {
        self.selected_card == Some(card)
            && matches!(self.focus, DashboardFocus::Cards | DashboardFocus::Rows)
    }

    fn is_selected(&self, row: &DashboardRow) -> bool {
        self.selected.as_ref() == Some(row)
    }

    /// The marker and background of a selectable line.
    fn row_prefix(&self, row: &DashboardRow) -> (Span<'static>, Style) {
        if !self.is_selected(row) {
            return (Span::raw("  "), Style::default());
        }
        let marker = Span::styled(
            format!("{} ", self.icons.highlight_arrow),
            Style::default()
                .fg(self.theme.border_focused)
                .add_modifier(Modifier::BOLD),
        );
        let background = if self.has_keys(row) {
            Style::default().bg(self.theme.highlight_bg)
        } else {
            Style::default()
        };
        (marker, background)
    }
}

pub(crate) fn render_tab_dashboard(
    f: &mut Frame,
    render: &RenderCtx<'_>,
    app: &App,
    area: Rect,
    block: Block,
) {
    let inner = block.inner(area);
    f.render_widget(block, area);

    let now = app.dashboard_now();
    let rows = app.dashboard_rows(now);
    let selected = app.dashboard.selected_row(&rows).cloned();
    let selected_card = app.dashboard.selected_card(&rows);
    let ctx = Context {
        app,
        theme: &render.theme,
        icons: render.icons,
        kind: render.kind,
        now,
        signed_in: app.dashboard_signed_in_user(),
        person: app.dashboard_person(),
        rows,
        selected,
        selected_card,
        focus: app.dashboard.focus,
    };
    let cards = app.dashboard_cards();
    let grid = card_grid(inner, cards.len());
    app.dashboard.grid_columns.set(grid.columns);
    if ctx.focus == DashboardFocus::List && !grid.shows_list {
        render_item_list(f, &ctx, inner);
        return;
    }

    let card_lines: Vec<&[DashboardCard]> = cards.chunks(grid.columns).collect();
    let grid_height = if grid.shows_list {
        card_rows(card_lines.len())
    } else {
        inner.height
    };
    let [grid_area, list_area] =
        Layout::vertical([Constraint::Length(grid_height), Constraint::Min(0)]).areas(inner);
    let selected_line = ctx
        .selected_card
        .and_then(|card| cards.iter().position(|c| *c == card))
        .map_or(0, |index| index / grid.columns);
    let shown = visible_card_lines(card_lines.len(), selected_line, grid_area.height);
    let row_areas =
        Layout::vertical(vec![Constraint::Length(CARD_HEIGHT); shown.len()]).split(grid_area);
    for (row_cards, row_area) in card_lines[shown].iter().zip(row_areas.iter()) {
        let column_areas = Layout::horizontal(vec![
            Constraint::Ratio(1, grid.columns as u32);
            grid.columns
        ])
        .split(*row_area);
        for (card, card_area) in row_cards.iter().zip(column_areas.iter()) {
            render_card(f, &ctx, *card, *card_area);
        }
    }

    if grid.shows_list {
        render_item_list(f, &ctx, list_area);
    }
}

/// How the cards share the tab: how many sit on a line, and whether the
/// item list keeps its rows below them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CardGrid {
    columns: usize,
    shows_list: bool,
}

#[derive(Clone, Copy)]
enum GridWidthRule {
    TwoColumns,
    OneColumn,
}

const GRID_WIDTH_RULES: &[GridWidthRule] = &[GridWidthRule::TwoColumns, GridWidthRule::OneColumn];

#[derive(Clone, Copy)]
enum GridHeightRule {
    DropList,
}

const GRID_HEIGHT_RULES: &[GridHeightRule] = &[GridHeightRule::DropList];

/// Three cards to a line while each keeps `CARD_MIN_WIDTH` columns, then two,
/// then one. The item list then keeps `LIST_MIN_HEIGHT` rows below the card
/// lines while they all fit, and is dropped otherwise. Fewer columns would
/// only add card lines, so the width alone picks them.
fn card_grid(inner: Rect, card_count: usize) -> CardGrid {
    let columns = compact(
        3,
        GRID_WIDTH_RULES,
        |columns: usize| inner.width / columns as u16 >= CARD_MIN_WIDTH,
        |_, rule| match rule {
            GridWidthRule::TwoColumns => 2,
            GridWidthRule::OneColumn => 1,
        },
    );
    let cards_height = card_rows(card_count.div_ceil(columns));
    compact(
        CardGrid {
            columns,
            shows_list: true,
        },
        GRID_HEIGHT_RULES,
        |grid| {
            let list_height = if grid.shows_list { LIST_MIN_HEIGHT } else { 0 };
            cards_height.saturating_add(list_height) <= inner.height
        },
        |grid, GridHeightRule::DropList| CardGrid {
            shows_list: false,
            ..grid
        },
    )
}

fn card_rows(card_lines: usize) -> u16 {
    u16::try_from(card_lines)
        .unwrap_or(u16::MAX)
        .saturating_mul(CARD_HEIGHT)
}

/// The card lines drawn in `height` rows: all of them when they fit, else
/// as many as fit (at least one, clipped) around `selected_line`, so the
/// card `step_card` moves to is always on screen.
fn visible_card_lines(total: usize, selected_line: usize, height: u16) -> Range<usize> {
    let shown = usize::from(height / CARD_HEIGHT).max(1).min(total);
    let first = selected_line
        .saturating_sub(shown.saturating_sub(1))
        .min(total - shown);
    first..first + shown
}

/// What a card can say about its tab's list.
#[derive(Debug, PartialEq, Eq)]
enum ListAvailability<'a> {
    Ready,
    Loading,
    Failed(&'a str),
    NotLoaded,
    /// A group or org has no activity feed to count.
    NoActivityFeed,
}

fn list_availability(app: &App, tab: Tab) -> ListAvailability<'_> {
    if app.has_rows(tab) {
        return ListAvailability::Ready;
    }
    if app.loading_tabs.contains(&tab) || app.loading_tabs.contains(&Tab::Dashboard) {
        return ListAvailability::Loading;
    }
    if let Some(reason) = app.fetch_failures.get(&tab) {
        return ListAvailability::Failed(reason);
    }
    if app.loaded_tabs.contains(&tab) {
        ListAvailability::Ready
    } else {
        ListAvailability::NotLoaded
    }
}

/// The open-work view counts the MR/PR and Issues lists, the activity views
/// the feed, so the card waits for or explains what its view reads instead
/// of drawing every contributor as zero.
fn contributors_availability(app: &App) -> ListAvailability<'_> {
    match (
        app.dashboard.contributors_view,
        &app.dashboard.contributions,
    ) {
        (ContributorsView::OpenWork, _) => open_work_availability(app),
        (_, ContributionFeed::Loaded(_)) => ListAvailability::Ready,
        _ if !app.scope.is_repository() => ListAvailability::NoActivityFeed,
        _ if app.loading_tabs.contains(&Tab::Dashboard) => ListAvailability::Loading,
        (_, ContributionFeed::Failed(reason)) => ListAvailability::Failed(reason),
        (_, ContributionFeed::NotLoaded) => ListAvailability::NotLoaded,
    }
}

/// Loading while a shown source loads with no rows. A failure only once no
/// source is ready, since one loaded list already has counts to show.
fn open_work_availability(app: &App) -> ListAvailability<'_> {
    let sources: Vec<ListAvailability> = app
        .dashboard_cards()
        .into_iter()
        .filter(|card| matches!(card, DashboardCard::MergeRequests | DashboardCard::Issues))
        .filter_map(DashboardCard::tab)
        .map(|tab| list_availability(app, tab))
        .collect();
    if sources.contains(&ListAvailability::Loading) {
        return ListAvailability::Loading;
    }
    if sources.is_empty() || sources.contains(&ListAvailability::Ready) {
        return ListAvailability::Ready;
    }
    sources
        .into_iter()
        .find(|source| matches!(source, ListAvailability::Failed(_)))
        .unwrap_or(ListAvailability::NotLoaded)
}

fn card_title(ctx: &Context, card: DashboardCard) -> Line<'static> {
    let theme = ctx.theme;
    let header = Style::default()
        .fg(theme.header_fg)
        .add_modifier(Modifier::BOLD);
    let mut title = match (card, card.tab()) {
        (DashboardCard::Contributors, _) | (_, None) => vec![Span::styled(
            format!(
                " {} Contributors · {} ",
                ctx.icons.user,
                contributors_view_label(ctx.app.dashboard.contributors_view)
            ),
            header,
        )],
        (DashboardCard::MergeRequests | DashboardCard::Issues, Some(tab)) => vec![Span::styled(
            format!(
                " {} · {} ",
                tab.title(ctx.kind, ctx.icons),
                ctx.who().card_suffix()
            ),
            header,
        )],
        (_, Some(tab)) => vec![Span::styled(
            format!(" {} ", tab.title(ctx.kind, ctx.icons)),
            header,
        )],
    };
    if let Some(stale) = card.tab().and_then(|tab| ctx.app.stale_tabs.get(&tab)) {
        title.push(Span::styled(
            match stale {
                StaleData::FromCache => " cached ",
                StaleData::RefreshFailed => " offline ",
            },
            Style::default()
                .bg(theme.red)
                .fg(theme.highlight_bg)
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(title)
}

/// Why a card's counts may be short. The notes sit on the bottom border,
/// which no heading or badge shares, so even a card at `CARD_MIN_WIDTH`
/// holds the longest of them whole.
fn card_note(ctx: &Context, card: DashboardCard) -> Option<Line<'static>> {
    let feed_note = match card {
        DashboardCard::Contributors => feed_cut_off_note(ctx),
        _ => None,
    };
    let notes: Vec<String> = feed_note
        .into_iter()
        .chain(ctx.app.dashboard_count_cut(card).map(count_cut_note))
        .collect();
    if notes.is_empty() {
        return None;
    }
    Some(Line::styled(
        format!(" {} ", notes.join(" ")),
        Style::default().fg(ctx.theme.yellow),
    ))
}

fn render_card(f: &mut Frame, ctx: &Context, card: DashboardCard, area: Rect) {
    let theme = ctx.theme;
    let has_keys = ctx.is_card_focused(card);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(if has_keys && ctx.focus == DashboardFocus::Cards {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(Style::default().fg(if has_keys {
            theme.border_focused
        } else {
            theme.border
        }))
        .title(card_title(ctx, card));
    if let Some(note) = card_note(ctx, card) {
        block = block.title_bottom(note);
    }
    let body_area = block.inner(area);
    let (body_width, body_height) = (usize::from(body_area.width), usize::from(body_area.height));

    let availability = match card {
        DashboardCard::Contributors => contributors_availability(ctx.app),
        _ => card.tab().map_or(ListAvailability::Ready, |tab| {
            list_availability(ctx.app, tab)
        }),
    };
    let paragraph = match availability {
        ListAvailability::Ready => Paragraph::new(card_body(ctx, card, body_width, body_height)),
        ListAvailability::Loading => Paragraph::new(Line::styled(
            format!("{} Loading...", ctx.icons.label_loading),
            Style::default().fg(theme.text_muted),
        )),
        ListAvailability::Failed(reason) => Paragraph::new(Line::styled(
            format!(
                "{} Failed to load: {}",
                ctx.icons.status_failed,
                sanitize_untrusted(reason)
            ),
            Style::default().fg(theme.red),
        ))
        .wrap(Wrap { trim: true }),
        ListAvailability::NotLoaded => Paragraph::new(Line::styled(
            format!(
                "Not loaded yet. Press {} to fetch.",
                ctx.app
                    .config
                    .keybindings
                    .display(crate::keymap::Action::GlobalRefresh)
            ),
            Style::default().fg(theme.text_muted),
        ))
        .wrap(Wrap { trim: true }),
        ListAvailability::NoActivityFeed => Paragraph::new(Line::styled(
            "No activity feed in this scope.",
            Style::default().fg(theme.text_muted),
        ))
        .wrap(Wrap { trim: true }),
    };
    f.render_widget(paragraph.block(block), area);
}

fn card_body(
    ctx: &Context,
    card: DashboardCard,
    width: usize,
    height: usize,
) -> Vec<Line<'static>> {
    match card {
        DashboardCard::MergeRequests => merge_requests_body(ctx),
        DashboardCard::Issues => issues_body(ctx),
        DashboardCard::Contributors => contributors_body(ctx, width, height),
        DashboardCard::Pipelines => pipelines_body(ctx, width),
        DashboardCard::Milestones => milestones_body(ctx),
        DashboardCard::Releases => releases_body(ctx),
    }
}

/// `▶ 12 ● label`: the count bold in `color` while non-zero, muted at zero.
fn queue_line(
    ctx: &Context,
    row: &DashboardRow,
    count: usize,
    icon: &str,
    label: String,
    color: Color,
) -> Line<'static> {
    let theme = ctx.theme;
    let (marker, background) = ctx.row_prefix(row);
    let count_style = if count == 0 {
        Style::default().fg(theme.text_muted)
    } else {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    };
    Line::from(vec![
        marker,
        Span::styled(format!("{count:>COUNT_WIDTH$} {icon} "), count_style),
        Span::styled(label, Style::default().fg(theme.text_normal)),
    ])
    .style(background)
}

fn mr_queue_label(queue: MrQueue, who: Who, idle: IdleDays) -> String {
    match (queue, who) {
        (MrQueue::Open, Who::Everyone) => "open".to_string(),
        (MrQueue::Open, Who::You) => "open, involving you".to_string(),
        (MrQueue::Open, Who::Someone(login)) => {
            format!("open, involving @{}", sanitize_untrusted(login))
        }
        (MrQueue::AwaitingReview, Who::Everyone) => "waiting for review".to_string(),
        (MrQueue::AwaitingReview, Who::You) => "waiting for your review".to_string(),
        (MrQueue::AwaitingReview, Who::Someone(login)) => {
            format!("waiting for @{}'s review", sanitize_untrusted(login))
        }
        (MrQueue::ChangesRequested, Who::Everyone) => "changes requested".to_string(),
        (MrQueue::ChangesRequested, Who::You) => "returned to you".to_string(),
        (MrQueue::ChangesRequested, Who::Someone(login)) => {
            format!("returned to @{}", sanitize_untrusted(login))
        }
        (MrQueue::ReadyToMerge, _) => "ready to merge".to_string(),
        (MrQueue::Blocked, _) => BLOCKED_QUEUE_LABEL.to_string(),
        (MrQueue::Idle, _) => format!("idle {}+ days", idle.mrs),
        (MrQueue::Drafts, _) => "drafts".to_string(),
    }
}

fn mr_queue_style<'a>(queue: MrQueue, ctx: &Context<'a>) -> (&'a str, Color) {
    let (icons, theme) = (ctx.icons, ctx.theme);
    match queue {
        MrQueue::Open => (&icons.state_open, theme.green),
        MrQueue::AwaitingReview => (&icons.workflow_review, theme.yellow),
        MrQueue::ChangesRequested => (&icons.workflow_returned, theme.yellow),
        MrQueue::ReadyToMerge => (&icons.approval_approved, theme.green),
        MrQueue::Blocked => (&icons.merge_conflict, theme.red),
        MrQueue::Idle => (&icons.clock, theme.yellow),
        MrQueue::Drafts => (&icons.status_draft, theme.text_normal),
    }
}

fn merge_requests_body(ctx: &Context) -> Vec<Line<'static>> {
    let mrs = &ctx.app.mrs.items;
    ctx.rows
        .iter()
        .filter_map(|row| match row {
            DashboardRow::Mr(queue) => Some(*queue),
            _ => None,
        })
        .map(|queue| {
            let (icon, color) = mr_queue_style(queue, ctx);
            queue_line(
                ctx,
                &DashboardRow::Mr(queue),
                queue.count(mrs, ctx.person, ctx.signed_in, ctx.now),
                icon,
                mr_queue_label(queue, ctx.who(), ctx.now.idle),
                color,
            )
        })
        .collect()
}

fn issue_queue_label(queue: IssueQueue, who: Who, idle: IdleDays) -> String {
    match (queue, who) {
        (IssueQueue::Open, Who::Everyone) => "open".to_string(),
        (IssueQueue::Open, Who::You) => "assigned to you".to_string(),
        (IssueQueue::Open, Who::Someone(login)) => {
            format!("assigned to @{}", sanitize_untrusted(login))
        }
        (IssueQueue::Overdue, _) => "overdue".to_string(),
        (IssueQueue::DueSoon, _) => format!("due within {} days", dashboard::DUE_SOON_DAYS),
        (IssueQueue::Idle, _) => format!("idle {}+ days", idle.issues),
        (IssueQueue::Unassigned, Who::Everyone) => "unassigned".to_string(),
        (IssueQueue::Unassigned, Who::You) => "opened by you, unassigned".to_string(),
        (IssueQueue::Unassigned, Who::Someone(login)) => {
            format!("opened by @{}, unassigned", sanitize_untrusted(login))
        }
    }
}

fn issue_queue_style<'a>(queue: IssueQueue, ctx: &Context<'a>) -> (&'a str, Color) {
    let (icons, theme) = (ctx.icons, ctx.theme);
    match queue {
        IssueQueue::Open => (&icons.state_open, theme.green),
        IssueQueue::Overdue => (&icons.status_failed, theme.red),
        IssueQueue::DueSoon => (&icons.status_pending, theme.yellow),
        IssueQueue::Idle => (&icons.clock, theme.yellow),
        IssueQueue::Unassigned => (&icons.user, theme.yellow),
    }
}

fn issues_body(ctx: &Context) -> Vec<Line<'static>> {
    let issues = &ctx.app.issues.items;
    ctx.rows
        .iter()
        .filter_map(|row| match row {
            DashboardRow::Issue(queue) => Some(*queue),
            _ => None,
        })
        .map(|queue| {
            let (icon, color) = issue_queue_style(queue, ctx);
            queue_line(
                ctx,
                &DashboardRow::Issue(queue),
                queue.count(issues, ctx.person, ctx.now),
                icon,
                issue_queue_label(queue, ctx.who(), ctx.now.idle),
                color,
            )
        })
        .collect()
}

fn contributors_body(ctx: &Context, width: usize, height: usize) -> Vec<Line<'static>> {
    let (theme, app) = (ctx.theme, ctx.app);
    let view = app.dashboard.contributors_view;
    let sort = app.dashboard.contributors_sort;
    let name_width = width
        .saturating_sub(2 + 2 + ContributorsView::COLUMN_COUNT * CONTRIBUTOR_COLUMN_CHARS)
        .max(4);
    let w = CONTRIBUTOR_COLUMN_CHARS;
    let columns = view.columns(ctx.kind);
    let sort_label = sort.map(|sort| (columns[sort.column], !sort.is_descending));
    let header: String = columns
        .iter()
        .map(|column| format!("{:>w$}", sort_header_label(column, sort_label, ctx.icons)))
        .collect();
    let mut lines = vec![Line::styled(
        format!("    {:<name_width$}{header}", "who"),
        Style::default().fg(theme.text_muted),
    )];

    let people: Vec<&DashboardRow> = ctx
        .rows
        .iter()
        .filter(|row| matches!(row, DashboardRow::Contributor(_)))
        .collect();
    let visible = height.saturating_sub(lines.len());
    let selected_index = people
        .iter()
        .position(|row| ctx.is_selected(row))
        .unwrap_or(0);
    let first = selected_index.saturating_sub(visible.saturating_sub(1));

    for row in people.iter().skip(first).take(visible) {
        let DashboardRow::Contributor(person) = row else {
            continue;
        };
        let (marker, background) = ctx.row_prefix(row);
        let is_shown = *person == app.dashboard.person;
        let shown_marker = if is_shown {
            format!("{} ", ctx.icons.check_on)
        } else {
            "  ".to_string()
        };
        let name = match person {
            DashboardPerson::Everyone => "everyone".to_string(),
            DashboardPerson::Me => match ctx.signed_in {
                Some(login) => format!("you (@{})", sanitize_untrusted(login)),
                None => "you (login unknown)".to_string(),
            },
            DashboardPerson::User(login) => format!("@{}", sanitize_untrusted(login)),
        };
        let stats = app
            .contributor_stats(person, ctx.now)
            .unwrap_or([None; ContributorsView::COLUMN_COUNT]);
        let numbers: String = stats
            .iter()
            .map(|value| match value {
                Some(value) => format!("{value:>w$}"),
                None => format!("{:>w$}", "-"),
            })
            .collect();
        let is_late = view
            .overdue_column()
            .and_then(|column| stats[column])
            .is_some_and(|overdue| overdue > 0);
        let late_color = if is_late {
            theme.red
        } else {
            theme.text_normal
        };
        lines.push(
            Line::from(vec![
                marker,
                Span::styled(shown_marker, Style::default().fg(theme.green)),
                Span::styled(
                    format!("{:<name_width$}", fit(&name, name_width)),
                    Style::default()
                        .fg(theme.text_normal)
                        .add_modifier(if is_shown {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(numbers, Style::default().fg(late_color)),
            ])
            .style(background),
        );
    }
    lines
}

fn contributors_view_label(view: ContributorsView) -> String {
    match view {
        ContributorsView::OpenWork => "open work".to_string(),
        ContributorsView::Activity(window) => activity_window_label(window),
    }
}

fn activity_window_label(window: ActivityWindow) -> String {
    match window.days() {
        1 => "last 24h".to_string(),
        days => format!("last {days} days"),
    }
}

/// Why activity counts may be short: the feed ended, or a page of it failed,
/// inside the window.
fn feed_cut_off_note(ctx: &Context) -> Option<String> {
    let since = ctx.app.dashboard.contributors_view.activity_since(ctx.now);
    let ContributorsView::Activity(_) = ctx.app.dashboard.contributors_view else {
        return None;
    };
    let feed = &ctx.app.dashboard.contributions;
    let cut_off_at = feed.cut_off_after(since)?;
    let what = if feed.read_error().is_some() {
        "read failed past"
    } else {
        "feed ends"
    };
    Some(format!("({what} {})", time_ago(cut_off_at)))
}

/// Why a card's counts may be short.
fn count_cut_note(cut: CountCut) -> String {
    match cut {
        CountCut::FetchLimit(limit) => format!("(newest {limit} only)"),
        CountCut::StateFilter => "(State filter hides open)".to_string(),
    }
}

fn pipelines_body(ctx: &Context, width: usize) -> Vec<Line<'static>> {
    let (theme, icons) = (ctx.theme, ctx.icons);
    let health = dashboard::assess_pipelines(ctx.app.root_pipelines());
    if health.recent.is_empty() {
        return vec![Line::styled(
            "No pipelines yet",
            Style::default().fg(theme.text_muted),
        )];
    }

    let rate = match health.success_rate_percent {
        Some(percent) => Span::styled(
            format!("{percent}% succeeded"),
            Style::default()
                .fg(match percent {
                    90.. => theme.green,
                    60.. => theme.yellow,
                    _ => theme.red,
                })
                .add_modifier(Modifier::BOLD),
        ),
        None => Span::styled("none finished", Style::default().fg(theme.text_muted)),
    };
    let mut lines = vec![
        Line::from(vec![
            rate,
            Span::styled(
                format!(" of the last {}", health.recent.len()),
                Style::default().fg(theme.text_muted),
            ),
        ]),
        Line::from(fitted_pipeline_dots(health.recent, icons, theme, width)),
        Line::from(vec![
            Span::styled(
                format!("{} running", health.in_progress),
                Style::default().fg(theme.blue),
            ),
            Span::styled(" · ", Style::default().fg(theme.text_muted)),
            Span::styled(
                format!("{} failed", health.failed),
                Style::default().fg(if health.failed == 0 {
                    theme.text_muted
                } else {
                    theme.red
                }),
            ),
        ]),
    ];
    for pipeline in &health.latest_runs {
        let (fg, _, _, badge) =
            badge_style_for("Status", &pipeline.status, false, theme.bg, theme, icons);
        let when = pipeline.created_at().unwrap_or(pipeline.updated_at());
        lines.push(Line::from(vec![
            Span::styled(
                badge.unwrap_or_else(|| format!(" {} ", pipeline.status)),
                Style::default().fg(fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                sanitize_untrusted(&run_line_label(pipeline, &ctx.app.scope)).into_owned(),
                Style::default().fg(theme.text_normal),
            ),
            Span::styled(
                format!(" {}", time_ago(when)),
                Style::default().fg(theme.text_muted),
            ),
        ]));
    }
    lines
}

/// The ref, the workflow when the run has one (GitHub) and, in a group or
/// org, the project in front: one ref can carry several run lines.
fn run_line_label(pipeline: &Pipeline, scope: &Scope) -> String {
    let mut label = format_ref(&pipeline.r#ref);
    if !pipeline.name.is_empty() {
        label = format!("{label} · {}", pipeline.name);
    }
    if scope.is_group() && !pipeline.project_path.is_empty() {
        label = format!("{} {label}", pipeline.project_path);
    }
    label
}

/// The newest runs that fit in `max_width` cells, oldest to newest left to
/// right: `recent` is newest first, and a dot's display width varies with the
/// icon set (the nerd font's dots are double-width emoji, ascii's are single
/// cells), so a fixed dot count can overflow or waste space.
fn fitted_pipeline_dots(
    recent: &[Pipeline],
    icons: &Icons,
    theme: &Theme,
    max_width: usize,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut used = 0;
    for pipeline in recent {
        let dot = status_dot(&pipeline.status, icons);
        let dot_width = Span::raw(dot).width();
        if used + dot_width > max_width {
            break;
        }
        used += dot_width;
        spans.push(Span::styled(
            dot.to_string(),
            Style::default().fg(outcome_color(&pipeline.status, theme)),
        ));
    }
    spans.reverse();
    spans
}

fn outcome_color(status: &str, theme: &Theme) -> Color {
    match PipelineOutcome::of(status) {
        PipelineOutcome::Succeeded => theme.pipeline_success,
        PipelineOutcome::Failed => theme.pipeline_failed,
        PipelineOutcome::InProgress => theme.pipeline_running,
        PipelineOutcome::Inconclusive => theme.pipeline_skipped,
    }
}

fn milestones_body(ctx: &Context) -> Vec<Line<'static>> {
    let theme = ctx.theme;
    let app = ctx.app;
    let upcoming = dashboard::upcoming_milestones(
        &app.milestones.items,
        |iid| app.milestone_progress(iid),
        ctx.now.today,
        MILESTONES_SHOWN,
    );
    if upcoming.is_empty() {
        return vec![Line::styled(
            "No open milestones",
            Style::default().fg(theme.text_muted),
        )];
    }
    upcoming
        .iter()
        .flat_map(|progress| milestone_lines(progress, theme, ctx.icons))
        .collect()
}

fn milestone_lines(
    progress: &MilestoneProgress,
    theme: &Theme,
    icons: &Icons,
) -> [Line<'static>; 2] {
    let (bar, bar_color) = match progress.issues {
        Some((closed, total)) => progress_bar(closed, total, theme, icons),
        None => ("no issues".to_string(), theme.text_muted),
    };
    let (due, due_color) = match progress.due_in_days {
        Some(days) if days < 0 => (format!(" {}d late", -days), theme.red),
        Some(0) => (" due today".to_string(), theme.yellow),
        Some(days) => (format!(" due in {days}d"), theme.text_muted),
        None => (String::new(), theme.text_muted),
    };
    [
        Line::styled(
            sanitize_untrusted(&progress.milestone.title).into_owned(),
            Style::default()
                .fg(theme.text_normal)
                .add_modifier(Modifier::BOLD),
        ),
        Line::from(vec![
            Span::styled(
                format!("{bar:<PROGRESS_BAR_WIDTH$}"),
                Style::default().fg(bar_color),
            ),
            Span::styled(due, Style::default().fg(due_color)),
        ]),
    ]
}

fn releases_body(ctx: &Context) -> Vec<Line<'static>> {
    let theme = ctx.theme;
    let releases = ctx.app.releases.items();
    let Some(latest) = releases.first() else {
        return vec![Line::styled(
            "No releases yet",
            Style::default().fg(theme.text_muted),
        )];
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(
            sanitize_untrusted(&latest.tag_name).into_owned(),
            Style::default()
                .fg(theme.green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", sanitize_untrusted(&latest.name)),
            Style::default().fg(theme.text_normal),
        ),
    ])];
    let released = match &latest.author_name {
        Some(author) => format!(
            "released {} by {}",
            time_ago(&latest.released_at),
            sanitize_untrusted(author)
        ),
        None => format!("released {}", time_ago(&latest.released_at)),
    };
    lines.push(Line::styled(
        released,
        Style::default().fg(theme.text_muted),
    ));
    if let Some(commit_title) = &latest.commit_title {
        lines.push(Line::styled(
            sanitize_untrusted(commit_title).into_owned(),
            Style::default().fg(theme.text_muted),
        ));
    }
    lines.push(Line::styled(
        format!("{} releases listed", releases.len()),
        Style::default().fg(theme.text_muted),
    ));
    lines
}

/// The title of the item list: what the selected row lists.
fn item_list_title(ctx: &Context) -> String {
    let window = activity_window_label(ctx.app.dashboard.contributors_view.activity_window());
    let activity_of = |who: Who| match who {
        Who::Everyone => format!("Everyone's activity, feed of the {window}"),
        Who::You => format!("Your activity, feed of the {window}"),
        Who::Someone(login) => {
            format!(
                "Activity of @{}, feed of the {window}",
                sanitize_untrusted(login)
            )
        }
    };
    let mut label = match &ctx.selected {
        Some(DashboardRow::Mr(queue)) => {
            return capitalized(mr_queue_label(*queue, ctx.who(), ctx.now.idle));
        }
        Some(DashboardRow::Issue(queue)) => {
            return capitalized(issue_queue_label(*queue, ctx.who(), ctx.now.idle));
        }
        Some(DashboardRow::Contributor(person)) => {
            activity_of(Who::of(person.resolve(ctx.signed_in), ctx.signed_in))
        }
        Some(DashboardRow::Card(DashboardCard::Pipelines)) => {
            return Tab::Pipelines.title(ctx.kind, ctx.icons);
        }
        Some(DashboardRow::Card(DashboardCard::Milestones)) => {
            return "Open milestones, soonest due first".to_string();
        }
        Some(DashboardRow::Card(DashboardCard::Releases)) => return "Releases".to_string(),
        Some(DashboardRow::Card(_)) | None => activity_of(ctx.who()),
    };
    if let ContributionFeed::Failed(reason) = &ctx.app.dashboard.contributions {
        label.push_str(&format!(
            " (no commits or comments: {})",
            sanitize_untrusted(reason)
        ));
    } else if let Some(reason) = ctx.app.dashboard.contributions.read_error() {
        label.push_str(&format!(
            " (activity cut short: {})",
            sanitize_untrusted(reason)
        ));
    }
    label
}

fn capitalized(label: String) -> String {
    let mut chars = label.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => label,
    }
}

fn render_item_list(f: &mut Frame, ctx: &Context, area: Rect) {
    let (theme, app) = (ctx.theme, ctx.app);
    let items = app.dashboard_items_for(ctx.selected.as_ref(), ctx.now);
    let is_focused = ctx.focus == DashboardFocus::List;
    let hint = match ctx.focus {
        DashboardFocus::Tab => " Enter navigate the cards ",
        DashboardFocus::Cards => " h/j/k/l move · Enter open card · Esc leave ",
        DashboardFocus::Rows => " j/k pick · Enter list · Esc cards ",
        DashboardFocus::List => " Enter open · Esc back ",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if is_focused {
            theme.border_focused
        } else {
            theme.border
        }))
        .title(Span::styled(
            format!(" {} · {} ", item_list_title(ctx), items.len()),
            Style::default()
                .fg(theme.header_fg)
                .add_modifier(Modifier::BOLD),
        ))
        .title(Line::styled(hint, Style::default().fg(theme.text_muted)).right_aligned());
    if items.is_empty() {
        f.render_widget(
            Paragraph::new(Line::styled(
                "Nothing to list",
                Style::default().fg(theme.text_muted),
            ))
            .block(block),
            area,
        );
        return;
    }

    let highlight_symbol = format!("{} ", ctx.icons.highlight_arrow);
    let columns = item_list_columns(
        block.inner(area).width,
        &highlight_symbol,
        reference_width(&items),
    );
    let rows = items.iter().map(|item| item_row(ctx, item, &columns));
    let table = Table::new(rows, columns.constraints())
        .block(block)
        .column_spacing(LIST_COLUMN_SPACING)
        .row_highlight_style(Style::default().bg(theme.highlight_bg))
        .highlight_symbol(highlight_symbol)
        .highlight_spacing(HighlightSpacing::Always);
    let mut state = TableState::default()
        .with_selected(is_focused.then(|| app.dashboard.list_cursor.min(items.len() - 1)));
    f.render_stateful_widget(table, area, &mut state);
}

/// With `HighlightSpacing::Always` the table reserves the highlight symbol's
/// width before it lays out the columns, so they fit in what is left.
fn item_list_columns(
    inner_width: u16,
    highlight_symbol: &str,
    reference_width: u16,
) -> ListColumns {
    let selection_width = u16::try_from(Span::raw(highlight_symbol).width()).unwrap_or(u16::MAX);
    ListColumns::fit(inner_width.saturating_sub(selection_width), reference_width)
}

/// The reference column fits the longest pipeline id or release tag listed.
fn reference_width(items: &[DashboardItem]) -> u16 {
    let widest = items
        .iter()
        .map(|item| match item {
            DashboardItem::Pipeline(pipeline) => format!("#{}", pipeline.id).len(),
            DashboardItem::Release(release) => release.tag_name.chars().count(),
            DashboardItem::Activity(_) | DashboardItem::Milestone(_) => 0,
        })
        .max()
        .unwrap_or(0);
    u16::try_from(widest)
        .unwrap_or(u16::MAX)
        .saturating_add(1)
        .clamp(LIST_REF_WIDTH, LIST_REF_MAX_WIDTH)
}

/// Which optional item-list columns fit alongside the icon, reference and
/// title: `author`, `people`, `note` and `updated` are dropped, least useful
/// first, until the title keeps at least `LIST_TITLE_MIN_WIDTH` cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ListColumns {
    reference_width: u16,
    show_author: bool,
    show_people: bool,
    show_note: bool,
    show_updated: bool,
}

/// The item-list column a compaction step gives up.
#[derive(Clone, Copy)]
enum ListColumnDrop {
    Updated,
    Note,
    People,
    Author,
}

const LIST_COLUMN_DROPS: &[ListColumnDrop] = &[
    ListColumnDrop::Updated,
    ListColumnDrop::Note,
    ListColumnDrop::People,
    ListColumnDrop::Author,
];

impl ListColumns {
    fn fit(available: u16, reference_width: u16) -> Self {
        compact(
            Self {
                reference_width,
                show_author: true,
                show_people: true,
                show_note: true,
                show_updated: true,
            },
            LIST_COLUMN_DROPS,
            |columns| columns.title_width(available) >= LIST_TITLE_MIN_WIDTH,
            |columns, drop| match drop {
                ListColumnDrop::Updated => Self {
                    show_updated: false,
                    ..columns
                },
                ListColumnDrop::Note => Self {
                    show_note: false,
                    ..columns
                },
                ListColumnDrop::People => Self {
                    show_people: false,
                    ..columns
                },
                ListColumnDrop::Author => Self {
                    show_author: false,
                    ..columns
                },
            },
        )
    }

    fn title_width(&self, available: u16) -> u16 {
        let mut used = LIST_ICON_WIDTH + self.reference_width;
        let mut count: u16 = 3;
        if self.show_author {
            used += LIST_AUTHOR_WIDTH;
            count += 1;
        }
        if self.show_people {
            used += LIST_PEOPLE_WIDTH;
            count += 1;
        }
        if self.show_note {
            used += LIST_NOTE_WIDTH;
            count += 1;
        }
        if self.show_updated {
            used += LIST_UPDATED_WIDTH;
            count += 1;
        }
        used += LIST_COLUMN_SPACING * (count - 1);
        available.saturating_sub(used)
    }

    fn constraints(&self) -> Vec<Constraint> {
        let mut constraints = vec![
            Constraint::Length(LIST_ICON_WIDTH),
            Constraint::Length(self.reference_width),
            Constraint::Fill(1),
        ];
        if self.show_author {
            constraints.push(Constraint::Length(LIST_AUTHOR_WIDTH));
        }
        if self.show_people {
            constraints.push(Constraint::Length(LIST_PEOPLE_WIDTH));
        }
        if self.show_note {
            constraints.push(Constraint::Length(LIST_NOTE_WIDTH));
        }
        if self.show_updated {
            constraints.push(Constraint::Length(LIST_UPDATED_WIDTH));
        }
        constraints
    }
}

fn item_row(ctx: &Context, item: &DashboardItem, columns: &ListColumns) -> Row<'static> {
    match item {
        DashboardItem::Activity(activity) => activity_row(ctx, activity, columns),
        DashboardItem::Pipeline(pipeline) => pipeline_row(ctx, pipeline, columns),
        DashboardItem::Milestone(progress) => milestone_row(ctx, progress, columns),
        DashboardItem::Release(release) => release_row(ctx, release, columns),
    }
}

/// One entry of the item list, in its columns.
struct ListRow<'a> {
    icon: String,
    color: Color,
    reference: String,
    title: String,
    author: &'a str,
    people: String,
    note: Vec<Span<'static>>,
    when: String,
}

fn list_row(ctx: &Context, row: ListRow, columns: &ListColumns) -> Row<'static> {
    let theme = ctx.theme;
    let mut cells = vec![
        Cell::from(row.icon).style(Style::default().fg(row.color)),
        Cell::from(row.reference).style(Style::default().fg(row.color)),
        Cell::from(row.title).style(Style::default().fg(theme.text_normal)),
    ];
    if columns.show_author {
        let author = if row.author.is_empty() {
            String::new()
        } else {
            format!("@{}", sanitize_untrusted(row.author))
        };
        cells.push(Cell::from(author).style(Style::default().fg(theme.text_muted)));
    }
    if columns.show_people {
        cells.push(Cell::from(row.people).style(Style::default().fg(theme.text_muted)));
    }
    if columns.show_note {
        cells.push(Cell::from(Line::from(row.note)));
    }
    if columns.show_updated {
        cells.push(Cell::from(row.when).style(Style::default().fg(theme.text_muted)));
    }
    Row::new(cells)
}

/// A run: its ref and commit, who triggered it, status and duration.
fn pipeline_row(ctx: &Context, pipeline: &Pipeline, columns: &ListColumns) -> Row<'static> {
    let (theme, icons) = (ctx.theme, ctx.icons);
    let color = outcome_color(&pipeline.status, theme);
    let commit = pipeline
        .commit
        .as_ref()
        .map(|commit| commit.title.as_str())
        .filter(|title| !title.is_empty())
        .unwrap_or(pipeline.display_title());
    let r#ref = sanitize_untrusted(&format_ref(&pipeline.r#ref)).into_owned();
    let title = if commit.is_empty() {
        r#ref
    } else {
        format!("{ref} · {}", sanitize_untrusted(commit))
    };
    let author = if pipeline.actor_login.is_empty() {
        pipeline
            .commit
            .as_ref()
            .map_or("", |commit| commit.author_name.as_str())
    } else {
        pipeline.actor_login.as_str()
    };
    let duration = pipeline
        .duration_seconds
        .map(|seconds| format!("{}m {}s", seconds / 60, seconds % 60))
        .unwrap_or_default();
    list_row(
        ctx,
        ListRow {
            icon: status_dot(&pipeline.status, icons).to_string(),
            color,
            reference: format!("#{}", pipeline.id),
            title,
            author,
            people: pipeline.status.clone(),
            note: vec![note_span(duration, theme.text_muted)],
            when: time_ago(pipeline.created_at().unwrap_or(pipeline.updated_at())),
        },
        columns,
    )
}

/// A milestone: its issue progress and due date.
fn milestone_row(
    ctx: &Context,
    progress: &MilestoneProgress,
    columns: &ListColumns,
) -> Row<'static> {
    let theme = ctx.theme;
    let [_, progress_line] = milestone_lines(progress, theme, ctx.icons);
    let milestone = progress.milestone;
    let issues = match progress.issues {
        Some((closed, total)) => format!("{closed}/{total} issues closed"),
        None => "no issues".to_string(),
    };
    list_row(
        ctx,
        ListRow {
            icon: ctx.icons.tab_milestone.clone(),
            color: theme.blue,
            reference: format!("%{}", milestone.iid),
            title: sanitize_untrusted(&milestone.title).into_owned(),
            author: "",
            people: issues,
            note: progress_line.spans,
            when: milestone.due_date.clone().unwrap_or_default(),
        },
        columns,
    )
}

/// A release: its tag, name and the first line of its notes.
fn release_row(ctx: &Context, release: &Release, columns: &ListColumns) -> Row<'static> {
    let theme = ctx.theme;
    let notes = release
        .description
        .as_deref()
        .and_then(|notes| notes.lines().map(str::trim).find(|line| !line.is_empty()))
        .map(|line| sanitize_untrusted(line).into_owned());
    let name = sanitize_untrusted(&release.name).into_owned();
    let title = match notes {
        Some(notes) => format!("{name} · {notes}"),
        None => name,
    };
    list_row(
        ctx,
        ListRow {
            icon: ctx.icons.tab_release.clone(),
            color: theme.green,
            reference: sanitize_untrusted(&release.tag_name).into_owned(),
            title,
            author: release.author_name.as_deref().unwrap_or(""),
            people: release
                .commit_title
                .as_deref()
                .map(|title| sanitize_untrusted(title).into_owned())
                .unwrap_or_default(),
            note: Vec::new(),
            when: time_ago(&release.released_at),
        },
        columns,
    )
}

fn activity_row(ctx: &Context, item: &Activity, columns: &ListColumns) -> Row<'static> {
    let (theme, icons) = (ctx.theme, ctx.icons);
    let mr_marker = ctx.kind.mr_reference_prefix();
    let (icon, color, reference, title, author, people, note) = match item {
        Activity::Issue(issue) => (
            &icons.tab_issue,
            theme.blue,
            format!("#{}", issue.iid),
            sanitize_untrusted(&issue.title).into_owned(),
            &issue.author.username,
            people_cell(
                issue.assignees.iter().map(|a| a.username.as_str()),
                "unassigned",
            ),
            issue_note(ctx, issue),
        ),
        Activity::MergeRequest(mr) => (
            &icons.tab_pr,
            theme.purple,
            format!("{mr_marker}{}", mr.iid),
            sanitize_untrusted(&mr.title).into_owned(),
            &mr.author.username,
            people_cell(
                mr.reviewers.iter().map(|r| r.username.as_str()),
                "no reviewer",
            ),
            mr_note(ctx, mr),
        ),
        Activity::Contribution(contribution) => {
            let (icon, color, kind) = match contribution.kind {
                ContributionKind::Commit => (&icons.commit, theme.green, "commit"),
                ContributionKind::Comment => (&icons.comment, theme.yellow, "comment"),
                ContributionKind::ThreadComment => {
                    (&icons.thread_unresolved, theme.yellow, "thread comment")
                }
                ContributionKind::Review => (&icons.action_review, theme.purple, "review"),
            };
            let reference = match &contribution.target {
                Some(target) if target.kind == TargetKind::MergeRequest => {
                    format!("{mr_marker}{}", target.iid)
                }
                Some(target) => format!("#{}", target.iid),
                None => String::new(),
            };
            let title = contribution_title(ctx.app, contribution);
            let location = contribution
                .location
                .as_deref()
                .map(|location| sanitize_untrusted(location).into_owned())
                .unwrap_or_default();
            (
                icon,
                color,
                reference,
                title,
                &contribution.author,
                kind.to_string(),
                vec![note_span(location, theme.text_muted)],
            )
        }
    };
    list_row(
        ctx,
        ListRow {
            icon: icon.clone(),
            color,
            reference,
            title,
            author,
            people,
            note,
            when: time_ago(item.updated_at()),
        },
        columns,
    )
}

/// A contribution's summary, followed by the title of the issue or MR/PR it
/// was made on when the feed or the listed items know it.
fn contribution_title(app: &App, contribution: &Contribution) -> String {
    let summary = sanitize_untrusted(&contribution.summary).into_owned();
    let on = contribution.target.as_ref().and_then(|target| {
        target
            .title
            .as_deref()
            .or_else(|| listed_title(app, target))
    });
    match on {
        Some(on) => format!("{summary} · on {}", sanitize_untrusted(on)),
        None => summary,
    }
}

fn listed_title<'a>(app: &'a App, target: &ContributionTarget) -> Option<&'a str> {
    match target.kind {
        TargetKind::Issue => app
            .issues
            .items
            .iter()
            .find(|issue| issue.iid == target.iid)
            .map(|issue| issue.title.as_str()),
        TargetKind::MergeRequest => app
            .mrs
            .items
            .iter()
            .find(|mr| mr.iid == target.iid)
            .map(|mr| mr.title.as_str()),
    }
}

/// `text` cut to at most `width` characters, its `...` included.
fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        truncate(text, width.saturating_sub(3))
    }
}

/// `@a, @b`, or `nobody` when there is no one.
fn people_cell<'a>(logins: impl Iterator<Item = &'a str>, nobody: &str) -> String {
    let listed: Vec<String> = logins
        .map(|login| format!("@{}", sanitize_untrusted(login)))
        .collect();
    if listed.is_empty() {
        nobody.to_string()
    } else {
        listed.join(", ")
    }
}

fn note_span(text: String, color: Color) -> Span<'static> {
    Span::styled(format!("{text} "), Style::default().fg(color))
}

fn idle_note(ctx: &Context, updated_at: &str, idle_days: u32) -> Option<Span<'static>> {
    dashboard::days_since(updated_at, ctx.now.instant)
        .filter(|days| *days >= i64::from(idle_days))
        .map(|days| note_span(format!("idle {days}d"), ctx.theme.yellow))
}

/// Why the MR/PR may need someone: closed state, merge blockers, review
/// state, draft, idleness.
fn mr_note(ctx: &Context, mr: &MergeRequest) -> Vec<Span<'static>> {
    let theme = ctx.theme;
    if mr.state != "opened" {
        return vec![note_span(mr.state.clone(), theme.text_muted)];
    }
    let mut notes = Vec::new();
    if let Some(mergeability) = &mr.mergeability {
        if mergeability.conflicts {
            notes.push(note_span("conflict".to_string(), theme.red));
        } else if mergeability.needs_rebase {
            notes.push(note_span("rebase".to_string(), theme.yellow));
        }
    }
    if let Some(approval) = &mr.approval {
        if approval.changes_requested {
            notes.push(note_span("changes requested".to_string(), theme.yellow));
        } else if approval.approved {
            notes.push(note_span("approved".to_string(), theme.green));
        }
    }
    if mr.draft {
        notes.push(note_span("draft".to_string(), theme.text_muted));
    }
    notes.extend(idle_note(ctx, &mr.updated_at, ctx.now.idle.mrs));
    notes
}

/// Why the issue may need someone: closed state, due date, idleness.
fn issue_note(ctx: &Context, issue: &Issue) -> Vec<Span<'static>> {
    let theme = ctx.theme;
    if issue.state != "opened" {
        return vec![note_span(issue.state.clone(), theme.text_muted)];
    }
    let mut notes = Vec::new();
    match dashboard::days_until(issue.due_date.as_deref(), ctx.now.today) {
        Some(days) if days < 0 => notes.push(note_span(format!("{}d late", -days), theme.red)),
        Some(0) => notes.push(note_span("due today".to_string(), theme.yellow)),
        Some(days) if days <= dashboard::DUE_SOON_DAYS => {
            notes.push(note_span(format!("due in {days}d"), theme.yellow));
        }
        Some(days) => notes.push(note_span(format!("due in {days}d"), theme.text_muted)),
        None => {}
    }
    notes.extend(idle_note(ctx, &issue.updated_at, ctx.now.idle.issues));
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::contributions::ContributionHistory;

    fn dashboard_app(scope: Scope) -> App {
        let mut app = App::default();
        app.config.dashboard.enabled = true;
        app.scope = scope;
        app
    }

    fn repository() -> Scope {
        Scope::Repository("group/project".to_string())
    }

    fn in_activity_view(mut app: App) -> App {
        app.dashboard.contributors_view = ContributorsView::Activity(ActivityWindow::Week);
        app
    }

    #[test]
    fn an_activity_view_in_a_group_says_there_is_no_feed() {
        let app = in_activity_view(dashboard_app(Scope::Group("group".to_string())));

        assert_eq!(
            contributors_availability(&app),
            ListAvailability::NoActivityFeed
        );
    }

    #[test]
    fn an_activity_view_waits_for_the_feed_then_shows_why_it_failed() {
        let mut app = in_activity_view(dashboard_app(repository()));
        assert_eq!(contributors_availability(&app), ListAvailability::NotLoaded);

        app.loading_tabs.insert(Tab::Dashboard);
        assert_eq!(contributors_availability(&app), ListAvailability::Loading);

        app.loading_tabs.clear();
        app.dashboard.contributions = ContributionFeed::Failed("HTTP 502".to_string());
        assert_eq!(
            contributors_availability(&app),
            ListAvailability::Failed("HTTP 502")
        );

        app.dashboard.contributions = ContributionFeed::Loaded(ContributionHistory::default());
        assert_eq!(contributors_availability(&app), ListAvailability::Ready);
    }

    #[test]
    fn open_work_waits_for_a_loading_list_and_counts_once_one_list_loaded() {
        let mut app = dashboard_app(repository());
        app.loading_tabs.insert(Tab::MergeRequests);
        assert_eq!(contributors_availability(&app), ListAvailability::Loading);

        app.loading_tabs.clear();
        app.fetch_failures
            .insert(Tab::Issues, "HTTP 500".to_string());
        assert_eq!(
            contributors_availability(&app),
            ListAvailability::Failed("HTTP 500")
        );

        app.loaded_tabs.insert(Tab::MergeRequests);
        assert_eq!(contributors_availability(&app), ListAvailability::Ready);
    }

    #[test]
    fn a_cards_count_note_fits_whole_on_its_bottom_border_at_the_minimum_width() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = dashboard_app(repository());
        app.gitlab_client = Some(crate::domain::client::GitlabClient {
            is_github: false,
            backend: crate::backend::create_backend(false),
            tx: None,
            page_size: 100,
            api_per_page: 100,
            show_read_notifications: false,
            opened_pipeline: None,
        });
        app.mrs.items = (1..=100)
            .map(|iid| {
                serde_json::from_value(serde_json::json!({
                    "iid": iid,
                    "title": format!("mr {iid}"),
                    "state": "opened",
                    "updated_at": "2026-10-28T00:00:00Z",
                    "author": { "username": "bob" },
                    "milestone": null,
                    "target_branch": "main",
                    "draft": false,
                    "description": null,
                }))
                .expect("a valid merge request")
            })
            .collect();
        let (theme, icons) = (Theme::default(), Icons::ascii());
        let now = app.dashboard_now();
        let ctx = Context {
            app: &app,
            theme: &theme,
            icons: &icons,
            kind: app.kind(),
            now,
            signed_in: None,
            person: Person::Everyone,
            rows: app.dashboard_rows(now),
            selected: None,
            selected_card: None,
            focus: DashboardFocus::Tab,
        };
        let mut terminal =
            Terminal::new(TestBackend::new(CARD_MIN_WIDTH, CARD_HEIGHT)).expect("a test terminal");

        terminal
            .draw(|f| render_card(f, &ctx, DashboardCard::Contributors, f.area()))
            .expect("the card draws");

        let buffer = terminal.backend().buffer();
        let row = |y: u16| -> String {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        };
        assert_eq!(row(0), "╭ @ Contributors · open work ──────╮");
        assert_eq!(row(CARD_HEIGHT - 1), "╰ (newest 100 only) ───────────────╯");
    }

    #[test]
    fn cards_sit_three_to_a_line_while_each_keeps_its_minimum_width() {
        assert_eq!(
            card_grid(Rect::new(0, 0, 3 * CARD_MIN_WIDTH, 30), 6),
            CardGrid {
                columns: 3,
                shows_list: true
            }
        );
        assert_eq!(
            card_grid(Rect::new(0, 0, 3 * CARD_MIN_WIDTH - 1, 30), 6),
            CardGrid {
                columns: 2,
                shows_list: true
            }
        );
    }

    #[test]
    fn an_80_by_24_terminal_stacks_the_cards_one_to_a_line_without_the_list() {
        assert_eq!(
            card_grid(Rect::new(0, 0, 56, 14), 6),
            CardGrid {
                columns: 1,
                shows_list: false
            }
        );
    }

    #[test]
    fn the_item_list_gives_way_when_the_card_lines_leave_it_too_few_rows() {
        let cards_height = 2 * CARD_HEIGHT;
        assert_eq!(
            card_grid(
                Rect::new(0, 0, 3 * CARD_MIN_WIDTH, cards_height + LIST_MIN_HEIGHT - 1),
                6
            ),
            CardGrid {
                columns: 3,
                shows_list: false
            }
        );
    }

    #[test]
    fn a_grid_taller_than_its_rows_scrolls_to_keep_the_selected_line_drawn() {
        assert_eq!(visible_card_lines(2, 1, 30), 0..2);
        assert_eq!(visible_card_lines(6, 4, 14), 4..5);
        assert_eq!(visible_card_lines(6, 5, 3 * CARD_HEIGHT), 3..6);
        assert_eq!(visible_card_lines(6, 0, CARD_HEIGHT - 4), 0..1);
    }

    #[test]
    fn the_item_list_drops_updated_first_to_keep_the_title_its_minimum() {
        let reference_width = 7;
        let all_columns = LIST_ICON_WIDTH
            + reference_width
            + LIST_AUTHOR_WIDTH
            + LIST_PEOPLE_WIDTH
            + LIST_NOTE_WIDTH
            + LIST_UPDATED_WIDTH
            + 6 * LIST_COLUMN_SPACING
            + LIST_TITLE_MIN_WIDTH;
        let every_column = ListColumns {
            reference_width,
            show_author: true,
            show_people: true,
            show_note: true,
            show_updated: true,
        };

        assert_eq!(ListColumns::fit(all_columns, reference_width), every_column);
        assert_eq!(
            ListColumns::fit(all_columns - 1, reference_width),
            ListColumns {
                show_updated: false,
                ..every_column
            }
        );
    }

    #[test]
    fn the_item_list_columns_leave_room_for_the_highlight_symbol() {
        let reference_width = 7;
        let all_columns = LIST_ICON_WIDTH
            + reference_width
            + LIST_AUTHOR_WIDTH
            + LIST_PEOPLE_WIDTH
            + LIST_NOTE_WIDTH
            + LIST_UPDATED_WIDTH
            + 6 * LIST_COLUMN_SPACING
            + LIST_TITLE_MIN_WIDTH;

        assert!(item_list_columns(all_columns + 2, "> ", reference_width).show_updated);
        assert!(!item_list_columns(all_columns, "> ", reference_width).show_updated);
    }

    #[test]
    fn a_contribution_without_a_title_in_the_feed_takes_the_listed_mr_title() {
        let mut app = App::default();
        app.mrs.items = vec![
            serde_json::from_value(serde_json::json!({
                "iid": 9,
                "title": "Add cache",
                "state": "opened",
                "updated_at": "2026-10-28T00:00:00Z",
                "author": { "username": "bob" },
                "milestone": null,
                "target_branch": "main",
                "draft": false,
                "description": null,
            }))
            .expect("a valid merge request"),
        ];
        let review_comment = |iid: u64, title: Option<&str>| Contribution {
            id: "1".to_string(),
            author: "cy".to_string(),
            kind: ContributionKind::ThreadComment,
            created_at: "2026-10-28T00:00:00Z".to_string(),
            made_at: chrono::DateTime::parse_from_rfc3339("2026-10-28T00:00:00Z")
                .expect("valid time")
                .with_timezone(&chrono::Utc),
            summary: "nit".to_string(),
            target: Some(ContributionTarget {
                kind: TargetKind::MergeRequest,
                iid,
                title: title.map(str::to_string),
            }),
            location: None,
        };

        assert_eq!(
            contribution_title(&app, &review_comment(9, None)),
            "nit · on Add cache"
        );
        assert_eq!(
            contribution_title(&app, &review_comment(9, Some("Add a cache"))),
            "nit · on Add a cache"
        );
        assert_eq!(contribution_title(&app, &review_comment(10, None)), "nit");
    }
}
