use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

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

/// A character that can stand in a key sequence. A space is excluded so that
/// `"Ctrl+w j"` stays free to spell a sequence of modified keys later.
fn is_sequence_char(c: char) -> bool {
    !c.is_whitespace() && !c.is_control()
}

/// One key a binding names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    /// A character with no modifier other than the Shift an uppercase letter
    /// carries. The only kind of key a sequence is made of.
    Char(char),
    /// `Tab` without modifiers: with Shift the terminal reports `BackTab`.
    Tab,
    /// A named key, matched whatever the modifiers.
    Named(KeyCode),
    /// `Ctrl+` and an ASCII letter, kept lowercase.
    Ctrl(char),
    /// `Alt+` and an ASCII character, kept lowercase.
    Alt(char),
    CtrlEnter,
    AltEnter,
}

impl Key {
    fn matches(self, event: &KeyEvent) -> bool {
        match self {
            Key::Char(c) => {
                event.code == KeyCode::Char(c) && event.modifiers == expected_modifiers(c)
            }
            Key::Tab => event.code == KeyCode::Tab && event.modifiers.is_empty(),
            Key::Named(code) => event.code == code,
            Key::Ctrl(c) => match event.code {
                // Without the keyboard enhancement protocol a terminal sends
                // Ctrl+<letter> as the ASCII control character alone.
                KeyCode::Char(ch) => {
                    (ch.to_ascii_lowercase() == c
                        && event.modifiers.contains(KeyModifiers::CONTROL))
                        || ch == ((c as u8 - b'a' + 1) as char)
                }
                _ => false,
            },
            Key::Alt(c) => {
                matches!(event.code, KeyCode::Char(ch) if ch.to_ascii_lowercase() == c)
                    && event.modifiers.contains(KeyModifiers::ALT)
            }
            // Terminals without the enhancement protocol report Ctrl+Enter as
            // Ctrl+J or a bare line feed.
            Key::CtrlEnter => {
                (event.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(
                        event.code,
                        KeyCode::Enter | KeyCode::Char('j' | '\n' | '\r')
                    ))
                    || (event.code == KeyCode::Char('\n') && event.modifiers.is_empty())
            }
            Key::AltEnter => {
                event.modifiers.contains(KeyModifiers::ALT) && event.code == KeyCode::Enter
            }
        }
    }

    fn event(self) -> KeyEvent {
        let (code, modifiers) = match self {
            Key::Char(c) => (KeyCode::Char(c), expected_modifiers(c)),
            Key::Tab => (KeyCode::Tab, KeyModifiers::NONE),
            Key::Named(code) => (code, KeyModifiers::NONE),
            Key::Ctrl(c) => (KeyCode::Char(c), KeyModifiers::CONTROL),
            Key::Alt(c) => (KeyCode::Char(c), KeyModifiers::ALT),
            Key::CtrlEnter => (KeyCode::Enter, KeyModifiers::CONTROL),
            Key::AltEnter => (KeyCode::Enter, KeyModifiers::ALT),
        };
        KeyEvent::new(code, modifiers)
    }
}

/// A binding string read into the keys it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Binding<'a> {
    Key(Key),
    /// Two or more characters typed one after the other.
    Sequence(&'a str),
}

/// The one reader of binding strings. `None` for a spelling no keypress can
/// match: an unknown name or modifier, a space inside the binding, or an
/// empty string.
fn parse_binding(binding: &str) -> Option<Binding<'_>> {
    if let Some(key) = named_key(binding) {
        return Some(Binding::Key(key));
    }
    if let Some(digits) = binding
        .strip_prefix('F')
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
    {
        return function_key(digits).map(Binding::Key);
    }
    if let Some((modifier, key)) = binding
        .split_once('+')
        .filter(|(modifier, _)| !modifier.is_empty())
    {
        return modified_key(modifier, key).map(Binding::Key);
    }
    let mut chars = binding.chars();
    match (chars.next(), chars.next()) {
        (Some(' '), None) => Some(Binding::Key(Key::Char(' '))),
        (Some(c), None) => is_sequence_char(c).then_some(Binding::Key(Key::Char(c))),
        (Some(_), Some(_)) => binding
            .chars()
            .all(is_sequence_char)
            .then_some(Binding::Sequence(binding)),
        (None, _) => None,
    }
}

