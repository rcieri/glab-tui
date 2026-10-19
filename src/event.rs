use crossterm::event::{self, Event as CrosstermEvent, KeyEvent, MouseEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub static PAUSED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Event {
    Tick,
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    PipelineJobs(u64, Vec<crate::domain::pipelines::Job>),
    /// The children of the pipeline currently descended into, re-fetched.
    ChildLevelFetched(u64, crate::domain::pipelines::ChildLevel),
    /// What lies below a pipeline opened from the Pipelines tab.
    PipelineOpened {
        pipeline_id: u64,
        opening: crate::handlers::tabs::PipelineOpening,
        result: Result<crate::handlers::tabs::PipelineContents, String>,
    },
    /// The backend client for `scope`, built off the event loop because its
    /// backend detection runs `git` and `gh`/`glab auth status`.
    ClientReady {
        scope: crate::scope::Scope,
        result: Result<crate::domain::client::GitlabClient, String>,
    },
    /// Issues listed with the given state query.
    IssuesFetched(
        crate::domain::state_query::StateQuery,
        Vec<crate::domain::issues::Issue>,
    ),
    /// MRs/PRs listed with the given state query.
    MrsFetched(
        crate::domain::state_query::StateQuery,
        Vec<crate::domain::mr::MergeRequest>,
    ),
    PipelinesFetched(Vec<crate::domain::pipelines::Pipeline>),
    RunnersFetched(Vec<crate::domain::runners::Runner>),
    ReleasesFetched(Vec<crate::domain::releases::Release>),
    SelectorItemsFetched(Vec<String>),
    RepoAttributesFetched {
        labels: Vec<crate::domain::labels::Label>,
        members: Vec<String>,
    },
    ProjectAttributesFetched {
        project: String,
        labels: Vec<crate::domain::labels::Label>,
        members: Vec<String>,
        milestones: Vec<String>,
        branches: Vec<String>,
    },
    /// A tab's list refresh failed. Only list refreshes send it: over cached
    /// rows the handler shows the offline badge and drops the message.
    FetchFailed(crate::app::Tab, String),
    /// The view is built off the event loop: parsing and highlighting a large
    /// diff takes seconds. `comments` fails on its own, leaving the diff
    /// viewable.
    DiffFetched {
        diff_view: Box<crate::app::DiffView>,
        comments: Result<Vec<crate::domain::mr::DiscussionNote>, String>,
    },
    DiffFetchFailed(String),
    /// `show_read` is the filter the list was requested with. A result for a
    /// filter the user has since toggled away from is stale.
    TodosFetched {
        notifications: Vec<crate::domain::notifications::Notification>,
        show_read: bool,
    },
    TodoChanged(crate::domain::notifications::TodoChange, Result<(), String>),
    JobsTabFetched(u64, Vec<crate::domain::pipelines::Job>),
    CommandStarted(String),
    CommandCompleted(crate::app::Tab, Result<(), String>),
    /// Non-error feedback from a background task, shown in the header bar.
    Notice(String),
    /// A background custom command finished all its runs.
    CustomCommandFinished(crate::handlers::custom_commands::RunReport),
    TerminalCommandLogged {
        timestamp: String,
        command: String,
        status: String,
    },
    MilestonesFetched(Vec<crate::domain::milestones::Milestone>),
    MilestoneIssuesFetched(u64, Vec<crate::domain::issues::Issue>),
    JobTraceFetched(u64, Result<crate::domain::job_trace::JobTrace, String>),
    /// What a followed job log gained since it was last read.
    JobTraceFollowed(u64, Result<crate::domain::job_trace::TraceUpdate, String>),
    MilestoneUpdated,
    MilestoneClosed,
    MilestoneReopened,
    MilestoneDeleted,
    ReleaseUpdated,
    ReleaseDeleted,
    IssueDeleted,
    MrDeleted,
    BranchesFetched(Vec<crate::domain::branches::Branch>),
    EnvironmentsFetched(Vec<crate::domain::deployments::Environment>),
    DeploymentsFetched {
        environment: String,
        deployments: Vec<crate::domain::deployments::Deployment>,
    },
    /// Result of fetching MRs/PRs that close an issue. `Ok(vec![])` is
    /// legitimate (the issue has no closing MRs) and is not an error.
    RelatedMrsFetched {
        issue_iid: u64,
        result: Result<Vec<crate::domain::issues::RelatedMrRef>, String>,
    },
    /// Result of fetching issues closed by an MR/PR.
    MrRelatedIssuesFetched {
        mr_iid: u64,
        result: Result<Vec<crate::domain::mr::RelatedIssueRef>, String>,
    },
    /// Single item fetched by the "go to issue/MR by ID" prompt. `Ok` carries
    /// the item so the handler can insert it into the loaded set if absent.
    IssueFetched(u64, Result<Box<crate::domain::issues::Issue>, String>),
    MrFetched(u64, Result<Box<crate::domain::mr::MergeRequest>, String>),
    /// On-demand stack fetch for one GitHub PR. `Ok(None)` means the PR is not
    /// in a stack, `Err` is the API failure.
    PrStackFetched {
        pr_number: u64,
        project_path: String,
        result: Result<Option<crate::domain::mr::PrStack>, String>,
    },
    /// Batch stack positions for the PRs in `pr_numbers`, fetched only while the
    /// Stack column needs them. A queried PR absent from `stacks` is not stacked.
    PrStackSummariesFetched {
        project_path: String,
        pr_numbers: Vec<u64>,
        stacks: std::collections::HashMap<u64, crate::domain::mr::StackInfo>,
    },
    /// A optimistic UI mutation for `runner_id` failed; restore the row's
    /// prior `status`/`active` fields. Sender fills `status`/`active` with
    /// the values that were on screen before the user pressed pause/resume.
    RunnerStateRevert {
        runner_id: u64,
        status: String,
        active: bool,
    },
    /// An optimistic pipeline retry/cancel failed; restore the row's prior status.
    PipelineStatusRevert {
        pipeline_id: u64,
        status: String,
    },
    /// An optimistic job retry/start/cancel failed; restore the row's prior status.
    JobStatusRevert {
        job_id: u64,
        status: String,
    },
    /// A background fetch result, tagged with the scope generation that was
    /// current when the fetch started. Built only by `ScopedSender`.
    Scoped(ScopeGeneration, Box<Event>),
}

impl Event {
    /// Unwraps a `Scoped` event requested in the `current` generation and
    /// drops one requested before the last repository or group switch, so a
    /// slow fetch never lands in the scope that replaced it. Other events
    /// pass through.
    pub fn for_scope(self, current: ScopeGeneration) -> Option<Event> {
        match self {
            Event::Scoped(generation, event) if generation == current => Some(*event),
            Event::Scoped(..) => None,
            event => Some(event),
        }
    }
}

/// Counts repository/group switches. Two fetches started in the same
/// generation belong to the same scope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScopeGeneration(u64);

impl ScopeGeneration {
    pub fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// The sending half handed to background fetches: every event it sends is
/// wrapped in `Event::Scoped` with the generation it was created for.
#[derive(Clone, Debug)]
pub struct ScopedSender {
    tx: mpsc::UnboundedSender<Event>,
    generation: ScopeGeneration,
}

impl ScopedSender {
    pub fn new(tx: mpsc::UnboundedSender<Event>, generation: ScopeGeneration) -> Self {
        Self { tx, generation }
    }

    pub fn send(&self, event: Event) -> Result<(), mpsc::error::SendError<Event>> {
        self.tx
            .send(Event::Scoped(self.generation, Box::new(event)))
    }
}

#[derive(Debug)]
pub struct EventHandler {
    sender: mpsc::UnboundedSender<Event>,
    receiver: mpsc::UnboundedReceiver<Event>,
}

impl EventHandler {
    pub fn new(tick_rate: u64) -> Self {
        let tick_rate = Duration::from_millis(tick_rate);
        let (sender, receiver) = mpsc::unbounded_channel();
        let _sender = sender.clone();

        // A dedicated OS thread, not a tokio task: crossterm's poll and read
        // block, and on a runtime worker they would stall every task queued
        // behind them for up to the poll timeout.
        std::thread::spawn(move || {
            let mut last_tick = Instant::now();
            loop {
                if PAUSED.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50));
                    last_tick = Instant::now();
                    continue;
                }

                let timeout = tick_rate
                    .checked_sub(last_tick.elapsed())
                    .unwrap_or_else(|| Duration::from_secs(0));
                let poll_timeout = std::cmp::min(timeout, Duration::from_millis(20));

                let Ok(has_event) = event::poll(poll_timeout) else {
                    break;
                };
                if has_event {
                    let Ok(raw) = event::read() else {
                        break;
                    };
                    let e = match raw {
                        CrosstermEvent::Key(e) => {
                            if e.kind == event::KeyEventKind::Press {
                                Event::Key(e)
                            } else {
                                continue;
                            }
                        }
                        CrosstermEvent::Mouse(e) => Event::Mouse(e),
                        CrosstermEvent::Resize(w, h) => Event::Resize(w, h),
                        _ => continue,
                    };
                    if _sender.send(e).is_err() {
                        break;
                    }
                }

                if last_tick.elapsed() >= tick_rate {
                    if _sender.send(Event::Tick).is_err() {
                        break;
                    }
                    last_tick = Instant::now();
                }
            }
        });

        Self { sender, receiver }
    }

    pub fn sender(&self) -> mpsc::UnboundedSender<Event> {
        self.sender.clone()
    }

    pub async fn next(&mut self) -> Option<Event> {
        self.receiver.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received(
        rx: &mut mpsc::UnboundedReceiver<Event>,
        current: ScopeGeneration,
    ) -> Option<Event> {
        rx.try_recv().expect("an event was sent").for_scope(current)
    }

    #[test]
    fn result_sent_before_a_scope_switch_is_dropped() {
        let mut app = crate::app::App::default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.scoped_sender(tx)
            .send(Event::IssuesFetched(
                crate::domain::state_query::StateQuery::All,
                Vec::new(),
            ))
            .expect("receiver is open");

        app.reset_on_scope_change();

        assert!(received(&mut rx, app.scope_generation).is_none());
    }

    #[test]
    fn result_sent_in_the_current_scope_is_unwrapped() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let current = ScopeGeneration::default().next();
        ScopedSender::new(tx, current)
            .send(Event::FetchFailed(
                crate::app::Tab::Issues,
                "boom".to_string(),
            ))
            .expect("receiver is open");

        match received(&mut rx, current) {
            Some(Event::FetchFailed(crate::app::Tab::Issues, message)) => {
                assert_eq!(message, "boom")
            }
            other => panic!("expected the unwrapped FetchFailed, got {other:?}"),
        }
    }

    #[test]
    fn unscoped_event_passes_through_any_generation() {
        let current = ScopeGeneration::default().next().next();
        assert!(matches!(Event::Tick.for_scope(current), Some(Event::Tick)));
    }
}
