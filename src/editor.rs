use crate::AppTerminal;
use std::io::Write;

pub(crate) fn try_push_keyboard_enhancement_flags<W: Write>(w: &mut W) {
    if crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) {
        let _ = crossterm::execute!(
            w,
            crossterm::event::PushKeyboardEnhancementFlags(
                crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            ),
        );
    }
}

pub(crate) fn try_pop_keyboard_enhancement_flags<W: Write>(w: &mut W) {
    if crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) {
        let _ = crossterm::execute!(w, crossterm::event::PopKeyboardEnhancementFlags);
    }
}

pub fn editor_name() -> String {
    std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "helix".to_string())
}

pub fn edit_in_editor(current_val: &str, terminal: &mut AppTerminal) -> Option<String> {
    edit_in_editor_with_suffix(current_val, ".md", terminal)
}

pub fn edit_in_editor_with_suffix(
    current_val: &str,
    suffix: &str,
    terminal: &mut AppTerminal,
) -> Option<String> {
    let editor = editor_name();

    let mut tmp = tempfile::Builder::new().suffix(suffix).tempfile().ok()?;
    std::io::Write::write_all(&mut tmp, current_val.as_bytes()).ok()?;
    let file_path = tmp.into_temp_path();
    let path_buf = file_path.to_path_buf();

    crate::event::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
    std::thread::sleep(std::time::Duration::from_millis(50));

    let result = (|| {
        crossterm::terminal::disable_raw_mode().ok()?;
        let mut stdout = std::io::stdout();
        try_pop_keyboard_enhancement_flags(&mut stdout);
        crossterm::execute!(
            stdout,
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
        )
        .ok()?;

        let mut cmd = std::process::Command::new(&editor);
        cmd.arg(&path_buf);
        cmd.stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let status = cmd.spawn().ok()?.wait().ok()?;
        if !status.success() {
            return None;
        }

        let content = std::fs::read_to_string(&path_buf).ok()?;
        let trimmed = content.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })();

    // Restore terminal for the TUI. Each operation is best-effort: even if one
    // fails we attempt the next — all three are independent raw-mode gates.
    let mut stdout = std::io::stdout();
    let _ = crossterm::terminal::enable_raw_mode();
    let _ = crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
    );
    try_push_keyboard_enhancement_flags(&mut stdout);
    while crossterm::event::poll(std::time::Duration::from_secs(0)).unwrap_or(false) {
        let _ = crossterm::event::read();
    }
    let _ = terminal.clear();
    crate::event::PAUSED.store(false, std::sync::atomic::Ordering::Relaxed);

    result
}
