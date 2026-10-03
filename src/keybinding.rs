use crossterm::event::{KeyCode, KeyModifiers};

/// The modifiers a binding character's own KeyEvent must carry: `SHIFT` for
/// an uppercase letter (crossterm 0.29 attaches it to every uppercase
/// character event), none otherwise.
fn expected_modifiers(c: char) -> KeyModifiers {
    if c.is_uppercase() {
        KeyModifiers::SHIFT
    } else {
        KeyModifiers::NONE
    }
}

pub fn keybinding_matches(binding: &str, event: &crossterm::event::KeyEvent) -> bool {
    match binding {
        "Tab" => event.code == KeyCode::Tab && event.modifiers.is_empty(),
        "Shift+Tab" => event.code == KeyCode::BackTab,
        "Enter" => event.code == KeyCode::Enter,
        "Esc" => event.code == KeyCode::Esc,
        "Backspace" => event.code == KeyCode::Backspace,
        "Space" => event.code == KeyCode::Char(' '),
        "Up" => event.code == KeyCode::Up,
        "Down" => event.code == KeyCode::Down,
        "Left" => event.code == KeyCode::Left,
        "Right" => event.code == KeyCode::Right,
        "Home" => event.code == KeyCode::Home,
        "End" => event.code == KeyCode::End,
        "PageUp" => event.code == KeyCode::PageUp,
        "PageDown" => event.code == KeyCode::PageDown,
        "F5" => event.code == KeyCode::F(5),
        "Ctrl+Enter" | "Ctrl+Return" => {
            (event
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
                && (event.code == KeyCode::Enter
                    || event.code == KeyCode::Char('j')
                    || event.code == KeyCode::Char('\n')
                    || event.code == KeyCode::Char('\r')))
                || (event.code == KeyCode::Char('\n') && event.modifiers.is_empty())
        }
        "Alt+Enter" | "Alt+Return" => {
            event
                .modifiers
                .contains(crossterm::event::KeyModifiers::ALT)
                && event.code == KeyCode::Enter
        }
        other if other.starts_with('F') && other.len() <= 3 => {
            if let Ok(n) = other[1..].parse::<u8>() {
                event.code == KeyCode::F(n)
            } else {
                false
            }
        }
        other
            if (other.starts_with("Alt+")
                || other.starts_with("alt+")
                || other.starts_with("ALT+"))
                && other.len() == 5 =>
        {
            let c = (other.as_bytes()[4] as char).to_ascii_lowercase();
            match event.code {
                KeyCode::Char(ch) => {
                    ch.to_ascii_lowercase() == c
                        && event
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::ALT)
                }
                _ => false,
            }
        }
        other
            if (other.starts_with("Ctrl+")
                || other.starts_with("ctrl+")
                || other.starts_with("CTRL+"))
                && other.len() == 6 =>
        {
            let c = (other.as_bytes()[5] as char).to_ascii_lowercase();
            if c.is_ascii_lowercase() {
                let ascii_ctrl = (c as u8 - b'a' + 1) as char;
                match event.code {
                    KeyCode::Char(ch) => {
                        (ch.to_ascii_lowercase() == c
                            && event
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL))
                            || ch == ascii_ctrl
                    }
                    _ => false,
                }
            } else {
                false
            }
        }
        other if other.len() == 1 => {
            let c = other.chars().next().unwrap();
            event.code == KeyCode::Char(c) && event.modifiers == expected_modifiers(c)
        }
        _ => false,
    }
}

/// The two-character named key tokens `keybinding_matches` matches
/// literally (the arrow key `"Up"`, the function key `"F5"`), as opposed to
/// two literal characters. `keybinding_char_sets` (in `src/app.rs`) uses
/// this to avoid registering their first character as a sequence prefix.
pub fn is_named_two_char_binding(binding: &str) -> bool {
    matches!(binding, "Up" | "F5")
}

/// Like `keybinding_matches`, but also resolves two-character sequence
/// bindings (e.g. `"gg"`) against a pending first keypress. `pending` is the
/// character captured on the previous keystroke, if any.
///
/// Tries `keybinding_matches` first, so named two-character tokens like
/// `"Up"` or `"F5"` keep matching their key normally instead of being
/// misread as a two-character sequence.
pub fn matches_with_pending(
    binding: &str,
    pending: Option<char>,
    event: &crossterm::event::KeyEvent,
) -> bool {
    if keybinding_matches(binding, event) {
        return true;
    }
    if binding.chars().count() == 2 && !binding.contains('+') {
        let mut chars = binding.chars();
        if let (Some(first), Some(second)) = (chars.next(), chars.next()) {
            return pending == Some(first)
                && event.code == KeyCode::Char(second)
                && event.modifiers.is_empty();
        }
    }
    false
}

