use ratatui::{
    layout::{Constraint, Direction, Layout, Rect, Alignment},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph, Clear, Table, Row, Cell, BorderType},
    text::{Line, Span},
    Frame,
};

use crate::app::{App, Tab};
use crate::utils::format::{truncate, time_ago};

struct Theme {
    bg: Color,
    border: Color,
    border_focused: Color,
    header_fg: Color,
    highlight_bg: Color,
    text_normal: Color,
    text_muted: Color,
    
    // Status colors (Catppuccin Mocha themed)
    green: Color,      // success, open
    green_bg: Color,   // status pill bg
    red: Color,        // failed, closed
    red_bg: Color,
    blue: Color,       // running, active
    blue_bg: Color,
    yellow: Color,     // pending, warning
    yellow_bg: Color,
    purple: Color,     // merged, releases
    purple_bg: Color,
}

const THEME: Theme = Theme {
    bg: Color::Rgb(30, 30, 46),            // Catppuccin Mocha Base
    border: Color::Rgb(88, 91, 112),       // Surface1
    border_focused: Color::Rgb(137, 180, 250), // Blue
    header_fg: Color::Rgb(180, 190, 254),  // Lavender
    highlight_bg: Color::Rgb(49, 50, 68),  // Surface0
    text_normal: Color::Rgb(205, 214, 244),// Text
    text_muted: Color::Rgb(166, 173, 200), // Subtext0
    
    green: Color::Rgb(166, 227, 161),
    green_bg: Color::Rgb(34, 58, 38),
    red: Color::Rgb(243, 139, 168),
    red_bg: Color::Rgb(68, 34, 38),
    blue: Color::Rgb(137, 180, 250),
    blue_bg: Color::Rgb(34, 48, 68),
    yellow: Color::Rgb(249, 226, 175),
    yellow_bg: Color::Rgb(58, 54, 38),
    purple: Color::Rgb(203, 166, 247),
    purple_bg: Color::Rgb(48, 34, 58),
};

struct StageSummary {
    name: String,
    success: usize,
    total: usize,
    percent: usize,
    status: String,
}

fn get_stages_summary(jobs: &[crate::gitlab::pipelines::Job]) -> Vec<StageSummary> {
    let mut stage_names = Vec::new();
    let mut stage_jobs = std::collections::HashMap::new();
    for j in jobs {
        if !stage_names.contains(&j.stage) {
            stage_names.push(j.stage.clone());
        }
        stage_jobs.entry(j.stage.clone()).or_insert_with(Vec::new).push(j.status.clone());
    }

    let mut summaries = Vec::new();
    for stage in stage_names {
        if let Some(statuses) = stage_jobs.get(&stage) {
            let total = statuses.len();
            let success = statuses.iter().filter(|s| *s == "success").count();
            let percent = if total > 0 { (success * 100) / total } else { 0 };
            
            let stage_status = if statuses.iter().any(|s| s == "failed") {
                "failed".to_string()
            } else if statuses.iter().any(|s| s == "running") {
                "running".to_string()
            } else if statuses.iter().any(|s| s == "pending") {
                "pending".to_string()
            } else if statuses.iter().any(|s| s == "canceled") {
                "canceled".to_string()
            } else if statuses.iter().all(|s| s == "success") {
                "success".to_string()
            } else {
                "skipped".to_string()
            };

            summaries.push(StageSummary {
                name: stage,
                success,
                total,
                percent,
                status: stage_status,
            });
        }
    }
    summaries
}

fn get_stages_dots(jobs: &[crate::gitlab::pipelines::Job]) -> String {
    let summaries = get_stages_summary(jobs);
    let mut dots = String::new();
    for s in summaries {
        let dot = match s.status.as_str() {
            "success" => "🟢",
            "failed" => "🔴",
            "running" => "🔵",
            "canceled" => "⚫",
            "pending" => "🟡",
            _ => "⚪",
        };
        dots.push_str(dot);
    }
    dots
}

