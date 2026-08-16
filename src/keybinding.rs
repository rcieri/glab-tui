use crossterm::event::KeyCode;

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
        other if other.starts_with("Ctrl+") && other.len() == 6 => {
            let c = other.as_bytes()[5];
            event.code == KeyCode::Char(c as char)
                && event
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL)
        }
        other if other.len() == 1 => {
            let c = other.chars().next().unwrap();
            event.code == KeyCode::Char(c) && event.modifiers.is_empty()
        }
        _ => false,
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
    fn uppercase_single_char_binding_does_not_match_shifted_key() {
        // crossterm 0.29 attaches KeyModifiers::SHIFT to every uppercase
        // character event, but the `other.len() == 1` arm above requires
        // `event.modifiers.is_empty()`. So a binding configured as the
        // literal uppercase letter (e.g. "A") never matches the KeyEvent a
        // user actually generates by pressing Shift+A.
        //
        // This is documented, current, intentional behavior: per
        // AGENTS.md's "Keybinding System" section, every uppercase
        // user-facing action must be dispatched with a paired bare
        // `KeyCode::Char(...)` check alongside `keybinding_matches(...)`,
        // e.g.:
        //   _ if key_event.code == KeyCode::Char('A')
        //       || keybinding_matches(&app.config.keybindings.mrs.revoke_mr, key_event) => { ... }
        //
        // Do not "fix" this assertion to expect `true` — that would assert
        // a false expectation. Fix the call site instead.
        let event = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT);
        assert!(!keybinding_matches("A", &event));
    }

    #[test]
    fn ctrl_prefixed_binding_matches_control_modified_key() {
        let event = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);
        assert!(keybinding_matches("Ctrl+r", &event));
    }
}
