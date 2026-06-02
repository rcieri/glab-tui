mod app;
mod event;
mod ui;
mod gitlab;
pub mod utils;

use anyhow::Result;
use app::App;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use event::{Event, EventHandler};
use ratatui::{backend::CrosstermBackend, Terminal, widgets::ListState};
use std::io;

type AppTerminal = Terminal<CrosstermBackend<std::io::Stdout>>;

fn edit_in_editor(current_val: &str, terminal: &mut AppTerminal) -> Option<String> {
    // Choose editor
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "helix".to_string());

    // Create a unique temporary file
    let mut tmp = match tempfile::NamedTempFile::new() {
        Ok(f) => f,
        Err(_) => return None,
    };
    // Write current description (or empty) to file
    if std::io::Write::write_all(&mut tmp, current_val.as_bytes()).is_err() {
        return None;
    }

    // Suspend TUI
    crate::event::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
    std::thread::sleep(std::time::Duration::from_millis(50));
    crossterm::terminal::disable_raw_mode().ok()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
    )
    .ok()?;

    // Launch external editor
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("cmd");
        c.args(&["/c", &format!("{} \"{}\"", editor, tmp.path().to_string_lossy())]);
        c
    } else {
        let mut c = std::process::Command::new(&editor);
        c.arg(tmp.path());
        c
    };
    cmd.stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    if let Ok(mut child) = cmd.spawn() {
        child.wait().ok()?;
    }

    // Resume TUI
    crossterm::terminal::enable_raw_mode().ok()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
    )
    .ok()?;
    while crossterm::event::poll(std::time::Duration::from_secs(0)).unwrap_or(false) {
        let _ = crossterm::event::read();
    }
    let _ = terminal.clear();
    crate::event::PAUSED.store(false, std::sync::atomic::Ordering::Relaxed);

    // Read edited content
    let content = match std::fs::read_to_string(tmp.path()) {
        Ok(c) => c,
        Err(_) => return None,
    };
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

// old edit_in_editor implementation removed


async fn apply_field_text_change(
    app: &mut App,
    entity_type: &str,
    iid: u64,
    field_type: &str,
    value: String,
    terminal: &mut AppTerminal,
) {
    match field_type {
        "title" => {
            run_glab_update(entity_type, iid, &["--title", &value], terminal).await;
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
                run_glab_update(entity_type, iid, &["--target-branch", &value], terminal).await;
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.target_branch = value;
                }
            }
        }
        "due_date" => {
            if entity_type == "issue" {
                if value == "YYYY-MM-DD" || value.trim().is_empty() {
                    run_glab_update(entity_type, iid, &["--due-date", ""], terminal).await;
                } else {
                    run_glab_update(entity_type, iid, &["--due-date", &value], terminal).await;
                }
            }
        }
        "weight" => {
            if entity_type == "issue" {
                run_glab_update(entity_type, iid, &["--weight", &value], terminal).await;
            }
        }
        "runner_description" => {
            run_glab_cmd(&["api", "-X", "PUT", &format!("runners/{}", iid), "-f", &format!("description={}", value)], terminal).await;
            if let Some(runner) = app.runners.items.iter_mut().find(|r| r.id == iid) {
                runner.description = Some(value);
            }
        }
        _ => {}
    }
}

fn translate_glab_to_gh(args: &[&str]) -> Vec<String> {
    if args.is_empty() {
        return vec![];
    }
    let mut gh_args = vec![];
    let cmd = args[0];
    match cmd {
        "issue" => {
            gh_args.push("issue".to_string());
            if args.len() > 1 && args[1] == "create" {
                gh_args.push("create".to_string());
                let mut title = None;
                for i in 2..args.len() {
                    if args[i] == "--title" && i + 1 < args.len() {
                        title = Some(args[i+1]);
                    }
                }
                if let Some(t) = title {
                    gh_args.push("--title".to_string());
                    gh_args.push(t.to_string());
                }
                gh_args.push("--body".to_string());
                gh_args.push("".to_string());
            } else if args.len() > 1 && args[1] == "update" {
                gh_args.push("edit".to_string());
                if args.len() > 2 {
                    gh_args.push(args[2].to_string());
                }
                let mut i = 3;
                while i < args.len() {
                    match args[i] {
                        "--title" => {
                            if i + 1 < args.len() {
                                gh_args.push("--title".to_string());
                                gh_args.push(args[i+1].to_string());
                                i += 2;
                            } else { i += 1; }
                        }
                        "--label" => {
                            if i + 1 < args.len() {
                                gh_args.push("--label".to_string());
                                gh_args.push(args[i+1].to_string());
                                i += 2;
                            } else { i += 1; }
                        }
                        "--unlabel" => {
                            if i + 1 < args.len() && args[i+1] == "all" {
                                if i + 2 < args.len() && args[i+2] == "--label" {
                                    // skip
                                } else {
                                    gh_args.push("--label".to_string());
                                    gh_args.push("".to_string());
                                }
                                i += 2;
                            } else { i += 1; }
                        }
                        "--assignee" => {
                            if i + 1 < args.len() {
                                gh_args.push("--assignee".to_string());
                                gh_args.push(args[i+1].to_string());
                                i += 2;
                            } else { i += 1; }
                        }
                        "--unassign" => {
                            gh_args.push("--assignee".to_string());
                            gh_args.push("".to_string());
                            i += 1;
                        }
                        "--milestone" => {
                            if i + 1 < args.len() {
                                gh_args.push("--milestone".to_string());
                                let ms = if args[i+1] == "0" { "" } else { args[i+1] };
                                gh_args.push(ms.to_string());
                                i += 2;
                            } else { i += 1; }
                        }
                        "-d" => {
                            if i + 1 < args.len() && args[i+1] == "-" {
                                gh_args.push("--body-file".to_string());
                                gh_args.push("-".to_string());
                                i += 2;
                            } else { i += 1; }
                        }
                        _ => { i += 1; }
                    }
                }
            } else {
                for arg in &args[1..] {
                    gh_args.push(arg.to_string());
                }
            }
        }
        "mr" => {
            gh_args.push("pr".to_string());
            if args.len() > 1 {
                match args[1] {
                    "create" => {
                        gh_args.push("create".to_string());
                        let mut issue_id = None;
                        let mut i = 2;
                        while i < args.len() {
                            if (args[i] == "-i" || args[i] == "--related-issue") && i + 1 < args.len() {
                                issue_id = Some(args[i+1]);
                                i += 2;
                            } else {
                                i += 1;
                            }
                        }
                        gh_args.push("--fill".to_string());
                        if let Some(id) = issue_id {
                            gh_args.push("--body".to_string());
                            gh_args.push(format!("Resolves #{}", id));
                        }
                    }
                    "update" => {
                        gh_args.push("edit".to_string());
                        if args.len() > 2 {
                            gh_args.push(args[2].to_string());
                        }
                        let mut i = 3;
                        while i < args.len() {
                            match args[i] {
                                "--title" => {
                                    if i + 1 < args.len() {
                                        gh_args.push("--title".to_string());
                                        gh_args.push(args[i+1].to_string());
                                        i += 2;
                                    } else { i += 1; }
                                }
                                "--label" => {
                                    if i + 1 < args.len() {
                                        gh_args.push("--label".to_string());
                                        gh_args.push(args[i+1].to_string());
                                        i += 2;
                                    } else { i += 1; }
                                }
                                "--unlabel" => {
                                    if i + 1 < args.len() && args[i+1] == "all" {
                                        if i + 2 < args.len() && args[i+2] == "--label" {
                                            // skip
                                        } else {
                                            gh_args.push("--label".to_string());
                                            gh_args.push("".to_string());
                                        }
                                        i += 2;
                                    } else { i += 1; }
                                }
                                "--assignee" => {
                                    if i + 1 < args.len() {
                                        gh_args.push("--assignee".to_string());
                                        gh_args.push(args[i+1].to_string());
                                        i += 2;
                                    } else { i += 1; }
                                }
                                "--unassign" => {
                                    gh_args.push("--assignee".to_string());
                                    gh_args.push("".to_string());
                                    i += 1;
                                }
                                "--reviewer" => {
                                    if i + 1 < args.len() {
                                        gh_args.push("--reviewer".to_string());
                                        gh_args.push(args[i+1].to_string());
                                        i += 2;
                                    } else { i += 1; }
                                }
                                "--milestone" => {
                                    if i + 1 < args.len() {
                                        gh_args.push("--milestone".to_string());
                                        let ms = if args[i+1] == "0" { "" } else { args[i+1] };
                                        gh_args.push(ms.to_string());
                                        i += 2;
                                    } else { i += 1; }
                                }
                                "--target-branch" => {
                                    if i + 1 < args.len() {
                                        gh_args.push("--base".to_string());
                                        gh_args.push(args[i+1].to_string());
                                        i += 2;
                                    } else { i += 1; }
                                }
                                "-d" => {
                                    if i + 1 < args.len() && args[i+1] == "-" {
                                        gh_args.push("--body-file".to_string());
                                        gh_args.push("-".to_string());
                                        i += 2;
                                    } else { i += 1; }
                                }
                                _ => { i += 1; }
                            }
                        }
                    }
                    "approve" => {
                        gh_args.push("review".to_string());
                        if args.len() > 2 {
                            gh_args.push(args[2].to_string());
                        }
                        gh_args.push("--approve".to_string());
                    }
                    "merge" => {
                        gh_args.push("merge".to_string());
                        if args.len() > 2 {
                            gh_args.push(args[2].to_string());
                        }
                        gh_args.push("--delete-branch".to_string());
                        gh_args.push("--squash".to_string());
                    }
                    "diff" => {
                        gh_args.push("diff".to_string());
                        if args.len() > 2 {
                            gh_args.push(args[2].to_string());
                        }
                    }
                    "view" => {
                        gh_args.push("view".to_string());
                        if args.len() > 2 {
                            gh_args.push(args[2].to_string());
                        }
                        gh_args.push("--web".to_string());
                    }
                    _ => {
                        for arg in &args[1..] {
                            gh_args.push(arg.to_string());
                        }
                    }
                }
            }
        }
        "ci" => {
            gh_args.push("run".to_string());
            if args.len() > 1 && args[1] == "view" {
                gh_args.push("view".to_string());
                if args.len() > 2 {
                    gh_args.push(args[2].to_string());
                }
                gh_args.push("--web".to_string());
            } else {
                for arg in &args[1..] {
                    gh_args.push(arg.to_string());
                }
            }
        }
        "job" => {
            gh_args.push("run".to_string());
            if args.len() > 1 {
                if args[1] == "view" {
                    gh_args.push("view".to_string());
                    if args.len() > 2 {
                        gh_args.push(args[2].to_string());
                    }
                    gh_args.push("--web".to_string());
                } else if args[1] == "artifact" {
                    gh_args.push("download".to_string());
                }
            }
        }
        "release" => {
            gh_args.push("release".to_string());
            if args.len() > 1 && args[1] == "view" {
                gh_args.push("view".to_string());
                if args.len() > 2 {
                    gh_args.push(args[2].to_string());
                }
                gh_args.push("--web".to_string());
            } else {
                for arg in &args[1..] {
                    gh_args.push(arg.to_string());
                }
            }
        }
        _ => {
            for arg in args {
                gh_args.push(arg.to_string());
            }
        }
    }
    gh_args
}

