#![allow(clippy::all)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_assignments)]

mod app;
mod config;
mod event;
mod gitlab;
mod ui;
pub mod utils;

use anyhow::Result;
use app::App;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use event::{Event, EventHandler};
use ratatui::{Terminal, backend::CrosstermBackend, widgets::ListState};
use std::io;

type AppTerminal = Terminal<CrosstermBackend<std::io::Stdout>>;

struct Cli {
    is_github: bool,
}

impl Cli {
    fn program(&self) -> &'static str {
        if self.is_github { "gh" } else { "glab" }
    }

    fn entity<'a>(&self, name: &'a str) -> &'a str {
        if self.is_github && name == "mr" {
            "pr"
        } else {
            name
        }
    }

    fn sub_update(&self) -> &'static str {
        if self.is_github { "edit" } else { "update" }
    }

    fn flag_description(&self) -> &'static str {
        if self.is_github {
            "--body"
        } else {
            "--description"
        }
    }

    fn flag_description_short(&self) -> &'static str {
        if self.is_github { "--body" } else { "-d" }
    }

    fn flag_branch(&self) -> &'static str {
        if self.is_github { "-r" } else { "-b" }
    }

    fn flag_input(&self) -> &'static str {
        if self.is_github { "-f" } else { "-i" }
    }

    fn flag_variable(&self) -> &'static str {
        if self.is_github { "-f" } else { "--variables" }
    }

    fn flag_web(&self) -> &'static str {
        if self.is_github { "--web" } else { "-w" }
    }

    fn input_separator(&self) -> &str {
        if self.is_github { "=" } else { ":" }
    }
}

struct UpdateCmd {
    is_github: bool,
    args: Vec<String>,
}

impl UpdateCmd {
    fn new(is_github: bool, entity: &str, iid: u64) -> Self {
        let e = if is_github && entity == "mr" {
            "pr"
        } else {
            entity
        };
        let cmd = if is_github { "edit" } else { "update" };
        Self {
            is_github,
            args: vec![e.to_string(), cmd.to_string(), iid.to_string()],
        }
    }

    fn flag(mut self, name: &str, value: &str) -> Self {
        let (name, value) = match (self.is_github, name) {
            (true, "-d" | "--description") => ("--body", value),
            (true, "--unlabel") if value == "all" => ("--label", ""),
            (true, "--unassign") => ("--assignee", ""),
            (true, "--target-branch") => ("--base", value),
            (true, "--milestone") if value == "0" => ("--milestone", ""),
            _ => (name, value),
        };
        self.args.push(name.to_string());
        self.args.push(value.to_string());
        self
    }

    fn flag_bool(mut self, name: &str) -> Self {
        self.args.push(name.to_string());
        self
    }

    fn build(&self) -> Vec<String> {
        self.args.clone()
    }
}

fn app_cli(app: &App) -> Cli {
    Cli {
        is_github: app
            .gitlab_client
            .as_ref()
            .map(|c| c.is_github)
            .unwrap_or(false),
    }
}

async fn run_cli(
    cli: &Cli,
    args: &[String],
    terminal: &mut AppTerminal,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    tab: crate::app::Tab,
) {
    let program = cli.program();
    let is_interactive = if cli.is_github {
        args.iter().any(|a| a == "-e")
    } else {
        args.windows(2)
            .any(|w| (w[0] == "-d" || w[0] == "--description") && w[1] == "-")
    };

    if is_interactive {
        crate::event::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let cancelled = (|| -> Option<bool> {
            disable_raw_mode().ok()?;
            execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture).ok()?;

            let mut actual_args = args.to_vec();
            let mut cancel = false;

            if cli.is_github && actual_args.iter().any(|arg| arg == "-e") {
                actual_args.retain(|arg| arg != "-e");

                let is_pr = actual_args.iter().any(|arg| arg == "pr");
                let is_edit = actual_args.iter().any(|arg| arg == "edit");
                let entity_type = if is_pr { "pr" } else { "issue" };
                let mut initial_body = String::new();

                if is_edit {
                    if let Some(pos) = actual_args.iter().position(|arg| arg == "edit") {
                        if pos + 1 < actual_args.len() {
                            let id = &actual_args[pos + 1];
                            let output = std::process::Command::new("gh")
                                .args([entity_type, "view", id, "--json", "body", "--jq", ".body"])
                                .output();
                            if let Ok(out) = output {
                                if out.status.success() {
                                    initial_body =
                                        String::from_utf8_lossy(&out.stdout).trim().to_string();
                                }
                            }
                        }
                    }
                }

                if initial_body.is_empty() {
                    let template_type = if is_pr { "mr" } else { "issue" };
                    initial_body = get_default_template(template_type).unwrap_or_default();
                }

                let edited_body = (|| {
                    let editor = std::env::var("EDITOR")
                        .or_else(|_| std::env::var("VISUAL"))
                        .unwrap_or_else(|_| "helix".to_string());
                    let mut tmp = tempfile::Builder::new().suffix(".md").tempfile().ok()?;
                    std::io::Write::write_all(&mut tmp, initial_body.as_bytes()).ok()?;
                    let file_path = tmp.into_temp_path();

                    let mut cmd = std::process::Command::new(&editor);
                    cmd.arg(file_path.as_os_str());
                    cmd.stdin(std::process::Stdio::inherit())
                        .stdout(std::process::Stdio::inherit())
                        .stderr(std::process::Stdio::inherit());
                    if let Ok(mut child) = cmd.spawn() {
                        child.wait().ok()?;
                    }

                    let content = std::fs::read_to_string(&file_path).ok()?;
                    Some(content.trim().to_string())
                })();

                if let Some(body) = edited_body {
                    actual_args.push("--body".to_string());
                    actual_args.push(body);
                } else {
                    cancel = true;
                }
            }

            if !cancel {
                let mut cmd = std::process::Command::new(program);
                cmd.args(&actual_args);
                cmd.stdin(std::process::Stdio::inherit());
                cmd.stdout(std::process::Stdio::inherit());
                cmd.stderr(std::process::Stdio::inherit());

                if let Ok(mut child) = cmd.spawn() {
                    let _ = child.wait();
                }
            }

            Some(cancel)
        })();

        // Always restore terminal and reset PAUSED
        let _ = enable_raw_mode();
        let _ = execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture);
        while crossterm::event::poll(std::time::Duration::from_secs(0)).unwrap_or(false) {
            let _ = crossterm::event::read();
        }
        let _ = terminal.clear();
        crate::event::PAUSED.store(false, std::sync::atomic::Ordering::Relaxed);

        if cancelled.unwrap_or(true) {
            let _ = tx.send(Event::CommandCompleted(
                tab,
                Err("Edit cancelled".to_string()),
            ));
        } else {
            let _ = tx.send(Event::CommandCompleted(tab, Ok(())));
        }
    } else {
        let status_msg = format!("{} {}", program, args.join(" "));
        let _ = tx.send(Event::CommandStarted(status_msg));

        let tx_clone = tx.clone();
        let program = program.to_string();
        let actual_args = args.to_vec();

        tokio::spawn(async move {
            let mut cmd = tokio::process::Command::new(&program);
            cmd.args(&actual_args);

            match cmd.output().await {
                Ok(output) => {
                    if output.status.success() {
                        let _ = tx_clone.send(Event::CommandCompleted(tab, Ok(())));
                    } else {
                        let err_msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
                        let _ = tx_clone.send(Event::CommandCompleted(
                            tab,
                            Err(format!("Command failed: {}", err_msg)),
                        ));
                    }
                }
                Err(e) => {
                    let _ = tx_clone.send(Event::CommandCompleted(
                        tab,
                        Err(format!("Failed to execute command: {}", e)),
                    ));
                }
            }
        });
    }
}

fn editor_name() -> String {
    std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "helix".to_string())
}

fn edit_in_editor(current_val: &str, terminal: &mut AppTerminal) -> Option<String> {
    edit_in_editor_with_suffix(current_val, ".md", terminal)
}

fn edit_in_editor_with_suffix(
    current_val: &str,
    suffix: &str,
    terminal: &mut AppTerminal,
) -> Option<String> {
    let editor = editor_name();

    let mut tmp = tempfile::Builder::new().suffix(suffix).tempfile().ok()?;
    std::io::Write::write_all(&mut tmp, current_val.as_bytes()).ok()?;
    let file_path = tmp.into_temp_path();

    crate::event::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
    std::thread::sleep(std::time::Duration::from_millis(50));

    let result = (|| {
        crossterm::terminal::disable_raw_mode().ok()?;
        crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
        )
        .ok()?;

        let mut cmd = std::process::Command::new(&editor);
        cmd.arg(file_path.as_os_str());
        cmd.stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        if let Ok(mut child) = cmd.spawn() {
            child.wait().ok()?;
        }

        let content = std::fs::read_to_string(&file_path).ok()?;
        let trimmed = content.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })();

    // Always resume TUI — PAUSED is reset even if the closure returned None
    let _ = crossterm::terminal::enable_raw_mode();
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
    );
    while crossterm::event::poll(std::time::Duration::from_secs(0)).unwrap_or(false) {
        let _ = crossterm::event::read();
    }
    let _ = terminal.clear();
    crate::event::PAUSED.store(false, std::sync::atomic::Ordering::Relaxed);

    result
}

async fn apply_field_text_change(
    app: &mut App,
    entity_type: &str,
    iid: u64,
    field_type: &str,
    value: String,
    terminal: &mut AppTerminal,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    tab: crate::app::Tab,
) {
    let cli = app_cli(app);
    match field_type {
        "title" => {
            let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                .flag("--title", &value)
                .build();
            run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            if entity_type == "issue" {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.title = value;
                }
            } else if entity_type == "mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.title = value;
                }
            }
        }
        "target_branch" => {
            if entity_type == "mr" {
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag("--target-branch", &value)
                    .build();
                run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.target_branch = value;
                }
            }
        }
        "due_date" => {
            if entity_type == "issue" {
                let flag_value = if value == "YYYY-MM-DD" || value.trim().is_empty() {
                    ""
                } else {
                    &value
                };
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag("--due-date", flag_value)
                    .build();
                run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.due_date = if flag_value.is_empty() {
                        None
                    } else {
                        Some(flag_value.to_string())
                    };
                }
            }
        }
        "weight" => {
            if entity_type == "issue" {
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag("--weight", &value)
                    .build();
                run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            }
        }
        "runner_description" => {
            let args: Vec<String> = vec![
                "api".into(),
                "-X".into(),
                "PUT".into(),
                format!("runners/{}", iid),
                "-f".into(),
                format!("description={}", value),
            ];
            run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            if let Some(runner) = app.runners.items.iter_mut().find(|r| r.id == iid) {
                runner.description = Some(value);
            }
        }
        "description" => {
            let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                .flag("-d", &value)
                .build();
            run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            if entity_type == "issue" {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.description = Some(value);
                }
            } else if entity_type == "mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.description = Some(value);
                }
            }
        }
        _ => {
            if entity_type == "milestone" {
                let milestone_opt = app.milestones.items.iter().find(|m| m.iid == iid).cloned();
                if let Some(milestone) = milestone_opt {
                    let mut title = milestone.title.clone();
                    let mut start_date = milestone.start_date.clone();
                    let mut due_date = milestone.due_date.clone();
                    let mut description = milestone.description.clone().unwrap_or_default();

                    match field_type {
                        "title" => title = value.clone(),
                        "start_date" => start_date = Some(value.clone()),
                        "due_date" => due_date = Some(value.clone()),
                        "description" => description = value.clone(),
                        _ => {}
                    }

                    let client = app.gitlab_client.clone().unwrap();
                    let project_path = app.project_context.clone();
                    let tx_spawn = tx.clone();
                    let _ = tx.send(Event::CommandStarted(format!(
                        "Updating milestone #{}",
                        iid
                    )));
                    tokio::spawn(async move {
                        let res = crate::gitlab::milestones::update_milestone(
                            &client,
                            &project_path,
                            iid,
                            &title,
                            &description,
                            start_date.as_deref(),
                            due_date.as_deref(),
                        )
                        .await;
                        match res {
                            Ok(_) => {
                                let _ = tx_spawn.send(Event::MilestoneUpdated);
                            }
                            Err(e) => {
                                let _ = tx_spawn.send(Event::CommandCompleted(
                                    crate::app::Tab::Milestones,
                                    Err(e.to_string()),
                                ));
                            }
                        }
                    });
                }
            }
        }
    }
}

async fn apply_selector_changes(
    app: &mut App,
    entity_type: &str,
    iid: u64,
    field_type: &str,
    values: Vec<String>,
    terminal: &mut AppTerminal,
) {
    let cli = app_cli(app);
    let tx = app.tx.clone().unwrap();
    let tab = app.active_tab;
    match field_type {
        "labels" => {
            let current_labels: Vec<String> = if entity_type == "issue" {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .map(|i| i.labels.clone())
                    .unwrap_or_default()
            } else if entity_type == "mr" {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.labels.clone())
                    .unwrap_or_default()
            } else {
                vec![]
            };
            let current_set: std::collections::HashSet<&str> =
                current_labels.iter().map(String::as_str).collect();
            let new_set: std::collections::HashSet<&str> =
                values.iter().map(String::as_str).collect();
            let to_remove: Vec<&&str> = current_set.difference(&new_set).collect();
            let to_add: Vec<&&str> = new_set.difference(&current_set).collect();

            let mut cmd = UpdateCmd::new(cli.is_github, entity_type, iid);
            if cli.is_github {
                for r in &to_remove {
                    cmd = cmd.flag("--remove-label", r);
                }
                for a in &to_add {
                    cmd = cmd.flag("--add-label", a);
                }
            } else {
                if !to_remove.is_empty() {
                    let joined = to_remove
                        .iter()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    cmd = cmd.flag("--unlabel", &joined);
                }
                if !to_add.is_empty() {
                    let joined = to_add
                        .iter()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    cmd = cmd.flag("--label", &joined);
                }
            }
            let args = cmd.build();
            if !to_remove.is_empty() || !to_add.is_empty() {
                run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            }

            if entity_type == "issue" {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.labels = values;
                }
            } else if entity_type == "mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.labels = values;
                }
            }
        }
        "assignees" => {
            let current_usernames: Vec<String> = if entity_type == "issue" {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .map(|i| i.assignees.iter().map(|a| a.username.clone()).collect())
                    .unwrap_or_default()
            } else if entity_type == "mr" {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.assignees.iter().map(|a| a.username.clone()).collect())
                    .unwrap_or_default()
            } else {
                vec![]
            };
            let values_clean: Vec<String> = values
                .iter()
                .map(|v| v.trim_start_matches('@').to_string())
                .collect();
            let current_set: std::collections::HashSet<&str> =
                current_usernames.iter().map(String::as_str).collect();
            let new_set: std::collections::HashSet<&str> =
                values_clean.iter().map(String::as_str).collect();
            let to_remove: Vec<&&str> = current_set.difference(&new_set).collect();
            let to_add: Vec<&&str> = new_set.difference(&current_set).collect();

            let mut cmd = UpdateCmd::new(cli.is_github, entity_type, iid);
            if cli.is_github {
                for r in &to_remove {
                    cmd = cmd.flag("--remove-assignee", r);
                }
                for a in &to_add {
                    cmd = cmd.flag("--add-assignee", a);
                }
            } else {
                if !to_remove.is_empty() {
                    let joined = to_remove
                        .iter()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    cmd = cmd.flag("--unassign", &joined);
                }
                if !to_add.is_empty() {
                    let joined = to_add
                        .iter()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    cmd = cmd.flag("--assignee", &joined);
                }
            }
            let args = cmd.build();
            if !to_remove.is_empty() || !to_add.is_empty() {
                run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            }

            let new_assignees: Vec<crate::gitlab::issues::Assignee> = values_clean
                .iter()
                .map(|u| crate::gitlab::issues::Assignee {
                    username: u.clone(),
                })
                .collect();

            if entity_type == "issue" {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.assignees = new_assignees;
                }
            } else if entity_type == "mr" {
                let mr_assignees: Vec<crate::gitlab::mr::Assignee> = new_assignees
                    .iter()
                    .map(|a| crate::gitlab::mr::Assignee {
                        username: a.username.clone(),
                    })
                    .collect();
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.assignees = mr_assignees;
                }
            }
        }
        "reviewers" => {
            if entity_type == "mr" {
                let clean_values: Vec<String> = values
                    .iter()
                    .map(|v| v.trim_start_matches('@').to_string())
                    .collect();
                let reviewers_comma = clean_values.join(",");

                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag("--reviewer", &reviewers_comma)
                    .build();
                run_cli(&cli, &args, terminal, tx.clone(), tab).await;

                let new_reviewers: Vec<crate::gitlab::mr::Reviewer> = clean_values
                    .into_iter()
                    .map(|u| crate::gitlab::mr::Reviewer { username: u })
                    .collect();

                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.reviewers = new_reviewers;
                }
            }
        }
        "milestone" => {
            if let Some(milestone_title) = values.first() {
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag("--milestone", milestone_title)
                    .build();
                run_cli(&cli, &args, terminal, tx.clone(), tab).await;

                let new_milestone = Some(crate::gitlab::issues::Milestone {
                    title: milestone_title.clone(),
                });
                if entity_type == "issue" {
                    if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                        item.milestone = new_milestone;
                    }
                } else if entity_type == "mr" {
                    let mr_milestone = Some(crate::gitlab::mr::Milestone {
                        title: milestone_title.clone(),
                    });
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.milestone = mr_milestone;
                    }
                }
            } else {
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag("--milestone", "0")
                    .build();
                run_cli(&cli, &args, terminal, tx.clone(), tab).await;

                if entity_type == "issue" {
                    if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                        item.milestone = None;
                    }
                } else if entity_type == "mr" {
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.milestone = None;
                    }
                }
            }
        }
        "confidential" => {
            if let Some(val) = values.first() {
                let flag = if val.to_lowercase() == "confidential" {
                    "--confidential"
                } else {
                    "--public"
                };
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag_bool(flag)
                    .build();
                run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            }
        }
        "draft_status" => {
            if let Some(val) = values.first() {
                let action = if val.to_lowercase() == "draft" {
                    "--draft"
                } else {
                    "--ready"
                };
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag_bool(action)
                    .build();
                run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                if entity_type == "mr" {
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.draft = val.to_lowercase() == "draft";
                    }
                }
            }
        }
        "target_branch" => {
            if let Some(val) = values.first() {
                if entity_type == "mr" {
                    let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                        .flag("--target-branch", val)
                        .build();
                    run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.target_branch = val.clone();
                    }
                }
            }
        }
        "description_edit_choice" => {
            app.selector = None;
            let choice = values.first().cloned().unwrap_or_default();

            if iid == 0 {
                if let Some(ref mut menu) = app.edit_menu {
                    if let Some(f) = menu.fields.iter_mut().find(|f| f.0 == "Description") {
                        if choice == "Edit (basic)" {
                            // Inline text input for new entity Description field
                            app.text_input = Some(crate::app::TextInput {
                                title: " Edit Description ".to_string(),
                                value: f.1.clone(),
                                cursor_idx: f.1.len(),
                                action: crate::app::TextInputAction::EditNewField {
                                    field_idx: menu
                                        .fields
                                        .iter()
                                        .position(|f| f.0 == "Description")
                                        .unwrap_or(0),
                                },
                            });
                        } else {
                            // Edit ($EDITOR) — external editor
                            let current_val = if f.1.trim().is_empty() {
                                let template_type = if entity_type == "new_mr" {
                                    "mr"
                                } else {
                                    "issue"
                                };
                                get_default_template(template_type).unwrap_or_default()
                            } else {
                                f.1.clone()
                            };
                            if let Some(new_desc) = edit_in_editor(&current_val, terminal) {
                                f.1 = new_desc;
                            }
                        }
                    }
                }
            } else {
                let current_desc = if entity_type == "issue" {
                    app.issues
                        .items
                        .iter()
                        .find(|i| i.iid == iid)
                        .and_then(|i| i.description.clone())
                        .unwrap_or_default()
                } else {
                    app.mrs
                        .items
                        .iter()
                        .find(|m| m.iid == iid)
                        .and_then(|m| m.description.clone())
                        .unwrap_or_default()
                };

                if choice == "Edit (basic)" {
                    app.text_input = Some(crate::app::TextInput {
                        title: " Edit Description ".to_string(),
                        value: current_desc.clone(),
                        cursor_idx: current_desc.len(),
                        action: crate::app::TextInputAction::EditField {
                            entity_iid: iid,
                            entity_type: entity_type.to_string(),
                            field_type: "description".to_string(),
                        },
                    });
                } else {
                    if let Some(new_desc) = edit_in_editor(&current_desc, terminal) {
                        if entity_type == "issue" {
                            if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                                item.description = Some(new_desc.clone());
                            }
                        } else if entity_type == "mr" {
                            if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                                item.description = Some(new_desc.clone());
                            }
                        }
                        let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                            .flag("-d", &new_desc)
                            .build();
                        run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                    }
                    if let Some(client) = &app.gitlab_client {
                        if entity_type == "issue" {
                            if let Ok(updated) =
                                gitlab::issues::get_issue(client, &app.project_context, iid).await
                            {
                                if let Some(item) =
                                    app.issues.items.iter_mut().find(|i| i.iid == iid)
                                {
                                    *item = updated;
                                }
                            }
                        } else if entity_type == "mr" {
                            if let Ok(updated) =
                                gitlab::mr::get_mr(client, &app.project_context, iid).await
                            {
                                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid)
                                {
                                    *item = updated;
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn rebuild_edit_menu(app: &mut App, entity_type: &str, entity_iid: u64) {
    if entity_type == "issue" {
        if let Some(issue) = app.issues.items.iter().find(|i| i.iid == entity_iid) {
            let labels = if issue.labels.is_empty() {
                "None".to_string()
            } else {
                issue.labels.join(", ")
            };
            let milestone = issue
                .milestone
                .as_ref()
                .map(|m| m.title.clone())
                .unwrap_or_else(|| "None".to_string());
            let assignees = if issue.assignees.is_empty() {
                "None".to_string()
            } else {
                issue
                    .assignees
                    .iter()
                    .map(|a| format!("@{}", a.username))
                    .collect::<Vec<_>>()
                    .join(", ")
            };

            let selected_idx = app.edit_menu.as_ref().map(|m| m.selected_idx).unwrap_or(0);

            let cli = app_cli(app);
            let mut fields = vec![
                ("Title".to_string(), issue.title.clone()),
                ("Labels".to_string(), labels),
                ("Assignees".to_string(), assignees),
                ("Milestone".to_string(), milestone),
            ];
            if !cli.is_github {
                fields.push(("Confidential".to_string(), "Toggle/Set".to_string()));
                fields.push((
                    "Due Date".to_string(),
                    issue.due_date.clone().unwrap_or_else(|| "Set".to_string()),
                ));
                fields.push(("Weight".to_string(), "Set".to_string()));
            }
            fields.push((
                "Description".to_string(),
                issue.description.clone().unwrap_or_default(),
            ));

            app.edit_menu = Some(crate::app::EditMenu {
                title: format!("Edit Issue #{}", issue.iid),
                fields,
                selected_idx,
                entity_iid: issue.iid,
                entity_type: "issue".to_string(),
                state: {
                    let mut s = ListState::default();
                    s.select(Some(selected_idx));
                    s
                },
            });
        }
    } else if entity_type == "mr" {
        if let Some(mr) = app.mrs.items.iter().find(|m| m.iid == entity_iid) {
            let labels = if mr.labels.is_empty() {
                "None".to_string()
            } else {
                mr.labels.join(", ")
            };
            let milestone = mr
                .milestone
                .as_ref()
                .map(|m| m.title.clone())
                .unwrap_or_else(|| "None".to_string());
            let assignees = if mr.assignees.is_empty() {
                "None".to_string()
            } else {
                mr.assignees
                    .iter()
                    .map(|a| format!("@{}", a.username))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let reviewers = if mr.reviewers.is_empty() {
                "None".to_string()
            } else {
                mr.reviewers
                    .iter()
                    .map(|r| format!("@{}", r.username))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let draft_status = if mr.draft { "Draft" } else { "Ready" };

            let selected_idx = app.edit_menu.as_ref().map(|m| m.selected_idx).unwrap_or(0);

            let pr_suffix = if app
                .gitlab_client
                .as_ref()
                .map(|c| c.is_github)
                .unwrap_or(false)
            {
                "PR"
            } else {
                "MR"
            };
            app.edit_menu = Some(crate::app::EditMenu {
                title: format!("Edit {} #{}", pr_suffix, mr.iid),
                fields: vec![
                    ("Title".to_string(), mr.title.clone()),
                    ("Labels".to_string(), labels),
                    ("Assignees".to_string(), assignees),
                    ("Reviewers".to_string(), reviewers),
                    ("Milestone".to_string(), milestone),
                    ("Target Branch".to_string(), mr.target_branch.clone()),
                    ("Status (Draft/Ready)".to_string(), draft_status.to_string()),
                    (
                        "Description".to_string(),
                        mr.description.clone().unwrap_or_default(),
                    ),
                ],
                selected_idx,
                entity_iid: mr.iid,
                entity_type: "mr".to_string(),
                state: {
                    let mut s = ListState::default();
                    s.select(Some(selected_idx));
                    s
                },
            });
        }
    } else if entity_type == "milestone" {
        if let Some(milestone) = app.milestones.items.iter().find(|m| m.iid == entity_iid) {
            let selected_idx = app.edit_menu.as_ref().map(|m| m.selected_idx).unwrap_or(0);
            let cli = app_cli(app);
            let mut fields = vec![("Title".to_string(), milestone.title.clone())];
            if !cli.is_github {
                fields.push((
                    "Start Date".to_string(),
                    milestone.start_date.clone().unwrap_or_default(),
                ));
            }
            fields.push((
                "Due Date".to_string(),
                milestone.due_date.clone().unwrap_or_default(),
            ));
            fields.push((
                "Description".to_string(),
                milestone.description.clone().unwrap_or_default(),
            ));

            app.edit_menu = Some(crate::app::EditMenu {
                title: format!("Edit Milestone #{}", milestone.iid),
                fields,
                selected_idx,
                entity_iid: milestone.iid,
                entity_type: "milestone".to_string(),
                state: {
                    let mut s = ListState::default();
                    s.select(Some(selected_idx));
                    s
                },
            });
        }
    }
}

async fn handle_entity_update(
    app: &mut App,
    entity_type: &str,
    iid: u64,
    code: KeyCode,
    terminal: &mut AppTerminal,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    tab: crate::app::Tab,
) {
    let cli = app_cli(app);
    match code {
        KeyCode::Char('t') => {
            let current_title = if entity_type == "issue" {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .map(|i| i.title.clone())
                    .unwrap_or_default()
            } else {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.title.clone())
                    .unwrap_or_default()
            };

            if let Some(new_title) = edit_in_editor(&current_title, terminal) {
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag("--title", &new_title)
                    .build();
                run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                if entity_type == "issue" {
                    if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                        item.title = new_title;
                    }
                } else if entity_type == "mr" {
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.title = new_title;
                    }
                }
            }
        }
        KeyCode::Char('r') => {
            if entity_type == "mr" {
                let is_draft = app
                    .mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.draft)
                    .unwrap_or(false);
                let action = if is_draft { "--ready" } else { "--draft" };
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag_bool(action)
                    .build();
                run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.draft = !is_draft;
                }
            }
        }
        KeyCode::Char('g') => {
            if entity_type == "mr" {
                let current_branch = app
                    .mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.target_branch.clone())
                    .unwrap_or_default();
                if let Some(target) = edit_in_editor(&current_branch, terminal) {
                    let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                        .flag("--target-branch", &target)
                        .build();
                    run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.target_branch = target;
                    }
                }
            }
        }
        KeyCode::Char('c') => {
            if entity_type == "issue" {
                if let Some(res) = edit_in_editor("public", terminal) {
                    let flag = if res.to_lowercase().contains("confidential") {
                        "--confidential"
                    } else {
                        "--public"
                    };
                    let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                        .flag_bool(flag)
                        .build();
                    run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                }
            }
        }
        KeyCode::Char('u') => {
            if entity_type == "issue" {
                if let Some(due_date) = edit_in_editor("YYYY-MM-DD", terminal) {
                    let flag_value = if due_date == "YYYY-MM-DD" || due_date.is_empty() {
                        ""
                    } else {
                        &due_date
                    };
                    let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                        .flag("--due-date", flag_value)
                        .build();
                    run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                }
            }
        }
        KeyCode::Char('w') => {
            if entity_type == "issue" {
                if let Some(weight) = edit_in_editor("0", terminal) {
                    let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                        .flag("--weight", &weight)
                        .build();
                    run_cli(&cli, &args, terminal, tx.clone(), tab).await;
                }
            }
        }
        KeyCode::Char('d') => {
            let current_desc = if entity_type == "issue" {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .and_then(|i| i.description.clone())
                    .unwrap_or_default()
            } else {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .and_then(|m| m.description.clone())
                    .unwrap_or_default()
            };
            app.text_input = Some(crate::app::TextInput {
                title: " Edit Description ".to_string(),
                value: current_desc.clone(),
                cursor_idx: current_desc.len(),
                action: crate::app::TextInputAction::EditField {
                    entity_iid: iid,
                    entity_type: entity_type.to_string(),
                    field_type: "description".to_string(),
                },
            });
        }
        KeyCode::Char('D') => {
            let current_desc = if entity_type == "issue" {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .and_then(|i| i.description.clone())
                    .unwrap_or_default()
            } else {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .and_then(|m| m.description.clone())
                    .unwrap_or_default()
            };
            if let Some(new_desc) = edit_in_editor(&current_desc, terminal) {
                if entity_type == "issue" {
                    if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                        item.description = Some(new_desc.clone());
                    }
                } else if entity_type == "mr" {
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.description = Some(new_desc.clone());
                    }
                }
                let args = UpdateCmd::new(cli.is_github, entity_type, iid)
                    .flag("-d", &new_desc)
                    .build();
                run_cli(&cli, &args, terminal, tx.clone(), tab).await;
            }
        }
        _ => {}
    }
}

