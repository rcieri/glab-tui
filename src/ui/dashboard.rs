use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, HighlightSpacing, Paragraph, Row, Table, TableState, Wrap,
    },
};

use super::helpers::{badge_style_for, progress_bar, sort_header_label, status_dot};
use crate::app::{App, StaleData, Tab};
use crate::backend::BackendKind;
use crate::config::{ICONS, Icons, THEME, Theme};
use crate::dashboard::{
    ContributionFeed, ContributorsView, DashboardCard, DashboardPerson, DashboardRow,
};
use crate::domain::contributions::{ContributionKind, TargetKind};
use crate::domain::dashboard::{
    self, Activity, ISSUE_IDLE_DAYS, IssueQueue, MR_IDLE_DAYS, MilestoneProgress, MrQueue, Now,
    Person, PipelineOutcome,
};
use crate::domain::issues::Issue;
use crate::domain::mr::MergeRequest;
use crate::utils::format::{format_ref, sanitize_untrusted, time_ago, truncate};

/// Rows of one card, borders included.
const CARD_HEIGHT: u16 = 9;
const CARD_BODY_LINES: usize = CARD_HEIGHT as usize - 2;
/// From this inner width on, cards sit three to a row instead of two.
const THREE_COLUMN_MIN_WIDTH: u16 = 120;
/// Below this many rows the item list is left out.
const LIST_MIN_HEIGHT: u16 = 3;
const COUNT_WIDTH: usize = 3;
/// Each milestone takes a title line and a progress line.
const MILESTONES_SHOWN: usize = CARD_BODY_LINES / 2;
/// Width of `[██████████] 100%`, the widest progress bar.
const PROGRESS_BAR_CHARS: usize = 17;
/// Width of one number column of the Contributors card.
const CONTRIBUTOR_COLUMN_CHARS: usize = 5;
const LIST_REF_WIDTH: u16 = 7;
const LIST_AUTHOR_WIDTH: u16 = 16;
const LIST_PEOPLE_WIDTH: u16 = 20;
const LIST_NOTE_WIDTH: u16 = 24;
const LIST_UPDATED_WIDTH: u16 = 14;

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
            Who::Someone(login) => format!("@{login}"),
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
}

