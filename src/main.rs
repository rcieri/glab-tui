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
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Write};

fn prompt_user(prompt: &str) -> Option<String> {
    crate::event::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
    disable_raw_mode().unwrap();
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture).unwrap();
    
    print!("{}", prompt);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    let res = if io::stdin().read_line(&mut input).is_ok() {
        let input = input.trim().to_string();
        enable_raw_mode().unwrap();
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture).unwrap();
        if input.is_empty() { None } else { Some(input) }
    } else {
        enable_raw_mode().unwrap();
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture).unwrap();
        None
    };
    crate::event::PAUSED.store(false, std::sync::atomic::Ordering::Relaxed);
    res
}

async fn run_glab_cmd(args: &[&str]) {
    crate::event::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
    disable_raw_mode().unwrap();
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture).unwrap();
    
    let mut cmd = std::process::Command::new("glab");
    for arg in args {
        cmd.arg(arg);
    }
    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());
    
    if let Ok(mut child) = cmd.spawn() {
        let _ = child.wait();
    }
    
    enable_raw_mode().unwrap();
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture).unwrap();
    crate::event::PAUSED.store(false, std::sync::atomic::Ordering::Relaxed);
}

async fn run_glab_update(entity_type: &str, id: u64, args: &[&str]) {
    let id_str = id.to_string();
    let mut cmd_args = vec![entity_type, "update", &id_str];
    cmd_args.extend_from_slice(args);
    run_glab_cmd(&cmd_args).await;
}