fn get_default_template(template_type: &str) -> Option<String> {
    let paths = if template_type == "issue" {
        vec![
            ".github/issue_template.md",
            ".github/ISSUE_TEMPLATE.md",
            ".gitlab/issue_template.md",
        ]
    } else {
        vec![
            ".github/pull_request_template.md",
            ".github/PULL_REQUEST_TEMPLATE.md",
            ".gitlab/merge_request_template.md",
        ]
    };

    for path in &paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            return Some(content);
        }
    }

    let dirs = if template_type == "issue" {
        vec![".github/ISSUE_TEMPLATE", ".gitlab/issue_templates"]
    } else {
        vec![
            ".github/PULL_REQUEST_TEMPLATE",
            ".gitlab/merge_request_templates",
        ]
    };

    for dir in &dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut md_files = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().map(|ext| ext == "md").unwrap_or(false) {
                    md_files.push(path);
                }
            }
            md_files.sort();
            if let Some(default_path) = md_files
                .iter()
                .find(|p| p.file_name().map(|n| n == "default.md").unwrap_or(false))
            {
                if let Ok(content) = std::fs::read_to_string(default_path) {
                    return Some(content);
                }
            }
            if let Some(first_path) = md_files.first() {
                if let Ok(content) = std::fs::read_to_string(first_path) {
                    return Some(content);
                }
            }
        }
    }

    None
}

fn parse_key_value_pairs(input: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut current = String::new();
    let mut in_parens: i32 = 0;
    for c in input.chars() {
        match c {
            '(' => {
                in_parens += 1;
                current.push(c);
            }
            ')' => {
                in_parens = in_parens.saturating_sub(1);
                current.push(c);
            }
            ',' if in_parens == 0 => {
                if !current.trim().is_empty() {
                    if let Some(pos) = current.find(':').or_else(|| current.find('=')) {
                        let k = current[..pos].trim().to_string();
                        let v = current[pos + 1..].trim().to_string();
                        if !k.is_empty() {
                            pairs.push((k, v));
                        }
                    }
                }
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }
    if !current.trim().is_empty() {
        if let Some(pos) = current.find(':').or_else(|| current.find('=')) {
            let k = current[..pos].trim().to_string();
            let v = current[pos + 1..].trim().to_string();
            if !k.is_empty() {
                pairs.push((k, v));
            }
        }
    }
    pairs
}

fn get_current_branch() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !branch.is_empty() {
            return Some(branch);
        }
    }
    let output = std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !branch.is_empty() {
            return Some(branch);
        }
    }
    None
}

fn slugify(s: &str) -> String {
    let mut slug = String::with_capacity(s.len());
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if c.is_ascii() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

fn get_default_branch() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "origin/HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let branch = branch
            .strip_prefix("origin/")
            .unwrap_or(&branch)
            .to_string();
        if !branch.is_empty() && branch != "HEAD" {
            return Some(branch);
        }
    }
    None
}

fn get_branches() -> Vec<String> {
    let output = std::process::Command::new("git")
        .args(["branch", "-a"])
        .output()
        .ok();
    if let Some(output) = output {
        if output.status.success() {
            let mut branches: Vec<String> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| {
                    let line = line.trim();
                    if line.is_empty() {
                        return None;
                    }
                    let name = line.strip_prefix('*').unwrap_or(line).trim().to_string();
                    let name = name
                        .strip_prefix("remotes/origin/")
                        .unwrap_or(&name)
                        .to_string();
                    if name.is_empty() || name.contains(" -> ") {
                        return None;
                    }
                    Some(name)
                })
                .collect();
            branches.sort();
            branches.dedup();
            return branches;
        }
    }
    Vec::new()
}

fn keybinding_matches(binding: &str, event: &crossterm::event::KeyEvent) -> bool {
    match binding {
        "Tab" => event.code == KeyCode::Tab && event.modifiers.is_empty(),
        "Shift+Tab" => event.code == KeyCode::BackTab,
        "Enter" => event.code == KeyCode::Enter,
        "Esc" => event.code == KeyCode::Esc,
        "Backspace" => event.code == KeyCode::Backspace,
        "Space" => event.code == KeyCode::Char(' '),
        "Up" => event.code == KeyCode::Up,
        "Down" => event.code == KeyCode::Down,
        "Left" => event.code == KeyCode::Left,
        "Right" => event.code == KeyCode::Right,
        "Home" => event.code == KeyCode::Home,
        "End" => event.code == KeyCode::End,
        "PageUp" => event.code == KeyCode::PageUp,
        "PageDown" => event.code == KeyCode::PageDown,
        "F5" => event.code == KeyCode::F(5),
        other if other.starts_with("Ctrl+") && other.len() == 6 => {
            let c = other.as_bytes()[5];
            event.code == KeyCode::Char(c as char)
                && event
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL)
        }
        other if other.len() == 1 => {
            let c = other.chars().next().unwrap();
            event.code == KeyCode::Char(c) && event.modifiers.is_empty()
        }
        _ => false,
    }
}

