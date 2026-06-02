use ratatui::{
    layout::{Constraint, Direction, Layout, Rect, Alignment},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph, Clear, Table, Row, Cell, BorderType},
    text::{Line, Span},
    Frame,
};

use crate::app::{App, Tab};
use crate::utils::format::{truncate, time_ago, format_ref, render_markdown};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

fn highlight_fuzzy_match(text: &str, indices: &[usize], base_style: Style, highlight_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut current_chunk = String::new();
    let mut is_highlighted = false;

    let index_set: std::collections::HashSet<usize> = indices.iter().cloned().collect();

    for (i, c) in text.chars().enumerate() {
        let char_is_highlighted = index_set.contains(&i);
        if char_is_highlighted != is_highlighted {
            if !current_chunk.is_empty() {
                spans.push(Span::styled(
                    current_chunk.clone(),
                    if is_highlighted { highlight_style } else { base_style }
                ));
                current_chunk.clear();
            }
            is_highlighted = char_is_highlighted;
        }
        current_chunk.push(c);
    }

    if !current_chunk.is_empty() {
        spans.push(Span::styled(
            current_chunk,
            if is_highlighted { highlight_style } else { base_style }
        ));
    }

    spans
}

struct Theme {
    bg: Color,
    border: Color,
    border_focused: Color,
    header_fg: Color,
    highlight_bg: Color,
    inactive_bg: Color,
    text_normal: Color,
    text_muted: Color,
    
    // Status colors (Sunset themed)
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
    bg: Color::Rgb(18, 18, 20),            // dark slate base
    border: Color::Rgb(80, 80, 88),        // muted gray border for inactive panes
    border_focused: Color::Rgb(49, 191, 103), // vibrant green for active panes
    header_fg: Color::Rgb(49, 191, 103),  // vibrant green for active headers
    highlight_bg: Color::Rgb(49, 191, 103), // selection highlight background is green
    inactive_bg: Color::Rgb(49, 50, 68),   // dark gray surface for selection hover or inactive elements
    text_normal: Color::Rgb(216, 222, 233),// light text
    text_muted: Color::Rgb(130, 130, 138), // muted gray text
    
    green: Color::Rgb(49, 191, 103),       // success / open (vibrant green)
    green_bg: Color::Rgb(20, 45, 28),      // dark green background for pill
    red: Color::Rgb(224, 73, 83),          // failed / closed
    red_bg: Color::Rgb(50, 20, 25),        // dark red background for pill
    blue: Color::Rgb(61, 139, 255),        // running / active
    blue_bg: Color::Rgb(15, 35, 60),       // dark blue background for pill
    yellow: Color::Rgb(235, 180, 50),      // pending / warning
    yellow_bg: Color::Rgb(45, 35, 15),     // dark yellow background for pill
    purple: Color::Rgb(168, 122, 243),     // merged / releases
    purple_bg: Color::Rgb(38, 25, 55),     // dark purple background for pill
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
            let success = statuses.iter().filter(|s| *s == "success" || *s == "skipped").count();
            let percent = if total > 0 { (success * 100) / total } else { 0 };
            
            let stage_status = if statuses.iter().any(|s| s == "failed") {
                "failed".to_string()
            } else if statuses.iter().any(|s| s == "running") {
                "running".to_string()
            } else if statuses.iter().any(|s| s == "pending" || s == "preparing" || s == "waiting_for_resource") {
                "pending".to_string()
            } else if statuses.iter().any(|s| s == "canceled") {
                "canceled".to_string()
            } else if statuses.iter().any(|s| s == "success") {
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

fn append_stage_summaries(text: &mut Vec<Line<'static>>, jobs: &[crate::gitlab::pipelines::Job]) {
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
            Span::styled(format!("{:15} ", truncate(&s.name, 15)), Style::default().fg(THEME.text_normal)),
            Span::styled(" ❯ ", Style::default().fg(THEME.text_muted)),
            Span::styled(format!("{:>4} ", format!("{}%", s.percent)), Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
            Span::styled(format!("({}/{})", s.success, s.total), Style::default().fg(THEME.text_muted)),
        ]));
    }
}