impl Context<'_> {
    fn who(&self) -> Who<'_> {
        Who::of(self.person, self.signed_in)
    }

    /// Whether `row` has the keys: selected while the item list is not focused.
    fn has_keys(&self, row: &DashboardRow) -> bool {
        self.is_selected(row) && !self.app.dashboard.is_list_focused
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

pub(crate) fn render_tab_dashboard(f: &mut Frame, app: &App, area: Rect, block: Block) {
    let theme = THEME.read().unwrap();
    let icons = ICONS.read().unwrap();
    let inner = block.inner(area);
    f.render_widget(block, area);

    let now = Now::current();
    let rows = app.dashboard_rows(now);
    let selected = app.dashboard.selected_row(&rows).cloned();
    let ctx = Context {
        app,
        theme: &theme,
        icons: &icons,
        kind: app.kind(),
        now,
        signed_in: app.dashboard_signed_in_user(),
        person: app.dashboard_person(),
        rows,
        selected,
    };
    let cards = app.dashboard_cards();
    let columns = if inner.width >= THREE_COLUMN_MIN_WIDTH {
        3
    } else {
        2
    };
    let grid_rows = u16::try_from(cards.len().div_ceil(columns)).unwrap_or(u16::MAX);
    let [grid_area, list_area] = Layout::vertical([
        Constraint::Length(grid_rows.saturating_mul(CARD_HEIGHT)),
        Constraint::Min(0),
    ])
    .areas(inner);

    let row_areas = Layout::vertical(vec![
        Constraint::Length(CARD_HEIGHT);
        usize::from(grid_rows)
    ])
    .split(grid_area);
    for (row_cards, row_area) in cards.chunks(columns).zip(row_areas.iter()) {
        let column_areas = Layout::horizontal(vec![Constraint::Ratio(1, columns as u32); columns])
            .split(*row_area);
        for (card, card_area) in row_cards.iter().zip(column_areas.iter()) {
            render_card(f, &ctx, *card, *card_area);
        }
    }

    if list_area.height >= LIST_MIN_HEIGHT {
        render_item_list(f, &ctx, list_area);
    }
}

/// What a card can say about its tab's list.
enum ListAvailability<'a> {
    Ready,
    Loading,
    Failed(&'a str),
    NotLoaded,
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

fn card_title(ctx: &Context, card: DashboardCard) -> Line<'static> {
    let theme = ctx.theme;
    let header = Style::default()
        .fg(theme.header_fg)
        .add_modifier(Modifier::BOLD);
    let mut title = match (card, card.tab()) {
        (DashboardCard::Contributors, _) | (_, None) => {
            let mut spans = vec![Span::styled(
                format!(
                    " {} Contributors · {} ",
                    ctx.icons.user,
                    contributors_view_label(ctx.app.dashboard.contributors_view)
                ),
                header,
            )];
            if let Some(note) = feed_cut_off_note(ctx) {
                spans.push(Span::styled(
                    format!("{note} "),
                    Style::default().fg(theme.yellow),
                ));
            }
            spans
        }
        (DashboardCard::MergeRequests | DashboardCard::Issues, Some(tab)) => vec![Span::styled(
            format!(" {} · {} ", tab.title(ctx.kind), ctx.who().card_suffix()),
            header,
        )],
        (_, Some(tab)) => vec![Span::styled(format!(" {} ", tab.title(ctx.kind)), header)],
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

fn render_card(f: &mut Frame, ctx: &Context, card: DashboardCard, area: Rect) {
    let theme = ctx.theme;
    let has_keys = ctx
        .selected
        .as_ref()
        .is_some_and(|row| row.card() == card && ctx.has_keys(row));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if has_keys {
            theme.border_focused
        } else {
            theme.border
        }))
        .title(card_title(ctx, card));
    let body_width = usize::from(block.inner(area).width);

    let availability = match card {
        DashboardCard::Contributors => ListAvailability::Ready,
        _ => card.tab().map_or(ListAvailability::Ready, |tab| {
            list_availability(ctx.app, tab)
        }),
    };
    let paragraph = match availability {
        ListAvailability::Ready => Paragraph::new(card_body(ctx, card, body_width)),
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
                ctx.app.config.keybindings.global.refresh
            ),
            Style::default().fg(theme.text_muted),
        ))
        .wrap(Wrap { trim: true }),
    };
    f.render_widget(paragraph.block(block), area);
}

