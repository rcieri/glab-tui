#![no_main]

use glab_tui_crate::app::DiffView;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let raw_diff = String::from_utf8_lossy(data).into_owned();
    DiffView::new(0, String::new(), raw_diff);
});
