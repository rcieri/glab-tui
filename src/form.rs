//! A dialog built from typed groups of fields and a row of buttons, with one
//! focus model for all of them. `ui::form` draws it.
//!
//! Focus moves between groups with `Tab` / `BackTab` and within a group with
//! `j` / `k`, which continue into the neighbouring group at either end. `h` /
//! `l` move across the radios of a group and across the buttons. `Space`
//! selects a radio, flips a toggle or folds a collapsible section, and so
//! does `Enter` there. `Enter` on a button activates it, and from a line
//! input the primary one.
//! `Esc` leaves a line input being edited before it cancels the form.

use crate::app::TextBuffer;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

/// Names a group so its owner can read it back without relying on its
/// position or its label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GroupId(pub &'static str);

/// Where a group sits: `index` in the form, and `child` inside the
/// collapsible section at that index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupPath {
    pub index: usize,
    pub child: Option<usize>,
}

impl GroupPath {
    pub fn top(index: usize) -> Self {
        Self { index, child: None }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FoldedChildren {
    Include,
    Skip,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FormTone {
    #[default]
    Normal,
    /// The primary action cannot be undone: the border turns red and the
    /// focus and `Enter` go to Cancel, so a reflexive `Enter` does nothing.
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonTone {
    Neutral,
    Positive,
    Destructive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub label: String,
    pub tone: ButtonTone,
}

impl Button {
    pub fn new(label: impl Into<String>, tone: ButtonTone) -> Self {
        Self {
            label: label.into(),
            tone,
        }
    }
}

/// A form's buttons: the primary first and Cancel last, so neither index can
/// name a button that is not there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonRow(Vec<Button>);

impl ButtonRow {
    pub fn new(primary: Button, others: impl IntoIterator<Item = Button>, cancel: Button) -> Self {
        let mut buttons = vec![primary];
        buttons.extend(others);
        buttons.push(cancel);
        Self(buttons)
    }

    /// One button that both runs and dismisses the form.
    pub fn single(button: Button) -> Self {
        Self(vec![button])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The group at this path, on its row `Radio::cursor` / `Toggles::cursor`.
    Group(GroupPath),
    Button(usize),
}

/// What a key or a click asks the form's owner to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormAction {
    /// The form changed or ignored the key: nothing for the owner to do.
    None,
    /// Run the button at this index.
    Activate(usize),
    Cancel,
    /// Edit `text` in `$EDITOR` and hand the result to
    /// [`Form::apply_editor_text`] for `path`.
    OpenEditor {
        path: GroupPath,
        text: String,
    },
}

/// Single choice. A group with one choice is drawn as plain text and is no
/// focus stop, since there is nothing to choose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Radio {
    pub choices: Vec<String>,
    pub selected: usize,
    pub cursor: usize,
}

impl Radio {
    pub fn new(choices: Vec<String>, selected: usize) -> Self {
        Self {
            choices,
            selected,
            cursor: selected,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toggle {
    pub label: String,
    pub is_on: bool,
    is_touched: bool,
}

impl Toggle {
    pub fn new(label: impl Into<String>, is_on: bool) -> Self {
        Self {
            label: label.into(),
            is_on,
            is_touched: false,
        }
    }
}

/// Independent on/off boxes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toggles {
    pub items: Vec<Toggle>,
    pub cursor: usize,
}

impl Toggles {
    pub fn new(items: Vec<Toggle>) -> Self {
        Self { items, cursor: 0 }
    }
}

/// One line of text. A [`Preview`] right after it is its body: `$EDITOR`
/// edits both as one text, the line first, then a blank line, then the body,
/// the layout of a commit message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineInput {
    pub buffer: TextBuffer,
    /// Characters past which the counter turns to a warning colour. The text
    /// may still grow past it.
    pub soft_limit: Option<usize>,
}

impl LineInput {
    pub fn new(value: String, soft_limit: Option<usize>) -> Self {
        Self {
            buffer: TextBuffer::new(value),
            soft_limit,
        }
    }

    pub fn char_count(&self) -> usize {
        self.buffer.value().chars().count()
    }

    pub fn is_over_limit(&self) -> bool {
        self.soft_limit
            .is_some_and(|limit| self.char_count() > limit)
    }
}

/// Read-only lines, folded to their first line and a `+N lines` hint when
/// collapsed or when the form needs the room.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    pub lines: Vec<String>,
    pub is_collapsed: bool,
}

/// A section whose groups only show while it is expanded. `key` opens it
/// and moves the focus to its header from anywhere in the form, and folds
/// it from inside. Its groups are not nested further.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collapsible {
    pub label: String,
    pub key: char,
    pub is_expanded: bool,
    pub groups: Vec<Group>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    Queued,
    Running,
    Done,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListEntry {
    pub label: String,
    pub progress: Progress,
}

impl ListEntry {
    pub fn queued(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            progress: Progress::Queued,
        }
    }
}

/// The items a bulk action runs on, and how far it got with each.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemList {
    pub entries: Vec<ListEntry>,
}

impl ItemList {
    pub fn has_started(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.progress != Progress::Queued)
    }