fn spawn_refresh_active_tab(
    client: &gitlab::client::GitlabClient,
    project_context: &str,
    tab: app::Tab,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let client = client.clone();
    let project_context = project_context.to_string();
    tokio::spawn(async move {
        match tab {
            app::Tab::Issues => {
                match gitlab::issues::list_issues(&client, &project_context, true).await {
                    Ok(issues) => {
                        let _ = tx.send(Event::IssuesFetched(issues));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch issues: {}", e),
                        ));
                    }
                }
            }
            app::Tab::MergeRequests => {
                match gitlab::mr::list_mrs(&client, &project_context, true).await {
                    Ok(mrs) => {
                        let _ = tx.send(Event::MrsFetched(mrs));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch MRs: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Pipelines => {
                match gitlab::pipelines::list_pipelines(&client, &project_context).await {
                    Ok(pipelines) => {
                        let _ = tx.send(Event::PipelinesFetched(pipelines));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch pipelines: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Runners => {
                match gitlab::runners::list_runners(&client, &project_context).await {
                    Ok(runners) => {
                        let _ = tx.send(Event::RunnersFetched(runners));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch runners: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Releases => {
                match gitlab::releases::list_releases(&client, &project_context).await {
                    Ok(releases) => {
                        let _ = tx.send(Event::ReleasesFetched(releases));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch releases: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Todos => {
                match gitlab::notifications::list_notifications(&client, true).await {
                    Ok(notifs) => {
                        let _ = tx.send(Event::TodosFetched(notifs));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch notifications: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Jobs => {
                let branch_name = get_current_branch();
                let mut found_pipeline_id = None;

                if let Some(branch) = &branch_name {
                    let mr_iid = match gitlab::mr::list_mrs(&client, &project_context, false).await
                    {
                        Ok(mrs) => mrs
                            .into_iter()
                            .find(|m| &m.source_branch == branch)
                            .map(|m| m.iid),
                        Err(_) => None,
                    };

                    if let Ok(pipelines) =
                        gitlab::pipelines::list_pipelines(&client, &project_context).await
                    {
                        let target_ref =
                            mr_iid.map(|iid| format!("refs/merge-requests/{}/head", iid));
                        if let Some(pipeline) = pipelines.into_iter().find(|p| {
                            &p.r#ref == branch
                                || target_ref.as_ref().map_or(false, |tr| &p.r#ref == tr)
                        }) {
                            found_pipeline_id = Some(pipeline.id);
                        }
                    }
                }

                if let Some(pipeline_id) = found_pipeline_id {
                    match gitlab::pipelines::list_pipeline_jobs(
                        &client,
                        &project_context,
                        pipeline_id,
                    )
                    .await
                    {
                        Ok(jobs) => {
                            let _ = tx.send(Event::JobsTabFetched(pipeline_id, jobs));
                        }
                        Err(e) => {
                            let _ = tx.send(Event::FetchFailed(
                                tab,
                                format!("Failed to fetch jobs for pipeline {}: {}", pipeline_id, e),
                            ));
                        }
                    }
                } else {
                    let _ = tx.send(Event::FetchFailed(
                        tab,
                        "No pipeline found for the current branch/MR.".to_string(),
                    ));
                }
            }
            app::Tab::Milestones => {
                match gitlab::milestones::list_milestones(&client, &project_context).await {
                    Ok(milestones) => {
                        let _ = tx.send(Event::MilestonesFetched(milestones));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch milestones: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Terminal => {}
        }
    });
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut custom_repo: Option<String> = None;
    let mut custom_dir: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("glab-tui - GitLab/GitHub terminal user interface");
                println!();
                println!("Usage:");
                println!("  glab-tui [options]");
                println!();
                println!("Options:");
                println!("  -h, --help               Show this help message");
                println!("  -v, --version            Show version information");
                println!("  -u, --update             Check and install updates");
                println!("  -r, --repo <namespace>   Specify git repo context (e.g., group/repo)");
                println!("  -d, --dir <path>         Specify local repository directory to run in");
                return Ok(());
            }
            "-v" | "--version" => {
                println!("glab-tui version {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-u" | "--update" => {
                println!("Checking for updates...");
                match crate::utils::update::perform_self_update().await {
                    Ok(updated) => {
                        if updated {
                            println!(
                                "Successfully updated to the latest version! Please restart glab-tui."
                            );
                        } else {
                            println!("Already up to date.");
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!("Update failed: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            "-r" | "--repo" => {
                if i + 1 < args.len() {
                    custom_repo = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --repo option requires a namespace argument");
                    std::process::exit(1);
                }
            }
            "-d" | "--dir" => {
                if i + 1 < args.len() {
                    custom_dir = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --dir option requires a path argument");
                    std::process::exit(1);
                }
            }
            unknown => {
                eprintln!("Error: Unknown argument '{}'", unknown);
                eprintln!("Run 'glab-tui --help' for usage details.");
                std::process::exit(1);
            }
        }
    }

    if let Some(ref dir) = custom_dir {
        if let Err(e) = std::env::set_current_dir(dir) {
            eprintln!("Error changing directory to '{}': {}", dir, e);
            std::process::exit(1);
        }
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and event handler
    let mut app = App::new();
    let mut events = EventHandler::new(250);
    app.tx = Some(events.sender());

    // Initialize gitlab context
    if let Some(repo) = custom_repo {
        app.project_context = repo;
    } else if let Ok(context) = gitlab::client::get_project_context().await {
        app.project_context = context;
    }

    // Add current directory to recent repositories list
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(cwd_str) = cwd.to_str() {
            crate::utils::cache::add_recent_repo(cwd_str);
        }
    }

    // Load offline cache
    let cache = crate::utils::cache::load_cache(&app.project_context);
    app.issues.items = cache.issues;
    app.mrs.items = cache.mrs;
    app.pipelines.items = cache.pipelines;
    app.runners.items = cache.runners;
    app.releases.items = cache.releases;
    app.todos.items = cache.todos;
    app.milestones.items = cache.milestones;
    app.pipeline_jobs = cache.pipeline_jobs;

    if !app.issues.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Issues);
    }
    if !app.mrs.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::MergeRequests);
    }
    if !app.pipelines.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Pipelines);
    }
    if !app.runners.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Runners);
    }
    if !app.releases.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Releases);
    }
    if !app.todos.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Todos);
    }
    if !app.milestones.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Milestones);
    }
    app.update_filter_selection();

    if let Ok(mut client) = gitlab::client::GitlabClient::new().await {
        client.tx = Some(events.sender());
        app.gitlab_client = Some(client.clone());
        let tx = events.sender();
        if app.issues.items.is_empty() {
            app.start_loading_tab(app.active_tab);
        }
        spawn_refresh_active_tab(&client, &app.project_context, app.active_tab, tx.clone());
    } else {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        app.terminal_commands.push(crate::app::TerminalCommand {
            timestamp,
            command: "Initialization: gitlab client".to_string(),
            status: "Failed: Failed to initialize GitLab client".to_string(),
        });
        app.error_message = Some("Failed to initialize GitLab client".to_string());
    }

    let mut last_refresh = std::time::Instant::now();
    let mut last_active_tab = app.active_tab;

    // Run app
    while app.running {
        if app.active_tab == app::Tab::Pipelines {
            if let Some(client) = &app.gitlab_client {
                if let Some(idx) = app.pipelines.state.selected() {
                    let pipe_id = app.filtered_pipelines().get(idx).map(|p| p.id);
                    if let Some(pipe_id) = pipe_id {
                        if !app.pipeline_jobs.contains_key(&pipe_id)
                            && !app.fetching_pipelines.contains(&pipe_id)
                        {
                            app.fetching_pipelines.insert(pipe_id);
                            let client_clone = client.clone();
                            let project_context = app.project_context.clone();
                            let tx = events.sender();
                            tokio::spawn(async move {
                                if let Ok(jobs) = gitlab::pipelines::list_pipeline_jobs(
                                    &client_clone,
                                    &project_context,
                                    pipe_id,
                                )
                                .await
                                {
                                    let _ = tx.send(Event::PipelineJobs(pipe_id, jobs));
                                } else {
                                    let _ = tx.send(Event::PipelineJobs(pipe_id, vec![]));
                                }
                            });
                        }
                    }
                }
            }
        }

        if app.active_tab == app::Tab::Milestones {
            if let Some(client) = &app.gitlab_client {
                if let Some(idx) = app.milestones.state.selected() {
                    let milestone_iid = app.filtered_milestones().get(idx).map(|m| m.iid);
                    if let Some(iid) = milestone_iid {
                        if app.selected_milestone_iid != Some(iid) {
                            app.selected_milestone_iid = Some(iid);
                            app.selected_milestone_issues = None;
                            let client_clone = client.clone();
                            let project_context = app.project_context.clone();
                            let tx = events.sender();
                            tokio::spawn(async move {
                                if let Ok(issues) = gitlab::milestones::list_milestone_issues(
                                    &client_clone,
                                    &project_context,
                                    iid,
                                )
                                .await
                                {
                                    let _ = tx.send(Event::MilestoneIssuesFetched(iid, issues));
                                } else {
                                    let _ = tx.send(Event::MilestoneIssuesFetched(iid, vec![]));
                                }
                            });
                        }
                    }
                }
            }
        }

        terminal.draw(|f| ui::render(f, &mut app))?;

        if let Some(event) = events.next().await {
            match event {
                Event::Tick => {
                    app.tick();
                    if app.active_tab != last_active_tab {
                        last_active_tab = app.active_tab;
                        last_refresh = std::time::Instant::now();
                    } else if last_refresh.elapsed() >= std::time::Duration::from_secs(60) {
                        if app.text_input.is_none()
                            && app.edit_menu.is_none()
                            && app.selector.is_none()
                            && !app.loading_tabs.contains(&app.active_tab)
                        {
                            if let Some(client) = app.gitlab_client.clone() {
                                app.start_loading_tab(app.active_tab);
                                spawn_refresh_active_tab(
                                    &client,
                                    &app.project_context,
                                    app.active_tab,
                                    events.sender(),
                                );
                            }
                        }
                        last_refresh = std::time::Instant::now();
                    }
                }
                Event::PipelineJobs(id, jobs) => {
                    app.fetching_pipelines.remove(&id);
                    app.pipeline_jobs.insert(id, jobs.clone());

                    let mut is_active = false;
                    if app.active_tab == app::Tab::Jobs && app.active_pipeline_id == Some(id) {
                        is_active = true;
                    } else if app.active_tab == app::Tab::Pipelines {
                        if let Some(idx) = app.pipelines.state.selected() {
                            if app.filtered_pipelines().get(idx).map(|p| p.id) == Some(id) {
                                is_active = true;
                            }
                        }
                    }

                    if is_active {
                        app.selected_pipeline_jobs = Some(jobs);
                        app.jobs_list_state
                            .select(app.selected_job_index.or(Some(0)));
                    }

                    let mut cache = crate::utils::cache::load_cache(&app.project_context);
                    cache.pipeline_jobs = app.pipeline_jobs.clone();
                    crate::utils::cache::save_cache(&app.project_context, &cache);
                }
                Event::JobsTabFetched(pipeline_id, jobs) => {
                    app.complete_loading_tab(app::Tab::Jobs, "Success");
                    app.loaded_tabs.insert(app::Tab::Jobs);
                    app.selected_pipeline_jobs = Some(jobs);
                    app.active_pipeline_id = Some(pipeline_id);
                    app.selected_job_index = Some(0);
                    app.jobs_list_state.select(Some(0));
                    app.job_trace_scroll = 0;
                    app.job_trace = None;
                }
                Event::JobTraceFetched(job_id, result) => {
                    app.job_trace_loading = false;
                    let current_selected_job_id = match app.active_tab {
                        app::Tab::Jobs => {
                            if let Some(idx) = app.selected_job_index {
                                app.filtered_jobs().get(idx).map(|j| j.id)
                            } else {
                                None
                            }
                        }
                        app::Tab::Pipelines => {
                            if let Some(idx) = app.selected_job_index {
                                app.selected_pipeline_jobs
                                    .as_ref()
                                    .and_then(|jobs| jobs.get(idx).map(|j| j.id))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    if current_selected_job_id == Some(job_id) {
                        match result {
                            Ok(trace) => {
                                app.job_trace = Some(trace);
                                app.job_trace_needs_scroll_to_bottom = true;
                                app.details_zoomed = true;
                            }
                            Err(e) => {
                                app.error_message = Some(e);
                            }
                        }
                    }
                }
                Event::IssuesFetched(issues) => {
                    app.complete_loading_tab(app::Tab::Issues, "Success");
                    app.loaded_tabs.insert(app::Tab::Issues);
                    app.refreshed_tabs.insert(app::Tab::Issues);
                    app.status_message = None;
                    app.issues.items = issues;
                    app.update_filter_selection();
                    let mut cache = crate::utils::cache::load_cache(&app.project_context);
                    cache.issues = app.issues.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &cache);
                }
                Event::MrsFetched(mrs) => {
                    app.complete_loading_tab(app::Tab::MergeRequests, "Success");
                    app.loaded_tabs.insert(app::Tab::MergeRequests);
                    app.refreshed_tabs.insert(app::Tab::MergeRequests);
                    app.status_message = None;
                    app.mrs.items = mrs;
                    app.update_filter_selection();
                    let mut cache = crate::utils::cache::load_cache(&app.project_context);
                    cache.mrs = app.mrs.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &cache);
                }
                Event::PipelinesFetched(pipelines) => {
                    app.complete_loading_tab(app::Tab::Pipelines, "Success");
                    app.loaded_tabs.insert(app::Tab::Pipelines);
                    app.refreshed_tabs.insert(app::Tab::Pipelines);
                    app.status_message = None;
                    app.pipelines.items = pipelines;
                    app.update_filter_selection();
                    let new_ids: std::collections::HashSet<u64> =
                        app.pipelines.items.iter().map(|p| p.id).collect();
                    app.pipeline_jobs.retain(|id, _| new_ids.contains(id));
                    app.fetching_pipelines.clear();
                    let mut cache = crate::utils::cache::load_cache(&app.project_context);
                    cache.pipelines = app.pipelines.items.clone();
                    cache.pipeline_jobs = app.pipeline_jobs.clone();
                    crate::utils::cache::save_cache(&app.project_context, &cache);
                }
                Event::TodosFetched(notifs) => {
                    app.complete_loading_tab(app::Tab::Todos, "Success");
                    app.loaded_tabs.insert(app::Tab::Todos);
                    app.refreshed_tabs.insert(app::Tab::Todos);
                    app.status_message = None;
                    app.todos.items = notifs;
                    app.update_filter_selection();
                    let mut cache = crate::utils::cache::load_cache(&app.project_context);
                    cache.todos = app.todos.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &cache);
                }
                Event::RunnersFetched(runners) => {
                    app.complete_loading_tab(app::Tab::Runners, "Success");
                    app.loaded_tabs.insert(app::Tab::Runners);
                    app.refreshed_tabs.insert(app::Tab::Runners);
                    app.status_message = None;
                    app.runners.items = runners;
                    app.update_filter_selection();
                    let mut cache = crate::utils::cache::load_cache(&app.project_context);
                    cache.runners = app.runners.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &cache);
                }
                Event::ReleasesFetched(releases) => {
                    app.complete_loading_tab(app::Tab::Releases, "Success");
                    app.loaded_tabs.insert(app::Tab::Releases);
                    app.refreshed_tabs.insert(app::Tab::Releases);
                    app.status_message = None;
                    app.releases.items = releases;
                    app.update_filter_selection();
                    let mut cache = crate::utils::cache::load_cache(&app.project_context);
                    cache.releases = app.releases.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &cache);
                }
                Event::MilestonesFetched(milestones) => {
                    app.complete_loading_tab(app::Tab::Milestones, "Success");
                    app.loaded_tabs.insert(app::Tab::Milestones);
                    app.refreshed_tabs.insert(app::Tab::Milestones);
                    app.status_message = None;
                    app.milestones.items = milestones;
                    app.update_filter_selection();
                    let mut cache = crate::utils::cache::load_cache(&app.project_context);
                    cache.milestones = app.milestones.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &cache);
                }
                Event::MilestoneIssuesFetched(_, issues) => {
                    app.selected_milestone_issues = Some(issues);
                }
                Event::SelectorItemsFetched(items) => {
                    if let Some(mut selector) = app.selector.take() {
                        selector.all_items = items;
                        selector.is_loading = false;
                        app.selector = Some(selector);
                    }
                }
                Event::FetchFailed(tab, err_msg) => {
                    app.complete_loading_tab(tab, &format!("Failed: {}", err_msg));
                    let has_cached_items = match tab {
                        app::Tab::Issues => !app.issues.items.is_empty(),
                        app::Tab::MergeRequests => !app.mrs.items.is_empty(),
                        app::Tab::Pipelines => !app.pipelines.items.is_empty(),
                        app::Tab::Runners => !app.runners.items.is_empty(),
                        app::Tab::Releases => !app.releases.items.is_empty(),
                        app::Tab::Todos => !app.todos.items.is_empty(),
                        app::Tab::Milestones => !app.milestones.items.is_empty(),
                        _ => false,
                    };
                    if has_cached_items {
                        app.status_message = Some("Offline / Connection failed".to_string());
                    } else {
                        app.error_message = Some(err_msg);
                    }
                }
                Event::DiffFetched {
                    mr_iid,
                    raw_diff,
                    comments,
                } => {
                    app.diff_loading = false;
                    app.diff_view = Some(crate::app::DiffView::new(mr_iid, raw_diff));
                    app.current_comments = comments;
                    app.last_fetched_mr_iid = Some(mr_iid);
                    app.in_review_mode = true;
                    if let Some(pos) = app
                        .terminal_commands
                        .iter()
                        .rposition(|cmd| cmd.command.contains("diff") && cmd.status == "Running")
                    {
                        app.terminal_commands[pos].status = "Success".to_string();
                    }
                }
                Event::DiffFetchFailed(err_msg) => {
                    app.diff_loading = false;
                    app.error_message = Some(err_msg.clone());
                    if let Some(pos) = app
                        .terminal_commands
                        .iter()
                        .rposition(|cmd| cmd.command.contains("diff") && cmd.status == "Running")
                    {
                        app.terminal_commands[pos].status = format!("Failed: {}", err_msg);
                    }
                }
                Event::TerminalCommandLogged {
                    timestamp,
                    command,
                    status,
                } => {
                    if status == "Running" {
                        app.terminal_commands.push(crate::app::TerminalCommand {
                            timestamp,
                            command,
                            status,
                        });
                    } else if let Some(pos) = app
                        .terminal_commands
                        .iter()
                        .rposition(|cmd| cmd.command == command && cmd.status == "Running")
                    {
                        app.terminal_commands[pos].status = status;
                    } else {
                        app.terminal_commands.push(crate::app::TerminalCommand {
                            timestamp,
                            command,
                            status,
                        });
                    }
                }
                Event::CommandStarted(msg) => {
                    let timestamp = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
                    app.terminal_commands.push(crate::app::TerminalCommand {
                        timestamp,
                        command: msg,
                        status: "Running".to_string(),
                    });
                    // Force an immediate render so the "Running..." banner is visible
                    // even if CommandCompleted arrives in the very next event.
                    terminal.draw(|f| ui::render(f, &mut app))?;
                }
                Event::CommandCompleted(tab, res) => {
                    let status = match &res {
                        Ok(_) => "Success".to_string(),
                        Err(e) => format!("Failed: {}", e),
                    };
                    if let Some(pos) = app.terminal_commands.iter().rposition(|cmd| {
                        (cmd.command.contains("glab")
                            || cmd.command.contains("gh")
                            || cmd.command.contains("submit")
                            || cmd.command.contains("bulk"))
                            && cmd.status == "Running"
                    }) {
                        app.terminal_commands[pos].status = status;
                    }
                    match res {
                        Ok(_) => {
                            if let Some(client) = app.gitlab_client.clone() {
                                if !app.loading_tabs.contains(&tab) {
                                    app.start_loading_tab(tab);
                                }
                                spawn_refresh_active_tab(
                                    &client,
                                    &app.project_context,
                                    tab,
                                    events.sender(),
                                );
                            }
                            if let Some(diff_view) = &app.diff_view {
                                let client = app.gitlab_client.clone();
                                let project_context = app.project_context.clone();
                                let tx = events.sender();
                                let mr_iid = diff_view.mr_iid;
                                let mr_iid_str = mr_iid.to_string();
                                tokio::spawn(async move {
                                    let is_github = match tokio::process::Command::new("git")
                                        .args(["remote", "get-url", "origin"])
                                        .output()
                                        .await
                                        .map(|o| {
                                            String::from_utf8_lossy(&o.stdout)
                                                .contains("github.com")
                                        }) {
                                        Ok(true) => true,
                                        _ => false,
                                    };

                                    let program = if is_github { "gh" } else { "glab" };
                                    let (entity, sub) = if is_github {
                                        ("pr", "diff")
                                    } else {
                                        ("mr", "diff")
                                    };
                                    let cmd_args = vec![
                                        entity.to_string(),
                                        sub.to_string(),
                                        mr_iid_str.clone(),
                                    ];

                                    let mut cmd = tokio::process::Command::new(program);
                                    cmd.args(&cmd_args);

                                    let diff_res = cmd.output().await;

                                    let comments = if let Some(ref c) = client {
                                        crate::gitlab::mr::list_mr_notes(
                                            c,
                                            &project_context,
                                            mr_iid,
                                        )
                                        .await
                                        .unwrap_or_default()
                                    } else {
                                        vec![]
                                    };

                                    if let Ok(output) = diff_res {
                                        if output.status.success() {
                                            let raw_diff = String::from_utf8_lossy(&output.stdout)
                                                .into_owned();
                                            let _ = tx.send(Event::DiffFetched {
                                                mr_iid,
                                                raw_diff,
                                                comments,
                                            });
                                        }
                                    }
                                });
                            }
                        }
                        Err(err) => {
                            app.error_message = Some(err);
                        }
                    }
                }
                Event::Key(key_event) => {
                    if key_event.code == KeyCode::Char('c')
                        && key_event
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                    {
                        app.quit();
                        continue;
                    }

                    if keybinding_matches(&app.config.keybindings.global.quit, &key_event)
                        && app.text_input.is_none()
                        && app.edit_menu.is_none()
                        && app.selector.is_none()
                        && !app.focus_column_checklist
                    {
                        app.quit();
                        continue;
                    }

                    if let Some(mr_iid) = app.show_submit_review_prompt {
                        match key_event.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                                app.show_submit_review_prompt = None;
                                app.selector = Some(crate::app::Selector {
                                    title: " Submit Pull Request Review ".to_string(),
                                    all_items: vec![
                                        "Approve".to_string(),
                                        "Request Changes".to_string(),
                                        "Comment".to_string(),
                                    ],
                                    selected_items: std::collections::HashSet::new(),
                                    cursor_idx: 0,
                                    search_query: String::new(),
                                    is_filtering: false,
                                    is_loading: false,
                                    entity_iid: mr_iid,
                                    entity_type: "mr".to_string(),
                                    field_type: "review_submit_status".to_string(),
                                    multi_select: false,
                                    state: ListState::default(),
                                });
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') => {
                                app.show_submit_review_prompt = None;
                                app.draft_comments.clear();
                                app.in_review_mode = false;
                                app.diff_view = None;
                            }
                            KeyCode::Esc | KeyCode::Char('q') => {
                                app.show_submit_review_prompt = None;
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if keybinding_matches(&app.config.keybindings.global.help, &key_event)
                        && app.text_input.is_none()
                        && app.edit_menu.is_none()
                        && app.selector.is_none()
                        && !app.show_help
                        && !app.focus_column_checklist
                    {
                        app.show_help = true;
                        app.help_search_query.clear();
                        continue;
                    }

                    if app.show_help {
                        match key_event.code {
                            KeyCode::Esc | KeyCode::Enter => {
                                app.show_help = false;
                                app.help_search_query.clear();
                            }
                            KeyCode::Backspace => {
                                app.help_search_query.pop();
                            }
                            KeyCode::Char(c) => {
                                app.help_search_query.push(c);
                            }
                            _ => {}
                        }
                        continue;
                    }

                    let is_switch_repo = (key_event.code == KeyCode::Char('s')
                        && key_event
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL))
                        || (key_event.code == KeyCode::Char('S')
                            && key_event
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL));

                    if is_switch_repo
                        && app.text_input.is_none()
                        && app.edit_menu.is_none()
                        && app.selector.is_none()
                    {
                        let items = crate::utils::cache::get_switchable_repos();

                        app.selector = Some(crate::app::Selector {
                            title: " Switch Repository ".to_string(),
                            all_items: items,
                            selected_items: {
                                let mut s = std::collections::HashSet::new();
                                if let Ok(cwd) = std::env::current_dir() {
                                    if let Some(name) = cwd.file_name().and_then(|n| n.to_str()) {
                                        s.insert(name.to_string());
                                    }
                                }
                                s
                            },
                            cursor_idx: 0,
                            search_query: String::new(),
                            is_filtering: true,
                            is_loading: false,
                            entity_iid: 0,
                            entity_type: "app".to_string(),
                            field_type: "switch_repo".to_string(),
                            multi_select: false,
                            state: {
                                let mut s = ListState::default();
                                s.select(Some(0));
                                s
                            },
                        });
                        continue;
                    }

                    let is_refresh = key_event.code == KeyCode::F(5)
                        || (key_event.code == KeyCode::Char('r')
                            && key_event
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL))
                        || (key_event.code == KeyCode::Char('R')
                            && key_event
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL))
                        || keybinding_matches(&app.config.keybindings.global.refresh, &key_event);

                    if is_refresh
                        && app.text_input.is_none()
                        && app.date_picker.is_none()
                        && app.edit_menu.is_none()
                        && app.selector.is_none()
                    {
                        last_refresh = std::time::Instant::now();
                        if let Some(client) = app.gitlab_client.clone() {
                            if !app.loading_tabs.contains(&app.active_tab) {
                                app.start_loading_tab(app.active_tab);
                                spawn_refresh_active_tab(
                                    &client,
                                    &app.project_context,
                                    app.active_tab,
                                    events.sender(),
                                );
                            }
                        }
                        continue;
                    }

                    if let Some(mut date_picker) = app.date_picker.take() {
                        match key_event.code {
                            KeyCode::Esc | KeyCode::Char('q') => {
                                // Cancel date picker
                            }
                            KeyCode::Char('h') | KeyCode::Left => {
                                date_picker.move_day(-1);
                                app.date_picker = Some(date_picker);
                            }
                            KeyCode::Char('l') | KeyCode::Right => {
                                date_picker.move_day(1);
                                app.date_picker = Some(date_picker);
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                date_picker.move_day(-7);
                                app.date_picker = Some(date_picker);
                            }
                            KeyCode::Char('j') | KeyCode::Down => {
                                date_picker.move_day(7);
                                app.date_picker = Some(date_picker);
                            }
                            KeyCode::Char('[') | KeyCode::PageUp => {
                                date_picker.move_month(-1);
                                app.date_picker = Some(date_picker);
                            }
                            KeyCode::Char(']') | KeyCode::PageDown => {
                                date_picker.move_month(1);
                                app.date_picker = Some(date_picker);
                            }
                            KeyCode::Enter => {
                                let selected_val = date_picker.value_string();
                                match date_picker.action {
                                    crate::app::DatePickerAction::EditField {
                                        entity_iid,
                                        entity_type,
                                        field_type,
                                    } => {
                                        let active_tab = app.active_tab;
                                        apply_field_text_change(
                                            &mut app,
                                            &entity_type,
                                            entity_iid,
                                            &field_type,
                                            selected_val,
                                            &mut terminal,
                                            events.sender(),
                                            active_tab,
                                        )
                                        .await;
                                        rebuild_edit_menu(&mut app, &entity_type, entity_iid);
                                    }
                                    crate::app::DatePickerAction::EditNewField { field_idx } => {
                                        if let Some(ref mut menu) = app.edit_menu {
                                            if field_idx < menu.fields.len() {
                                                menu.fields[field_idx].1 = selected_val;
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {
                                app.date_picker = Some(date_picker);
                            }
                        }
                        continue;
                    }

                    if let Some(mut text_input) = app.text_input.take() {
                        if key_event
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                            && key_event.code == KeyCode::Char('e')
                        {
                            if let Some(new_val) = edit_in_editor(&text_input.value, &mut terminal)
                            {
                                text_input.value = new_val.clone();
                                text_input.cursor_idx = new_val.len();
                            }
                            app.text_input = Some(text_input);
                            continue;
                        }
                        match key_event.code {
                            KeyCode::Esc => {
                                // Cancel
                            }
                            KeyCode::Backspace => {
                                if text_input.cursor_idx > 0 {
                                    text_input.value.remove(text_input.cursor_idx - 1);
                                    text_input.cursor_idx -= 1;
                                }
                                app.text_input = Some(text_input);
                            }
                            KeyCode::Delete => {
                                if text_input.cursor_idx < text_input.value.len() {
                                    text_input.value.remove(text_input.cursor_idx);
                                }
                                app.text_input = Some(text_input);
                            }
                            KeyCode::Left => {
                                if text_input.cursor_idx > 0 {
                                    text_input.cursor_idx -= 1;
                                }
                                app.text_input = Some(text_input);
                            }
                            KeyCode::Right => {
                                if text_input.cursor_idx < text_input.value.len() {
                                    text_input.cursor_idx += 1;
                                }
                                app.text_input = Some(text_input);
                            }
                            KeyCode::Char(c) => {
                                text_input.value.insert(text_input.cursor_idx, c);
                                text_input.cursor_idx += 1;
                                app.text_input = Some(text_input);
                            }
                            KeyCode::Enter => {
                                let value = text_input.value.clone();
                                match text_input.action {
                                    crate::app::TextInputAction::EditField {
                                        entity_iid,
                                        entity_type,
                                        field_type,
                                    } => {
                                        let active_tab = app.active_tab;
                                        apply_field_text_change(
                                            &mut app,
                                            &entity_type,
                                            entity_iid,
                                            &field_type,
                                            value,
                                            &mut terminal,
                                            events.sender(),
                                            active_tab,
                                        )
                                        .await;
                                        rebuild_edit_menu(&mut app, &entity_type, entity_iid);
                                    }
                                    crate::app::TextInputAction::CreateIssue => {
                                        if !value.trim().is_empty() {
                                            let cli = app_cli(&app);
                                            let mut args: Vec<String> =
                                                vec!["issue".into(), "create".into()];
                                            if !cli.is_github {
                                                args.push("-y".into());
                                            }
                                            args.push("--title".into());
                                            args.push(value.clone());
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                        }
                                    }
                                    crate::app::TextInputAction::AddReviewComment {
                                        mr_iid,
                                        file_path,
                                        line_num,
                                        old_line_num,
                                        end_line_num,
                                        end_old_line_num,
                                    } => {
                                        if !value.trim().is_empty() {
                                            if app.in_review_mode {
                                                app.draft_comments.push(crate::app::DraftComment {
                                                    file_path,
                                                    line_num,
                                                    old_line_num,
                                                    end_line_num,
                                                    end_old_line_num,
                                                    body: value,
                                                });
                                                app.status_message = Some(format!(
                                                    "Added draft comment. ({} pending)",
                                                    app.draft_comments.len()
                                                ));
                                            } else {
                                                let cli = app_cli(&app);
                                                let mut args = if cli.is_github {
                                                    vec![
                                                        "pr".to_string(),
                                                        "comment".to_string(),
                                                        mr_iid.to_string(),
                                                        "--body".to_string(),
                                                        value,
                                                    ]
                                                } else {
                                                    vec![
                                                        "mr".to_string(),
                                                        "note".to_string(),
                                                        "create".to_string(),
                                                        mr_iid.to_string(),
                                                        "--file-path".to_string(),
                                                        file_path,
                                                        "-m".to_string(),
                                                        value,
                                                    ]
                                                };
                                                if !cli.is_github {
                                                    if let Some(line) = line_num {
                                                        args.push("--line".to_string());
                                                        args.push(line.to_string());
                                                    } else if let Some(old_line) = old_line_num {
                                                        args.push("--old-line".to_string());
                                                        args.push(old_line.to_string());
                                                    }
                                                }
                                                run_cli(
                                                    &cli,
                                                    &args,
                                                    &mut terminal,
                                                    events.sender(),
                                                    app.active_tab,
                                                )
                                                .await;
                                            }
                                        }
                                    }
                                    crate::app::TextInputAction::EnterPipelineId => {
                                        if let Ok(pipeline_id) = value.trim().parse::<u64>() {
                                            if let Some(client) = &app.gitlab_client {
                                                app.loading_tabs.insert(app::Tab::Jobs);
                                                let client_clone = client.clone();
                                                let project_context = app.project_context.clone();
                                                let tx = events.sender();
                                                tokio::spawn(async move {
                                                    match gitlab::pipelines::list_pipeline_jobs(
                                                        &client_clone,
                                                        &project_context,
                                                        pipeline_id,
                                                    )
                                                    .await
                                                    {
                                                        Ok(jobs) => {
                                                            let _ = tx.send(Event::JobsTabFetched(
                                                                pipeline_id,
                                                                jobs,
                                                            ));
                                                        }
                                                        Err(e) => {
                                                            let _ = tx.send(Event::FetchFailed(app::Tab::Jobs, format!("Failed to fetch jobs for pipeline {}: {}", pipeline_id, e)));
                                                        }
                                                    }
                                                });
                                            }
                                        } else {
                                            app.error_message =
                                                Some("Invalid pipeline ID".to_string());
                                        }
                                    }
                                    crate::app::TextInputAction::CreateRelease => {
                                        if !value.trim().is_empty() {
                                            let tag_name = value.trim().to_string();
                                            let tx = events.sender();
                                            let is_github = app
                                                .gitlab_client
                                                .as_ref()
                                                .map(|c| c.is_github)
                                                .unwrap_or(false);
                                            let program = if is_github { "gh" } else { "glab" };
                                            let _ = tx.send(Event::CommandStarted(format!(
                                                "Creating Release: {} release create {}",
                                                program, tag_name
                                            )));
                                            let active_tab = app.active_tab;
                                            tokio::spawn(async move {
                                                let last_tag = if let Ok(output) =
                                                    tokio::process::Command::new("git")
                                                        .args(["describe", "--tags", "--abbrev=0"])
                                                        .output()
                                                        .await
                                                {
                                                    let t = String::from_utf8_lossy(&output.stdout)
                                                        .trim()
                                                        .to_string();
                                                    if t.is_empty() { None } else { Some(t) }
                                                } else {
                                                    None
                                                };

                                                let log_args = if let Some(ref tag) = last_tag {
                                                    vec![
                                                        "log".to_string(),
                                                        format!("{}..HEAD", tag),
                                                        "--oneline".to_string(),
                                                    ]
                                                } else {
                                                    vec!["log".to_string(), "--oneline".to_string()]
                                                };

                                                let commits = if let Ok(output) =
                                                    tokio::process::Command::new("git")
                                                        .args(&log_args)
                                                        .output()
                                                        .await
                                                {
                                                    String::from_utf8_lossy(&output.stdout)
                                                        .lines()
                                                        .map(|line| {
                                                            let parts: Vec<&str> =
                                                                line.splitn(2, ' ').collect();
                                                            if parts.len() == 2 {
                                                                format!(
                                                                    "- {} ({})",
                                                                    parts[1], parts[0]
                                                                )
                                                            } else {
                                                                format!("- {}", line)
                                                            }
                                                        })
                                                        .collect::<Vec<_>>()
                                                        .join("\n")
                                                } else {
                                                    "".to_string()
                                                };

                                                let title_range = if let Some(ref tag) = last_tag {
                                                    format!("Changes since {}", tag)
                                                } else {
                                                    "All Changes".to_string()
                                                };

                                                let changelog = format!(
                                                    "## Release Notes\n\n### {}\n\n{}\n",
                                                    title_range,
                                                    if commits.is_empty() {
                                                        "- No changes found".to_string()
                                                    } else {
                                                        commits
                                                    }
                                                );

                                                let temp_path = std::env::temp_dir().join(format!(
                                                    "glab-tui-release-{}.md",
                                                    tag_name
                                                ));
                                                if let Ok(_) =
                                                    std::fs::write(&temp_path, &changelog)
                                                {
                                                    let temp_str =
                                                        temp_path.to_string_lossy().to_string();

                                                    let is_github =
                                                        match tokio::process::Command::new("git")
                                                            .args(["remote", "get-url", "origin"])
                                                            .output()
                                                            .await
                                                        {
                                                            Ok(output)
                                                                if output.status.success() =>
                                                            {
                                                                let url = String::from_utf8_lossy(
                                                                    &output.stdout,
                                                                );
                                                                url.contains("github.com")
                                                            }
                                                            _ => false,
                                                        };

                                                    let program =
                                                        if is_github { "gh" } else { "glab" };
                                                    let args = [
                                                        "release", "create", &tag_name, "-F",
                                                        &temp_str,
                                                    ];

                                                    let mut cmd =
                                                        tokio::process::Command::new(program);
                                                    cmd.args(&args);

                                                    match cmd.output().await {
                                                        Ok(output) => {
                                                            let _ =
                                                                std::fs::remove_file(&temp_path);
                                                            if output.status.success() {
                                                                let _ = tx.send(
                                                                    Event::CommandCompleted(
                                                                        active_tab,
                                                                        Ok(()),
                                                                    ),
                                                                );
                                                            } else {
                                                                let err_msg =
                                                                    String::from_utf8_lossy(
                                                                        &output.stderr,
                                                                    )
                                                                    .trim()
                                                                    .to_string();
                                                                let _ = tx.send(
                                                                    Event::CommandCompleted(
                                                                        active_tab,
                                                                        Err(format!(
                                                                            "Command failed: {}",
                                                                            err_msg
                                                                        )),
                                                                    ),
                                                                );
                                                            }
                                                        }
                                                        Err(e) => {
                                                            let _ =
                                                                std::fs::remove_file(&temp_path);
                                                            let _ = tx.send(Event::CommandCompleted(
                                                                active_tab,
                                                                Err(format!("Failed to execute command: {}", e)),
                                                            ));
                                                        }
                                                    }
                                                } else {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        active_tab,
                                                        Err("Failed to write temporary changelog file".to_string()),
                                                    ));
                                                }
                                            });
                                        }
                                    }
                                    crate::app::TextInputAction::CreateMilestone => {
                                        if !value.trim().is_empty() {
                                            let title = value.trim().to_string();
                                            let is_github = app
                                                .gitlab_client
                                                .as_ref()
                                                .map(|c| c.is_github)
                                                .unwrap_or(false);
                                            let project_context = app.project_context.clone();
                                            let encoded_path = project_context.replace("/", "%2F");
                                            let tx = events.sender();
                                            let _ = tx.send(Event::CommandStarted(format!(
                                                "Creating milestone: {}",
                                                title
                                            )));
                                            tokio::spawn(async move {
                                                if is_github {
                                                    let gh_repo = encoded_path.replace("%2F", "/");
                                                    let cmd = tokio::process::Command::new("gh")
                                                        .args([
                                                            "api",
                                                            &format!(
                                                                "repos/{}/milestones",
                                                                gh_repo
                                                            ),
                                                            "-f",
                                                            &format!("title={}", title),
                                                        ])
                                                        .output()
                                                        .await;
                                                    match cmd {
                                                        Ok(out) if out.status.success() => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::Milestones,
                                                                    Ok(()),
                                                                ));
                                                        }
                                                        Ok(out) => {
                                                            let err = String::from_utf8_lossy(
                                                                &out.stderr,
                                                            )
                                                            .trim()
                                                            .to_string();
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::Milestones,
                                                                    Err(format!("Failed: {}", err)),
                                                                ));
                                                        }
                                                        Err(e) => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::Milestones,
                                                                    Err(format!("Error: {}", e)),
                                                                ));
                                                        }
                                                    }
                                                } else {
                                                    let endpoint = format!(
                                                        "/projects/{}/milestones",
                                                        encoded_path
                                                    );
                                                    let cmd = tokio::process::Command::new("glab")
                                                        .args([
                                                            "api",
                                                            "-X",
                                                            "POST",
                                                            &endpoint,
                                                            "-f",
                                                            &format!("title={}", title),
                                                        ])
                                                        .output()
                                                        .await;
                                                    match cmd {
                                                        Ok(out) if out.status.success() => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::Milestones,
                                                                    Ok(()),
                                                                ));
                                                        }
                                                        Ok(out) => {
                                                            let err = String::from_utf8_lossy(
                                                                &out.stderr,
                                                            )
                                                            .trim()
                                                            .to_string();
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::Milestones,
                                                                    Err(format!("Failed: {}", err)),
                                                                ));
                                                        }
                                                        Err(e) => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::Milestones,
                                                                    Err(format!("Error: {}", e)),
                                                                ));
                                                        }
                                                    }
                                                }
                                            });
                                        }
                                    }
                                    crate::app::TextInputAction::ReplyToComment {
                                        mr_iid,
                                        comment_id,
                                        ref discussion_id,
                                    } => {
                                        if !value.trim().is_empty() {
                                            let client = app.gitlab_client.clone();
                                            let project_context = app.project_context.clone();
                                            let tx = events.sender();
                                            let is_github =
                                                client.as_ref().map_or(false, |c| c.is_github);
                                            let discussion_id_clone = discussion_id.clone();
                                            let value_clone = value.clone();

                                            let _ = tx.send(Event::CommandStarted(format!(
                                                "Replying to comment ID {} in MR #{}",
                                                comment_id, mr_iid
                                            )));

                                            tokio::spawn(async move {
                                                if let Some(client) = client {
                                                    let output = if is_github {
                                                        let payload = serde_json::json!({
                                                            "body": value_clone,
                                                            "in_reply_to": comment_id,
                                                        });
                                                        let temp_path =
                                                            std::env::temp_dir().join(format!(
                                                                "glab-tui-reply-{}.json",
                                                                comment_id
                                                            ));
                                                        let _ = std::fs::write(
                                                            &temp_path,
                                                            serde_json::to_string(&payload)
                                                                .unwrap(),
                                                        );
                                                        let temp_str =
                                                            temp_path.to_string_lossy().to_string();

                                                        let res = tokio::process::Command::new(
                                                            "gh",
                                                        )
                                                        .args([
                                                            "api",
                                                            &format!(
                                                                "repos/{}/pulls/{}/comments",
                                                                project_context, mr_iid
                                                            ),
                                                            "--input",
                                                            &temp_str,
                                                            "-X",
                                                            "POST",
                                                        ])
                                                        .output()
                                                        .await;
                                                        let _ = std::fs::remove_file(&temp_path);
                                                        res
                                                    } else {
                                                        let encoded_path =
                                                            project_context.replace("/", "%2F");
                                                        let payload = serde_json::json!({
                                                            "body": value_clone,
                                                        });
                                                        let temp_path =
                                                            std::env::temp_dir().join(format!(
                                                                "glab-tui-reply-{}.json",
                                                                comment_id
                                                            ));
                                                        let _ = std::fs::write(
                                                            &temp_path,
                                                            serde_json::to_string(&payload)
                                                                .unwrap(),
                                                        );
                                                        let temp_str =
                                                            temp_path.to_string_lossy().to_string();

                                                        let res = tokio::process::Command::new("glab")
                                                            .args([
                                                                "api",
                                                                &format!(
                                                                    "projects/{}/merge_requests/{}/discussions/{}/notes",
                                                                    encoded_path, mr_iid, discussion_id_clone
                                                                ),
                                                                "--input",
                                                                &temp_str,
                                                                "-X",
                                                                "POST"
                                                            ])
                                                            .output()
                                                            .await;
                                                        let _ = std::fs::remove_file(&temp_path);
                                                        res
                                                    };

                                                    match output {
                                                        Ok(out) if out.status.success() => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::MergeRequests,
                                                                    Ok(()),
                                                                ));
                                                        }
                                                        Ok(out) => {
                                                            let err = String::from_utf8_lossy(
                                                                &out.stderr,
                                                            )
                                                            .trim()
                                                            .to_string();
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::MergeRequests,
                                                                    Err(err),
                                                                ));
                                                        }
                                                        Err(e) => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::MergeRequests,
                                                                    Err(e.to_string()),
                                                                ));
                                                        }
                                                    }
                                                }
                                            });
                                        }
                                    }
                                    crate::app::TextInputAction::SubmitReviewFinal {
                                        mr_iid,
                                        status,
                                    } => {
                                        let is_github = app
                                            .gitlab_client
                                            .as_ref()
                                            .map_or(false, |c| c.is_github);
                                        let tx = events.sender();
                                        let comments = app.draft_comments.clone();
                                        app.draft_comments.clear();
                                        app.in_review_mode = false;

                                        let project_context = app.project_context.clone();
                                        let status_clone = status.clone();
                                        let value_clone = value.clone();

                                        tokio::spawn(async move {
                                            if is_github {
                                                let github_event = match status_clone.as_str() {
                                                    "Approve" => "APPROVE",
                                                    "Request Changes" => "REQUEST_CHANGES",
                                                    _ => "COMMENT",
                                                };
                                                let mut json_comments = serde_json::json!([]);
                                                if let Some(arr) = json_comments.as_array_mut() {
                                                    for comment in &comments {
                                                        let line = comment
                                                            .line_num
                                                            .or(comment.old_line_num)
                                                            .unwrap_or(1);
                                                        let side = if comment.old_line_num.is_some()
                                                        {
                                                            "LEFT"
                                                        } else {
                                                            "RIGHT"
                                                        };
                                                        let mut obj = serde_json::json!({
                                                            "path": comment.file_path,
                                                            "line": line,
                                                            "side": side,
                                                            "body": comment.body,
                                                        });
                                                        // Add multi-line range if applicable
                                                        if let Some(end_l) = comment.end_line_num {
                                                            if let Some(start_l) = comment.line_num
                                                            {
                                                                if end_l != start_l {
                                                                    if let Some(obj_map) =
                                                                        obj.as_object_mut()
                                                                    {
                                                                        obj_map.insert(
                                                                            "start_line"
                                                                                .to_string(),
                                                                            serde_json::json!(
                                                                                start_l.min(end_l)
                                                                            ),
                                                                        );
                                                                        obj_map.insert(
                                                                            "start_side"
                                                                                .to_string(),
                                                                            serde_json::json!(
                                                                                "RIGHT"
                                                                            ),
                                                                        );
                                                                        obj_map.insert(
                                                                            "line".to_string(),
                                                                            serde_json::json!(
                                                                                start_l.max(end_l)
                                                                            ),
                                                                        );
                                                                        obj_map.insert(
                                                                            "side".to_string(),
                                                                            serde_json::json!(
                                                                                "RIGHT"
                                                                            ),
                                                                        );
                                                                    }
                                                                }
                                                            }
                                                        } else if let Some(end_o) =
                                                            comment.end_old_line_num
                                                        {
                                                            if let Some(oln) = comment.old_line_num
                                                            {
                                                                if end_o != oln {
                                                                    if let Some(obj_map) =
                                                                        obj.as_object_mut()
                                                                    {
                                                                        obj_map.insert(
                                                                            "start_line"
                                                                                .to_string(),
                                                                            serde_json::json!(
                                                                                oln.min(end_o)
                                                                            ),
                                                                        );
                                                                        obj_map.insert(
                                                                            "start_side"
                                                                                .to_string(),
                                                                            serde_json::json!(
                                                                                "LEFT"
                                                                            ),
                                                                        );
                                                                        obj_map.insert(
                                                                            "line".to_string(),
                                                                            serde_json::json!(
                                                                                oln.max(end_o)
                                                                            ),
                                                                        );
                                                                        obj_map.insert(
                                                                            "side".to_string(),
                                                                            serde_json::json!(
                                                                                "LEFT"
                                                                            ),
                                                                        );
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        arr.push(obj);
                                                    }
                                                }
                                                let payload = serde_json::json!({
                                                    "body": value_clone,
                                                    "event": github_event,
                                                    "comments": json_comments,
                                                });
                                                let temp_path = std::env::temp_dir().join(format!(
                                                    "glab-tui-review-{}.json",
                                                    mr_iid
                                                ));
                                                if let Ok(_) = std::fs::write(
                                                    &temp_path,
                                                    serde_json::to_string(&payload).unwrap(),
                                                ) {
                                                    let temp_str =
                                                        temp_path.to_string_lossy().to_string();
                                                    let _ =
                                                        tx.send(Event::CommandStarted(format!(
                                                            "Submitting Review: gh api submit review MR #{}",
                                                            mr_iid
                                                        )));
                                                    let output = tokio::process::Command::new("gh")
                                                        .args([
                                                            "api",
                                                            &format!(
                                                                "repos/{}/pulls/{}/reviews",
                                                                project_context, mr_iid
                                                            ),
                                                            "--input",
                                                            &temp_str,
                                                        ])
                                                        .output()
                                                        .await;
                                                    let _ = std::fs::remove_file(&temp_path);
                                                    match output {
                                                        Ok(out) if out.status.success() => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::MergeRequests,
                                                                    Ok(()),
                                                                ));
                                                        }
                                                        Ok(out) => {
                                                            let err = String::from_utf8_lossy(
                                                                &out.stderr,
                                                            )
                                                            .trim()
                                                            .to_string();
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::MergeRequests,
                                                                    Err(format!(
                                                                        "Submit review failed: {}",
                                                                        err
                                                                    )),
                                                                ));
                                                        }
                                                        Err(e) => {
                                                            let _ =
                                                                tx.send(Event::CommandCompleted(
                                                                    app::Tab::MergeRequests,
                                                                    Err(format!(
                                                                        "Failed to run gh: {}",
                                                                        e
                                                                    )),
                                                                ));
                                                        }
                                                    }
                                                }
                                            } else {
                                                let _ = tx.send(Event::CommandStarted(format!(
                                                    "Submitting Review: glab submit review MR #{}",
                                                    mr_iid
                                                )));
                                                let encoded_path =
                                                    project_context.replace("/", "%2F");
                                                let mut success = true;
                                                let mut err_msg = String::new();

                                                // Fetch MR details to get base_sha, start_sha, and head_sha
                                                let mr_output =
                                                    tokio::process::Command::new("glab")
                                                        .args([
                                                            "api",
                                                            &format!(
                                                                "projects/{}/merge_requests/{}",
                                                                encoded_path, mr_iid
                                                            ),
                                                        ])
                                                        .output()
                                                        .await;

                                                let (base_sha, start_sha, head_sha) =
                                                    if let Ok(out) = mr_output {
                                                        if out.status.success() {
                                                            if let Ok(v) = serde_json::from_slice::<
                                                                serde_json::Value,
                                                            >(
                                                                &out.stdout
                                                            ) {
                                                                let base =
                                                                    v["diff_refs"]["base_sha"]
                                                                        .as_str()
                                                                        .map(|s| s.to_string());
                                                                let start =
                                                                    v["diff_refs"]["start_sha"]
                                                                        .as_str()
                                                                        .map(|s| s.to_string());
                                                                let head =
                                                                    v["diff_refs"]["head_sha"]
                                                                        .as_str()
                                                                        .map(|s| s.to_string());
                                                                (base, start, head)
                                                            } else {
                                                                (None, None, None)
                                                            }
                                                        } else {
                                                            (None, None, None)
                                                        }
                                                    } else {
                                                        (None, None, None)
                                                    };

                                                for comment in &comments {
                                                    let mut position = serde_json::json!({
                                                        "position_type": "text",
                                                        "new_path": comment.file_path,
                                                    });
                                                    if let Some(ref base) = base_sha {
                                                        position["base_sha"] =
                                                            serde_json::json!(base);
                                                    }
                                                    if let Some(ref start) = start_sha {
                                                        position["start_sha"] =
                                                            serde_json::json!(start);
                                                    }
                                                    if let Some(ref head) = head_sha {
                                                        position["head_sha"] =
                                                            serde_json::json!(head);
                                                    }
                                                    if let Some(line_num) = comment.line_num {
                                                        position["new_line"] =
                                                            serde_json::json!(line_num);
                                                    }
                                                    if let Some(old_line_num) = comment.old_line_num
                                                    {
                                                        position["old_line"] =
                                                            serde_json::json!(old_line_num);
                                                        position["old_path"] =
                                                            serde_json::json!(comment.file_path);
                                                    }

                                                    // Multi-line range for GitLab
                                                    if let Some(end_l) = comment.end_line_num {
                                                        if let Some(start_l) = comment.line_num {
                                                            if end_l != start_l {
                                                                let line_range = serde_json::json!({
                                                                    "start": {"line_code": "", "type": "new_line"},
                                                                    "end": {"line_code": "", "type": "new_line"},
                                                                });
                                                                if let Some(lr) =
                                                                    line_range.as_object()
                                                                {
                                                                    position["line_range"] = serde_json::json!({
                                                                        "start": {
                                                                            "line_code": "",
                                                                            "type": "new_line",
                                                                            "new_line": start_l.min(end_l),
                                                                        },
                                                                        "end": {
                                                                            "line_code": "",
                                                                            "type": "new_line",
                                                                            "new_line": start_l.max(end_l),
                                                                        },
                                                                    });
                                                                }
                                                            }
                                                        }
                                                    } else if let Some(end_o) =
                                                        comment.end_old_line_num
                                                    {
                                                        if let Some(start_o) = comment.old_line_num
                                                        {
                                                            if end_o != start_o {
                                                                let line_range = serde_json::json!({
                                                                    "start": {"line_code": "", "type": "old_line"},
                                                                    "end": {"line_code": "", "type": "old_line"},
                                                                });
                                                                if let Some(lr) =
                                                                    line_range.as_object()
                                                                {
                                                                    position["line_range"] = serde_json::json!({
                                                                        "start": {
                                                                            "line_code": "",
                                                                            "type": "old_line",
                                                                            "old_line": start_o.min(end_o),
                                                                        },
                                                                        "end": {
                                                                            "line_code": "",
                                                                            "type": "old_line",
                                                                            "old_line": start_o.max(end_o),
                                                                        },
                                                                    });
                                                                }
                                                            }
                                                        }
                                                    }

                                                    let draft_payload = serde_json::json!({
                                                        "note": comment.body,
                                                        "position": position,
                                                    });
                                                    let temp_path = std::env::temp_dir().join(
                                                        format!("glab-tui-draft-{}.json", mr_iid),
                                                    );
                                                    if let Ok(_) = std::fs::write(
                                                        &temp_path,
                                                        serde_json::to_string(&draft_payload)
                                                            .unwrap(),
                                                    ) {
                                                        let temp_str =
                                                            temp_path.to_string_lossy().to_string();
                                                        let output = tokio::process::Command::new("glab")
                                                            .args([
                                                                "api",
                                                                &format!("projects/{}/merge_requests/{}/draft_notes", encoded_path, mr_iid),
                                                                "--input",
                                                                &temp_str,
                                                                "-X",
                                                                "POST",
                                                            ])
                                                            .output()
                                                            .await;
                                                        let _ = std::fs::remove_file(&temp_path);
                                                        if let Ok(out) = output {
                                                            if !out.status.success() {
                                                                success = false;
                                                                err_msg = String::from_utf8_lossy(
                                                                    &out.stderr,
                                                                )
                                                                .trim()
                                                                .to_string();
                                                                break;
                                                            }
                                                        } else {
                                                            success = false;
                                                            err_msg = "Failed to run glab api"
                                                                .to_string();
                                                            break;
                                                        }
                                                    }
                                                }

                                                if success {
                                                    let publish_success = if !comments.is_empty() {
                                                        let publish_output = tokio::process::Command::new("glab")
                                                            .args([
                                                                "api",
                                                                &format!("projects/{}/merge_requests/{}/draft_notes/bulk_publish", encoded_path, mr_iid),
                                                                "-X",
                                                                "POST",
                                                            ])
                                                            .output()
                                                            .await;
                                                        match publish_output {
                                                            Ok(out) if out.status.success() => true,
                                                            Ok(out) => {
                                                                err_msg = String::from_utf8_lossy(
                                                                    &out.stderr,
                                                                )
                                                                .trim()
                                                                .to_string();
                                                                false
                                                            }
                                                            Err(e) => {
                                                                err_msg = format!(
                                                                    "Failed to publish draft notes: {}",
                                                                    e
                                                                );
                                                                false
                                                            }
                                                        }
                                                    } else {
                                                        true
                                                    };

                                                    if publish_success {
                                                        if status_clone == "Approve" {
                                                            let approve_output = tokio::process::Command::new("glab")
                                                                .args([
                                                                    "api",
                                                                    &format!("projects/{}/merge_requests/{}/approve", encoded_path, mr_iid),
                                                                    "-X",
                                                                    "POST",
                                                                ])
                                                                .output()
                                                                .await;
                                                            if let Ok(out) = approve_output {
                                                                if !out.status.success() {
                                                                    let approval_err =
                                                                        String::from_utf8_lossy(
                                                                            &out.stderr,
                                                                        )
                                                                        .trim()
                                                                        .to_string();
                                                                    let _ = tx.send(Event::FetchFailed(
                                                                        app::Tab::MergeRequests,
                                                                        format!("MR approval failed: {}", approval_err),
                                                                    ));
                                                                }
                                                            }
                                                        }

                                                        if !value_clone.trim().is_empty() {
                                                            let note_payload = serde_json::json!({
                                                                "body": value_clone,
                                                            });
                                                            let temp_path = std::env::temp_dir()
                                                                .join(format!(
                                                                    "glab-tui-note-{}.json",
                                                                    mr_iid
                                                                ));
                                                            if let Ok(_) = std::fs::write(
                                                                &temp_path,
                                                                serde_json::to_string(
                                                                    &note_payload,
                                                                )
                                                                .unwrap(),
                                                            ) {
                                                                let temp_str = temp_path
                                                                    .to_string_lossy()
                                                                    .to_string();
                                                                let _ = tokio::process::Command::new("glab")
                                                                    .args([
                                                                        "api",
                                                                        &format!("projects/{}/merge_requests/{}/notes", encoded_path, mr_iid),
                                                                        "--input",
                                                                        &temp_str,
                                                                        "-X",
                                                                        "POST",
                                                                    ])
                                                                    .output()
                                                                    .await;
                                                                let _ = std::fs::remove_file(
                                                                    &temp_path,
                                                                );
                                                            }
                                                        }

                                                        let _ = tx.send(Event::CommandCompleted(
                                                            app::Tab::MergeRequests,
                                                            Ok(()),
                                                        ));
                                                    } else {
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            app::Tab::MergeRequests,
                                                            Err(format!(
                                                                "Bulk publish failed: {}",
                                                                err_msg
                                                            )),
                                                        ));
                                                    }
                                                } else {
                                                    let _ = tx.send(Event::CommandCompleted(
                                                        app::Tab::MergeRequests,
                                                        Err(format!(
                                                            "Draft notes creation failed: {}",
                                                            err_msg
                                                        )),
                                                    ));
                                                }
                                            }
                                        });
                                    }
                                    crate::app::TextInputAction::EditNewField { field_idx } => {
                                        // Write the value directly into the edit_menu fields
                                        // (no CLI call — iid==0 means this entity is not yet created)
                                        if let Some(ref mut menu) = app.edit_menu {
                                            if let Some(field) = menu.fields.get_mut(field_idx) {
                                                field.1 = value.clone();
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {
                                app.text_input = Some(text_input);
                            }
                        }
                        continue;
                    }

                    if let Some(mut selector) = app.selector.take() {
                        if selector.is_filtering {
                            match key_event.code {
                                KeyCode::Enter | KeyCode::Esc => {
                                    selector.is_filtering = false;
                                    app.selector = Some(selector);
                                }
                                KeyCode::Backspace => {
                                    selector.search_query.pop();
                                    selector.cursor_idx = 0;
                                    selector.state.select(Some(0));
                                    app.selector = Some(selector);
                                }
                                KeyCode::Char(c) => {
                                    selector.search_query.push(c);
                                    selector.cursor_idx = 0;
                                    selector.state.select(Some(0));
                                    app.selector = Some(selector);
                                }
                                _ => {
                                    app.selector = Some(selector);
                                }
                            }
                        } else {
                            let filtered_items = selector.get_filtered_items();
                            match key_event.code {
                                KeyCode::Esc => {
                                    // Close selector, go back to EditMenu (it is already in app.edit_menu)
                                }
                                KeyCode::Char('f') | KeyCode::Char('/') | KeyCode::Char('i') => {
                                    let has_filter = selector.field_type != "comment_action_select"
                                        && selector.field_type != "review_submit_status"
                                        && selector.field_type != "description_edit_choice";
                                    if has_filter {
                                        selector.is_filtering = true;
                                    }
                                    app.selector = Some(selector);
                                }
                                KeyCode::Char('j') | KeyCode::Down => {
                                    if !filtered_items.is_empty() {
                                        selector.cursor_idx =
                                            (selector.cursor_idx + 1) % filtered_items.len();
                                        selector.state.select(Some(selector.cursor_idx));
                                    }
                                    app.selector = Some(selector);
                                }
                                KeyCode::Char('k') | KeyCode::Up => {
                                    if !filtered_items.is_empty() {
                                        if selector.cursor_idx == 0 {
                                            selector.cursor_idx = filtered_items.len() - 1;
                                        } else {
                                            selector.cursor_idx -= 1;
                                        }
                                        selector.state.select(Some(selector.cursor_idx));
                                    }
                                    app.selector = Some(selector);
                                }
                                KeyCode::Char(' ') => {
                                    if !filtered_items.is_empty() {
                                        let item = &filtered_items[selector.cursor_idx];
                                        if item.starts_with("+ Create \"") {
                                            let clean_val =
                                                selector.search_query.trim().to_string();
                                            if !clean_val.is_empty() {
                                                if selector.multi_select {
                                                    if selector.selected_items.contains(&clean_val)
                                                    {
                                                        selector.selected_items.remove(&clean_val);
                                                    } else {
                                                        selector.selected_items.insert(clean_val);
                                                    }
                                                } else {
                                                    selector.selected_items.clear();
                                                    selector.selected_items.insert(clean_val);
                                                }
                                            }
                                        } else {
                                            if selector.multi_select {
                                                if selector.selected_items.contains(item) {
                                                    selector.selected_items.remove(item);
                                                } else {
                                                    selector.selected_items.insert(item.clone());
                                                }
                                            } else {
                                                if selector.selected_items.contains(item) {
                                                    selector.selected_items.remove(item);
                                                } else {
                                                    selector.selected_items.clear();
                                                    selector.selected_items.insert(item.clone());
                                                }
                                            }
                                        }
                                    }
                                    app.selector = Some(selector);
                                }
                                KeyCode::Enter => {
                                    let field_type = selector.field_type.clone();
                                    if field_type == "column_filter" {
                                        if let Some((tab, col)) = app.column_filter_context.take() {
                                            app.set_column_filter(
                                                tab,
                                                &col,
                                                selector.selected_items.clone(),
                                            );
                                            app.update_filter_selection();
                                        }
                                        continue;
                                    }
                                    if field_type == "switch_repo" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val =
                                            selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val =
                                                Some(filtered_items[selector.cursor_idx].clone());
                                        }

                                        if let Some(mut path) = selected_val {
                                            if path.starts_with("+ Create \"") {
                                                path = selector.search_query.trim().to_string();
                                            }

                                            let repos_dir = crate::utils::cache::get_repos_dir();
                                            let target_path =
                                                if std::path::Path::new(&path).is_absolute() {
                                                    std::path::PathBuf::from(&path)
                                                } else {
                                                    repos_dir.join(&path)
                                                };

                                            let target_path_str =
                                                target_path.to_string_lossy().into_owned();
                                            if crate::utils::cache::is_git_repo(&target_path_str) {
                                                if std::env::set_current_dir(&target_path).is_ok() {
                                                    crate::utils::cache::add_recent_repo(
                                                        &target_path_str,
                                                    );

                                                    if let Ok(context) =
                                                        gitlab::client::get_project_context().await
                                                    {
                                                        app.project_context = context;
                                                    }
                                                    if let Ok(mut client) =
                                                        gitlab::client::GitlabClient::new().await
                                                    {
                                                        client.tx = Some(events.sender());
                                                        app.gitlab_client = Some(client.clone());
                                                    } else {
                                                        app.gitlab_client = None;
                                                    }

                                                    app.loaded_tabs.clear();
                                                    app.loading_tabs.clear();
                                                    app.refreshed_tabs.clear();
                                                    app.status_message = None;
                                                    app.issues.items.clear();
                                                    app.mrs.items.clear();
                                                    app.pipelines.items.clear();
                                                    app.runners.items.clear();
                                                    app.releases.items.clear();
                                                    app.todos.items.clear();
                                                    app.milestones.items.clear();
                                                    app.pipeline_jobs.clear();
                                                    app.fetching_pipelines.clear();

                                                    let cache = crate::utils::cache::load_cache(
                                                        &app.project_context,
                                                    );
                                                    app.issues.items = cache.issues;
                                                    app.mrs.items = cache.mrs;
                                                    app.pipelines.items = cache.pipelines;
                                                    app.runners.items = cache.runners;
                                                    app.releases.items = cache.releases;
                                                    app.todos.items = cache.todos;
                                                    app.milestones.items = cache.milestones;

                                                    if !app.issues.items.is_empty() {
                                                        app.loaded_tabs.insert(app::Tab::Issues);
                                                    }
                                                    if !app.mrs.items.is_empty() {
                                                        app.loaded_tabs
                                                            .insert(app::Tab::MergeRequests);
                                                    }
                                                    if !app.pipelines.items.is_empty() {
                                                        app.loaded_tabs.insert(app::Tab::Pipelines);
                                                    }
                                                    if !app.runners.items.is_empty() {
                                                        app.loaded_tabs.insert(app::Tab::Runners);
                                                    }
                                                    if !app.releases.items.is_empty() {
                                                        app.loaded_tabs.insert(app::Tab::Releases);
                                                    }
                                                    if !app.todos.items.is_empty() {
                                                        app.loaded_tabs.insert(app::Tab::Todos);
                                                    }
                                                    if !app.milestones.items.is_empty() {
                                                        app.loaded_tabs
                                                            .insert(app::Tab::Milestones);
                                                    }

                                                    app.issues.state.select(
                                                        if app.issues.items.is_empty() {
                                                            None
                                                        } else {
                                                            Some(0)
                                                        },
                                                    );
                                                    app.mrs.state.select(
                                                        if app.mrs.items.is_empty() {
                                                            None
                                                        } else {
                                                            Some(0)
                                                        },
                                                    );
                                                    app.pipelines.state.select(
                                                        if app.pipelines.items.is_empty() {
                                                            None
                                                        } else {
                                                            Some(0)
                                                        },
                                                    );
                                                    app.update_filter_selection();

                                                    if let Some(client) = &app.gitlab_client {
                                                        let has_cached = match app.active_tab {
                                                            app::Tab::Issues => {
                                                                !app.issues.items.is_empty()
                                                            }
                                                            app::Tab::MergeRequests => {
                                                                !app.mrs.items.is_empty()
                                                            }
                                                            app::Tab::Pipelines => {
                                                                !app.pipelines.items.is_empty()
                                                            }
                                                            app::Tab::Runners => {
                                                                !app.runners.items.is_empty()
                                                            }
                                                            app::Tab::Releases => {
                                                                !app.releases.items.is_empty()
                                                            }
                                                            app::Tab::Todos => {
                                                                !app.todos.items.is_empty()
                                                            }
                                                            app::Tab::Milestones => {
                                                                !app.milestones.items.is_empty()
                                                            }
                                                            _ => false,
                                                        };
                                                        if !has_cached {
                                                            app.loading_tabs.insert(app.active_tab);
                                                        }
                                                        spawn_refresh_active_tab(
                                                            client,
                                                            &app.project_context,
                                                            app.active_tab,
                                                            events.sender(),
                                                        );
                                                    }
                                                } else {
                                                    app.error_message = Some(format!(
                                                        "Could not change directory to: {}",
                                                        path
                                                    ));
                                                }
                                            } else {
                                                app.error_message = Some(format!(
                                                    "Not a valid git repository: {}",
                                                    path
                                                ));
                                            }
                                        }
                                        continue;
                                    }

                                    if field_type == "create_mr" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val =
                                            selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val =
                                                Some(filtered_items[selector.cursor_idx].clone());
                                        }

                                        app.selector = None;

                                        let is_github = app
                                            .gitlab_client
                                            .as_ref()
                                            .map(|c| c.is_github)
                                            .unwrap_or(false);
                                        let pr_suffix = if is_github {
                                            "Pull Request"
                                        } else {
                                            "Merge Request"
                                        };

                                        let mut title_val = String::new();
                                        let mut labels_val = String::new();
                                        let mut assignees_val = String::new();
                                        let mut milestone_val = String::new();
                                        let mut source_branch_val =
                                            get_current_branch().unwrap_or_default();
                                        let mut description_val = String::new();
                                        let mut issue_iid = 0;

                                        if let Some(item) = selected_val {
                                            if item != "Create blank (No issue)" {
                                                let id_val = item.clone();
                                                let parsed_iid = if id_val.starts_with('#') {
                                                    id_val
                                                        .strip_prefix('#')
                                                        .and_then(|s| {
                                                            s.split(|c: char| !c.is_numeric())
                                                                .next()
                                                        })
                                                        .and_then(|s| s.parse::<u64>().ok())
                                                } else {
                                                    id_val.trim().parse::<u64>().ok()
                                                };

                                                if let Some(iid) = parsed_iid {
                                                    if let Some(issue) = app
                                                        .issues
                                                        .items
                                                        .iter()
                                                        .find(|i| i.iid == iid)
                                                    {
                                                        issue_iid = issue.iid;
                                                        title_val = issue.title.clone();
                                                        source_branch_val = format!(
                                                            "{}-{}",
                                                            issue.iid,
                                                            slugify(&issue.title)
                                                        );
                                                        if !issue.labels.is_empty() {
                                                            labels_val = issue.labels.join(", ");
                                                        }
                                                        if !issue.assignees.is_empty() {
                                                            assignees_val = issue
                                                                .assignees
                                                                .iter()
                                                                .map(|a| format!("@{}", a.username))
                                                                .collect::<Vec<_>>()
                                                                .join(", ");
                                                        }
                                                        if let Some(ref m) = issue.milestone {
                                                            milestone_val = m.title.clone();
                                                        }
                                                        if let Some(ref d) = issue.description {
                                                            description_val = format!(
                                                                "Closes #{}\n\n{}",
                                                                issue.iid, d
                                                            );
                                                        } else {
                                                            let mr_tmpl =
                                                                get_default_template("mr")
                                                                    .unwrap_or_default();
                                                            if mr_tmpl.is_empty() {
                                                                description_val = format!(
                                                                    "Closes #{}",
                                                                    issue.iid
                                                                );
                                                            } else {
                                                                description_val = format!(
                                                                    "Closes #{}\n\n{}",
                                                                    issue.iid, mr_tmpl
                                                                );
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        app.edit_menu = Some(crate::app::EditMenu {
                                            title: format!("Create {}", pr_suffix),
                                            fields: vec![
                                                ("Title".to_string(), title_val),
                                                ("Source Branch".to_string(), source_branch_val),
                                                (
                                                    "Target Branch".to_string(),
                                                    get_default_branch()
                                                        .unwrap_or_else(|| "main".to_string()),
                                                ),
                                                ("Labels".to_string(), labels_val),
                                                ("Assignees".to_string(), assignees_val),
                                                ("Reviewers".to_string(), String::new()),
                                                ("Milestone".to_string(), milestone_val),
                                                (
                                                    "Status (Draft/Ready)".to_string(),
                                                    "Draft".to_string(),
                                                ),
                                                ("Description".to_string(), description_val),
                                            ],
                                            selected_idx: 0,
                                            entity_iid: issue_iid,
                                            entity_type: "new_mr".to_string(),
                                            state: {
                                                let mut s = ListState::default();
                                                s.select(Some(0));
                                                s
                                            },
                                        });
                                        continue;
                                    }

                                    if field_type == "review_submit_status" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val =
                                            selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val =
                                                Some(filtered_items[selector.cursor_idx].clone());
                                        }

                                        let status =
                                            selected_val.unwrap_or_else(|| "Comment".to_string());
                                        app.selector = None;
                                        app.text_input = Some(crate::app::TextInput {
                                            title: format!(
                                                " Submit Review ({}) - Summary/Description ",
                                                status
                                            ),
                                            value: String::new(),
                                            cursor_idx: 0,
                                            action:
                                                crate::app::TextInputAction::SubmitReviewFinal {
                                                    mr_iid: selector.entity_iid,
                                                    status,
                                                },
                                        });
                                        continue;
                                    }

                                    if field_type == "comment_select" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val =
                                            selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val =
                                                Some(filtered_items[selector.cursor_idx].clone());
                                        }

                                        if let Some(val) = selected_val {
                                            if let Some(id_str) = val
                                                .strip_prefix("ID: ")
                                                .and_then(|s| s.split(" |").next())
                                            {
                                                if let Ok(comment_id) = id_str.parse::<u64>() {
                                                    if let Some(comment) = app
                                                        .current_comments
                                                        .iter()
                                                        .find(|c| c.id == comment_id)
                                                        .cloned()
                                                    {
                                                        let is_github = app
                                                            .gitlab_client
                                                            .as_ref()
                                                            .map_or(false, |c| c.is_github);

                                                        let mut actions =
                                                            vec!["Reply to Thread".to_string()];

                                                        if !is_github {
                                                            let is_resolved =
                                                                comment.resolved.unwrap_or(false);
                                                            if is_resolved {
                                                                actions.push(
                                                                    "Unresolve Thread".to_string(),
                                                                );
                                                            } else {
                                                                actions.push(
                                                                    "Resolve Thread".to_string(),
                                                                );
                                                            }
                                                        }

                                                        actions.push("Edit Comment".to_string());
                                                        actions.push("Delete Comment".to_string());

                                                        app.selector = Some(crate::app::Selector {
                                                            title: format!(
                                                                " Actions for Comment {} ",
                                                                comment_id
                                                            ),
                                                            all_items: actions,
                                                            selected_items:
                                                                std::collections::HashSet::new(),
                                                            cursor_idx: 0,
                                                            search_query: String::new(),
                                                            is_filtering: false,
                                                            is_loading: false,
                                                            entity_iid: comment_id,
                                                            entity_type: selector
                                                                .entity_iid
                                                                .to_string(), // Store MR IID as string
                                                            field_type: "comment_action_select"
                                                                .to_string(),
                                                            multi_select: false,
                                                            state: ListState::default(),
                                                        });
                                                        continue;
                                                    }
                                                }
                                            }
                                        }
                                        app.selector = None;
                                        continue;
                                    }

                                    if field_type == "comment_action_select" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val =
                                            selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val =
                                                Some(filtered_items[selector.cursor_idx].clone());
                                        }

                                        app.selector = None;

                                        if let Some(action_str) = selected_val {
                                            let comment_id = selector.entity_iid;
                                            let mr_iid =
                                                selector.entity_type.parse::<u64>().unwrap_or(0);

                                            let comment = app
                                                .current_comments
                                                .iter()
                                                .find(|c| c.id == comment_id)
                                                .cloned();

                                            if let Some(comment) = comment {
                                                match action_str.as_str() {
                                                    "Reply to Thread" => {
                                                        let discussion_id = comment
                                                            .discussion_id
                                                            .clone()
                                                            .unwrap_or_else(|| {
                                                                comment.id.to_string()
                                                            });
                                                        app.text_input = Some(crate::app::TextInput {
                                                            title: format!(" Reply to @{} ", comment.author.username),
                                                            value: String::new(),
                                                            cursor_idx: 0,
                                                            action: crate::app::TextInputAction::ReplyToComment {
                                                                mr_iid,
                                                                comment_id,
                                                                discussion_id,
                                                            },
                                                        });
                                                    }
                                                    "Resolve Thread" | "Unresolve Thread" => {
                                                        let is_resolve =
                                                            action_str == "Resolve Thread";
                                                        let client = app.gitlab_client.clone();
                                                        let project_context =
                                                            app.project_context.clone();
                                                        let tx = events.sender();
                                                        let discussion_id = comment
                                                            .discussion_id
                                                            .clone()
                                                            .unwrap_or_default();

                                                        let status_desc = if is_resolve {
                                                            "Resolving"
                                                        } else {
                                                            "Unresolving"
                                                        };
                                                        let _ = tx.send(Event::CommandStarted(
                                                            format!(
                                                                "{} thread MR #{}",
                                                                status_desc, mr_iid
                                                            ),
                                                        ));

                                                        tokio::spawn(async move {
                                                            if let Some(client) = client {
                                                                let encoded_path = project_context
                                                                    .replace("/", "%2F");
                                                                let res_str = if is_resolve {
                                                                    "true"
                                                                } else {
                                                                    "false"
                                                                };
                                                                let output = tokio::process::Command::new("glab")
                                                                    .args([
                                                                        "api",
                                                                        &format!(
                                                                            "projects/{}/merge_requests/{}/discussions/{}?resolved={}",
                                                                            encoded_path, mr_iid, discussion_id, res_str
                                                                        ),
                                                                        "-X",
                                                                        "PUT",
                                                                    ])
                                                                    .output()
                                                                    .await;

                                                                match output {
                                                                    Ok(out)
                                                                        if out.status.success() =>
                                                                    {
                                                                        let _ = tx.send(Event::CommandCompleted(
                                                                            app::Tab::MergeRequests,
                                                                            Ok(()),
                                                                        ));
                                                                    }
                                                                    Ok(out) => {
                                                                        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                                                                        let _ = tx.send(Event::CommandCompleted(
                                                                            app::Tab::MergeRequests,
                                                                            Err(err),
                                                                        ));
                                                                    }
                                                                    Err(e) => {
                                                                        let _ = tx.send(Event::CommandCompleted(
                                                                            app::Tab::MergeRequests,
                                                                            Err(e.to_string()),
                                                                        ));
                                                                    }
                                                                }
                                                            }
                                                        });
                                                    }
                                                    "Edit Comment" => {
                                                        let client = app.gitlab_client.clone();
                                                        let project_context =
                                                            app.project_context.clone();
                                                        let tx = events.sender();

                                                        app.status_message = Some(
                                                            "Opening editor to edit comment..."
                                                                .to_string(),
                                                        );

                                                        let is_github = client
                                                            .as_ref()
                                                            .map_or(false, |c| c.is_github);
                                                        let ext = std::path::Path::new(
                                                            comment
                                                                .position
                                                                .as_ref()
                                                                .and_then(|p| p.new_path.as_ref())
                                                                .map(|s| s.as_str())
                                                                .unwrap_or("md"),
                                                        )
                                                        .extension()
                                                        .and_then(|s| s.to_str())
                                                        .unwrap_or("md");
                                                        let suffix = format!(".{}", ext);

                                                        let new_body = edit_in_editor_with_suffix(
                                                            &comment.body,
                                                            &suffix,
                                                            &mut terminal,
                                                        );
                                                        if let Some(body) = new_body {
                                                            if body != comment.body
                                                                && !body.trim().is_empty()
                                                            {
                                                                let _ = tx.send(
                                                                    Event::CommandStarted(format!(
                                                                        "Editing comment MR #{}",
                                                                        mr_iid
                                                                    )),
                                                                );

                                                                tokio::spawn(async move {
                                                                    if let Some(client) = client {
                                                                        let output = if is_github {
                                                                            let endpoint =
                                                                                if comment
                                                                                    .position
                                                                                    .is_some()
                                                                                {
                                                                                    format!("repos/{}/pulls/comments/{}", project_context, comment_id)
                                                                                } else {
                                                                                    format!("repos/{}/issues/comments/{}", project_context, comment_id)
                                                                                };
                                                                            let payload = serde_json::json!({ "body": body });
                                                                            let temp_path = std::env::temp_dir().join(format!("glab-tui-edit-{}.json", comment_id));
                                                                            let _ = std::fs::write(&temp_path, serde_json::to_string(&payload).unwrap());
                                                                            let temp_str = temp_path.to_string_lossy().to_string();

                                                                            let res = tokio::process::Command::new("gh")
                                                                                .args(["api", &endpoint, "--input", &temp_str, "-X", "PATCH"])
                                                                                .output()
                                                                                .await;
                                                                            let _ = std::fs::remove_file(&temp_path);
                                                                            res
                                                                        } else {
                                                                            let encoded_path =
                                                                                project_context
                                                                                    .replace(
                                                                                        "/", "%2F",
                                                                                    );
                                                                            let payload = serde_json::json!({ "body": body });
                                                                            let temp_path = std::env::temp_dir().join(format!("glab-tui-edit-{}.json", comment_id));
                                                                            let _ = std::fs::write(&temp_path, serde_json::to_string(&payload).unwrap());
                                                                            let temp_str = temp_path.to_string_lossy().to_string();

                                                                            let res = tokio::process::Command::new("glab")
                                                                                .args([
                                                                                    "api",
                                                                                    &format!("projects/{}/merge_requests/{}/notes/{}", encoded_path, mr_iid, comment_id),
                                                                                    "--input",
                                                                                    &temp_str,
                                                                                    "-X",
                                                                                    "PUT"
                                                                                ])
                                                                                .output()
                                                                                .await;
                                                                            let _ = std::fs::remove_file(&temp_path);
                                                                            res
                                                                        };

                                                                        match output {
                                                                            Ok(out)
                                                                                if out
                                                                                    .status
                                                                                    .success() =>
                                                                            {
                                                                                let _ = tx.send(Event::CommandCompleted(
                                                                                    app::Tab::MergeRequests,
                                                                                    Ok(()),
                                                                                ));
                                                                            }
                                                                            Ok(out) => {
                                                                                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                                                                                let _ = tx.send(Event::CommandCompleted(
                                                                                    app::Tab::MergeRequests,
                                                                                    Err(err),
                                                                                ));
                                                                            }
                                                                            Err(e) => {
                                                                                let _ = tx.send(Event::CommandCompleted(
                                                                                    app::Tab::MergeRequests,
                                                                                    Err(e.to_string()),
                                                                                ));
                                                                            }
                                                                        }
                                                                    }
                                                                });
                                                            }
                                                        }
                                                    }
                                                    "Delete Comment" => {
                                                        let client = app.gitlab_client.clone();
                                                        let project_context =
                                                            app.project_context.clone();
                                                        let tx = events.sender();
                                                        let is_github = client
                                                            .as_ref()
                                                            .map_or(false, |c| c.is_github);

                                                        let _ = tx.send(Event::CommandStarted(
                                                            format!(
                                                                "Deleting comment MR #{}",
                                                                mr_iid
                                                            ),
                                                        ));

                                                        tokio::spawn(async move {
                                                            if let Some(client) = client {
                                                                let output = if is_github {
                                                                    let endpoint = if comment
                                                                        .position
                                                                        .is_some()
                                                                    {
                                                                        format!(
                                                                            "repos/{}/pulls/comments/{}",
                                                                            project_context,
                                                                            comment_id
                                                                        )
                                                                    } else {
                                                                        format!(
                                                                            "repos/{}/issues/comments/{}",
                                                                            project_context,
                                                                            comment_id
                                                                        )
                                                                    };
                                                                    tokio::process::Command::new(
                                                                        "gh",
                                                                    )
                                                                    .args([
                                                                        "api", &endpoint, "-X",
                                                                        "DELETE",
                                                                    ])
                                                                    .output()
                                                                    .await
                                                                } else {
                                                                    let encoded_path =
                                                                        project_context
                                                                            .replace("/", "%2F");
                                                                    tokio::process::Command::new("glab")
                                                                        .args([
                                                                            "api",
                                                                            &format!("projects/{}/merge_requests/{}/notes/{}", encoded_path, mr_iid, comment_id),
                                                                            "-X",
                                                                            "DELETE"
                                                                        ])
                                                                        .output()
                                                                        .await
                                                                };

                                                                match output {
                                                                    Ok(out)
                                                                        if out.status.success() =>
                                                                    {
                                                                        let _ = tx.send(Event::CommandCompleted(
                                                                            app::Tab::MergeRequests,
                                                                            Ok(()),
                                                                        ));
                                                                    }
                                                                    Ok(out) => {
                                                                        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                                                                        let _ = tx.send(Event::CommandCompleted(
                                                                            app::Tab::MergeRequests,
                                                                            Err(err),
                                                                        ));
                                                                    }
                                                                    Err(e) => {
                                                                        let _ = tx.send(Event::CommandCompleted(
                                                                            app::Tab::MergeRequests,
                                                                            Err(e.to_string()),
                                                                        ));
                                                                    }
                                                                }
                                                            }
                                                        });
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                        continue;
                                    }

                                    let entity_type = selector.entity_type.clone();
                                    let entity_iid = selector.entity_iid;
                                    let filtered_items = selector.get_filtered_items();
                                    let mut selected_list: Vec<String> =
                                        selector.selected_items.iter().cloned().collect();

                                    // Include highlighted item in selection if nothing auto-selected
                                    if !filtered_items.is_empty() {
                                        let item = &filtered_items[selector.cursor_idx];
                                        if item.starts_with("+ Create \"") {
                                            let query = selector.search_query.trim().to_string();
                                            if !query.is_empty() {
                                                if selector.multi_select {
                                                    if !selected_list.contains(&query) {
                                                        selected_list.push(query);
                                                    }
                                                } else {
                                                    selected_list = vec![query];
                                                }
                                            }
                                        } else if !selector.multi_select && selected_list.is_empty()
                                        {
                                            selected_list.push(item.clone());
                                        }
                                    }

                                    if field_type == "description_edit_choice" {
                                        app.selector = None;
                                        let choice =
                                            selected_list.first().cloned().unwrap_or_default();

                                        if entity_iid == 0 {
                                            if let Some(ref mut menu) = app.edit_menu {
                                                let field_idx = menu
                                                    .fields
                                                    .iter()
                                                    .position(|f| f.0 == "Description")
                                                    .unwrap_or(0);
                                                if let Some(f) = menu
                                                    .fields
                                                    .iter_mut()
                                                    .find(|f| f.0 == "Description")
                                                {
                                                    if choice == "Edit (basic)" {
                                                        let tmpl_val = if f.1.trim().is_empty() {
                                                            let template_type =
                                                                if entity_type == "new_mr" {
                                                                    "mr"
                                                                } else {
                                                                    "issue"
                                                                };
                                                            get_default_template(template_type)
                                                                .unwrap_or_default()
                                                        } else {
                                                            f.1.clone()
                                                        };
                                                        app.text_input = Some(
                                                            crate::app::TextInput {
                                                                title:
                                                                    " Edit Description "
                                                                        .to_string(),
                                                                value: tmpl_val.clone(),
                                                                cursor_idx: tmpl_val.len(),
                                                                action:
                                                                    crate::app::TextInputAction::EditNewField {
                                                                        field_idx,
                                                                    },
                                                            },
                                                        );
                                                    } else {
                                                        let current_val = if f.1.trim().is_empty() {
                                                            let template_type =
                                                                if entity_type == "new_mr" {
                                                                    "mr"
                                                                } else {
                                                                    "issue"
                                                                };
                                                            get_default_template(template_type)
                                                                .unwrap_or_default()
                                                        } else {
                                                            f.1.clone()
                                                        };
                                                        if let Some(new_desc) = edit_in_editor(
                                                            &current_val,
                                                            &mut terminal,
                                                        ) {
                                                            f.1 = new_desc;
                                                        }
                                                    }
                                                }
                                            }
                                        } else {
                                            let current_desc = if entity_type == "issue" {
                                                app.issues
                                                    .items
                                                    .iter()
                                                    .find(|i| i.iid == entity_iid)
                                                    .and_then(|i| i.description.clone())
                                                    .unwrap_or_default()
                                            } else {
                                                app.mrs
                                                    .items
                                                    .iter()
                                                    .find(|m| m.iid == entity_iid)
                                                    .and_then(|m| m.description.clone())
                                                    .unwrap_or_default()
                                            };

                                            if choice == "Edit (basic)" {
                                                app.text_input = Some(crate::app::TextInput {
                                                    title: " Edit Description ".to_string(),
                                                    value: current_desc.clone(),
                                                    cursor_idx: current_desc.len(),
                                                    action:
                                                        crate::app::TextInputAction::EditField {
                                                            entity_iid,
                                                            entity_type: entity_type.clone(),
                                                            field_type: "description".to_string(),
                                                        },
                                                });
                                            } else {
                                                if let Some(new_desc) =
                                                    edit_in_editor(&current_desc, &mut terminal)
                                                {
                                                    if entity_type == "issue" {
                                                        if let Some(item) = app
                                                            .issues
                                                            .items
                                                            .iter_mut()
                                                            .find(|i| i.iid == entity_iid)
                                                        {
                                                            item.description =
                                                                Some(new_desc.clone());
                                                        }
                                                    } else if entity_type == "mr" {
                                                        if let Some(item) = app
                                                            .mrs
                                                            .items
                                                            .iter_mut()
                                                            .find(|m| m.iid == entity_iid)
                                                        {
                                                            item.description =
                                                                Some(new_desc.clone());
                                                        }
                                                    }
                                                    let cli = app_cli(&app);
                                                    let args = UpdateCmd::new(
                                                        cli.is_github,
                                                        &entity_type,
                                                        entity_iid,
                                                    )
                                                    .flag("-d", &new_desc)
                                                    .build();
                                                    run_cli(
                                                        &cli,
                                                        &args,
                                                        &mut terminal,
                                                        events.sender(),
                                                        app.active_tab,
                                                    )
                                                    .await;
                                                }
                                                if let Some(client) = &app.gitlab_client {
                                                    if entity_type == "issue" {
                                                        if let Ok(updated) =
                                                            gitlab::issues::get_issue(
                                                                client,
                                                                &app.project_context,
                                                                entity_iid,
                                                            )
                                                            .await
                                                        {
                                                            if let Some(item) = app
                                                                .issues
                                                                .items
                                                                .iter_mut()
                                                                .find(|i| i.iid == entity_iid)
                                                            {
                                                                *item = updated;
                                                            }
                                                        }
                                                    } else if entity_type == "mr" {
                                                        if let Ok(updated) = gitlab::mr::get_mr(
                                                            client,
                                                            &app.project_context,
                                                            entity_iid,
                                                        )
                                                        .await
                                                        {
                                                            if let Some(item) = app
                                                                .mrs
                                                                .items
                                                                .iter_mut()
                                                                .find(|m| m.iid == entity_iid)
                                                            {
                                                                *item = updated;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        continue;
                                    }

                                    if entity_iid == 0 || entity_type.starts_with("new_") {
                                        // Write the values directly to the active field of app.edit_menu
                                        if let Some(ref mut menu) = app.edit_menu {
                                            let target_field_name = match field_type.as_str() {
                                                "labels" => "Labels",
                                                "assignees" => "Assignees",
                                                "reviewers" => "Reviewers",
                                                "milestone" => "Milestone",
                                                "confidential" => "Confidential",
                                                "draft_status" => "Status (Draft/Ready)",
                                                "mr_pipeline" => "Merge Request Pipeline",
                                                "source_branch" => "Source Branch",
                                                "target_branch" => "Target Branch",
                                                _ => "",
                                            };
                                            if !target_field_name.is_empty() {
                                                if let Some(f) = menu
                                                    .fields
                                                    .iter_mut()
                                                    .find(|f| f.0 == target_field_name)
                                                {
                                                    let display_val = if field_type
                                                        == "confidential"
                                                    {
                                                        selected_list
                                                            .first()
                                                            .cloned()
                                                            .unwrap_or_else(|| "No".to_string())
                                                    } else if field_type == "draft_status" {
                                                        selected_list
                                                            .first()
                                                            .cloned()
                                                            .unwrap_or_else(|| "Ready".to_string())
                                                    } else if field_type == "mr_pipeline" {
                                                        selected_list
                                                            .first()
                                                            .cloned()
                                                            .unwrap_or_else(|| "No".to_string())
                                                    } else {
                                                        selected_list.join(", ")
                                                    };
                                                    f.1 = display_val;
                                                }
                                            }
                                        }
                                    } else {
                                        apply_selector_changes(
                                            &mut app,
                                            &entity_type,
                                            entity_iid,
                                            &field_type,
                                            selected_list,
                                            &mut terminal,
                                        )
                                        .await;

                                        if let Some(client) = &app.gitlab_client {
                                            spawn_refresh_active_tab(
                                                client,
                                                &app.project_context,
                                                app.active_tab,
                                                events.sender(),
                                            );
                                        }

                                        rebuild_edit_menu(&mut app, &entity_type, entity_iid);
                                    }
                                }
                                _ => {
                                    app.selector = Some(selector);
                                }
                            }
                        }
                        continue;
                    }

                    if let Some(mut menu) = app.edit_menu.take() {
                        match key_event.code {
                            KeyCode::Esc => {
                                // close menu
                            }
                            KeyCode::Char('j') | KeyCode::Down => {
                                let is_new =
                                    menu.entity_iid == 0 || menu.entity_type.starts_with("new_");
                                let max_idx = if is_new {
                                    menu.fields.len() + 1 // fields + spacer + submit
                                } else {
                                    menu.fields.len() - 1
                                };
                                menu.selected_idx = if menu.selected_idx >= max_idx {
                                    0
                                } else {
                                    menu.selected_idx + 1
                                };
                                // Skip the spacer row (index == fields.len())
                                if is_new && menu.selected_idx == menu.fields.len() {
                                    menu.selected_idx += 1;
                                }
                                menu.state.select(Some(menu.selected_idx));
                                app.edit_menu = Some(menu);
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                let is_new =
                                    menu.entity_iid == 0 || menu.entity_type.starts_with("new_");
                                let max_idx = if is_new {
                                    menu.fields.len() + 1
                                } else {
                                    menu.fields.len() - 1
                                };
                                menu.selected_idx = if menu.selected_idx == 0 {
                                    max_idx
                                } else {
                                    menu.selected_idx - 1
                                };
                                // Skip the spacer row (index == fields.len())
                                if is_new && menu.selected_idx == menu.fields.len() {
                                    menu.selected_idx = menu.fields.len().saturating_sub(1);
                                }
                                menu.state.select(Some(menu.selected_idx));
                                app.edit_menu = Some(menu);
                            }
                            KeyCode::Enter => {
                                let entity_iid = menu.entity_iid;
                                let entity_type = menu.entity_type.clone();
                                let is_new_entity =
                                    entity_iid == 0 || entity_type.starts_with("new_");
                                let is_on_submit =
                                    is_new_entity && menu.selected_idx == menu.fields.len() + 1;

                                if is_on_submit {
                                    if entity_type == "new_issue" {
                                        let cli = app_cli(&app);
                                        let title = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Title")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let description = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Description")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let labels = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Labels")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let assignees = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Assignees")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let milestone = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Milestone")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let confidential = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Confidential")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let due_date = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Due Date")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let weight = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Weight")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();

                                        let mut cmd_args: Vec<String> =
                                            vec!["issue".into(), "create".into()];
                                        if !title.is_empty() {
                                            cmd_args.push("--title".into());
                                            cmd_args.push(title);
                                        }
                                        if !description.is_empty() {
                                            cmd_args.push(cli.flag_description().to_string());
                                            cmd_args.push(description);
                                        }
                                        if !labels.is_empty() {
                                            cmd_args.push("--label".into());
                                            cmd_args.push(labels.replace(", ", ","));
                                        }
                                        if !assignees.is_empty() {
                                            let clean = assignees
                                                .split(',')
                                                .map(|a| {
                                                    a.trim().trim_start_matches('@').to_string()
                                                })
                                                .collect::<Vec<_>>()
                                                .join(",");
                                            cmd_args.push("--assignee".into());
                                            cmd_args.push(clean);
                                        }
                                        if !milestone.is_empty() {
                                            cmd_args.push("--milestone".into());
                                            cmd_args.push(milestone);
                                        }
                                        if confidential.to_lowercase() == "yes" {
                                            cmd_args.push("--confidential".into());
                                        }
                                        if !due_date.is_empty() {
                                            cmd_args.push("--due-date".into());
                                            cmd_args.push(due_date);
                                        }
                                        if !weight.is_empty() && weight != "0" {
                                            cmd_args.push("--weight".into());
                                            cmd_args.push(weight);
                                        }

                                        app.edit_menu = None;
                                        run_cli(
                                            &cli,
                                            &cmd_args,
                                            &mut terminal,
                                            events.sender(),
                                            app.active_tab,
                                        )
                                        .await;

                                        if let Some(client) = &app.gitlab_client {
                                            spawn_refresh_active_tab(
                                                client,
                                                &app.project_context,
                                                app.active_tab,
                                                events.sender(),
                                            );
                                        }
                                        continue;
                                    } else if entity_type == "new_mr" {
                                        let cli = app_cli(&app);
                                        let title = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Title")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let source = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Source Branch")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let target = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Target Branch")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let labels = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Labels")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let assignees = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Assignees")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let reviewers = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Reviewers")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let milestone = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Milestone")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let status = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Status (Draft/Ready)")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let description = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Description")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let mr_pipeline = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Merge Request Pipeline")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();

                                        if !source.is_empty() {
                                            let exists = std::process::Command::new("git")
                                                .args(["rev-parse", "--verify", "--quiet", &source])
                                                .output()
                                                .ok()
                                                .map_or(false, |o| o.status.success());
                                            if !exists {
                                                let _ = std::process::Command::new("git")
                                                    .args(["branch", &source, "HEAD"])
                                                    .output();
                                            }
                                            let _ = std::process::Command::new("git")
                                                .args(["push", "-u", "origin", &source])
                                                .output();
                                        }

                                        let entity_iid_str = menu.entity_iid.to_string();
                                        let mut cmd_args: Vec<String> =
                                            vec![cli.entity("mr").into(), "create".into()];
                                        if !cli.is_github {
                                            cmd_args.push("-y".into());
                                        }
                                        if menu.entity_iid > 0 {
                                            if cli.is_github {
                                                cmd_args.push("--body".into());
                                                cmd_args
                                                    .push(format!("Resolves #{}", entity_iid_str));
                                            } else {
                                                cmd_args.push("--related-issue".into());
                                                cmd_args.push(entity_iid_str);
                                            }
                                        }
                                        if !title.is_empty() {
                                            cmd_args.push("--title".into());
                                            cmd_args.push(title);
                                        }
                                        if !source.is_empty() {
                                            let flag = if cli.is_github {
                                                "--head"
                                            } else {
                                                "--source-branch"
                                            };
                                            cmd_args.push(flag.to_string());
                                            cmd_args.push(source);
                                        }
                                        if !target.is_empty() {
                                            let flag = if cli.is_github {
                                                "--base"
                                            } else {
                                                "--target-branch"
                                            };
                                            cmd_args.push(flag.to_string());
                                            cmd_args.push(target);
                                        }
                                        if !labels.is_empty() {
                                            cmd_args.push("--label".into());
                                            cmd_args.push(labels.replace(", ", ","));
                                        }
                                        if !assignees.is_empty() {
                                            let clean = assignees
                                                .split(',')
                                                .map(|a| {
                                                    a.trim().trim_start_matches('@').to_string()
                                                })
                                                .collect::<Vec<_>>()
                                                .join(",");
                                            cmd_args.push("--assignee".into());
                                            cmd_args.push(clean);
                                        }
                                        if !reviewers.is_empty() {
                                            let clean = reviewers
                                                .split(',')
                                                .map(|r| {
                                                    r.trim().trim_start_matches('@').to_string()
                                                })
                                                .collect::<Vec<_>>()
                                                .join(",");
                                            cmd_args.push("--reviewer".into());
                                            cmd_args.push(clean);
                                        }
                                        if !milestone.is_empty() {
                                            cmd_args.push("--milestone".into());
                                            cmd_args.push(milestone);
                                        }
                                        if status.to_lowercase() == "draft" {
                                            cmd_args.push("--draft".into());
                                        }
                                        if mr_pipeline.to_lowercase() == "yes" {
                                            if cli.is_github {
                                                // gh pr create doesn't have --create-pipeline
                                            } else {
                                                cmd_args.push("--create-pipeline".into());
                                            }
                                        }
                                        if !description.is_empty() {
                                            cmd_args.push(cli.flag_description().to_string());
                                            cmd_args.push(description);
                                        } else if cli.is_github {
                                            cmd_args.push("--body".into());
                                            cmd_args.push("".into());
                                        }

                                        app.edit_menu = None;
                                        run_cli(
                                            &cli,
                                            &cmd_args,
                                            &mut terminal,
                                            events.sender(),
                                            app.active_tab,
                                        )
                                        .await;

                                        if let Some(client) = &app.gitlab_client {
                                            spawn_refresh_active_tab(
                                                client,
                                                &app.project_context,
                                                app.active_tab,
                                                events.sender(),
                                            );
                                        }
                                        continue;
                                    } else if entity_type == "new_milestone" {
                                        let title = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Title")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let description = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Description")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let start_date = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Start Date")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let due_date = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Due Date")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();

                                        let cli = app_cli(&app);
                                        let is_github = cli.is_github;
                                        let project_context = app.project_context.clone();
                                        let encoded_path = project_context.replace("/", "%2F");
                                        let tx = events.sender();
                                        let _ = tx.send(Event::CommandStarted(format!(
                                            "Creating milestone: {}",
                                            title
                                        )));
                                        app.edit_menu = None;
                                        tokio::spawn(async move {
                                            if is_github {
                                                let gh_repo = encoded_path.replace("%2F", "/");
                                                let due_on = if !due_date.is_empty()
                                                    && due_date != "YYYY-MM-DD"
                                                {
                                                    format!("{}T00:00:00Z", due_date.trim())
                                                } else {
                                                    "".to_string()
                                                };
                                                let mut args = vec![
                                                    "api".to_string(),
                                                    format!("repos/{}/milestones", gh_repo),
                                                    "-f".to_string(),
                                                    format!("title={}", title),
                                                ];
                                                if !description.is_empty() {
                                                    args.push("-f".to_string());
                                                    args.push(format!(
                                                        "description={}",
                                                        description
                                                    ));
                                                }
                                                if !due_on.is_empty() {
                                                    args.push("-f".to_string());
                                                    args.push(format!("due_on={}", due_on));
                                                }
                                                let cmd = tokio::process::Command::new("gh")
                                                    .args(&args)
                                                    .output()
                                                    .await;
                                                match cmd {
                                                    Ok(out) if out.status.success() => {
                                                        let _ = tx.send(Event::MilestoneUpdated);
                                                    }
                                                    Ok(out) => {
                                                        let err =
                                                            String::from_utf8_lossy(&out.stderr)
                                                                .trim()
                                                                .to_string();
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            app::Tab::Milestones,
                                                            Err(format!("Failed: {}", err)),
                                                        ));
                                                    }
                                                    Err(e) => {
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            app::Tab::Milestones,
                                                            Err(format!("Error: {}", e)),
                                                        ));
                                                    }
                                                }
                                            } else {
                                                let endpoint = format!(
                                                    "/projects/{}/milestones",
                                                    encoded_path
                                                );
                                                let mut args = vec![
                                                    "api".to_string(),
                                                    "-X".to_string(),
                                                    "POST".to_string(),
                                                    endpoint,
                                                    "-f".to_string(),
                                                    format!("title={}", title),
                                                ];
                                                if !description.is_empty() {
                                                    args.push("-f".to_string());
                                                    args.push(format!(
                                                        "description={}",
                                                        description
                                                    ));
                                                }
                                                if !start_date.is_empty()
                                                    && start_date != "YYYY-MM-DD"
                                                {
                                                    args.push("-f".to_string());
                                                    args.push(format!("start_date={}", start_date));
                                                }
                                                if !due_date.is_empty() && due_date != "YYYY-MM-DD"
                                                {
                                                    args.push("-f".to_string());
                                                    args.push(format!("due_date={}", due_date));
                                                }
                                                let cmd = tokio::process::Command::new("glab")
                                                    .args(&args)
                                                    .output()
                                                    .await;
                                                match cmd {
                                                    Ok(out) if out.status.success() => {
                                                        let _ = tx.send(Event::MilestoneUpdated);
                                                    }
                                                    Ok(out) => {
                                                        let err =
                                                            String::from_utf8_lossy(&out.stderr)
                                                                .trim()
                                                                .to_string();
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            app::Tab::Milestones,
                                                            Err(format!("Failed: {}", err)),
                                                        ));
                                                    }
                                                    Err(e) => {
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            app::Tab::Milestones,
                                                            Err(format!("Error: {}", e)),
                                                        ));
                                                    }
                                                }
                                            }
                                        });
                                        continue;
                                    } else if entity_type == "new_pipeline" {
                                        let cli = app_cli(&app);
                                        let branch = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Branch / Ref")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let mr = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Merge Request Pipeline")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let variables = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Variables")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let inputs = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Inputs")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let workflow = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Workflow / CI File (GitHub)")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();

                                        let var_pairs = parse_key_value_pairs(&variables);
                                        let mut var_strs = Vec::new();
                                        for (k, v) in &var_pairs {
                                            var_strs.push(format!(
                                                "{}{}{}",
                                                k,
                                                cli.input_separator(),
                                                v
                                            ));
                                        }

                                        let input_pairs = parse_key_value_pairs(&inputs);
                                        let mut input_strs = Vec::new();
                                        for (k, v) in &input_pairs {
                                            input_strs.push(format!(
                                                "{}{}{}",
                                                k,
                                                cli.input_separator(),
                                                v
                                            ));
                                        }

                                        let mut cmd_args: Vec<String> = vec![
                                            if cli.is_github {
                                                "workflow".into()
                                            } else {
                                                "ci".into()
                                            },
                                            "run".into(),
                                        ];
                                        if !workflow.is_empty() {
                                            cmd_args.push(workflow);
                                        }
                                        if !branch.is_empty() {
                                            cmd_args.push(cli.flag_branch().to_string());
                                            cmd_args.push(branch);
                                        }
                                        if mr.to_lowercase() == "yes" && !cli.is_github {
                                            cmd_args.push("--mr".into());
                                        }

                                        let var_flag = cli.flag_variable();
                                        for s in &var_strs {
                                            cmd_args.push(var_flag.to_string());
                                            cmd_args.push(s.clone());
                                        }
                                        let input_flag = cli.flag_input();
                                        for s in &input_strs {
                                            cmd_args.push(input_flag.to_string());
                                            cmd_args.push(s.clone());
                                        }

                                        app.edit_menu = None;
                                        run_cli(
                                            &cli,
                                            &cmd_args,
                                            &mut terminal,
                                            events.sender(),
                                            app.active_tab,
                                        )
                                        .await;

                                        if let Some(client) = &app.gitlab_client {
                                            spawn_refresh_active_tab(
                                                client,
                                                &app.project_context,
                                                app.active_tab,
                                                events.sender(),
                                            );
                                        }
                                    } else if entity_type == "new_release" {
                                        let tag = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Tag")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let name = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Release Name")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();
                                        let description = menu
                                            .fields
                                            .iter()
                                            .find(|(k, _)| k == "Description")
                                            .map(|(_, v)| v.trim().to_string())
                                            .unwrap_or_default();

                                        if !tag.is_empty() {
                                            let cli = app_cli(&app);
                                            let mut cmd_args = vec![
                                                "release".to_string(),
                                                "create".to_string(),
                                                tag,
                                            ];
                                            if !name.is_empty() {
                                                cmd_args.push("-n".to_string());
                                                cmd_args.push(name);
                                            }
                                            if !description.is_empty() {
                                                if cli.is_github {
                                                    cmd_args.push("-n".to_string());
                                                    cmd_args.push(description);
                                                } else {
                                                    cmd_args.push("-N".to_string());
                                                    cmd_args.push(description);
                                                }
                                            }
                                            app.edit_menu = None;
                                            run_cli(
                                                &cli,
                                                &cmd_args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;

                                            if let Some(client) = &app.gitlab_client {
                                                spawn_refresh_active_tab(
                                                    client,
                                                    &app.project_context,
                                                    app.active_tab,
                                                    events.sender(),
                                                );
                                            }
                                        }
                                        continue;
                                    }
                                }

                                // Not on submit — act on the currently selected field
                                let field_name = if menu.selected_idx < menu.fields.len() {
                                    menu.fields[menu.selected_idx].0.clone()
                                } else {
                                    String::new()
                                };

                                if field_name == "Labels"
                                    || field_name == "Assignees"
                                    || field_name == "Reviewers"
                                    || field_name == "Milestone"
                                    || field_name == "Confidential"
                                    || field_name == "Status (Draft/Ready)"
                                    || field_name == "Merge Request Pipeline"
                                    || field_name == "Source Branch"
                                    || field_name == "Target Branch"
                                {
                                    let mut current_set = std::collections::HashSet::new();
                                    let field_type = match field_name.as_str() {
                                        "Labels" => "labels",
                                        "Assignees" => "assignees",
                                        "Reviewers" => "reviewers",
                                        "Milestone" => "milestone",
                                        "Confidential" => "confidential",
                                        "Status (Draft/Ready)" => "draft_status",
                                        "Merge Request Pipeline" => "mr_pipeline",
                                        "Source Branch" => "source_branch",
                                        "Target Branch" => "target_branch",
                                        _ => "",
                                    };
                                    let multi_select = match field_type {
                                        "labels" | "assignees" | "reviewers" => true,
                                        _ => false,
                                    };

                                    let mut all_items = Vec::new();
                                    let mut is_loading = true;

                                    if field_type == "confidential" {
                                        all_items =
                                            vec!["Public".to_string(), "Confidential".to_string()];
                                        is_loading = false;
                                    } else if field_type == "draft_status" {
                                        all_items = vec!["Draft".to_string(), "Ready".to_string()];
                                        is_loading = false;
                                        let is_new_entity =
                                            entity_iid == 0 || entity_type.starts_with("new_");
                                        if is_new_entity {
                                            let current_val =
                                                menu.fields[menu.selected_idx].1.clone();
                                            if !current_val.is_empty() {
                                                current_set.insert(current_val);
                                            } else {
                                                current_set.insert("Ready".to_string());
                                            }
                                        } else if let Some(mr) =
                                            app.mrs.items.iter().find(|m| m.iid == entity_iid)
                                        {
                                            current_set.insert(if mr.draft {
                                                "Draft".to_string()
                                            } else {
                                                "Ready".to_string()
                                            });
                                        }
                                    } else if field_type == "mr_pipeline" {
                                        all_items = vec!["Yes".to_string(), "No".to_string()];
                                        is_loading = false;
                                        let is_new_entity =
                                            entity_iid == 0 || entity_type.starts_with("new_");
                                        if is_new_entity {
                                            let current_val =
                                                menu.fields[menu.selected_idx].1.clone();
                                            if !current_val.is_empty() {
                                                current_set.insert(current_val);
                                            } else {
                                                current_set.insert("No".to_string());
                                            }
                                        }
                                    } else if field_type == "source_branch"
                                        || field_type == "target_branch"
                                    {
                                        all_items = get_branches();
                                        is_loading = false;
                                    }

                                    if entity_iid == 0 || entity_type.starts_with("new_") {
                                        let current_val = menu.fields[menu.selected_idx].1.clone();
                                        if !current_val.is_empty()
                                            && field_type != "draft_status"
                                            && field_type != "mr_pipeline"
                                        {
                                            if multi_select {
                                                for item in current_val.split(',') {
                                                    let trimmed = item.trim().to_string();
                                                    if !trimmed.is_empty() {
                                                        current_set.insert(trimmed);
                                                    }
                                                }
                                            } else {
                                                current_set.insert(current_val);
                                            }
                                        }
                                    } else if entity_type == "issue" {
                                        if let Some(issue) =
                                            app.issues.items.iter().find(|i| i.iid == entity_iid)
                                        {
                                            match field_type {
                                                "labels" => {
                                                    for l in &issue.labels {
                                                        current_set.insert(l.clone());
                                                    }
                                                }
                                                "assignees" => {
                                                    for a in &issue.assignees {
                                                        current_set
                                                            .insert(format!("@{}", a.username));
                                                    }
                                                }
                                                "milestone" => {
                                                    if let Some(m) = &issue.milestone {
                                                        current_set.insert(m.title.clone());
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                    } else if entity_type == "mr" {
                                        if let Some(mr) =
                                            app.mrs.items.iter().find(|m| m.iid == entity_iid)
                                        {
                                            match field_type {
                                                "labels" => {
                                                    for l in &mr.labels {
                                                        current_set.insert(l.clone());
                                                    }
                                                }
                                                "assignees" => {
                                                    for a in &mr.assignees {
                                                        current_set
                                                            .insert(format!("@{}", a.username));
                                                    }
                                                }
                                                "reviewers" => {
                                                    for r in &mr.reviewers {
                                                        current_set
                                                            .insert(format!("@{}", r.username));
                                                    }
                                                }
                                                "milestone" => {
                                                    if let Some(m) = &mr.milestone {
                                                        current_set.insert(m.title.clone());
                                                    }
                                                }
                                                "target_branch" => {
                                                    current_set.insert(mr.target_branch.clone());
                                                }
                                                _ => {}
                                            }
                                        }
                                    }

                                    let start_idx = if multi_select {
                                        0
                                    } else {
                                        current_set
                                            .iter()
                                            .next()
                                            .and_then(|sel| all_items.iter().position(|a| a == sel))
                                            .unwrap_or(0)
                                    };

                                    app.selector = Some(crate::app::Selector {
                                        title: format!("Select {}", field_name),
                                        all_items,
                                        selected_items: current_set,
                                        cursor_idx: start_idx,
                                        search_query: String::new(),
                                        is_filtering: false,
                                        is_loading,
                                        entity_iid,
                                        entity_type: entity_type.clone(),
                                        field_type: field_type.to_string(),
                                        multi_select,
                                        state: {
                                            let mut s = ListState::default();
                                            s.select(Some(0));
                                            s
                                        },
                                    });

                                    app.edit_menu = Some(menu);

                                    if is_loading {
                                        if let Some(client) = &app.gitlab_client {
                                            let client = client.clone();
                                            let project_context = app.project_context.clone();
                                            let field_type = field_type.to_string();
                                            let tx = events.sender();
                                            tokio::spawn(async move {
                                                let res = match field_type.as_str() {
                                                    "labels" => {
                                                        client.fetch_labels(&project_context).await
                                                    }
                                                    "assignees" | "reviewers" => {
                                                        client.fetch_members(&project_context).await
                                                    }
                                                    "milestone" => {
                                                        client
                                                            .fetch_milestones(&project_context)
                                                            .await
                                                    }
                                                    _ => Ok(Vec::new()),
                                                };
                                                if let Ok(items) = res {
                                                    let _ =
                                                        tx.send(Event::SelectorItemsFetched(items));
                                                } else {
                                                    let _ = tx.send(Event::SelectorItemsFetched(
                                                        Vec::new(),
                                                    ));
                                                }
                                            });
                                        }
                                    }
                                    continue;
                                }

                                if field_name == "Description" {
                                    let current_val = if entity_iid == 0
                                        || entity_type.starts_with("new_")
                                    {
                                        let raw_val = menu.fields[menu.selected_idx].1.clone();
                                        if raw_val.trim().is_empty() {
                                            let template_type = if entity_type == "new_mr" {
                                                "mr"
                                            } else {
                                                "issue"
                                            };
                                            get_default_template(template_type).unwrap_or_default()
                                        } else {
                                            raw_val
                                        }
                                    } else {
                                        if entity_type == "issue" {
                                            app.issues
                                                .items
                                                .iter()
                                                .find(|i| i.iid == entity_iid)
                                                .and_then(|i| i.description.clone())
                                                .unwrap_or_default()
                                        } else {
                                            app.mrs
                                                .items
                                                .iter()
                                                .find(|m| m.iid == entity_iid)
                                                .and_then(|m| m.description.clone())
                                                .unwrap_or_default()
                                        }
                                    };
                                    let action =
                                        if entity_iid == 0 || entity_type.starts_with("new_") {
                                            crate::app::TextInputAction::EditNewField {
                                                field_idx: menu.selected_idx,
                                            }
                                        } else {
                                            crate::app::TextInputAction::EditField {
                                                entity_iid,
                                                entity_type: entity_type.clone(),
                                                field_type: "description".to_string(),
                                            }
                                        };
                                    app.text_input = Some(crate::app::TextInput {
                                        title: " Edit Description ".to_string(),
                                        value: current_val.clone(),
                                        cursor_idx: current_val.len(),
                                        action,
                                    });
                                    app.edit_menu = Some(menu);
                                    continue;
                                }

                                if field_name == "Due Date" || field_name == "Start Date" {
                                    let current_val =
                                        if entity_iid == 0 || entity_type.starts_with("new_") {
                                            menu.fields[menu.selected_idx].1.clone()
                                        } else {
                                            if entity_type == "issue" {
                                                app.issues
                                                    .items
                                                    .iter()
                                                    .find(|i| i.iid == entity_iid)
                                                    .and_then(|i| i.due_date.clone())
                                                    .unwrap_or_default()
                                            } else if entity_type == "milestone" {
                                                let m = app
                                                    .milestones
                                                    .items
                                                    .iter()
                                                    .find(|m| m.iid == entity_iid);
                                                if field_name == "Start Date" {
                                                    m.and_then(|m| m.start_date.clone())
                                                        .unwrap_or_default()
                                                } else {
                                                    m.and_then(|m| m.due_date.clone())
                                                        .unwrap_or_default()
                                                }
                                            } else {
                                                String::new()
                                            }
                                        };
                                    let action =
                                        if entity_iid == 0 || entity_type.starts_with("new_") {
                                            crate::app::DatePickerAction::EditNewField {
                                                field_idx: menu.selected_idx,
                                            }
                                        } else {
                                            crate::app::DatePickerAction::EditField {
                                                entity_iid,
                                                entity_type: entity_type.clone(),
                                                field_type: if field_name == "Start Date" {
                                                    "start_date".to_string()
                                                } else {
                                                    "due_date".to_string()
                                                },
                                            }
                                        };
                                    app.date_picker = Some(crate::app::DatePicker::new(
                                        format!(" Select {}", field_name),
                                        &current_val,
                                        action,
                                    ));
                                    app.edit_menu = Some(menu);
                                    continue;
                                }

                                if field_name == "Title"
                                    || field_name == "Weight"
                                    || field_name == "Branch / Ref"
                                    || field_name == "Variables"
                                    || field_name == "Inputs"
                                    || field_name == "Workflow / CI File (GitHub)"
                                    || field_name == "Release Name"
                                    || field_name == "Tag"
                                {
                                    let current_val =
                                        if entity_iid == 0 || entity_type.starts_with("new_") {
                                            menu.fields[menu.selected_idx].1.clone()
                                        } else {
                                            let field_type = match field_name.as_str() {
                                                "Title" => "title",
                                                "Target Branch" => "target_branch",
                                                "Weight" => "weight",
                                                "Release Name" => "release_name",
                                                "Tag" => "tag",
                                                _ => "",
                                            };
                                            match field_type {
                                                "title" => {
                                                    if entity_type == "issue" {
                                                        app.issues
                                                            .items
                                                            .iter()
                                                            .find(|i| i.iid == entity_iid)
                                                            .map(|i| i.title.clone())
                                                            .unwrap_or_default()
                                                    } else if entity_type == "milestone" {
                                                        app.milestones
                                                            .items
                                                            .iter()
                                                            .find(|m| m.iid == entity_iid)
                                                            .map(|m| m.title.clone())
                                                            .unwrap_or_default()
                                                    } else {
                                                        app.mrs
                                                            .items
                                                            .iter()
                                                            .find(|m| m.iid == entity_iid)
                                                            .map(|m| m.title.clone())
                                                            .unwrap_or_default()
                                                    }
                                                }
                                                "target_branch" => app
                                                    .mrs
                                                    .items
                                                    .iter()
                                                    .find(|m| m.iid == entity_iid)
                                                    .map(|m| m.target_branch.clone())
                                                    .unwrap_or_default(),
                                                "weight" => "0".to_string(),
                                                "release_name" => app
                                                    .releases
                                                    .items
                                                    .get(entity_iid as usize)
                                                    .map(|r| r.name.clone())
                                                    .unwrap_or_default(),
                                                "tag" => app
                                                    .releases
                                                    .items
                                                    .get(entity_iid as usize)
                                                    .map(|r| r.tag_name.clone())
                                                    .unwrap_or_default(),
                                                _ => String::new(),
                                            }
                                        };

                                    let action =
                                        if entity_iid == 0 || entity_type.starts_with("new_") {
                                            crate::app::TextInputAction::EditNewField {
                                                field_idx: menu.selected_idx,
                                            }
                                        } else {
                                            let field_type = match field_name.as_str() {
                                                "Title" => "title",
                                                "Target Branch" => "target_branch",
                                                "Weight" => "weight",
                                                "Release Name" => "release_name",
                                                "Tag" => "tag",
                                                _ => "",
                                            };
                                            crate::app::TextInputAction::EditField {
                                                entity_iid,
                                                entity_type: entity_type.clone(),
                                                field_type: field_type.to_string(),
                                            }
                                        };

                                    app.text_input = Some(crate::app::TextInput {
                                        title: format!("Edit {}", field_name),
                                        cursor_idx: current_val.len(),
                                        value: current_val,
                                        action,
                                    });

                                    app.edit_menu = Some(menu);
                                    continue;
                                }
                            }
                            _ => {
                                app.edit_menu = Some(menu);
                            }
                        }
                        continue;
                    }

                    if let Some(mut diff_view) = app.diff_view.take() {
                        let in_selection = diff_view.selection_start.is_some();
                        match key_event.code {
                            KeyCode::Esc => {
                                if in_selection {
                                    diff_view.selection_start = None;
                                    diff_view.selection_end = None;
                                } else if !diff_view.focus_on_files {
                                    diff_view.focus_on_files = true;
                                } else {
                                    if !app.draft_comments.is_empty() {
                                        app.show_submit_review_prompt = Some(diff_view.mr_iid);
                                    } else {
                                        app.diff_view = None;
                                        continue;
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('q') => {
                                if in_selection {
                                    diff_view.selection_start = None;
                                    diff_view.selection_end = None;
                                } else {
                                    if !app.draft_comments.is_empty() {
                                        app.show_submit_review_prompt = Some(diff_view.mr_iid);
                                    } else {
                                        app.diff_view = None;
                                        continue;
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Tab => {
                                diff_view.focus_on_files = !diff_view.focus_on_files;
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('h') | KeyCode::Left => {
                                if diff_view.focus_on_files {
                                    if !diff_view.visible_nodes.is_empty() {
                                        let node = &diff_view.visible_nodes
                                            [diff_view.selected_visible_idx];
                                        if node.is_dir && node.is_expanded {
                                            let path_id = node.path_id.clone();
                                            diff_view.root_node.toggle_expanded(&path_id, "");
                                            let mut visible = Vec::new();
                                            diff_view.root_node.flatten(0, "", &mut visible);
                                            diff_view.visible_nodes = visible;
                                            diff_view.cursor_idx = 0;
                                            diff_view.scroll_offset = 0;
                                            diff_view.update_active_lines();
                                        }
                                    }
                                } else {
                                    diff_view.focus_on_files = true;
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('l') | KeyCode::Right => {
                                if diff_view.focus_on_files {
                                    if !diff_view.visible_nodes.is_empty() {
                                        let node = &diff_view.visible_nodes
                                            [diff_view.selected_visible_idx];
                                        if node.is_dir && !node.is_expanded {
                                            let path_id = node.path_id.clone();
                                            diff_view.root_node.toggle_expanded(&path_id, "");
                                            let mut visible = Vec::new();
                                            diff_view.root_node.flatten(0, "", &mut visible);
                                            diff_view.visible_nodes = visible;
                                            diff_view.cursor_idx = 0;
                                            diff_view.scroll_offset = 0;
                                            diff_view.update_active_lines();
                                        } else {
                                            diff_view.focus_on_files = false;
                                        }
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Enter | KeyCode::Char(' ') => {
                                if diff_view.focus_on_files {
                                    if !diff_view.visible_nodes.is_empty() {
                                        let node = &diff_view.visible_nodes
                                            [diff_view.selected_visible_idx];
                                        if node.is_dir {
                                            let path_id = node.path_id.clone();
                                            diff_view.root_node.toggle_expanded(&path_id, "");
                                            let mut visible = Vec::new();
                                            diff_view.root_node.flatten(0, "", &mut visible);
                                            diff_view.visible_nodes = visible;
                                            diff_view.cursor_idx = 0;
                                            diff_view.scroll_offset = 0;
                                            diff_view.update_active_lines();
                                        } else {
                                            diff_view.focus_on_files = false;
                                        }
                                    }
                                } else {
                                    diff_view.focus_on_files = true;
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('d') => {
                                if !diff_view.focus_on_files {
                                    let old_side_by_side = diff_view.side_by_side;
                                    let old_cursor = diff_view.cursor_idx;
                                    diff_view.side_by_side = !diff_view.side_by_side;
                                    diff_view.update_active_lines();

                                    if old_side_by_side {
                                        if let Some(sline) =
                                            diff_view.side_by_side_lines.get(old_cursor)
                                        {
                                            let target_line =
                                                sline.right.as_ref().or(sline.left.as_ref());
                                            if let Some(target) = target_line {
                                                if let Some(new_idx) =
                                                    diff_view.lines.iter().position(|l| {
                                                        l.file_path == target.file_path
                                                            && l.old_line_num == target.old_line_num
                                                            && l.new_line_num == target.new_line_num
                                                            && l.line_type == target.line_type
                                                    })
                                                {
                                                    diff_view.cursor_idx = new_idx;
                                                }
                                            }
                                        }
                                    } else {
                                        if let Some(uline) = diff_view.lines.get(old_cursor) {
                                            if let Some(new_idx) =
                                                diff_view.side_by_side_lines.iter().position(|l| {
                                                    if uline.line_type
                                                        == crate::app::DiffLineType::HunkHeader
                                                        || uline.line_type
                                                            == crate::app::DiffLineType::Meta
                                                    {
                                                        l.line_type == uline.line_type
                                                            && l.left.as_ref().map_or(false, |x| {
                                                                x.content == uline.content
                                                            })
                                                    } else {
                                                        l.left.as_ref().map_or(false, |x| {
                                                            x.old_line_num == uline.old_line_num
                                                                && x.new_line_num
                                                                    == uline.new_line_num
                                                                && x.file_path == uline.file_path
                                                        }) || l.right.as_ref().map_or(false, |x| {
                                                            x.old_line_num == uline.old_line_num
                                                                && x.new_line_num
                                                                    == uline.new_line_num
                                                                && x.file_path == uline.file_path
                                                        })
                                                    }
                                                })
                                            {
                                                diff_view.cursor_idx = new_idx;
                                            }
                                        }
                                    }

                                    diff_view.scroll_offset =
                                        diff_view.cursor_idx.saturating_sub(5);
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('j') | KeyCode::Down => {
                                if diff_view.focus_on_files {
                                    if !diff_view.visible_nodes.is_empty() {
                                        let old_idx = diff_view.selected_visible_idx;
                                        diff_view.selected_visible_idx =
                                            (diff_view.selected_visible_idx + 1)
                                                .min(diff_view.visible_nodes.len() - 1);
                                        if diff_view.selected_visible_idx != old_idx {
                                            diff_view.cursor_idx = 0;
                                            diff_view.scroll_offset = 0;
                                            diff_view.update_active_lines();
                                        }
                                    }
                                } else {
                                    let active_len = if diff_view.side_by_side {
                                        diff_view.side_by_side_lines.len()
                                    } else {
                                        diff_view.lines.len()
                                    };
                                    if active_len > 0 {
                                        let new_idx =
                                            (diff_view.cursor_idx + 1).min(active_len - 1);
                                        if in_selection {
                                            diff_view.selection_end = Some(new_idx);
                                        }
                                        diff_view.cursor_idx = new_idx;
                                        diff_view.update_selected_file_from_cursor();
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                if diff_view.focus_on_files {
                                    if diff_view.selected_visible_idx > 0 {
                                        let old_idx = diff_view.selected_visible_idx;
                                        diff_view.selected_visible_idx -= 1;
                                        if diff_view.selected_visible_idx != old_idx {
                                            diff_view.cursor_idx = 0;
                                            diff_view.scroll_offset = 0;
                                            diff_view.update_active_lines();
                                        }
                                    }
                                } else {
                                    if diff_view.cursor_idx > 0 {
                                        let new_idx = diff_view.cursor_idx - 1;
                                        if in_selection {
                                            diff_view.selection_end = Some(new_idx);
                                        }
                                        diff_view.cursor_idx = new_idx;
                                        diff_view.update_selected_file_from_cursor();
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('J') => {
                                let active_len = if diff_view.side_by_side {
                                    diff_view.side_by_side_lines.len()
                                } else {
                                    diff_view.lines.len()
                                };
                                if active_len > 0 {
                                    let scroll_amount = 10;
                                    let new_idx =
                                        (diff_view.cursor_idx + scroll_amount).min(active_len - 1);
                                    if in_selection && !diff_view.focus_on_files {
                                        diff_view.selection_end = Some(new_idx);
                                    }
                                    diff_view.cursor_idx = new_idx;
                                    if !diff_view.focus_on_files {
                                        diff_view.update_selected_file_from_cursor();
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('K') => {
                                let scroll_amount = 10;
                                let new_idx = diff_view.cursor_idx.saturating_sub(scroll_amount);
                                if in_selection && !diff_view.focus_on_files {
                                    diff_view.selection_end = Some(new_idx);
                                }
                                diff_view.cursor_idx = new_idx;
                                if !diff_view.focus_on_files {
                                    diff_view.update_selected_file_from_cursor();
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('v') | KeyCode::Char('V') => {
                                if !diff_view.focus_on_files {
                                    if in_selection {
                                        diff_view.selection_start = None;
                                        diff_view.selection_end = None;
                                        app.status_message =
                                            Some("Selection cancelled.".to_string());
                                    } else {
                                        diff_view.selection_start = Some(diff_view.cursor_idx);
                                        diff_view.selection_end = Some(diff_view.cursor_idx);
                                        app.status_message = Some(
                                            "Selection started. Use j/k to extend, Esc to cancel, c to comment."
                                                .to_string(),
                                        );
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('a') => {
                                if !diff_view.focus_on_files {
                                    let sline = if diff_view.side_by_side {
                                        diff_view
                                            .side_by_side_lines
                                            .get(diff_view.cursor_idx)
                                            .cloned()
                                    } else {
                                        diff_view.lines.get(diff_view.cursor_idx).map(|l| {
                                            crate::app::SideBySideLine {
                                                left: Some(l.clone()),
                                                right: Some(l.clone()),
                                                line_type: l.line_type.clone(),
                                            }
                                        })
                                    };

                                    if let Some(sline) = sline {
                                        let matching_current: Vec<_> = app
                                            .current_comments
                                            .iter()
                                            .filter(|c| {
                                                if c.system {
                                                    return false;
                                                }
                                                if let Some(ref pos) = c.position {
                                                    let path_matches =
                                                        sline.left.as_ref().map_or(false, |l| {
                                                            pos.old_path.as_deref()
                                                                == Some(&l.file_path)
                                                        }) || sline.right.as_ref().map_or(
                                                            false,
                                                            |r| {
                                                                pos.new_path.as_deref()
                                                                    == Some(&r.file_path)
                                                            },
                                                        );

                                                    path_matches
                                                        && ((pos.new_line.is_some()
                                                            && sline.right.as_ref().and_then(
                                                                |r| {
                                                                    r.new_line_num.map(|n| n as u64)
                                                                },
                                                            ) == pos.new_line)
                                                            || (pos.old_line.is_some()
                                                                && sline.left.as_ref().and_then(
                                                                    |l| {
                                                                        l.old_line_num
                                                                            .map(|n| n as u64)
                                                                    },
                                                                ) == pos.old_line))
                                                } else {
                                                    false
                                                }
                                            })
                                            .collect();

                                        if matching_current.is_empty() {
                                            app.status_message = Some(
                                                "No comments on this line to interact with."
                                                    .to_string(),
                                            );
                                        } else if matching_current.len() == 1 {
                                            let comment = matching_current[0];
                                            let comment_id = comment.id;
                                            let is_github = app
                                                .gitlab_client
                                                .as_ref()
                                                .map_or(false, |c| c.is_github);

                                            let mut actions = vec!["Reply to Thread".to_string()];

                                            if !is_github {
                                                let is_resolved = comment.resolved.unwrap_or(false);
                                                if is_resolved {
                                                    actions.push("Unresolve Thread".to_string());
                                                } else {
                                                    actions.push("Resolve Thread".to_string());
                                                }
                                            }

                                            actions.push("Edit Comment".to_string());
                                            actions.push("Delete Comment".to_string());

                                            app.selector = Some(crate::app::Selector {
                                                title: format!(
                                                    " Actions for Comment {} ",
                                                    comment_id
                                                ),
                                                all_items: actions,
                                                selected_items: std::collections::HashSet::new(),
                                                cursor_idx: 0,
                                                search_query: String::new(),
                                                is_filtering: false,
                                                is_loading: false,
                                                entity_iid: comment_id,
                                                entity_type: diff_view.mr_iid.to_string(),
                                                field_type: "comment_action_select".to_string(),
                                                multi_select: false,
                                                state: ListState::default(),
                                            });
                                        } else {
                                            let items: Vec<String> = matching_current
                                                .iter()
                                                .map(|c| {
                                                    let clean_body = c.body.replace('\n', " ");
                                                    let truncated = if clean_body.len() > 40 {
                                                        format!("{}...", &clean_body[..40])
                                                    } else {
                                                        clean_body
                                                    };
                                                    format!(
                                                        "ID: {} | @{}: {}",
                                                        c.id, c.author.username, truncated
                                                    )
                                                })
                                                .collect();

                                            app.selector = Some(crate::app::Selector {
                                                title: " Select Comment to Interact ".to_string(),
                                                all_items: items,
                                                selected_items: std::collections::HashSet::new(),
                                                cursor_idx: 0,
                                                search_query: String::new(),
                                                is_filtering: false,
                                                is_loading: false,
                                                entity_iid: diff_view.mr_iid,
                                                entity_type: "mr".to_string(),
                                                field_type: "comment_select".to_string(),
                                                multi_select: false,
                                                state: ListState::default(),
                                            });
                                        }
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('c') => {
                                if let Some(range) = diff_view.get_comment_range() {
                                    app.text_input = Some(crate::app::TextInput {
                                        title: format!(" Add Comment to {} ", range.file_path),
                                        value: String::new(),
                                        cursor_idx: 0,
                                        action: crate::app::TextInputAction::AddReviewComment {
                                            mr_iid: diff_view.mr_iid,
                                            file_path: range.file_path,
                                            line_num: range.line_num,
                                            old_line_num: range.old_line_num,
                                            end_line_num: range.end_line_num,
                                            end_old_line_num: range.end_old_line_num,
                                        },
                                    });
                                    // Clear selection after starting a comment
                                    diff_view.selection_start = None;
                                    diff_view.selection_end = None;
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('C') => {
                                if let Some(range) = diff_view.get_comment_range() {
                                    app.status_message =
                                        Some("Opening editor for comment...".to_string());
                                    let comment_content = edit_in_editor("", &mut terminal);
                                    if let Some(body) = comment_content {
                                        if !body.trim().is_empty() {
                                            if app.in_review_mode {
                                                app.draft_comments.push(crate::app::DraftComment {
                                                    file_path: range.file_path.clone(),
                                                    line_num: range.line_num,
                                                    old_line_num: range.old_line_num,
                                                    end_line_num: range.end_line_num,
                                                    end_old_line_num: range.end_old_line_num,
                                                    body,
                                                });
                                                app.status_message = Some(format!(
                                                    "Added draft comment. ({} pending)",
                                                    app.draft_comments.len()
                                                ));
                                            } else {
                                                let cli = app_cli(&app);
                                                let mut args = if cli.is_github {
                                                    vec![
                                                        "pr".to_string(),
                                                        "comment".to_string(),
                                                        diff_view.mr_iid.to_string(),
                                                        "--body".to_string(),
                                                        body,
                                                    ]
                                                } else {
                                                    vec![
                                                        "mr".to_string(),
                                                        "note".to_string(),
                                                        "create".to_string(),
                                                        diff_view.mr_iid.to_string(),
                                                        "--file-path".to_string(),
                                                        range.file_path.clone(),
                                                        "-m".to_string(),
                                                        body,
                                                    ]
                                                };
                                                if !cli.is_github {
                                                    if let Some(ln) = range.line_num {
                                                        args.push("--line".to_string());
                                                        args.push(ln.to_string());
                                                    } else if let Some(old_line) =
                                                        range.old_line_num
                                                    {
                                                        args.push("--old-line".to_string());
                                                        args.push(old_line.to_string());
                                                    }
                                                }
                                                run_cli(
                                                    &cli,
                                                    &args,
                                                    &mut terminal,
                                                    events.sender(),
                                                    app.active_tab,
                                                )
                                                .await;
                                            }
                                        }
                                    }
                                    // Clear selection after starting a comment
                                    diff_view.selection_start = None;
                                    diff_view.selection_end = None;
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('r') => {
                                let is_github =
                                    app.gitlab_client.as_ref().map_or(false, |c| c.is_github);
                                app.selector = Some(crate::app::Selector {
                                    title: format!(
                                        " Submit {} Review ",
                                        if is_github {
                                            "Pull Request"
                                        } else {
                                            "Merge Request"
                                        }
                                    ),
                                    all_items: vec![
                                        "Approve".to_string(),
                                        "Request Changes".to_string(),
                                        "Comment".to_string(),
                                    ],
                                    selected_items: std::collections::HashSet::new(),
                                    cursor_idx: 0,
                                    search_query: String::new(),
                                    is_filtering: false,
                                    is_loading: false,
                                    entity_iid: diff_view.mr_iid,
                                    entity_type: "mr".to_string(),
                                    field_type: "review_submit_status".to_string(),
                                    multi_select: false,
                                    state: ListState::default(),
                                });
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('e') => {
                                if let Some(range) = diff_view.get_comment_range() {
                                    let content = range
                                        .lines
                                        .iter()
                                        .map(|l| {
                                            let c = l.content.as_str();
                                            if c.starts_with('+')
                                                || c.starts_with('-')
                                                || c.starts_with(' ')
                                            {
                                                if c.len() > 1 { &c[1..] } else { "" }
                                            } else {
                                                c
                                            }
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n");

                                    app.status_message =
                                        Some("Opening editor for code suggestion...".to_string());
                                    let ext = std::path::Path::new(&range.file_path)
                                        .extension()
                                        .and_then(|s| s.to_str())
                                        .unwrap_or("md");
                                    let suffix = format!(".{}", ext);
                                    let editor_content = edit_in_editor_with_suffix(
                                        &content,
                                        &suffix,
                                        &mut terminal,
                                    );
                                    if let Some(suggestion) = editor_content {
                                        let body = format!("```suggestion\n{}\n```", suggestion);

                                        if app.in_review_mode {
                                            app.draft_comments.push(crate::app::DraftComment {
                                                file_path: range.file_path.clone(),
                                                line_num: range.line_num,
                                                old_line_num: range.old_line_num,
                                                end_line_num: range.end_line_num,
                                                end_old_line_num: range.end_old_line_num,
                                                body,
                                            });
                                            app.status_message = Some(format!(
                                                "Added suggestion draft. ({} pending)",
                                                app.draft_comments.len()
                                            ));
                                        } else {
                                            let cli = app_cli(&app);
                                            let mut args = if cli.is_github {
                                                vec![
                                                    "pr".to_string(),
                                                    "comment".to_string(),
                                                    diff_view.mr_iid.to_string(),
                                                    "--body".to_string(),
                                                    body,
                                                ]
                                            } else {
                                                vec![
                                                    "mr".to_string(),
                                                    "note".to_string(),
                                                    "create".to_string(),
                                                    diff_view.mr_iid.to_string(),
                                                    "--file-path".to_string(),
                                                    range.file_path.clone(),
                                                    "-m".to_string(),
                                                    body,
                                                ]
                                            };
                                            if !cli.is_github {
                                                if let Some(ln) = range.line_num {
                                                    args.push("--line".to_string());
                                                    args.push(ln.to_string());
                                                } else if let Some(oln) = range.old_line_num {
                                                    args.push("--old-line".to_string());
                                                    args.push(oln.to_string());
                                                }
                                            }
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                        }
                                    }
                                    diff_view.selection_start = None;
                                    diff_view.selection_end = None;
                                }
                                app.diff_view = Some(diff_view);
                            }
                            _ => {
                                app.diff_view = Some(diff_view);
                            }
                        }
                        continue;
                    }

                    if app.focus_column_checklist {
                        let is_github = app
                            .gitlab_client
                            .as_ref()
                            .map(|c| c.is_github)
                            .unwrap_or(false);
                        let cols = app.active_tab.columns(is_github);
                        let group_cols: Vec<&str> = cols.iter().copied().collect();
                        let cols_end = cols.len();
                        let group_end = cols_end + group_cols.len();
                        let order_end = group_end + 2;
                        let theme_end = order_end + crate::config::THEME_PRESETS.len();
                        let max_idx = theme_end.saturating_sub(1);

                        match key_event.code {
                            KeyCode::Esc | KeyCode::Char(',') => {
                                app.focus_column_checklist = false;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                if app.column_checklist_idx < max_idx {
                                    app.column_checklist_idx += 1;
                                } else {
                                    app.column_checklist_idx = 0;
                                }
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                if app.column_checklist_idx > 0 {
                                    app.column_checklist_idx -= 1;
                                } else {
                                    app.column_checklist_idx = max_idx;
                                }
                            }
                            KeyCode::Char('J') => {
                                app.column_checklist_idx = match app.column_checklist_idx {
                                    idx if idx < cols_end => cols_end,
                                    idx if idx < group_end => group_end,
                                    idx if idx < order_end => order_end,
                                    _ => 0,
                                };
                            }
                            KeyCode::Char('K') => {
                                app.column_checklist_idx = match app.column_checklist_idx {
                                    idx if idx >= order_end => cols_end,
                                    idx if idx >= group_end => 0,
                                    _ => order_end,
                                };
                            }
                            KeyCode::Char(' ') => {
                                let idx = app.column_checklist_idx;
                                if idx < cols_end {
                                    if let Some(col_name) = cols.get(idx) {
                                        let col_str = col_name.to_string();
                                        if let Some(set) =
                                            app.enabled_columns.get_mut(&app.active_tab)
                                        {
                                            if set.contains(&col_str) {
                                                set.remove(&col_str);
                                            } else {
                                                set.insert(col_str);
                                            }
                                            app.update_filter_selection();
                                        }
                                    }
                                } else if idx < group_end {
                                    let group_idx = idx - cols_end;
                                    if let Some(col) = group_cols.get(group_idx) {
                                        if app.group_by_column.as_deref() == Some(col) {
                                            app.group_by_column = None;
                                        } else {
                                            app.group_by_column = Some(col.to_string());
                                        }
                                        app.group_list_state.select(Some(0));
                                        app.update_filter_selection();
                                    }
                                } else if idx < order_end {
                                    app.group_ascending = idx == group_end;
                                    app.update_filter_selection();
                                } else if idx < theme_end {
                                    let theme_idx = idx - order_end;
                                    if let Some(name) = crate::config::THEME_PRESETS.get(theme_idx)
                                    {
                                        crate::config::set_theme_preset(name);
                                        app.config.theme_preset = Some(name.to_string());
                                    }
                                }
                                if let Some(client) = app.gitlab_client.clone() {
                                    app.start_loading_tab(app.active_tab);
                                    spawn_refresh_active_tab(
                                        &client,
                                        &app.project_context,
                                        app.active_tab,
                                        events.sender(),
                                    );
                                }
                            }
                            KeyCode::Enter => {
                                let idx = app.column_checklist_idx;
                                if idx < cols_end {
                                    if let Some(col_name) = cols.get(idx) {
                                        let col_str = col_name.to_string();
                                        let all_values = app
                                            .collect_unique_column_values(app.active_tab, &col_str);
                                        let selected = app
                                            .column_filters
                                            .get(&app.active_tab)
                                            .and_then(|f| f.get(&col_str))
                                            .cloned()
                                            .unwrap_or_default();
                                        app.column_filter_context =
                                            Some((app.active_tab, col_str.clone()));
                                        app.selector = Some(crate::app::Selector {
                                            title: format!("Filter by {}", col_name),
                                            all_items: all_values,
                                            selected_items: selected,
                                            cursor_idx: 0,
                                            search_query: String::new(),
                                            is_filtering: false,
                                            is_loading: false,
                                            entity_iid: 0,
                                            entity_type: String::new(),
                                            field_type: "column_filter".to_string(),
                                            multi_select: true,
                                            state: {
                                                let mut s = ratatui::widgets::ListState::default();
                                                s.select(Some(0));
                                                s
                                            },
                                        });
                                    }
                                } else if idx < group_end {
                                    let group_idx = idx - cols_end;
                                    if let Some(col) = group_cols.get(group_idx) {
                                        if app.group_by_column.as_deref() == Some(col) {
                                            app.group_by_column = None;
                                        } else {
                                            app.group_by_column = Some(col.to_string());
                                        }
                                        app.group_list_state.select(Some(0));
                                        app.update_filter_selection();
                                    }
                                    if let Some(client) = app.gitlab_client.clone() {
                                        app.start_loading_tab(app.active_tab);
                                        spawn_refresh_active_tab(
                                            &client,
                                            &app.project_context,
                                            app.active_tab,
                                            events.sender(),
                                        );
                                    }
                                } else if idx < order_end {
                                    app.group_ascending = idx == group_end;
                                    app.update_filter_selection();
                                    if let Some(client) = app.gitlab_client.clone() {
                                        app.start_loading_tab(app.active_tab);
                                        spawn_refresh_active_tab(
                                            &client,
                                            &app.project_context,
                                            app.active_tab,
                                            events.sender(),
                                        );
                                    }
                                } else if idx < theme_end {
                                    let theme_idx = idx - order_end;
                                    if let Some(name) = crate::config::THEME_PRESETS.get(theme_idx)
                                    {
                                        crate::config::set_theme_preset(name);
                                        app.config.theme_preset = Some(name.to_string());
                                    }
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if app.is_typing_search {
                        match key_event.code {
                            KeyCode::Enter | KeyCode::Esc => app.is_typing_search = false,
                            KeyCode::Backspace => {
                                app.search_query.pop();
                                app.update_filter_selection();
                            }
                            KeyCode::Char(c) => {
                                app.search_query.push(c);
                                app.update_filter_selection();
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if keybinding_matches(&app.config.keybindings.global.search, &key_event)
                        && !app.is_typing_search
                        && app.text_input.is_none()
                        && app.edit_menu.is_none()
                        && app.selector.is_none()
                        && !app.focus_column_checklist
                    {
                        app.is_typing_search = true;
                        continue;
                    }

                    if keybinding_matches(&app.config.keybindings.global.configure, &key_event)
                        && !app.focus_column_checklist
                        && app.text_input.is_none()
                        && app.edit_menu.is_none()
                        && app.selector.is_none()
                    {
                        app.focus_column_checklist = true;
                        app.column_checklist_idx = 0;
                        continue;
                    }

                    if key_event.code == KeyCode::Char(',') && !app.focus_column_checklist {
                        app.focus_column_checklist = true;
                        app.column_checklist_idx = 0;
                        continue;
                    }

                    let mut handled = true;
                    match app.active_tab {
                        app::Tab::Issues => match key_event.code {
                            _ if (key_event.code == KeyCode::Char('n')
                                || keybinding_matches(
                                    &app.config.keybindings.issues.create_issue,
                                    &key_event,
                                )) =>
                            {
                                let is_github = app
                                    .gitlab_client
                                    .as_ref()
                                    .map(|c| c.is_github)
                                    .unwrap_or(false);
                                let mut fields = vec![
                                    ("Title".to_string(), String::new()),
                                    ("Labels".to_string(), String::new()),
                                    ("Assignees".to_string(), String::new()),
                                    ("Milestone".to_string(), String::new()),
                                ];
                                if !is_github {
                                    fields.push(("Confidential".to_string(), "No".to_string()));
                                    fields.push(("Due Date".to_string(), String::new()));
                                    fields.push(("Weight".to_string(), "0".to_string()));
                                }
                                fields.push(("Description".to_string(), String::new()));
                                app.edit_menu = Some(crate::app::EditMenu {
                                    title: "Create Issue".to_string(),
                                    fields,
                                    selected_idx: 0,
                                    entity_iid: 0,
                                    entity_type: "new_issue".to_string(),
                                    state: {
                                        let mut s = ListState::default();
                                        s.select(Some(0));
                                        s
                                    },
                                });
                            }
                            _ if (key_event.code == KeyCode::Char('e')
                                || keybinding_matches(
                                    &app.config.keybindings.issues.edit_entity,
                                    &key_event,
                                )) =>
                            {
                                if let Some(selected_idx) = app.issues.state.selected() {
                                    let filtered = app.filtered_issues();
                                    if let Some(issue) = filtered.get(selected_idx) {
                                        let labels = if issue.labels.is_empty() {
                                            "None".to_string()
                                        } else {
                                            issue.labels.join(", ")
                                        };
                                        let milestone = issue
                                            .milestone
                                            .as_ref()
                                            .map(|m| m.title.clone())
                                            .unwrap_or_else(|| "None".to_string());
                                        let assignees = if issue.assignees.is_empty() {
                                            "None".to_string()
                                        } else {
                                            issue
                                                .assignees
                                                .iter()
                                                .map(|a| format!("@{}", a.username))
                                                .collect::<Vec<_>>()
                                                .join(", ")
                                        };
                                        let is_github = app
                                            .gitlab_client
                                            .as_ref()
                                            .map(|c| c.is_github)
                                            .unwrap_or(false);
                                        let mut fields = vec![
                                            ("Title".to_string(), issue.title.clone()),
                                            ("Labels".to_string(), labels),
                                            ("Assignees".to_string(), assignees),
                                            ("Milestone".to_string(), milestone),
                                        ];
                                        if !is_github {
                                            fields.push((
                                                "Confidential".to_string(),
                                                "Toggle/Set".to_string(),
                                            ));
                                            fields.push((
                                                "Due Date".to_string(),
                                                issue
                                                    .due_date
                                                    .clone()
                                                    .unwrap_or_else(|| "Set".to_string()),
                                            ));
                                            fields.push(("Weight".to_string(), "Set".to_string()));
                                        }
                                        fields.push((
                                            "Description".to_string(),
                                            issue.description.clone().unwrap_or_default(),
                                        ));
                                        app.edit_menu = Some(crate::app::EditMenu {
                                            title: format!("Edit Issue #{}", issue.iid),
                                            fields,
                                            selected_idx: 0,
                                            entity_iid: issue.iid,
                                            entity_type: "issue".to_string(),
                                            state: {
                                                let mut s = ListState::default();
                                                s.select(Some(0));
                                                s
                                            },
                                        });
                                    }
                                }
                            }
                            _ if (key_event.code == KeyCode::Char('c')
                                || keybinding_matches(
                                    &app.config.keybindings.issues.close_entity,
                                    &key_event,
                                )) =>
                            {
                                if let Some(selected_idx) = app.issues.state.selected() {
                                    let filtered = app.filtered_issues();
                                    if let Some(issue) = filtered.get(selected_idx) {
                                        let issue_iid = issue.iid;
                                        let cli = app_cli(&app);
                                        let args = vec![
                                            "issue".to_string(),
                                            "close".to_string(),
                                            issue_iid.to_string(),
                                        ];
                                        run_cli(
                                            &cli,
                                            &args,
                                            &mut terminal,
                                            events.sender(),
                                            app.active_tab,
                                        )
                                        .await;
                                        if let Some(pos) =
                                            app.issues.items.iter().position(|i| i.iid == issue_iid)
                                        {
                                            app.issues.items.remove(pos);
                                        }
                                        app.update_filter_selection();
                                    }
                                }
                            }
                            KeyCode::Char('o') => {
                                if let Some(selected_idx) = app.issues.state.selected() {
                                    if let Some(issue) = app.filtered_issues().get(selected_idx) {
                                        let cli = app_cli(&app);
                                        let args = vec![
                                            "issue".to_string(),
                                            "view".to_string(),
                                            issue.iid.to_string(),
                                            cli.flag_web().to_string(),
                                        ];
                                        run_cli(
                                            &cli,
                                            &args,
                                            &mut terminal,
                                            events.sender(),
                                            app.active_tab,
                                        )
                                        .await;
                                    }
                                }
                            }
                            _ if (key_event.code == KeyCode::Char('r')
                                || keybinding_matches(
                                    &app.config.keybindings.issues.reopen_entity,
                                    &key_event,
                                )) =>
                            {
                                if let Some(selected_idx) = app.issues.state.selected() {
                                    let filtered = app.filtered_issues();
                                    if let Some(issue) = filtered.get(selected_idx) {
                                        let issue_iid = issue.iid;
                                        let cli = app_cli(&app);
                                        let args = vec![
                                            "issue".to_string(),
                                            "reopen".to_string(),
                                            issue_iid.to_string(),
                                        ];
                                        run_cli(
                                            &cli,
                                            &args,
                                            &mut terminal,
                                            events.sender(),
                                            app.active_tab,
                                        )
                                        .await;
                                    }
                                }
                            }
                            _ => handled = false,
                        },
                        app::Tab::MergeRequests => {
                            if key_event.code == KeyCode::Char('n')
                                || keybinding_matches(
                                    &app.config.keybindings.mrs.create_mr,
                                    &key_event,
                                )
                            {
                                let is_github = app
                                    .gitlab_client
                                    .as_ref()
                                    .map(|c| c.is_github)
                                    .unwrap_or(false);
                                let pr_suffix = if is_github {
                                    "Pull Request"
                                } else {
                                    "Merge Request"
                                };

                                let mut all_items = vec!["Create blank (No issue)".to_string()];
                                let is_loading = app.issues.items.is_empty();
                                if !is_loading {
                                    for issue in &app.issues.items {
                                        if issue.state == "opened" || issue.state == "open" {
                                            all_items
                                                .push(format!("#{} {}", issue.iid, issue.title));
                                        }
                                    }
                                }

                                app.selector = Some(crate::app::Selector {
                                    title: format!(" Select Issue to Base {} On ", pr_suffix),
                                    all_items,
                                    selected_items: std::collections::HashSet::new(),
                                    cursor_idx: 0,
                                    search_query: String::new(),
                                    is_filtering: false,
                                    is_loading,
                                    entity_iid: 0,
                                    entity_type: "new_mr_selector".to_string(),
                                    field_type: "create_mr".to_string(),
                                    multi_select: false,
                                    state: {
                                        let mut s = ListState::default();
                                        s.select(Some(0));
                                        s
                                    },
                                });

                                if is_loading {
                                    if let Some(client) = &app.gitlab_client {
                                        spawn_refresh_active_tab(
                                            client,
                                            &app.project_context,
                                            app::Tab::Issues,
                                            events.sender(),
                                        );
                                    }
                                }
                            } else if let Some(selected_idx) = app.mrs.state.selected() {
                                let filtered = app.filtered_mrs();
                                let mr_info = filtered
                                    .get(selected_idx)
                                    .map(|item| (item.iid, item.title.clone()));
                                if let Some((mr_iid, mr_title)) = mr_info {
                                    match key_event.code {
                                        _ if (key_event.code == KeyCode::Char('e')
                                            || keybinding_matches(
                                                &app.config.keybindings.mrs.edit_entity,
                                                &key_event,
                                            )) =>
                                        {
                                            let mr = filtered.get(selected_idx).unwrap();
                                            let labels = if mr.labels.is_empty() {
                                                "None".to_string()
                                            } else {
                                                mr.labels.join(", ")
                                            };
                                            let milestone = mr
                                                .milestone
                                                .as_ref()
                                                .map(|m| m.title.clone())
                                                .unwrap_or_else(|| "None".to_string());
                                            let assignees = if mr.assignees.is_empty() {
                                                "None".to_string()
                                            } else {
                                                mr.assignees
                                                    .iter()
                                                    .map(|a| format!("@{}", a.username))
                                                    .collect::<Vec<_>>()
                                                    .join(", ")
                                            };
                                            let reviewers = if mr.reviewers.is_empty() {
                                                "None".to_string()
                                            } else {
                                                mr.reviewers
                                                    .iter()
                                                    .map(|r| format!("@{}", r.username))
                                                    .collect::<Vec<_>>()
                                                    .join(", ")
                                            };
                                            let draft_status =
                                                if mr.draft { "Draft" } else { "Ready" };
                                            let pr_suffix = if app
                                                .gitlab_client
                                                .as_ref()
                                                .map(|c| c.is_github)
                                                .unwrap_or(false)
                                            {
                                                "PR"
                                            } else {
                                                "MR"
                                            };
                                            app.edit_menu = Some(crate::app::EditMenu {
                                                title: format!("Edit {} #{}", pr_suffix, mr.iid),
                                                fields: vec![
                                                    ("Title".to_string(), mr.title.clone()),
                                                    ("Labels".to_string(), labels),
                                                    ("Assignees".to_string(), assignees),
                                                    ("Reviewers".to_string(), reviewers),
                                                    ("Milestone".to_string(), milestone),
                                                    (
                                                        "Target Branch".to_string(),
                                                        mr.target_branch.clone(),
                                                    ),
                                                    (
                                                        "Status (Draft/Ready)".to_string(),
                                                        draft_status.to_string(),
                                                    ),
                                                    (
                                                        "Description".to_string(),
                                                        mr.description.clone().unwrap_or_default(),
                                                    ),
                                                ],
                                                selected_idx: 0,
                                                entity_iid: mr.iid,
                                                entity_type: "mr".to_string(),
                                                state: {
                                                    let mut s = ListState::default();
                                                    s.select(Some(0));
                                                    s
                                                },
                                            });
                                        }
                                        _ if (key_event.code == KeyCode::Char('a')
                                            || keybinding_matches(
                                                &app.config.keybindings.mrs.approve_mr,
                                                &key_event,
                                            )) =>
                                        {
                                            let cli = app_cli(&app);
                                            let args = if cli.is_github {
                                                vec![
                                                    "pr".to_string(),
                                                    "review".to_string(),
                                                    mr_iid.to_string(),
                                                    "--approve".to_string(),
                                                ]
                                            } else {
                                                vec![
                                                    "mr".to_string(),
                                                    "approve".to_string(),
                                                    mr_iid.to_string(),
                                                ]
                                            };
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                        }
                                        _ if (key_event.code == KeyCode::Char('m')
                                            || keybinding_matches(
                                                &app.config.keybindings.mrs.merge_mr,
                                                &key_event,
                                            )) =>
                                        {
                                            let cli = app_cli(&app);
                                            let args = if cli.is_github {
                                                vec![
                                                    "pr".to_string(),
                                                    "merge".to_string(),
                                                    mr_iid.to_string(),
                                                    "--delete-branch".to_string(),
                                                    "--squash".to_string(),
                                                ]
                                            } else {
                                                vec![
                                                    "mr".to_string(),
                                                    "merge".to_string(),
                                                    mr_iid.to_string(),
                                                    "--remove-source-branch".to_string(),
                                                    "--squash".to_string(),
                                                ]
                                            };
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                            if let Some(pos) =
                                                app.mrs.items.iter().position(|m| m.iid == mr_iid)
                                            {
                                                app.mrs.items.remove(pos);
                                            }
                                            app.update_filter_selection();
                                        }
                                        _ if (key_event.code == KeyCode::Char('v')
                                            || keybinding_matches(
                                                &app.config.keybindings.mrs.view_diff,
                                                &key_event,
                                            )) =>
                                        {
                                            app.diff_loading = true;
                                            let tx = events.sender();
                                            let mr_iid = mr_iid;
                                            let mr_iid_str = mr_iid.to_string();
                                            let client = app.gitlab_client.clone();
                                            let project_context = app.project_context.clone();
                                            tokio::spawn(async move {
                                                let is_github =
                                                    match tokio::process::Command::new("git")
                                                        .args(["remote", "get-url", "origin"])
                                                        .output()
                                                        .await
                                                        .map(|o| {
                                                            String::from_utf8_lossy(&o.stdout)
                                                                .contains("github.com")
                                                        }) {
                                                        Ok(true) => true,
                                                        _ => false,
                                                    };

                                                let program = if is_github { "gh" } else { "glab" };
                                                let (entity, sub) = if is_github {
                                                    ("pr", "diff")
                                                } else {
                                                    ("mr", "diff")
                                                };
                                                let cmd_args = vec![
                                                    entity.to_string(),
                                                    sub.to_string(),
                                                    mr_iid_str.clone(),
                                                ];
                                                let status_msg = format!(
                                                    "Fetching Diff & Comments: {} {}",
                                                    program,
                                                    cmd_args.join(" ")
                                                );
                                                let _ = tx.send(Event::CommandStarted(status_msg));

                                                let mut cmd = tokio::process::Command::new(program);
                                                cmd.args(&cmd_args);

                                                let diff_res = cmd.output().await;

                                                let comments = if let Some(ref c) = client {
                                                    crate::gitlab::mr::list_mr_notes(
                                                        c,
                                                        &project_context,
                                                        mr_iid,
                                                    )
                                                    .await
                                                    .unwrap_or_default()
                                                } else {
                                                    vec![]
                                                };

                                                match diff_res {
                                                    Ok(output) => {
                                                        if output.status.success() {
                                                            let raw_diff = String::from_utf8_lossy(
                                                                &output.stdout,
                                                            )
                                                            .into_owned();
                                                            let _ = tx.send(Event::DiffFetched {
                                                                mr_iid,
                                                                raw_diff,
                                                                comments,
                                                            });
                                                        } else {
                                                            let err_msg = String::from_utf8_lossy(
                                                                &output.stderr,
                                                            );
                                                            let _ = tx.send(
                                                                Event::DiffFetchFailed(format!(
                                                                    "Failed to fetch diff: {}",
                                                                    err_msg
                                                                )),
                                                            );
                                                        }
                                                    }
                                                    Err(_) => {
                                                        let _ = tx.send(Event::DiffFetchFailed("Failed to execute CLI tool to fetch diff".to_string()));
                                                    }
                                                }
                                            });
                                        }
                                        KeyCode::Char('o') => {
                                            let cli = app_cli(&app);
                                            let args = vec![
                                                cli.entity("mr").to_string(),
                                                "view".to_string(),
                                                mr_iid.to_string(),
                                                cli.flag_web().to_string(),
                                            ];
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                        }
                                        _ if (key_event.code == KeyCode::Char('s')
                                            || keybinding_matches(
                                                &app.config.keybindings.mrs.toggle_draft,
                                                &key_event,
                                            )) =>
                                        {
                                            let cli = app_cli(&app);
                                            let is_draft = mr_title.starts_with("Draft:")
                                                || mr_title.starts_with("WIP:");
                                            let action =
                                                if is_draft { "--ready" } else { "--draft" };
                                            let args = UpdateCmd::new(cli.is_github, "mr", mr_iid)
                                                .flag_bool(action)
                                                .build();
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                        }
                                        _ if (key_event.code == KeyCode::Char('c')
                                            || keybinding_matches(
                                                &app.config.keybindings.mrs.close_entity,
                                                &key_event,
                                            )) =>
                                        {
                                            let cli = app_cli(&app);
                                            let args = vec![
                                                cli.entity("mr").to_string(),
                                                "close".to_string(),
                                                mr_iid.to_string(),
                                            ];
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                            if let Some(pos) =
                                                app.mrs.items.iter().position(|m| m.iid == mr_iid)
                                            {
                                                app.mrs.items.remove(pos);
                                            }
                                            app.update_filter_selection();
                                        }
                                        _ if (key_event.code == KeyCode::Char('r')
                                            || keybinding_matches(
                                                &app.config.keybindings.mrs.reopen_entity,
                                                &key_event,
                                            )) =>
                                        {
                                            let cli = app_cli(&app);
                                            let args = vec![
                                                cli.entity("mr").to_string(),
                                                "reopen".to_string(),
                                                mr_iid.to_string(),
                                            ];
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                        }
                                        _ => handled = false,
                                    }
                                } else {
                                    handled = false;
                                }
                            } else {
                                handled = false;
                            }
                        }
                        app::Tab::Pipelines => {
                            if key_event.code == KeyCode::Char('n') {
                                app.edit_menu = Some(crate::app::EditMenu {
                                    title: "Run Pipeline".to_string(),
                                    fields: vec![
                                        (
                                            "Branch / Ref".to_string(),
                                            get_current_branch()
                                                .unwrap_or_else(|| "main".to_string()),
                                        ),
                                        ("Merge Request Pipeline".to_string(), "No".to_string()),
                                        ("Variables".to_string(), String::new()),
                                        ("Inputs".to_string(), String::new()),
                                        ("Workflow / CI File (GitHub)".to_string(), String::new()),
                                    ],
                                    selected_idx: 0,
                                    entity_iid: 0,
                                    entity_type: "new_pipeline".to_string(),
                                    state: {
                                        let mut s = ListState::default();
                                        s.select(Some(0));
                                        s
                                    },
                                });
                            } else if key_event.code == KeyCode::Char('p')
                                || keybinding_matches(
                                    &app.config.keybindings.pipelines.trigger_pipeline,
                                    &key_event,
                                )
                            {
                                let cli = app_cli(&app);
                                let args = if cli.is_github {
                                    vec!["workflow".to_string(), "run".to_string()]
                                } else {
                                    vec!["ci".to_string(), "run".to_string(), "--mr".to_string()]
                                };
                                run_cli(
                                    &cli,
                                    &args,
                                    &mut terminal,
                                    events.sender(),
                                    app.active_tab,
                                )
                                .await;
                            } else if let Some(selected_idx) = app.pipelines.state.selected() {
                                if let Some(item) = app.filtered_pipelines().get(selected_idx) {
                                    let pipe_id = item.id;
                                    match key_event.code {
                                        KeyCode::Char(' ') => {
                                            if app.selected_pipelines.contains(&pipe_id) {
                                                app.selected_pipelines.remove(&pipe_id);
                                            } else {
                                                app.selected_pipelines.insert(pipe_id);
                                            }
                                        }
                                        _ if (key_event.code == KeyCode::Char('r')
                                            || keybinding_matches(
                                                &app.config.keybindings.pipelines.retry,
                                                &key_event,
                                            )) =>
                                        {
                                            if let Some(client) = &app.gitlab_client {
                                                let client_clone = client.clone();
                                                let project_context = app.project_context.clone();
                                                let tx = events.sender();
                                                let active_tab = app.active_tab;
                                                if !app.selected_pipelines.is_empty() {
                                                    let pipe_ids: Vec<u64> = app
                                                        .selected_pipelines
                                                        .iter()
                                                        .cloned()
                                                        .collect();
                                                    for p_id in &pipe_ids {
                                                        if let Some(p) = app
                                                            .pipelines
                                                            .items
                                                            .iter_mut()
                                                            .find(|pipe| pipe.id == *p_id)
                                                        {
                                                            p.status = "running".to_string();
                                                        }
                                                    }
                                                    app.selected_pipelines.clear();
                                                    tokio::spawn(async move {
                                                        for p_id in pipe_ids {
                                                            let endpoint = format!(
                                                                "projects/{}/pipelines/{}/retry",
                                                                project_context.replace("/", "%2F"),
                                                                p_id
                                                            );
                                                            let _ = client_clone
                                                                .fetch_raw_api(&endpoint)
                                                                .await;
                                                        }
                                                        tokio::time::sleep(
                                                            std::time::Duration::from_secs(1),
                                                        )
                                                        .await;
                                                        spawn_refresh_active_tab(
                                                            &client_clone,
                                                            &project_context,
                                                            active_tab,
                                                            tx.clone(),
                                                        );
                                                    });
                                                } else {
                                                    if let Some(p) = app
                                                        .pipelines
                                                        .items
                                                        .iter_mut()
                                                        .find(|pipe| pipe.id == pipe_id)
                                                    {
                                                        p.status = "running".to_string();
                                                    }
                                                    let tx = events.sender();
                                                    tokio::spawn(async move {
                                                        let endpoint = format!(
                                                            "projects/{}/pipelines/{}/retry",
                                                            project_context.replace("/", "%2F"),
                                                            pipe_id
                                                        );
                                                        let _ = client_clone
                                                            .fetch_raw_api(&endpoint)
                                                            .await;
                                                        tokio::time::sleep(
                                                            std::time::Duration::from_secs(1),
                                                        )
                                                        .await;
                                                        spawn_refresh_active_tab(
                                                            &client_clone,
                                                            &project_context,
                                                            active_tab,
                                                            tx,
                                                        );
                                                    });
                                                }
                                            }
                                        }
                                        _ if (key_event.code == KeyCode::Char('d')
                                            || keybinding_matches(
                                                &app.config.keybindings.pipelines.cancel,
                                                &key_event,
                                            )) =>
                                        {
                                            if let Some(p) = app
                                                .pipelines
                                                .items
                                                .iter_mut()
                                                .find(|pipe| pipe.id == pipe_id)
                                            {
                                                p.status = "canceled".to_string();
                                            }
                                            if let Some(client) = &app.gitlab_client {
                                                let client_clone = client.clone();
                                                let project_context = app.project_context.clone();
                                                let tx = events.sender();
                                                let active_tab = app.active_tab;
                                                tokio::spawn(async move {
                                                    let endpoint = format!(
                                                        "projects/{}/pipelines/{}/cancel",
                                                        project_context.replace("/", "%2F"),
                                                        pipe_id
                                                    );
                                                    let _ =
                                                        client_clone.fetch_raw_api(&endpoint).await;
                                                    tokio::time::sleep(
                                                        std::time::Duration::from_secs(1),
                                                    )
                                                    .await;
                                                    spawn_refresh_active_tab(
                                                        &client_clone,
                                                        &project_context,
                                                        active_tab,
                                                        tx,
                                                    );
                                                });
                                            }
                                        }
                                        KeyCode::Char('o') => {
                                            let cli = app_cli(&app);
                                            let (entity, sub) = if cli.is_github {
                                                ("run", "view")
                                            } else {
                                                ("ci", "view")
                                            };
                                            let args = vec![
                                                entity.to_string(),
                                                sub.to_string(),
                                                pipe_id.to_string(),
                                                cli.flag_web().to_string(),
                                            ];
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                        }
                                        _ => handled = false,
                                    }
                                } else {
                                    handled = false;
                                }
                            } else {
                                handled = false;
                            }
                        }
                        app::Tab::Jobs => {
                            if key_event.code == KeyCode::Char('p') {
                                app.text_input = Some(crate::app::TextInput {
                                    title: " Enter Pipeline ID ".to_string(),
                                    value: String::new(),
                                    cursor_idx: 0,
                                    action: crate::app::TextInputAction::EnterPipelineId,
                                });
                            } else if let Some(idx) = app.selected_job_index {
                                let job_info =
                                    app.filtered_jobs().get(idx).map(|j| (j.id, j.name.clone()));
                                if let Some((job_id, job_name)) = job_info {
                                    match key_event.code {
                                        KeyCode::Char(' ') => {
                                            if app.selected_jobs.contains(&job_id) {
                                                app.selected_jobs.remove(&job_id);
                                            } else {
                                                app.selected_jobs.insert(job_id);
                                            }
                                        }
                                        KeyCode::Char('r') => {
                                            if let Some(client) = &app.gitlab_client {
                                                let client_clone = client.clone();
                                                let project_context = app.project_context.clone();
                                                let pipe_id = app.active_pipeline_id.unwrap_or(0);
                                                let tx = events.sender();

                                                if !app.selected_jobs.is_empty() {
                                                    let job_ids: Vec<u64> =
                                                        app.selected_jobs.iter().cloned().collect();
                                                    if let Some(jobs_mut) =
                                                        &mut app.selected_pipeline_jobs
                                                    {
                                                        for j in jobs_mut.iter_mut() {
                                                            if app.selected_jobs.contains(&j.id) {
                                                                j.status = "running".to_string();
                                                            }
                                                        }
                                                    }
                                                    app.selected_jobs.clear();
                                                    tokio::spawn(async move {
                                                        for j_id in job_ids {
                                                            let endpoint = format!(
                                                                "projects/{}/jobs/{}/retry",
                                                                project_context.replace("/", "%2F"),
                                                                j_id
                                                            );
                                                            let _ = client_clone
                                                                .fetch_raw_api(&endpoint)
                                                                .await;
                                                        }
                                                        tokio::time::sleep(
                                                            std::time::Duration::from_secs(1),
                                                        )
                                                        .await;
                                                        if let Ok(jobs) =
                                                            gitlab::pipelines::list_pipeline_jobs(
                                                                &client_clone,
                                                                &project_context,
                                                                pipe_id,
                                                            )
                                                            .await
                                                        {
                                                            let _ = tx.send(Event::PipelineJobs(
                                                                pipe_id, jobs,
                                                            ));
                                                        }
                                                    });
                                                } else {
                                                    if let Some(jobs_mut) =
                                                        &mut app.selected_pipeline_jobs
                                                    {
                                                        if let Some(j) = jobs_mut.get_mut(idx) {
                                                            j.status = "running".to_string();
                                                        }
                                                    }
                                                    tokio::spawn(async move {
                                                        let endpoint = format!(
                                                            "projects/{}/jobs/{}/retry",
                                                            project_context.replace("/", "%2F"),
                                                            job_id
                                                        );
                                                        let _ = client_clone
                                                            .fetch_raw_api(&endpoint)
                                                            .await;
                                                        tokio::time::sleep(
                                                            std::time::Duration::from_secs(1),
                                                        )
                                                        .await;
                                                        if let Ok(jobs) =
                                                            gitlab::pipelines::list_pipeline_jobs(
                                                                &client_clone,
                                                                &project_context,
                                                                pipe_id,
                                                            )
                                                            .await
                                                        {
                                                            let _ = tx.send(Event::PipelineJobs(
                                                                pipe_id, jobs,
                                                            ));
                                                        }
                                                    });
                                                }
                                            }
                                        }
                                        KeyCode::Char('s') => {
                                            if let Some(jobs) = &app.selected_pipeline_jobs {
                                                if let Some(highlighted_job) = jobs.get(idx) {
                                                    let stage_name = &highlighted_job.stage;
                                                    for job in jobs {
                                                        if &job.stage == stage_name {
                                                            app.selected_jobs.insert(job.id);
                                                        }
                                                    }
                                                    app.status_message = Some(format!(
                                                        "Selected all jobs in stage '{}'",
                                                        stage_name
                                                    ));
                                                }
                                            }
                                        }
                                        KeyCode::Char('c') => {
                                            if let Some(client) = &app.gitlab_client {
                                                let client_clone = client.clone();
                                                let project_context = app.project_context.clone();
                                                let pipe_id = app.active_pipeline_id.unwrap_or(0);
                                                let tx = events.sender();

                                                if !app.selected_jobs.is_empty() {
                                                    let job_ids: Vec<u64> =
                                                        app.selected_jobs.iter().cloned().collect();
                                                    if let Some(jobs_mut) =
                                                        &mut app.selected_pipeline_jobs
                                                    {
                                                        for j in jobs_mut.iter_mut() {
                                                            if app.selected_jobs.contains(&j.id) {
                                                                j.status = "canceled".to_string();
                                                            }
                                                        }
                                                    }
                                                    app.selected_jobs.clear();
                                                    tokio::spawn(async move {
                                                        if client_clone.is_github {
                                                            let endpoint = format!(
                                                                "projects/{}/pipelines/{}/cancel",
                                                                project_context.replace("/", "%2F"),
                                                                pipe_id
                                                            );
                                                            let _ = client_clone
                                                                .fetch_raw_api(&endpoint)
                                                                .await;
                                                        } else {
                                                            for j_id in job_ids {
                                                                let endpoint = format!(
                                                                    "projects/{}/jobs/{}/cancel",
                                                                    project_context
                                                                        .replace("/", "%2F"),
                                                                    j_id
                                                                );
                                                                let _ = client_clone
                                                                    .fetch_raw_api(&endpoint)
                                                                    .await;
                                                            }
                                                        }
                                                        tokio::time::sleep(
                                                            std::time::Duration::from_secs(1),
                                                        )
                                                        .await;
                                                        if let Ok(jobs) =
                                                            gitlab::pipelines::list_pipeline_jobs(
                                                                &client_clone,
                                                                &project_context,
                                                                pipe_id,
                                                            )
                                                            .await
                                                        {
                                                            let _ = tx.send(Event::PipelineJobs(
                                                                pipe_id, jobs,
                                                            ));
                                                        }
                                                    });
                                                } else {
                                                    if let Some(jobs_mut) =
                                                        &mut app.selected_pipeline_jobs
                                                    {
                                                        if let Some(j) = jobs_mut.get_mut(idx) {
                                                            j.status = "canceled".to_string();
                                                        }
                                                    }
                                                    tokio::spawn(async move {
                                                        let endpoint = if client_clone.is_github {
                                                            format!(
                                                                "projects/{}/pipelines/{}/cancel",
                                                                project_context.replace("/", "%2F"),
                                                                pipe_id
                                                            )
                                                        } else {
                                                            format!(
                                                                "projects/{}/jobs/{}/cancel",
                                                                project_context.replace("/", "%2F"),
                                                                job_id
                                                            )
                                                        };
                                                        let _ = client_clone
                                                            .fetch_raw_api(&endpoint)
                                                            .await;
                                                        tokio::time::sleep(
                                                            std::time::Duration::from_secs(1),
                                                        )
                                                        .await;
                                                        if let Ok(jobs) =
                                                            gitlab::pipelines::list_pipeline_jobs(
                                                                &client_clone,
                                                                &project_context,
                                                                pipe_id,
                                                            )
                                                            .await
                                                        {
                                                            let _ = tx.send(Event::PipelineJobs(
                                                                pipe_id, jobs,
                                                            ));
                                                        }
                                                    });
                                                }
                                            }
                                        }
                                        KeyCode::Char('d') => {
                                            let cli = app_cli(&app);
                                            let args = if cli.is_github {
                                                vec![
                                                    "run".to_string(),
                                                    "download".to_string(),
                                                    "--pattern".to_string(),
                                                    job_name,
                                                ]
                                            } else {
                                                vec![
                                                    "job".to_string(),
                                                    "artifact".to_string(),
                                                    "master".to_string(),
                                                    job_name,
                                                ]
                                            };
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                        }
                                        KeyCode::Char('o') => {
                                            let cli = app_cli(&app);
                                            let args = if cli.is_github {
                                                if let Some(pipe_id) = app.active_pipeline_id {
                                                    vec![
                                                        "run".to_string(),
                                                        "view".to_string(),
                                                        pipe_id.to_string(),
                                                        cli.flag_web().to_string(),
                                                    ]
                                                } else {
                                                    vec![
                                                        "run".to_string(),
                                                        "view".to_string(),
                                                        job_id.to_string(),
                                                        cli.flag_web().to_string(),
                                                    ]
                                                }
                                            } else {
                                                vec![
                                                    "job".to_string(),
                                                    "view".to_string(),
                                                    job_id.to_string(),
                                                    cli.flag_web().to_string(),
                                                ]
                                            };
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                        }
                                        KeyCode::Char('e') => {
                                            let temp_file = std::env::temp_dir()
                                                .join(format!("job_{}_trace.txt", job_id));
                                            if let Some(trace) = &app.job_trace {
                                                let _ = std::fs::write(&temp_file, trace);
                                            } else if let Some(_) = &app.gitlab_client {
                                                let _ = std::fs::write(
                                                    &temp_file,
                                                    "Trace will be here",
                                                );
                                            }
                                            crate::event::PAUSED
                                                .store(true, std::sync::atomic::Ordering::Relaxed);
                                            disable_raw_mode().unwrap();
                                            execute!(
                                                io::stdout(),
                                                LeaveAlternateScreen,
                                                DisableMouseCapture
                                            )
                                            .unwrap();
                                            let editor = std::env::var("EDITOR")
                                                .or_else(|_| std::env::var("VISUAL"))
                                                .unwrap_or_else(|_| "helix".to_string());
                                            let mut cmd = std::process::Command::new(&editor);
                                            cmd.arg(temp_file.as_os_str());
                                            cmd.stdin(std::process::Stdio::inherit());
                                            cmd.stdout(std::process::Stdio::inherit());
                                            cmd.stderr(std::process::Stdio::inherit());
                                            if let Ok(mut child) = cmd.spawn() {
                                                let _ = child.wait();
                                            }
                                            enable_raw_mode().unwrap();
                                            execute!(
                                                io::stdout(),
                                                EnterAlternateScreen,
                                                EnableMouseCapture
                                            )
                                            .unwrap();
                                            terminal.clear().unwrap();
                                            crate::event::PAUSED
                                                .store(false, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        KeyCode::Enter => {
                                            if app.job_trace.is_some() {
                                                app.details_zoomed = !app.details_zoomed;
                                            } else if let Some(client) = &app.gitlab_client {
                                                let client = client.clone();
                                                let project_context = app.project_context.clone();
                                                let tx = events.sender();
                                                app.job_trace_loading = true;
                                                tokio::spawn(async move {
                                                    let res = gitlab::pipelines::get_job_trace(
                                                        &client,
                                                        &project_context,
                                                        job_id,
                                                    )
                                                    .await;
                                                    let _ = tx.send(Event::JobTraceFetched(
                                                        job_id,
                                                        res.map_err(|e| e.to_string()),
                                                    ));
                                                });
                                            }
                                        }
                                        _ => handled = false,
                                    }
                                } else {
                                    handled = false;
                                }
                            } else {
                                handled = false;
                            }
                        }
                        app::Tab::Runners => {
                            if let Some(selected_idx) = app.runners.state.selected() {
                                if let Some(item) = app.filtered_runners().get(selected_idx) {
                                    let runner_id = item.id;
                                    match key_event.code {
                                        KeyCode::Char('p') => {
                                            let cli = app_cli(&app);
                                            let args: Vec<String> = vec![
                                                "api".into(),
                                                "-X".into(),
                                                "PUT".into(),
                                                format!("runners/{}", runner_id),
                                                "-f".into(),
                                                "paused=true".into(),
                                            ];
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                            if let Some(runner) = app
                                                .runners
                                                .items
                                                .iter_mut()
                                                .find(|r| r.id == runner_id)
                                            {
                                                runner.status = "paused".to_string();
                                                runner.active = false;
                                            }
                                        }
                                        KeyCode::Char('r') => {
                                            let cli = app_cli(&app);
                                            let args: Vec<String> = vec![
                                                "api".into(),
                                                "-X".into(),
                                                "PUT".into(),
                                                format!("runners/{}", runner_id),
                                                "-f".into(),
                                                "paused=false".into(),
                                            ];
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                            if let Some(runner) = app
                                                .runners
                                                .items
                                                .iter_mut()
                                                .find(|r| r.id == runner_id)
                                            {
                                                runner.status = "online".to_string();
                                                runner.active = true;
                                            }
                                        }
                                        KeyCode::Char('e') => {
                                            let current_desc =
                                                item.description.clone().unwrap_or_default();
                                            app.text_input = Some(crate::app::TextInput {
                                                title: " Edit Runner Description ".to_string(),
                                                cursor_idx: current_desc.len(),
                                                value: current_desc,
                                                action: crate::app::TextInputAction::EditField {
                                                    entity_iid: runner_id,
                                                    entity_type: "runner".to_string(),
                                                    field_type: "runner_description".to_string(),
                                                },
                                            });
                                        }
                                        _ => handled = false,
                                    }
                                } else {
                                    handled = false;
                                }
                            } else {
                                handled = false;
                            }
                        }
                        app::Tab::Releases => match key_event.code {
                            _ if (key_event.code == KeyCode::Char('n')
                                || keybinding_matches(
                                    &app.config.keybindings.releases.create_release,
                                    &key_event,
                                )) =>
                            {
                                app.edit_menu = Some(crate::app::EditMenu {
                                    title: "Create Release".to_string(),
                                    fields: vec![
                                        ("Tag".to_string(), String::new()),
                                        ("Release Name".to_string(), String::new()),
                                        ("Description".to_string(), String::new()),
                                    ],
                                    selected_idx: 0,
                                    entity_iid: 0,
                                    entity_type: "new_release".to_string(),
                                    state: {
                                        let mut s = ListState::default();
                                        s.select(Some(0));
                                        s
                                    },
                                });
                            }
                            _ if (key_event.code == KeyCode::Char('e')
                                || keybinding_matches(
                                    &app.config.keybindings.releases.edit_release,
                                    &key_event,
                                )) =>
                            {
                                if let Some(selected_idx) = app.releases.state.selected() {
                                    let release_tag = {
                                        let filtered = app.filtered_releases();
                                        filtered.get(selected_idx).map(|r| r.tag_name.clone())
                                    };
                                    if let Some(tag_name) = release_tag {
                                        if let Some(idx) = app
                                            .releases
                                            .items
                                            .iter()
                                            .position(|r| r.tag_name == tag_name)
                                        {
                                            rebuild_edit_menu(&mut app, "release", idx as u64);
                                        }
                                    }
                                }
                            }
                            _ if (key_event.code == KeyCode::Char('d')
                                || keybinding_matches(
                                    &app.config.keybindings.releases.delete_release,
                                    &key_event,
                                )) =>
                            {
                                if let Some(selected_idx) = app.releases.state.selected() {
                                    let filtered = app.filtered_releases();
                                    if let Some(release) = filtered.get(selected_idx) {
                                        app.confirm_popup =
                                            Some(crate::app::ConfirmAction::DeleteRelease(
                                                release.tag_name.clone(),
                                            ));
                                    }
                                }
                            }
                            _ if (key_event.code == KeyCode::Char('o')
                                || keybinding_matches(
                                    &app.config.keybindings.releases.open_in_browser,
                                    &key_event,
                                )) =>
                            {
                                if let Some(selected_idx) = app.releases.state.selected() {
                                    let filtered = app.filtered_releases();
                                    if let Some(release) = filtered.get(selected_idx) {
                                        let cli = app_cli(&app);
                                        let args = vec![
                                            "release".to_string(),
                                            "view".to_string(),
                                            release.tag_name.clone(),
                                            cli.flag_web().to_string(),
                                        ];
                                        run_cli(
                                            &cli,
                                            &args,
                                            &mut terminal,
                                            events.sender(),
                                            app.active_tab,
                                        )
                                        .await;
                                    }
                                }
                            }
                            _ => handled = false,
                        },
                        app::Tab::Todos => {
                            if let Some(selected_idx) = app.todos.state.selected() {
                                if let Some(item) = app.filtered_todos().get(selected_idx) {
                                    match key_event.code {
                                        KeyCode::Enter => {
                                            let n_id = item.id.clone();
                                            let target_iid = item.target_iid;
                                            let target_type = item.target_type.clone();
                                            let client_opt = app.gitlab_client.clone();
                                            if let Some(client) = client_opt {
                                                tokio::spawn(async move {
                                                    let _ = gitlab::notifications::mark_notification_as_read(&client, &n_id).await;
                                                });
                                            }
                                            app.active_tab = match target_type.as_str() {
                                                "MergeRequest" => app::Tab::MergeRequests,
                                                _ => app::Tab::Issues,
                                            };
                                            app.update_filter_selection();
                                            match app.active_tab {
                                                app::Tab::Issues => {
                                                    if let Some(pos) = app
                                                        .issues
                                                        .items
                                                        .iter()
                                                        .position(|i| i.iid == target_iid)
                                                    {
                                                        app.issues.state.select(Some(pos));
                                                    }
                                                }
                                                app::Tab::MergeRequests => {
                                                    if let Some(pos) = app
                                                        .mrs
                                                        .items
                                                        .iter()
                                                        .position(|m| m.iid == target_iid)
                                                    {
                                                        app.mrs.state.select(Some(pos));
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                        KeyCode::Char('o') => {
                                            let cli = app_cli(&app);
                                            let entity =
                                                if item.target_type.contains("MergeRequest") {
                                                    cli.entity("mr")
                                                } else {
                                                    "issue"
                                                };
                                            let args = vec![
                                                entity.to_string(),
                                                "view".to_string(),
                                                item.target_iid.to_string(),
                                                cli.flag_web().to_string(),
                                            ];
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                        }
                                        _ => handled = false,
                                    }
                                } else {
                                    handled = false;
                                }
                            } else {
                                handled = false;
                            }
                        }
                        app::Tab::Milestones => {
                            match key_event.code {
                                _ if (key_event.code == KeyCode::Char('n')
                                    || keybinding_matches(
                                        &app.config.keybindings.milestones.create_milestone,
                                        &key_event,
                                    )) =>
                                {
                                    let is_github = app
                                        .gitlab_client
                                        .as_ref()
                                        .map(|c| c.is_github)
                                        .unwrap_or(false);
                                    let mut fields = vec![
                                        ("Title".to_string(), String::new()),
                                        ("Description".to_string(), String::new()),
                                    ];
                                    if !is_github {
                                        fields.push(("Start Date".to_string(), String::new()));
                                    }
                                    fields.push(("Due Date".to_string(), String::new()));
                                    app.edit_menu = Some(crate::app::EditMenu {
                                        title: "Create Milestone".to_string(),
                                        fields,
                                        selected_idx: 0,
                                        entity_iid: 0,
                                        entity_type: "new_milestone".to_string(),
                                        state: {
                                            let mut s = ListState::default();
                                            s.select(Some(0));
                                            s
                                        },
                                    });
                                }
                                _ if (key_event.code == KeyCode::Char('e')
                                    || keybinding_matches(
                                        &app.config.keybindings.milestones.edit_milestone,
                                        &key_event,
                                    )) =>
                                {
                                    if let Some(selected_idx) = app.milestones.state.selected() {
                                        let milestone_iid = {
                                            let filtered = app.filtered_milestones();
                                            filtered.get(selected_idx).map(|m| m.iid)
                                        };
                                        if let Some(iid) = milestone_iid {
                                            rebuild_edit_menu(&mut app, "milestone", iid);
                                        }
                                    }
                                }
                                _ if (key_event.code == KeyCode::Char('c')
                                    || keybinding_matches(
                                        &app.config.keybindings.milestones.close_milestone,
                                        &key_event,
                                    )) =>
                                {
                                    if let Some(selected_idx) = app.milestones.state.selected() {
                                        let filtered = app.filtered_milestones();
                                        if let Some(milestone) = filtered.get(selected_idx) {
                                            let client = app.gitlab_client.clone().unwrap();
                                            let project_path = app.project_context.clone();
                                            let milestone_iid = milestone.iid;
                                            let tx = events.sender();
                                            let _ = tx.send(Event::CommandStarted(format!(
                                                "Closing milestone #{}",
                                                milestone_iid
                                            )));
                                            tokio::spawn(async move {
                                                let res = crate::gitlab::milestones::update_milestone_state(
                                                &client,
                                                &project_path,
                                                milestone_iid,
                                                true,
                                            ).await;
                                                match res {
                                                    Ok(_) => {
                                                        let _ = tx.send(Event::MilestoneClosed);
                                                    }
                                                    Err(e) => {
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            crate::app::Tab::Milestones,
                                                            Err(e.to_string()),
                                                        ));
                                                    }
                                                }
                                            });
                                        }
                                    }
                                }
                                _ if (key_event.code == KeyCode::Char('r')
                                    || keybinding_matches(
                                        &app.config.keybindings.milestones.reopen_milestone,
                                        &key_event,
                                    )) =>
                                {
                                    if let Some(selected_idx) = app.milestones.state.selected() {
                                        let filtered = app.filtered_milestones();
                                        if let Some(milestone) = filtered.get(selected_idx) {
                                            let client = app.gitlab_client.clone().unwrap();
                                            let project_path = app.project_context.clone();
                                            let milestone_iid = milestone.iid;
                                            let tx = events.sender();
                                            let _ = tx.send(Event::CommandStarted(format!(
                                                "Reopening milestone #{}",
                                                milestone_iid
                                            )));
                                            tokio::spawn(async move {
                                                let res = crate::gitlab::milestones::update_milestone_state(
                                                &client,
                                                &project_path,
                                                milestone_iid,
                                                false,
                                            ).await;
                                                match res {
                                                    Ok(_) => {
                                                        let _ = tx.send(Event::MilestoneReopened);
                                                    }
                                                    Err(e) => {
                                                        let _ = tx.send(Event::CommandCompleted(
                                                            crate::app::Tab::Milestones,
                                                            Err(e.to_string()),
                                                        ));
                                                    }
                                                }
                                            });
                                        }
                                    }
                                }
                                _ if (key_event.code == KeyCode::Char('d')
                                    || keybinding_matches(
                                        &app.config.keybindings.milestones.delete_milestone,
                                        &key_event,
                                    )) =>
                                {
                                    if let Some(selected_idx) = app.milestones.state.selected() {
                                        let filtered = app.filtered_milestones();
                                        if let Some(milestone) = filtered.get(selected_idx) {
                                            app.confirm_popup =
                                                Some(crate::app::ConfirmAction::DeleteMilestone(
                                                    milestone.iid,
                                                ));
                                        }
                                    }
                                }
                                _ if (key_event.code == KeyCode::Char('o')
                                    || keybinding_matches(
                                        &app.config.keybindings.milestones.open_in_browser,
                                        &key_event,
                                    )) =>
                                {
                                    if let Some(selected_idx) = app.milestones.state.selected() {
                                        let filtered = app.filtered_milestones();
                                        if let Some(milestone) = filtered.get(selected_idx) {
                                            let is_github = app
                                                .gitlab_client
                                                .as_ref()
                                                .map(|c| c.is_github)
                                                .unwrap_or(false);
                                            let cli = app_cli(&app);
                                            let args = if is_github {
                                                vec![
                                                    "browse".to_string(),
                                                    format!("milestone/{}", milestone.iid),
                                                ]
                                            } else {
                                                vec![
                                                    "milestone".to_string(),
                                                    "view".to_string(),
                                                    milestone.iid.to_string(),
                                                    cli.flag_web().to_string(),
                                                ]
                                            };
                                            run_cli(
                                                &cli,
                                                &args,
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                        }
                                    }
                                }
                                _ => handled = false,
                            }
                        }
                        app::Tab::Terminal => {
                            handled = false;
                        }
                    }

                    if !handled {
                        if keybinding_matches(
                            &app.config.keybindings.global.scroll_down,
                            &key_event,
                        ) {
                            match app.active_tab {
                                app::Tab::Issues => {
                                    app.issues_scroll = app.issues_scroll.saturating_add(1);
                                }
                                app::Tab::MergeRequests => {
                                    app.mrs_scroll = app.mrs_scroll.saturating_add(1);
                                }
                                _ => {}
                            }
                        } else if keybinding_matches(
                            &app.config.keybindings.global.scroll_up,
                            &key_event,
                        ) {
                            match app.active_tab {
                                app::Tab::Issues => {
                                    app.issues_scroll = app.issues_scroll.saturating_sub(1);
                                }
                                app::Tab::MergeRequests => {
                                    app.mrs_scroll = app.mrs_scroll.saturating_sub(1);
                                }
                                _ => {}
                            }
                        }

                        match key_event.code {
                            KeyCode::Char('?') | KeyCode::F(1) => {
                                app.show_help = true;
                            }
                            KeyCode::Char('u') => {
                                app.error_message = Some("Checking for updates...".to_string());
                                let tx = events.sender();
                                tokio::spawn(async move {
                                    match crate::utils::update::perform_self_update().await {
                                        Ok(true) => {
                                            let _ = tx.send(Event::FetchFailed(
                                                app::Tab::Todos,
                                                "Update complete! Please restart glab-tui."
                                                    .to_string(),
                                            ));
                                        }
                                        Ok(false) => {
                                            let _ = tx.send(Event::FetchFailed(
                                                app::Tab::Todos,
                                                "Already up to date.".to_string(),
                                            ));
                                        }
                                        Err(e) => {
                                            let _ = tx.send(Event::FetchFailed(
                                                app::Tab::Todos,
                                                format!("Update failed: {}", e),
                                            ));
                                        }
                                    }
                                });
                            }
                            KeyCode::Char('q') => {
                                if app.details_zoomed {
                                    app.details_zoomed = false;
                                } else {
                                    app.quit();
                                }
                            }
                            KeyCode::Char('m') | KeyCode::Char('M')
                                if app.active_tab == app::Tab::Jobs && app.job_trace.is_none() =>
                            {
                                app.collapse_matrix_jobs = !app.collapse_matrix_jobs;
                                app.selected_job_index = Some(0);
                                app.jobs_list_state.select(Some(0));
                            }

                            KeyCode::Esc | KeyCode::Backspace => {
                                if app.job_trace_loading {
                                    app.job_trace_loading = false;
                                } else if app.details_zoomed {
                                    app.details_zoomed = false;
                                    app.job_trace = None;
                                } else if app.active_tab == app::Tab::Jobs {
                                    if app.job_trace.is_some() {
                                        app.job_trace = None;
                                    } else {
                                        app.active_tab = app::Tab::Pipelines;
                                    }
                                } else if app.active_tab == app::Tab::Pipelines
                                    && app.selected_pipeline_jobs.is_some()
                                {
                                    if app.job_trace.is_some() {
                                        app.job_trace = None;
                                    } else {
                                        app.selected_pipeline_jobs = None;
                                        app.selected_job_index = None;
                                        app.selected_jobs.clear();
                                    }
                                }
                            }
                            KeyCode::Char('f') => {
                                app.is_typing_search = true;
                            }
                            KeyCode::Enter => match app.active_tab {
                                app::Tab::Todos => {
                                    if let Some(idx) = app.todos.state.selected() {
                                        if let Some(n) = app.filtered_todos().get(idx) {
                                            let n_id = n.id.clone();
                                            let target_iid = n.target_iid;
                                            let target_type = n.target_type.clone();
                                            let client_opt = app.gitlab_client.clone();
                                            if let Some(client) = client_opt {
                                                tokio::spawn(async move {
                                                    let _ = gitlab::notifications::mark_notification_as_read(&client, &n_id).await;
                                                });
                                            }
                                            app.active_tab = match target_type.as_str() {
                                                "MergeRequest" => app::Tab::MergeRequests,
                                                _ => app::Tab::Issues,
                                            };
                                            app.update_filter_selection();
                                            match app.active_tab {
                                                app::Tab::Issues => {
                                                    if let Some(pos) = app
                                                        .issues
                                                        .items
                                                        .iter()
                                                        .position(|i| i.iid == target_iid)
                                                    {
                                                        app.issues.state.select(Some(pos));
                                                    }
                                                }
                                                app::Tab::MergeRequests => {
                                                    if let Some(pos) = app
                                                        .mrs
                                                        .items
                                                        .iter()
                                                        .position(|m| m.iid == target_iid)
                                                    {
                                                        app.mrs.state.select(Some(pos));
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                                app::Tab::Pipelines => {
                                    if let Some(idx) = app.pipelines.state.selected() {
                                        let pipe_id =
                                            app.filtered_pipelines().get(idx).map(|p| p.id);
                                        if let Some(pipeline_id) = pipe_id {
                                            if let Some(client) = &app.gitlab_client {
                                                app.loading_tabs.insert(app::Tab::Jobs);
                                                if let Ok(jobs) =
                                                    gitlab::pipelines::list_pipeline_jobs(
                                                        client,
                                                        &app.project_context,
                                                        pipeline_id,
                                                    )
                                                    .await
                                                {
                                                    app.pipeline_jobs
                                                        .insert(pipeline_id, jobs.clone());
                                                    app.selected_pipeline_jobs = Some(jobs);
                                                    app.active_pipeline_id = Some(pipeline_id);
                                                    app.selected_job_index = Some(0);
                                                    app.jobs_list_state.select(Some(0));
                                                    app.job_trace_scroll = 0;
                                                    app.job_trace = None;
                                                    app.active_tab = app::Tab::Jobs;
                                                    app.loading_tabs.remove(&app::Tab::Jobs);
                                                } else {
                                                    app.error_message =
                                                        Some("Failed to fetch jobs".to_string());
                                                    app.loading_tabs.remove(&app::Tab::Jobs);
                                                }
                                            }
                                        }
                                    }
                                }
                                app::Tab::Jobs => {
                                    if app.job_trace.is_some() {
                                        app.details_zoomed = !app.details_zoomed;
                                    } else if let Some(idx) = app.selected_job_index {
                                        let job_info = app
                                            .filtered_jobs()
                                            .get(idx)
                                            .map(|j| (j.id, j.name.clone()));
                                        if let Some((job_id, _)) = job_info {
                                            if let Some(client) = &app.gitlab_client {
                                                let client = client.clone();
                                                let project_context = app.project_context.clone();
                                                let tx = events.sender();
                                                app.job_trace_loading = true;
                                                tokio::spawn(async move {
                                                    let res = gitlab::pipelines::get_job_trace(
                                                        &client,
                                                        &project_context,
                                                        job_id,
                                                    )
                                                    .await;
                                                    let _ = tx.send(Event::JobTraceFetched(
                                                        job_id,
                                                        res.map_err(|e| e.to_string()),
                                                    ));
                                                });
                                            }
                                        }
                                    }
                                }
                                _ => {
                                    app.details_zoomed = !app.details_zoomed;
                                }
                            },
                            _ if (key_event.code == KeyCode::Right
                                || key_event.code == KeyCode::Char('l')
                                || keybinding_matches(
                                    &app.config.keybindings.global.next_tab,
                                    &key_event,
                                )) =>
                            {
                                app.next_tab();
                                if let Some(client) = &app.gitlab_client {
                                    if !app.loading_tabs.contains(&app.active_tab)
                                        && !app.refreshed_tabs.contains(&app.active_tab)
                                    {
                                        if !app.loaded_tabs.contains(&app.active_tab) {
                                            app.loading_tabs.insert(app.active_tab);
                                        }
                                        spawn_refresh_active_tab(
                                            client,
                                            &app.project_context,
                                            app.active_tab,
                                            events.sender(),
                                        );
                                    }
                                }
                            }
                            _ if (key_event.code == KeyCode::Left
                                || key_event.code == KeyCode::Char('h')
                                || keybinding_matches(
                                    &app.config.keybindings.global.prev_tab,
                                    &key_event,
                                )) =>
                            {
                                app.previous_tab();
                                if let Some(client) = &app.gitlab_client {
                                    if !app.loading_tabs.contains(&app.active_tab)
                                        && !app.refreshed_tabs.contains(&app.active_tab)
                                    {
                                        if !app.loaded_tabs.contains(&app.active_tab) {
                                            app.loading_tabs.insert(app.active_tab);
                                        }
                                        spawn_refresh_active_tab(
                                            client,
                                            &app.project_context,
                                            app.active_tab,
                                            events.sender(),
                                        );
                                    }
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => match app.active_tab {
                                app::Tab::Issues => {
                                    app.issues.next(app.filtered_issues().len());
                                    app.issues_scroll = 0;
                                }
                                app::Tab::MergeRequests => {
                                    app.mrs.next(app.filtered_mrs().len());
                                    app.mrs_scroll = 0;
                                }
                                app::Tab::Pipelines => {
                                    app.pipelines.next(app.filtered_pipelines().len());
                                }
                                app::Tab::Jobs => {
                                    if app.job_trace.is_some() {
                                        app.job_trace_scroll =
                                            app.job_trace_scroll.saturating_add(1);
                                    } else {
                                        let len = app.filtered_jobs().len();
                                        if let Some(idx) = &mut app.selected_job_index {
                                            if len > 0 && *idx + 1 < len {
                                                *idx += 1;
                                                app.jobs_list_state.select(Some(*idx));
                                                app.job_trace = None;
                                            }
                                        }
                                    }
                                }
                                app::Tab::Runners => app.runners.next(app.filtered_runners().len()),
                                app::Tab::Releases => {
                                    app.releases.next(app.filtered_releases().len())
                                }
                                app::Tab::Todos => app.todos.next(app.filtered_todos().len()),
                                app::Tab::Milestones => {
                                    app.milestones.next(app.filtered_milestones().len())
                                }
                                app::Tab::Terminal => {
                                    app.terminal_scroll = app.terminal_scroll.saturating_sub(1);
                                }
                            },
                            KeyCode::Up | KeyCode::Char('k') => match app.active_tab {
                                app::Tab::Issues => {
                                    app.issues.previous(app.filtered_issues().len());
                                    app.issues_scroll = 0;
                                }
                                app::Tab::MergeRequests => {
                                    app.mrs.previous(app.filtered_mrs().len());
                                    app.mrs_scroll = 0;
                                }
                                app::Tab::Pipelines => {
                                    app.pipelines.previous(app.filtered_pipelines().len());
                                }
                                app::Tab::Jobs => {
                                    if app.job_trace.is_some() {
                                        app.job_trace_scroll =
                                            app.job_trace_scroll.saturating_sub(1);
                                    } else {
                                        if let Some(idx) = &mut app.selected_job_index {
                                            if *idx > 0 {
                                                *idx -= 1;
                                                app.jobs_list_state.select(Some(*idx));
                                                app.job_trace = None;
                                            }
                                        }
                                    }
                                }
                                app::Tab::Runners => {
                                    app.runners.previous(app.filtered_runners().len())
                                }
                                app::Tab::Releases => {
                                    app.releases.previous(app.filtered_releases().len())
                                }
                                app::Tab::Todos => app.todos.next(app.filtered_todos().len()),
                                app::Tab::Milestones => {
                                    app.milestones.previous(app.filtered_milestones().len())
                                }
                                app::Tab::Terminal => {
                                    app.terminal_scroll = app.terminal_scroll.saturating_add(1);
                                }
                            },
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_value_pairs() {
        let input = "key1:val1,key2:val2, replicas:int(3), debug:bool(false) ";
        let pairs = parse_key_value_pairs(input);
        assert_eq!(
            pairs,
            vec![
                ("key1".to_string(), "val1".to_string()),
                ("key2".to_string(), "val2".to_string()),
                ("replicas".to_string(), "int(3)".to_string()),
                ("debug".to_string(), "bool(false)".to_string())
            ]
        );
    }
}
