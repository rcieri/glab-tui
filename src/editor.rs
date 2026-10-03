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
    let mut tmp = tempfile::Builder::new().suffix(suffix).tempfile().ok()?;
    std::io::Write::write_all(&mut tmp, current_val.as_bytes()).ok()?;
    let file_path = tmp.into_temp_path();

    let mut editor = std::process::Command::new(editor_name());
    editor.arg(&file_path);
    let status = suspend_and_run(&mut editor, terminal).ok()?;
    if !status.success() {
        return None;
    }

    let content = std::fs::read_to_string(&file_path).ok()?;
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Hands the terminal to `command` until it exits, with inherited stdio.
pub fn suspend_and_run(
    command: &mut std::process::Command,
    terminal: &mut AppTerminal,
) -> std::io::Result<std::process::ExitStatus> {
    suspend_while(terminal, || {
        command
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
    })?
}

/// Runs `foreground` with the terminal handed over: pauses the event reader,
/// leaves the alternate screen and raw mode, then restores the TUI and drops
/// keypresses typed meanwhile. One handoff can cover several processes.
pub fn suspend_while<T>(
    terminal: &mut AppTerminal,
    foreground: impl FnOnce() -> T,
) -> std::io::Result<T> {
    crate::event::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
    std::thread::sleep(std::time::Duration::from_millis(50));

    let result = leave_tui().map(|()| {
        let _interrupts = InterruptGuard::install();
        foreground()
    });

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

/// Keeps Ctrl+C and Ctrl+\ from killing the TUI while a foreground child
/// has the terminal: leaving raw mode re-enables ISIG, and the TUI shares the
/// child's process group. A no-op handler rather than SIG_IGN, because exec
/// resets handlers to the default, so the child still gets the signal.
#[cfg(unix)]
struct InterruptGuard {
    previous: Vec<(libc::c_int, libc::sigaction)>,
}

#[cfg(unix)]
impl InterruptGuard {
    fn install() -> Self {
        extern "C" fn ignore(_: libc::c_int) {}
        let mut previous = Vec::new();
        for signal in [libc::SIGINT, libc::SIGQUIT] {
            // SAFETY: both structs are fully initialised before use, and the
            // handler does nothing, so it is async-signal-safe.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = ignore as extern "C" fn(libc::c_int) as usize;
                libc::sigemptyset(&mut action.sa_mask);
                let mut old: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(signal, &action, &mut old) == 0 {
                    previous.push((signal, old));
                }
            }
        }
        Self { previous }
    }
}

#[cfg(unix)]
impl Drop for InterruptGuard {
    fn drop(&mut self) {
        for (signal, old) in &self.previous {
            // SAFETY: restores a disposition previously returned by sigaction.
            unsafe {
                libc::sigaction(*signal, old, std::ptr::null_mut());
            }
        }
    }
}

#[cfg(not(unix))]
struct InterruptGuard;

#[cfg(not(unix))]
impl InterruptGuard {
    fn install() -> Self {
        Self
    }
}

fn leave_tui() -> std::io::Result<()> {
    crossterm::terminal::disable_raw_mode()?;
    let mut stdout = std::io::stdout();
    try_pop_keyboard_enhancement_flags(&mut stdout);
    crossterm::execute!(
        stdout,
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
        crossterm::cursor::Show,
    )
}
