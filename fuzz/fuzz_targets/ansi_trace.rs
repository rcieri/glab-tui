#![no_main]

use std::sync::LazyLock;

use glab_tui_crate::config::Theme;
use glab_tui_crate::utils::format::parse_ansi_trace;
use libfuzzer_sys::fuzz_target;

static THEME: LazyLock<Theme> = LazyLock::new(Theme::default);

fuzz_target!(|data: &[u8]| {
    let trace = String::from_utf8_lossy(data);
    parse_ansi_trace(&trace, &THEME);
});
