use similar::algorithms::{Capture, myers};
use similar::{DiffOp, diff_ratio};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Share of unchanged words below which a changed line counts as rewritten
/// rather than edited. Marking most of a rewritten line shows nothing the
/// row colour does not already say, so such pairs get no word marks.
const MIN_UNCHANGED_RATIO: f32 = 0.5;

/// Longest line diffed word by word. The word diff is quadratic in the worst
/// case, and a minified bundle or generated file can put megabytes on one
/// line; whole-line colouring is all such a line gets.
const MAX_LINE_BYTES: usize = 4096;

/// The byte ranges of a removed line and its added replacement that differ.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ChangedRanges {
    pub old: Vec<Range<usize>>,
    pub new: Vec<Range<usize>>,
}

/// Diffs `old` against `new` by Unicode words (UAX #29 word boundaries) and
/// returns the ranges of each that changed. `None` when the lines share too
/// little to pair (see [`MIN_UNCHANGED_RATIO`]) or either is longer than
/// [`MAX_LINE_BYTES`].
pub fn changed_ranges(old: &str, new: &str) -> Option<ChangedRanges> {
    if old.len() > MAX_LINE_BYTES || new.len() > MAX_LINE_BYTES {
        return None;
    }
    let old_words = Words::split(old);
    let new_words = Words::split(new);
    // Myers directly rather than `TextDiff`: `TextDiff` dispatches on an
    // `Algorithm` at run time, which links every algorithm `similar` has
    // (about 300 KiB of code in the release binary).
    let mut capture = Capture::new();
    let Ok(()) = myers::diff(
        &mut capture,
        old_words.words.as_slice(),
        0..old_words.words.len(),
        new_words.words.as_slice(),
        0..new_words.words.len(),
    );
    let ops = capture.into_ops();
    if diff_ratio(&ops, old_words.words.len(), new_words.words.len()) < MIN_UNCHANGED_RATIO {
        return None;
    }
    let mut changed = ChangedRanges::default();
    for op in &ops {
        match op {
            DiffOp::Delete { .. } => {
                push_range(&mut changed.old, old, old_words.byte_range(op.old_range()));
            }
            DiffOp::Insert { .. } => {
                push_range(&mut changed.new, new, new_words.byte_range(op.new_range()));
            }
            _ => {}
        }
    }
    Some(changed)
}

/// A line split at its word boundaries. The words partition the line, so
/// word `i` spans `starts[i]..starts[i + 1]`, the last start being the
/// line's length.
struct Words<'a> {
    words: Vec<&'a str>,
    starts: Vec<usize>,
}

impl<'a> Words<'a> {
    fn split(line: &'a str) -> Self {
        let (mut starts, words): (Vec<usize>, Vec<&str>) = line.split_word_bound_indices().unzip();
        starts.push(line.len());
        Self { words, starts }
    }

    fn byte_range(&self, words: Range<usize>) -> Range<usize> {
        self.starts[words.start]..self.starts[words.end]
    }
}

/// Appends `range` of `line`, joining it to the previous range when only
/// whitespace separates them, so `alpha beta` → `delta epsilon` reads as one
/// change instead of two words with an unmarked space between.
fn push_range(ranges: &mut Vec<Range<usize>>, line: &str, range: Range<usize>) {
    if range.is_empty() {
        return;
    }
    match ranges.last_mut() {
        Some(last) if line[last.end..range.start].trim().is_empty() => last.end = range.end,
        _ => ranges.push(range),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed_text<'a>(line: &'a str, ranges: &[Range<usize>]) -> Vec<&'a str> {
        ranges.iter().map(|r| &line[r.clone()]).collect()
    }

    #[test]
    fn only_the_changed_words_of_an_edited_line_are_marked() {
        let old = "let timeout = config.timeout_seconds(30);";
        let new = "let timeout = config.timeout_seconds(60);";
        let changed = changed_ranges(old, new).expect("lines are similar enough to pair");
        assert_eq!(changed_text(old, &changed.old), ["30"]);
        assert_eq!(changed_text(new, &changed.new), ["60"]);
    }

    #[test]
    fn adjacent_changed_words_merge_into_one_range() {
        let old = "call(alpha beta, gamma)";
        let new = "call(delta epsilon, gamma)";
        let changed = changed_ranges(old, new).expect("lines are similar enough to pair");
        assert_eq!(changed_text(old, &changed.old), ["alpha beta"]);
        assert_eq!(changed_text(new, &changed.new), ["delta epsilon"]);
    }

    #[test]
    fn a_word_only_added_marks_nothing_on_the_old_side() {
        let old = "fn run(a: u32) -> bool {";
        let new = "fn run(a: u32, b: u32) -> bool {";
        let changed = changed_ranges(old, new).expect("lines are similar enough to pair");
        assert_eq!(changed.old, Vec::<Range<usize>>::new());
        assert_eq!(changed_text(new, &changed.new), [", b: u32"]);
    }

    #[test]
    fn a_rewritten_line_gets_no_marks() {
        assert_eq!(
            changed_ranges("return cache.get(key);", "panic!(\"unreachable\")"),
            None
        );
    }

    #[test]
    fn ranges_fall_on_char_boundaries_of_multibyte_text() {
        let old = "println!(\"größe: {}\", n);";
        let new = "println!(\"breite: {}\", n);";
        let changed = changed_ranges(old, new).expect("lines are similar enough to pair");
        assert_eq!(changed_text(old, &changed.old), ["größe"]);
        assert_eq!(changed_text(new, &changed.new), ["breite"]);
    }

    #[test]
    fn a_line_past_the_size_limit_is_not_diffed() {
        let old = "x".repeat(MAX_LINE_BYTES + 1);
        let new = format!("{old}y");
        assert_eq!(changed_ranges(&old, &new), None);
    }
}
