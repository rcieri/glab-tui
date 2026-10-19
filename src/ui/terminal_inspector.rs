use super::helpers::{log_status_badge, rendered_line_count};
use super::modal::modal_area;
use crate::app::{App, OverlayKind, TerminalInspector};
use crate::command_log::TerminalCommand;
use crate::config::{ICONS, Icons, KeybindingTerminal, THEME, Theme};
use crate::utils::format::{expand_tabs, format_elapsed, sanitize_untrusted};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

const LABEL_WIDTH: usize = 11;
const TAB_WIDTH: usize = 4;

pub(crate) fn render_terminal_inspector(f: &mut Frame, app: &mut App, size: Rect) {
    let Some(inspector) = app.terminal_inspector else {
        return;
    };
    let Some(entry) = app.terminal_commands.get(inspector.row) else {
        app.terminal_inspector = None;
        return;
    };
    let theme = *THEME.read().unwrap();
    let icons = ICONS.read().unwrap();
    let lines = inspector_lines(entry, &app.config.keybindings.terminal, &theme, &icons);
    let hints = key_hints(&app.config.keybindings.terminal);

    let title = format!("{} Command Details", icons.tab_terminal);
    let (body, area) = modal_area(f, &title, 80, 80, 60, 14, size);
    app.overlay_stack
        .push((OverlayKind::TerminalInspector, area));
    let [content, hints_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(body);

    let page_height = usize::from(content.height);
    let total = rendered_line_count(&lines, usize::from(content.width), true);
    let scroll = inspector.scroll.min(total.saturating_sub(page_height));
    app.terminal_inspector = Some(TerminalInspector {
        scroll,
        page_height,
        ..inspector
    });

    f.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(theme.text_normal).bg(theme.bg))
            .wrap(Wrap { trim: false })
            .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
        content,
    );
    f.render_widget(
        Paragraph::new(hints).style(Style::default().fg(theme.text_muted).bg(theme.bg)),
        hints_area,
    );
}

fn key_hints(keys: &KeybindingTerminal) -> String {
    format!(
        " Esc close · j/k scroll · {} copy command · {} copy output · {} re-run",
        keys.copy_command, keys.copy_output, keys.rerun_command
    )
}

fn inspector_lines(
    entry: &TerminalCommand,
    keys: &KeybindingTerminal,
    theme: &Theme,
    icons: &Icons,
) -> Vec<Line<'static>> {
    let execution = &entry.execution;
    let (status_text, status_color) = log_status_badge(&entry.status, icons, theme);
    let mut lines = vec![field(
        "Status",
        Span::styled(
            status_text.trim_end().to_string(),
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
        theme,
    )];
    if let Some(code) = execution.exit_code {
        lines.push(text_field("Exit code", code.to_string(), theme));
    }

    match &execution.invocation {
        Some(invocation) => {
            let command = invocation.command_line();
            if let Some(action) = action_of(&entry.command, &command) {
                lines.push(text_field("Action", action, theme));
            }
            lines.push(text_field("Command", command, theme));
        }
        None => lines.push(text_field("Entry", entry.command.trim().to_string(), theme)),
    }

    let started = match execution.started_at {
        Some(started_at) => started_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        None if !entry.timestamp.is_empty() => entry.timestamp.clone(),
        None => "unknown".to_string(),
    };
    lines.push(text_field("Started", started, theme));
    let duration = match execution.duration {
        Some(duration) => format_elapsed(duration),
        None if entry.status == "Running" => "still running".to_string(),
        None => "not recorded".to_string(),
    };
    lines.push(text_field("Duration", duration, theme));
    let rerun = match &execution.invocation {
        Some(invocation) if invocation.stdin.is_some() => {
            format!(
                "press {} (replays the recorded request body)",
                keys.rerun_command
            )
        }
        Some(_) => format!("press {}", keys.rerun_command),
        None => "not available: no glab, gh or git call recorded".to_string(),
    };
    lines.push(text_field("Re-run", rerun, theme));

    if let Some(reason) = entry
        .failure_reason()
        .filter(|reason| reason.trim() != execution.stderr.trim())
    {
        push_section(&mut lines, "Error", reason, theme.red, theme);
    }
    if execution.invocation.is_some() {
        push_section(&mut lines, "stderr", &execution.stderr, theme.red, theme);
        push_section(&mut lines, "stdout", &execution.stdout, theme.blue, theme);
    } else {
        lines.push(Line::default());
        lines.push(text_field(
            "Output",
            "not captured for this entry".to_string(),
            theme,
        ));
    }
    lines
}

