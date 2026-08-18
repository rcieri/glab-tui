#![allow(dead_code)]

use ratatui::{
    layout::Alignment,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Cell,
};

use crate::config::{Icons, THEME, Theme};
use crate::utils::format::truncate;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use std::collections::HashMap;

pub(crate) fn highlight_fuzzy_match(
    text: &str,
    indices: &[usize],
    base_style: Style,
    highlight_style: Style,
) -> Vec<Span<'static>> {
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
                    if is_highlighted {
                        highlight_style
                    } else {
                        base_style
                    },
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
            if is_highlighted {
                highlight_style
            } else {
                base_style
            },
        ));
    }

    spans
}

pub(crate) fn get_label_color(label: &str, label_colors: &HashMap<String, Color>) -> Color {
    // Prefer the label's real color from the API when it is dark enough to
    // read as foreground text on the active theme background.
    if let Some(&c) = label_colors.get(label) {
        if !is_light_color(c) {
            return c;
        }
    }
    let mut hash: u32 = 5381;
    for c in label.bytes() {
        hash = ((hash << 5).wrapping_add(hash)).wrapping_add(c as u32);
    }
    let palette = crate::config::THEME.read().unwrap().label_palette;
    let idx = (hash % (palette.len() as u32)) as usize;
    palette[idx]
}

/// GitHub label colors are designed as background fills; the lightest ones are
/// unreadable as foreground text on a dark theme, so they fall back to the
/// theme palette instead.
fn is_light_color(color: Color) -> bool {
    match color {
        Color::Rgb(r, g, b) => {
            let luminance = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
            luminance > 180.0
        }
        _ => false,
    }
}

pub(crate) fn floor_char_boundary(s: &str, mut index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    while !s.is_char_boundary(index) {
        index -= 1;
    }
    index
}

pub(crate) fn render_labels_cell(
    labels: &[String],
    label_colors: &HashMap<String, Color>,
    query: &str,
    is_selected: bool,
    is_checked: bool,
    max_len: usize,
) -> Cell<'static> {
    if labels.is_empty() {
        let mut style = Style::default().fg(THEME.read().unwrap().text_muted);
        if is_selected {
            style = style
                .bg(THEME.read().unwrap().highlight_bg)
                .add_modifier(Modifier::BOLD);
        } else if is_checked {
            style = style.bg(THEME.read().unwrap().checked_bg);
        }
        return Cell::from(Line::from("—").alignment(Alignment::Left)).style(style);
    }

    let mut char_styles: Vec<(char, Style)> = Vec::new();
    let mut current_len = 0;

    let base_bg = if is_selected {
        Some(THEME.read().unwrap().highlight_bg)
    } else if is_checked {
        Some(THEME.read().unwrap().checked_bg)
    } else {
        None
    };

    for (idx, label) in labels.iter().enumerate() {
        if current_len >= max_len {
            break;
        }
        if idx > 0 {
            let comma = ", ";
            if current_len + comma.len() > max_len {
                let mut style = Style::default().fg(THEME.read().unwrap().text_muted);
                if let Some(bg) = base_bg {
                    style = style.bg(bg);
                }
                char_styles.push(('…', style));
                break;
            }
            let mut style = Style::default().fg(THEME.read().unwrap().text_normal);
            if let Some(bg) = base_bg {
                style = style.bg(bg);
            }
            for c in comma.chars() {
                char_styles.push((c, style));
            }
            current_len += comma.len();
        }

        let label_color = get_label_color(label, label_colors);
        let mut label_style = Style::default()
            .fg(label_color)
            .add_modifier(Modifier::BOLD);
        if let Some(bg) = base_bg {
            label_style = label_style.bg(bg);
        }

        let mut text_to_add = label.as_str();
        let mut truncated = false;
        if current_len + text_to_add.len() > max_len {
            let allowed = max_len - current_len;
            if allowed > 1 {
                // Snap to a valid char boundary to avoid panicking on multi-byte
                // characters (emojis, accented letters, etc.).
                let safe_end = floor_char_boundary(text_to_add, allowed - 1);
                text_to_add = &text_to_add[..safe_end];
                truncated = true;
            } else {
                let mut style = Style::default().fg(THEME.read().unwrap().text_muted);
                if let Some(bg) = base_bg {
                    style = style.bg(bg);
                }
                char_styles.push(('…', style));
                break;
            }
        }

        for c in text_to_add.chars() {
            char_styles.push((c, label_style));
        }
        current_len += text_to_add.len();

        if truncated {
            let mut style = Style::default().fg(THEME.read().unwrap().text_muted);
            if let Some(bg) = base_bg {
                style = style.bg(bg);
            }
            char_styles.push(('…', style));
            break;
        }
    }

    let concatenated_text: String = char_styles.iter().map(|(c, _)| *c).collect();
    let index_set: std::collections::HashSet<usize> = if query.trim().is_empty() {
        std::collections::HashSet::new()
    } else {
        let matcher = SkimMatcherV2::default();
        if let Some((_, indices)) = matcher.fuzzy_indices(&concatenated_text, query) {
            indices.into_iter().collect()
        } else {
            std::collections::HashSet::new()
        }
    };

    let mut spans = Vec::new();
    let mut current_chunk = String::new();
    let mut current_style = Style::default();
    let mut first = true;

    for (i, (c, mut style)) in char_styles.into_iter().enumerate() {
        if index_set.contains(&i) {
            style = style
                .fg(THEME.read().unwrap().yellow)
                .add_modifier(Modifier::BOLD);
        }

        if first {
            current_style = style;
            first = false;
        }

        if style != current_style {
            if !current_chunk.is_empty() {
                spans.push(Span::styled(current_chunk.clone(), current_style));
                current_chunk.clear();
            }
            current_style = style;
        }
        current_chunk.push(c);
    }

    if !current_chunk.is_empty() {
        spans.push(Span::styled(current_chunk, current_style));
    }

    let mut cell_style = Style::default();
    if let Some(bg) = base_bg {
        cell_style = cell_style.bg(bg);
    }

    Cell::from(Line::from(spans).alignment(Alignment::Left)).style(cell_style)
}