async fn handle_entity_update(app: &mut App, entity_type: &str, iid: u64, code: KeyCode, terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>) {
    match code {
        KeyCode::Char('t') => {
            if let Some(new_title) = prompt_user("Enter new title: ") {
                run_glab_update(entity_type, iid, &["--title", &new_title]).await;
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
        KeyCode::Char('l') => {
            if let Some(new_labels) = prompt_user("Enter labels (comma separated, or 'none' to clear): ") {
                let labels_vec = if new_labels.to_lowercase() == "none" {
                    run_glab_update(entity_type, iid, &["--unlabel", "all"]).await;
                    vec![]
                } else {
                    run_glab_update(entity_type, iid, &["--label", &new_labels]).await;
                    new_labels.split(',').map(|s| s.trim().to_string()).collect()
                };
                if entity_type == "issue" {
                    if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                        item.labels = labels_vec;
                    }
                } else if entity_type == "mr" {
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.labels = labels_vec;
                    }
                }
            }
        }
        KeyCode::Char('a') => {
            if let Some(assignee) = prompt_user("Enter assignee username: ") {
                run_glab_update(entity_type, iid, &["--assignee", &assignee]).await;
            }
        }
        KeyCode::Char('d') => {
            run_glab_update(entity_type, iid, &["--description", "-"]).await;
            terminal.clear().unwrap();
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
                if let Ok(issues) = gitlab::issues::list_issues(&client, &project_context).await {
                    let _ = tx.send(Event::IssuesFetched(issues));
                }
            }
            app::Tab::MergeRequests => {
                if let Ok(mrs) = gitlab::mr::list_mrs(&client, &project_context).await {
                    let _ = tx.send(Event::MrsFetched(mrs));
                }
            }
            app::Tab::Pipelines => {
                if let Ok(pipelines) = gitlab::pipelines::list_pipelines(&client, &project_context).await {
                    let _ = tx.send(Event::PipelinesFetched(pipelines));
                }
            }
            app::Tab::Runners => {
                if let Ok(runners) = gitlab::runners::list_runners(&client, &project_context).await {
                    let _ = tx.send(Event::RunnersFetched(runners));
                }
            }
            app::Tab::Releases => {
                if let Ok(releases) = gitlab::releases::list_releases(&client, &project_context).await {
                    let _ = tx.send(Event::ReleasesFetched(releases));
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
        for tab in &app::Tab::ALL {
            app.loading_tabs.insert(*tab);
        }
        let tx = events.sender();
        let project_context = app.project_context.clone();

        // Spawn parallel background requests
        let client_c = client.clone();
        let project_c = project_context.clone();
        let tx_c = tx.clone();
        tokio::spawn(async move {
            if let Ok(issues) = gitlab::issues::list_issues(&client_c, &project_c).await {
                let _ = tx_c.send(Event::IssuesFetched(issues));
            }
        });

        let client_c = client.clone();
        let project_c = project_context.clone();
        let tx_c = tx.clone();
        tokio::spawn(async move {
            if let Ok(mrs) = gitlab::mr::list_mrs(&client_c, &project_c).await {
                let _ = tx_c.send(Event::MrsFetched(mrs));
            }
        });

        let client_c = client.clone();
        let project_c = project_context.clone();
        let tx_c = tx.clone();
        tokio::spawn(async move {
            if let Ok(pipelines) = gitlab::pipelines::list_pipelines(&client_c, &project_c).await {
                let _ = tx_c.send(Event::PipelinesFetched(pipelines));
            }
        });

        let client_c = client.clone();
        let project_c = project_context.clone();
        let tx_c = tx.clone();
        tokio::spawn(async move {
            if let Ok(runners) = gitlab::runners::list_runners(&client_c, &project_c).await {
                let _ = tx_c.send(Event::RunnersFetched(runners));
            }
        });

        let client_c = client.clone();
        let project_c = project_context.clone();
        let tx_c = tx.clone();
        tokio::spawn(async move {
            if let Ok(releases) = gitlab::releases::list_releases(&client_c, &project_c).await {
                let _ = tx_c.send(Event::ReleasesFetched(releases));
            }
        });
    } else {
        app.error_message = Some("Failed to initialize GitLab client".to_string());
    }

    // Run app
    while app.running {
        if app.active_tab == app::Tab::Pipelines {
            if let Some(client) = &app.gitlab_client {
                if let Some(idx) = app.pipelines.state.selected() {
                    if let Some(p) = app.pipelines.items.get(idx) {
                        if !app.pipeline_jobs.contains_key(&p.id) && !app.fetching_pipelines.contains(&p.id) {
                            app.fetching_pipelines.insert(p.id);
                            let client_clone = client.clone();
                            let project_context = app.project_context.clone();
                            let tx = events.sender();
                            let pipe_id = p.id;
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
                    app.pipeline_jobs.insert(id, jobs);
                }
                Event::IssuesFetched(issues) => {
                    app.loading_tabs.remove(&app::Tab::Issues);
                    let old_selected = app.issues.state.selected();
                    app.issues.items = issues;
                    if !app.issues.items.is_empty() {
                        let new_selected = old_selected.map(|idx| idx.min(app.issues.items.len() - 1)).unwrap_or(0);
                        app.issues.state.select(Some(new_selected));
                    } else {
                        app.issues.state.select(None);
                    }
                }
                Event::MrsFetched(mrs) => {
                    app.loading_tabs.remove(&app::Tab::MergeRequests);
                    let old_selected = app.mrs.state.selected();
                    app.mrs.items = mrs;
                    if !app.mrs.items.is_empty() {
                        let new_selected = old_selected.map(|idx| idx.min(app.mrs.items.len() - 1)).unwrap_or(0);
                        app.mrs.state.select(Some(new_selected));
                    } else {
                        app.mrs.state.select(None);
                    }
                }
                Event::PipelinesFetched(pipelines) => {
                    app.loading_tabs.remove(&app::Tab::Pipelines);
                    let old_selected = app.pipelines.state.selected();
                    app.pipelines.items = pipelines;
                    if !app.pipelines.items.is_empty() {
                        let new_selected = old_selected.map(|idx| idx.min(app.pipelines.items.len() - 1)).unwrap_or(0);
                        app.pipelines.state.select(Some(new_selected));
                    } else {
                        app.pipelines.state.select(None);
                    }
                    app.pipeline_jobs.clear();
                    app.fetching_pipelines.clear();
                }
                Event::RunnersFetched(runners) => {
                    app.loading_tabs.remove(&app::Tab::Runners);
                    let old_selected = app.runners.state.selected();
                    app.runners.items = runners;
                    if !app.runners.items.is_empty() {
                        let new_selected = old_selected.map(|idx| idx.min(app.runners.items.len() - 1)).unwrap_or(0);
                        app.runners.state.select(Some(new_selected));
                    } else {
                        app.runners.state.select(None);
                    }
                }
                Event::ReleasesFetched(releases) => {
                    app.loading_tabs.remove(&app::Tab::Releases);
                    let old_selected = app.releases.state.selected();
                    app.releases.items = releases;
                    if !app.releases.items.is_empty() {
                        let new_selected = old_selected.map(|idx| idx.min(app.releases.items.len() - 1)).unwrap_or(0);
                        app.releases.state.select(Some(new_selected));
                    } else {
                        app.releases.state.select(None);
                    }
                }
                Event::Key(key_event) => {
                    if app.error_message.is_some() {
                        if key_event.code == KeyCode::Enter || key_event.code == KeyCode::Esc {
                            app.error_message = None;
                        }
                        continue;
                    }

                    if app.is_typing_search {
                        match key_event.code {
                            KeyCode::Enter | KeyCode::Esc => app.is_typing_search = false,
                            KeyCode::Backspace => {
                                app.search_query.pop();
                            }
                            KeyCode::Char(c) => {
                                app.search_query.push(c);
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if key_event.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
                        match app.active_tab {
                            app::Tab::Issues => {
                                if key_event.code == KeyCode::Char('n') {
                                    if let Some(title) = prompt_user("Enter issue title: ") {
                                        run_glab_cmd(&["issue", "create", "-y", "--title", &title]).await;
                                    }
                                } else if let Some(selected_idx) = app.issues.state.selected() {
                                    let issue_iid = app.issues.items.get(selected_idx).map(|item| item.iid);
                                    if let Some(iid) = issue_iid {
                                        handle_entity_update(&mut app, "issue", iid, key_event.code, &mut terminal).await;
                                    }
                                }
                            }
                            app::Tab::MergeRequests => {
                                if key_event.code == KeyCode::Char('n') {
                                    if let Some(issue_id) = prompt_user("Enter issue ID for new MR: ") {
                                        run_glab_cmd(&["mr", "create", "-i", &issue_id, "--copy-issue-labels", "--create-source-branch", "--squash-before-merge"]).await;
                                    }
                                } else if let Some(selected_idx) = app.mrs.state.selected() {
                                    let mr_info = app.mrs.items.get(selected_idx).map(|item| (item.iid, item.title.clone()));
                                    if let Some((mr_iid, mr_title)) = mr_info {
                                        match key_event.code {
                                            KeyCode::Char('a') => {
                                                run_glab_cmd(&["mr", "approve", &mr_iid.to_string()]).await;
                                            }
                                            KeyCode::Char('m') => {
                                                run_glab_cmd(&["mr", "merge", &mr_iid.to_string(), "--remove-source-branch", "--squash"]).await;
                                                app.mrs.items.remove(selected_idx);
                                                if app.mrs.items.is_empty() {
                                                    app.mrs.state.select(None);
                                                } else {
                                                    let new_sel = selected_idx.min(app.mrs.items.len() - 1);
                                                    app.mrs.state.select(Some(new_sel));
                                                }
                                            }
                                            KeyCode::Char('f') => {
                                                run_glab_cmd(&["mr", "diff", &mr_iid.to_string()]).await;
                                            }
                                            KeyCode::Char('o') => {
                                                run_glab_cmd(&["mr", "view", &mr_iid.to_string(), "-w"]).await;
                                            }
                                            KeyCode::Char('s') => {
                                                let is_draft = mr_title.starts_with("Draft:") || mr_title.starts_with("WIP:");
                                                let action = if is_draft { "--ready" } else { "--draft" };
                                                run_glab_update("mr", mr_iid, &[action]).await;
                                                if let Some(mr) = app.mrs.items.iter_mut().find(|m| m.iid == mr_iid) {
                                                    if is_draft {
                                                        if mr.title.starts_with("Draft: ") {
                                                            mr.title = mr.title.replacen("Draft: ", "", 1);
                                                        } else if mr.title.starts_with("Draft:") {
                                                            mr.title = mr.title.replacen("Draft:", "", 1);
                                                        } else if mr.title.starts_with("WIP:") {
                                                            mr.title = mr.title.replacen("WIP:", "", 1);
                                                        }
                                                    } else {
                                                        mr.title = format!("Draft: {}", mr.title);
                                                    }
                                                }
                                            }
                                            _ => handle_entity_update(&mut app, "mr", mr_iid, key_event.code, &mut terminal).await,
                                        }
                                    }
                                }
                            }
                            app::Tab::Pipelines => {
                                if key_event.code == KeyCode::Char('p') {
                                    run_glab_cmd(&["ci", "run", "--mr"]).await;
                                } else if let Some(jobs) = &app.selected_pipeline_jobs {
                                    if let Some(idx) = app.selected_job_index {
                                        if let Some(job) = jobs.get(idx) {
                                            match key_event.code {
                                                KeyCode::Char('r') => {
                                                    let endpoint = format!("projects/{}/jobs/{}/retry", app.project_context.replace("/", "%2F"), job.id);
                                                    if let Some(client) = &app.gitlab_client {
                                                        let _ = client.fetch_raw_api(&endpoint).await;
                                                    }
                                                }
                                                KeyCode::Char('d') => {
                                                    run_glab_cmd(&["job", "artifact", "master", &job.name]).await;
                                                }
                                                KeyCode::Char('o') => {
                                                    run_glab_cmd(&["job", "view", &job.id.to_string(), "-w"]).await;
                                                }
                                                KeyCode::Char('e') => {
                                                    let temp_file = std::env::temp_dir().join(format!("job_{}_trace.txt", job.id));
                                                    if let Some(trace) = &app.job_trace {
                                                        let _ = std::fs::write(&temp_file, trace);
                                                    } else if let Some(_) = &app.gitlab_client {
                                                        let _ = std::fs::write(&temp_file, "Trace will be here");
                                                    }
                                                    crate::event::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
                                                    disable_raw_mode().unwrap();
                                                    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture).unwrap();
                                                    let mut cmd = std::process::Command::new("hx");
                                                    cmd.arg(&temp_file);
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
                                                _ => {}
                                            }
                                        }
                                    }
                                } else if let Some(selected_idx) = app.pipelines.state.selected() {
                                    if let Some(item) = app.pipelines.items.get(selected_idx) {
                                        let pipe_id = item.id;
                                        match key_event.code {
                                            KeyCode::Char('r') => {
                                                let endpoint = format!("projects/{}/pipelines/{}/retry", app.project_context.replace("/", "%2F"), pipe_id);
                                                if let Some(client) = &app.gitlab_client {
                                                    let _ = client.fetch_raw_api(&endpoint).await;
                                                }
                                                if let Some(p) = app.pipelines.items.iter_mut().find(|pipe| pipe.id == pipe_id) {
                                                    p.status = "running".to_string();
                                                }
                                            }
                                            KeyCode::Char('d') => {
                                                let endpoint = format!("projects/{}/pipelines/{}/cancel", app.project_context.replace("/", "%2F"), pipe_id);
                                                if let Some(client) = &app.gitlab_client {
                                                    let _ = client.fetch_raw_api(&endpoint).await;
                                                }
                                                if let Some(p) = app.pipelines.items.iter_mut().find(|pipe| pipe.id == pipe_id) {
                                                    p.status = "canceled".to_string();
                                                }
                                            }
                                            KeyCode::Char('o') => run_glab_cmd(&["ci", "view", &pipe_id.to_string(), "-w"]).await,
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            app::Tab::Runners => {
                                if let Some(selected_idx) = app.runners.state.selected() {
                                    if let Some(item) = app.runners.items.get(selected_idx) {
                                        let runner_id = item.id;
                                        match key_event.code {
                                            KeyCode::Char('p') => {
                                                run_glab_cmd(&["api", "-X", "PUT", &format!("runners/{}", runner_id), "-f", "paused=true"]).await;
                                                if let Some(runner) = app.runners.items.iter_mut().find(|r| r.id == runner_id) {
                                                    runner.status = "paused".to_string();
                                                    runner.active = false;
                                                }
                                            }
                                            KeyCode::Char('r') => {
                                                run_glab_cmd(&["api", "-X", "PUT", &format!("runners/{}", runner_id), "-f", "paused=false"]).await;
                                                if let Some(runner) = app.runners.items.iter_mut().find(|r| r.id == runner_id) {
                                                    runner.status = "online".to_string();
                                                    runner.active = true;
                                                }
                                            }
                                            KeyCode::Char('e') => {
                                                if let Some(desc) = prompt_user("Enter new description: ") {
                                                    run_glab_cmd(&["api", "-X", "PUT", &format!("runners/{}", runner_id), "-f", &format!("description={}", desc)]).await;
                                                    if let Some(runner) = app.runners.items.iter_mut().find(|r| r.id == runner_id) {
                                                        runner.description = Some(desc);
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            app::Tab::Releases => {
                                if let Some(item) = app.releases.state.selected().and_then(|i| app.releases.items.get(i)) {
                                    if key_event.code == KeyCode::Char('o') {
                                        run_glab_cmd(&["release", "view", &item.tag_name, "-w"]).await;
                                    }
                                }
                            }
                        }
                        if let Some(client) = &app.gitlab_client {
                            let client_clone = client.clone();
                            let project_context = app.project_context.clone();
                            let active_tab = app.active_tab;
                            let tx = events.sender();
                            app.loading_tabs.insert(active_tab);
                            tokio::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                                spawn_refresh_active_tab(&client_clone, &project_context, active_tab, tx);
                            });
                        }
                        continue;
                    }

                    match key_event.code {
                        KeyCode::F(5) => {
                            if let Some(client) = &app.gitlab_client {
                                if !app.loading_tabs.contains(&app.active_tab) {
                                    app.loading_tabs.insert(app.active_tab);
                                    spawn_refresh_active_tab(client, &app.project_context, app.active_tab, events.sender());
                                }
                            }
                        }
                        KeyCode::Char('q') => app.quit(),
                        KeyCode::Esc | KeyCode::Backspace => {
                            if app.active_tab == app::Tab::Pipelines && app.selected_pipeline_jobs.is_some() {
                                if app.job_trace.is_some() {
                                    app.job_trace = None;
                                } else {
                                    app.selected_pipeline_jobs = None;
                                    app.selected_job_index = None;
                                }
                            } else {
                                app.quit();
                            }
                        }
                        KeyCode::Char('/') => {
                            app.is_typing_search = true;
                        }
                        KeyCode::Enter => {
                            match app.active_tab {
                                app::Tab::Pipelines => {
                                    if let Some(jobs) = &app.selected_pipeline_jobs {
                                        if let Some(idx) = app.selected_job_index {
                                            if let Some(job) = jobs.get(idx) {
                                                if let Some(client) = &app.gitlab_client {
                                                    if let Ok(trace) = gitlab::pipelines::get_job_trace(client, &app.project_context, job.id).await {
                                                        app.job_trace = Some(trace);
                                                    } else {
                                                        app.error_message = Some("Failed to fetch job trace".to_string());
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        if let Some(idx) = app.pipelines.state.selected() {
                                            if let Some(p) = app.pipelines.items.get(idx) {
                                                if let Some(client) = &app.gitlab_client {
                                                    if let Ok(jobs) = gitlab::pipelines::list_pipeline_jobs(client, &app.project_context, p.id).await {
                                                        app.selected_pipeline_jobs = Some(jobs);
                                                        app.selected_job_index = Some(0);
                                                        app.job_trace = None;
                                                    } else {
                                                        app.error_message = Some("Failed to fetch jobs".to_string());
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                app::Tab::Releases => {
                                    if let Some(idx) = app.releases.state.selected() {
                                        if let Some(r) = app.releases.items.get(idx) {
                                            run_glab_cmd(&["release", "view", &r.tag_name]).await;
                                            if let Some(client) = &app.gitlab_client {
                                                app.loading_tabs.insert(app.active_tab);
                                                spawn_refresh_active_tab(client, &app.project_context, app.active_tab, events.sender());
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }

                        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                            if app.selected_pipeline_jobs.is_none() {
                                app.next_tab();
                            }
                        }
                        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                            if app.selected_pipeline_jobs.is_none() {
                                app.previous_tab();
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            match app.active_tab {
                                app::Tab::Issues => app.issues.next(app.issues.items.len()),
                                app::Tab::MergeRequests => app.mrs.next(app.mrs.items.len()),
                                app::Tab::Pipelines => {
                                    if let Some(jobs) = &app.selected_pipeline_jobs {
                                        if let Some(idx) = &mut app.selected_job_index {
                                            if *idx + 1 < jobs.len() {
                                                *idx += 1;
                                                app.job_trace = None;
                                            }
                                        }
                                    } else {
                                        app.pipelines.next(app.pipelines.items.len());
                                    }
                                }
                                app::Tab::Runners => app.runners.next(app.runners.items.len()),
                                app::Tab::Releases => app.releases.next(app.releases.items.len()),
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            match app.active_tab {
                                app::Tab::Issues => app.issues.previous(app.issues.items.len()),
                                app::Tab::MergeRequests => app.mrs.previous(app.mrs.items.len()),
                                app::Tab::Pipelines => {
                                    if app.selected_pipeline_jobs.is_some() {
                                        if let Some(idx) = &mut app.selected_job_index {
                                            if *idx > 0 {
                                                *idx -= 1;
                                                app.job_trace = None;
                                            }
                                        }
                                    } else {
                                        app.pipelines.previous(app.pipelines.items.len());
                                    }
                                }
                                app::Tab::Runners => app.runners.previous(app.runners.items.len()),
                                app::Tab::Releases => app.releases.previous(app.releases.items.len()),
                            }
                        }
                        _ => {}
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
