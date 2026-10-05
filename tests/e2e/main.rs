#![cfg(unix)]
use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod combinations;
mod config;
mod custom_keybindings;
mod keybindings;
mod layout;
mod pagination;
mod pr_diff_fallback;
mod review_threads;
mod scenarios;
mod tabs;
mod workspace;

// Declarations of libc functions
unsafe extern "C" {
    fn forkpty(
        amaster: *mut std::os::raw::c_int,
        name: *mut std::os::raw::c_char,
        termp: *const libc::termios,
        winp: *const libc::winsize,
    ) -> libc::pid_t;
}

pub struct Pty {
    pub master: RawFd,
    pub child_pid: libc::pid_t,
}

impl Pty {
    pub fn spawn(
        cmd: &str,
        args: &[&str],
        envs: &[(&str, &str)],
        rows: u16,
        cols: u16,
        cwd: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        let mut master: std::os::raw::c_int = 0;
        let win = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        // Prepare child args and strings in the parent process (before fork)
        let c_cmd = std::ffi::CString::new(cmd).unwrap();
        let mut c_args = vec![c_cmd.clone()];
        for arg in args {
            c_args.push(std::ffi::CString::new(*arg).unwrap());
        }

        let arg_ptrs: Vec<*const std::os::raw::c_char> = c_args
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();

        // Convert environment variables to CStrings before fork
        // We also want to inherit current environment variables but override/add the specified ones.
        let mut c_envs = Vec::new();
        // Override list keys
        let override_keys: std::collections::HashSet<&str> = envs.iter().map(|(k, _)| *k).collect();
        for (k, v) in std::env::vars() {
            if !override_keys.contains(k.as_str()) && k != "GLAB_TUI_CONFIG" {
                let env_str = format!("{k}={v}");
                if let Ok(c_env) = std::ffi::CString::new(env_str) {
                    c_envs.push(c_env);
                }
            }
        }
        for &(k, v) in envs {
            if let Ok(c_env) = std::ffi::CString::new(format!("{k}={v}")) {
                c_envs.push(c_env);
            }
        }

        let env_ptrs: Vec<*const std::os::raw::c_char> = c_envs
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();

        // Convert path to CString
        let c_cwd = cwd.and_then(|path| path.to_str().and_then(|s| std::ffi::CString::new(s).ok()));

        // Block all signals before fork to prevent signal handler interference
        // in the child process. This is critical on macOS where fork() in
        // multi-threaded programs can deadlock if signal handlers are active.
        let mut old_mask: libc::sigset_t = unsafe { std::mem::zeroed() };
        unsafe {
            let mut mask: libc::sigset_t = std::mem::zeroed();
            libc::sigfillset(&mut mask);
            libc::pthread_sigmask(libc::SIG_SETMASK, &mask, &mut old_mask);
        }

        let pid = unsafe { forkpty(&mut master, std::ptr::null_mut(), std::ptr::null(), &win) };

        // Restore the original signal mask in both parent and child.
        // The child needs a clean signal state before execve(); the mask
        // is preserved across exec so we must unblock signals beforehand.
        unsafe {
            libc::pthread_sigmask(libc::SIG_SETMASK, &old_mask, std::ptr::null_mut());
        }

        if pid < 0 {
            return Err("forkpty failed".to_string());
        }

        if pid == 0 {
            // Child process
            // Do NOT allocate any memory or use complex library calls!
            // Only async-signal-safe system calls should be used here.
            if let Some(ref dir) = c_cwd {
                unsafe {
                    libc::chdir(dir.as_ptr());
                }
            }

            unsafe {
                libc::execve(c_cmd.as_ptr(), arg_ptrs.as_ptr(), env_ptrs.as_ptr());
                libc::_exit(127);
            }
        }

        // Parent process
        // Set master FD to non-blocking
        unsafe {
            let flags = libc::fcntl(master, libc::F_GETFL, 0);
            libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        Ok(Self {
            master,
            child_pid: pid,
        })
    }

    pub fn read_output(&self) -> Vec<u8> {
        let mut buf = [0u8; 4096];
        let mut output = Vec::new();
        loop {
            let n = unsafe {
                libc::read(
                    self.master,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            };
            if n > 0 {
                output.extend_from_slice(&buf[..n as usize]);
            } else {
                break;
            }
        }
        output
    }

    pub fn write_input(&self, data: &[u8]) {
        let mut written = 0;
        while written < data.len() {
            let n = unsafe {
                libc::write(
                    self.master,
                    data[written..].as_ptr() as *const libc::c_void,
                    data.len() - written,
                )
            };
            if n > 0 {
                written += n as usize;
            } else {
                break;
            }
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.child_pid, libc::SIGKILL);

            // Wait for child to exit with a timeout to prevent hangs.
            // On macOS, fork() in multi-threaded programs can cause the child
            // to enter an uninterruptible state, so a blocking waitpid() may
            // never return.
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let mut status = 0;
                let res = libc::waitpid(self.child_pid, &mut status, libc::WNOHANG);
                if res != 0 || Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }

            libc::close(self.master);
        }
    }
}