fn card_body(ctx: &Context, card: DashboardCard, width: usize) -> Vec<Line<'static>> {
    match card {
        DashboardCard::MergeRequests => merge_requests_body(ctx),
        DashboardCard::Issues => issues_body(ctx),
        DashboardCard::Contributors => contributors_body(ctx, width),
        DashboardCard::Pipelines => pipelines_body(ctx),
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

fn mr_queue_label(queue: MrQueue, who: Who) -> String {
    match (queue, who) {
        (MrQueue::Open, Who::Everyone) => "open".to_string(),
        (MrQueue::Open, Who::You) => "open, involving you".to_string(),
        (MrQueue::Open, Who::Someone(login)) => format!("open, involving @{login}"),
        (MrQueue::AwaitingReview, Who::Everyone) => "waiting for review".to_string(),
        (MrQueue::AwaitingReview, Who::You) => "waiting for your review".to_string(),
        (MrQueue::AwaitingReview, Who::Someone(login)) => format!("waiting for @{login}'s review"),
        (MrQueue::ChangesRequested, Who::Everyone) => "changes requested".to_string(),
        (MrQueue::ChangesRequested, Who::You) => "returned to you".to_string(),
        (MrQueue::ChangesRequested, Who::Someone(login)) => format!("returned to @{login}"),
        (MrQueue::ReadyToMerge, _) => "ready to merge".to_string(),
        (MrQueue::Blocked, _) => "conflict or rebase needed".to_string(),
        (MrQueue::Idle, _) => format!("idle {MR_IDLE_DAYS}+ days"),
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
    MrQueue::ALL
        .into_iter()
        .map(|queue| {
            let (icon, color) = mr_queue_style(queue, ctx);
            queue_line(
                ctx,
                &DashboardRow::Mr(queue),
                queue.count(mrs, ctx.person, ctx.signed_in, ctx.now),
                icon,
                mr_queue_label(queue, ctx.who()),
                color,
            )
        })
        .collect()
}

fn issue_queue_label(queue: IssueQueue, who: Who) -> String {
    match (queue, who) {
        (IssueQueue::Open, Who::Everyone) => "open".to_string(),
        (IssueQueue::Open, Who::You) => "assigned to you".to_string(),
        (IssueQueue::Open, Who::Someone(login)) => format!("assigned to @{login}"),
        (IssueQueue::Overdue, _) => "overdue".to_string(),
        (IssueQueue::DueSoon, _) => format!("due within {} days", dashboard::DUE_SOON_DAYS),
        (IssueQueue::Idle, _) => format!("idle {ISSUE_IDLE_DAYS}+ days"),
        (IssueQueue::Unassigned, Who::Everyone) => "unassigned".to_string(),
        (IssueQueue::Unassigned, Who::You) => "opened by you, unassigned".to_string(),
        (IssueQueue::Unassigned, Who::Someone(login)) => {
            format!("opened by @{login}, unassigned")
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
    IssueQueue::ALL
        .into_iter()
        .filter(|queue| ctx.kind.is_gitlab() || !queue.needs_due_dates())
        .map(|queue| {
            let (icon, color) = issue_queue_style(queue, ctx);
            queue_line(
                ctx,
                &DashboardRow::Issue(queue),
                queue.count(issues, ctx.person, ctx.now),
                icon,
                issue_queue_label(queue, ctx.who()),
                color,
            )
        })
        .collect()
}

fn contributors_body(ctx: &Context, width: usize) -> Vec<Line<'static>> {
    let (theme, app) = (ctx.theme, ctx.app);
    let view = app.dashboard.contributors_view;
    let sort = app.dashboard.contributors_sort;
    let name_width = width
        .saturating_sub(2 + 2 + ContributorsView::COLUMN_COUNT * CONTRIBUTOR_COLUMN_CHARS)
        .max(4);
    let w = CONTRIBUTOR_COLUMN_CHARS;
    let sort_label = sort.map(|sort| (view.columns()[sort.column], !sort.is_descending));
    let header: String = view
        .columns()
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
    let visible = CARD_BODY_LINES - lines.len();
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
                Some(login) => format!("you (@{login})"),
                None => "you (login unknown)".to_string(),
            },
            DashboardPerson::User(login) => format!("@{}", sanitize_untrusted(login)),
        };
        let stats = app.contributor_stats(person, ctx.now);
        let numbers: String = match stats {
            Some(stats) => stats.iter().map(|value| format!("{value:>w$}")).collect(),
            None => format!("{:>w$}", "-").repeat(ContributorsView::COLUMN_COUNT),
        };
        let is_late = view == ContributorsView::OpenWork && stats.is_some_and(|s| s[3] > 0);
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
        ContributorsView::Activity(window) if window.days() == 1 => "last 24h".to_string(),
        ContributorsView::Activity(window) => format!("last {} days", window.days()),
    }
}

/// Why activity counts may be short: the feed ended inside the window.
fn feed_cut_off_note(ctx: &Context) -> Option<String> {
    let since = ctx.app.dashboard.contributors_view.activity_since(ctx.now);
    let ContributorsView::Activity(_) = ctx.app.dashboard.contributors_view else {
        return None;
    };
    let cut_off_at = ctx.app.dashboard.contributions.cut_off_after(since)?;
    Some(format!("(feed ends {})", time_ago(cut_off_at)))
}