fn named_key(name: &str) -> Option<Key> {
    let code = match name {
        "Tab" => return Some(Key::Tab),
        "Ctrl+Enter" | "Ctrl+Return" => return Some(Key::CtrlEnter),
        "Alt+Enter" | "Alt+Return" => return Some(Key::AltEnter),
        "Shift+Tab" => KeyCode::BackTab,
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Backspace" => KeyCode::Backspace,
        "Space" => KeyCode::Char(' '),
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        _ => return None,
    };
    Some(Key::Named(code))
}

/// `F1` to `F99`.
fn function_key(digits: &str) -> Option<Key> {
    const MAX_DIGITS: usize = 2;
    if digits.len() > MAX_DIGITS {
        return None;
    }
    digits.parse().ok().map(|n| Key::Named(KeyCode::F(n)))
}

fn modified_key(modifier: &str, key: &str) -> Option<Key> {
    let mut chars = key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    match modifier {
        "Ctrl" | "ctrl" | "CTRL" if c.is_ascii_alphabetic() => {
            Some(Key::Ctrl(c.to_ascii_lowercase()))
        }
        "Alt" | "alt" | "ALT" if c.is_ascii_graphic() => Some(Key::Alt(c.to_ascii_lowercase())),
        _ => None,
    }
}

/// The character a keypress contributes to a key sequence, if it can be part
/// of one.
fn sequence_char(event: &KeyEvent) -> Option<char> {
    match event.code {
        KeyCode::Char(c) if is_sequence_char(c) && event.modifiers == expected_modifiers(c) => {
            Some(c)
        }
        _ => None,
    }
}

/// What the event loop hands to the key handlers: one key, or the characters
/// of a sequence that completed a binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyPress {
    Key(KeyEvent),
    Sequence(String),
}

/// Stands in for a sequence wherever a handler reads the key code itself, so
/// a fixed key (`KeyCode::Char('j')`) never fires on the last character of a
/// sequence. No terminal reports `KeyCode::Null`.
const SEQUENCE_EVENT: KeyEvent = KeyEvent::new(KeyCode::Null, KeyModifiers::NONE);

impl std::ops::Deref for KeyPress {
    type Target = KeyEvent;

    fn deref(&self) -> &KeyEvent {
        match self {
            KeyPress::Key(event) => event,
            KeyPress::Sequence(_) => &SEQUENCE_EVENT,
        }
    }
}

impl From<KeyEvent> for KeyPress {
    fn from(event: KeyEvent) -> Self {
        KeyPress::Key(event)
    }
}

/// Whether `press` is the key or the whole sequence `binding` names.
pub fn keybinding_matches(binding: &str, press: &KeyPress) -> bool {
    match (parse_binding(binding), press) {
        (Some(Binding::Key(key)), KeyPress::Key(event)) => key.matches(event),
        (Some(Binding::Sequence(sequence)), KeyPress::Sequence(typed)) => sequence == typed,
        _ => false,
    }
}

/// The key event a single-key binding stands for, or `None` when the binding
/// is a sequence or not spelled in the grammar `keybinding_matches` accepts.
/// Lets a binding be tested against other bindings without a real keypress.
pub fn binding_key_event(binding: &str) -> Option<KeyEvent> {
    match parse_binding(binding)? {
        Binding::Key(key) => Some(key.event()),
        Binding::Sequence(_) => None,
    }
}

/// Every configured key sequence, and every start of one the event loop has
/// to wait on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeySequences {
    complete: HashSet<String>,
    prefixes: HashSet<String>,
}