/// First character of `bytes` and its encoded length. Bytes that do not
/// start a complete UTF-8 sequence (e.g. one split across two reads) are taken
/// one at a time.
fn decode_utf8_char(bytes: &[u8]) -> (char, usize) {
    let len = match bytes[0] {
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    };
    bytes
        .get(..len)
        .and_then(|seq| std::str::from_utf8(seq).ok())
        .and_then(|s| s.chars().next())
        .map_or((bytes[0] as char, 1), |ch| (ch, len))
}

pub struct TerminalEmulator {
    pub rows: usize,
    pub cols: usize,
    pub grid: Vec<Vec<char>>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    /// Tail of the previous read that ended mid escape sequence or mid
    /// UTF-8 character; PTY reads split output at arbitrary byte offsets.
    pending: Vec<u8>,
}

impl TerminalEmulator {
    pub fn new(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            grid: vec![vec![' '; cols]; rows],
            cursor_row: 0,
            cursor_col: 0,
            pending: Vec::new(),
        }
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        let mut buffered = std::mem::take(&mut self.pending);
        buffered.extend_from_slice(bytes);
        let bytes = &buffered[..];
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if b == 0x1b {
                if i + 1 == bytes.len() {
                    self.pending = bytes[i..].to_vec();
                    return;
                }
                if bytes[i + 1] == b'[' {
                    let start = i;
                    i += 2;
                    let mut seq_chars = Vec::new();
                    while i < bytes.len() {
                        let c = bytes[i];
                        seq_chars.push(c);
                        if c.is_ascii_lowercase() || c.is_ascii_uppercase() {
                            break;
                        }
                        i += 1;
                    }
                    if i == bytes.len() {
                        self.pending = bytes[start..].to_vec();
                        return;
                    }
                    let last_char = seq_chars.last().cloned().unwrap_or(b' ');
                    let mut parts = Vec::new();
                    let mut current = 0;
                    let mut has_digit = false;
                    for &c in &seq_chars[..seq_chars.len().saturating_sub(1)] {
                        if c.is_ascii_digit() {
                            current = current * 10 + (c - b'0') as usize;
                            has_digit = true;
                        } else if c == b';' {
                            parts.push(current);
                            current = 0;
                            has_digit = false;
                        }
                    }
                    if has_digit {
                        parts.push(current);
                    }
                    match last_char {
                        b'H' | b'f' => {
                            let r = parts.first().copied().unwrap_or(1);
                            let c = parts.get(1).copied().unwrap_or(1);
                            self.cursor_row = r.saturating_sub(1).min(self.rows - 1);
                            self.cursor_col = c.saturating_sub(1).min(self.cols - 1);
                        }
                        b'J' => {
                            let mode = parts.first().copied().unwrap_or(0);
                            if mode == 2 {
                                for r in 0..self.rows {
                                    for c in 0..self.cols {
                                        self.grid[r][c] = ' ';
                                    }
                                }
                            }
                        }
                        b'K' => {
                            let mode = parts.first().copied().unwrap_or(0);
                            if mode == 0 {
                                for c in self.cursor_col..self.cols {
                                    self.grid[self.cursor_row][c] = ' ';
                                }
                            }
                        }
                        b'A' => {
                            let val = parts.first().copied().unwrap_or(1);
                            self.cursor_row = self.cursor_row.saturating_sub(val);
                        }
                        b'B' => {
                            let val = parts.first().copied().unwrap_or(1);
                            self.cursor_row = (self.cursor_row + val).min(self.rows - 1);
                        }
                        b'C' => {
                            let val = parts.first().copied().unwrap_or(1);
                            self.cursor_col = (self.cursor_col + val).min(self.cols - 1);
                        }
                        b'D' => {
                            let val = parts.first().copied().unwrap_or(1);
                            self.cursor_col = self.cursor_col.saturating_sub(val);
                        }
                        _ => {}
                    }
                    // Step past the final byte so it is not drawn as text.
                    i += 1;
                } else {
                    i += 1;
                }
            } else if b == b'\r' {
                self.cursor_col = 0;
                i += 1;
            } else if b == b'\n' {
                self.cursor_row = (self.cursor_row + 1).min(self.rows - 1);
                i += 1;
            } else if b == b'\t' {
                self.cursor_col = ((self.cursor_col / 8) + 1) * 8;
                if self.cursor_col >= self.cols {
                    self.cursor_col = self.cols - 1;
                }
                i += 1;
            } else {
                // One cell per character, not per byte: box drawing, icons
                // and `❯` are multi-byte UTF-8.
                let expected_len = match b {
                    0xC0..=0xDF => 2,
                    0xE0..=0xEF => 3,
                    0xF0..=0xF7 => 4,
                    _ => 1,
                };
                if i + expected_len > bytes.len() {
                    self.pending = bytes[i..].to_vec();
                    return;
                }
                let (ch, len) = decode_utf8_char(&bytes[i..]);
                if self.cursor_row < self.rows && self.cursor_col < self.cols {
                    self.grid[self.cursor_row][self.cursor_col] = ch;
                }
                self.cursor_col += 1;
                if self.cursor_col >= self.cols {
                    self.cursor_col = self.cols - 1;
                }
                i += len;
            }
        }
    }

    pub fn get_text(&self) -> String {
        let mut res = String::new();
        for r in 0..self.rows {
            let row_str: String = self.grid[r].iter().collect();
            res.push_str(&row_str);
            res.push('\n');
        }
        res
    }
}