fn add_cmd(text: &mut Vec<Line<'static>>, key: &str, desc: &str) {
    let padded_key = format!(" {:^3} ", key);
    text.push(Line::from(vec![
        Span::styled(padded_key, Style::default().bg(THEME.border_focused).fg(THEME.bg).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {}", desc), Style::default().fg(THEME.text_normal)),
    ]));
}

pub fn render(f: &mut Frame, app: &mut App) {
    let render_fuzzy_cell = |text: &str, query: &str, is_selected: bool, base_style: Style| {
        if query.trim().is_empty() {
            Cell::from(text.to_string()).style(base_style)
        } else {
            let matcher = SkimMatcherV2::default();
            if let Some((_, indices)) = matcher.fuzzy_indices(text, query) {
                let highlight_style = if is_selected {
                    Style::default().bg(THEME.highlight_bg).fg(THEME.yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(THEME.yellow).add_modifier(Modifier::BOLD)
                };
                let styled_base = if is_selected {
                    Style::default().bg(THEME.highlight_bg).fg(THEME.bg).add_modifier(Modifier::BOLD)
                } else {
                    base_style
                };
                Cell::from(Line::from(highlight_fuzzy_match(text, &indices, styled_base, highlight_style)))
            } else {
                Cell::from(text.to_string()).style(base_style)
            }
        }
    };

    let size = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(size);

    // Top: Title & Context (Zellij Vibe Horizontal Bar)
    let mut title_spans = vec![
        Span::styled(" GLAB-TUI ", Style::default().bg(THEME.border_focused).fg(THEME.bg).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" ❯ {} ", app.project_context), Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD)),
    ];
    if app.is_typing_search {
        title_spans.push(Span::styled(" SEARCHING ", Style::default().bg(THEME.yellow).fg(THEME.bg).add_modifier(Modifier::BOLD)));
        title_spans.push(Span::styled(format!(" {}_ ", app.search_query), Style::default().fg(THEME.yellow)));
    } else if !app.search_query.is_empty() {
        title_spans.push(Span::styled(" FILTERED ", Style::default().bg(THEME.yellow).fg(THEME.bg).add_modifier(Modifier::BOLD)));
        title_spans.push(Span::styled(format!(" {} ", app.search_query), Style::default().fg(THEME.yellow)));
    }

    let title = Paragraph::new(Line::from(title_spans))
        .style(Style::default().bg(THEME.bg))
        .block(Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(THEME.border))
        );
    f.render_widget(title, chunks[0]);

    // Middle: Sidebar | Main Area | Preview Area
    let can_zoom = app.active_tab != Tab::Pipelines || app.selected_pipeline_jobs.is_some();
    let middle_chunks = if app.details_zoomed && can_zoom {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(0),
                Constraint::Length(0),
                Constraint::Min(0),
            ])
            .split(chunks[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(22),
                Constraint::Min(0),
                Constraint::Length(45),
            ])
            .split(chunks[1])
    };

    // Sidebar Navigation & Commands Layout
    let sidebar_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .split(middle_chunks[0]);

    // Sidebar: Tabs
    let is_github = app.gitlab_client.as_ref().map(|c| c.is_github).unwrap_or(false);
    let sidebar_items: Vec<ListItem> = Tab::ALL
        .iter()
        .map(|t| {
            let title = format!("  {}  ", t.title(is_github).to_uppercase());
            if *t == app.active_tab {
                ListItem::new(title)
                    .style(Style::default().bg(THEME.border_focused).fg(THEME.bg).add_modifier(Modifier::BOLD))
            } else {
                ListItem::new(title)
                    .style(Style::default().fg(THEME.text_muted))
            }
        })
        .collect();
    
    let sidebar = List::new(sidebar_items)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.border))
            .title(" Navigation ")
            .title_style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD))
        );
    f.render_widget(sidebar, sidebar_chunks[0]);

    // Render Commands sidebar block
    let commands_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border))
        .title(" Commands ")
        .title_style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD));

    let mut commands_text = Vec::new();
    let pr_suffix = if is_github { "PR" } else { "MR" };
    match app.active_tab {
        Tab::Issues => {
            add_cmd(&mut commands_text, "e", "Edit params");
            add_cmd(&mut commands_text, "f", "Search");
            add_cmd(&mut commands_text, "n", "New Issue");
            add_cmd(&mut commands_text, "c", "Close Issue");
            add_cmd(&mut commands_text, "J/K", "Scroll Desc");
            add_cmd(&mut commands_text, "C-r", "Refresh");
            add_cmd(&mut commands_text, "?", "Help");
            add_cmd(&mut commands_text, "q", "Quit");
        }
        Tab::MergeRequests => {
            add_cmd(&mut commands_text, "e", "Edit params");
            add_cmd(&mut commands_text, "f", "Search");
            add_cmd(&mut commands_text, "n", &format!("New {}", pr_suffix));
            add_cmd(&mut commands_text, "m", &format!("Merge {}", pr_suffix));
            add_cmd(&mut commands_text, "a", &format!("Approve {}", pr_suffix));
            add_cmd(&mut commands_text, "v", "Diff/Changes");
            add_cmd(&mut commands_text, "o", "View Browser");
            add_cmd(&mut commands_text, "s", "Toggle Draft");
            add_cmd(&mut commands_text, "J/K", "Scroll Desc");
            add_cmd(&mut commands_text, "C-r", "Refresh");
            add_cmd(&mut commands_text, "?", "Help");
            add_cmd(&mut commands_text, "q", "Quit");
        }
        Tab::Pipelines => {
            if app.job_trace.is_some() {
                add_cmd(&mut commands_text, "j/k", "Scroll Trace");
                add_cmd(&mut commands_text, "Esc", "Close Trace");
            } else if app.selected_pipeline_jobs.is_some() {
                add_cmd(&mut commands_text, "Ent", "View Trace");
                add_cmd(&mut commands_text, "Spc", "Toggle Select");
                add_cmd(&mut commands_text, "r", "Retry Job(s)");
                add_cmd(&mut commands_text, "d", "Download Art");
                add_cmd(&mut commands_text, "o", "View Browser");
                add_cmd(&mut commands_text, "e", "View Helix");
                add_cmd(&mut commands_text, "Esc", "Back to Pipes");
            } else {
                add_cmd(&mut commands_text, "Ent", "View Jobs");
                add_cmd(&mut commands_text, "r", "Retry Pipe");
                add_cmd(&mut commands_text, "p", &format!("Run {} Pipe", pr_suffix));
                add_cmd(&mut commands_text, "c", "Cancel Pipe");
                add_cmd(&mut commands_text, "o", "View Browser");
                add_cmd(&mut commands_text, "f", "Search");
                add_cmd(&mut commands_text, "C-r", "Refresh");
                add_cmd(&mut commands_text, "?", "Help");
                add_cmd(&mut commands_text, "q", "Quit");
            }
        }
        Tab::Runners => {
            add_cmd(&mut commands_text, "p", "Pause");
            add_cmd(&mut commands_text, "r", "Resume");
            add_cmd(&mut commands_text, "e", "Edit Desc");
            add_cmd(&mut commands_text, "f", "Search");
            add_cmd(&mut commands_text, "C-r", "Refresh");
            add_cmd(&mut commands_text, "?", "Help");
            add_cmd(&mut commands_text, "q", "Quit");
        }
        Tab::Releases => {
            add_cmd(&mut commands_text, "Ent", "View Notes");
            add_cmd(&mut commands_text, "o", "View Browser");
            add_cmd(&mut commands_text, "f", "Search");
            add_cmd(&mut commands_text, "C-r", "Refresh");
            add_cmd(&mut commands_text, "?", "Help");
            add_cmd(&mut commands_text, "q", "Quit");
        }
    }

    let commands_para = Paragraph::new(commands_text)
        .block(commands_block)
        .wrap(ratatui::widgets::Wrap { trim: true });
    f.render_widget(commands_para, sidebar_chunks[1]);

    // Main Area Title
    let tab_title = if app.loading_tabs.contains(&app.active_tab) {
        format!(" {} (loading...) ", app.active_tab.title(is_github))
    } else {
        format!(" {} ", app.active_tab.title(is_github))
    };
    let main_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border_focused))
        .title(tab_title)
        .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD));
    
    let highlight_style = Style::default().bg(THEME.highlight_bg).fg(THEME.bg).add_modifier(Modifier::BOLD);
    let header_style = Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD);

    match app.active_tab {
        Tab::Issues => {
            if app.issues.items.is_empty() && app.loading_tabs.contains(&app.active_tab) {
                f.render_widget(Paragraph::new("\n\n Loading issues...").alignment(Alignment::Center).block(main_block.clone()).style(Style::default().fg(THEME.text_muted)), middle_chunks[1]);
                f.render_widget(Paragraph::new("Select an item to view details...").block(Block::default().borders(Borders::ALL).title(" Details ").border_style(Style::default().fg(THEME.border))).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
            } else {
                let filtered_issues = App::filter_issues_list(&app.issues.items, &app.search_query);
                
                let rows = filtered_issues.iter().enumerate().map(|(idx, i)| {
                    let (state_text, state_style) = if i.state == "opened" {
                        ("  OPEN  ", Style::default().fg(THEME.green).bg(THEME.green_bg).add_modifier(Modifier::BOLD))
                    } else {
                        (" CLOSED ", Style::default().fg(THEME.red).bg(THEME.red_bg).add_modifier(Modifier::BOLD))
                    };
                    let is_selected = app.issues.state.selected() == Some(idx);
                    Row::new(vec![
                        render_fuzzy_cell(&format!("#{}", i.iid), &app.search_query, is_selected, Style::default().fg(THEME.text_normal)),
                        Cell::from(state_text).style(state_style),
                        render_fuzzy_cell(&truncate(&i.title, 100), &app.search_query, is_selected, Style::default().fg(THEME.text_normal)),
                        render_fuzzy_cell(&truncate(&i.author.username, 15), &app.search_query, is_selected, Style::default().fg(THEME.blue)),
                        Cell::from(time_ago(&i.updated_at)).style(Style::default().fg(THEME.yellow)),
                    ]).height(1)
                });

                let widths = [
                    Constraint::Length(10),
                    Constraint::Length(10),
                    Constraint::Percentage(50),
                    Constraint::Length(18),
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
                    .border_style(Style::default().fg(THEME.border))
                    .title(" Details ")
                    .title_style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD));
                if let Some(selected) = app.issues.state.selected() {
                    if let Some(issue) = filtered_issues.get(selected) {
                        let labels = if issue.labels.is_empty() { "None".to_string() } else { issue.labels.join(", ") };
                        let milestone = issue.milestone.as_ref().map(|m| m.title.as_str()).unwrap_or("None");
                        let assignees = if issue.assignees.is_empty() {
                            "None".to_string()
                        } else {
                            issue.assignees.iter().map(|a| format!("@{}", a.username)).collect::<Vec<_>>().join(", ")
                        };
                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled("Title:     ", Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD)),
                            Span::styled(&issue.title, Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD)),
                        ]));
                        text.push(Line::from(""));
                        text.push(Line::from(vec![
                            Span::styled("Author:    ", Style::default().fg(THEME.text_muted)),
                            Span::styled(format!("@{}", issue.author.username), Style::default().fg(THEME.blue)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Assignees: ", Style::default().fg(THEME.text_muted)),
                            Span::styled(assignees, Style::default().fg(THEME.blue)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Milestone: ", Style::default().fg(THEME.text_muted)),
                            Span::styled(milestone, Style::default().fg(THEME.purple)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("State:     ", Style::default().fg(THEME.text_muted)),
                            Span::styled(
                                if issue.state == "opened" { "OPEN" } else { "CLOSED" },
                                Style::default().fg(if issue.state == "opened" { THEME.green } else { THEME.red }).add_modifier(Modifier::BOLD)
                            ),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Updated:   ", Style::default().fg(THEME.text_muted)),
                            Span::styled(time_ago(&issue.updated_at), Style::default().fg(THEME.yellow)),
                        ]));
                        text.push(Line::from(""));
                        text.push(Line::from(vec![
                            Span::styled("Labels:    ", Style::default().fg(THEME.text_muted)),
                            Span::styled(labels, Style::default().fg(THEME.purple)),
                        ]));
                        if let Some(desc) = &issue.description {
                            if !desc.trim().is_empty() {
                                text.push(Line::from(""));
                                text.push(Line::from(vec![
                                    Span::styled("Description:", Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD)),
                                ]));
                                text.extend(render_markdown(desc));
                            }
                        }

                        let viewport_height = middle_chunks[2].height.saturating_sub(2) as usize;
                        let content_length = text.len();
                        let max_scroll = content_length.saturating_sub(viewport_height) as u16;
                        app.issues_scroll = app.issues_scroll.min(max_scroll);

                        let title_suffix = if content_length > viewport_height {
                            let percent = (app.issues_scroll as usize * 100) / max_scroll.max(1) as usize;
                            format!(" [Shift+J/K | {}%] ", percent.min(100))
                        } else {
                            String::new()
                        };

                        let preview_block = Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(THEME.border))
                            .title(format!(" Details{} ", title_suffix))
                            .title_style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD));

                        f.render_widget(
                            Paragraph::new(text)
                                .block(preview_block)
                                .wrap(ratatui::widgets::Wrap { trim: true })
                                .scroll((app.issues_scroll, 0)),
                            middle_chunks[2]
                        );
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
                f.render_widget(Paragraph::new("Select an item to view details...").block(Block::default().borders(Borders::ALL).title(" Details ").border_style(Style::default().fg(THEME.border))).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
            } else {
                let filtered_mrs = App::filter_mrs_list(&app.mrs.items, &app.search_query);
                
                let rows = filtered_mrs.iter().enumerate().map(|(idx, m)| {
                    let (state_text, state_style) = if m.state == "opened" {
                        ("  OPEN  ", Style::default().fg(THEME.green).bg(THEME.green_bg).add_modifier(Modifier::BOLD))
                    } else if m.state == "merged" {
                        (" MERGED ", Style::default().fg(THEME.purple).bg(THEME.purple_bg).add_modifier(Modifier::BOLD))
                    } else {
                        (" CLOSED ", Style::default().fg(THEME.red).bg(THEME.red_bg).add_modifier(Modifier::BOLD))
                    };
                    let is_selected = app.mrs.state.selected() == Some(idx);
                    Row::new(vec![
                        render_fuzzy_cell(&format!("!{}", m.iid), &app.search_query, is_selected, Style::default().fg(THEME.text_normal)),
                        Cell::from(state_text).style(state_style),
                        render_fuzzy_cell(&truncate(&m.title, 100), &app.search_query, is_selected, Style::default().fg(THEME.text_normal)),
                        render_fuzzy_cell(&truncate(&m.author.username, 15), &app.search_query, is_selected, Style::default().fg(THEME.blue)),
                        Cell::from(time_ago(&m.updated_at)).style(Style::default().fg(THEME.yellow)),
                    ]).height(1)
                });

                let widths = [
                    Constraint::Length(10),
                    Constraint::Length(10),
                    Constraint::Percentage(50),
                    Constraint::Length(18),
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
                    .border_style(Style::default().fg(THEME.border))
                    .title(" Details ")
                    .title_style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD));
                if let Some(selected) = app.mrs.state.selected() {
                    if let Some(mr) = filtered_mrs.get(selected) {
                        let labels = if mr.labels.is_empty() { "None".to_string() } else { mr.labels.join(", ") };
                        let milestone = mr.milestone.as_ref().map(|m| m.title.as_str()).unwrap_or("None");
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
                        let draft_status = if mr.draft { "DRAFT" } else { "READY" };
                        let draft_color = if mr.draft { THEME.yellow } else { THEME.green };

                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled("Title:     ", Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD)),
                            Span::styled(&mr.title, Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD)),
                        ]));
                        text.push(Line::from(""));
                        text.push(Line::from(vec![
                            Span::styled("Author:    ", Style::default().fg(THEME.text_muted)),
                            Span::styled(format!("@{}", mr.author.username), Style::default().fg(THEME.blue)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Assignees: ", Style::default().fg(THEME.text_muted)),
                            Span::styled(assignees, Style::default().fg(THEME.blue)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Reviewers: ", Style::default().fg(THEME.text_muted)),
                            Span::styled(reviewers, Style::default().fg(THEME.blue)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Milestone: ", Style::default().fg(THEME.text_muted)),
                            Span::styled(milestone, Style::default().fg(THEME.purple)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Target:    ", Style::default().fg(THEME.text_muted)),
                            Span::styled(&mr.target_branch, Style::default().fg(THEME.purple)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("State:     ", Style::default().fg(THEME.text_muted)),
                            Span::styled(
                                if mr.state == "opened" { "OPEN" } else if mr.state == "merged" { "MERGED" } else { "CLOSED" },
                                Style::default().fg(if mr.state == "opened" { THEME.green } else if mr.state == "merged" { THEME.purple } else { THEME.red }).add_modifier(Modifier::BOLD)
                            ),
                            Span::styled(" (", Style::default().fg(THEME.text_muted)),
                            Span::styled(draft_status, Style::default().fg(draft_color).add_modifier(Modifier::BOLD)),
                            Span::styled(")", Style::default().fg(THEME.text_muted)),
                        ]));
                        text.push(Line::from(vec![
                            Span::styled("Updated:   ", Style::default().fg(THEME.text_muted)),
                            Span::styled(time_ago(&mr.updated_at), Style::default().fg(THEME.yellow)),
                        ]));
                        text.push(Line::from(""));
                        text.push(Line::from(vec![
                            Span::styled("Labels:    ", Style::default().fg(THEME.text_muted)),
                            Span::styled(labels, Style::default().fg(THEME.purple)),
                        ]));
                        if let Some(desc) = &mr.description {
                            if !desc.trim().is_empty() {
                                text.push(Line::from(""));
                                text.push(Line::from(vec![
                                    Span::styled("Description:", Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD)),
                                ]));
                                text.extend(render_markdown(desc));
                            }
                        }

                        let viewport_height = middle_chunks[2].height.saturating_sub(2) as usize;
                        let content_length = text.len();
                        let max_scroll = content_length.saturating_sub(viewport_height) as u16;
                        app.mrs_scroll = app.mrs_scroll.min(max_scroll);

                        let title_suffix = if content_length > viewport_height {
                            let percent = (app.mrs_scroll as usize * 100) / max_scroll.max(1) as usize;
                            format!(" [Shift+J/K | {}%] ", percent.min(100))
                        } else {
                            String::new()
                        };

                        let preview_block = Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(THEME.border))
                            .title(format!(" Details{} ", title_suffix))
                            .title_style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD));

                        f.render_widget(
                            Paragraph::new(text)
                                .block(preview_block)
                                .wrap(ratatui::widgets::Wrap { trim: true })
                                .scroll((app.mrs_scroll, 0)),
                            middle_chunks[2]
                        );
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
                f.render_widget(Paragraph::new("Select a pipeline to view details...").block(Block::default().borders(Borders::ALL).title(" Details ").border_style(Style::default().fg(THEME.border))).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
            } else {
                if let Some(jobs) = &app.selected_pipeline_jobs {
                    let rows = jobs.iter().enumerate().map(|(i, j)| {
                        let (status_text, status_color, bg_color) = match j.status.as_str() {
                            "success" => (" SUCCESS ", THEME.green, THEME.green_bg),
                            "failed" => ("  FAILED ", THEME.red, THEME.red_bg),
                            "running" => (" RUNNING ", THEME.blue, THEME.blue_bg),
                            "canceled" => ("CANCELED ", THEME.text_muted, THEME.inactive_bg),
                            "pending" => (" PENDING ", THEME.yellow, THEME.yellow_bg),
                            "skipped" => (" SKIPPED ", THEME.text_muted, THEME.inactive_bg),
                            "manual" => ("  MANUAL ", THEME.text_muted, THEME.inactive_bg),
                            _ => (" UNKNOWN ", THEME.text_muted, THEME.inactive_bg),
                        };
                        let style = if Some(i) == app.selected_job_index {
                            Style::default().bg(THEME.highlight_bg).fg(THEME.bg).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        };
                        let is_selected = app.selected_jobs.contains(&j.id);
                        let id_prefix = if is_selected { "[x] " } else { "[ ] " };
                        Row::new(vec![
                            Cell::from(format!("{}{}", id_prefix, j.id)),
                            Cell::from(j.stage.clone()),
                            Cell::from(status_text).style(Style::default().fg(status_color).bg(bg_color).add_modifier(Modifier::BOLD)),
                            Cell::from(j.name.clone()),
                        ]).style(style).height(1)
                    });

                    let widths = [
                        Constraint::Length(14),
                        Constraint::Length(15),
                        Constraint::Length(12),
                        Constraint::Percentage(60),
                    ];

                    let table = Table::new(rows, widths)
                        .header(Row::new(vec!["ID", "Stage", "Status", "Name"]).style(header_style).height(1))
                        .block(Block::default()
                            .borders(Borders::ALL)
                            .title(" Jobs (Esc to go back) ")
                            .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD))
                            .border_style(Style::default().fg(THEME.border_focused)));
                    
                    let mut state = app.jobs_list_state.clone();
                    f.render_stateful_widget(table, middle_chunks[1], &mut state);
                    app.jobs_list_state = state;

                    if let Some(trace) = &app.job_trace {
                        let width = middle_chunks[2].width.saturating_sub(2) as usize;
                        let height = middle_chunks[2].height.saturating_sub(2) as usize;
                        let total_lines = count_wrapped_lines(trace, width);
                        let max_scroll = total_lines.saturating_sub(height) as u16;

                        if app.job_trace_needs_scroll_to_bottom {
                            app.job_trace_scroll = max_scroll;
                            app.job_trace_needs_scroll_to_bottom = false;
                        } else {
                            app.job_trace_scroll = app.job_trace_scroll.min(max_scroll);
                        }

                        let title_suffix = if total_lines > height {
                            let percent = (app.job_trace_scroll as usize * 100) / max_scroll.max(1) as usize;
                            format!(" [j/k | {}%] ", percent.min(100))
                        } else {
                            String::new()
                        };

                        let preview_block = Block::default()
                            .borders(Borders::ALL)
                            .title(format!(" Details / Trace{} ", title_suffix))
                            .title_style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD))
                            .border_style(Style::default().fg(THEME.border));

                        f.render_widget(
                            Paragraph::new(trace.as_str())
                                .block(preview_block)
                                .wrap(ratatui::widgets::Wrap { trim: false })
                                .scroll((app.job_trace_scroll, 0)),
                            middle_chunks[2],
                        );
                    } else {
                        let preview_block = Block::default()
                            .borders(Borders::ALL)
                            .title(" Details / Trace ")
                            .title_style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD))
                            .border_style(Style::default().fg(THEME.border));
                        let mut text = Vec::new();
                        text.push(Line::from(vec![
                            Span::styled("Stages Success Rate:", Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD)),
                        ]));
                        text.push(Line::from(""));
                        append_stage_summaries(&mut text, jobs);
                        f.render_widget(Paragraph::new(text).block(preview_block), middle_chunks[2]);
                    }
                } else {
                    let filtered_pipelines = App::filter_pipelines_list(&app.pipelines.items, &app.search_query, &app.pipeline_jobs);
                        
                    let rows = filtered_pipelines.iter().enumerate().map(|(idx, p)| {
                        let (status_text, status_color, bg_color) = match p.status.as_str() {
                            "success" => (" SUCCESS ", THEME.green, THEME.green_bg),
                            "failed" => ("  FAILED ", THEME.red, THEME.red_bg),
                            "running" => (" RUNNING ", THEME.blue, THEME.blue_bg),
                            "canceled" => ("CANCELED ", THEME.text_muted, THEME.inactive_bg),
                            "pending" => (" PENDING ", THEME.yellow, THEME.yellow_bg),
                            "skipped" => (" SKIPPED ", THEME.text_muted, THEME.inactive_bg),
                            "manual" => ("  MANUAL ", THEME.text_muted, THEME.inactive_bg),
                            _ => (" UNKNOWN ", THEME.text_muted, THEME.inactive_bg),
                        };
                        let stages_dots = if let Some(jobs) = app.pipeline_jobs.get(&p.id) {
                            get_stages_dots(jobs)
                        } else {
                            "⏳".to_string()
                        };
                        let is_checked = app.selected_pipelines.contains(&p.id);
                        let id_prefix = if is_checked { "[x] " } else { "[ ] " };
                        let is_row_highlighted = app.pipelines.state.selected() == Some(idx);
                        Row::new(vec![
                            render_fuzzy_cell(&format!("{}#{}", id_prefix, p.id), &app.search_query, is_row_highlighted, Style::default().fg(THEME.text_normal)),
                            Cell::from(status_text).style(Style::default().fg(status_color).bg(bg_color).add_modifier(Modifier::BOLD)),
                            Cell::from(stages_dots),
                            render_fuzzy_cell(&truncate(&format_ref(&p.r#ref), 100), &app.search_query, is_row_highlighted, Style::default().fg(THEME.purple)),
                            Cell::from(time_ago(&p.updated_at)).style(Style::default().fg(THEME.yellow)),
                        ]).height(1)
                    });

                    let widths = [
                        Constraint::Length(14),
                        Constraint::Length(12),
                        Constraint::Length(24),
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
                        .title(" Details ")
                        .title_style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD))
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
                                Span::styled(format_ref(&p.r#ref), Style::default().fg(THEME.purple)),
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
                                append_stage_summaries(&mut text, jobs);
                            } else {
                                text.push(Line::from(vec![
                                    Span::styled("Loading stages...", Style::default().fg(THEME.text_muted).add_modifier(Modifier::ITALIC)),
                                ]));
                            }
                            text.push(Line::from(""));
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
                f.render_widget(Paragraph::new("Select a runner to view details...").block(Block::default().borders(Borders::ALL).title(" Details ").border_style(Style::default().fg(THEME.border))).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
            } else {
                let filtered_runners = App::filter_runners_list(&app.runners.items, &app.search_query);
                
                let rows = filtered_runners.iter().enumerate().map(|(idx, r)| {
                    let (status_text, status_color, bg_color) = match r.status.as_str() {
                        "online" => (" ONLINE  ", THEME.green, THEME.green_bg),
                        "paused" => (" PAUSED  ", THEME.yellow, THEME.yellow_bg),
                        "offline" => (" OFFLINE ", THEME.red, THEME.red_bg),
                        _ => (" UNKNOWN ", THEME.text_muted, THEME.inactive_bg),
                    };
                    let desc = r.description.as_deref().unwrap_or("No description");
                    let is_row_highlighted = app.runners.state.selected() == Some(idx);
                    Row::new(vec![
                        render_fuzzy_cell(&r.id.to_string(), &app.search_query, is_row_highlighted, Style::default().fg(THEME.text_normal)),
                        render_fuzzy_cell(&truncate(desc, 100), &app.search_query, is_row_highlighted, Style::default().fg(THEME.text_normal)),
                        Cell::from(status_text).style(Style::default().fg(status_color).bg(bg_color).add_modifier(Modifier::BOLD)),
                        Cell::from(r.active.to_string()).style(Style::default().fg(if r.active { THEME.green } else { THEME.red })),
                    ]).height(1)
                });

                let widths = [
                    Constraint::Length(12),
                    Constraint::Percentage(55),
                    Constraint::Length(14),
                    Constraint::Length(10),
                ];

                let table = Table::new(rows, widths)
                    .header(Row::new(vec!["ID", "Description", "Status", "Active"]).style(header_style).height(1))
                    .block(main_block)
                    .row_highlight_style(highlight_style)
                    .highlight_symbol(" ❯ ");
                
                f.render_stateful_widget(table, middle_chunks[1], &mut app.runners.state);

                let preview_block = Block::default()
                    .borders(Borders::ALL)
                    .title(" Details ")
                    .title_style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD))
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
                f.render_widget(Paragraph::new("Select a release to view details...").block(Block::default().borders(Borders::ALL).title(" Details ").border_style(Style::default().fg(THEME.border))).style(Style::default().fg(THEME.text_muted)), middle_chunks[2]);
            } else {
                let filtered_releases = App::filter_releases_list(&app.releases.items, &app.search_query);
                
                let rows = filtered_releases.iter().enumerate().map(|(idx, r)| {
                    let is_row_highlighted = app.releases.state.selected() == Some(idx);
                    Row::new(vec![
                        render_fuzzy_cell(&r.tag_name, &app.search_query, is_row_highlighted, Style::default().fg(THEME.green).add_modifier(Modifier::BOLD)),
                        render_fuzzy_cell(&truncate(&r.name, 100), &app.search_query, is_row_highlighted, Style::default().fg(THEME.text_normal)),
                        render_fuzzy_cell(&truncate(&r.released_at, 10), &app.search_query, is_row_highlighted, Style::default().fg(THEME.yellow)),
                    ]).height(1)
                });

                let widths = [
                    Constraint::Length(20),
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
                    .title(" Details ")
                    .title_style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD))
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



    // Error Popup overlay
    if let Some(err) = &app.error_message {
        let block = Block::default()
            .title(" Error ")
            .title_style(Style::default().fg(THEME.red).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .style(Style::default().fg(THEME.red).bg(Color::Reset));
        let paragraph = Paragraph::new(err.clone())
            .block(block)
            .alignment(Alignment::Center);
        
        let area = centered_rect(60, 20, size);
        f.render_widget(Clear, area); //this clears out the background
        f.render_widget(paragraph, area);
    }

    if let Some(menu) = &mut app.edit_menu {
        let block = Block::default()
            .title(format!(" {} ", menu.title))
            .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.border_focused))
            .style(Style::default().bg(Color::Reset));
            
        let area = centered_rect(50, 45, size);
        
        let items: Vec<ListItem> = menu.fields.iter().enumerate().map(|(i, (label, val))| {
            let style = if i == menu.selected_idx {
                Style::default().bg(THEME.highlight_bg).fg(THEME.bg).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(THEME.text_normal)
            };
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(format!("  {:20} ", label), Style::default().fg(THEME.text_muted)),
                    Span::styled(" ❯ ", Style::default().fg(THEME.text_muted)),
                    Span::styled(val, style),
                ])
            ])
        }).collect();
        
        let list = List::new(items)
            .block(block)
            .style(Style::default().bg(Color::Reset));
            
        f.render_widget(Clear, area);
        let mut state = menu.state.clone();
        f.render_stateful_widget(list, area, &mut state);
        menu.state = state;
    }

    if let Some(selector) = &mut app.selector {
        let block = Block::default()
            .title(format!(" {} ", selector.title))
            .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.border_focused))
            .style(Style::default().bg(Color::Reset));
            
        let area = centered_rect(50, 60, size);
        
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3), // Search/Filter
                Constraint::Min(0),    // List of items
                Constraint::Length(2), // Help/Info footer
            ].as_ref())
            .split(area);
            
        let border_color = if selector.is_filtering { THEME.border_focused } else { THEME.text_muted };
        let search_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(" Filter (press 'f' or '/' to focus) ");
            
        let search_text = if selector.is_filtering {
            format!("{}▋", selector.search_query)
        } else if selector.search_query.is_empty() {
            "Type to filter...".to_string()
        } else {
            selector.search_query.clone()
        };
        
        let search_style = if selector.search_query.is_empty() && !selector.is_filtering {
            Style::default().fg(THEME.text_muted).add_modifier(Modifier::ITALIC)
        } else {
            Style::default().fg(THEME.text_normal)
        };
        
        let search_p = Paragraph::new(search_text)
            .block(search_block)
            .style(search_style);
            
        let footer_text = if selector.is_filtering {
            "  Esc/Enter: Stop filtering • Backspace: Delete  "
        } else {
            "  j/k: Navigate • Space: Toggle • Enter: Save & Exit • f: Filter • Esc: Back  "
        };
        let footer_p = Paragraph::new(footer_text)
            .style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::ITALIC));
            
        f.render_widget(Clear, area);
        f.render_widget(block, area);
        f.render_widget(search_p, chunks[0]);
        
        if selector.is_loading {
            let p = Paragraph::new("\n  Loading options from GitLab...")
                .style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::ITALIC));
            f.render_widget(p, chunks[1]);
        } else {
            let filtered_items = selector.get_filtered_items_with_indices();
            if filtered_items.is_empty() {
                let p = Paragraph::new("\n  No matching options found.")
                    .style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::ITALIC));
                f.render_widget(p, chunks[1]);
            } else {
                let items: Vec<ListItem> = filtered_items.iter().enumerate().map(|(i, (item, indices))| {
                    let is_selected = if item.starts_with("+ Create \"") {
                        let clean_val = selector.search_query.trim().to_string();
                        selector.selected_items.contains(&clean_val)
                    } else {
                        selector.selected_items.contains(item)
                    };
                    
                    let marker = if is_selected { " ▣ " } else { " ☐ " };
                    let marker_color = if is_selected { THEME.border_focused } else { THEME.text_muted };
                    
                    let style = if i == selector.cursor_idx {
                        Style::default().bg(THEME.highlight_bg).fg(THEME.bg).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(THEME.text_normal)
                    };
                    
                    let highlight_style = if i == selector.cursor_idx {
                        Style::default().bg(THEME.highlight_bg).fg(THEME.yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(THEME.yellow).add_modifier(Modifier::BOLD)
                    };
                    
                    let mut line_spans = vec![
                        Span::styled(marker, Style::default().fg(marker_color).add_modifier(Modifier::BOLD))
                    ];
                    
                    if let Some(indices) = indices {
                        line_spans.extend(highlight_fuzzy_match(item, indices, style, highlight_style));
                    } else {
                        line_spans.push(Span::styled(item.clone(), style));
                    }
                    
                    ListItem::new(vec![Line::from(line_spans)])
                }).collect();
                
                let list = List::new(items).style(Style::default().bg(Color::Reset));
                let mut state = selector.state.clone();
                f.render_stateful_widget(list, chunks[1], &mut state);
                selector.state = state;
            }
        }
        f.render_widget(footer_p, chunks[2]);
    }

    if let Some(text_input) = &app.text_input {
        let block = Block::default()
            .title(format!(" {} ", text_input.title))
            .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.border_focused))
            .style(Style::default().bg(Color::Reset));
            
        let area = centered_rect(40, 15, size);
        
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Min(0),    // Value input line
                Constraint::Length(1), // Help footer
            ].as_ref())
            .split(area);
            
        let mut display_val = text_input.value.clone();
        if text_input.cursor_idx <= display_val.len() {
            display_val.insert(text_input.cursor_idx, '▋');
        } else {
            display_val.push('▋');
        }
        
        let value_p = Paragraph::new(display_val)
            .style(Style::default().fg(THEME.text_normal));
            
        let footer_p = Paragraph::new("  Enter: Confirm • Esc: Cancel  ")
            .style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::ITALIC));
            
        f.render_widget(Clear, area);
        f.render_widget(block, area);
        f.render_widget(value_p, chunks[0]);
        f.render_widget(footer_p, chunks[1]);
    }

    if app.show_help {
        struct Shortcut {
            category: &'static str,
            key: &'static str,
            action: &'static str,
        }

        let shortcuts = [
            Shortcut { category: "Global & Nav", key: "Tab / l / →", action: "Next tab" },
            Shortcut { category: "Global & Nav", key: "S-Tab / h / ←", action: "Previous tab" },
            Shortcut { category: "Global & Nav", key: "j / k / ↓ / ↑", action: "Select item / Scroll jobs" },
            Shortcut { category: "Global & Nav", key: "J / K", action: "Scroll description / trace" },
            Shortcut { category: "Global & Nav", key: "f / /", action: "Open fuzzy search / filter bar" },
            Shortcut { category: "Global & Nav", key: "F5 / Ctrl+R", action: "Refresh active tab data" },
            Shortcut { category: "Global & Nav", key: "? / F1", action: "Show this help modal" },
            Shortcut { category: "Global & Nav", key: "q / Esc", action: "Quit / Close overlay" },
            
            Shortcut { category: "Issues & MRs", key: "n", action: "Create new Issue / MR" },
            Shortcut { category: "Issues & MRs", key: "e", action: "Open parameter edit menu" },
            Shortcut { category: "Issues & MRs", key: "c", action: "Close selected Issue" },
            Shortcut { category: "Issues & MRs", key: "a", action: "Approve selected MR" },
            Shortcut { category: "Issues & MRs", key: "m", action: "Merge selected MR (squash + delete)" },
            Shortcut { category: "Issues & MRs", key: "s", action: "Toggle Draft / Ready status" },
            Shortcut { category: "Issues & MRs", key: "v", action: "View Merge Request diff changes" },
            
            Shortcut { category: "Pipelines", key: "Enter", action: "View jobs list / View job trace" },
            Shortcut { category: "Pipelines", key: "Esc / Backspc", action: "Go back (jobs -> pipes, trace -> jobs)" },
            Shortcut { category: "Pipelines", key: "p", action: "Trigger new pipeline from MR" },
            Shortcut { category: "Pipelines", key: "r", action: "Retry pipeline or selected job(s)" },
            Shortcut { category: "Pipelines", key: "c / d", action: "Cancel pipeline execution" },
            Shortcut { category: "Pipelines", key: "d", action: "Download pipeline job artifact" },
            Shortcut { category: "Pipelines", key: "e", action: "Open job trace in external $EDITOR" },
            Shortcut { category: "Pipelines", key: "Space", action: "Check / uncheck item for bulk retry" },
            
            Shortcut { category: "Other Tabs", key: "p / r", action: "Pause / Resume runner" },
            Shortcut { category: "Other Tabs", key: "e", action: "Edit runner description text" },
            Shortcut { category: "Other Tabs", key: "Enter", action: "View release notes in terminal" },
            Shortcut { category: "Other Tabs", key: "o", action: "Open MR/Pipeline/Release in browser" },
        ];

        let block = Block::default()
            .title(" Keyboard Shortcuts ")
            .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.border_focused))
            .border_type(BorderType::Double)
            .style(Style::default().bg(Color::Reset));

        let area = centered_rect_fixed(72, 37, size);

        let help_chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3), // Search / Filter
                Constraint::Min(0),    // Table
                Constraint::Length(1), // Help footer
            ].as_ref())
            .split(area);

        let border_color = if app.help_search_query.is_empty() { THEME.border } else { THEME.border_focused };
        let search_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(" Filter Shortcuts (Type to filter, Esc/Enter to exit) ")
            .title_style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::BOLD));
            
        let search_text = if app.help_search_query.is_empty() {
            "Type to search commands...▋".to_string()
        } else {
            format!("{}▋", app.help_search_query)
        };
        
        let search_style = if app.help_search_query.is_empty() {
            Style::default().fg(THEME.text_muted).add_modifier(Modifier::ITALIC)
        } else {
            Style::default().fg(THEME.text_normal)
        };
        
        let search_p = Paragraph::new(search_text)
            .style(search_style)
            .block(search_block);

        let rows: Vec<Row> = if app.help_search_query.is_empty() {
            vec![
                Row::new(vec![Cell::from(""), Cell::from(""), Cell::from("")]),
                
                // Section 1: Global & Navigation
                Row::new(vec![
                    Cell::from(Span::styled("Global & Nav", Style::default().fg(THEME.purple).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Tab / l / →", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Next tab", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("S-Tab / h / ←", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Previous tab", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("j / k / ↓ / ↑", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Select item / Scroll jobs", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("J / K", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Scroll description / trace", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("f / /", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Open fuzzy search / filter bar", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("F5 / Ctrl+R", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Refresh active tab data", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("? / F1", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Show this help modal", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("q / Esc", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Quit / Close overlay", Style::default().fg(THEME.text_normal))),
                ]),
                
                Row::new(vec![Cell::from(""), Cell::from(""), Cell::from("")]), // spacer

                // Section 2: Issues & MRs
                Row::new(vec![
                    Cell::from(Span::styled("Issues & MRs", Style::default().fg(THEME.purple).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("n", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Create new Issue / MR", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("e", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Open parameter edit menu", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("c", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Close selected Issue", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("a", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Approve selected MR", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("m", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Merge selected MR (squash + delete)", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("s", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Toggle Draft / Ready status", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("v", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("View Merge Request diff changes", Style::default().fg(THEME.text_normal))),
                ]),

                Row::new(vec![Cell::from(""), Cell::from(""), Cell::from("")]), // spacer

                // Section 3: Pipelines & Jobs
                Row::new(vec![
                    Cell::from(Span::styled("Pipelines", Style::default().fg(THEME.purple).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Enter", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("View jobs list / View job trace", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("Esc / Backspc", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Go back (jobs -> pipes, trace -> jobs)", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("p", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Trigger new pipeline from MR", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("r", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Retry pipeline or selected job(s)", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("c / d", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Cancel pipeline execution", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("d", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Download pipeline job artifact", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("e", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Open job trace in external $EDITOR", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("Space", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Check / uncheck item for bulk retry", Style::default().fg(THEME.text_normal))),
                ]),

                Row::new(vec![Cell::from(""), Cell::from(""), Cell::from("")]), // spacer

                // Section 4: Runners, Releases & Extras
                Row::new(vec![
                    Cell::from(Span::styled("Other Tabs", Style::default().fg(THEME.purple).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("p / r", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Pause / Resume runner", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("e", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Edit runner description text", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("Enter", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("View release notes in terminal", Style::default().fg(THEME.text_normal))),
                ]),
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Span::styled("o", Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                    Cell::from(Span::styled("Open MR/Pipeline/Release in browser", Style::default().fg(THEME.text_normal))),
                ]),
            ]
        } else {
            let query = app.help_search_query.to_lowercase();
            shortcuts.iter()
                .filter(|s| {
                    s.category.to_lowercase().contains(&query)
                        || s.key.to_lowercase().contains(&query)
                        || s.action.to_lowercase().contains(&query)
                })
                .map(|s| {
                    Row::new(vec![
                        Cell::from(Span::styled(s.category, Style::default().fg(THEME.purple).add_modifier(Modifier::BOLD))),
                        Cell::from(Span::styled(s.key, Style::default().fg(THEME.text_normal).add_modifier(Modifier::BOLD))),
                        Cell::from(Span::styled(s.action, Style::default().fg(THEME.text_normal))),
                    ])
                })
                .collect()
        };

        let widths = [
            Constraint::Length(16),
            Constraint::Length(18),
            Constraint::Min(0),
        ];

        let header_style = Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD);
        let table = Table::new(rows, widths)
            .header(Row::new(vec![
                Cell::from(Span::styled("Category", header_style)),
                Cell::from(Span::styled("Key", header_style)),
                Cell::from(Span::styled("Action", header_style)),
            ]).height(1))
            .block(block)
            .row_highlight_style(Style::default())
            .column_spacing(2);

        let footer_p = Paragraph::new(" Press Esc or Enter to close ")
            .alignment(Alignment::Center)
            .style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::ITALIC));

        f.render_widget(Clear, area);
        f.render_widget(search_p, help_chunks[0]);
        f.render_widget(table, help_chunks[1]);
        f.render_widget(footer_p, help_chunks[2]);
    }

    if app.diff_loading {
        let area = centered_rect(50, 20, size);
        let block = Block::default()
            .title(" Fetching Diff ")
            .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.border_focused))
            .style(Style::default().bg(Color::Reset));
        
        let text = vec![
            Line::from(""),
            Line::from(Span::styled("   Fetching Pull Request / Merge Request Diff...", Style::default().fg(THEME.text_normal))),
            Line::from(Span::styled("   Please wait, running CLI tool in background...", Style::default().fg(THEME.text_muted))),
            Line::from(""),
        ];
        
        let paragraph = Paragraph::new(text)
            .block(block)
            .alignment(Alignment::Left);
            
        f.render_widget(Clear, area);
        f.render_widget(paragraph, area);
    }

    if let Some(diff_view) = app.diff_view.take() {
        let area = centered_rect(95, 95, size);
        
        let outer_block = Block::default()
            .title(format!(" Pull Request / Merge Request Diff #{} ", diff_view.mr_iid))
            .title_style(Style::default().fg(THEME.header_fg).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.border))
            .style(Style::default().bg(Color::Reset));
            
        let inner_area = outer_block.inner(area);
        
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),    // Main content split
                Constraint::Length(1), // Help / controls footer
            ].as_ref())
            .split(inner_area);
            
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25), // Files list
                Constraint::Percentage(75), // Diff content
            ].as_ref())
            .split(chunks[0]);
            
        // 1. Render Files list on the left
        let files_block = Block::default()
            .title(" Files ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if diff_view.focus_on_files {
                THEME.border_focused
            } else {
                THEME.border
            }));
            
        let mut file_items = Vec::new();
        for (i, node) in diff_view.visible_nodes.iter().enumerate() {
            let is_selected = i == diff_view.selected_visible_idx;
            
            let indent = "  ".repeat(node.depth);
            let indicator = if node.is_dir {
                if node.is_expanded { "- " } else { "+ " }
            } else {
                "  "
            };
            
            let display_str = format!(" {}{}{}", indent, indicator, node.name);
            
            let item_style = if is_selected {
                if diff_view.focus_on_files {
                    Style::default().bg(THEME.highlight_bg).fg(THEME.bg).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().bg(THEME.border).fg(THEME.text_normal)
                }
            } else if node.is_dir {
                Style::default().fg(THEME.blue).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(THEME.text_normal)
            };
            
            file_items.push(ListItem::new(display_str).style(item_style));
        }
        let files_list = List::new(file_items).block(files_block);
        
        // 2. Render Diff content on the right
        let diff_block = Block::default()
            .title(" Diff ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if !diff_view.focus_on_files {
                THEME.border_focused
            } else {
                THEME.border
            }));
            
        let list_height = (main_chunks[1].height as usize).saturating_sub(2);
        
        let mut updated_diff_view = diff_view;
        if updated_diff_view.cursor_idx < updated_diff_view.scroll_offset {
            updated_diff_view.scroll_offset = updated_diff_view.cursor_idx;
        } else if updated_diff_view.cursor_idx >= updated_diff_view.scroll_offset + list_height {
            updated_diff_view.scroll_offset = updated_diff_view.cursor_idx - list_height + 1;
        }
        
        let start = updated_diff_view.scroll_offset;
        let end = (start + list_height).min(updated_diff_view.lines.len());
        
        let mut list_lines = Vec::new();
        for idx in start..end {
            let line = &updated_diff_view.lines[idx];
            let is_cursor = idx == updated_diff_view.cursor_idx;
            
            let old_str = line.old_line_num.map(|n| n.to_string()).unwrap_or_else(|| " ".to_string());
            let new_str = line.new_line_num.map(|n| n.to_string()).unwrap_or_else(|| " ".to_string());
            
            let num_style = Style::default().fg(THEME.text_muted);
            let mut line_spans = vec![
                Span::styled(if is_cursor { " ❯ " } else { "   " }, Style::default().fg(THEME.yellow).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:>4} ", old_str), num_style),
                Span::styled(format!("{:>4} │ ", new_str), num_style),
            ];
            
            let content_style = match line.line_type {
                crate::app::DiffLineType::Addition => Style::default().fg(Color::Rgb(140, 220, 140)).bg(Color::Rgb(20, 45, 25)),
                crate::app::DiffLineType::Deletion => Style::default().fg(Color::Rgb(220, 140, 140)).bg(Color::Rgb(50, 20, 25)),
                crate::app::DiffLineType::Meta => Style::default().fg(THEME.blue).add_modifier(Modifier::BOLD),
                crate::app::DiffLineType::HunkHeader => Style::default().fg(THEME.purple).add_modifier(Modifier::BOLD),
                crate::app::DiffLineType::Normal => Style::default().fg(THEME.text_normal),
            };
            
            let final_content_style = if is_cursor {
                content_style.add_modifier(Modifier::UNDERLINED).add_modifier(Modifier::BOLD)
            } else {
                content_style
            };
            
            line_spans.push(Span::styled(&line.content, final_content_style));
            list_lines.push(Line::from(line_spans));
        }
        
        let diff_para = Paragraph::new(list_lines).block(diff_block);
        
        let footer_p = Paragraph::new(" Esc/q: Exit • Tab: Toggle Focus • h/l/Left/Right: Switch Panels • j/k/↑/↓: Navigate • J/K: Next/Prev Hunk • c: Comment on Line ")
            .alignment(Alignment::Center)
            .style(Style::default().fg(THEME.text_muted).add_modifier(Modifier::ITALIC));
            
        f.render_widget(Clear, area);
        f.render_widget(outer_block, area);
        f.render_widget(files_list, main_chunks[0]);
        f.render_widget(diff_para, main_chunks[1]);
        f.render_widget(footer_p, chunks[1]);
        
        app.diff_view = Some(updated_diff_view);
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

