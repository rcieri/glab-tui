#![no_main]

use std::sync::LazyLock;

use glab_tui_crate::config::Theme;
use glab_tui_crate::utils::markdown::render_markdown;
use libfuzzer_sys::fuzz_target;

static THEME: LazyLock<Theme> = LazyLock::new(Theme::default);

fuzz_target!(|input: (u16, &str)| {
    let (width, markdown) = input;
    render_markdown(markdown, &THEME, width);
});