pub struct Sandbox {
    pub temp_dir: tempfile::TempDir,
    pub home_dir: PathBuf,
    pub config_dir: PathBuf,
    pub bin_dir: PathBuf,
    pub repo_dir: PathBuf,
    pub log_path: PathBuf,
}

impl Sandbox {
    pub fn new(is_github: bool) -> Result<Self, std::io::Error> {
        let temp_dir = tempfile::tempdir()?;
        let home_dir = temp_dir.path().join("home");
        let config_dir = temp_dir.path().join("config");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");

        std::fs::create_dir_all(&home_dir)?;
        std::fs::create_dir_all(&config_dir)?;
        std::fs::create_dir_all(&bin_dir)?;
        std::fs::create_dir_all(&repo_dir)?;

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mock_gh = manifest_dir.join("tests").join("mocks").join("gh");
        let mock_glab = manifest_dir.join("tests").join("mocks").join("glab");
        std::fs::copy(&mock_gh, bin_dir.join("gh"))?;
        std::fs::copy(&mock_glab, bin_dir.join("glab"))?;

        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(bin_dir.join("gh"), std::fs::Permissions::from_mode(0o755))?;
        std::fs::set_permissions(bin_dir.join("glab"), std::fs::Permissions::from_mode(0o755))?;

        let log_path = temp_dir.path().join("cli_calls.log");

        let remote_url = if is_github {
            "git@github.com:test-owner/test-repo.git"
        } else {
            "git@gitlab.com:test-owner/test-repo.git"
        };

        let _ = std::process::Command::new("git")
            .arg("init")
            .current_dir(&repo_dir)
            .output();
        let _ = std::process::Command::new("git")
            .args(["remote", "add", "origin", remote_url])
            .current_dir(&repo_dir)
            .output();

        Ok(Self {
            temp_dir,
            home_dir,
            config_dir,
            bin_dir,
            repo_dir,
            log_path,
        })
    }

    pub fn envs(&self) -> Vec<(String, String)> {
        let path_env = format!(
            "{}:{}",
            self.bin_dir.to_str().unwrap(),
            std::env::var("PATH").unwrap_or_default()
        );
        vec![
            (
                "HOME".to_string(),
                self.home_dir.to_str().unwrap().to_string(),
            ),
            (
                "XDG_CONFIG_HOME".to_string(),
                self.config_dir.to_str().unwrap().to_string(),
            ),
            ("PATH".to_string(), path_env),
            (
                "TEST_LOG_PATH".to_string(),
                self.log_path.to_str().unwrap().to_string(),
            ),
            (
                "TEST_FIXTURES_DIR".to_string(),
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests")
                    .join("fixtures")
                    .to_str()
                    .unwrap()
                    .to_string(),
            ),
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("SHELL".to_string(), "/bin/sh".to_string()),
        ]
    }
}

pub struct TestSession {
    pub sandbox: Sandbox,
    pub pty: Pty,
    pub emulator: TerminalEmulator,
}

impl TestSession {
    pub fn new(is_github: bool, rows: u16, cols: u16) -> Self {
        Self::with_config(is_github, rows, cols, None)
    }