/// The description an entry carries in front of its command line, or the
/// whole entry text when it does not end with that command line (an action
/// logged by `CommandStarted` that a backend call settled).
fn action_of(entry_text: &str, command: &str) -> Option<String> {
    let entry_text = entry_text.trim();
    let action = match entry_text.strip_suffix(command) {
        Some(prefix) => prefix.trim_end().trim_end_matches(':').trim_end(),
        None => entry_text,
    };
    (!action.is_empty()).then(|| sanitize_untrusted(action).into_owned())
}

fn field(label: &str, value: Span<'static>, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label:<LABEL_WIDTH$}"),
            Style::default()
                .fg(theme.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        value,
    ])
}

fn text_field(label: &str, value: String, theme: &Theme) -> Line<'static> {
    let value = sanitize_untrusted(&value).into_owned();
    field(label, Span::raw(value), theme)
}

fn push_section(
    lines: &mut Vec<Line<'static>>,
    title: &str,
    text: &str,
    color: ratatui::style::Color,
    theme: &Theme,
) {
    lines.push(Line::default());
    lines.push(Line::styled(
        title.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ));
    if text.trim().is_empty() {
        lines.push(Line::styled(
            "(empty)",
            Style::default()
                .fg(theme.text_muted)
                .add_modifier(Modifier::ITALIC),
        ));
        return;
    }
    let text = expand_tabs(&sanitize_untrusted(text), TAB_WIDTH);
    lines.extend(text.lines().map(|line| Line::raw(line.to_string())));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_log::{Execution, Invocation, Program};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn screen_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn inspector_shows_the_full_command_its_timing_and_both_streams() {
        let mut app = App::default();
        let row = app.terminal_commands.push(TerminalCommand {
            timestamp: "10:00:01".to_string(),
            command: "FETCH ISSUES: glab issue list -R group/project".to_string(),
            status: "Failed: 404 Not Found".to_string(),
            execution: Execution {
                started_at: None,
                duration: Some(std::time::Duration::from_millis(2400)),
                exit_code: Some(1),
                stdout: "partial page".to_string(),
                stderr: "GET https://gitlab.com/api/v4: 404 Not Found".to_string(),
                invocation: Some(Invocation::new(
                    Program::Glab,
                    &["issue", "list", "-R", "group/project"],
                )),
            },
        });
        app.terminal_inspector = Some(TerminalInspector::new(row));
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

        terminal
            .draw(|f| render_terminal_inspector(f, &mut app, f.area()))
            .unwrap();

        let screen = screen_text(&terminal);
        for expected in [
            "Exit code  1",
            "Action     FETCH ISSUES",
            "Command    glab issue list -R group/project",
            "Started    10:00:01",
            "Duration   2.4s",
            "Re-run     press r",
            "404 Not Found",
            "GET https://gitlab.com/api/v4: 404 Not Found",
            "partial page",
        ] {
            assert!(
                screen.contains(expected),
                "missing {expected:?} in\n{screen}"
            );
        }
    }

    #[test]
    fn inspector_closes_once_its_entry_leaves_the_log() {
        let mut app = App::default();
        let row = app.terminal_commands.push(TerminalCommand {
            command: "glab mr list".to_string(),
            status: "Success".to_string(),
            ..Default::default()
        });
        app.terminal_inspector = Some(TerminalInspector::new(row));
        app.clear_terminal_log();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        terminal
            .draw(|f| render_terminal_inspector(f, &mut app, f.area()))
            .unwrap();

        assert_eq!(app.terminal_inspector, None);
    }

    #[test]
    fn inspector_scroll_is_clamped_to_the_last_page() {
        let mut app = App::default();
        let row = app.terminal_commands.push(TerminalCommand {
            command: "glab ci trace".to_string(),
            status: "Success".to_string(),
            execution: Execution {
                stdout: (0..200).map(|n| format!("line {n}\n")).collect(),
                invocation: Some(Invocation::new(Program::Glab, &["ci", "trace"])),
                ..Default::default()
            },
            ..Default::default()
        });
        app.terminal_inspector = Some(TerminalInspector {
            scroll: usize::MAX,
            ..TerminalInspector::new(row)
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();

        terminal
            .draw(|f| render_terminal_inspector(f, &mut app, f.area()))
            .unwrap();

        let screen = screen_text(&terminal);
        assert!(
            screen.contains("line 199"),
            "last line not shown in\n{screen}"
        );
        let inspector = app.terminal_inspector.expect("still open");
        assert!(inspector.scroll < usize::MAX);
        assert!(inspector.page_height > 0);
    }
}