impl KeySequences {
    pub fn new<'a>(bindings: impl IntoIterator<Item = &'a str>) -> Self {
        let mut sequences = Self::default();
        for binding in bindings {
            if let Some(Binding::Sequence(sequence)) = parse_binding(binding) {
                for (end, _) in sequence.char_indices().skip(1) {
                    sequences.prefixes.insert(sequence[..end].to_string());
                }
                sequences.complete.insert(sequence.to_string());
            }
        }
        sequences
    }

    fn is_prefix(&self, typed: &str) -> bool {
        self.prefixes.contains(typed)
    }

    fn is_complete(&self, typed: &str) -> bool {
        self.complete.contains(typed)
    }
}

/// The keys typed towards a sequence, resolved the way vim resolves mappings:
/// keys that start a binding wait, a binding nothing longer starts with fires
/// at once, and one that also starts a longer binding fires once
/// `keybinding_timeout_ms` passes without a key. A key that completes nothing
/// drops the whole sequence.
#[derive(Debug, Default)]
pub struct KeyBuffer {
    sequences: KeySequences,
    typed: String,
    last_key_at: Option<Instant>,
    ready: VecDeque<KeyPress>,
}

impl KeyBuffer {
    pub fn new(sequences: KeySequences) -> Self {
        Self {
            sequences,
            ..Self::default()
        }
    }

    /// Replaces the bindings after a config reload; keys typed towards the
    /// old ones are dropped.
    pub fn set_sequences(&mut self, sequences: KeySequences) {
        self.sequences = sequences;
        self.discard();
    }

    /// A keypress read by a text field, which never takes part in a sequence.
    pub fn pass_through(&mut self, event: KeyEvent) {
        self.discard();
        self.ready.push_back(KeyPress::Key(event));
    }

    pub fn push(&mut self, event: KeyEvent, now: Instant, timeout: Duration) {
        self.expire(now, timeout);
        let Some(c) = sequence_char(&event) else {
            if self.typed.is_empty() {
                self.ready.push_back(KeyPress::Key(event));
            } else {
                self.discard();
            }
            return;
        };
        let is_first_key = self.typed.is_empty();
        self.typed.push(c);
        if self.sequences.is_prefix(&self.typed) {
            self.last_key_at = Some(now);
            return;
        }
        let typed = self.take_typed();
        if is_first_key {
            self.ready.push_back(KeyPress::Key(event));
        } else if self.sequences.is_complete(&typed) {
            self.ready.push_back(KeyPress::Sequence(typed));
        }
    }

