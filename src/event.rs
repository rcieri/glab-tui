#![allow(dead_code)]

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
    PipelineJobs(u64, Vec<crate::gitlab::pipelines::Job>),
    IssuesFetched(Vec<crate::gitlab::issues::Issue>),
    MrsFetched(Vec<crate::gitlab::mr::MergeRequest>),
    PipelinesFetched(Vec<crate::gitlab::pipelines::Pipeline>),
    RunnersFetched(Vec<crate::gitlab::runners::Runner>),
    ReleasesFetched(Vec<crate::gitlab::releases::Release>),
    SelectorItemsFetched(Vec<String>),
    FetchFailed(crate::app::Tab, String),
    DiffFetched {
        mr_iid: u64,
        raw_diff: String,
        comments: Vec<crate::gitlab::mr::DiscussionNote>,
    },
    DiffFetchFailed(String),
    TodosFetched(Vec<crate::gitlab::notifications::Notification>),
    JobsTabFetched(u64, Vec<crate::gitlab::pipelines::Job>),
    CommandStarted(String),
    CommandCompleted(crate::app::Tab, Result<(), String>),
    TerminalCommandLogged {
        timestamp: String,
        command: String,
        status: String,
    },
    MilestonesFetched(Vec<crate::gitlab::milestones::Milestone>),
    MilestoneIssuesFetched(u64, Vec<crate::gitlab::issues::Issue>),
    JobTraceFetched(u64, Result<String, String>),
    MilestoneUpdated,
    MilestoneClosed,
    MilestoneReopened,
    MilestoneDeleted,
    ReleaseUpdated,
    ReleaseDeleted,
    BranchesFetched(Vec<crate::gitlab::branches::Branch>),
    EnvironmentsFetched(Vec<crate::gitlab::deployments::Environment>),
    DeploymentsFetched(Vec<crate::gitlab::deployments::Deployment>),
    IssueCommentsFetched {
        iid: u64,
        discussions: Vec<crate::gitlab::discussions::Discussion>,
    },
    MrCommentsFetched {
        iid: u64,
        discussions: Vec<crate::gitlab::discussions::Discussion>,
    },
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

        tokio::spawn(async move {
            let mut last_tick = Instant::now();
            loop {
                if PAUSED.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    last_tick = Instant::now();
                    continue;
                }

                let timeout = tick_rate
                    .checked_sub(last_tick.elapsed())
                    .unwrap_or_else(|| Duration::from_secs(0));
                let poll_timeout = std::cmp::min(timeout, Duration::from_millis(20));

                if event::poll(poll_timeout).expect("failed to poll new events") {
                    let e = match event::read().expect("failed to read event") {
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