/// The key event a single-key binding stands for, or `None` when the binding
/// is not spelled in the grammar `keybinding_matches` accepts. Lets a binding
/// be tested against other bindings without a real keypress.
pub fn binding_key_event(binding: &str) -> Option<crossterm::event::KeyEvent> {
    use crossterm::event::KeyEvent;
    let named = match binding {
        "Tab" => Some(KeyCode::Tab),
        "Shift+Tab" => Some(KeyCode::BackTab),
        "Enter" => Some(KeyCode::Enter),
        "Esc" => Some(KeyCode::Esc),
        "Backspace" => Some(KeyCode::Backspace),
        "Space" => Some(KeyCode::Char(' ')),
        "Up" => Some(KeyCode::Up),
        "Down" => Some(KeyCode::Down),
        "Left" => Some(KeyCode::Left),
        "Right" => Some(KeyCode::Right),
        "Home" => Some(KeyCode::Home),
        "End" => Some(KeyCode::End),
        "PageUp" => Some(KeyCode::PageUp),
        "PageDown" => Some(KeyCode::PageDown),
        _ => None,
    };
    if let Some(code) = named {
        return Some(KeyEvent::new(code, KeyModifiers::NONE));
    }
    match binding {
        "Ctrl+Enter" | "Ctrl+Return" => {
            return Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
        }
        "Alt+Enter" | "Alt+Return" => {
            return Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        }
        _ => {}
    }
    if let Some(number) = binding.strip_prefix('F').filter(|n| n.len() <= 2) {
        return number
            .parse::<u8>()
            .ok()
            .map(|n| KeyEvent::new(KeyCode::F(n), KeyModifiers::NONE));
    }
    if let Some((modifier, key)) = binding.split_once('+').filter(|_| binding.len() > 1) {
        let modifiers = match modifier {
            "Ctrl" | "ctrl" | "CTRL" => KeyModifiers::CONTROL,
            "Alt" | "alt" | "ALT" => KeyModifiers::ALT,
            _ => return None,
        };
        let mut chars = key.chars();
        return match (chars.next(), chars.next()) {
            (Some(c), None)
                if c.is_ascii_alphabetic() || (modifiers == KeyModifiers::ALT && c.is_ascii()) =>
            {
                Some(KeyEvent::new(
                    KeyCode::Char(c.to_ascii_lowercase()),
                    modifiers,
                ))
            }
            _ => None,
        };
    }
    let mut chars = binding.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii() => {
            Some(KeyEvent::new(KeyCode::Char(c), expected_modifiers(c)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::keybinding_matches;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn lowercase_single_char_binding_matches_unmodified_key() {
        let event = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(keybinding_matches("a", &event));
    }

    #[test]
    fn uppercase_single_char_binding_matches_shifted_key() {
        // crossterm 0.29 attaches KeyModifiers::SHIFT to every uppercase
        // character event. Per AGENTS.md's "Keybinding System" section,
        // every keypress must be matched through `keybinding_matches()` so
        // users can remap it; a binding configured as the literal uppercase
        // letter (e.g. "A") must therefore match the KeyEvent a user
        // actually generates by pressing Shift+A.
        let event = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT);
        assert!(keybinding_matches("A", &event));
    }

    #[test]
    fn lowercase_single_char_binding_does_not_match_shifted_key() {
        let event = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SHIFT);
        assert!(!keybinding_matches("a", &event));
    }

    #[test]
    fn ctrl_prefixed_binding_matches_control_modified_key() {
        let event = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);
        assert!(keybinding_matches("Ctrl+r", &event));
        let event_x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert!(keybinding_matches("Ctrl+x", &event_x));
        let event_upper_x = KeyEvent::new(KeyCode::Char('X'), KeyModifiers::CONTROL);
        assert!(keybinding_matches("Ctrl+x", &event_upper_x));
        let event_ascii_ctrl_x = KeyEvent::new(KeyCode::Char('\x18'), KeyModifiers::NONE);
        assert!(keybinding_matches("Ctrl+x", &event_ascii_ctrl_x));
        let event_ascii_ctrl_x_ctrl = KeyEvent::new(KeyCode::Char('\x18'), KeyModifiers::CONTROL);
        assert!(keybinding_matches("Ctrl+x", &event_ascii_ctrl_x_ctrl));
    }

    #[test]
    fn alt_prefixed_binding_matches_alt_modified_key() {
        let event = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT);
        assert!(keybinding_matches("Alt+w", &event));
        let event_upper = KeyEvent::new(KeyCode::Char('W'), KeyModifiers::ALT);
        assert!(keybinding_matches("Alt+w", &event_upper));
    }

    #[test]
    fn ctrl_enter_matches_ctrl_modified_enter() {
        let event_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL);
        assert!(keybinding_matches("Ctrl+Enter", &event_enter));
        assert!(keybinding_matches("Ctrl+Return", &event_enter));

        let event_j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL);
        assert!(keybinding_matches("Ctrl+Enter", &event_j));

        let event_nl = KeyEvent::new(KeyCode::Char('\n'), KeyModifiers::NONE);
        assert!(keybinding_matches("Ctrl+Enter", &event_nl));

        let event_unmodified = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert!(!keybinding_matches("Ctrl+Enter", &event_unmodified));
    }

    #[test]
    fn two_char_binding_matches_second_key_when_pending_holds_first() {
        let event = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(super::matches_with_pending("gg", Some('g'), &event));
    }

    #[test]
    fn two_char_binding_does_not_match_wrong_second_key() {
        let event = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        assert!(!super::matches_with_pending("gg", Some('g'), &event));
    }

    #[test]
    fn two_char_binding_does_not_match_without_pending() {
        let event = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(!super::matches_with_pending("gg", None, &event));
    }

    /// The single-key path is unchanged when no pending key is held.
    #[test]
    fn single_char_binding_behaves_like_keybinding_matches_with_no_pending() {
        let event = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(
            super::matches_with_pending("q", None, &event),
            keybinding_matches("q", &event)
        );
    }

    /// When a pending key is held, a single-char binding still matches its
    /// own key — the pending character is not silently consumed.
    #[test]
    fn single_char_binding_still_matches_when_pending_holds_different_key() {
        let event = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(
            super::matches_with_pending("q", Some('g'), &event),
            keybinding_matches("q", &event)
        );
    }

    #[test]
    fn named_two_char_binding_matches_its_key_regardless_of_pending() {
        // "Up" and "F5" are two-character *named* tokens recognized by
        // keybinding_matches, not two literal characters. They must keep
        // matching their key even though they happen to be two chars long.
        let event = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert!(super::matches_with_pending("Up", None, &event));

        let event = KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE);
        assert!(super::matches_with_pending("F5", Some('g'), &event));
    }

    #[test]
    fn two_char_sequence_binding_counts_chars_not_bytes() {
        // "öö" is two characters but four bytes. matches_with_pending must
        // agree with keybinding_char_sets (which counts chars) on what
        // counts as "two characters", or the binding is dead.
        let event = KeyEvent::new(KeyCode::Char('ö'), KeyModifiers::NONE);
        assert!(super::matches_with_pending("öö", Some('ö'), &event));
    }

    /// Sequences can only spell plain characters; a `Ctrl+`-prefixed
    /// binding never matches through the sequence path.
    #[test]
    fn ctrl_prefixed_binding_does_not_match_through_sequence_path() {
        let event = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(!super::matches_with_pending("Ctrl+g", Some('g'), &event));
    }

    #[test]
    fn empty_binding_never_matches() {
        let event = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE);
        assert!(!keybinding_matches("", &event));
        let event_shift = KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT);
        assert!(!keybinding_matches("", &event_shift));
        let event_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert!(!keybinding_matches("", &event_enter));
    }

    /// Every spelling the parser accepts must produce an event its own
    /// binding matches, or a key could be accepted in config and never fire.
    #[test]
    fn binding_key_event_round_trips_through_keybinding_matches() {
        for binding in [
            "a",
            "W",
            "+",
            "?",
            "Space",
            "Enter",
            "Esc",
            "Tab",
            "Shift+Tab",
            "Backspace",
            "Up",
            "PageDown",
            "Home",
            "F1",
            "F12",
            "Ctrl+g",
            "ctrl+G",
            "CTRL+x",
            "Alt+x",
            "Alt+1",
            "Ctrl+Enter",
            "Alt+Return",
        ] {
            let event = super::binding_key_event(binding)
                .unwrap_or_else(|| panic!("{binding:?} should parse"));
            assert!(
                keybinding_matches(binding, &event),
                "{binding:?} does not match its own event {event:?}"
            );
        }
    }

    /// Spellings `keybinding_matches` can never match are rejected, so they
    /// are reported instead of silently doing nothing.
    #[test]
    fn binding_key_event_rejects_spellings_that_never_match() {
        for binding in [
            "", "gg", "F", "F100", "Ctrl+1", "Ctrl+ab", "Shift+a", "Meta+x", "é", "Alt+é", "enter",
        ] {
            assert_eq!(super::binding_key_event(binding), None, "{binding:?}");
        }
    }
}
