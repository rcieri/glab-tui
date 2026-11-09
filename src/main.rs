use glab_tui_crate::*;

use anyhow::Result;
use app::App;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use effect::Effect;
use event::{Event, EventHandler};
use ratatui::{Terminal, backend::CrosstermBackend, widgets::ListState};
use std::io;

/// Runs what `update` asked for, in order. A foreground job's outcome goes
/// back through `update` before the next effect runs. Returns `false` once an
/// effect quits.
fn run_effects(app: &mut App, terminal: &mut AppTerminal, effects: Vec<Effect>) -> bool {
    let mut queue = std::collections::VecDeque::from(effects);
    while let Some(effect) = queue.pop_front() {
        match effect {
            Effect::Spawn(task) => {
                tokio::spawn(task);
            }
            Effect::SpawnThread(job) => {
                std::thread::spawn(job);
            }
            Effect::Suspend(job) => {
                if let Some(event) = job.run(app, terminal) {
                    queue.extend(update::update(app, event));
                }
            }
            Effect::Quit => return false,
        }
    }
    true
}

#[tokio::main]
async fn main() -> Result<()> {
    use clap::Parser;

    // ── Subcommand dispatch ──
    let cli = cli::Cli::parse();

    if let Some(ref path) = cli.config {
        // Resolved before `--dir` changes the working directory.
        match std::fs::canonicalize(path) {
            Ok(file) if file.is_file() => crate::config::use_config_file(file),
            Ok(_) => {
                eprintln!("Error: --config '{}' is not a file", path.display());
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("Error: --config '{}': {}", path.display(), e);
                std::process::exit(1);
            }
        }
    }

    if let Some(dir) = &cli.dir {
        if let Err(e) = std::env::set_current_dir(dir) {
            eprintln!("Error changing directory to '{dir}': {e}");
            std::process::exit(1);
        }
    }

    if let Some(cmd) = cli.command {
        match cmd {
            cli::Commands::Doctor => {
                cli::run_doctor().await;
                return Ok(());
            }
            cli::Commands::CleanCache { dry_run } => {
                cli::run_clean_cache(dry_run);
                return Ok(());
            }
            cli::Commands::Cache => {
                cli::run_cache_list();
                return Ok(());
            }
            cli::Commands::Open { entity, id } => {
                cli::run_open_in_browser(&entity, &id).await;
                return Ok(());
            }
            cli::Commands::Repos => {
                cli::run_repos_list();
                return Ok(());
            }
            cli::Commands::Review { command } => {
                if cli.group.is_some() {
                    eprintln!("error: review works on one repository; pass --repo, not --group");
                    std::process::exit(2);
                }
                match cli::review::run(command, cli.repo).await {
                    Ok(output) => {
                        println!("{output:#}");
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!("error: {e:#}");
                        std::process::exit(1);
                    }
                }
            }
        }
    }

    if cli.update {
        cli::run_update().await;
        return Ok(());
    }

    let custom_repo = cli.repo;

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    crate::editor::try_push_keyboard_enhancement_flags(&mut stdout);
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and event handler
    let mut app = App::new();
    if let Some(ref tab_str) = cli.tab {
        match crate::app::Tab::from_str(tab_str) {
            Some(tab) => app.active_tab = tab,
            None => {
                eprintln!(
                    "Error: --tab '{tab_str}' is not a recognised tab.\nValid values: dashboard, issues, mrs, pr, pipelines, jobs, runners, releases, todos, milestones, branches, environments, terminal."
                );
                std::process::exit(2);
            }
        }
    }
    let mut events = EventHandler::new(250);
    app.tx = events.sender();

    // Initialize scope context
    if let Some(group) = cli.group {
        app.scope = crate::scope::Scope::Group(group.clone());
        crate::utils::cache::add_recent_group(&group);
    } else if let Some(repo) = custom_repo {
        app.scope = crate::scope::Scope::Repository(repo);
    } else if let Ok(context) = domain::client::get_project_context() {
        app.scope = crate::scope::Scope::Repository(context);
    }

    // Add current directory to recent repositories list
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(cwd_str) = cwd.to_str() {
            crate::utils::cache::add_recent_repo(cwd_str);
        }
    }

    // Load offline cache
    let cache = crate::utils::cache::load_cache(app.scope.as_str());
    app.project_cache = cache.clone();
    app.restore_cached_issues(cache.issues, cache.issues_state_query);
    app.rebuild_milestone_progress_cache();
    app.restore_cached_mrs(cache.mrs, cache.mrs_state_query);
    // workflow is #[serde(skip)] — cached rows arrive with it unset even
    // though the approval state it derives from was persisted and just
    // loaded above.
    crate::fetch::derive_workflow(&mut app.mrs.items);
    crate::fetch::sync_linked_references(&mut app.issues.items, &mut app.mrs.items);
    app.pipelines.items = cache.pipelines;
    app.runners.set_items(cache.runners);
    app.releases.set_items(cache.releases);
    app.todos.items = cache.todos;
    app.milestones.items = cache.milestones;
    app.pipeline_jobs = cache.pipeline_jobs;
    app.branches.set_items(cache.branches);
    app.environments.set_items(cache.environments);
    app.milestone_issues_cache = cache.milestone_issues;
    app.rebuild_milestone_progress_cache();
    app.cached_labels = cache.labels;
    if app.config.fetch_label_colors {
        app.label_colors = cache
            .label_colors
            .iter()
            .filter_map(|(name, hex)| crate::config::hex_to_color(hex).map(|c| (name.clone(), c)))
            .collect();
    }
    app.cached_members = cache.members;

    app.mark_cached_tabs_stale();

    if !app.issues.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Issues);
    }
    if !app.mrs.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::MergeRequests);
    }
    if !app.pipelines.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Pipelines);
    }
    if !app.runners.items().is_empty() {
        app.loaded_tabs.insert(app::Tab::Runners);
    }
    if !app.releases.items().is_empty() {
        app.loaded_tabs.insert(app::Tab::Releases);
    }
    if !app.todos.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Todos);
    }
    if !app.milestones.items.is_empty() {
        app.loaded_tabs.insert(app::Tab::Milestones);
    }
    if !app.branches.items().is_empty() {
        app.loaded_tabs.insert(app::Tab::Branches);
    }
    if !app.environments.items().is_empty() {
        app.loaded_tabs.insert(app::Tab::Environments);
    }
    app.update_filter_selection();

    let mut fx = effect::Effects::default();
    if let Ok(mut client) = domain::client::GitlabClient::new(&app.config).await {
        client.page_size = app.config.page_size;
        client.api_per_page = app.config.api_per_page_clamped();
        client.tx = Some(events.sender());
        app.gitlab_client = Some(client.clone());
        // Remember the group implied by the current repo scope so the
        // Switch view accumulates every group worked in, not just explicit
        // group switches. GitLab groups and GitHub orgs both qualify.
        if let crate::scope::Scope::Repository(r) = &app.scope {
            if let Some(group) = r.rsplit_once('/').map(|(g, _)| g.to_string()) {
                crate::utils::cache::add_recent_group(&group);
            }
        }
        let tx = app.scoped_sender(events.sender());
        if app.issues.items.is_empty() {
            app.start_loading_tab(app.active_tab);
        }
        spawn_refresh_active_tab(
            &mut fx,
            &client,
            &app.scope,
            app.requested_state_queries(),
            app.active_tab,
            tx.clone(),
        );
        // If configured, queue background fetches for remaining enabled tabs
        // so a quick tab-switch lands on already-populated data.
        if app.config.prefetch_tabs {
            spawn_refresh_all_tabs(&mut fx, &client, &app, tx.clone());
        }
        spawn_fetch_repo_attributes(&mut fx, &client.muted(), &app.scope, tx);
    } else {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        app.terminal_commands
            .push(crate::command_log::TerminalCommand {
                timestamp,
                command: "Initialization: gitlab client".to_string(),
                status: "Failed: Failed to initialize GitLab client".to_string(),
                ..Default::default()
            });
        app.show_error("Failed to initialize GitLab client".to_string());
    }

    // If we couldn't detect a valid project, prompt to select a cached repo
    if app.scope.as_str() == "unknown/unknown" || app.scope.as_str() == "group/repository" {
        let switchable = crate::utils::cache::get_switchable_repos();
        if !switchable.is_empty() {
            let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
            app.terminal_commands
                .push(crate::command_log::TerminalCommand {
                    timestamp,
                    command: "Startup: Not in a git repository".to_string(),
                    status: "Failed: No repo detected — select one below or press Esc".to_string(),
                    ..Default::default()
                });
            let mut switch_repo_paths = std::collections::HashMap::new();
            for entry in &switchable {
                switch_repo_paths.insert(entry.display.clone(), entry.absolute_path.clone());
            }
            app.switch_repo_paths = switch_repo_paths;
            app.switch_repo_groups = std::collections::HashSet::new();
            app.selector = Some(crate::app::Selector {
                title: " No Repo Detected — Select a Repository ".to_string(),
                all_items: switchable.iter().map(|e| e.display.clone()).collect(),
                selected_items: std::collections::HashSet::new(),
                cursor_idx: 0,
                search_query: String::new(),
                is_filtering: false,
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
                request: None,
            });
        }
    }

    let tx = app.tx.clone();
    fetch::prefetch_highlighted(&mut app, &tx, &mut fx);
    let mut effects = fx.into_vec();
    loop {
        if !run_effects(&mut app, &mut terminal, effects) {
            break;
        }
        app.sync_list_views();
        app.discard_stale_deployments();
        terminal.draw(|f| ui::render(f, &mut app))?;

        let event = loop {
            if let Some(press) = app.key_buffer.pop_ready() {
                break Some(Event::KeyPress(press));
            }
            match events
                .next()
                .await
                .and_then(|event| event.for_scope(app.scope_generation))
            {
                Some(Event::Key(key_event)) => app.receive_key(key_event),
                other => break other,
            }
        };
        effects = match event {
            Some(event) => update::update(&mut app, event),
            None => Vec::new(),
        };
    }

    // Restore terminal
    disable_raw_mode()?;
    crate::editor::try_pop_keyboard_enhancement_flags(terminal.backend_mut());
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
