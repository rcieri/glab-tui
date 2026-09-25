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

/// Like `keybinding_matches`, but also resolves two-character sequence
/// bindings (e.g. `"gg"`) against a pending first keypress. `pending` is the
/// character captured on the previous keystroke, if any.
///
/// Sequence spelling cannot contain a modifier (no `+`), so this only
/// resolves the all-plain-text two-character form. The single-key path
/// delegates to `keybinding_matches` so its semantics are unchanged when
/// `pending` is `None`.
#[allow(dead_code)] // used by bindings that opt into two-character sequences
pub fn matches_with_pending(
    binding: &str,
    pending: Option<char>,
    event: &crossterm::event::KeyEvent,
) -> bool {
    if binding.len() == 2 && !binding.contains('+') {
        let mut chars = binding.chars();
        if let (Some(first), Some(second)) = (chars.next(), chars.next()) {
            return pending == Some(first)
                && event.code == KeyCode::Char(second)
                && event.modifiers.is_empty();
        }
    }
    keybinding_matches(binding, event)
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

    /// Sequences can only spell plain characters; a `Ctrl+`-prefixed
    /// binding never matches through the sequence path.
    #[test]
    fn ctrl_prefixed_binding_does_not_match_through_sequence_path() {
        let event = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(!super::matches_with_pending("Ctrl+g", Some('g'), &event));
    }
}