async fn run_glab_cmd(args: &[&str], terminal: &mut AppTerminal) {
    crate::event::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    disable_raw_mode().unwrap();
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture).unwrap();
    
    let is_github = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("github.com"))
        .unwrap_or(false);

    let mut cmd = if is_github {
        let gh_args = translate_glab_to_gh(args);
        let mut c = std::process::Command::new("gh");
        for arg in gh_args {
            c.arg(arg);
        }
        c
    } else {
        let mut c = std::process::Command::new("glab");
        for arg in args {
            c.arg(arg);
        }
        c
    };

    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());
    
    if let Ok(mut child) = cmd.spawn() {
        let _ = child.wait();
    }
    
    enable_raw_mode().unwrap();
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture).unwrap();
    while crossterm::event::poll(std::time::Duration::from_secs(0)).unwrap_or(false) {
        let _ = crossterm::event::read();
    }
    let _ = terminal.clear();
    crate::event::PAUSED.store(false, std::sync::atomic::Ordering::Relaxed);
}

async fn run_glab_update(entity_type: &str, id: u64, args: &[&str], terminal: &mut AppTerminal) {
    let id_str = id.to_string();
    let mut cmd_args = vec![entity_type, "update", &id_str];
    cmd_args.extend_from_slice(args);
    run_glab_cmd(&cmd_args, terminal).await;
}

async fn apply_selector_changes(
    app: &mut App,
    entity_type: &str,
    iid: u64,
    field_type: &str,
    values: Vec<String>,
    terminal: &mut AppTerminal,
) {
    match field_type {
        "labels" => {
            let labels_comma = values.join(",");
            if labels_comma.is_empty() {
                run_glab_update(entity_type, iid, &["--unlabel", "all"], terminal).await;
            } else {
                run_glab_update(entity_type, iid, &["--unlabel", "all", "--label", &labels_comma], terminal).await;
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
            let clean_values: Vec<String> = values.iter().map(|v| v.trim_start_matches('@').to_string()).collect();
            let assignees_comma = clean_values.join(",");
            
            if assignees_comma.is_empty() {
                run_glab_update(entity_type, iid, &["--unassign"], terminal).await;
            } else {
                run_glab_update(entity_type, iid, &["--assignee", &assignees_comma], terminal).await;
            }
            
            let new_assignees: Vec<crate::gitlab::issues::Assignee> = clean_values.iter().map(|u| {
                crate::gitlab::issues::Assignee { username: u.clone() }
            }).collect();
            
            if entity_type == "issue" {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.assignees = new_assignees;
                }
            } else if entity_type == "mr" {
                let mr_assignees: Vec<crate::gitlab::mr::Assignee> = new_assignees.iter().map(|a| {
                    crate::gitlab::mr::Assignee { username: a.username.clone() }
                }).collect();
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.assignees = mr_assignees;
                }
            }
        }
        "reviewers" => {
            if entity_type == "mr" {
                let clean_values: Vec<String> = values.iter().map(|v| v.trim_start_matches('@').to_string()).collect();
                let reviewers_comma = clean_values.join(",");
                
                run_glab_update(entity_type, iid, &["--reviewer", &reviewers_comma], terminal).await;
                
                let new_reviewers: Vec<crate::gitlab::mr::Reviewer> = clean_values.into_iter().map(|u| {
                    crate::gitlab::mr::Reviewer { username: u }
                }).collect();
                
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.reviewers = new_reviewers;
                }
            }
        }
        "milestone" => {
            if let Some(milestone_title) = values.first() {
                run_glab_update(entity_type, iid, &["--milestone", milestone_title], terminal).await;
                
                let new_milestone = Some(crate::gitlab::issues::Milestone { title: milestone_title.clone() });
                if entity_type == "issue" {
                    if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                        item.milestone = new_milestone;
                    }
                } else if entity_type == "mr" {
                    let mr_milestone = Some(crate::gitlab::mr::Milestone { title: milestone_title.clone() });
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.milestone = mr_milestone;
                    }
                }
            } else {
                run_glab_update(entity_type, iid, &["--milestone", "0"], terminal).await;
                
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
                if val.to_lowercase() == "confidential" {
                    run_glab_update(entity_type, iid, &["--confidential"], terminal).await;
                } else {
                    run_glab_update(entity_type, iid, &["--public"], terminal).await;
                }
            }
        }
        "draft_status" => {
            if let Some(val) = values.first() {
                let action = if val.to_lowercase() == "draft" { "--draft" } else { "--ready" };
                run_glab_update(entity_type, iid, &[action], terminal).await;
                if entity_type == "mr" {
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.draft = val.to_lowercase() == "draft";
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
            let labels = if issue.labels.is_empty() { "None".to_string() } else { issue.labels.join(", ") };
            let milestone = issue.milestone.as_ref().map(|m| m.title.clone()).unwrap_or_else(|| "None".to_string());
            let assignees = if issue.assignees.is_empty() {
                "None".to_string()
            } else {
                issue.assignees.iter().map(|a| format!("@{}", a.username)).collect::<Vec<_>>().join(", ")
            };
            
            let selected_idx = app.edit_menu.as_ref().map(|m| m.selected_idx).unwrap_or(0);
            
            app.edit_menu = Some(crate::app::EditMenu {
                title: format!("Edit Issue #{}", issue.iid),
                fields: vec![
                    ("Title".to_string(), issue.title.clone()),
                    ("Labels".to_string(), labels),
                    ("Assignees".to_string(), assignees),
                    ("Milestone".to_string(), milestone),
                    ("Confidential".to_string(), "Toggle/Set".to_string()),
                    ("Due Date".to_string(), "Set".to_string()),
                    ("Weight".to_string(), "Set".to_string()),
                                    ("Description".to_string(), issue.description.clone().unwrap_or_default()),
                ],
                selected_idx,
                entity_iid: issue.iid,
                entity_type: "issue".to_string(),
                state: { let mut s = ListState::default(); s.select(Some(selected_idx)); s },
            });
        }
    } else if entity_type == "mr" {
        if let Some(mr) = app.mrs.items.iter().find(|m| m.iid == entity_iid) {
            let labels = if mr.labels.is_empty() { "None".to_string() } else { mr.labels.join(", ") };
            let milestone = mr.milestone.as_ref().map(|m| m.title.clone()).unwrap_or_else(|| "None".to_string());
            let assignees = if mr.assignees.is_empty() {
                "None".to_string()
            } else {
                mr.assignees.iter().map(|a| format!("@{}", a.username)).collect::<Vec<_>>().join(", ")
            };
            let reviewers = if mr.reviewers.is_empty() {
                "None".to_string()
            } else {
                mr.reviewers.iter().map(|r| format!("@{}", r.username)).collect::<Vec<_>>().join(", ")
            };
            let draft_status = if mr.draft { "Draft" } else { "Ready" };
            
            let selected_idx = app.edit_menu.as_ref().map(|m| m.selected_idx).unwrap_or(0);

            let pr_suffix = if app.gitlab_client.as_ref().map(|c| c.is_github).unwrap_or(false) { "PR" } else { "MR" };
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
                    ("Description".to_string(), mr.description.clone().unwrap_or_default()),
                ],
                selected_idx,
                entity_iid: mr.iid,
                entity_type: "mr".to_string(),
                state: { let mut s = ListState::default(); s.select(Some(selected_idx)); s },
            });
        }
    }
}