    /// Resolves the waiting keys once `timeout` passed since the last one: a
    /// complete binding fires, a single key is handed on as typed (it may be
    /// a fixed key or a custom command), anything else is dropped.
    pub fn expire(&mut self, now: Instant, timeout: Duration) {
        let Some(last_key_at) = self.last_key_at else {
            return;
        };
        if now.saturating_duration_since(last_key_at) < timeout {
            return;
        }
        let typed = self.take_typed();
        let mut chars = typed.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => self.ready.push_back(KeyPress::Key(Key::Char(c).event())),
            _ if self.sequences.is_complete(&typed) => {
                self.ready.push_back(KeyPress::Sequence(typed));
            }
            _ => {}
        }
    }

    pub fn pop_ready(&mut self) -> Option<KeyPress> {
        self.ready.pop_front()
    }

    fn take_typed(&mut self) -> String {
        self.last_key_at = None;
        std::mem::take(&mut self.typed)
    }

    fn discard(&mut self) {
        self.typed.clear();
        self.last_key_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{KeyBuffer, KeyPress, KeySequences, binding_key_event, keybinding_matches};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::time::{Duration, Instant};

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyPress {
        KeyPress::Key(KeyEvent::new(code, modifiers))
    }

    fn char_event(c: char) -> KeyEvent {
        let modifiers = if c.is_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        KeyEvent::new(KeyCode::Char(c), modifiers)
    }

    fn sequence(typed: &str) -> KeyPress {
        KeyPress::Sequence(typed.to_string())
    }

    #[test]
    fn lowercase_single_char_binding_matches_unmodified_key() {
        assert!(keybinding_matches(
            "a",
            &press(KeyCode::Char('a'), KeyModifiers::NONE)
        ));
    }

    #[test]
    fn uppercase_single_char_binding_matches_shifted_key() {
        // crossterm 0.29 attaches KeyModifiers::SHIFT to every uppercase
        // character event, so a binding configured as the literal uppercase
        // letter must match the event Shift+A produces.
        assert!(keybinding_matches(
            "A",
            &press(KeyCode::Char('A'), KeyModifiers::SHIFT)
        ));
    }

    #[test]
    fn lowercase_single_char_binding_does_not_match_shifted_key() {
        assert!(!keybinding_matches(
            "a",
            &press(KeyCode::Char('a'), KeyModifiers::SHIFT)
        ));
    }

    #[test]
    fn ctrl_prefixed_binding_matches_control_modified_key() {
        assert!(keybinding_matches(
            "Ctrl+r",
            &press(KeyCode::Char('r'), KeyModifiers::CONTROL)
        ));
        assert!(keybinding_matches(
            "Ctrl+x",
            &press(KeyCode::Char('x'), KeyModifiers::CONTROL)
        ));
        assert!(keybinding_matches(
            "Ctrl+x",
            &press(KeyCode::Char('X'), KeyModifiers::CONTROL)
        ));
        assert!(keybinding_matches(
            "Ctrl+x",
            &press(KeyCode::Char('\x18'), KeyModifiers::NONE)
        ));
        assert!(keybinding_matches(
            "Ctrl+x",
            &press(KeyCode::Char('\x18'), KeyModifiers::CONTROL)
        ));
    }

    #[test]
    fn alt_prefixed_binding_matches_alt_modified_key() {
        assert!(keybinding_matches(
            "Alt+w",
            &press(KeyCode::Char('w'), KeyModifiers::ALT)
        ));
        assert!(keybinding_matches(
            "Alt+w",
            &press(KeyCode::Char('W'), KeyModifiers::ALT)
        ));
    }

    #[test]
    fn ctrl_enter_matches_ctrl_modified_enter() {
        let ctrl_enter = press(KeyCode::Enter, KeyModifiers::CONTROL);
        assert!(keybinding_matches("Ctrl+Enter", &ctrl_enter));
        assert!(keybinding_matches("Ctrl+Return", &ctrl_enter));
        assert!(keybinding_matches(
            "Ctrl+Enter",
            &press(KeyCode::Char('j'), KeyModifiers::CONTROL)
        ));
        assert!(keybinding_matches(
            "Ctrl+Enter",
            &press(KeyCode::Char('\n'), KeyModifiers::NONE)
        ));
        assert!(!keybinding_matches(
            "Ctrl+Enter",
            &press(KeyCode::Enter, KeyModifiers::NONE)
        ));
    }

    #[test]
    fn function_keys_match_their_key_and_not_a_sequence() {
        assert!(keybinding_matches(
            "F1",
            &press(KeyCode::F(1), KeyModifiers::NONE)
        ));
        assert!(keybinding_matches(
            "F12",
            &press(KeyCode::F(12), KeyModifiers::NONE)
        ));
        assert!(!keybinding_matches("F1", &sequence("F1")));
    }

    #[test]
    fn empty_binding_never_matches() {
        for event in [
            press(KeyCode::Char('s'), KeyModifiers::NONE),
            press(KeyCode::Char('S'), KeyModifiers::SHIFT),
            press(KeyCode::Enter, KeyModifiers::NONE),
        ] {
            assert!(!keybinding_matches("", &event));
        }
    }

    #[test]
    fn sequence_binding_matches_only_its_whole_sequence() {
        assert!(keybinding_matches("gcc", &sequence("gcc")));
        assert!(!keybinding_matches("gcc", &sequence("gc")));
        assert!(!keybinding_matches(
            "gg",
            &press(KeyCode::Char('g'), KeyModifiers::NONE)
        ));
        assert!(!keybinding_matches("g", &sequence("gg")));
    }

    #[test]
    fn lone_space_is_the_space_key_and_a_space_inside_a_binding_matches_nothing() {
        let space = press(KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(keybinding_matches(" ", &space));
        assert!(keybinding_matches("Space", &space));
        assert!(!keybinding_matches("g g", &sequence("g g")));
        assert!(!keybinding_matches("g g", &sequence("gg")));
        assert_eq!(
            KeySequences::new(["g g", "Ctrl+w j"]),
            KeySequences::default()
        );
    }

    /// Every spelling the parser accepts must produce an event its own
    /// binding matches, or a key could be accepted in config and never fire.
    #[test]
    fn binding_key_event_round_trips_through_keybinding_matches() {
        for binding in [
            "a",
            "W",
            "F",
            "é",
            "+",
            "?",
            " ",
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
            "Alt++",
            "Ctrl+Enter",
            "Alt+Return",
        ] {
            let event =
                binding_key_event(binding).unwrap_or_else(|| panic!("{binding:?} should parse"));
            assert!(
                keybinding_matches(binding, &KeyPress::Key(event)),
                "{binding:?} does not match its own event {event:?}"
            );
        }
    }

    /// Sequences and spellings `keybinding_matches` can never match have no
    /// single key event.
    #[test]
    fn binding_key_event_rejects_sequences_and_spellings_that_never_match() {
        for binding in [
            "", "gg", "enter", "F100", "Ctrl+1", "Ctrl+ab", "Shift+a", "Meta+x", "Alt+é", "Alt+ ",
            "g g", "Ctrl+w j", "g+",
        ] {
            assert_eq!(binding_key_event(binding), None, "{binding:?}");
        }
    }

    #[test]
    fn named_keys_and_modifier_combos_never_start_a_sequence() {
        let sequences = KeySequences::new([
            "F1",
            "F5",
            "Up",
            "End",
            "PageDown",
            "Ctrl+x",
            "Alt+x",
            "Shift+Tab",
        ]);
        assert_eq!(sequences, KeySequences::default());
    }

    const TIMEOUT: Duration = Duration::from_millis(1000);

    fn buffer(bindings: &[&str]) -> KeyBuffer {
        KeyBuffer::new(KeySequences::new(bindings.iter().copied()))
    }

    fn type_keys(buffer: &mut KeyBuffer, keys: &str, at: Instant) {
        for c in keys.chars() {
            buffer.push(char_event(c), at, TIMEOUT);
        }
    }

    fn drain(buffer: &mut KeyBuffer) -> Vec<KeyPress> {
        std::iter::from_fn(|| buffer.pop_ready()).collect()
    }

    #[test]
    fn key_that_starts_no_sequence_is_handed_on_at_once() {
        let mut buffer = buffer(&["zz"]);
        type_keys(&mut buffer, "j", Instant::now());
        assert_eq!(drain(&mut buffer), vec![KeyPress::Key(char_event('j'))]);
    }

    #[test]
    fn keys_wait_while_they_start_a_binding() {
        let mut buffer = buffer(&["gcc"]);
        type_keys(&mut buffer, "gc", Instant::now());
        assert_eq!(drain(&mut buffer), vec![]);
    }

    #[test]
    fn complete_sequence_that_starts_nothing_longer_fires_at_once() {
        let mut buffer = buffer(&["gcc", "zz"]);
        let now = Instant::now();
        type_keys(&mut buffer, "gcc", now);
        type_keys(&mut buffer, "zz", now);
        assert_eq!(drain(&mut buffer), vec![sequence("gcc"), sequence("zz")]);
    }

    #[test]
    fn complete_sequence_that_starts_a_longer_one_fires_after_the_timeout() {
        let mut buffer = buffer(&["gc", "gcc"]);
        let start = Instant::now();
        type_keys(&mut buffer, "gc", start);

        buffer.expire(start + TIMEOUT - Duration::from_millis(1), TIMEOUT);
        assert_eq!(drain(&mut buffer), vec![]);

        buffer.expire(start + TIMEOUT, TIMEOUT);
        assert_eq!(drain(&mut buffer), vec![sequence("gc")]);
    }

    #[test]
    fn single_key_that_starts_a_sequence_is_handed_on_after_the_timeout() {
        let mut buffer = buffer(&["gg"]);
        let start = Instant::now();
        type_keys(&mut buffer, "g", start);
        buffer.expire(start + TIMEOUT, TIMEOUT);
        assert_eq!(drain(&mut buffer), vec![KeyPress::Key(char_event('g'))]);
    }

    #[test]
    fn timeout_restarts_with_every_key() {
        let mut buffer = buffer(&["gc", "gcc"]);
        let start = Instant::now();
        type_keys(&mut buffer, "g", start);
        let second_key_at = start + Duration::from_millis(900);
        type_keys(&mut buffer, "c", second_key_at);

        buffer.expire(start + TIMEOUT, TIMEOUT);
        assert_eq!(drain(&mut buffer), vec![]);

        buffer.expire(second_key_at + TIMEOUT, TIMEOUT);
        assert_eq!(drain(&mut buffer), vec![sequence("gc")]);
    }

    #[test]
    fn unfinished_sequence_lapses_after_the_timeout() {
        let mut buffer = buffer(&["gcc"]);
        let start = Instant::now();
        type_keys(&mut buffer, "gc", start);
        buffer.expire(start + TIMEOUT, TIMEOUT);
        assert_eq!(drain(&mut buffer), vec![]);

        type_keys(&mut buffer, "j", start + TIMEOUT);
        assert_eq!(drain(&mut buffer), vec![KeyPress::Key(char_event('j'))]);
    }

    #[test]
    fn key_that_completes_nothing_drops_the_whole_sequence() {
        let mut buffer = buffer(&["g", "gg"]);
        let now = Instant::now();
        type_keys(&mut buffer, "gj", now);
        buffer.push(
            KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
            now,
            TIMEOUT,
        );
        buffer.push(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            now,
            TIMEOUT,
        );
        assert_eq!(drain(&mut buffer), vec![]);

        type_keys(&mut buffer, "j", now);
        assert_eq!(drain(&mut buffer), vec![KeyPress::Key(char_event('j'))]);
    }

    #[test]
    fn key_typed_after_the_timeout_flushes_the_waiting_binding_first() {
        let mut buffer = buffer(&["gc", "gcc"]);
        let start = Instant::now();
        type_keys(&mut buffer, "gc", start);
        type_keys(&mut buffer, "j", start + TIMEOUT);
        assert_eq!(
            drain(&mut buffer),
            vec![sequence("gc"), KeyPress::Key(char_event('j'))]
        );
    }

    #[test]
    fn uppercase_and_non_ascii_characters_form_sequences() {
        let mut buffer = buffer(&["gG", "öö"]);
        let now = Instant::now();
        type_keys(&mut buffer, "gGöö", now);
        assert_eq!(drain(&mut buffer), vec![sequence("gG"), sequence("öö")]);
    }

    #[test]
    fn modified_character_never_extends_a_sequence() {
        let mut buffer = buffer(&["gg"]);
        let now = Instant::now();
        let ctrl_g = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL);
        buffer.push(ctrl_g, now, TIMEOUT);
        assert_eq!(drain(&mut buffer), vec![KeyPress::Key(ctrl_g)]);
    }

    #[test]
    fn key_passed_through_to_a_text_field_drops_waiting_keys() {
        let mut buffer = buffer(&["gg"]);
        let now = Instant::now();
        type_keys(&mut buffer, "g", now);
        buffer.pass_through(char_event('g'));
        buffer.expire(now + TIMEOUT, TIMEOUT);
        assert_eq!(drain(&mut buffer), vec![KeyPress::Key(char_event('g'))]);
    }

    #[test]
    fn sequence_press_hides_its_last_key_from_fixed_key_checks() {
        assert_eq!(sequence("gg").code, KeyCode::Null);
        assert_eq!(
            press(KeyCode::Char('g'), KeyModifiers::NONE).code,
            KeyCode::Char('g')
        );
    }
}