fn pipelines_body(ctx: &Context) -> Vec<Line<'static>> {
    let (theme, icons) = (ctx.theme, ctx.icons);
    let health = dashboard::assess_pipelines(&ctx.app.pipelines.items);
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
        Line::from(
            health
                .recent
                .iter()
                .rev()
                .map(|pipeline| {
                    Span::styled(
                        status_dot(&pipeline.status, icons).to_string(),
                        Style::default().fg(outcome_color(&pipeline.status, theme)),
                    )
                })
                .collect::<Vec<_>>(),
        ),
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
    for pipeline in &health.latest_per_ref {
        let (fg, _, _, badge) =
            badge_style_for("Status", &pipeline.status, false, theme.bg, theme, icons);
        let when = pipeline.created_at().unwrap_or(pipeline.updated_at());
        lines.push(Line::from(vec![
            Span::styled(
                badge.unwrap_or_else(|| format!(" {} ", pipeline.status)),
                Style::default().fg(fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                sanitize_untrusted(&format_ref(&pipeline.r#ref)).into_owned(),
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
        .flat_map(|progress| milestone_lines(progress, theme))
        .collect()
}

fn milestone_lines(progress: &MilestoneProgress, theme: &Theme) -> [Line<'static>; 2] {
    let (bar, bar_color) = match progress.issues {
        Some((closed, total)) => progress_bar(closed, total, theme),
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
                format!("{bar:<PROGRESS_BAR_CHARS$}"),
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
    let window = contributors_view_label(ctx.app.dashboard.contributors_view);
    let window = match ctx.app.dashboard.contributors_view {
        ContributorsView::OpenWork => "last 30 days".to_string(),
        ContributorsView::Activity(_) => window,
    };
    let activity_of = |who: Who| match who {
        Who::Everyone => format!("Everyone's activity, {window}"),
        Who::You => format!("Your activity, {window}"),
        Who::Someone(login) => format!("Activity of @{login}, {window}"),
    };
    let mut label = match &ctx.selected {
        Some(DashboardRow::Mr(queue)) => return capitalized(mr_queue_label(*queue, ctx.who())),
        Some(DashboardRow::Issue(queue)) => {
            return capitalized(issue_queue_label(*queue, ctx.who()));
        }
        Some(DashboardRow::Contributor(person)) => {
            activity_of(Who::of(person.resolve(ctx.signed_in), ctx.signed_in))
        }
        Some(DashboardRow::Card(_)) | None => activity_of(ctx.who()),
    };
    if let ContributionFeed::Failed(reason) = &ctx.app.dashboard.contributions {
        label.push_str(&format!(
            " (no commits or comments: {})",
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
    let is_focused = app.dashboard.is_list_focused;
    let hint = if is_focused {
        " Enter open · Esc back "
    } else {
        " Enter browse "
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

    let rows = items.iter().map(|item| item_row(ctx, item));
    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Length(LIST_REF_WIDTH),
            Constraint::Fill(1),
            Constraint::Length(LIST_AUTHOR_WIDTH),
            Constraint::Length(LIST_PEOPLE_WIDTH),
            Constraint::Length(LIST_NOTE_WIDTH),
            Constraint::Length(LIST_UPDATED_WIDTH),
        ],
    )
    .block(block)
    .row_highlight_style(Style::default().bg(theme.highlight_bg))
    .highlight_symbol(format!("{} ", ctx.icons.highlight_arrow))
    .highlight_spacing(HighlightSpacing::Always);
    let mut state = TableState::default()
        .with_selected(is_focused.then(|| app.dashboard.list_cursor.min(items.len() - 1)));
    f.render_stateful_widget(table, area, &mut state);
}

fn item_row(ctx: &Context, item: &Activity) -> Row<'static> {
    let (theme, icons) = (ctx.theme, ctx.icons);
    let mr_marker = if ctx.kind.is_github() { '#' } else { '!' };
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
            let summary = sanitize_untrusted(&contribution.summary).into_owned();
            let title = match &contribution.target {
                Some(target) if !target.title.is_empty() => {
                    format!("{summary} · on {}", sanitize_untrusted(&target.title))
                }
                _ => summary,
            };
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
    Row::new(vec![
        Cell::from(icon.clone()).style(Style::default().fg(color)),
        Cell::from(reference).style(Style::default().fg(color)),
        Cell::from(title).style(Style::default().fg(theme.text_normal)),
        Cell::from(format!("@{}", sanitize_untrusted(author)))
            .style(Style::default().fg(theme.text_muted)),
        Cell::from(people).style(Style::default().fg(theme.text_muted)),
        Cell::from(Line::from(note)),
        Cell::from(time_ago(item.updated_at())).style(Style::default().fg(theme.text_muted)),
    ])
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

fn idle_note(ctx: &Context, updated_at: &str, idle_days: i64) -> Option<Span<'static>> {
    dashboard::days_since(updated_at, ctx.now.instant)
        .filter(|days| *days >= idle_days)
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
    notes.extend(idle_note(ctx, &mr.updated_at, MR_IDLE_DAYS));
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
    notes.extend(idle_note(ctx, &issue.updated_at, ISSUE_IDLE_DAYS));
    notes
}