    /// Entries that finished, successfully or not.
    pub fn finished_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| matches!(entry.progress, Progress::Done | Progress::Failed(_)))
            .count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteLevel {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub level: NoteLevel,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupKind {
    Radio(Radio),
    Toggles(Toggles),
    LineInput(LineInput),
    Preview(Preview),
    Collapsible(Collapsible),
    ItemList(ItemList),
    Note(Note),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub id: Option<GroupId>,
    pub label: Option<String>,
    pub kind: GroupKind,
    is_loading: bool,
    is_touched: bool,
}

impl Group {
    pub fn new(kind: GroupKind) -> Self {
        Self {
            id: None,
            label: None,
            kind,
            is_loading: false,
            is_touched: false,
        }
    }

    pub fn note(level: NoteLevel, text: impl Into<String>) -> Self {
        Self::new(GroupKind::Note(Note {
            level,
            text: text.into(),
        }))
    }

    pub fn with_id(mut self, id: GroupId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Opens the group with fallback values while the real ones are fetched.
    /// They arrive through [`Form::apply_loaded`].
    pub fn loading(mut self) -> Self {
        self.is_loading = true;
        self
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading
    }

    /// Whether the user changed the group, which keeps values that arrive
    /// later from overwriting it. The toggles of a group are tracked one by
    /// one, so this holds once any of them was flipped.
    pub fn is_touched(&self) -> bool {
        self.is_touched
            || matches!(&self.kind, GroupKind::Toggles(toggles) if toggles.items.iter().any(|t| t.is_touched))
    }

    pub fn is_focus_stop(&self) -> bool {
        match &self.kind {
            GroupKind::Radio(radio) => radio.choices.len() > 1,
            GroupKind::Toggles(toggles) => !toggles.items.is_empty(),
            GroupKind::LineInput(_) | GroupKind::Collapsible(_) => true,
            GroupKind::Preview(_) | GroupKind::ItemList(_) | GroupKind::Note(_) => false,
        }
    }

    /// Rows `j` / `k` step through before leaving the group.
    fn focus_rows(&self) -> usize {
        match &self.kind {
            GroupKind::Radio(radio) => radio.choices.len(),
            GroupKind::Toggles(toggles) => toggles.items.len(),
            _ => 1,
        }
    }

    pub fn focus_row(&self) -> usize {
        match &self.kind {
            GroupKind::Radio(radio) => radio.cursor,
            GroupKind::Toggles(toggles) => toggles.cursor,
            _ => 0,
        }
    }

    fn set_focus_row(&mut self, row: usize) {
        match &mut self.kind {
            GroupKind::Radio(radio) => radio.cursor = row,
            GroupKind::Toggles(toggles) => toggles.cursor = row,
            _ => {}
        }
    }
}

/// Where the last draw put each clickable row and button.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormLayout {
    pub targets: Vec<(Rect, ClickTarget)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickTarget {
    /// Row `row` of the group at `path`, as counted by `j` / `k`.
    Row {
        path: GroupPath,
        row: usize,
    },
    Button(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyHint {
    pub key: &'static str,
    pub action: String,
}

impl KeyHint {
    fn new(key: &'static str, action: impl Into<String>) -> Self {
        Self {
            key,
            action: action.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Form {
    pub title: String,
    tone: FormTone,
    groups: Vec<Group>,
    buttons: Vec<Button>,
    primary: usize,
    cancel: usize,
    focus: Focus,
    is_editing: bool,
    /// First body row on screen, moved by each draw to keep the focus visible.
    pub scroll: usize,
    pub layout: Option<FormLayout>,
}

impl Form {
    /// Opens with the focus on the primary button, so an immediate `Enter`
    /// runs it.
    pub fn new(title: impl Into<String>, groups: Vec<Group>, row: ButtonRow) -> Self {
        let ButtonRow(buttons) = row;
        let cancel = buttons.len() - 1;
        Self {
            title: title.into(),
            tone: FormTone::Normal,
            groups,
            buttons,
            primary: 0,
            cancel,
            focus: Focus::Button(0),
            is_editing: false,
            scroll: 0,
            layout: None,
        }
    }

    pub fn danger(mut self) -> Self {
        self.tone = FormTone::Danger;
        self.primary = self.cancel;
        self.focus = Focus::Button(self.cancel);
        self
    }

    pub fn tone(&self) -> FormTone {
        self.tone
    }

    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    pub fn buttons(&self) -> &[Button] {
        &self.buttons
    }

    pub fn primary(&self) -> usize {
        self.primary
    }

    pub fn cancel(&self) -> usize {
        self.cancel
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    pub fn focused_button(&self) -> Option<usize> {
        match self.focus {
            Focus::Button(button) => Some(button),
            Focus::Group(_) => None,
        }
    }

    pub fn is_editing(&self) -> bool {
        self.is_editing
    }

    pub fn is_loading(&self) -> bool {
        self.visible_paths()
            .any(|path| self.group_at(path).is_some_and(Group::is_loading))
    }

    pub fn group_at(&self, path: GroupPath) -> Option<&Group> {
        let group = self.groups.get(path.index)?;
        match (path.child, &group.kind) {
            (None, _) => Some(group),
            (Some(child), GroupKind::Collapsible(section)) => section.groups.get(child),
            (Some(_), _) => None,
        }
    }

    fn group_at_mut(&mut self, path: GroupPath) -> Option<&mut Group> {
        let group = self.groups.get_mut(path.index)?;
        match path.child {
            None => Some(group),
            Some(child) => match &mut group.kind {
                GroupKind::Collapsible(section) => section.groups.get_mut(child),
                _ => None,
            },
        }
    }

    pub fn path_of(&self, id: GroupId) -> Option<GroupPath> {
        self.all_paths()
            .find(|&path| self.group_at(path).is_some_and(|g| g.id == Some(id)))
    }

    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.path_of(id).and_then(|path| self.group_at(path))
    }

    pub fn radio(&self, id: GroupId) -> Option<&Radio> {
        match &self.group(id)?.kind {
            GroupKind::Radio(radio) => Some(radio),
            _ => None,
        }
    }

    pub fn toggles(&self, id: GroupId) -> Option<&Toggles> {
        match &self.group(id)?.kind {
            GroupKind::Toggles(toggles) => Some(toggles),
            _ => None,
        }
    }

    /// Hands values that arrived after the form opened to the group `id`,
    /// unless the user already changed it. A toggle the user flipped keeps
    /// its value while the others take the loaded ones. Either way the group
    /// stops loading. Returns whether `update` ran.
    pub fn apply_loaded(&mut self, id: GroupId, update: impl FnOnce(&mut GroupKind)) -> bool {
        let Some(group) = self.path_of(id).and_then(|path| self.group_at_mut(path)) else {
            return false;
        };
        group.is_loading = false;
        if group.is_touched {
            return false;
        }
        let flipped: Vec<(usize, bool)> = match &group.kind {
            GroupKind::Toggles(toggles) => toggles
                .items
                .iter()
                .enumerate()
                .filter(|(_, toggle)| toggle.is_touched)
                .map(|(index, toggle)| (index, toggle.is_on))
                .collect(),
            _ => Vec::new(),
        };
        update(&mut group.kind);
        match &mut group.kind {
            GroupKind::Radio(radio) => radio.cursor = radio.selected,
            GroupKind::Toggles(toggles) => {
                for (index, is_on) in flipped {
                    if let Some(toggle) = toggles.items.get_mut(index) {
                        toggle.is_on = is_on;
                        toggle.is_touched = true;
                    }
                }
            }
            _ => {}
        }
        true
    }

    /// Every group path, the children of folded sections included.
    fn all_paths(&self) -> impl Iterator<Item = GroupPath> + '_ {
        self.paths(FoldedChildren::Include)
    }

    /// The groups on screen: the children of folded sections are left out.
    pub fn visible_paths(&self) -> impl Iterator<Item = GroupPath> + '_ {
        self.paths(FoldedChildren::Skip)
    }

    fn paths(&self, folded: FoldedChildren) -> impl Iterator<Item = GroupPath> + '_ {
        self.groups
            .iter()
            .enumerate()
            .flat_map(move |(index, group)| {
                let children = match &group.kind {
                    GroupKind::Collapsible(section)
                        if section.is_expanded || folded == FoldedChildren::Include =>
                    {
                        0..section.groups.len()
                    }
                    _ => 0..0,
                };
                std::iter::once(GroupPath::top(index)).chain(children.map(move |child| GroupPath {
                    index,
                    child: Some(child),
                }))
            })
    }

    /// The groups `Tab` stops on, in order. The button row comes after them.
    fn focus_stops(&self) -> Vec<GroupPath> {
        self.visible_paths()
            .filter(|&path| self.group_at(path).is_some_and(Group::is_focus_stop))
            .collect()
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> FormAction {
        let is_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if is_ctrl && key.code == KeyCode::Char('e') {
            return self.open_editor();
        }
        if self.is_editing {
            return self.handle_editing_key(key);
        }
        match key.code {
            KeyCode::Tab => self.step_stop(1),
            KeyCode::BackTab => self.step_stop(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step_row(1),
            KeyCode::Up | KeyCode::Char('k') => self.step_row(-1),
            KeyCode::Left | KeyCode::Char('h') => self.step_across(-1),
            KeyCode::Right | KeyCode::Char('l') => self.step_across(1),
            KeyCode::Char(' ') => self.press_focused(),
            KeyCode::Enter => match self.focus {
                Focus::Button(button) => return FormAction::Activate(button),
                Focus::Group(_) if self.focused_line_input().is_some() => {
                    return FormAction::Activate(self.primary);
                }
                Focus::Group(_) => self.press_focused(),
            },
            KeyCode::Esc => return FormAction::Cancel,
            KeyCode::Char('e') => return self.open_editor(),
            KeyCode::Char('i') if self.focused_line_input().is_some() => self.is_editing = true,
            KeyCode::Char(c) => self.press_section_with_key(c),
            _ => {}
        }
        FormAction::None
    }

    fn handle_editing_key(&mut self, key: &KeyEvent) -> FormAction {
        match key.code {
            KeyCode::Enter => return FormAction::Activate(self.primary),
            KeyCode::Esc => self.is_editing = false,
            KeyCode::Tab => self.step_stop(1),
            KeyCode::BackTab => self.step_stop(-1),
            KeyCode::Char(_) if crate::keybinding::is_shortcut(key.code, key.modifiers) => {}
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete => {
                if let Some(path) = self.focused_line_input() {
                    if let Some(group) = self.group_at_mut(path) {
                        if let GroupKind::LineInput(input) = &mut group.kind {
                            input.buffer.apply_edit_key(key.code);
                            group.is_touched = true;
                        }
                    }
                }
            }
            KeyCode::Left | KeyCode::Right => {
                if let Some(path) = self.focused_line_input() {
                    if let Some(GroupKind::LineInput(input)) =
                        self.group_at_mut(path).map(|group| &mut group.kind)
                    {
                        input.buffer.apply_edit_key(key.code);
                    }
                }
            }
            _ => {}
        }
        FormAction::None
    }

    fn focused_line_input(&self) -> Option<GroupPath> {
        match self.focus {
            Focus::Group(path)
                if matches!(
                    self.group_at(path).map(|g| &g.kind),
                    Some(GroupKind::LineInput(_))
                ) =>
            {
                Some(path)
            }
            _ => None,
        }
    }

    /// The focused line input, else the first one on screen.
    fn editor_target(&self) -> Option<GroupPath> {
        self.focused_line_input().or_else(|| {
            self.visible_paths().find(|&path| {
                matches!(
                    self.group_at(path).map(|g| &g.kind),
                    Some(GroupKind::LineInput(_))
                )
            })
        })
    }

    fn open_editor(&mut self) -> FormAction {
        let Some(path) = self.editor_target() else {
            return FormAction::None;
        };
        let Some(text) = self.editor_text(path) else {
            return FormAction::None;
        };
        FormAction::OpenEditor { path, text }
    }

    /// The text `$EDITOR` opens for the line input at `path`: its value, and
    /// the lines of the preview after it below a blank line.
    pub fn editor_text(&self, path: GroupPath) -> Option<String> {
        let GroupKind::LineInput(input) = &self.group_at(path)?.kind else {
            return None;
        };
        let mut text = input.buffer.value().to_string();
        if let Some(preview) = self.body_preview(path) {
            if !preview.lines.is_empty() {
                text.push_str("\n\n");
                text.push_str(&preview.lines.join("\n"));
            }
        }
        Some(text)
    }

    /// Splits `text` from `$EDITOR` back into the line input at `path` (its
    /// first line) and the preview after it (the rest, without the blank
    /// line between them).
    pub fn apply_editor_text(&mut self, path: GroupPath, text: &str) {
        let mut lines = text.trim_end().lines();
        let first = lines.next().unwrap_or_default().trim_end().to_string();
        let rest: Vec<String> = lines
            .skip_while(|line| line.trim().is_empty())
            .map(str::to_string)
            .collect();
        let preview_path = self.body_preview_path(path);
        let Some(group) = self.group_at_mut(path) else {
            return;
        };
        let GroupKind::LineInput(input) = &mut group.kind else {
            return;
        };
        if input.buffer.value() != first {
            input.buffer.set_value(first);
            group.is_touched = true;
        }
        if let Some(preview_path) = preview_path {
            if let Some(group) = self.group_at_mut(preview_path) {
                if let GroupKind::Preview(preview) = &mut group.kind {
                    if preview.lines != rest {
                        preview.lines = rest;
                        group.is_touched = true;
                    }
                }
            }
        }
    }

    fn body_preview_path(&self, path: GroupPath) -> Option<GroupPath> {
        let next = match path.child {
            None => GroupPath::top(path.index + 1),
            Some(child) => GroupPath {
                index: path.index,
                child: Some(child + 1),
            },
        };
        matches!(
            self.group_at(next).map(|g| &g.kind),
            Some(GroupKind::Preview(_))
        )
        .then_some(next)
    }

    fn body_preview(&self, path: GroupPath) -> Option<&Preview> {
        match &self.group_at(self.body_preview_path(path)?)?.kind {
            GroupKind::Preview(preview) => Some(preview),
            _ => None,
        }
    }

    /// Moves to the next (`step > 0`) or previous focus stop, the button row
    /// being the last one, wrapping at both ends. Landing on a line input
    /// starts editing it. A form with no stop steps across its buttons
    /// instead, so `Tab` still reaches Cancel.
    fn step_stop(&mut self, step: isize) {
        let stops = self.focus_stops();
        if stops.is_empty() {
            self.step_across(step);
            return;
        }
        let count = stops.len() + 1;
        let current = match self.focus {
            Focus::Group(path) => stops.iter().position(|&p| p == path).unwrap_or(0),
            Focus::Button(_) => stops.len(),
        };
        let next = (current as isize + step).rem_euclid(count as isize) as usize;
        self.is_editing = false;
        match stops.get(next) {
            Some(&path) => {
                self.enter_group(path, step);
                self.is_editing = self.focused_line_input().is_some();
            }
            None => self.focus = Focus::Button(self.primary),
        }
    }

    /// Moves one row down (`step > 0`) or up within the focused group, and
    /// past its last or first row into the neighbouring stop, without
    /// wrapping around the form.
    fn step_row(&mut self, step: isize) {
        let stops = self.focus_stops();
        match self.focus {
            Focus::Group(path) => {
                let Some(group) = self.group_at(path) else {
                    return;
                };
                let row = group.focus_row() as isize + step;
                if (0..group.focus_rows() as isize).contains(&row) {
                    if let Some(group) = self.group_at_mut(path) {
                        group.set_focus_row(row as usize);
                    }
                    return;
                }
                let position = stops.iter().position(|&p| p == path).unwrap_or(0);
                match position.checked_add_signed(step) {
                    Some(next) if next < stops.len() => self.enter_group(stops[next], step),
                    Some(_) => self.focus = Focus::Button(self.primary),
                    None => {}
                }
            }
            Focus::Button(_) if step < 0 => {
                if let Some(&last) = stops.last() {
                    self.enter_group(last, step);
                }
            }
            Focus::Button(_) => {}
        }
    }

    /// Focuses the group at `path` on its first row when entered from above
    /// (`step > 0`), on its last row from below. A radio is entered on its
    /// selected choice, so the highlight and the selection agree.
    fn enter_group(&mut self, path: GroupPath, step: isize) {
        if let Some(group) = self.group_at_mut(path) {
            let row = match &group.kind {
                GroupKind::Radio(radio) => radio.selected,
                _ if step > 0 => 0,
                _ => group.focus_rows().saturating_sub(1),
            };
            group.set_focus_row(row);
        }
        self.focus = Focus::Group(path);
    }

    /// Moves across the radios of the focused group or across the buttons,
    /// stopping at either end.
    fn step_across(&mut self, step: isize) {
        match self.focus {
            Focus::Button(button) => {
                let last = self.buttons.len() - 1;
                self.focus = Focus::Button(button.saturating_add_signed(step).min(last));
            }
            Focus::Group(path) => {
                if let Some(GroupKind::Radio(radio)) =
                    self.group_at_mut(path).map(|group| &mut group.kind)
                {
                    let last = radio.choices.len().saturating_sub(1);
                    radio.cursor = radio.cursor.saturating_add_signed(step).min(last);
                }
            }
        }
    }

    fn press_focused(&mut self) {
        let Focus::Group(path) = self.focus else {
            return;
        };
        let Some(group) = self.group_at_mut(path) else {
            return;
        };
        match &mut group.kind {
            GroupKind::Radio(radio) => {
                radio.selected = radio.cursor;
                group.is_touched = true;
            }
            GroupKind::Toggles(toggles) => {
                if let Some(toggle) = toggles.items.get_mut(toggles.cursor) {
                    toggle.is_on = !toggle.is_on;
                    toggle.is_touched = true;
                }
            }
            GroupKind::Collapsible(_) => self.toggle_section(path.index),
            GroupKind::LineInput(_) => self.is_editing = true,
            _ => {}
        }
    }

    fn press_section_with_key(&mut self, key: char) {
        let section = self.groups.iter().position(
            |group| matches!(&group.kind, GroupKind::Collapsible(section) if section.key == key),
        );
        if let Some(index) = section {
            self.press_section(index);
        }
    }

    /// Folds or unfolds the section at `index` when the focus is in it.
    /// From outside it only opens the section and focuses its header: the
    /// renderer folds sections the focus is outside of once the form is
    /// short, so a toggle from there would flip between two states drawn
    /// the same.
    fn press_section(&mut self, index: usize) {
        let has_focus = matches!(self.focus, Focus::Group(path) if path.index == index);
        if has_focus {
            self.toggle_section(index);
            return;
        }
        if let Some(GroupKind::Collapsible(section)) =
            self.groups.get_mut(index).map(|group| &mut group.kind)
        {
            section.is_expanded = true;
        }
        self.focus = Focus::Group(GroupPath::top(index));
        self.is_editing = false;
    }

    /// Folds or unfolds the section at `index`. Folding it with the focus
    /// inside moves the focus to its header.
    fn toggle_section(&mut self, index: usize) {
        let Some(GroupKind::Collapsible(section)) =
            self.groups.get_mut(index).map(|group| &mut group.kind)
        else {
            return;
        };
        section.is_expanded = !section.is_expanded;
        let is_expanded = section.is_expanded;
        if let Focus::Group(path) = self.focus {
            if !is_expanded && path.index == index && path.child.is_some() {
                self.focus = Focus::Group(GroupPath::top(index));
                self.is_editing = false;
            }
        }
    }

    /// A click on `target`: the same as moving the focus there and pressing
    /// `Space`, or `Enter` on a button. A section header clicked from
    /// outside the section is pressed like its key instead.
    pub fn click(&mut self, target: ClickTarget) -> FormAction {
        match target {
            ClickTarget::Button(button) if button < self.buttons.len() => {
                self.focus = Focus::Button(button);
                self.is_editing = false;
                FormAction::Activate(button)
            }
            ClickTarget::Button(_) => FormAction::None,
            ClickTarget::Row { path, row } => {
                let Some(group) = self.group_at_mut(path) else {
                    return FormAction::None;
                };
                if !group.is_focus_stop() {
                    return FormAction::None;
                }
                if path.child.is_none() && matches!(group.kind, GroupKind::Collapsible(_)) {
                    self.press_section(path.index);
                    return FormAction::None;
                }
                group.set_focus_row(row);
                self.focus = Focus::Group(path);
                self.is_editing = false;
                self.press_focused();
                FormAction::None
            }
        }
    }

    pub fn click_at(&mut self, column: u16, row: u16) -> FormAction {
        let target = self.layout.as_ref().and_then(|layout| {
            layout
                .targets
                .iter()
                .find(|(rect, _)| rect.contains((column, row).into()))
                .map(|&(_, target)| target)
        });
        target.map_or(FormAction::None, |target| self.click(target))
    }

    /// The keys the focused group answers to, then the form-wide ones, for
    /// the footer.
    pub fn key_hints(&self) -> Vec<KeyHint> {
        let primary = self.buttons[self.primary].label.to_lowercase();
        if self.is_editing {
            return vec![
                KeyHint::new("Enter", primary),
                KeyHint::new("Tab", "next"),
                KeyHint::new("Esc", "leave field"),
                KeyHint::new("Ctrl+E", "editor"),
            ];
        }
        let mut hints = match self.focus {
            Focus::Button(_) => vec![KeyHint::new("h/l", "button")],
            Focus::Group(path) => match self.group_at(path).map(|g| &g.kind) {
                Some(GroupKind::Radio(_)) => vec![
                    KeyHint::new("h/l", "move"),
                    KeyHint::new("Space/Enter", "select"),
                ],
                Some(GroupKind::Toggles(_)) => vec![
                    KeyHint::new("j/k", "move"),
                    KeyHint::new("Space/Enter", "toggle"),
                ],
                Some(GroupKind::LineInput(_)) => vec![KeyHint::new("i", "type")],
                Some(GroupKind::Collapsible(section)) => vec![KeyHint::new(
                    "Space/Enter",
                    if section.is_expanded {
                        "collapse"
                    } else {
                        "expand"
                    },
                )],
                _ => Vec::new(),
            },
        };
        if self.editor_target().is_some() {
            hints.push(KeyHint::new("e", "editor"));
        }
        hints.push(KeyHint::new("Tab", "next"));
        let enter = match self.focus {
            Focus::Button(button) => Some(self.buttons[button].label.to_lowercase()),
            Focus::Group(_) if self.focused_line_input().is_some() => Some(primary),
            Focus::Group(_) => None,
        };
        if let Some(enter) = enter {
            hints.push(KeyHint::new("Enter", enter));
        }
        hints.push(KeyHint::new("Esc", "cancel"));
        hints
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRATEGY: GroupId = GroupId("strategy");
    const OPTIONS: GroupId = GroupId("options");
    const SUBJECT: GroupId = GroupId("subject");

    fn press(form: &mut Form, code: KeyCode) -> FormAction {
        form.handle_key(&KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn strategy(selected: usize) -> Group {
        Group::new(GroupKind::Radio(Radio::new(
            vec!["Merge".into(), "Squash".into(), "Rebase".into()],
            selected,
        )))
        .with_id(STRATEGY)
        .with_label("Strategy")
    }

    fn options() -> Group {
        Group::new(GroupKind::Toggles(Toggles::new(vec![
            Toggle::new("Delete branch", true),
            Toggle::new("Auto-merge", false),
        ])))
        .with_id(OPTIONS)
        .with_label("Options")
    }

    fn merge_form() -> Form {
        Form::new(
            "Merge !1",
            vec![
                Group::note(NoteLevel::Info, "feature → main"),
                strategy(1),
                options(),
            ],
            ButtonRow::new(
                Button::new("Merge", ButtonTone::Positive),
                [],
                Button::new("Cancel", ButtonTone::Neutral),
            ),
        )
    }

    #[test]
    fn a_new_form_focuses_the_primary_button_so_enter_runs_it() {
        let mut form = merge_form();
        assert_eq!(form.focus(), Focus::Button(0));
        assert_eq!(press(&mut form, KeyCode::Enter), FormAction::Activate(0));
    }

    #[test]
    fn tab_walks_the_focusable_groups_then_the_buttons_and_wraps() {
        let mut form = merge_form();
        let mut visited = Vec::new();
        for _ in 0..4 {
            press(&mut form, KeyCode::Tab);
            visited.push(form.focus());
        }
        assert_eq!(
            visited,
            [
                Focus::Group(GroupPath::top(1)),
                Focus::Group(GroupPath::top(2)),
                Focus::Button(0),
                Focus::Group(GroupPath::top(1)),
            ]
        );
        press(&mut form, KeyCode::BackTab);
        assert_eq!(form.focus(), Focus::Button(0));
    }

    #[test]
    fn tab_back_onto_a_radio_lands_on_its_selected_choice() {
        let mut form = merge_form();
        press(&mut form, KeyCode::Tab);
        press(&mut form, KeyCode::Char('l'));
        press(&mut form, KeyCode::Tab);
        press(&mut form, KeyCode::BackTab);
        assert_eq!(form.radio(STRATEGY).map(|r| r.cursor), Some(1));
        press(&mut form, KeyCode::Char(' '));
        assert_eq!(form.radio(STRATEGY).map(|r| r.selected), Some(1));
    }

    #[test]
    fn j_and_k_move_within_a_group_and_continue_into_its_neighbours() {
        let mut form = merge_form();
        press(&mut form, KeyCode::Char('k'));
        assert_eq!(form.focus(), Focus::Group(GroupPath::top(2)));
        assert_eq!(form.toggles(OPTIONS).map(|t| t.cursor), Some(1));
        press(&mut form, KeyCode::Char('k'));
        press(&mut form, KeyCode::Char('k'));
        assert_eq!(form.focus(), Focus::Group(GroupPath::top(1)));
        assert_eq!(
            form.radio(STRATEGY).map(|r| r.cursor),
            Some(1),
            "a radio is entered on its selected choice"
        );
        for _ in 0..3 {
            press(&mut form, KeyCode::Char('k'));
        }
        assert_eq!(form.radio(STRATEGY).map(|r| r.cursor), Some(0));
        assert_eq!(
            form.focus(),
            Focus::Group(GroupPath::top(1)),
            "stops at the top"
        );
        for _ in 0..5 {
            press(&mut form, KeyCode::Char('j'));
        }
        assert_eq!(form.focus(), Focus::Button(0));
    }

    #[test]
    fn space_and_enter_select_the_radio_under_the_cursor_without_running_the_primary() {
        let mut form = merge_form();
        press(&mut form, KeyCode::Tab);
        press(&mut form, KeyCode::Char('l'));
        assert_eq!(
            form.radio(STRATEGY).map(|r| r.selected),
            Some(1),
            "moving does not select"
        );
        press(&mut form, KeyCode::Char(' '));
        assert_eq!(form.radio(STRATEGY).map(|r| r.selected), Some(2));
        press(&mut form, KeyCode::Char('h'));
        assert_eq!(press(&mut form, KeyCode::Enter), FormAction::None);
        assert_eq!(form.radio(STRATEGY).map(|r| r.selected), Some(1));
    }

    #[test]
    fn enter_on_a_toggle_flips_it_instead_of_merging() {
        let mut form = merge_form();
        press(&mut form, KeyCode::Char('k'));
        press(&mut form, KeyCode::Char('k'));
        assert_eq!(press(&mut form, KeyCode::Enter), FormAction::None);
        assert_eq!(form.toggles(OPTIONS).map(|t| t.items[0].is_on), Some(false));
    }

    #[test]
    fn space_flips_only_the_toggle_under_the_cursor() {
        let mut form = merge_form();
        press(&mut form, KeyCode::Char('k'));
        press(&mut form, KeyCode::Char(' '));
        let states: Vec<bool> = form
            .toggles(OPTIONS)
            .map(|t| t.items.iter().map(|i| i.is_on).collect())
            .unwrap_or_default();
        assert_eq!(states, [true, true]);
    }

    #[test]
    fn a_radio_with_one_choice_is_no_focus_stop() {
        let mut form = merge_form();
        form.groups[1] = Group::new(GroupKind::Radio(Radio::new(vec!["Squash".into()], 0)));
        press(&mut form, KeyCode::Tab);
        assert_eq!(form.focus(), Focus::Group(GroupPath::top(2)));
    }

    #[test]
    fn h_and_l_move_across_the_buttons_and_stop_at_the_ends() {
        let mut form = merge_form();
        press(&mut form, KeyCode::Char('h'));
        assert_eq!(form.focus(), Focus::Button(0));
        press(&mut form, KeyCode::Char('l'));
        press(&mut form, KeyCode::Char('l'));
        assert_eq!(form.focus(), Focus::Button(1));
        assert_eq!(press(&mut form, KeyCode::Enter), FormAction::Activate(1));
    }

    #[test]
    fn tab_steps_across_the_buttons_of_a_form_without_focus_stops() {
        let mut form = Form::new(
            "Rebase !1",
            vec![Group::note(NoteLevel::Info, "Rebase !1 onto main?")],
            ButtonRow::new(
                Button::new("Rebase", ButtonTone::Positive),
                [],
                Button::new("Cancel", ButtonTone::Neutral),
            ),
        );
        press(&mut form, KeyCode::Tab);
        assert_eq!(form.focus(), Focus::Button(1));
        press(&mut form, KeyCode::Tab);
        assert_eq!(form.focus(), Focus::Button(1), "stops at Cancel");
        assert_eq!(press(&mut form, KeyCode::Enter), FormAction::Activate(1));
        press(&mut form, KeyCode::BackTab);
        assert_eq!(form.focus(), Focus::Button(0));
    }

    #[test]
    fn a_danger_form_opens_on_cancel_and_enter_on_a_field_runs_nothing() {
        let mut form = merge_form().danger();
        assert_eq!(form.tone(), FormTone::Danger);
        assert_eq!(form.focus(), Focus::Button(1));
        press(&mut form, KeyCode::Tab);
        assert_eq!(press(&mut form, KeyCode::Enter), FormAction::None);
        press(&mut form, KeyCode::BackTab);
        assert_eq!(
            form.focus(),
            Focus::Button(1),
            "the button row is entered on Cancel"
        );
        press(&mut form, KeyCode::Char('h'));
        assert_eq!(press(&mut form, KeyCode::Enter), FormAction::Activate(0));
    }

    fn message_form() -> Form {
        Form::new(
            "Merge",
            vec![
                Group::new(GroupKind::LineInput(LineInput::new("Fix".into(), Some(5))))
                    .with_id(SUBJECT),
                Group::new(GroupKind::Preview(Preview {
                    lines: vec!["Body".into()],
                    is_collapsed: false,
                })),
            ],
            ButtonRow::new(
                Button::new("Merge", ButtonTone::Positive),
                [],
                Button::new("Cancel", ButtonTone::Neutral),
            ),
        )
    }

    fn subject(form: &Form) -> String {
        match &form.group(SUBJECT).map(|g| &g.kind) {
            Some(GroupKind::LineInput(input)) => input.buffer.value().to_string(),
            _ => String::new(),
        }
    }

    #[test]
    fn tab_onto_a_line_input_types_into_it_and_esc_leaves_it_before_cancelling() {
        let mut form = message_form();
        press(&mut form, KeyCode::Tab);
        assert!(form.is_editing());
        for c in "es j".chars() {
            press(&mut form, KeyCode::Char(c));
        }
        assert_eq!(subject(&form), "Fixes j");
        assert_eq!(press(&mut form, KeyCode::Esc), FormAction::None);
        assert!(!form.is_editing());
        assert_eq!(press(&mut form, KeyCode::Esc), FormAction::Cancel);
    }

    #[test]
    fn enter_while_typing_runs_the_primary_button() {
        let mut form = message_form();
        press(&mut form, KeyCode::Tab);
        assert_eq!(press(&mut form, KeyCode::Enter), FormAction::Activate(0));
    }

    #[test]
    fn altgr_characters_reach_a_line_input_but_ctrl_shortcuts_do_not() {
        let mut form = message_form();
        press(&mut form, KeyCode::Tab);
        let ctrl_alt = KeyModifiers::CONTROL | KeyModifiers::ALT;
        form.handle_key(&KeyEvent::new(KeyCode::Char('@'), ctrl_alt));
        form.handle_key(&KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(subject(&form), "Fix@");
    }

    #[test]
    fn the_counter_warns_past_the_soft_limit_without_cutting_the_text() {
        let mut input = LineInput::new("12345".into(), Some(5));
        assert!(!input.is_over_limit());
        input.buffer.insert_char('6');
        assert!(input.is_over_limit());
        assert_eq!(input.char_count(), 6);
    }

    #[test]
    fn e_opens_the_line_and_its_body_in_the_editor_and_the_result_is_split_back() {
        let mut form = message_form();
        let action = press(&mut form, KeyCode::Char('e'));
        assert_eq!(
            action,
            FormAction::OpenEditor {
                path: GroupPath::top(0),
                text: "Fix\n\nBody".to_string(),
            }
        );
        form.apply_editor_text(GroupPath::top(0), "Fix the bug\n\n\nFirst\nSecond\n");
        assert_eq!(subject(&form), "Fix the bug");
        assert_eq!(
            form.groups()[1].kind,
            GroupKind::Preview(Preview {
                lines: vec!["First".into(), "Second".into()],
                is_collapsed: false,
            })
        );
    }

    #[test]
    fn text_saved_unchanged_in_the_editor_leaves_the_groups_untouched() {
        let mut form = message_form();
        let text = form.editor_text(GroupPath::top(0)).unwrap_or_default();
        form.apply_editor_text(GroupPath::top(0), &text);
        assert_eq!(
            (form.groups()[0].is_touched(), form.groups()[1].is_touched()),
            (false, false)
        );
    }

    #[test]
    fn values_loaded_late_fill_untouched_groups_but_never_overwrite_an_edit() {
        let mut form = merge_form();
        form.groups[1] = strategy(1).loading();
        form.groups[2] = options().loading();
        assert!(form.is_loading());

        press(&mut form, KeyCode::Char('k'));
        press(&mut form, KeyCode::Char(' '));
        let strategy_applied = form.apply_loaded(STRATEGY, |kind| {
            if let GroupKind::Radio(radio) = kind {
                radio.selected = 2;
            }
        });
        let options_applied = form.apply_loaded(OPTIONS, |kind| {
            if let GroupKind::Toggles(toggles) = kind {
                toggles.items[0].is_on = false;
            }
        });

        assert_eq!((strategy_applied, options_applied), (true, true));
        assert_eq!(
            form.radio(STRATEGY).map(|r| (r.selected, r.cursor)),
            Some((2, 2)),
            "the cursor follows the loaded choice"
        );
        assert_eq!(
            form.toggles(OPTIONS)
                .map(|t| (t.items[0].is_on, t.items[1].is_on)),
            Some((false, true)),
            "the loaded Delete branch applies, the flipped Auto-merge stays"
        );
        assert!(!form.is_loading());
    }

    fn form_with_advanced() -> Form {
        Form::new(
            "Merge",
            vec![
                options(),
                Group::new(GroupKind::Collapsible(Collapsible {
                    label: "Advanced".into(),
                    key: 'a',
                    is_expanded: false,
                    groups: vec![Group::new(GroupKind::Toggles(Toggles::new(vec![
                        Toggle::new("Bypass rules", false),
                    ])))],
                })),
            ],
            ButtonRow::new(
                Button::new("Merge", ButtonTone::Positive),
                [],
                Button::new("Cancel", ButtonTone::Neutral),
            ),
        )
    }

    #[test]
    fn a_folded_section_hides_its_groups_from_tab_until_its_key_unfolds_it() {
        let mut form = form_with_advanced();
        assert_eq!(form.focus_stops().len(), 2);
        press(&mut form, KeyCode::Char('a'));
        assert_eq!(
            form.focus_stops(),
            [
                GroupPath::top(0),
                GroupPath::top(1),
                GroupPath {
                    index: 1,
                    child: Some(0)
                }
            ]
        );
    }

    #[test]
    fn folding_a_section_with_the_focus_inside_moves_it_to_the_header() {
        let mut form = form_with_advanced();
        press(&mut form, KeyCode::Char('a'));
        press(&mut form, KeyCode::Tab);
        assert_eq!(
            form.focus(),
            Focus::Group(GroupPath {
                index: 1,
                child: Some(0)
            })
        );
        press(&mut form, KeyCode::Char('a'));
        assert_eq!(form.focus(), Focus::Group(GroupPath::top(1)));
    }

    #[test]
    fn the_section_key_from_outside_opens_the_section_and_focuses_its_header() {
        let mut form = form_with_advanced();
        press(&mut form, KeyCode::Char('a'));
        press(&mut form, KeyCode::Tab);
        press(&mut form, KeyCode::Tab);
        assert_eq!(form.focus(), Focus::Button(0));
        press(&mut form, KeyCode::Char('a'));
        assert_eq!(form.focus(), Focus::Group(GroupPath::top(1)));
        assert_eq!(form.focus_stops().len(), 3, "the section stays open");
    }

    #[test]
    fn clicking_a_toggle_flips_it_and_clicking_a_button_runs_it() {
        let mut form = merge_form();
        let path = GroupPath::top(2);
        assert_eq!(
            form.click(ClickTarget::Row { path, row: 1 }),
            FormAction::None
        );
        assert_eq!(form.focus(), Focus::Group(path));
        assert_eq!(form.toggles(OPTIONS).map(|t| t.items[1].is_on), Some(true));
        assert_eq!(form.click(ClickTarget::Button(1)), FormAction::Activate(1));
    }

    #[test]
    fn the_footer_names_the_keys_of_the_focused_group() {
        let mut form = merge_form();
        let keys = |form: &Form| -> Vec<String> {
            form.key_hints()
                .into_iter()
                .map(|h| format!("{} {}", h.key, h.action))
                .collect()
        };
        assert_eq!(
            keys(&form),
            ["h/l button", "Tab next", "Enter merge", "Esc cancel"]
        );
        press(&mut form, KeyCode::Tab);
        assert_eq!(
            keys(&form),
            ["h/l move", "Space/Enter select", "Tab next", "Esc cancel"]
        );
    }

    #[test]
    fn a_bulk_list_counts_finished_entries_failures_included() {
        let list = ItemList {
            entries: vec![
                ListEntry {
                    label: "!1".into(),
                    progress: Progress::Done,
                },
                ListEntry {
                    label: "!2".into(),
                    progress: Progress::Failed("conflict".into()),
                },
                ListEntry {
                    label: "!3".into(),
                    progress: Progress::Running,
                },
                ListEntry::queued("!4"),
            ],
        };
        assert!(list.has_started());
        assert_eq!(list.finished_count(), 2);
    }
}
