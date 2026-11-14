//! What `update::update` asks the event loop to do once it has applied an
//! event: start background work, hand the terminal to a child process or
//! stop. `update` never awaits or spawns anything itself, so every task,
//! thread and terminal handoff it starts is in the list it returns.

use crate::AppTerminal;
use crate::app::App;
use crate::domain::job_trace::JobTrace;
use crate::domain::mr::DiscussionNote;
use crate::domain::review::DraftComment;
use crate::event::Event;
use crate::handlers::custom_commands::TerminalRuns;
use std::pin::Pin;

/// Background work that reports back through the event channel.
pub type Task = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

pub enum Effect {
    /// Runs a task on the tokio runtime.
    Spawn(Task),
    /// Runs blocking work on its own OS thread.
    SpawnThread(Box<dyn FnOnce() + Send + 'static>),
    /// Hands the terminal to a foreground child process until it exits.
    Suspend(Box<ForegroundJob>),
    /// Leaves the event loop.
    Quit,
}

impl std::fmt::Debug for Effect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Effect::Spawn(_) => f.write_str("Spawn"),
            Effect::SpawnThread(_) => f.write_str("SpawnThread"),
            Effect::Suspend(job) => f.debug_tuple("Suspend").field(job).finish(),
            Effect::Quit => f.write_str("Quit"),
        }
    }
}

/// The effects one event produced, in the order they were asked for.
#[derive(Debug, Default)]
pub struct Effects(Vec<Effect>);

impl Effects {
    pub fn spawn(&mut self, task: impl Future<Output = ()> + Send + 'static) {
        self.0.push(Effect::Spawn(Box::pin(task)));
    }

    /// Like `spawn`, for a task that is dropped, along with whatever it was
    /// awaiting, once the returned handle is.
    pub fn spawn_cancellable(
        &mut self,
        task: impl Future<Output = ()> + Send + 'static,
    ) -> CancelHandle {
        let (handle, cancelled) = tokio::sync::oneshot::channel::<()>();
        self.spawn(async move {
            tokio::select! {
                _ = cancelled => {}
                () = task => {}
            }
        });
        CancelHandle {
            _cancels_on_drop: handle,
        }
    }

    pub fn spawn_thread(&mut self, job: impl FnOnce() + Send + 'static) {
        self.0.push(Effect::SpawnThread(Box::new(job)));
    }

    pub fn suspend(&mut self, job: ForegroundJob) {
        self.0.push(Effect::Suspend(Box::new(job)));
    }

    pub fn quit(&mut self) {
        self.0.push(Effect::Quit);
    }

    pub fn into_vec(self) -> Vec<Effect> {
        self.0
    }
}

/// Stops a task started with `Effects::spawn_cancellable` when dropped.
#[derive(Debug)]
pub struct CancelHandle {
    _cancels_on_drop: tokio::sync::oneshot::Sender<()>,
}

/// A foreground child process that needs the terminal.
#[derive(Debug)]
pub enum ForegroundJob {
    /// Shows the open job log read-only in the editor.
    ViewJobTrace { job_id: u64 },
    /// Shows a deployment's job log read-only in the editor.
    ViewDeploymentLog { deployment_id: u64, log: JobTrace },
    /// Edits `text` in the editor. The result comes back as
    /// `Event::EditorClosed` for `purpose`.
    Edit {
        text: String,
        file_suffix: String,
        purpose: EditPurpose,
    },
    /// Runs custom commands in the foreground.
    RunCommands(TerminalRuns),
}

/// What an edited text is for, so `update` can apply it once the editor
/// closes.
#[derive(Clone, Debug)]
pub enum EditPurpose {
    /// The value of the open text input.
    TextInput,
    /// The summary of the submit-review overlay.
    ReviewSummary,
    /// The Description or Release Notes field of the open edit menu.
    MenuDescription,
    /// The body of the draft comment at this index.
    DraftBody { index: usize },
    /// A pushed comment on MR `mr_iid`, edited in place.
    CommentBody {
        mr_iid: u64,
        comment: Box<DiscussionNote>,
    },
    /// A new comment on the open diff, anchored like `draft`.
    DiffComment { draft: DraftComment },
    /// A code suggestion on the open diff, anchored like `draft`.
    DiffSuggestion { draft: DraftComment },
    /// The line input at `path` in the open confirmation dialog's form.
    DialogField { path: crate::form::GroupPath },
}

impl ForegroundJob {
    /// Runs the job with the terminal handed over and returns the event that
    /// reports its outcome, if it has one.
    pub fn run(self, app: &App, terminal: &mut AppTerminal) -> Option<Event> {
        match self {
            ForegroundJob::ViewJobTrace { job_id } => {
                let view = app.job_trace.as_ref()?;
                let prefix = format!("job_{job_id}_trace_");
                crate::editor::view_in_editor(view.trace().text(), &prefix, terminal)
                    .err()
                    .map(|error| {
                        Event::ViewerFailed(format!("Failed to open job trace in editor: {error}"))
                    })
            }
            ForegroundJob::ViewDeploymentLog { deployment_id, log } => {
                let prefix = format!("deployment_{deployment_id}_log_");
                crate::editor::view_in_editor(log.text(), &prefix, terminal)
                    .err()
                    .map(|error| {
                        Event::ViewerFailed(format!(
                            "Failed to open deployment log in editor: {error}"
                        ))
                    })
            }
            ForegroundJob::Edit {
                text,
                file_suffix,
                purpose,
            } => {
                let text = crate::editor::edit_in_editor_with_suffix(&text, &file_suffix, terminal);
                Some(Event::EditorClosed { purpose, text })
            }
            ForegroundJob::RunCommands(runs) => {
                Some(Event::CustomCommandFinished(runs.run(terminal)))
            }
        }
    }
}
