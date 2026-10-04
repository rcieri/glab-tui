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

pub fn editor_command(file_path: &std::path::Path) -> std::process::Command {
    let raw = editor_command_string();
    editor_process(&raw, file_path)
}

pub fn editor_command_string() -> String {
    std::env::var("EDITOR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("VISUAL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "helix".to_string())
}

/// Backwards compatibility helper for callers expecting the raw editor command string.
///
/// Deprecated: prefer [`editor_command_string`] (returns a full shell command, not
/// just a binary name). This shim exists to avoid breaking any downstream callers
/// and will be removed in a future cleanup.
#[deprecated(since = "0.9.3", note = "use `editor_command_string()` instead")]
pub fn editor_name() -> String {
    editor_command_string()
}

/// Creates a shell command that executes the editor string, passing the file path
/// as a positional argument (`"$@"` on Unix, embedded quoted path on Windows) so
/// arguments in `$EDITOR` (e.g. `code --wait`, `nvim -u minimal.lua`,
/// `omarchy-launch-editor --inline`) are honored without breaking path escaping.
///
/// Prefer [`editor_command`] for normal use; this is exposed as `pub(crate)` for
/// unit testing of the command-building logic.
pub(crate) fn editor_process(editor: &str, file_path: &std::path::Path) -> std::process::Command {
    if cfg!(windows) {
        // On Windows, `cmd /C "string"` does not expand %1 from subsequent
        // argument slots — the file path must be embedded directly in the
        // command string. We quote it with double-quotes; paths containing
        // double-quotes are vanishingly rare on Windows.
        let path_str = file_path.to_string_lossy();
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").arg(format!("{editor} \"{path_str}\""));
        cmd
    } else {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c")
            .arg(format!("{editor} \"$@\""))
            .arg("sh")
            .arg(file_path);
        cmd
    }
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

    let mut editor = editor_command(&file_path);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn editor_process_preserves_arguments_and_passes_file_as_positional_arg() {
        let file_path = Path::new("/tmp/test note.md");
        let cmd = editor_process("nvim -u minimal.lua --clean", file_path);
        if cfg!(windows) {
            assert_eq!(cmd.get_program(), "cmd");
            let args: Vec<&std::ffi::OsStr> = cmd.get_args().collect();
            // Windows: path is embedded directly in the /C string (no %1 expansion)
            assert_eq!(args[0], "/C");
            assert_eq!(args[1], "nvim -u minimal.lua --clean \"/tmp/test note.md\"");
            assert_eq!(args.len(), 2, "no trailing file arg on Windows");
        } else {
            assert_eq!(cmd.get_program(), "sh");
            let args: Vec<&std::ffi::OsStr> = cmd.get_args().collect();
            assert_eq!(args[0], "-c");
            assert_eq!(args[1], "nvim -u minimal.lua --clean \"$@\"");
            assert_eq!(args[2], "sh");
            assert_eq!(args[3], file_path.as_os_str());
        }
    }

    #[test]
    fn editor_command_string_falls_back_when_editor_is_empty() {
        let _guard = crate::config::TEST_ENV_MUTEX.lock().unwrap();
        let prev_editor = std::env::var("EDITOR").ok();
        let prev_visual = std::env::var("VISUAL").ok();

        // 1. When EDITOR is whitespace, fall back to VISUAL
        unsafe {
            std::env::set_var("EDITOR", "   ");
            std::env::set_var("VISUAL", "code --wait");
        }
        assert_eq!(editor_command_string(), "code --wait");

        // 2. When both are empty/whitespace, fall back to default "helix"
        unsafe {
            std::env::set_var("EDITOR", "");
            std::env::set_var("VISUAL", " ");
        }
        assert_eq!(editor_command_string(), "helix");

        // 3. When EDITOR has a command with flags, it is returned
        unsafe {
            std::env::set_var("EDITOR", "omarchy-launch-editor --inline");
        }
        assert_eq!(editor_command_string(), "omarchy-launch-editor --inline");

        // Restore original env
        unsafe {
            match prev_editor {
                Some(v) => std::env::set_var("EDITOR", v),
                None => std::env::remove_var("EDITOR"),
            }
            match prev_visual {
                Some(v) => std::env::set_var("VISUAL", v),
                None => std::env::remove_var("VISUAL"),
            }
        }
    }
}