    /// Launch a session with extra environment variables, used to steer the
    /// CLI mocks into failure modes the tests need to assert on.
    pub fn with_envs(is_github: bool, rows: u16, cols: u16, extra: &[(&str, &str)]) -> Self {
        Self::launch(Sandbox::new(is_github).unwrap(), rows, cols, extra)
    }

    /// Launch in a sandbox the test has already prepared (e.g. committed to
    /// its repository), with extra environment variables for the CLI mocks.
    pub fn launch(sandbox: Sandbox, rows: u16, cols: u16, extra: &[(&str, &str)]) -> Self {
        let bin_path = find_glab_tui_binary();
        let mut envs_vec = sandbox.envs();
        for (k, v) in extra {
            envs_vec.push((k.to_string(), v.to_string()));
        }
        let envs_ref: Vec<(&str, &str)> = envs_vec
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let pty = Pty::spawn(
            bin_path.to_str().unwrap(),
            &[],
            &envs_ref,
            rows,
            cols,
            Some(&sandbox.repo_dir),
        )
        .unwrap();

        Self {
            sandbox,
            pty,
            emulator: TerminalEmulator::new(rows as usize, cols as usize),
        }
    }

    /// Launch a session with `config_toml` written to the sandbox's global
    /// config before the app starts, so configuration-dependent behaviour is
    /// in effect on the very first fetch.
    pub fn with_config(is_github: bool, rows: u16, cols: u16, config_toml: Option<&str>) -> Self {
        let sandbox = Sandbox::new(is_github).unwrap();
        if let Some(toml) = config_toml {
            let conf_dir = sandbox.config_dir.join("glab-tui");
            std::fs::create_dir_all(&conf_dir).unwrap();
            std::fs::write(conf_dir.join("config.toml"), toml).unwrap();
        }
        let bin_path = find_glab_tui_binary();
        let envs_vec = sandbox.envs();
        let envs_ref: Vec<(&str, &str)> = envs_vec
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let pty = Pty::spawn(
            bin_path.to_str().unwrap(),
            &[],
            &envs_ref,
            rows,
            cols,
            Some(&sandbox.repo_dir),
        )
        .unwrap();

        Self {
            sandbox,
            pty,
            emulator: TerminalEmulator::new(rows as usize, cols as usize),
        }
    }

    pub fn wait_for_screen_contains(
        &mut self,
        expected: &str,
        timeout_ms: u64,
    ) -> Result<(), String> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(timeout_ms) {
            // Check if child process has exited
            let mut status = 0;
            let res = unsafe { libc::waitpid(self.pty.child_pid, &mut status, libc::WNOHANG) };
            if res > 0 {
                let exit_reason = if libc::WIFEXITED(status) {
                    format!("exit code {}", libc::WEXITSTATUS(status))
                } else if libc::WIFSIGNALED(status) {
                    format!("signal {}", libc::WTERMSIG(status))
                } else {
                    "unknown".to_string()
                };
                let bytes = self.pty.read_output();
                if !bytes.is_empty() {
                    self.emulator.write_bytes(&bytes);
                }
                return Err(format!(
                    "Child process exited prematurely with {}. Screen content:\n{}",
                    exit_reason,
                    self.emulator.get_text()
                ));
            }

            let bytes = self.pty.read_output();
            if !bytes.is_empty() {
                self.emulator.write_bytes(&bytes);
                let text = self.emulator.get_text();
                if text.contains(expected) {
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let text = self.emulator.get_text();
        Err(format!(
            "Timeout waiting for screen to contain '{expected}'. Current screen:
{text}"
        ))
    }

    pub fn send_input(&self, data: &[u8]) {
        self.pty.write_input(data);
    }

    /// Pump output into the emulator for a fixed time. Needed for assertions
    /// about what is *not* on screen, where there is nothing to wait for.
    pub fn settle(&mut self, ms: u64) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(ms) {
            let bytes = self.pty.read_output();
            if !bytes.is_empty() {
                self.emulator.write_bytes(&bytes);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn get_cli_calls(&self) -> String {
        std::fs::read_to_string(&self.sandbox.log_path).unwrap_or_default()
    }
}

fn find_glab_tui_binary() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let mut path = exe.clone();
        path.pop(); // to deps/
        if path.file_name().and_then(|s| s.to_str()) == Some("deps") {
            path.pop(); // to debug/ or release/
        }
        let bin = path.join("glab-tui");
        if bin.exists() {
            return bin;
        }
    }
    PathBuf::from("target/debug/glab-tui")
}