fn centered_rect_fixed(width: u16, height: u16, r: Rect) -> Rect {
    let w = width.min(r.width);
    let h = height.min(r.height);
    let x = r.x + (r.width - w) / 2;
    let y = r.y + (r.height - h) / 2;
    Rect::new(x, y, w, h)
}

pub fn count_wrapped_lines(text: &str, width: usize) -> usize {
    if width == 0 {
        return 0;
    }
    let mut total_lines = 0;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            total_lines += 1;
            continue;
        }
        let mut current_line_len = 0;
        let mut first = true;
        for word in line.split(' ') {
            let word_len = word.chars().count();
            if word_len == 0 {
                if current_line_len + 1 > width {
                    total_lines += 1;
                    current_line_len = 1;
                } else {
                    current_line_len += 1;
                }
                continue;
            }
            
            let space_needed = if first { 0 } else { 1 };
            if current_line_len + space_needed + word_len <= width {
                current_line_len += space_needed + word_len;
                first = false;
            } else {
                if word_len > width {
                    if current_line_len + space_needed < width {
                        let remaining_on_current = width - (current_line_len + space_needed);
                        let mut remaining_word_len = word_len - remaining_on_current;
                        total_lines += 1;
                        while remaining_word_len > width {
                            total_lines += 1;
                            remaining_word_len -= width;
                        }
                        current_line_len = remaining_word_len;
                    } else {
                        total_lines += 1;
                        let mut remaining_word_len = word_len;
                        while remaining_word_len > width {
                            total_lines += 1;
                            remaining_word_len -= width;
                        }
                        current_line_len = remaining_word_len;
                    }
                } else {
                    total_lines += 1;
                    current_line_len = word_len;
                }
                first = false;
            }
        }
        total_lines += 1;
    }
    
    if text.is_empty() {
        return 0;
    }
    total_lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::pipelines::Job;

    #[test]
    fn test_get_stages_summary() {
        // Test case 1: Stage with mixed success and skipped jobs
        // This stage should be reported as "success" status, and 100% success rate.
        let jobs = vec![
            Job {
                id: 1,
                stage: "build".to_string(),
                name: "compile".to_string(),
                status: "success".to_string(),
            },
            Job {
                id: 2,
                stage: "build".to_string(),
                name: "cache".to_string(),
                status: "skipped".to_string(),
            },
            Job {
                id: 3,
                stage: "test".to_string(),
                name: "unit".to_string(),
                status: "failed".to_string(),
            },
            Job {
                id: 4,
                stage: "test".to_string(),
                name: "integration".to_string(),
                status: "success".to_string(),
            },
        ];

        let summaries = get_stages_summary(&jobs);
        assert_eq!(summaries.len(), 2);

        // Build stage verification (success + skipped = success/100%)
        let build = summaries.iter().find(|s| s.name == "build").unwrap();
        assert_eq!(build.status, "success");
        assert_eq!(build.success, 2);
        assert_eq!(build.total, 2);
        assert_eq!(build.percent, 100);

        // Test stage verification (failed + success = failed/50%)
        let test = summaries.iter().find(|s| s.name == "test").unwrap();
        assert_eq!(test.status, "failed");
        assert_eq!(test.success, 1);
        assert_eq!(test.total, 2);
        assert_eq!(test.percent, 50);
    }

    #[test]
    fn test_count_wrapped_lines() {
        assert_eq!(count_wrapped_lines("", 10), 0);
        assert_eq!(count_wrapped_lines("", 0), 0);
        assert_eq!(count_wrapped_lines("hello", 10), 1);
        assert_eq!(count_wrapped_lines("hello", 5), 1);
        assert_eq!(count_wrapped_lines("hello", 3), 2);
        assert_eq!(count_wrapped_lines("hello world", 5), 2);
        assert_eq!(count_wrapped_lines("a b c", 3), 2);
        assert_eq!(count_wrapped_lines("hello\nworld", 10), 2);
        assert_eq!(count_wrapped_lines("hello\r\nworld", 10), 2);
    }
}