pub(crate) struct StageSummary {
    pub(crate) name: String,
    pub(crate) success: usize,
    pub(crate) total: usize,
    pub(crate) percent: usize,
    pub(crate) status: String,
}

pub(crate) fn get_stages_summary(jobs: &[crate::domain::pipelines::Job]) -> Vec<StageSummary> {
    let mut stage_names = Vec::new();
    let mut stage_jobs = std::collections::HashMap::new();
    for j in jobs {
        let stage_name = j.stage().to_string();
        if !stage_names.contains(&stage_name) {
            stage_names.push(stage_name.clone());
        }
        stage_jobs
            .entry(stage_name)
            .or_insert_with(Vec::new)
            .push(j.status().to_string());
    }

    let mut summaries = Vec::new();
    for stage in stage_names {
        if let Some(statuses) = stage_jobs.get(&stage) {
            let total = statuses.len();
            let success = statuses
                .iter()
                .filter(|s| *s == "success" || *s == "skipped")
                .count();
            let percent = if total > 0 {
                (success * 100) / total
            } else {
                0
            };

            let stage_status = if statuses.iter().any(|s| s == "failed") {
                "failed".to_string()
            } else if statuses.iter().any(|s| s == "running") {
                "running".to_string()
            } else if statuses
                .iter()
                .any(|s| s == "pending" || s == "preparing" || s == "waiting_for_resource")
            {
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

pub(crate) fn get_stages_dots(jobs: &[crate::domain::pipelines::Job]) -> String {
    let icons = crate::config::ICONS.read().unwrap();
    let summaries = get_stages_summary(jobs);
    let mut dots = String::new();
    for s in summaries {
        let dot = match s.status.as_str() {
            "success" => &icons.dot_success,
            "failed" => &icons.dot_failed,
            "running" => &icons.dot_running,
            "canceled" => &icons.dot_canceled,
            "pending" => &icons.dot_pending,
            _ => &icons.dot_skipped,
        };
        dots.push_str(dot);
    }
    dots
}

pub(crate) fn append_stage_summaries(
    text: &mut Vec<Line<'static>>,
    jobs: &[crate::domain::pipelines::Job],
) {
    let summaries = get_stages_summary(jobs);
    for s in summaries {
        let status_color = match s.status.as_str() {
            "success" => THEME.read().unwrap().green,
            "failed" => THEME.read().unwrap().red,
            "running" => THEME.read().unwrap().blue,
            "pending" => THEME.read().unwrap().yellow,
            _ => THEME.read().unwrap().text_muted,
        };
        text.push(Line::from(vec![
            Span::styled(
                format!("{:15} ", truncate(&s.name, 15)),
                Style::default().fg(THEME.read().unwrap().text_normal),
            ),
            Span::styled(" ❯ ", Style::default().fg(THEME.read().unwrap().text_muted)),
            Span::styled(
                format!("{:>4} ", format!("{}%", s.percent)),
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({}/{})", s.success, s.total),
                Style::default().fg(THEME.read().unwrap().text_muted),
            ),
        ]));
    }
}

pub(crate) fn append_job_summaries(
    text: &mut Vec<Line<'static>>,
    jobs: &[crate::domain::pipelines::Job],
) {
    for job in jobs {
        let name = job.name().to_string();
        let status = job.status().to_string();
        let status_color = match status.as_str() {
            "success" => THEME.read().unwrap().green,
            "failed" => THEME.read().unwrap().red,
            "running" => THEME.read().unwrap().blue,
            "pending" => THEME.read().unwrap().yellow,
            _ => THEME.read().unwrap().text_muted,
        };
        let duration = job
            .duration_seconds()
            .map(|d| format!("{}m {}s", d / 60, d % 60))
            .unwrap_or_else(|| "-".to_string());
        let needs = if job.needs().is_empty() {
            String::new()
        } else {
            format!(" needs: {}", job.needs().join(", "))
        };
        text.push(Line::from(vec![
            Span::styled(
                format!("{} ", truncate(&name, 24)),
                Style::default().fg(THEME.read().unwrap().text_normal),
            ),
            Span::styled(
                status.to_uppercase(),
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}{}", duration, needs),
                Style::default().fg(THEME.read().unwrap().text_muted),
            ),
        ]));
    }
}

#[allow(dead_code)]
fn add_cmd(text: &mut Vec<Line<'static>>, key: &str, desc: &str) {
    let padded_key = format!(" {:^3} ", key);
    text.push(Line::from(vec![
        Span::styled(
            padded_key,
            Style::default()
                .bg(THEME.read().unwrap().border_focused)
                .fg(THEME.read().unwrap().bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {}", desc),
            Style::default().fg(THEME.read().unwrap().text_normal),
        ),
    ]));
}

pub(crate) fn build_log_line(cmd: &crate::app::TerminalCommand, width: usize) -> Line<'static> {
    let time_str = if cmd.timestamp.len() >= 8 {
        let parts: Vec<&str> = cmd.timestamp.split('T').collect();
        if parts.len() > 1 {
            parts[1].chars().take(8).collect::<String>()
        } else {
            cmd.timestamp.chars().take(8).collect::<String>()
        }
    } else {
        cmd.timestamp.clone()
    };

    let icons = crate::config::ICONS.read().unwrap();
    let (status_text, status_color) = match cmd.status.as_str() {
        "Success" => (
            format!("{} SUCCESS", icons.status_success),
            THEME.read().unwrap().green,
        ),
        "Running" => (
            format!("{} RUNNING", icons.status_running),
            THEME.read().unwrap().yellow,
        ),
        s if s.starts_with("Failed") => (
            format!("{} FAILED ", icons.status_failed),
            THEME.read().unwrap().red,
        ),
        _ => (
            format!("{} PENDING", icons.status_pending),
            THEME.read().unwrap().yellow,
        ),
    };

    let err_detail = if cmd.status.starts_with("Failed: ") {
        Some(&cmd.status[8..])
    } else if cmd.status.starts_with("Failed") && cmd.status.len() > 6 {
        Some(&cmd.status[6..])
    } else {
        None
    };

    let cmd_clean = cmd.command.trim();
    let mut desc = "";
    let mut cmd_to_run = cmd_clean;

    if let Some(pos) = cmd_clean.find(": ") {
        desc = &cmd_clean[..pos];
        cmd_to_run = &cmd_clean[pos + 2..];
    }

    let desc_str = if desc.is_empty() {
        cmd_clean.to_uppercase()
    } else {
        desc.to_uppercase()
    };

    let time_len = 11; // "[HH:MM:SS] "
    let status_len = 7; // "SUCCESS"
    let sep1_len = 3; // " • "
    let action_len = 25; // Action padded to 25 chars
    let sep2_len = 3; // " • "
    let err_len = err_detail.map(|d| d.len() + 3).unwrap_or(0); // " (Error)"

    let reserved = time_len + status_len + sep1_len + action_len + sep2_len + err_len;
    let max_api_width = width.saturating_sub(reserved);

    let truncated_api = truncate(cmd_to_run, max_api_width);

    let (cmd_bin, cmd_args) = if truncated_api.starts_with("glab") {
        ("glab", truncated_api[4..].to_string())
    } else if truncated_api.starts_with("gh") {
        ("gh", truncated_api[2..].to_string())
    } else if truncated_api.starts_with("git") {
        ("git", truncated_api[3..].to_string())
    } else {
        ("", truncated_api.clone())
    };

    let mut spans = vec![
        // 1. Time
        Span::styled(
            format!("[{}] ", time_str),
            Style::default().fg(THEME.read().unwrap().text_muted),
        ),
        // 2. Status
        Span::styled(
            format!("{: <7}", status_text),
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
        // 3. Sep1
        Span::styled(" • ", Style::default().fg(THEME.read().unwrap().text_muted)),
        // 4. Action
        Span::styled(
            format!("{: <25}", desc_str),
            Style::default()
                .fg(THEME.read().unwrap().blue)
                .add_modifier(Modifier::BOLD),
        ),
        // 5. Sep2
        Span::styled(" • ", Style::default().fg(THEME.read().unwrap().text_muted)),
    ];

    // 6. API
    if !cmd_bin.is_empty() {
        spans.push(Span::styled(
            cmd_bin,
            Style::default()
                .fg(THEME.read().unwrap().yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(
        cmd_args,
        Style::default().fg(THEME.read().unwrap().text_normal),
    ));

    // 7. Error Detail
    if let Some(detail) = err_detail {
        spans.push(Span::styled(
            format!(" ({})", detail),
            Style::default().fg(THEME.read().unwrap().red),
        ));
    }

    Line::from(spans)
}

pub(crate) fn render_fuzzy_cell(
    text: &str,
    query: &str,
    is_selected: bool,
    is_checked: bool,
    base_style: Style,
    alignment: Alignment,
) -> Cell<'static> {
    let mut styled_base = base_style;
    if is_selected {
        styled_base = styled_base
            .bg(THEME.read().unwrap().highlight_bg)
            .add_modifier(Modifier::BOLD);
    } else if is_checked {
        styled_base = styled_base.bg(THEME.read().unwrap().checked_bg);
    }
    let line = if query.trim().is_empty() {
        Line::from(text.to_string()).alignment(alignment)
    } else {
        let matcher = SkimMatcherV2::default();
        if let Some((_, indices)) = matcher.fuzzy_indices(text, query) {
            let mut highlight_style = Style::default()
                .fg(THEME.read().unwrap().yellow)
                .add_modifier(Modifier::BOLD);
            if let Some(bg) = styled_base.bg {
                highlight_style = highlight_style.bg(bg);
            }
            Line::from(highlight_fuzzy_match(
                text,
                &indices,
                styled_base,
                highlight_style,
            ))
            .alignment(alignment)
        } else {
            Line::from(text.to_string()).alignment(alignment)
        }
    };
    Cell::from(line).style(styled_base)
}

// ── Shared rendering helpers (used by detail panes AND edit menus) ──

/// Return a styled span for a CI/Job status string.
pub(crate) fn status_span(status: &str) -> Span<'static> {
    let theme = THEME.read().unwrap();
    let icons = crate::config::ICONS.read().unwrap();
    match status {
        "success" => Span::styled(
            format!("{} SUCCESS", icons.status_success),
            Style::default()
                .fg(theme.green)
                .add_modifier(Modifier::BOLD),
        ),
        "failed" => Span::styled(
            format!("{} FAILED", icons.status_failed),
            Style::default().fg(theme.red).add_modifier(Modifier::BOLD),
        ),
        "running" => Span::styled(
            format!("{} RUNNING", icons.status_running),
            Style::default().fg(theme.blue).add_modifier(Modifier::BOLD),
        ),
        "canceled" => Span::styled(
            format!("{} CANCEL", icons.status_canceled),
            Style::default()
                .fg(theme.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        "pending" | "preparing" => Span::styled(
            format!("{} PENDING", icons.status_pending),
            Style::default()
                .fg(theme.yellow)
                .add_modifier(Modifier::BOLD),
        ),
        "skipped" => Span::styled(
            format!("{} SKIP", icons.status_skipped),
            Style::default()
                .fg(theme.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        "manual" => Span::styled(
            format!("{} MANUAL", icons.status_manual),
            Style::default()
                .fg(theme.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        _ => Span::styled(
            format!("{} UNKNOWN", icons.status_unknown),
            Style::default()
                .fg(theme.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
    }
}

/// Single source of truth for badge styling shared by the inspector preview and
/// the table column renderers. Maps a field `label`/`val` pair to its
/// `(fg, badge_background, bold, formatted_text)` so the two render paths can
/// never drift apart.
///
/// `item_bg` is the fallback background used by neutral badges (e.g. "canceled"
/// or a NO toggle); `is_selected` swaps the active badge background for the
/// highlighted background. `formatted_text` is `None` when the caller should
/// fall back to its own default display.
pub(crate) fn badge_style_for(
    label: &str,
    val: &str,
    is_selected: bool,
    item_bg: Color,
    theme: &Theme,
    icons: &Icons,
) -> (Color, Option<Color>, bool, Option<String>) {
    let active_bg = |color: Color| -> Option<Color> {
        if is_selected {
            Some(theme.highlight_bg)
        } else {
            Some(color)
        }
    };
    match label {
        "State" => match val.to_lowercase().as_str() {
            "opened" | "open" | "active" => (
                theme.green,
                active_bg(theme.green_bg),
                true,
                Some(format!(" {} OPEN ", icons.state_open)),
            ),
            "closed" | "close" => (
                theme.red,
                active_bg(theme.red_bg),
                true,
                Some(format!(" {} CLOSED ", icons.state_closed)),
            ),
            "merged" => (
                theme.purple,
                active_bg(theme.purple_bg),
                true,
                Some(format!(" {} MERGED ", icons.state_merged)),
            ),
            _ => (theme.text_normal, None, false, None),
        },
        "Status" | "Deploy Status" => match val.to_lowercase().as_str() {
            "success" | "online" | "ready" => (
                theme.green,
                active_bg(theme.green_bg),
                true,
                Some(format!(" {} SUCCESS ", icons.status_success)),
            ),
            "failed" | "offline" => (
                theme.red,
                active_bg(theme.red_bg),
                true,
                Some(format!(" {} FAILED ", icons.status_failed)),
            ),
            "running" => (
                theme.blue,
                active_bg(theme.blue_bg),
                true,
                Some(format!(" {} RUNNING ", icons.status_running)),
            ),
            "pending" | "waiting" | "draft" => (
                theme.yellow,
                active_bg(theme.yellow_bg),
                true,
                Some(format!(" {} PENDING ", icons.status_pending)),
            ),
            "canceled" | "cancelled" => (
                theme.text_muted,
                Some(item_bg),
                false,
                Some(format!(" {} CANCELED ", icons.status_canceled)),
            ),
            "paused" => (
                theme.yellow,
                active_bg(theme.yellow_bg),
                true,
                Some(format!(" {} PAUSED ", icons.runner_paused)),
            ),
            _ => (theme.text_normal, None, false, None),
        },
        "Approval" => match val.to_uppercase().as_str() {
            "APPROVED" => (
                theme.green,
                active_bg(theme.green_bg),
                true,
                Some(format!(" {} APPROVED ", icons.approval_approved)),
            ),
            "CHANGES" => (
                theme.red,
                active_bg(theme.red_bg),
                true,
                Some(format!(" {} CHANGES ", icons.approval_changes)),
            ),
            "YOURS" => (
                theme.blue,
                active_bg(theme.blue_bg),
                true,
                Some(format!(" \u{f007} YOURS ")),
            ),
            "AWAITING" => (
                theme.yellow,
                active_bg(theme.yellow_bg),
                true,
                Some(format!(" {} AWAITING ", icons.approval_pending)),
            ),
            _ => (theme.text_normal, None, false, None),
        },
        "Mergeable" => match val.to_uppercase().as_str() {
            "CLEAN" => (
                theme.green,
                active_bg(theme.green_bg),
                true,
                Some(format!(" {} CLEAN ", icons.merge_clean)),
            ),
            "CONFLICT" | "BLOCKED" => (
                theme.red,
                active_bg(theme.red_bg),
                true,
                Some(format!(" {} CONFLICT ", icons.merge_conflict)),
            ),
            "REBASE" | "BEHIND" => (
                theme.yellow,
                active_bg(theme.yellow_bg),
                true,
                Some(format!(" {} REBASE ", icons.merge_rebase)),
            ),
            _ => (theme.text_normal, None, false, None),
        },
        "Workflow" => match val.to_uppercase().as_str() {
            "APPROVED" => (
                theme.green,
                active_bg(theme.green_bg),
                true,
                Some(format!(" {} APPROVED ", icons.approval_approved)),
            ),
            "REVIEW" => (
                theme.blue,
                active_bg(theme.blue_bg),
                true,
                Some(format!(" {} REVIEW ", icons.workflow_review)),
            ),
            "CHANGES" => (
                theme.red,
                active_bg(theme.red_bg),
                true,
                Some(format!(" {} CHANGES ", icons.approval_changes)),
            ),
            "DRAFT" => (
                theme.yellow,
                active_bg(theme.yellow_bg),
                true,
                Some(format!(" {} DRAFT ", icons.status_draft)),
            ),
            _ => (theme.text_normal, None, false, None),
        },
        "Default" | "Protected" | "Can Push" | "Active" | "Confidential" => {
            if val == "YES" || val == "Yes" || val == "true" {
                (
                    theme.green,
                    active_bg(theme.green_bg),
                    true,
                    Some(format!(" {} YES ", icons.check_on)),
                )
            } else {
                (
                    theme.text_muted,
                    Some(item_bg),
                    false,
                    Some(format!(" {} NO ", icons.check_off)),
                )
            }
        }
        "Milestone" | "Branch" | "Ref" | "Deploy Ref" | "Stage" => {
            (theme.purple, None, false, None)
        }
        "Author" | "Assignees" | "Reviewers" | "Deployer" | "Target" | "Project" => {
            (theme.blue, None, false, None)
        }
        "Updated" | "Created" | "Duration" | "Released" | "Deployed" | "Date" | "Due Date"
        | "Start Date" | "Avg Wait" => (theme.yellow, None, false, None),
        "ID" | "SHA" | "Commit" | "Runner" | "Tag" | "Deploy SHA" | "Deploy ID" => {
            (theme.blue, None, false, None)
        }
        _ => (theme.text_normal, None, false, None),
    }
}

/// Return a styled yellow span for a relative timestamp.
pub(crate) fn time_ago_span(date: &str) -> Span<'static> {
    Span::styled(
        crate::utils::format::time_ago(date),
        Style::default().fg(THEME.read().unwrap().yellow),
    )
}

/// Split comma-separated values and render each with the given color.
pub(crate) fn comma_spans(text: &str, color: Color) -> Vec<Span<'static>> {
    let parts: Vec<String> = text.split(',').map(|s| s.trim().to_string()).collect();
    let mut spans = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(", "));
        }
        spans.push(Span::styled(
            part.clone(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    spans
}

/// Split comma-separated labels and render each with its hash-based color.
pub(crate) fn label_spans(text: &str) -> Vec<Span<'static>> {
    let parts: Vec<String> = text.split(',').map(|s| s.trim().to_string()).collect();
    let mut spans = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(", "));
        }
        spans.push(Span::styled(
            part.clone(),
            Style::default()
                .fg(get_label_color(part, &HashMap::new()))
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pipelines::Job;
    use crate::ui::diff::count_wrapped_lines;
    use crate::ui::diff::format_comment_with_suggestions;

    fn make_job(id: u64, stage: &str, name: &str, status: &str) -> Job {
        Job {
            id,
            stage: stage.to_string(),
            name: name.to_string(),
            status: status.to_string(),
            matrix: None,
            duration_seconds: None,
            runner: None,
            needs: Vec::new(),
        }
    }

    #[test]
    fn test_get_stages_summary() {
        // Test case 1: Stage with mixed success and skipped jobs
        // This stage should be reported as "success" status, and 100% success rate.
        let jobs = vec![
            make_job(1, "build", "compile", "success"),
            make_job(2, "build", "cache", "skipped"),
            make_job(3, "test", "unit", "failed"),
            make_job(4, "test", "integration", "success"),
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
        assert_eq!(count_wrapped_lines("hello\nworld", 10), 2);
    }

    #[test]
    fn test_get_label_color() {
        let colors = HashMap::new();
        let color1 = get_label_color("bug", &colors);
        let _color2 = get_label_color("feature", &colors);
        let color3 = get_label_color("bug", &colors);

        assert_eq!(color1, color3);
        match color1 {
            Color::Rgb(_, _, _) => {}
            _ => panic!("Expected Rgb color"),
        }
    }

    #[test]
    fn test_get_label_color_uses_api_color() {
        let mut colors = HashMap::new();
        colors.insert("bug".to_string(), Color::Rgb(215, 58, 74));
        assert_eq!(get_label_color("bug", &colors), Color::Rgb(215, 58, 74));
    }

    #[test]
    fn test_get_label_color_ignores_light_api_color() {
        let mut colors = HashMap::new();
        // GitHub yellow (fbca04) is too light to read as foreground text.
        colors.insert("bug".to_string(), Color::Rgb(251, 202, 4));
        let fallback = get_label_color("bug", &HashMap::new());
        assert_eq!(get_label_color("bug", &colors), fallback);
    }

    #[test]
    fn test_render_labels_cell() {
        let labels = vec!["bug".to_string(), "backend".to_string()];
        let colors = HashMap::new();
        let cell_empty = render_labels_cell(&[], &colors, "", false, false, 24);
        let cell_str_empty = format!("{:?}", cell_empty);
        assert!(cell_str_empty.contains("—"));

        let cell_normal = render_labels_cell(&labels, &colors, "", false, false, 24);
        let cell_str = format!("{:?}", cell_normal);
        assert!(cell_str.contains("bug"));
        assert!(cell_str.contains("backend"));
    }

    #[test]
    fn test_format_comment_with_suggestions() {
        let body = "This is a comment\n```suggestion\nnew line content\n```\noutside suggestion";
        let file_path = "src/app.rs";
        let all_lines = vec![crate::app::DiffLine {
            content: "old line content".to_string(),
            line_type: crate::app::DiffLineType::Deletion,
            file_path: "src/app.rs".to_string(),
            old_line_num: Some(1),
            new_line_num: None,
            syntax_highlighted: None,
            fuzzy_indices: None,
        }];
        let prefix = "prefix";
        let prefix_style = Style::default();

        let formatted = format_comment_with_suggestions(
            body,
            file_path,
            None,
            None,
            Some(1),
            Some(1),
            &all_lines,
            prefix,
            prefix_style,
        );

        assert_eq!(formatted.len(), 6);
        assert_eq!(formatted[0].2[0].1, "This is a comment");
        assert_eq!(formatted[1].2[0].1, "┌─── Code Suggestion ───");
        assert_eq!(formatted[2].2[0].1, "│ - ");
        assert_eq!(formatted[2].2[1].1, "old line content");
        assert_eq!(formatted[3].2[0].1, "│ + ");
        assert_eq!(formatted[3].2[1].1, "new line content");
        assert_eq!(formatted[4].2[0].1, "└─── End of Suggestion ───");
        assert_eq!(formatted[5].2[0].1, "outside suggestion");
    }

    #[test]
    fn test_floor_char_boundary() {
        let s = "👕hello";
        assert_eq!(floor_char_boundary(s, 0), 0);
        assert_eq!(floor_char_boundary(s, 1), 0);
        assert_eq!(floor_char_boundary(s, 2), 0);
        assert_eq!(floor_char_boundary(s, 3), 0);
        assert_eq!(floor_char_boundary(s, 4), 4);
        assert_eq!(floor_char_boundary(s, 5), 5);
        assert_eq!(floor_char_boundary(s, 999), s.len());
    }
}
