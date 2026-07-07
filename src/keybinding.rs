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