async fn handle_entity_update(app: &mut App, entity_type: &str, iid: u64, code: KeyCode, terminal: &mut AppTerminal) {
    match code {
        KeyCode::Char('t') => {
            let current_title = if entity_type == "issue" {
                app.issues.items.iter().find(|i| i.iid == iid).map(|i| i.title.clone()).unwrap_or_default()
            } else {
                app.mrs.items.iter().find(|m| m.iid == iid).map(|m| m.title.clone()).unwrap_or_default()
            };

            if let Some(new_title) = edit_in_editor(&current_title, terminal) {
                run_glab_update(entity_type, iid, &["--title", &new_title], terminal).await;
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
                let is_draft = app.mrs.items.iter().find(|m| m.iid == iid).map(|m| m.draft).unwrap_or(false);
                let action = if is_draft { "--ready" } else { "--draft" };
                run_glab_update(entity_type, iid, &[action], terminal).await;
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.draft = !is_draft;
                }
            }
        }
        KeyCode::Char('g') => {
            if entity_type == "mr" {
                let current_branch = app.mrs.items.iter().find(|m| m.iid == iid).map(|m| m.target_branch.clone()).unwrap_or_default();
                if let Some(target) = edit_in_editor(&current_branch, terminal) {
                    run_glab_update(entity_type, iid, &["--target-branch", &target], terminal).await;
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.target_branch = target;
                    }
                }
            }
        }
        KeyCode::Char('c') => {
            if entity_type == "issue" {
                if let Some(res) = edit_in_editor("public", terminal) {
                    if res.to_lowercase().contains("confidential") {
                        run_glab_update(entity_type, iid, &["--confidential"], terminal).await;
                    } else {
                        run_glab_update(entity_type, iid, &["--public"], terminal).await;
                    }
                }
            }
        }
        KeyCode::Char('u') => {
            if entity_type == "issue" {
                if let Some(due_date) = edit_in_editor("YYYY-MM-DD", terminal) {
                    if due_date == "YYYY-MM-DD" || due_date.is_empty() {
                        run_glab_update(entity_type, iid, &["--due-date", ""], terminal).await;
                    } else {
                        run_glab_update(entity_type, iid, &["--due-date", &due_date], terminal).await;
                    }
                }
            }
        }
        KeyCode::Char('w') => {
            if entity_type == "issue" {
                if let Some(weight) = edit_in_editor("0", terminal) {
                    run_glab_update(entity_type, iid, &["--weight", &weight], terminal).await;
                }
            }
        }
        KeyCode::Char('d') => {
            run_glab_update(entity_type, iid, &["-d", "-"], terminal).await;
            if let Some(client) = &app.gitlab_client {
                if entity_type == "issue" {
                    if let Ok(updated) = gitlab::issues::get_issue(client, &app.project_context, iid).await {
                        if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                            *item = updated;
                        }
                    }
                } else if entity_type == "mr" {
                    if let Ok(updated) = gitlab::mr::get_mr(client, &app.project_context, iid).await {
                        if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                            *item = updated;
                        }
                    }
                }
            }
        }
        _ => {}
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
                match gitlab::issues::list_issues(&client, &project_context).await {
                    Ok(issues) => { let _ = tx.send(Event::IssuesFetched(issues)); }
                    Err(e) => { let _ = tx.send(Event::FetchFailed(tab, format!("Failed to fetch issues: {}", e))); }
                }
            }
            app::Tab::MergeRequests => {
                match gitlab::mr::list_mrs(&client, &project_context).await {
                    Ok(mrs) => { let _ = tx.send(Event::MrsFetched(mrs)); }
                    Err(e) => { let _ = tx.send(Event::FetchFailed(tab, format!("Failed to fetch MRs: {}", e))); }
                }
            }
            app::Tab::Pipelines => {
                match gitlab::pipelines::list_pipelines(&client, &project_context).await {
                    Ok(pipelines) => { let _ = tx.send(Event::PipelinesFetched(pipelines)); }
                    Err(e) => { let _ = tx.send(Event::FetchFailed(tab, format!("Failed to fetch pipelines: {}", e))); }
                }
            }
            app::Tab::Runners => {
                match gitlab::runners::list_runners(&client, &project_context).await {
                    Ok(runners) => { let _ = tx.send(Event::RunnersFetched(runners)); }
                    Err(e) => { let _ = tx.send(Event::FetchFailed(tab, format!("Failed to fetch runners: {}", e))); }
                }
            }
            app::Tab::Releases => {
                match gitlab::releases::list_releases(&client, &project_context).await {
                    Ok(releases) => { let _ = tx.send(Event::ReleasesFetched(releases)); }
                    Err(e) => { let _ = tx.send(Event::FetchFailed(tab, format!("Failed to fetch releases: {}", e))); }
                }
            }
        }
    });
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and event handler
    let mut app = App::new();
    let mut events = EventHandler::new(250);

    // Initialize gitlab context
    if let Ok(context) = gitlab::client::get_project_context().await {
        app.project_context = context;
    }

    if let Ok(client) = gitlab::client::GitlabClient::new().await {
        app.gitlab_client = Some(client.clone());
        let tx = events.sender();
        app.loading_tabs.insert(app.active_tab);
        spawn_refresh_active_tab(&client, &app.project_context, app.active_tab, tx.clone());
    } else {
        app.error_message = Some("Failed to initialize GitLab client".to_string());
    }

    // Run app
    while app.running {
        if app.active_tab == app::Tab::Pipelines {
            if let Some(client) = &app.gitlab_client {
                if let Some(idx) = app.pipelines.state.selected() {
                    let pipe_id = app.filtered_pipelines().get(idx).map(|p| p.id);
                    if let Some(pipe_id) = pipe_id {
                        if !app.pipeline_jobs.contains_key(&pipe_id) && !app.fetching_pipelines.contains(&pipe_id) {
                            app.fetching_pipelines.insert(pipe_id);
                            let client_clone = client.clone();
                            let project_context = app.project_context.clone();
                            let tx = events.sender();
                            tokio::spawn(async move {
                                if let Ok(jobs) = gitlab::pipelines::list_pipeline_jobs(&client_clone, &project_context, pipe_id).await {
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

        terminal.draw(|f| ui::render(f, &mut app))?;

        if let Some(event) = events.next().await {
            match event {
                Event::Tick => app.tick(),
                Event::PipelineJobs(id, jobs) => {
                    app.fetching_pipelines.remove(&id);
                    app.pipeline_jobs.insert(id, jobs.clone());
                    let match_id = if let Some(idx) = app.pipelines.state.selected() {
                        app.filtered_pipelines().get(idx).map(|p| p.id) == Some(id)
                    } else {
                        false
                    };
                    if match_id && app.selected_pipeline_jobs.is_some() {
                        app.selected_pipeline_jobs = Some(jobs);
                        app.jobs_list_state.select(app.selected_job_index.or(Some(0)));
                    }
                }
                Event::IssuesFetched(issues) => {
                    app.loading_tabs.remove(&app::Tab::Issues);
                    app.loaded_tabs.insert(app::Tab::Issues);
                    app.issues.items = issues;
                    app.update_filter_selection();
                }
                Event::MrsFetched(mrs) => {
                    app.loading_tabs.remove(&app::Tab::MergeRequests);
                    app.loaded_tabs.insert(app::Tab::MergeRequests);
                    app.mrs.items = mrs;
                    app.update_filter_selection();
                }
                Event::PipelinesFetched(pipelines) => {
                    app.loading_tabs.remove(&app::Tab::Pipelines);
                    app.loaded_tabs.insert(app::Tab::Pipelines);
                    app.pipelines.items = pipelines;
                    app.update_filter_selection();
                    app.pipeline_jobs.clear();
                    app.fetching_pipelines.clear();
                }
                Event::RunnersFetched(runners) => {
                    app.loading_tabs.remove(&app::Tab::Runners);
                    app.loaded_tabs.insert(app::Tab::Runners);
                    app.runners.items = runners;
                    app.update_filter_selection();
                }
                Event::ReleasesFetched(releases) => {
                    app.loading_tabs.remove(&app::Tab::Releases);
                    app.loaded_tabs.insert(app::Tab::Releases);
                    app.releases.items = releases;
                    app.update_filter_selection();
                }
                Event::SelectorItemsFetched(items) => {
                    if let Some(mut selector) = app.selector.take() {
                        selector.all_items = items;
                        selector.is_loading = false;
                        app.selector = Some(selector);
                    }
                }
                Event::FetchFailed(tab, err_msg) => {
                    app.loading_tabs.remove(&tab);
                    app.error_message = Some(err_msg);
                }
                Event::DiffFetched(mr_iid, raw_diff) => {
                    app.diff_loading = false;
                    app.diff_view = Some(crate::app::DiffView::new(mr_iid, raw_diff));
                }
                Event::DiffFetchFailed(err_msg) => {
                    app.diff_loading = false;
                    app.error_message = Some(err_msg);
                }
                Event::Key(key_event) => {
                    if app.error_message.is_some() {
                        if key_event.code == KeyCode::Enter || key_event.code == KeyCode::Esc {
                            app.error_message = None;
                        }
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

                    let is_refresh = key_event.code == KeyCode::F(5) ||
                        (key_event.code == KeyCode::Char('r') && key_event.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)) ||
                        (key_event.code == KeyCode::Char('R') && key_event.modifiers.contains(crossterm::event::KeyModifiers::CONTROL));

                    if is_refresh && app.text_input.is_none() && app.edit_menu.is_none() && app.selector.is_none() {
                        if let Some(client) = &app.gitlab_client {
                            if !app.loading_tabs.contains(&app.active_tab) {
                                app.loading_tabs.insert(app.active_tab);
                                spawn_refresh_active_tab(client, &app.project_context, app.active_tab, events.sender());
                            }
                        }
                        continue;
                    }

                    if let Some(mut text_input) = app.text_input.take() {
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
                                    crate::app::TextInputAction::EditField { entity_iid, entity_type, field_type } => {
                                        apply_field_text_change(&mut app, &entity_type, entity_iid, &field_type, value, &mut terminal).await;
                                        if let Some(client) = &app.gitlab_client {
                                            spawn_refresh_active_tab(client, &app.project_context, app.active_tab, events.sender());
                                        }
                                        rebuild_edit_menu(&mut app, &entity_type, entity_iid);
                                    }
                                    crate::app::TextInputAction::CreateIssue => {
                                        if !value.trim().is_empty() {
                                            run_glab_cmd(&["issue", "create", "-y", "--title", &value], &mut terminal).await;
                                            if let Some(client) = &app.gitlab_client {
                                                spawn_refresh_active_tab(client, &app.project_context, app.active_tab, events.sender());
                                            }
                                        }
                                    }
                                    crate::app::TextInputAction::AddReviewComment { mr_iid, file_path, line_num, old_line_num } => {
                                        if !value.trim().is_empty() {
                                            let mut args = vec![
                                                "mr".to_string(),
                                                "note".to_string(),
                                                "create".to_string(),
                                                mr_iid.to_string(),
                                                "--file".to_string(),
                                                file_path,
                                                "-m".to_string(),
                                                value,
                                            ];
                                            if let Some(line) = line_num {
                                                args.push("--line".to_string());
                                                args.push(line.to_string());
                                            } else if let Some(old_line) = old_line_num {
                                                args.push("--old-line".to_string());
                                                args.push(old_line.to_string());
                                            }
                                            let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                                            run_glab_cmd(&args_ref, &mut terminal).await;
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
                                    selector.is_filtering = true;
                                    app.selector = Some(selector);
                                }
                                KeyCode::Char('j') | KeyCode::Down => {
                                    if !filtered_items.is_empty() {
                                        selector.cursor_idx = (selector.cursor_idx + 1) % filtered_items.len();
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
                                            let clean_val = selector.search_query.trim().to_string();
                                            if !clean_val.is_empty() {
                                                if selector.multi_select {
                                                    if selector.selected_items.contains(&clean_val) {
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
                                    if field_type == "create_mr" {
                                        let filtered_items = selector.get_filtered_items();
                                        let mut selected_val = selector.selected_items.iter().next().cloned();
                                        if selected_val.is_none() && !filtered_items.is_empty() {
                                            selected_val = Some(filtered_items[selector.cursor_idx].clone());
                                        }
                                        
                                        if let Some(item) = selected_val {
                                            let mut id_val = item.clone();
                                            if id_val.starts_with("+ Create \"") {
                                                id_val = selector.search_query.trim().to_string();
                                            }
                                            
                                            let parsed_iid = if id_val.starts_with('#') {
                                                id_val.strip_prefix('#')
                                                    .and_then(|s| s.split(':').next())
                                                    .and_then(|s| s.trim().parse::<u64>().ok())
                                            } else {
                                                id_val.trim().parse::<u64>().ok()
                                            };

                                            if let Some(issue_iid) = parsed_iid {
                                                app.selector = None;
                                                run_glab_cmd(&["mr", "create", "-i", &issue_iid.to_string(), "--copy-issue-labels", "--create-source-branch", "--squash-before-merge"], &mut terminal).await;
                                                if let Some(client) = &app.gitlab_client {
                                                    app.loading_tabs.insert(app.active_tab);
                                                    spawn_refresh_active_tab(client, &app.project_context, app.active_tab, events.sender());
                                                }
                                            }
                                        }
                                        continue;
                                    }
                                    
                                    let entity_type = selector.entity_type.clone();
                                    let entity_iid = selector.entity_iid;
                                    let selected_list: Vec<String> = selector.selected_items.iter().cloned().collect();
                                    
                                    apply_selector_changes(&mut app, &entity_type, entity_iid, &field_type, selected_list, &mut terminal).await;
                                    
                                    if let Some(client) = &app.gitlab_client {
                                        spawn_refresh_active_tab(client, &app.project_context, app.active_tab, events.sender());
                                    }
                                    
                                    rebuild_edit_menu(&mut app, &entity_type, entity_iid);
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
                                menu.selected_idx = (menu.selected_idx + 1) % menu.fields.len();
                                menu.state.select(Some(menu.selected_idx));
                                app.edit_menu = Some(menu);
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                if menu.selected_idx == 0 {
                                    menu.selected_idx = menu.fields.len() - 1;
                                } else {
                                    menu.selected_idx -= 1;
                                }
                                menu.state.select(Some(menu.selected_idx));
                                app.edit_menu = Some(menu);
                            }
                            KeyCode::Enter => {
                                let field_name = menu.fields[menu.selected_idx].0.clone();
                                let entity_iid = menu.entity_iid;
                                let entity_type = menu.entity_type.clone();
                                
                                if field_name == "Labels" || field_name == "Assignees" || field_name == "Reviewers" || field_name == "Milestone" || field_name == "Confidential" || field_name == "Status (Draft/Ready)" {
                                    let mut current_set = std::collections::HashSet::new();
                                    let field_type = match field_name.as_str() {
                                        "Labels" => "labels",
                                        "Assignees" => "assignees",
                                        "Reviewers" => "reviewers",
                                        "Milestone" => "milestone",
                                        "Confidential" => "confidential",
                                        "Status (Draft/Ready)" => "draft_status",
                                        _ => "",
                                    };
                                    let multi_select = match field_type {
                                        "labels" | "assignees" | "reviewers" => true,
                                        _ => false,
                                    };

                                    let mut all_items = Vec::new();
                                    let mut is_loading = true;

                                    if field_type == "confidential" {
                                        all_items = vec!["Public".to_string(), "Confidential".to_string()];
                                        is_loading = false;
                                        // Default Confidential representation in model is not explicitly boolean, so start empty
                                    } else if field_type == "draft_status" {
                                        all_items = vec!["Draft".to_string(), "Ready".to_string()];
                                        is_loading = false;
                                        if let Some(mr) = app.mrs.items.iter().find(|m| m.iid == entity_iid) {
                                            current_set.insert(if mr.draft { "Draft".to_string() } else { "Ready".to_string() });
                                        }
                                    } else if entity_type == "issue" {
                                        if let Some(issue) = app.issues.items.iter().find(|i| i.iid == entity_iid) {
                                            match field_type {
                                                "labels" => {
                                                    for l in &issue.labels {
                                                        current_set.insert(l.clone());
                                                    }
                                                }
                                                "assignees" => {
                                                    for a in &issue.assignees {
                                                        current_set.insert(format!("@{}", a.username));
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
                                        if let Some(mr) = app.mrs.items.iter().find(|m| m.iid == entity_iid) {
                                            match field_type {
                                                "labels" => {
                                                    for l in &mr.labels {
                                                        current_set.insert(l.clone());
                                                    }
                                                }
                                                "assignees" => {
                                                    for a in &mr.assignees {
                                                        current_set.insert(format!("@{}", a.username));
                                                    }
                                                }
                                                "reviewers" => {
                                                    for r in &mr.reviewers {
                                                        current_set.insert(format!("@{}", r.username));
                                                    }
                                                }
                                                "milestone" => {
                                                    if let Some(m) = &mr.milestone {
                                                        current_set.insert(m.title.clone());
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                    }

                                    app.selector = Some(crate::app::Selector {
                                        title: format!("Select {}", field_name),
                                        all_items,
                                        selected_items: current_set,
                                        cursor_idx: 0,
                                        search_query: String::new(),
                                        is_filtering: false,
                                        is_loading,
                                        entity_iid,
                                        entity_type: entity_type.clone(),
                                        field_type: field_type.to_string(),
                                        multi_select,
                                        state: { let mut s = ListState::default(); s.select(Some(0)); s },
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
                                                    "labels" => client.fetch_labels(&project_context).await,
                                                    "assignees" | "reviewers" => client.fetch_members(&project_context).await,
                                                    "milestone" => client.fetch_milestones(&project_context).await,
                                                    _ => Ok(Vec::new()),
                                                };
                                                if let Ok(items) = res {
                                                    let _ = tx.send(Event::SelectorItemsFetched(items));
                                                } else {
                                                    let _ = tx.send(Event::SelectorItemsFetched(Vec::new()));
                                                }
                                            });
                                        }
                                    }
                                    continue;
                                }

                                if field_name == "Title" || field_name == "Target Branch" || field_name == "Due Date" || field_name == "Weight" {
                                    let field_type = match field_name.as_str() {
                                        "Title" => "title",
                                        "Target Branch" => "target_branch",
                                        "Due Date" => "due_date",
                                        "Weight" => "weight",
                                        _ => "",
                                    };
                                    let current_val = match field_type {
                                        "title" => {
                                            if entity_type == "issue" {
                                                app.issues.items.iter().find(|i| i.iid == entity_iid).map(|i| i.title.clone()).unwrap_or_default()
                                            } else {
                                                app.mrs.items.iter().find(|m| m.iid == entity_iid).map(|m| m.title.clone()).unwrap_or_default()
                                            }
                                        }
                                        "target_branch" => {
                                            app.mrs.items.iter().find(|m| m.iid == entity_iid).map(|m| m.target_branch.clone()).unwrap_or_default()
                                        }
                                        "due_date" => "".to_string(),
                                        "weight" => "0".to_string(),
                                        _ => String::new(),
                                    };

                                    app.text_input = Some(crate::app::TextInput {
                                        title: format!("Edit {}", field_name),
                                        cursor_idx: current_val.len(),
                                        value: current_val,
                                        action: crate::app::TextInputAction::EditField {
                                            entity_iid,
                                            entity_type: entity_type.clone(),
                                            field_type: field_type.to_string(),
                                        },
                                    });

                                    app.edit_menu = Some(menu);
                                    continue;
                                }

                                if field_name == "Description" {
                                    handle_entity_update(&mut app, &entity_type, entity_iid, KeyCode::Char('d'), &mut terminal).await;
                                    if let Some(client) = &app.gitlab_client {
                                        spawn_refresh_active_tab(client, &app.project_context, app.active_tab, events.sender());
                                    }
                                    rebuild_edit_menu(&mut app, &entity_type, entity_iid);
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
                        match key_event.code {
                            KeyCode::Esc | KeyCode::Char('q') => {
                                app.diff_view = None;
                            }
                            KeyCode::Tab => {
                                diff_view.focus_on_files = !diff_view.focus_on_files;
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('h') | KeyCode::Left => {
                                if diff_view.focus_on_files {
                                    if !diff_view.visible_nodes.is_empty() {
                                        let node = &diff_view.visible_nodes[diff_view.selected_visible_idx];
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
                                        let node = &diff_view.visible_nodes[diff_view.selected_visible_idx];
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
                                        let node = &diff_view.visible_nodes[diff_view.selected_visible_idx];
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
                            KeyCode::Char('j') | KeyCode::Down => {
                                if diff_view.focus_on_files {
                                    if !diff_view.visible_nodes.is_empty() {
                                        let old_idx = diff_view.selected_visible_idx;
                                        diff_view.selected_visible_idx = (diff_view.selected_visible_idx + 1).min(diff_view.visible_nodes.len() - 1);
                                        if diff_view.selected_visible_idx != old_idx {
                                            diff_view.cursor_idx = 0;
                                            diff_view.scroll_offset = 0;
                                            diff_view.update_active_lines();
                                        }
                                    }
                                } else {
                                    if !diff_view.lines.is_empty() {
                                        diff_view.cursor_idx = (diff_view.cursor_idx + 1).min(diff_view.lines.len() - 1);
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
                                        diff_view.cursor_idx -= 1;
                                        diff_view.update_selected_file_from_cursor();
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('J') => {
                                if !diff_view.focus_on_files {
                                    if let Some(&next_hunk) = diff_view.hunks.iter().find(|&&idx| idx > diff_view.cursor_idx) {
                                        diff_view.cursor_idx = next_hunk;
                                        diff_view.update_selected_file_from_cursor();
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('K') => {
                                if !diff_view.focus_on_files {
                                    if let Some(&prev_hunk) = diff_view.hunks.iter().rev().find(|&&idx| idx < diff_view.cursor_idx) {
                                        diff_view.cursor_idx = prev_hunk;
                                        diff_view.update_selected_file_from_cursor();
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('c') => {
                                if let Some(line) = diff_view.lines.get(diff_view.cursor_idx) {
                                    let can_comment = match line.line_type {
                                        crate::app::DiffLineType::Addition | crate::app::DiffLineType::Deletion | crate::app::DiffLineType::Normal => true,
                                        _ => false,
                                    };
                                    if can_comment {
                                        app.text_input = Some(crate::app::TextInput {
                                            title: format!(" Add Comment to {} ", line.file_path),
                                            value: String::new(),
                                            cursor_idx: 0,
                                            action: crate::app::TextInputAction::AddReviewComment {
                                                mr_iid: diff_view.mr_iid,
                                                file_path: line.file_path.clone(),
                                                line_num: line.new_line_num,
                                                old_line_num: line.old_line_num,
                                            },
                                        });
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            _ => {
                                app.diff_view = Some(diff_view);
                            }
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

                    let mut handled = true;
                    match app.active_tab {
                        app::Tab::Issues => {
                            match key_event.code {
                                KeyCode::Char('n') => {
                                    app.text_input = Some(crate::app::TextInput {
                                        title: " Create New Issue Title ".to_string(),
                                        value: String::new(),
                                        cursor_idx: 0,
                                        action: crate::app::TextInputAction::CreateIssue,
                                    });
                                }
                                KeyCode::Char('e') => {
                                    if let Some(selected_idx) = app.issues.state.selected() {
                                        let filtered = app.filtered_issues();
                                        if let Some(issue) = filtered.get(selected_idx) {
                                            let labels = if issue.labels.is_empty() { "None".to_string() } else { issue.labels.join(", ") };
                                            let milestone = issue.milestone.as_ref().map(|m| m.title.clone()).unwrap_or_else(|| "None".to_string());
                                            let assignees = if issue.assignees.is_empty() {
                                                "None".to_string()
                                            } else {
                                                issue.assignees.iter().map(|a| format!("@{}", a.username)).collect::<Vec<_>>().join(", ")
                                            };
                                            app.edit_menu = Some(crate::app::EditMenu {
                                                title: format!("Edit Issue #{}", issue.iid),
                                                fields: vec![
                                                    ("Title".to_string(), issue.title.clone()),
                                                    ("Labels".to_string(), labels),
                                                    ("Assignees".to_string(), assignees),
                                                    ("Milestone".to_string(), milestone),
                                                    ("Confidential".to_string(), "Toggle/Set".to_string()),
                                                    ("Due Date".to_string(), "Set".to_string()),
                                                    ("Weight".to_string(), "Set".to_string()),
                                                    ("Description".to_string(), "(Helix)".to_string()),
                                                ],
                                                selected_idx: 0,
                                                entity_iid: issue.iid,
                                                entity_type: "issue".to_string(),
                                                state: { let mut s = ListState::default(); s.select(Some(0)); s },
                                            });
                                        }
                                    }
                                }
                                KeyCode::Char('c') => {
                                    if let Some(selected_idx) = app.issues.state.selected() {
                                        let filtered = app.filtered_issues();
                                        if let Some(issue) = filtered.get(selected_idx) {
                                            let issue_iid = issue.iid;
                                            run_glab_cmd(&["issue", "close", &issue_iid.to_string()], &mut terminal).await;
                                            if let Some(pos) = app.issues.items.iter().position(|i| i.iid == issue_iid) {
                                                app.issues.items.remove(pos);
                                            }
                                            app.update_filter_selection();
                                            if let Some(client) = &app.gitlab_client {
                                                spawn_refresh_active_tab(client, &app.project_context, app.active_tab, events.sender());
                                            }
                                        }
                                    }
                                }
                                _ => handled = false,
                            }
                        }
                        app::Tab::MergeRequests => {
                            if key_event.code == KeyCode::Char('n') {
                                let issue_options: Vec<String> = app.issues.items.iter()
                                    .map(|issue| format!("#{} : {}", issue.iid, issue.title))
                                    .collect();
                                
                                let pr_suffix = if app.gitlab_client.as_ref().map(|c| c.is_github).unwrap_or(false) { "PR" } else { "MR" };
                                
                                app.selector = Some(crate::app::Selector {
                                    title: format!(" Select Issue for New {} ", pr_suffix),
                                    all_items: issue_options,
                                    selected_items: std::collections::HashSet::new(),
                                    cursor_idx: 0,
                                    search_query: String::new(),
                                    is_filtering: true,
                                    is_loading: false,
                                    entity_iid: 0,
                                    entity_type: "mr".to_string(),
                                    field_type: "create_mr".to_string(),
                                    multi_select: false,
                                    state: { let mut s = ListState::default(); s.select(Some(0)); s },
                                });
                            } else if let Some(selected_idx) = app.mrs.state.selected() {
                                let filtered = app.filtered_mrs();
                                let mr_info = filtered.get(selected_idx).map(|item| (item.iid, item.title.clone()));
                                if let Some((mr_iid, mr_title)) = mr_info {
                                    match key_event.code {
                                        KeyCode::Char('e') => {
                                            let mr = filtered.get(selected_idx).unwrap();
                                            let labels = if mr.labels.is_empty() { "None".to_string() } else { mr.labels.join(", ") };
                                            let milestone = mr.milestone.as_ref().map(|m| m.title.clone()).unwrap_or_else(|| "None".to_string());
                                            let assignees = if mr.assignees.is_empty() {
                                                "None".to_string()
                                            } else {
                                                mr.assignees.iter().map(|a| format!("@{}", a.username)).collect::<Vec<_>>().join(", ")
                                            };
                                            let reviewers = if mr.reviewers.is_empty() {
                                                "None".to_string()
                                            } else {
                                                mr.reviewers.iter().map(|r| format!("@{}", r.username)).collect::<Vec<_>>().join(", ")
                                            };
                                            let draft_status = if mr.draft { "Draft" } else { "Ready" };
                                            let pr_suffix = if app.gitlab_client.as_ref().map(|c| c.is_github).unwrap_or(false) { "PR" } else { "MR" };
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
                                                    ("Description".to_string(), mr.description.clone().unwrap_or_default()),
                                                ],
                                                selected_idx: 0,
                                                entity_iid: mr.iid,
                                                entity_type: "mr".to_string(),
                                                state: { let mut s = ListState::default(); s.select(Some(0)); s },
                                            });
                                        }
                                        KeyCode::Char('a') => {
                                            run_glab_cmd(&["mr", "approve", &mr_iid.to_string()], &mut terminal).await;
                                            if let Some(client) = &app.gitlab_client {
                                                spawn_refresh_active_tab(client, &app.project_context, app.active_tab, events.sender());
                                            }
                                        }
                                        KeyCode::Char('m') => {
                                            run_glab_cmd(&["mr", "merge", &mr_iid.to_string(), "--remove-source-branch", "--squash"], &mut terminal).await;
                                            if let Some(pos) = app.mrs.items.iter().position(|m| m.iid == mr_iid) {
                                                app.mrs.items.remove(pos);
                                            }
                                            app.update_filter_selection();
                                            if let Some(client) = &app.gitlab_client {
                                                spawn_refresh_active_tab(client, &app.project_context, app.active_tab, events.sender());
                                            }
                                        }
                                        KeyCode::Char('v') => {
                                            app.diff_loading = true;
                                            let tx = events.sender();
                                            let mr_iid = mr_iid;
                                            let mr_iid_str = mr_iid.to_string();
                                            tokio::spawn(async move {
                                                let is_github = match tokio::process::Command::new("git")
                                                    .args(["remote", "get-url", "origin"])
                                                    .output()
                                                    .await
                                                    .map(|o| String::from_utf8_lossy(&o.stdout).contains("github.com"))
                                                {
                                                    Ok(true) => true,
                                                    _ => false,
                                                };

                                                let cmd_args = vec!["mr", "diff", &mr_iid_str];
                                                let mut cmd = if is_github {
                                                    let gh_args = translate_glab_to_gh(&cmd_args);
                                                    let mut c = tokio::process::Command::new("gh");
                                                    c.args(gh_args);
                                                    c
                                                } else {
                                                    let mut c = tokio::process::Command::new("glab");
                                                    c.args(&cmd_args);
                                                    c
                                                };

                                                match cmd.output().await {
                                                    Ok(output) => {
                                                        if output.status.success() {
                                                            let raw_diff = String::from_utf8_lossy(&output.stdout).into_owned();
                                                            let _ = tx.send(Event::DiffFetched(mr_iid, raw_diff));
                                                        } else {
                                                            let err_msg = String::from_utf8_lossy(&output.stderr);
                                                            let _ = tx.send(Event::DiffFetchFailed(format!("Failed to fetch diff: {}", err_msg)));
                                                        }
                                                    }
                                                    Err(_) => {
                                                        let _ = tx.send(Event::DiffFetchFailed("Failed to execute CLI tool to fetch diff".to_string()));
                                                    }
                                                }
                                            });
                                        }
                                        KeyCode::Char('o') => {
                                            run_glab_cmd(&["mr", "view", &mr_iid.to_string(), "-w"], &mut terminal).await;
                                        }
                                        KeyCode::Char('s') => {
                                            let is_draft = mr_title.starts_with("Draft:") || mr_title.starts_with("WIP:");
                                            let action = if is_draft { "--ready" } else { "--draft" };
                                            run_glab_update("mr", mr_iid, &[action], &mut terminal).await;
                                            if let Some(client) = &app.gitlab_client {
                                                spawn_refresh_active_tab(client, &app.project_context, app.active_tab, events.sender());
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
                        app::Tab::Pipelines => {
                            if key_event.code == KeyCode::Char('p') {
                                run_glab_cmd(&["ci", "run", "--mr"], &mut terminal).await;
                            } else if app.selected_pipeline_jobs.is_some() {
                                if let Some(idx) = app.selected_job_index {
                                    let job_info = app.selected_pipeline_jobs.as_ref().and_then(|jobs| jobs.get(idx)).map(|j| (j.id, j.name.clone()));
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
                                                    let pipe_id = app.pipelines.state.selected()
                                                        .and_then(|sel_idx| app.filtered_pipelines().get(sel_idx).map(|p| p.id))
                                                        .unwrap_or(0);
                                                    let tx = events.sender();
                                                    
                                                    if !app.selected_jobs.is_empty() {
                                                        let job_ids: Vec<u64> = app.selected_jobs.iter().cloned().collect();
                                                        if let Some(jobs_mut) = &mut app.selected_pipeline_jobs {
                                                            for j in jobs_mut.iter_mut() {
                                                                if app.selected_jobs.contains(&j.id) {
                                                                    j.status = "running".to_string();
                                                                }
                                                            }
                                                        }
                                                        app.selected_jobs.clear();
                                                        tokio::spawn(async move {
                                                            for j_id in job_ids {
                                                                let endpoint = format!("projects/{}/jobs/{}/retry", project_context.replace("/", "%2F"), j_id);
                                                                let _ = client_clone.fetch_raw_api(&endpoint).await;
                                                            }
                                                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                                            if let Ok(jobs) = gitlab::pipelines::list_pipeline_jobs(&client_clone, &project_context, pipe_id).await {
                                                                let _ = tx.send(Event::PipelineJobs(pipe_id, jobs));
                                                            }
                                                        });
                                                    } else {
                                                        if let Some(jobs_mut) = &mut app.selected_pipeline_jobs {
                                                            if let Some(j) = jobs_mut.get_mut(idx) {
                                                                j.status = "running".to_string();
                                                            }
                                                        }
                                                        tokio::spawn(async move {
                                                            let endpoint = format!("projects/{}/jobs/{}/retry", project_context.replace("/", "%2F"), job_id);
                                                            let _ = client_clone.fetch_raw_api(&endpoint).await;
                                                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                                            if let Ok(jobs) = gitlab::pipelines::list_pipeline_jobs(&client_clone, &project_context, pipe_id).await {
                                                                let _ = tx.send(Event::PipelineJobs(pipe_id, jobs));
                                                            }
                                                        });
                                                    }
                                                }
                                            }
                                            KeyCode::Char('d') => {
                                                run_glab_cmd(&["job", "artifact", "master", &job_name], &mut terminal).await;
                                            }
                                            KeyCode::Char('o') => {
                                                run_glab_cmd(&["job", "view", &job_id.to_string(), "-w"], &mut terminal).await;
                                            }
                                            KeyCode::Char('e') => {
                                                let temp_file = std::env::temp_dir().join(format!("job_{}_trace.txt", job_id));
                                                if let Some(trace) = &app.job_trace {
                                                    let _ = std::fs::write(&temp_file, trace);
                                                } else if let Some(_) = &app.gitlab_client {
                                                    let _ = std::fs::write(&temp_file, "Trace will be here");
                                                }
                                                crate::event::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
                                                disable_raw_mode().unwrap();
                                                execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture).unwrap();
                                                let editor = std::env::var("EDITOR")
                                                    .or_else(|_| std::env::var("VISUAL"))
                                                    .unwrap_or_else(|_| "helix".to_string());
                                                let mut cmd = if cfg!(target_os = "windows") {
                                                    let mut c = std::process::Command::new("cmd");
                                                    c.args(&["/c", &format!("{} \"{}\"", editor, temp_file.to_string_lossy())]);
                                                    c
                                                } else {
                                                    let mut c = std::process::Command::new(&editor);
                                                    c.arg(&temp_file);
                                                    c
                                                };
                                                cmd.stdin(std::process::Stdio::inherit());
                                                cmd.stdout(std::process::Stdio::inherit());
                                                cmd.stderr(std::process::Stdio::inherit());
                                                if let Ok(mut child) = cmd.spawn() {
                                                    let _ = child.wait();
                                                }
                                                enable_raw_mode().unwrap();
                                                execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture).unwrap();
                                                terminal.clear().unwrap();
                                                crate::event::PAUSED.store(false, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            _ => handled = false,
                                        }
                                    } else {
                                        handled = false;
                                    }
                                } else {
                                    handled = false;
                                }
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
                                        KeyCode::Char('r') => {
                                             if let Some(client) = &app.gitlab_client {
                                                 let client_clone = client.clone();
                                                 let project_context = app.project_context.clone();
                                                 let tx = events.sender();
                                                 let active_tab = app.active_tab;
                                                 if !app.selected_pipelines.is_empty() {
                                                     let pipe_ids: Vec<u64> = app.selected_pipelines.iter().cloned().collect();
                                                     for p_id in &pipe_ids {
                                                         if let Some(p) = app.pipelines.items.iter_mut().find(|pipe| pipe.id == *p_id) {
                                                             p.status = "running".to_string();
                                                         }
                                                     }
                                                     app.selected_pipelines.clear();
                                                     tokio::spawn(async move {
                                                         for p_id in pipe_ids {
                                                             let endpoint = format!("projects/{}/pipelines/{}/retry", project_context.replace("/", "%2F"), p_id);
                                                             let _ = client_clone.fetch_raw_api(&endpoint).await;
                                                         }
                                                         tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                                         spawn_refresh_active_tab(&client_clone, &project_context, active_tab, tx);
                                                     });
                                                 } else {
                                                     if let Some(p) = app.pipelines.items.iter_mut().find(|pipe| pipe.id == pipe_id) {
                                                         p.status = "running".to_string();
                                                     }
                                                     tokio::spawn(async move {
                                                         let endpoint = format!("projects/{}/pipelines/{}/retry", project_context.replace("/", "%2F"), pipe_id);
                                                         let _ = client_clone.fetch_raw_api(&endpoint).await;
                                                         tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                                         spawn_refresh_active_tab(&client_clone, &project_context, active_tab, tx);
                                                     });
                                                 }
                                             }
                                        }
                                        KeyCode::Char('d') => {
                                             if let Some(p) = app.pipelines.items.iter_mut().find(|pipe| pipe.id == pipe_id) {
                                                 p.status = "canceled".to_string();
                                             }
                                             if let Some(client) = &app.gitlab_client {
                                                 let client_clone = client.clone();
                                                 let project_context = app.project_context.clone();
                                                 let tx = events.sender();
                                                 let active_tab = app.active_tab;
                                                 tokio::spawn(async move {
                                                     let endpoint = format!("projects/{}/pipelines/{}/cancel", project_context.replace("/", "%2F"), pipe_id);
                                                     let _ = client_clone.fetch_raw_api(&endpoint).await;
                                                     tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                                     spawn_refresh_active_tab(&client_clone, &project_context, active_tab, tx);
                                                 });
                                             }
                                        }
                                        KeyCode::Char('o') => run_glab_cmd(&["ci", "view", &pipe_id.to_string(), "-w"], &mut terminal).await,
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
                                            run_glab_cmd(&["api", "-X", "PUT", &format!("runners/{}", runner_id), "-f", "paused=true"], &mut terminal).await;
                                            if let Some(runner) = app.runners.items.iter_mut().find(|r| r.id == runner_id) {
                                                runner.status = "paused".to_string();
                                                runner.active = false;
                                            }
                                        }
                                        KeyCode::Char('r') => {
                                            run_glab_cmd(&["api", "-X", "PUT", &format!("runners/{}", runner_id), "-f", "paused=false"], &mut terminal).await;
                                            if let Some(runner) = app.runners.items.iter_mut().find(|r| r.id == runner_id) {
                                                runner.status = "online".to_string();
                                                runner.active = true;
                                            }
                                        }
                                        KeyCode::Char('e') => {
                                            let current_desc = item.description.clone().unwrap_or_default();
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
                        app::Tab::Releases => {
                            if let Some(selected_idx) = app.releases.state.selected() {
                                if let Some(item) = app.filtered_releases().get(selected_idx) {
                                    match key_event.code {
                                        KeyCode::Char('o') => {
                                            run_glab_cmd(&["release", "view", &item.tag_name, "-w"], &mut terminal).await;
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
                    }

                    if !handled {
                        match key_event.code {
                            KeyCode::Char('?') | KeyCode::F(1) => {
                                app.show_help = true;
                            }
                            KeyCode::Char('q') => {
                                if app.details_zoomed {
                                    app.details_zoomed = false;
                                } else {
                                    app.quit();
                                }
                            }
                            KeyCode::Char('J') => {
                                match app.active_tab {
                                    app::Tab::Issues => {
                                        app.issues_scroll = app.issues_scroll.saturating_add(1);
                                    }
                                    app::Tab::MergeRequests => {
                                        app.mrs_scroll = app.mrs_scroll.saturating_add(1);
                                    }
                                    _ => {}
                                }
                            }
                            KeyCode::Char('K') => {
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
                            KeyCode::Esc | KeyCode::Backspace => {
                                if app.details_zoomed {
                                    app.details_zoomed = false;
                                } else if app.active_tab == app::Tab::Pipelines && app.selected_pipeline_jobs.is_some() {
                                    if app.job_trace.is_some() {
                                        app.job_trace = None;
                                    } else {
                                        app.selected_pipeline_jobs = None;
                                        app.selected_job_index = None;
                                        app.selected_jobs.clear();
                                    }
                                } else {
                                    app.quit();
                                }
                            }
                            KeyCode::Char('f') => {
                                app.is_typing_search = true;
                            }
                            KeyCode::Enter => {
                                match app.active_tab {
                                    app::Tab::Pipelines => {
                                        if app.job_trace.is_some() {
                                            app.details_zoomed = !app.details_zoomed;
                                        } else if let Some(jobs) = &app.selected_pipeline_jobs {
                                            if let Some(idx) = app.selected_job_index {
                                                if let Some(job) = jobs.get(idx) {
                                                    if let Some(client) = &app.gitlab_client {
                                                        if let Ok(trace) = gitlab::pipelines::get_job_trace(client, &app.project_context, job.id).await {
                                                            app.job_trace = Some(trace);
                                                            app.job_trace_needs_scroll_to_bottom = true;
                                                            app.details_zoomed = true;
                                                        } else {
                                                            app.error_message = Some("Failed to fetch job trace".to_string());
                                                        }
                                                    }
                                                }
                                            }
                                        } else {
                                            if let Some(idx) = app.pipelines.state.selected() {
                                                if let Some(p) = app.filtered_pipelines().get(idx) {
                                                    if let Some(client) = &app.gitlab_client {
                                                        if let Ok(jobs) = gitlab::pipelines::list_pipeline_jobs(client, &app.project_context, p.id).await {
                                                            app.pipeline_jobs.insert(p.id, jobs.clone());
                                                            app.selected_pipeline_jobs = Some(jobs);
                                                            app.selected_job_index = Some(0);
                                                            app.jobs_list_state.select(Some(0));
                                                            app.job_trace_scroll = 0;
                                                            app.job_trace = None;
                                                        } else {
                                                            app.error_message = Some("Failed to fetch jobs".to_string());
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    _ => {
                                        app.details_zoomed = !app.details_zoomed;
                                    }
                                }
                            }
                            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                                if app.selected_pipeline_jobs.is_none() {
                                    app.next_tab();
                                    if !app.loaded_tabs.contains(&app.active_tab) && !app.loading_tabs.contains(&app.active_tab) {
                                        if let Some(client) = &app.gitlab_client {
                                            app.loading_tabs.insert(app.active_tab);
                                            spawn_refresh_active_tab(client, &app.project_context, app.active_tab, events.sender());
                                        }
                                    }
                                }
                            }
                            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                                if app.selected_pipeline_jobs.is_none() {
                                    app.previous_tab();
                                    if !app.loaded_tabs.contains(&app.active_tab) && !app.loading_tabs.contains(&app.active_tab) {
                                        if let Some(client) = &app.gitlab_client {
                                            app.loading_tabs.insert(app.active_tab);
                                            spawn_refresh_active_tab(client, &app.project_context, app.active_tab, events.sender());
                                        }
                                    }
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                match app.active_tab {
                                    app::Tab::Issues => {
                                        app.issues.next(app.filtered_issues().len());
                                        app.issues_scroll = 0;
                                    }
                                    app::Tab::MergeRequests => {
                                        app.mrs.next(app.filtered_mrs().len());
                                        app.mrs_scroll = 0;
                                    }
                                    app::Tab::Pipelines => {
                                        if app.job_trace.is_some() {
                                            app.job_trace_scroll = app.job_trace_scroll.saturating_add(1);
                                        } else if let Some(jobs) = &app.selected_pipeline_jobs {
                                            if let Some(idx) = &mut app.selected_job_index {
                                                if *idx + 1 < jobs.len() {
                                                    *idx += 1;
                                                    app.jobs_list_state.select(Some(*idx));
                                                    app.job_trace = None;
                                                }
                                            }
                                        } else {
                                            app.pipelines.next(app.filtered_pipelines().len());
                                        }
                                    }
                                    app::Tab::Runners => app.runners.next(app.filtered_runners().len()),
                                    app::Tab::Releases => app.releases.next(app.filtered_releases().len()),
                                }
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                match app.active_tab {
                                    app::Tab::Issues => {
                                        app.issues.previous(app.filtered_issues().len());
                                        app.issues_scroll = 0;
                                    }
                                    app::Tab::MergeRequests => {
                                        app.mrs.previous(app.filtered_mrs().len());
                                        app.mrs_scroll = 0;
                                    }
                                    app::Tab::Pipelines => {
                                        if app.job_trace.is_some() {
                                            app.job_trace_scroll = app.job_trace_scroll.saturating_sub(1);
                                        } else if app.selected_pipeline_jobs.is_some() {
                                            if let Some(idx) = &mut app.selected_job_index {
                                                if *idx > 0 {
                                                    *idx -= 1;
                                                    app.jobs_list_state.select(Some(*idx));
                                                    app.job_trace = None;
                                                }
                                            }
                                        } else {
                                            app.pipelines.previous(app.filtered_pipelines().len());
                                        }
                                    }
                                    app::Tab::Runners => app.runners.previous(app.filtered_runners().len()),
                                    app::Tab::Releases => app.releases.previous(app.filtered_releases().len()),
                                }
                            }
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
    fn test_translate_glab_to_gh_issue_close() {
        let glab_args = vec!["issue", "close", "123"];
        let gh_args = translate_glab_to_gh(&glab_args);
        assert_eq!(gh_args, vec!["issue".to_string(), "close".to_string(), "123".to_string()]);
    }

    #[test]
    fn test_translate_glab_to_gh_issue_create() {
        let glab_args = vec!["issue", "create", "--title", "Bug report"];
        let gh_args = translate_glab_to_gh(&glab_args);
        assert_eq!(gh_args, vec![
            "issue".to_string(),
            "create".to_string(),
            "--title".to_string(),
            "Bug report".to_string(),
            "--body".to_string(),
            "".to_string()
        ]);
    }

    #[test]
    fn test_translate_glab_to_gh_mr_create_with_issue() {
        let glab_args = vec!["mr", "create", "-i", "123", "--copy-issue-labels"];
        let gh_args = translate_glab_to_gh(&glab_args);
        assert_eq!(gh_args, vec![
            "pr".to_string(),
            "create".to_string(),
            "--fill".to_string(),
            "--body".to_string(),
            "Resolves #123".to_string()
        ]);
    }
}

