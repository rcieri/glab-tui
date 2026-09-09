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
}
