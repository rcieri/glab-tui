#![allow(clippy::all)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_assignments)]

mod app;
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
        c.args(&[
            "/c",
            &format!("{} \"{}\"", editor, tmp.path().to_string_lossy()),
        ]);
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
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

// old edit_in_editor implementation removed

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
    match field_type {
        "title" => {
            run_glab_update(
                entity_type,
                iid,
                &["--title", &value],
                terminal,
                tx.clone(),
                tab,
            )
            .await;
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
                run_glab_update(
                    entity_type,
                    iid,
                    &["--target-branch", &value],
                    terminal,
                    tx.clone(),
                    tab,
                )
                .await;
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.target_branch = value;
                }
            }
        }
        "due_date" => {
            if entity_type == "issue" {
                if value == "YYYY-MM-DD" || value.trim().is_empty() {
                    run_glab_update(
                        entity_type,
                        iid,
                        &["--due-date", ""],
                        terminal,
                        tx.clone(),
                        tab,
                    )
                    .await;
                } else {
                    run_glab_update(
                        entity_type,
                        iid,
                        &["--due-date", &value],
                        terminal,
                        tx.clone(),
                        tab,
                    )
                    .await;
                }
            }
        }
        "weight" => {
            if entity_type == "issue" {
                run_glab_update(
                    entity_type,
                    iid,
                    &["--weight", &value],
                    terminal,
                    tx.clone(),
                    tab,
                )
                .await;
            }
        }
        "runner_description" => {
            run_glab_cmd(
                &[
                    "api",
                    "-X",
                    "PUT",
                    &format!("runners/{}", iid),
                    "-f",
                    &format!("description={}", value),
                ],
                terminal,
                tx.clone(),
                tab,
            )
            .await;
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
                        title = Some(args[i + 1]);
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
                                gh_args.push(args[i + 1].to_string());
                                i += 2;
                            } else {
                                i += 1;
                            }
                        }
                        "--label" => {
                            if i + 1 < args.len() {
                                gh_args.push("--label".to_string());
                                gh_args.push(args[i + 1].to_string());
                                i += 2;
                            } else {
                                i += 1;
                            }
                        }
                        "--unlabel" => {
                            if i + 1 < args.len() && args[i + 1] == "all" {
                                if i + 2 < args.len() && args[i + 2] == "--label" {
                                    // skip
                                } else {
                                    gh_args.push("--label".to_string());
                                    gh_args.push("".to_string());
                                }
                                i += 2;
                            } else {
                                i += 1;
                            }
                        }
                        "--assignee" => {
                            if i + 1 < args.len() {
                                gh_args.push("--assignee".to_string());
                                gh_args.push(args[i + 1].to_string());
                                i += 2;
                            } else {
                                i += 1;
                            }
                        }
                        "--unassign" => {
                            gh_args.push("--assignee".to_string());
                            gh_args.push("".to_string());
                            i += 1;
                        }
                        "--milestone" => {
                            if i + 1 < args.len() {
                                gh_args.push("--milestone".to_string());
                                let ms = if args[i + 1] == "0" { "" } else { args[i + 1] };
                                gh_args.push(ms.to_string());
                                i += 2;
                            } else {
                                i += 1;
                            }
                        }
                        "-d" | "--description" => {
                            if i + 1 < args.len() {
                                if args[i + 1] == "-" {
                                    gh_args.push("--body-file".to_string());
                                    gh_args.push("-".to_string());
                                } else {
                                    gh_args.push("--body".to_string());
                                    gh_args.push(args[i + 1].to_string());
                                }
                                i += 2;
                            } else {
                                i += 1;
                            }
                        }
                        _ => {
                            i += 1;
                        }
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
                        let mut title = None;
                        let mut body = None;
                        let mut label = None;
                        let mut assignee = None;
                        let mut milestone = None;
                        let mut reviewer = None;
                        let mut head = None;
                        let mut base = None;
                        let mut draft = false;
                        let mut issue_id = None;

                        let mut i = 2;
                        while i < args.len() {
                            match args[i] {
                                "--title" => {
                                    if i + 1 < args.len() {
                                        title = Some(args[i + 1]);
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "--description" => {
                                    if i + 1 < args.len() {
                                        body = Some(args[i + 1]);
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "--label" => {
                                    if i + 1 < args.len() {
                                        label = Some(args[i + 1]);
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "--assignee" => {
                                    if i + 1 < args.len() {
                                        assignee = Some(args[i + 1]);
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "--milestone" => {
                                    if i + 1 < args.len() {
                                        milestone = Some(args[i + 1]);
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "--reviewer" => {
                                    if i + 1 < args.len() {
                                        reviewer = Some(args[i + 1]);
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "--source-branch" => {
                                    if i + 1 < args.len() {
                                        head = Some(args[i + 1]);
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "--target-branch" => {
                                    if i + 1 < args.len() {
                                        base = Some(args[i + 1]);
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "--draft" => {
                                    draft = true;
                                    i += 1;
                                }
                                "-i" | "--related-issue" => {
                                    if i + 1 < args.len() {
                                        issue_id = Some(args[i + 1]);
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                _ => {
                                    i += 1;
                                }
                            }
                        }

                        if let Some(t) = title {
                            gh_args.push("--title".to_string());
                            gh_args.push(t.to_string());
                        }

                        if title.is_none() {
                            gh_args.push("--fill".to_string());
                        }

                        if let Some(b) = body {
                            gh_args.push("--body".to_string());
                            if let Some(id) = issue_id {
                                if !b.contains(&format!("#{}", id)) {
                                    gh_args.push(format!("Resolves #{}\n\n{}", id, b));
                                } else {
                                    gh_args.push(b.to_string());
                                }
                            } else {
                                gh_args.push(b.to_string());
                            }
                        } else if let Some(id) = issue_id {
                            gh_args.push("--body".to_string());
                            gh_args.push(format!("Resolves #{}", id));
                        } else if title.is_some() {
                            gh_args.push("--body".to_string());
                            gh_args.push("".to_string());
                        }

                        if let Some(l) = label {
                            gh_args.push("--label".to_string());
                            gh_args.push(l.to_string());
                        }
                        if let Some(a) = assignee {
                            gh_args.push("--assignee".to_string());
                            gh_args.push(a.to_string());
                        }
                        if let Some(m) = milestone {
                            gh_args.push("--milestone".to_string());
                            gh_args.push(m.to_string());
                        }
                        if let Some(r) = reviewer {
                            gh_args.push("--reviewer".to_string());
                            gh_args.push(r.to_string());
                        }
                        if let Some(h) = head {
                            gh_args.push("--head".to_string());
                            gh_args.push(h.to_string());
                        }
                        if let Some(b) = base {
                            gh_args.push("--base".to_string());
                            gh_args.push(b.to_string());
                        }
                        if draft {
                            gh_args.push("--draft".to_string());
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
                                        gh_args.push(args[i + 1].to_string());
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "--label" => {
                                    if i + 1 < args.len() {
                                        gh_args.push("--label".to_string());
                                        gh_args.push(args[i + 1].to_string());
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "--unlabel" => {
                                    if i + 1 < args.len() && args[i + 1] == "all" {
                                        if i + 2 < args.len() && args[i + 2] == "--label" {
                                            // skip
                                        } else {
                                            gh_args.push("--label".to_string());
                                            gh_args.push("".to_string());
                                        }
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "--assignee" => {
                                    if i + 1 < args.len() {
                                        gh_args.push("--assignee".to_string());
                                        gh_args.push(args[i + 1].to_string());
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "--unassign" => {
                                    gh_args.push("--assignee".to_string());
                                    gh_args.push("".to_string());
                                    i += 1;
                                }
                                "--reviewer" => {
                                    if i + 1 < args.len() {
                                        gh_args.push("--reviewer".to_string());
                                        gh_args.push(args[i + 1].to_string());
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "--milestone" => {
                                    if i + 1 < args.len() {
                                        gh_args.push("--milestone".to_string());
                                        let ms = if args[i + 1] == "0" { "" } else { args[i + 1] };
                                        gh_args.push(ms.to_string());
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "--target-branch" => {
                                    if i + 1 < args.len() {
                                        gh_args.push("--base".to_string());
                                        gh_args.push(args[i + 1].to_string());
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                "-d" | "--description" => {
                                    if i + 1 < args.len() {
                                        if args[i + 1] == "-" {
                                            gh_args.push("--body-file".to_string());
                                            gh_args.push("-".to_string());
                                        } else {
                                            gh_args.push("--body".to_string());
                                            gh_args.push(args[i + 1].to_string());
                                        }
                                        i += 2;
                                    } else {
                                        i += 1;
                                    }
                                }
                                _ => {
                                    i += 1;
                                }
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
            if args.len() > 1 && args[1] == "view" {
                gh_args.push("run".to_string());
                gh_args.push("view".to_string());
                if args.len() > 2 {
                    gh_args.push(args[2].to_string());
                }
                gh_args.push("--web".to_string());
            } else if args.len() > 1 && args[1] == "run" {
                gh_args.push("workflow".to_string());
                gh_args.push("run".to_string());
                let mut workflow = None;
                let mut branch = None;
                let mut inputs = Vec::new();
                let mut i = 2;
                while i < args.len() {
                    match args[i] {
                        "-b" | "--branch" => {
                            if i + 1 < args.len() {
                                branch = Some(args[i + 1]);
                                i += 2;
                            } else {
                                i += 1;
                            }
                        }
                        "--variables" | "--variables-env" | "-i" | "--input" => {
                            if i + 1 < args.len() {
                                let pair = args[i + 1];
                                if let Some(pos) = pair.find(':').or_else(|| pair.find('=')) {
                                    let k = &pair[..pos];
                                    let v = &pair[pos + 1..];
                                    inputs.push(format!("{}={}", k, v));
                                }
                                i += 2;
                            } else {
                                i += 1;
                            }
                        }
                        "--mr" => {
                            i += 1;
                        }
                        arg => {
                            workflow = Some(arg);
                            i += 1;
                        }
                    }
                }
                if let Some(wf) = workflow {
                    gh_args.push(wf.to_string());
                }
                if let Some(b) = branch {
                    gh_args.push("-r".to_string());
                    gh_args.push(b.to_string());
                }
                for input in inputs {
                    gh_args.push("-f".to_string());
                    gh_args.push(input);
                }
            } else {
                gh_args.push("run".to_string());
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

fn is_command_interactive(args: &[&str]) -> bool {
    args.iter().any(|&arg| arg == "-d" || arg == "--desc") && args.iter().any(|&arg| arg == "-")
}

fn get_command_description(args: &[&str], is_github: bool) -> String {
    let pr_suffix = if is_github { "PR" } else { "MR" };
    if args.len() >= 2 {
        match (args[0], args[1]) {
            ("issue", "create") => "Creating Issue".to_string(),
            ("issue", "close") => "Closing Issue".to_string(),
            ("mr", "create") => format!("Creating {}", pr_suffix),
            ("mr", "merge") => format!("Merging {}", pr_suffix),
            ("mr", "diff") => "Fetching Diff".to_string(),
            ("ci", "run") | ("workflow", "run") => "Running Pipeline".to_string(),
            _ => "Running Command".to_string(),
        }
    } else {
        "Running Command".to_string()
    }
}

async fn run_glab_cmd(
    args: &[&str],
    terminal: &mut AppTerminal,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    tab: crate::app::Tab,
) {
    let is_github = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("github.com"))
        .unwrap_or(false);

    let program = if is_github { "gh" } else { "glab" };
    let actual_args = if is_github {
        translate_glab_to_gh(args)
    } else {
        args.iter().map(|s| s.to_string()).collect::<Vec<_>>()
    };

    if is_command_interactive(args) {
        crate::event::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        disable_raw_mode().unwrap();
        execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture).unwrap();

        let mut cmd = std::process::Command::new(program);
        cmd.args(&actual_args);
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

        let _ = tx.send(Event::CommandCompleted(tab, Ok(())));
    } else {
        let desc = get_command_description(args, is_github);
        let status_msg = format!("{}: {} {}", desc, program, args.join(" "));
        let _ = tx.send(Event::CommandStarted(status_msg));

        let tx_clone = tx.clone();
        let program = program.to_string();

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

async fn run_glab_update(
    entity_type: &str,
    id: u64,
    args: &[&str],
    terminal: &mut AppTerminal,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    tab: crate::app::Tab,
) {
    let id_str = id.to_string();
    let mut cmd_args = vec![entity_type, "update", &id_str];
    cmd_args.extend_from_slice(args);
    run_glab_cmd(&cmd_args, terminal, tx, tab).await;
}

async fn apply_selector_changes(
    app: &mut App,
    entity_type: &str,
    iid: u64,
    field_type: &str,
    values: Vec<String>,
    terminal: &mut AppTerminal,
) {
    let tx = app.tx.clone().unwrap();
    let tab = app.active_tab;
    match field_type {
        "labels" => {
            let labels_comma = values.join(",");
            if labels_comma.is_empty() {
                run_glab_update(
                    entity_type,
                    iid,
                    &["--unlabel", "all"],
                    terminal,
                    tx.clone(),
                    tab,
                )
                .await;
            } else {
                run_glab_update(
                    entity_type,
                    iid,
                    &["--unlabel", "all", "--label", &labels_comma],
                    terminal,
                    tx.clone(),
                    tab,
                )
                .await;
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
            let clean_values: Vec<String> = values
                .iter()
                .map(|v| v.trim_start_matches('@').to_string())
                .collect();
            let assignees_comma = clean_values.join(",");

            if assignees_comma.is_empty() {
                run_glab_update(entity_type, iid, &["--unassign"], terminal, tx.clone(), tab).await;
            } else {
                run_glab_update(
                    entity_type,
                    iid,
                    &["--assignee", &assignees_comma],
                    terminal,
                    tx.clone(),
                    tab,
                )
                .await;
            }

            let new_assignees: Vec<crate::gitlab::issues::Assignee> = clean_values
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

                run_glab_update(
                    entity_type,
                    iid,
                    &["--reviewer", &reviewers_comma],
                    terminal,
                    tx.clone(),
                    tab,
                )
                .await;

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
                run_glab_update(
                    entity_type,
                    iid,
                    &["--milestone", milestone_title],
                    terminal,
                    tx.clone(),
                    tab,
                )
                .await;

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
                run_glab_update(
                    entity_type,
                    iid,
                    &["--milestone", "0"],
                    terminal,
                    tx.clone(),
                    tab,
                )
                .await;

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
                    run_glab_update(
                        entity_type,
                        iid,
                        &["--confidential"],
                        terminal,
                        tx.clone(),
                        tab,
                    )
                    .await;
                } else {
                    run_glab_update(entity_type, iid, &["--public"], terminal, tx.clone(), tab)
                        .await;
                }
            }
        }
        "draft_status" => {
            if let Some(val) = values.first() {
                let action = if val.to_lowercase() == "draft" {
                    "--draft"
                } else {
                    "--ready"
                };
                run_glab_update(entity_type, iid, &[action], terminal, tx.clone(), tab).await;
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
                    (
                        "Description".to_string(),
                        issue.description.clone().unwrap_or_default(),
                    ),
                ],
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
                run_glab_update(
                    entity_type,
                    iid,
                    &["--title", &new_title],
                    terminal,
                    tx.clone(),
                    tab,
                )
                .await;
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
                run_glab_update(entity_type, iid, &[action], terminal, tx.clone(), tab).await;
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
                    run_glab_update(
                        entity_type,
                        iid,
                        &["--target-branch", &target],
                        terminal,
                        tx.clone(),
                        tab,
                    )
                    .await;
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
                        run_glab_update(
                            entity_type,
                            iid,
                            &["--confidential"],
                            terminal,
                            tx.clone(),
                            tab,
                        )
                        .await;
                    } else {
                        run_glab_update(entity_type, iid, &["--public"], terminal, tx.clone(), tab)
                            .await;
                    }
                }
            }
        }
        KeyCode::Char('u') => {
            if entity_type == "issue" {
                if let Some(due_date) = edit_in_editor("YYYY-MM-DD", terminal) {
                    if due_date == "YYYY-MM-DD" || due_date.is_empty() {
                        run_glab_update(
                            entity_type,
                            iid,
                            &["--due-date", ""],
                            terminal,
                            tx.clone(),
                            tab,
                        )
                        .await;
                    } else {
                        run_glab_update(
                            entity_type,
                            iid,
                            &["--due-date", &due_date],
                            terminal,
                            tx.clone(),
                            tab,
                        )
                        .await;
                    }
                }
            }
        }
        KeyCode::Char('w') => {
            if entity_type == "issue" {
                if let Some(weight) = edit_in_editor("0", terminal) {
                    run_glab_update(
                        entity_type,
                        iid,
                        &["--weight", &weight],
                        terminal,
                        tx.clone(),
                        tab,
                    )
                    .await;
                }
            }
        }
        KeyCode::Char('d') => {
            let is_github = app
                .gitlab_client
                .as_ref()
                .map(|c| c.is_github)
                .unwrap_or(false);
            if is_github {
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
                    run_glab_update(
                        entity_type,
                        iid,
                        &["-d", &new_desc],
                        terminal,
                        tx.clone(),
                        tab,
                    )
                    .await;
                }
            } else {
                run_glab_update(entity_type, iid, &["-d", "-"], terminal, tx.clone(), tab).await;
            }
            if let Some(client) = &app.gitlab_client {
                if entity_type == "issue" {
                    if let Ok(updated) =
                        gitlab::issues::get_issue(client, &app.project_context, iid).await
                    {
                        if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                            *item = updated;
                        }
                    }
                } else if entity_type == "mr" {
                    if let Ok(updated) = gitlab::mr::get_mr(client, &app.project_context, iid).await
                    {
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

fn spawn_refresh_active_tab(
    client: &gitlab::client::GitlabClient,
    project_context: &str,
    tab: app::Tab,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    show_closed: bool,
) {
    let client = client.clone();
    let project_context = project_context.to_string();
    tokio::spawn(async move {
        match tab {
            app::Tab::Issues => {
                match gitlab::issues::list_issues(&client, &project_context, show_closed).await {
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
                match gitlab::mr::list_mrs(&client, &project_context, show_closed).await {
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
            app::Tab::Notifications => {
                match gitlab::notifications::list_notifications(&client).await {
                    Ok(notifs) => {
                        let _ = tx.send(Event::NotificationsFetched(notifs));
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
            app::Tab::Wiki => match gitlab::wiki::load_wiki_pages(&project_context).await {
                Ok(pages) => {
                    let _ = tx.send(Event::WikiFetched(pages));
                }
                Err(e) => {
                    let _ = tx.send(Event::FetchFailed(
                        tab,
                        format!("Failed to fetch wiki pages: {}", e),
                    ));
                }
            },
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
    app.notifications.items = cache.notifications;
    app.milestones.items = cache.milestones;
    app.wiki_pages.items = cache.wiki_pages;

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
    if !app.notifications.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Notifications);
    }
    if !app.milestones.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Milestones);
    }
    if !app.wiki_pages.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Wiki);
    }
    app.update_filter_selection();

    if let Ok(mut client) = gitlab::client::GitlabClient::new().await {
        client.tx = Some(events.sender());
        app.gitlab_client = Some(client.clone());
        let tx = events.sender();
        if app.issues.items.is_empty() {
            app.start_loading_tab(app.active_tab);
        }
        spawn_refresh_active_tab(
            &client,
            &app.project_context,
            app.active_tab,
            tx.clone(),
            app.is_column_visible(app.active_tab, "Show Closed Items"),
        );
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
                                    app.is_column_visible(app.active_tab, "Show Closed Items"),
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
                    app.pipeline_jobs.clear();
                    app.fetching_pipelines.clear();
                    let mut cache = crate::utils::cache::load_cache(&app.project_context);
                    cache.pipelines = app.pipelines.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &cache);
                }
                Event::NotificationsFetched(notifs) => {
                    app.complete_loading_tab(app::Tab::Notifications, "Success");
                    app.loaded_tabs.insert(app::Tab::Notifications);
                    app.refreshed_tabs.insert(app::Tab::Notifications);
                    app.status_message = None;
                    app.notifications.items = notifs;
                    app.update_filter_selection();
                    let mut cache = crate::utils::cache::load_cache(&app.project_context);
                    cache.notifications = app.notifications.items.clone();
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
                Event::WikiFetched(pages) => {
                    app.complete_loading_tab(app::Tab::Wiki, "Success");
                    app.loaded_tabs.insert(app::Tab::Wiki);
                    app.refreshed_tabs.insert(app::Tab::Wiki);
                    app.status_message = None;
                    app.wiki_pages.items = pages;
                    app.update_filter_selection();
                    let mut cache = crate::utils::cache::load_cache(&app.project_context);
                    cache.wiki_pages = app.wiki_pages.items.clone();
                    crate::utils::cache::save_cache(&app.project_context, &cache);
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
                        app::Tab::Notifications => !app.notifications.items.is_empty(),
                        app::Tab::Milestones => !app.milestones.items.is_empty(),
                        app::Tab::Wiki => !app.wiki_pages.items.is_empty(),
                        _ => false,
                    };
                    if has_cached_items {
                        app.status_message = Some("Offline / Connection failed".to_string());
                    } else {
                        app.error_message = Some(err_msg);
                    }
                }
                Event::DiffFetched(mr_iid, raw_diff) => {
                    app.diff_loading = false;
                    app.diff_view = Some(crate::app::DiffView::new(mr_iid, raw_diff));
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
                                    app.is_column_visible(app.active_tab, "Show Closed Items"),
                                );
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
                                .contains(crossterm::event::KeyModifiers::CONTROL));

                    if is_refresh
                        && app.text_input.is_none()
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
                                    app.is_column_visible(app.active_tab, "Show Closed Items"),
                                );
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
                                            run_glab_cmd(
                                                &["issue", "create", "-y", "--title", &value],
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
                                    } => {
                                        if !value.trim().is_empty() {
                                            if app.in_review_mode {
                                                app.draft_comments.push(crate::app::DraftComment {
                                                    file_path,
                                                    line_num,
                                                    old_line_num,
                                                    body: value,
                                                });
                                                app.status_message = Some(format!(
                                                    "Added draft comment. ({} pending)",
                                                    app.draft_comments.len()
                                                ));
                                            } else {
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
                                                let args_ref: Vec<&str> =
                                                    args.iter().map(|s| s.as_str()).collect();
                                                run_glab_cmd(
                                                    &args_ref,
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
                                                        arr.push(serde_json::json!({
                                                            "path": comment.file_path,
                                                            "line": line,
                                                            "body": comment.body,
                                                        }));
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
                                                        Ok(out) if out.status.success() => {
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
                                                                        let approval_err = String::from_utf8_lossy(&out.stderr).trim().to_string();
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
                                                                let temp_path = std::env::temp_dir(
                                                                )
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
                                                                        "Bulk publish failed: {}",
                                                                        err
                                                                    )),
                                                                ));
                                                        }
                                                        Err(e) => {
                                                            let _ = tx.send(Event::CommandCompleted(
                                                                app::Tab::MergeRequests,
                                                                Err(format!("Failed to publish draft notes: {}", e)),
                                                            ));
                                                        }
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
                                    selector.is_filtering = true;
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
                                                    app.notifications.items.clear();
                                                    app.milestones.items.clear();
                                                    app.wiki_pages.items.clear();
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
                                                    app.notifications.items = cache.notifications;
                                                    app.milestones.items = cache.milestones;
                                                    app.wiki_pages.items = cache.wiki_pages;

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
                                                    if !app.notifications.items.is_empty() {
                                                        app.loaded_tabs
                                                            .insert(app::Tab::Notifications);
                                                    }
                                                    if !app.milestones.items.is_empty() {
                                                        app.loaded_tabs
                                                            .insert(app::Tab::Milestones);
                                                    }
                                                    if !app.wiki_pages.items.is_empty() {
                                                        app.loaded_tabs.insert(app::Tab::Wiki);
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
                                                            app::Tab::Notifications => {
                                                                !app.notifications.items.is_empty()
                                                            }
                                                            app::Tab::Milestones => {
                                                                !app.milestones.items.is_empty()
                                                            }
                                                            app::Tab::Wiki => {
                                                                !app.wiki_pages.items.is_empty()
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
                                                            app.is_column_visible(
                                                                app.active_tab,
                                                                "Show Closed Items",
                                                            ),
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
                                        let mut description_val =
                                            get_default_template("mr").unwrap_or_default();
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
                                                (
                                                    "Source Branch".to_string(),
                                                    get_current_branch().unwrap_or_default(),
                                                ),
                                                ("Target Branch".to_string(), String::new()),
                                                ("Labels".to_string(), labels_val),
                                                ("Assignees".to_string(), assignees_val),
                                                ("Reviewers".to_string(), String::new()),
                                                ("Milestone".to_string(), milestone_val),
                                                (
                                                    "Status (Draft/Ready)".to_string(),
                                                    "Ready".to_string(),
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

                                    let entity_type = selector.entity_type.clone();
                                    let entity_iid = selector.entity_iid;
                                    let filtered_items = selector.get_filtered_items();
                                    let mut selected_list: Vec<String> =
                                        selector.selected_items.iter().cloned().collect();

                                    // Auto-select highlighted item on Enter for single-select fields if nothing selected
                                    if !selector.multi_select && selected_list.is_empty() {
                                        if !filtered_items.is_empty() {
                                            let item = &filtered_items[selector.cursor_idx];
                                            if !item.starts_with("+ Create \"") {
                                                selected_list.push(item.clone());
                                            }
                                        }
                                    }

                                    if entity_iid == 0 {
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
                                                app.is_column_visible(
                                                    app.active_tab,
                                                    "Show Closed Items",
                                                ),
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
                                let max_idx = if menu.entity_iid == 0 {
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
                                if menu.entity_iid == 0 && menu.selected_idx == menu.fields.len() {
                                    menu.selected_idx += 1;
                                }
                                menu.state.select(Some(menu.selected_idx));
                                app.edit_menu = Some(menu);
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                let max_idx = if menu.entity_iid == 0 {
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
                                if menu.entity_iid == 0 && menu.selected_idx == menu.fields.len() {
                                    menu.selected_idx = menu.fields.len().saturating_sub(1);
                                }
                                menu.state.select(Some(menu.selected_idx));
                                app.edit_menu = Some(menu);
                            }
                            KeyCode::Enter => {
                                let entity_iid = menu.entity_iid;
                                let entity_type = menu.entity_type.clone();
                                let is_on_submit =
                                    entity_iid == 0 && menu.selected_idx == menu.fields.len() + 1;

                                if is_on_submit {
                                    if entity_iid == 0 {
                                        if entity_type == "new_issue" {
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

                                            let mut cmd_args = vec!["issue", "create"];
                                            if !title.is_empty() {
                                                cmd_args.push("--title");
                                                cmd_args.push(&title);
                                            }
                                            if !description.is_empty() {
                                                cmd_args.push("--description");
                                                cmd_args.push(&description);
                                            }
                                            if !labels.is_empty() {
                                                cmd_args.push("--label");
                                                cmd_args.push(&labels);
                                            }
                                            let clean_assignees;
                                            if !assignees.is_empty() {
                                                clean_assignees = assignees
                                                    .split(',')
                                                    .map(|a| {
                                                        a.trim().trim_start_matches('@').to_string()
                                                    })
                                                    .collect::<Vec<_>>()
                                                    .join(",");
                                                cmd_args.push("--assignee");
                                                cmd_args.push(&clean_assignees);
                                            }
                                            if !milestone.is_empty() {
                                                cmd_args.push("--milestone");
                                                cmd_args.push(&milestone);
                                            }
                                            if confidential.to_lowercase() == "yes" {
                                                cmd_args.push("--confidential");
                                            }
                                            if !due_date.is_empty() {
                                                cmd_args.push("--due-date");
                                                cmd_args.push(&due_date);
                                            }
                                            if !weight.is_empty() && weight != "0" {
                                                cmd_args.push("--weight");
                                                cmd_args.push(&weight);
                                            }

                                            app.edit_menu = None;
                                            run_glab_cmd(
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
                                                    app.is_column_visible(
                                                        app.active_tab,
                                                        "Show Closed Items",
                                                    ),
                                                );
                                            }
                                            continue;
                                        } else if entity_type == "new_mr" {
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

                                            let entity_iid_str = menu.entity_iid.to_string();
                                            let mut cmd_args = vec!["mr", "create", "-y"];
                                            if menu.entity_iid > 0 {
                                                cmd_args.push("--related-issue");
                                                cmd_args.push(&entity_iid_str);
                                            }
                                            if !title.is_empty() {
                                                cmd_args.push("--title");
                                                cmd_args.push(&title);
                                            }
                                            if !source.is_empty() {
                                                cmd_args.push("--source-branch");
                                                cmd_args.push(&source);
                                            }
                                            if !target.is_empty() {
                                                cmd_args.push("--target-branch");
                                                cmd_args.push(&target);
                                            }
                                            if !labels.is_empty() {
                                                cmd_args.push("--label");
                                                cmd_args.push(&labels);
                                            }
                                            let clean_assignees;
                                            if !assignees.is_empty() {
                                                clean_assignees = assignees
                                                    .split(',')
                                                    .map(|a| {
                                                        a.trim().trim_start_matches('@').to_string()
                                                    })
                                                    .collect::<Vec<_>>()
                                                    .join(",");
                                                cmd_args.push("--assignee");
                                                cmd_args.push(&clean_assignees);
                                            }
                                            let clean_reviewers;
                                            if !reviewers.is_empty() {
                                                clean_reviewers = reviewers
                                                    .split(',')
                                                    .map(|r| {
                                                        r.trim().trim_start_matches('@').to_string()
                                                    })
                                                    .collect::<Vec<_>>()
                                                    .join(",");
                                                cmd_args.push("--reviewer");
                                                cmd_args.push(&clean_reviewers);
                                            }
                                            if !milestone.is_empty() {
                                                cmd_args.push("--milestone");
                                                cmd_args.push(&milestone);
                                            }
                                            if status.to_lowercase() == "draft" {
                                                cmd_args.push("--draft");
                                            }
                                            if mr_pipeline.to_lowercase() == "yes" {
                                                cmd_args.push("--create-pipeline");
                                            }
                                            if !description.is_empty() {
                                                cmd_args.push("--description");
                                                cmd_args.push(&description);
                                            }

                                            app.edit_menu = None;
                                            run_glab_cmd(
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
                                                    app.is_column_visible(
                                                        app.active_tab,
                                                        "Show Closed Items",
                                                    ),
                                                );
                                            }
                                            continue;
                                        } else if entity_type == "new_pipeline" {
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
                                            for (k, v) in var_pairs {
                                                var_strs.push(format!("{}:{}", k, v));
                                            }

                                            let input_pairs = parse_key_value_pairs(&inputs);
                                            let mut input_strs = Vec::new();
                                            for (k, v) in input_pairs {
                                                input_strs.push(format!("{}:{}", k, v));
                                            }

                                            let mut cmd_args = vec!["ci", "run"];
                                            if !workflow.is_empty() {
                                                cmd_args.push(&workflow);
                                            }
                                            if !branch.is_empty() {
                                                cmd_args.push("-b");
                                                cmd_args.push(&branch);
                                            }
                                            if mr.to_lowercase() == "yes" {
                                                cmd_args.push("--mr");
                                            }

                                            for s in &var_strs {
                                                cmd_args.push("--variables");
                                                cmd_args.push(s);
                                            }
                                            for s in &input_strs {
                                                cmd_args.push("-i");
                                                cmd_args.push(s);
                                            }

                                            app.edit_menu = None;
                                            run_glab_cmd(
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
                                                    app.is_column_visible(
                                                        app.active_tab,
                                                        "Show Closed Items",
                                                    ),
                                                );
                                            }
                                            continue;
                                        }
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
                                        if entity_iid == 0 {
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
                                        if entity_iid == 0 {
                                            let current_val =
                                                menu.fields[menu.selected_idx].1.clone();
                                            if !current_val.is_empty() {
                                                current_set.insert(current_val);
                                            } else {
                                                current_set.insert("No".to_string());
                                            }
                                        }
                                    }

                                    if entity_iid == 0 {
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

                                if field_name == "Title"
                                    || field_name == "Source Branch"
                                    || field_name == "Target Branch"
                                    || field_name == "Due Date"
                                    || field_name == "Weight"
                                    || field_name == "Description"
                                    || field_name == "Branch / Ref"
                                    || field_name == "Variables"
                                    || field_name == "Inputs"
                                    || field_name == "Workflow / CI File (GitHub)"
                                {
                                    let current_val = if entity_iid == 0 {
                                        menu.fields[menu.selected_idx].1.clone()
                                    } else {
                                        let field_type = match field_name.as_str() {
                                            "Title" => "title",
                                            "Target Branch" => "target_branch",
                                            "Due Date" => "due_date",
                                            "Weight" => "weight",
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
                                            "due_date" => "".to_string(),
                                            "weight" => "0".to_string(),
                                            _ => String::new(),
                                        }
                                    };

                                    let action = if entity_iid == 0 {
                                        crate::app::TextInputAction::EditNewField {
                                            field_idx: menu.selected_idx,
                                        }
                                    } else {
                                        let field_type = match field_name.as_str() {
                                            "Title" => "title",
                                            "Target Branch" => "target_branch",
                                            "Due Date" => "due_date",
                                            "Weight" => "weight",
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

                                if field_name == "Description" && entity_iid > 0 {
                                    let active_tab = app.active_tab;
                                    handle_entity_update(
                                        &mut app,
                                        &entity_type,
                                        entity_iid,
                                        KeyCode::Char('d'),
                                        &mut terminal,
                                        events.sender(),
                                        active_tab,
                                    )
                                    .await;
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
                                    if !diff_view.lines.is_empty() {
                                        diff_view.cursor_idx = (diff_view.cursor_idx + 1)
                                            .min(diff_view.lines.len() - 1);
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
                                    if let Some(&next_hunk) = diff_view
                                        .hunks
                                        .iter()
                                        .find(|&&idx| idx > diff_view.cursor_idx)
                                    {
                                        diff_view.cursor_idx = next_hunk;
                                        diff_view.update_selected_file_from_cursor();
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('K') => {
                                if !diff_view.focus_on_files {
                                    if let Some(&prev_hunk) = diff_view
                                        .hunks
                                        .iter()
                                        .rev()
                                        .find(|&&idx| idx < diff_view.cursor_idx)
                                    {
                                        diff_view.cursor_idx = prev_hunk;
                                        diff_view.update_selected_file_from_cursor();
                                    }
                                }
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('c') => {
                                if let Some(line) = diff_view.lines.get(diff_view.cursor_idx) {
                                    let can_comment = match line.line_type {
                                        crate::app::DiffLineType::Addition
                                        | crate::app::DiffLineType::Deletion
                                        | crate::app::DiffLineType::Normal => true,
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
                            KeyCode::Char('p') => {
                                app.in_review_mode = !app.in_review_mode;
                                app.status_message = Some(format!(
                                    "Review mode: {}. ({} pending comments)",
                                    if app.in_review_mode { "ON" } else { "OFF" },
                                    app.draft_comments.len()
                                ));
                                app.diff_view = Some(diff_view);
                            }
                            KeyCode::Char('r') => {
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
                                    entity_iid: diff_view.mr_iid,
                                    entity_type: "mr".to_string(),
                                    field_type: "review_submit_status".to_string(),
                                    multi_select: false,
                                    state: ListState::default(),
                                });
                                app.diff_view = Some(diff_view);
                            }
                            _ => {
                                app.diff_view = Some(diff_view);
                            }
                        }
                        continue;
                    }

                    if app.focus_column_checklist {
                        match key_event.code {
                            KeyCode::Esc | KeyCode::Char('t') | KeyCode::Tab | KeyCode::BackTab => {
                                app.focus_column_checklist = false;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                let len = app.active_tab.columns().len();
                                let max_idx = if len > 0 { len - 1 } else { 0 };
                                if app.column_checklist_idx < max_idx {
                                    app.column_checklist_idx += 1;
                                } else {
                                    app.column_checklist_idx = 0;
                                }
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                let len = app.active_tab.columns().len();
                                let max_idx = if len > 0 { len - 1 } else { 0 };
                                if app.column_checklist_idx > 0 {
                                    app.column_checklist_idx -= 1;
                                } else {
                                    app.column_checklist_idx = max_idx;
                                }
                            }
                            KeyCode::Char(' ') | KeyCode::Enter => {
                                let cols = app.active_tab.columns();
                                if let Some(col_name) = cols.get(app.column_checklist_idx) {
                                    let col_str = col_name.to_string();
                                    if let Some(set) = app.enabled_columns.get_mut(&app.active_tab)
                                    {
                                        if set.contains(&col_str) {
                                            set.remove(&col_str);
                                        } else {
                                            set.insert(col_str);
                                        }
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

                    if (key_event.code == KeyCode::Tab
                        || key_event.code == KeyCode::BackTab
                        || key_event.code == KeyCode::Char('t'))
                        && !app.focus_column_checklist
                    {
                        app.focus_column_checklist = true;
                        app.column_checklist_idx = 0;
                        continue;
                    }

                    let mut handled = true;
                    match app.active_tab {
                        app::Tab::Issues => match key_event.code {
                            KeyCode::Char('n') => {
                                app.edit_menu = Some(crate::app::EditMenu {
                                    title: "Create Issue".to_string(),
                                    fields: vec![
                                        ("Title".to_string(), String::new()),
                                        ("Labels".to_string(), String::new()),
                                        ("Assignees".to_string(), String::new()),
                                        ("Milestone".to_string(), String::new()),
                                        ("Confidential".to_string(), "No".to_string()),
                                        ("Due Date".to_string(), String::new()),
                                        ("Weight".to_string(), "0".to_string()),
                                        (
                                            "Description".to_string(),
                                            get_default_template("issue").unwrap_or_default(),
                                        ),
                                    ],
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
                            KeyCode::Char('e') => {
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
                                        app.edit_menu = Some(crate::app::EditMenu {
                                            title: format!("Edit Issue #{}", issue.iid),
                                            fields: vec![
                                                ("Title".to_string(), issue.title.clone()),
                                                ("Labels".to_string(), labels),
                                                ("Assignees".to_string(), assignees),
                                                ("Milestone".to_string(), milestone),
                                                (
                                                    "Confidential".to_string(),
                                                    "Toggle/Set".to_string(),
                                                ),
                                                ("Due Date".to_string(), "Set".to_string()),
                                                ("Weight".to_string(), "Set".to_string()),
                                                ("Description".to_string(), "(Helix)".to_string()),
                                            ],
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
                            KeyCode::Char('c') => {
                                if let Some(selected_idx) = app.issues.state.selected() {
                                    let filtered = app.filtered_issues();
                                    if let Some(issue) = filtered.get(selected_idx) {
                                        let issue_iid = issue.iid;
                                        run_glab_cmd(
                                            &["issue", "close", &issue_iid.to_string()],
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
                            _ => handled = false,
                        },
                        app::Tab::MergeRequests => {
                            if key_event.code == KeyCode::Char('n') {
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
                                    is_filtering: true,
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
                                            false,
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
                                        KeyCode::Char('e') => {
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
                                        KeyCode::Char('a') => {
                                            run_glab_cmd(
                                                &["mr", "approve", &mr_iid.to_string()],
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                        }
                                        KeyCode::Char('m') => {
                                            run_glab_cmd(
                                                &[
                                                    "mr",
                                                    "merge",
                                                    &mr_iid.to_string(),
                                                    "--remove-source-branch",
                                                    "--squash",
                                                ],
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
                                        KeyCode::Char('v') => {
                                            app.diff_loading = true;
                                            let tx = events.sender();
                                            let mr_iid = mr_iid;
                                            let mr_iid_str = mr_iid.to_string();
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

                                                let cmd_args = vec!["mr", "diff", &mr_iid_str];
                                                let program = if is_github { "gh" } else { "glab" };
                                                let status_msg = format!(
                                                    "Fetching Diff: {} {}",
                                                    program,
                                                    cmd_args.join(" ")
                                                );
                                                let _ = tx.send(Event::CommandStarted(status_msg));

                                                let mut cmd = if is_github {
                                                    let gh_args = translate_glab_to_gh(&cmd_args);
                                                    let mut c = tokio::process::Command::new("gh");
                                                    c.args(gh_args);
                                                    c
                                                } else {
                                                    let mut c =
                                                        tokio::process::Command::new("glab");
                                                    c.args(&cmd_args);
                                                    c
                                                };

                                                match cmd.output().await {
                                                    Ok(output) => {
                                                        if output.status.success() {
                                                            let raw_diff = String::from_utf8_lossy(
                                                                &output.stdout,
                                                            )
                                                            .into_owned();
                                                            let _ = tx.send(Event::DiffFetched(
                                                                mr_iid, raw_diff,
                                                            ));
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
                                            run_glab_cmd(
                                                &["mr", "view", &mr_iid.to_string(), "-w"],
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                        }
                                        KeyCode::Char('s') => {
                                            let is_draft = mr_title.starts_with("Draft:")
                                                || mr_title.starts_with("WIP:");
                                            let action =
                                                if is_draft { "--ready" } else { "--draft" };
                                            run_glab_update(
                                                "mr",
                                                mr_iid,
                                                &[action],
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
                            } else if key_event.code == KeyCode::Char('p') {
                                run_glab_cmd(
                                    &["ci", "run", "--mr"],
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
                                        KeyCode::Char('r') => {
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
                                                            tx,
                                                            false,
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
                                                            false,
                                                        );
                                                    });
                                                }
                                            }
                                        }
                                        KeyCode::Char('d') => {
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
                                                        false,
                                                    );
                                                });
                                            }
                                        }
                                        KeyCode::Char('o') => {
                                            run_glab_cmd(
                                                &["ci", "view", &pipe_id.to_string(), "-w"],
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await
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
                                                        for j_id in job_ids {
                                                            let endpoint = format!(
                                                                "projects/{}/jobs/{}/cancel",
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
                                                            j.status = "canceled".to_string();
                                                        }
                                                    }
                                                    tokio::spawn(async move {
                                                        let endpoint = format!(
                                                            "projects/{}/jobs/{}/cancel",
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
                                        KeyCode::Char('d') => {
                                            run_glab_cmd(
                                                &["job", "artifact", "master", &job_name],
                                                &mut terminal,
                                                events.sender(),
                                                app.active_tab,
                                            )
                                            .await;
                                        }
                                        KeyCode::Char('o') => {
                                            run_glab_cmd(
                                                &["job", "view", &job_id.to_string(), "-w"],
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
                                            let mut cmd = if cfg!(target_os = "windows") {
                                                let mut c = std::process::Command::new("cmd");
                                                c.args(&[
                                                    "/c",
                                                    &format!(
                                                        "{} \"{}\"",
                                                        editor,
                                                        temp_file.to_string_lossy()
                                                    ),
                                                ]);
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
                                                if let Ok(trace) = gitlab::pipelines::get_job_trace(
                                                    client,
                                                    &app.project_context,
                                                    job_id,
                                                )
                                                .await
                                                {
                                                    app.job_trace = Some(trace);
                                                    app.job_trace_needs_scroll_to_bottom = true;
                                                    app.details_zoomed = true;
                                                } else {
                                                    app.error_message = Some(
                                                        "Failed to fetch job trace".to_string(),
                                                    );
                                                }
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
                                            run_glab_cmd(
                                                &[
                                                    "api",
                                                    "-X",
                                                    "PUT",
                                                    &format!("runners/{}", runner_id),
                                                    "-f",
                                                    "paused=true",
                                                ],
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
                                            run_glab_cmd(
                                                &[
                                                    "api",
                                                    "-X",
                                                    "PUT",
                                                    &format!("runners/{}", runner_id),
                                                    "-f",
                                                    "paused=false",
                                                ],
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
                        app::Tab::Releases => {
                            if let Some(selected_idx) = app.releases.state.selected() {
                                if let Some(item) = app.filtered_releases().get(selected_idx) {
                                    match key_event.code {
                                        KeyCode::Char('o') => {
                                            run_glab_cmd(
                                                &["release", "view", &item.tag_name, "-w"],
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
                        app::Tab::Notifications => {
                            if let Some(selected_idx) = app.notifications.state.selected() {
                                if let Some(item) = app.filtered_notifications().get(selected_idx) {
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
                            handled = false;
                        }
                        app::Tab::Wiki => {
                            handled = false;
                        }
                        app::Tab::Terminal => {
                            handled = false;
                        }
                    }

                    if !handled {
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
                                                app::Tab::Notifications,
                                                "Update complete! Please restart glab-tui."
                                                    .to_string(),
                                            ));
                                        }
                                        Ok(false) => {
                                            let _ = tx.send(Event::FetchFailed(
                                                app::Tab::Notifications,
                                                "Already up to date.".to_string(),
                                            ));
                                        }
                                        Err(e) => {
                                            let _ = tx.send(Event::FetchFailed(
                                                app::Tab::Notifications,
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
                            KeyCode::Char('J') => match app.active_tab {
                                app::Tab::Issues => {
                                    app.issues_scroll = app.issues_scroll.saturating_add(1);
                                }
                                app::Tab::MergeRequests => {
                                    app.mrs_scroll = app.mrs_scroll.saturating_add(1);
                                }
                                _ => {}
                            },
                            KeyCode::Char('K') => match app.active_tab {
                                app::Tab::Issues => {
                                    app.issues_scroll = app.issues_scroll.saturating_sub(1);
                                }
                                app::Tab::MergeRequests => {
                                    app.mrs_scroll = app.mrs_scroll.saturating_sub(1);
                                }
                                _ => {}
                            },
                            KeyCode::Esc | KeyCode::Backspace => {
                                if app.details_zoomed {
                                    app.details_zoomed = false;
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
                                app::Tab::Notifications => {
                                    if let Some(idx) = app.notifications.state.selected() {
                                        if let Some(n) = app.filtered_notifications().get(idx) {
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
                                                if let Ok(trace) = gitlab::pipelines::get_job_trace(
                                                    client,
                                                    &app.project_context,
                                                    job_id,
                                                )
                                                .await
                                                {
                                                    app.job_trace = Some(trace);
                                                    app.job_trace_needs_scroll_to_bottom = true;
                                                    app.details_zoomed = true;
                                                } else {
                                                    app.error_message = Some(
                                                        "Failed to fetch job trace".to_string(),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                                _ => {
                                    app.details_zoomed = !app.details_zoomed;
                                }
                            },
                            KeyCode::Right | KeyCode::Char('l') => {
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
                                            app.is_column_visible(
                                                app.active_tab,
                                                "Show Closed Items",
                                            ),
                                        );
                                    }
                                }
                            }
                            KeyCode::Left | KeyCode::Char('h') => {
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
                                            app.is_column_visible(
                                                app.active_tab,
                                                "Show Closed Items",
                                            ),
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
                                app::Tab::Notifications => {
                                    app.notifications.next(app.filtered_notifications().len())
                                }
                                app::Tab::Milestones => {
                                    app.milestones.next(app.filtered_milestones().len())
                                }
                                app::Tab::Wiki => app.wiki_pages.next(app.filtered_wiki().len()),
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
                                app::Tab::Notifications => app
                                    .notifications
                                    .previous(app.filtered_notifications().len()),
                                app::Tab::Milestones => {
                                    app.milestones.previous(app.filtered_milestones().len())
                                }
                                app::Tab::Wiki => {
                                    app.wiki_pages.previous(app.filtered_wiki().len())
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
    fn test_translate_glab_to_gh_issue_close() {
        let glab_args = vec!["issue", "close", "123"];
        let gh_args = translate_glab_to_gh(&glab_args);
        assert_eq!(
            gh_args,
            vec!["issue".to_string(), "close".to_string(), "123".to_string()]
        );
    }

    #[test]
    fn test_translate_glab_to_gh_issue_create() {
        let glab_args = vec!["issue", "create", "--title", "Bug report"];
        let gh_args = translate_glab_to_gh(&glab_args);
        assert_eq!(
            gh_args,
            vec![
                "issue".to_string(),
                "create".to_string(),
                "--title".to_string(),
                "Bug report".to_string(),
                "--body".to_string(),
                "".to_string()
            ]
        );
    }

    #[test]
    fn test_translate_glab_to_gh_mr_create_with_issue() {
        let glab_args = vec!["mr", "create", "-i", "123", "--copy-issue-labels"];
        let gh_args = translate_glab_to_gh(&glab_args);
        assert_eq!(
            gh_args,
            vec![
                "pr".to_string(),
                "create".to_string(),
                "--fill".to_string(),
                "--body".to_string(),
                "Resolves #123".to_string()
            ]
        );
    }

    #[test]
    fn test_translate_glab_to_gh_mr_create_with_issue_and_body() {
        let glab_args = vec![
            "mr",
            "create",
            "-i",
            "123",
            "--title",
            "Pre-filled title",
            "--description",
            "This is the user edited description body.",
        ];
        let gh_args = translate_glab_to_gh(&glab_args);
        assert_eq!(
            gh_args,
            vec![
                "pr".to_string(),
                "create".to_string(),
                "--title".to_string(),
                "Pre-filled title".to_string(),
                "--body".to_string(),
                "Resolves #123\n\nThis is the user edited description body.".to_string()
            ]
        );
    }

    #[test]
    fn test_translate_glab_to_gh_release_create() {
        let glab_args = vec!["release", "create", "v1.0.0", "-F", "changelog.md"];
        let gh_args = translate_glab_to_gh(&glab_args);
        assert_eq!(
            gh_args,
            vec![
                "release".to_string(),
                "create".to_string(),
                "v1.0.0".to_string(),
                "-F".to_string(),
                "changelog.md".to_string()
            ]
        );
    }

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

    #[test]
    fn test_translate_glab_to_gh_ci_run() {
        let glab_args = vec![
            "ci",
            "run",
            "my-workflow.yml",
            "-b",
            "my-branch",
            "--variables",
            "var1:val1",
            "-i",
            "inp1:val1",
        ];
        let gh_args = translate_glab_to_gh(&glab_args);
        assert_eq!(
            gh_args,
            vec![
                "workflow".to_string(),
                "run".to_string(),
                "my-workflow.yml".to_string(),
                "-r".to_string(),
                "my-branch".to_string(),
                "-f".to_string(),
                "var1=val1".to_string(),
                "-f".to_string(),
                "inp1=val1".to_string(),
            ]
        );
    }

    #[test]
    fn test_translate_glab_to_gh_mr_update_description() {
        let glab_args = vec!["mr", "update", "123", "-d", "new description text"];
        let gh_args = translate_glab_to_gh(&glab_args);
        assert_eq!(
            gh_args,
            vec![
                "pr".to_string(),
                "edit".to_string(),
                "123".to_string(),
                "--body".to_string(),
                "new description text".to_string()
            ]
        );
    }

    #[test]
    fn test_translate_glab_to_gh_issue_update_description() {
        let glab_args = vec!["issue", "update", "123", "-d", "new description text"];
        let gh_args = translate_glab_to_gh(&glab_args);
        assert_eq!(
            gh_args,
            vec![
                "issue".to_string(),
                "edit".to_string(),
                "123".to_string(),
                "--body".to_string(),
                "new description text".to_string()
            ]
        );
    }
}