pub fn render(f: &mut Frame, app: &mut App) {
    let size = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(size);

    // Top: Title & Context
    let title_text = if app.is_typing_search {
        format!(" 🦊 GitLab TUI | {} | Search: {}_ ", app.project_context, app.search_query)
    } else if !app.search_query.is_empty() {
        format!(" 🦊 GitLab TUI | {} | Search: {} ", app.project_context, app.search_query)
    } else {
        format!(" 🦊 GitLab TUI | {} ", app.project_context)
    };

    let title = Paragraph::new(title_text)
        .style(Style::default().fg(THEME.border_focused).bg(THEME.bg).add_modifier(Modifier::BOLD))
        .block(Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(THEME.border_focused))
        );
    f.render_widget(title, chunks[0]);

    // Middle: Sidebar | Main Area | Preview Area
    let middle_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(15),
            Constraint::Percentage(55),
            Constraint::Percentage(30),
        ])
        .split(chunks[1]);

    // Sidebar: Tabs
    let sidebar_items: Vec<ListItem> = Tab::ALL
        .iter()
        .map(|t| {
            if *t == app.active_tab {
                ListItem::new(format!(" ❯ {} ", t.title()))
                    .style(Style::default().fg(THEME.purple).add_modifier(Modifier::BOLD))
            } else {
                ListItem::new(format!("   {} ", t.title()))
                    .style(Style::default().fg(THEME.text_muted))
            }
        })
        .collect();
    
    let sidebar = List::new(sidebar_items)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(THEME.border))
            .title(" Navigation ")
            .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD))
        );
    f.render_widget(sidebar, middle_chunks[0]);

    // Main Area Title
    let tab_title = if app.loading_tabs.contains(&app.active_tab) {
        format!(" {} (loading...) ", app.active_tab.title())
    } else {
        format!(" {} ", app.active_tab.title())
    };
    let main_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(THEME.border_focused))
        .title(tab_title)
        .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD));
    
    let sq = app.search_query.to_lowercase();
    let highlight_style = Style::default().bg(THEME.highlight_bg).fg(THEME.border_focused).add_modifier(Modifier::BOLD);
    let header_style = Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD);

    match app.active_tab {
        Tab::Issues => {
            if app.issues.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
                f.render_widget(Paragraph::new("\n\n Loading issues...").alignment(Alignment::Center).block(main_block.clone()).style(Style::default().fg(THEME.text_muted)), middle_chunks[1]);
                f.render_widget(Paragraph::new("Select an item to view details...").block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Details ").border_style(Style::default().fg(THEME.border))).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
            } else {
                let filtered_issues: Vec<_> = app.issues.items.iter()
                    .filter(|i| sq.is_empty() || i.title.to_lowercase().contains(&sq))
                    .collect();
                
                let rows = filtered_issues.iter().map(|i| {
                    let (state_text, state_style) = if i.state == "opened" {
                        ("  OPEN  ", Style::default().fg(THEME.green).bg(THEME.green_bg).add_modifier(Modifier::BOLD))
                    } else {
                        (" CLOSED ", Style::default().fg(THEME.red).bg(THEME.red_bg).add_modifier(Modifier::BOLD))
                    };
                    Row::new(vec![
                        Cell::from(format!("#{}", i.iid)),
                        Cell::from(state_text).style(state_style),
                        Cell::from(truncate(&i.title, 50)),
                        Cell::from(truncate(&i.author.username, 15)).style(Style::default().fg(THEME.blue)),
                        Cell::from(time_ago(&i.updated_at)).style(Style::default().fg(THEME.yellow)),
                    ]).height(1)
                });

                let widths = [
                    Constraint::Length(8),
                    Constraint::Length(10),
                    Constraint::Percentage(50),
                    Constraint::Length(15),
                    Constraint::Length(15),
                ];

                let table = Table::new(rows, widths)
                    .header(Row::new(vec!["ID", "State", "Title", "Author", "Updated"]).style(header_style).height(1))
                    .block(main_block)
                    .row_highlight_style(highlight_style)
                    .highlight_symbol(" ❯ ");
                
                f.render_stateful_widget(table, middle_chunks[1], &mut app.issues.state);

                let preview_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(THEME.border))
                    .title(" Details ")
                    .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD));
                if let Some(selected) = app.issues.state.selected() {
                    if let Some(issue) = filtered_issues.get(selected) {
                        let labels = if issue.labels.is_empty() { "None".to_string() } else { issue.labels.join(", ") };
                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled("Title:  ", Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD)),
                            Span::styled(&issue.title, Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD)),
                        ]));
                        text.push(Line::from(""));
                        text.push(Line::from(vec![
                            Span::styled("Author: ", Style::default().fg(THEME.text_muted)),
                            Span::styled(format!("@{}", issue.author.username), Style::default().fg(THEME.blue)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("State:  ", Style::default().fg(THEME.text_muted)),
                            Span::styled(
                                if issue.state == "opened" { "OPEN" } else { "CLOSED" },
                                Style::default().fg(if issue.state == "opened" { THEME.green } else { THEME.red }).add_modifier(Modifier::BOLD)
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Updated:", Style::default().fg(THEME.text_muted)),
                            Span::styled(time_ago(&issue.updated_at), Style::default().fg(THEME.yellow)),
                        ]));
                        text.push(Line::from(""));
                        text.push(Line::from(vec![
                            Span::styled("Labels: ", Style::default().fg(THEME.text_muted)),
                            Span::styled(labels, Style::default().fg(THEME.purple)),
                        ]));
                        f.render_widget(Paragraph::new(text).block(preview_block).wrap(ratatui::widgets::Wrap { trim: true }), middle_chunks[2]);
                    } else {
                        f.render_widget(Paragraph::new("").block(preview_block), middle_chunks[2]);
                    }
                } else {
                    f.render_widget(Paragraph::new("Select an item to view details...").block(preview_block).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
                }
            }
        },
        Tab::MergeRequests => {
            if app.mrs.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
                f.render_widget(Paragraph::new("\n\n Loading merge requests...").alignment(Alignment::Center).block(main_block.clone()).style(Style::default().fg(THEME.text_muted)), middle_chunks[1]);
                f.render_widget(Paragraph::new("Select an item to view details...").block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Details ").border_style(Style::default().fg(THEME.border))).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
            } else {
                let filtered_mrs: Vec<_> = app.mrs.items.iter()
                    .filter(|m| sq.is_empty() || m.title.to_lowercase().contains(&sq))
                    .collect();
                
                let rows = filtered_mrs.iter().map(|m| {
                    let (state_text, state_style) = if m.state == "opened" {
                        ("  OPEN  ", Style::default().fg(THEME.green).bg(THEME.green_bg).add_modifier(Modifier::BOLD))
                    } else if m.state == "merged" {
                        (" MERGED ", Style::default().fg(THEME.purple).bg(THEME.purple_bg).add_modifier(Modifier::BOLD))
                    } else {
                        (" CLOSED ", Style::default().fg(THEME.red).bg(THEME.red_bg).add_modifier(Modifier::BOLD))
                    };
                    Row::new(vec![
                        Cell::from(format!("!{}", m.iid)),
                        Cell::from(state_text).style(state_style),
                        Cell::from(truncate(&m.title, 50)),
                        Cell::from(truncate(&m.author.username, 15)).style(Style::default().fg(THEME.blue)),
                        Cell::from(time_ago(&m.updated_at)).style(Style::default().fg(THEME.yellow)),
                    ]).height(1)
                });

                let widths = [
                    Constraint::Length(8),
                    Constraint::Length(10),
                    Constraint::Percentage(50),
                    Constraint::Length(15),
                    Constraint::Length(15),
                ];

                let table = Table::new(rows, widths)
                    .header(Row::new(vec!["ID", "State", "Title", "Author", "Updated"]).style(header_style).height(1))
                    .block(main_block)
                    .row_highlight_style(highlight_style)
                    .highlight_symbol(" ❯ ");
                
                f.render_stateful_widget(table, middle_chunks[1], &mut app.mrs.state);

                let preview_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(THEME.border))
                    .title(" Details ")
                    .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD));
                if let Some(selected) = app.mrs.state.selected() {
                    if let Some(mr) = filtered_mrs.get(selected) {
                        let labels = if mr.labels.is_empty() { "None".to_string() } else { mr.labels.join(", ") };
                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled("Title:  ", Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD)),
                            Span::styled(&mr.title, Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD)),
                        ]));
                        text.push(Line::from(""));
                        text.push(Line::from(vec![
                            Span::styled("Author: ", Style::default().fg(THEME.text_muted)),
                            Span::styled(format!("@{}", mr.author.username), Style::default().fg(THEME.blue)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("State:  ", Style::default().fg(THEME.text_muted)),
                            Span::styled(
                                if mr.state == "opened" { "OPEN" } else if mr.state == "merged" { "MERGED" } else { "CLOSED" },
                                Style::default().fg(if mr.state == "opened" { THEME.green } else if mr.state == "merged" { THEME.purple } else { THEME.red }).add_modifier(Modifier::BOLD)
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Updated:", Style::default().fg(THEME.text_muted)),
                            Span::styled(time_ago(&mr.updated_at), Style::default().fg(THEME.yellow)),
                        ]));
                        text.push(Line::from(""));
                        text.push(Line::from(vec![
                            Span::styled("Labels: ", Style::default().fg(THEME.text_muted)),
                            Span::styled(labels, Style::default().fg(THEME.purple)),
                        ]));
                        f.render_widget(Paragraph::new(text).block(preview_block).wrap(ratatui::widgets::Wrap { trim: true }), middle_chunks[2]);
                    } else {
                        f.render_widget(Paragraph::new("").block(preview_block), middle_chunks[2]);
                    }
                } else {
                    f.render_widget(Paragraph::new("Select an item to view details...").block(preview_block).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
                }
            }
        },
        Tab::Pipelines => {
            if app.pipelines.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
                f.render_widget(Paragraph::new("\n\n Loading pipelines...").alignment(Alignment::Center).block(main_block.clone()).style(Style::default().fg(THEME.text_muted)), middle_chunks[1]);
                f.render_widget(Paragraph::new("Select a pipeline to view details...").block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Details ").border_style(Style::default().fg(THEME.border))).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
            } else {
                if let Some(jobs) = &app.selected_pipeline_jobs {
                    let rows = jobs.iter().enumerate().map(|(i, j)| {
                        let (status_text, status_color, bg_color) = match j.status.as_str() {
                            "success" => (" SUCCESS ", THEME.green, THEME.green_bg),
                            "failed" => ("  FAILED ", THEME.red, THEME.red_bg),
                            "running" => (" RUNNING ", THEME.blue, THEME.blue_bg),
                            "canceled" => ("CANCELED ", THEME.text_muted, THEME.highlight_bg),
                            "pending" => (" PENDING ", THEME.yellow, THEME.yellow_bg),
                            "skipped" => (" SKIPPED ", THEME.text_muted, THEME.highlight_bg),
                            "manual" => ("  MANUAL ", THEME.text_muted, THEME.highlight_bg),
                            _ => (" UNKNOWN ", THEME.text_muted, THEME.highlight_bg),
                        };
                        let style = if Some(i) == app.selected_job_index {
                            Style::default().bg(THEME.highlight_bg).fg(THEME.border_focused).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        };
                        Row::new(vec![
                            Cell::from(j.id.to_string()),
                            Cell::from(j.stage.clone()),
                            Cell::from(status_text).style(Style::default().fg(status_color).bg(bg_color).add_modifier(Modifier::BOLD)),
                            Cell::from(j.name.clone()),
                        ]).style(style).height(1)
                    });

                    let widths = [
                        Constraint::Length(10),
                        Constraint::Length(15),
                        Constraint::Length(12),
                        Constraint::Percentage(60),
                    ];

                    let table = Table::new(rows, widths)
                        .header(Row::new(vec!["ID", "Stage", "Status", "Name"]).style(header_style).height(1))
                        .block(Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .title(" Jobs (Esc to go back) ")
                            .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD))
                            .border_style(Style::default().fg(THEME.border_focused)));
                    
                    f.render_widget(table, middle_chunks[1]);

                    let preview_block = Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .title(" Details / Trace ")
                        .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD))
                        .border_style(Style::default().fg(THEME.border));
                    if let Some(trace) = &app.job_trace {
                        f.render_widget(Paragraph::new(trace.as_str()).block(preview_block).wrap(ratatui::widgets::Wrap { trim: false }), middle_chunks[2]);
                    } else {
                        let summaries = get_stages_summary(jobs);
                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled("Stages Success Rate:", Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD)),
                        ]));
                        text.push(Line::from(""));
                        for s in summaries {
                            let status_color = match s.status.as_str() {
                                "success" => THEME.green,
                                "failed" => THEME.red,
                                "running" => THEME.blue,
                                "pending" => THEME.yellow,
                                _ => THEME.text_muted,
                            };
                            text.push(Line::from(vec![
                                Span::styled(format!("{:15} ", s.name), Style::default().fg(THEME.text_normal)),
                                Span::styled(" ❯ ", Style::default().fg(THEME.text_muted)),
                                Span::styled(format!("{}% ", s.percent), Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
                                Span::styled(format!("({}/{})", s.success, s.total), Style::default().fg(THEME.text_muted)),
                            ]));
                        }
                        text.push(Line::from(""));
                        text.push(Line::from(vec![
                            Span::styled("Press Enter on a job to fetch trace.", Style::default().fg(THEME.text_muted).add_modifier(Modifier::ITALIC)),
                        ]));
                        f.render_widget(Paragraph::new(text).block(preview_block), middle_chunks[2]);
                    }
                } else {
                    let filtered_pipelines: Vec<_> = app.pipelines.items.iter()
                        .filter(|p| sq.is_empty() || p.r#ref.to_lowercase().contains(&sq))
                        .collect();
                        
                    let rows = filtered_pipelines.iter().map(|p| {
                        let (status_text, status_color, bg_color) = match p.status.as_str() {
                            "success" => (" SUCCESS ", THEME.green, THEME.green_bg),
                            "failed" => ("  FAILED ", THEME.red, THEME.red_bg),
                            "running" => (" RUNNING ", THEME.blue, THEME.blue_bg),
                            "canceled" => ("CANCELED ", THEME.text_muted, THEME.highlight_bg),
                            "pending" => (" PENDING ", THEME.yellow, THEME.yellow_bg),
                            "skipped" => (" SKIPPED ", THEME.text_muted, THEME.highlight_bg),
                            "manual" => ("  MANUAL ", THEME.text_muted, THEME.highlight_bg),
                            _ => (" UNKNOWN ", THEME.text_muted, THEME.highlight_bg),
                        };
                        let stages_dots = if let Some(jobs) = app.pipeline_jobs.get(&p.id) {
                            get_stages_dots(jobs)
                        } else {
                            "⏳".to_string()
                        };
                        Row::new(vec![
                            Cell::from(format!("#{}", p.id)),
                            Cell::from(status_text).style(Style::default().fg(status_color).bg(bg_color).add_modifier(Modifier::BOLD)),
                            Cell::from(stages_dots),
                            Cell::from(truncate(&p.r#ref, 40)).style(Style::default().fg(THEME.purple)),
                            Cell::from(time_ago(&p.updated_at)).style(Style::default().fg(THEME.yellow)),
                        ]).height(1)
                    });

                    let widths = [
                        Constraint::Length(10),
                        Constraint::Length(12),
                        Constraint::Length(14),
                        Constraint::Percentage(45),
                        Constraint::Length(15),
                    ];

                    let table = Table::new(rows, widths)
                        .header(Row::new(vec!["ID", "Status", "Stages", "Ref", "Updated"]).style(header_style).height(1))
                        .block(main_block)
                        .row_highlight_style(highlight_style)
                        .highlight_symbol(" ❯ ");
                    
                    f.render_stateful_widget(table, middle_chunks[1], &mut app.pipelines.state);

                    let preview_block = Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .title(" Details ")
                        .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD))
                        .border_style(Style::default().fg(THEME.border));
                    if let Some(selected) = app.pipelines.state.selected() {
                        if let Some(p) = filtered_pipelines.get(selected) {
                            let mut text = Vec::new();
                            text.push(Line::from(vec![
                                Span::styled("Pipeline ID: ", Style::default().fg(THEME.text_muted)),
                                Span::styled(format!("#{}", p.id), Style::default().fg(THEME.blue).add_modifier(Modifier::BOLD)),
                            ]));
                            text.push(Line::from(vec![
                                Span::styled("Ref:         ", Style::default().fg(THEME.text_muted)),
                                Span::styled(&p.r#ref, Style::default().fg(THEME.purple)),
                            ]));
                            
                            let (status_text, status_color) = match p.status.as_str() {
                                "success" => ("success", THEME.green),
                                "failed" => ("failed", THEME.red),
                                "running" => ("running", THEME.blue),
                                "canceled" => ("canceled", THEME.text_muted),
                                "pending" => ("pending", THEME.yellow),
                                _ => ("unknown", THEME.text_muted),
                            };
                            
                            text.push(Line::from(vec![
                                Span::styled("Status:      ", Style::default().fg(THEME.text_muted)),
                                Span::styled(status_text, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
                            ]));
                            text.push(Line::from(vec![
                                Span::styled("Updated:     ", Style::default().fg(THEME.text_muted)),
                                Span::styled(time_ago(&p.updated_at), Style::default().fg(THEME.yellow)),
                            ]));
                            text.push(Line::from(""));

                            if let Some(jobs) = app.pipeline_jobs.get(&p.id) {
                                text.push(Line::from(vec![
                                    Span::styled("Stages Success Rate:", Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD)),
                                ]));
                                text.push(Line::from(""));
                                let summaries = get_stages_summary(jobs);
                                for s in summaries {
                                    let status_color = match s.status.as_str() {
                                        "success" => THEME.green,
                                        "failed" => THEME.red,
                                        "running" => THEME.blue,
                                        "pending" => THEME.yellow,
                                        _ => THEME.text_muted,
                                    };
                                    text.push(Line::from(vec![
                                        Span::styled(format!("{:15} ", s.name), Style::default().fg(THEME.text_normal)),
                                        Span::styled(" ❯ ", Style::default().fg(THEME.text_muted)),
                                        Span::styled(format!("{}% ", s.percent), Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
                                        Span::styled(format!("({}/{})", s.success, s.total), Style::default().fg(THEME.text_muted)),
                                    ]));
                                }
                            } else {
                                text.push(Line::from(vec![
                                    Span::styled("Loading stages...", Style::default().fg(THEME.text_muted).add_modifier(Modifier::ITALIC)),
                                ]));
                            }
                            text.push(Line::from(""));
                            text.push(Line::from(vec![
                                Span::styled("Press Enter to view detailed job logs.", Style::default().fg(THEME.text_muted).add_modifier(Modifier::ITALIC)),
                            ]));
                            f.render_widget(Paragraph::new(text).block(preview_block), middle_chunks[2]);
                        } else {
                            f.render_widget(Paragraph::new("").block(preview_block), middle_chunks[2]);
                        }
                    } else {
                        f.render_widget(Paragraph::new("Select an item to view details...").block(preview_block).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
                    }
                }
            }
        },
        Tab::Runners => {
            if app.runners.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
                f.render_widget(Paragraph::new("\n\n Loading runners...").alignment(Alignment::Center).block(main_block.clone()).style(Style::default().fg(THEME.text_muted)), middle_chunks[1]);
                f.render_widget(Paragraph::new("Select a runner to view details...").block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Details ").border_style(Style::default().fg(THEME.border))).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
            } else {
                let filtered_runners: Vec<_> = app.runners.items.iter()
                    .filter(|r| sq.is_empty() || r.id.to_string().contains(&sq) || r.description.as_deref().unwrap_or("").to_lowercase().contains(&sq))
                    .collect();
                
                let rows = filtered_runners.iter().map(|r| {
                    let (status_text, status_color, bg_color) = match r.status.as_str() {
                        "online" => (" ONLINE  ", THEME.green, THEME.green_bg),
                        "paused" => (" PAUSED  ", THEME.yellow, THEME.yellow_bg),
                        "offline" => (" OFFLINE ", THEME.red, THEME.red_bg),
                        _ => (" UNKNOWN ", THEME.text_muted, THEME.highlight_bg),
                    };
                    let desc = r.description.as_deref().unwrap_or("No description");
                    Row::new(vec![
                        Cell::from(r.id.to_string()),
                        Cell::from(truncate(desc, 45)),
                        Cell::from(status_text).style(Style::default().fg(status_color).bg(bg_color).add_modifier(Modifier::BOLD)),
                        Cell::from(r.active.to_string()).style(Style::default().fg(if r.active { THEME.green } else { THEME.red })),
                    ]).height(1)
                });

                let widths = [
                    Constraint::Length(10),
                    Constraint::Percentage(55),
                    Constraint::Length(12),
                    Constraint::Length(8),
                ];

                let table = Table::new(rows, widths)
                    .header(Row::new(vec!["ID", "Description", "Status", "Active"]).style(header_style).height(1))
                    .block(main_block)
                    .row_highlight_style(highlight_style)
                    .highlight_symbol(" ❯ ");
                
                f.render_stateful_widget(table, middle_chunks[1], &mut app.runners.state);

                let preview_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(" Details ")
                    .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD))
                    .border_style(Style::default().fg(THEME.border));
                if let Some(selected) = app.runners.state.selected() {
                    if let Some(r) = filtered_runners.get(selected) {
                        let desc = r.description.as_deref().unwrap_or("None");
                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled("Runner ID:   ", Style::default().fg(THEME.text_muted)),
                            Span::styled(r.id.to_string(), Style::default().fg(THEME.blue).add_modifier(Modifier::BOLD)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Description: ", Style::default().fg(THEME.text_muted)),
                            Span::styled(desc, Style::default().fg(THEME.text_normal)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Status:      ", Style::default().fg(THEME.text_muted)),
                            Span::styled(&r.status, Style::default().fg(if r.status == "online" { THEME.green } else { THEME.red }).add_modifier(Modifier::BOLD)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Active:      ", Style::default().fg(THEME.text_muted)),
                            Span::styled(r.active.to_string(), Style::default().fg(if r.active { THEME.green } else { THEME.red })),
                        ]));
                        text.push(Line::from(""));
                        text.push(Line::from(vec![
                            Span::styled("ctrl-p: pause  •  ctrl-r: resume  •  ctrl-e: edit", Style::default().fg(THEME.text_muted).add_modifier(Modifier::ITALIC)),
                        ]));
                        f.render_widget(Paragraph::new(text).block(preview_block).wrap(ratatui::widgets::Wrap { trim: true }), middle_chunks[2]);
                    } else {
                        f.render_widget(Paragraph::new("").block(preview_block), middle_chunks[2]);
                    }
                } else {
                    f.render_widget(Paragraph::new("Select an item to view details...").block(preview_block).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
                }
            }
        },
        Tab::Releases => {
            if app.releases.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
                f.render_widget(Paragraph::new("\n\n Loading releases...").alignment(Alignment::Center).block(main_block.clone()).style(Style::default().fg(THEME.text_muted)), middle_chunks[1]);
                f.render_widget(Paragraph::new("Select a release to view details...").block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Details ").border_style(Style::default().fg(THEME.border))).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
            } else {
                let filtered_releases: Vec<_> = app.releases.items.iter()
                    .filter(|r| sq.is_empty() || r.name.to_lowercase().contains(&sq) || r.tag_name.to_lowercase().contains(&sq))
                    .collect();
                
                let rows = filtered_releases.iter().map(|r| {
                    Row::new(vec![
                        Cell::from(r.tag_name.clone()).style(Style::default().fg(THEME.green).add_modifier(Modifier::BOLD)),
                        Cell::from(truncate(&r.name, 45)),
                        Cell::from(truncate(&r.released_at, 10)).style(Style::default().fg(THEME.yellow)),
                    ]).height(1)
                });

                let widths = [
                    Constraint::Length(16),
                    Constraint::Percentage(60),
                    Constraint::Length(12),
                ];

                let table = Table::new(rows, widths)
                    .header(Row::new(vec!["Tag", "Release Name", "Date"]).style(header_style).height(1))
                    .block(main_block)
                    .row_highlight_style(highlight_style)
                    .highlight_symbol(" ❯ ");
                
                f.render_stateful_widget(table, middle_chunks[1], &mut app.releases.state);

                let preview_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(" Details ")
                    .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD))
                    .border_style(Style::default().fg(THEME.border));
                if let Some(selected) = app.releases.state.selected() {
                    if let Some(r) = filtered_releases.get(selected) {
                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled("Release: ", Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD)),
                            Span::styled(&r.name, Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Tag:     ", Style::default().fg(THEME.text_muted)),
                            Span::styled(&r.tag_name, Style::default().fg(THEME.green).add_modifier(Modifier::BOLD)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Date:    ", Style::default().fg(THEME.text_muted)),
                            Span::styled(&r.released_at, Style::default().fg(THEME.yellow)),
                        ]));
                        text.push(Line::from(""));
                        text.push(Line::from(vec![
                            Span::styled("Press ctrl-o to open in browser", Style::default().fg(THEME.text_muted).add_modifier(Modifier::ITALIC)),
                        ]));
                        f.render_widget(Paragraph::new(text).block(preview_block).wrap(ratatui::widgets::Wrap { trim: true }), middle_chunks[2]);
                    } else {
                        f.render_widget(Paragraph::new("").block(preview_block), middle_chunks[2]);
                    }
                } else {
                    f.render_widget(Paragraph::new("Select an item to view details...").block(preview_block).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
                }
            }
        }
    }

    // Bottom: Help Bar
    let help_text = match app.active_tab {
        Tab::Issues => "  h/l: Tabs • j/k: Nav • /: Search • ctrl-n: New • ctrl-t/l/a/d: Edit • F5: Refresh • Enter: View • q: Quit  ",
        Tab::MergeRequests => "  h/l: Tabs • j/k: Nav • /: Search • ctrl-n/a/m/s: Manage MR • ctrl-t/l/a/d: Edit • F5: Refresh • q: Quit  ",
        Tab::Pipelines => "  h/l: Tabs • j/k: Nav • /: Search • ctrl-p: Run MR Pipe • ctrl-r/d/o: Job acts • F5: Refresh • q: Quit  ",
        Tab::Runners => "  h/l: Tabs • j/k: Nav • /: Search • ctrl-p/r: Pause/Resume • ctrl-e: Edit • F5: Refresh • q: Quit  ",
        Tab::Releases => "  h/l: Tabs • j/k: Nav • /: Search • ctrl-o: Browser • Enter: Terminal • F5: Refresh • q: Quit  ",
    };
    let help = Paragraph::new(help_text)
        .style(Style::default().fg(THEME.text_normal).bg(THEME.highlight_bg).add_modifier(Modifier::BOLD))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::NONE));
    f.render_widget(help, chunks[2]);

    // Error Popup overlay
    if let Some(err) = &app.error_message {
        let block = Block::default()
            .title(" Error ")
            .title_style(Style::default().fg(THEME.red).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .style(Style::default().fg(THEME.red).bg(THEME.bg));
        let paragraph = Paragraph::new(err.clone())
            .block(block)
            .alignment(Alignment::Center);
        
        let area = centered_rect(60, 20, size);
        f.render_widget(Clear, area); //this clears out the background
        f.render_widget(paragraph, area);
    }
}

/// helper function to create a centered rect using up certain percentage of the available rect `r`
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
