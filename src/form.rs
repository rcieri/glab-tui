//! The model every overlay is drawn from: its groups of controls, its button
//! row, and which control holds the focus. `ui::form` lays it out and draws
//! it; the overlay owns the `Form` and reads its values back on submit.
//!
//! `F` names the controls (a radio choice, a toggle, a button), so a submit
//! reads typed ids instead of parsing labels.

use crate::app::TextBuffer;
use ratatui::layout::Rect;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice<F> {
    pub id: F,
    pub label: String,
}

/// Exactly one of `choices` is selected. A radio with a single choice has
/// nothing to pick: it is drawn as plain text and takes no focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Radio<F> {
    pub label: String,
    pub choices: Vec<Choice<F>>,
    pub selected: usize,
}

impl<F> Radio<F> {
    pub fn selected_id(&self) -> Option<&F> {
        self.choices.get(self.selected).map(|choice| &choice.id)
    }

    fn is_focusable(&self) -> bool {
        self.choices.len() > 1
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toggle<F> {
    pub id: F,
    pub label: String,
    pub checked: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineInput {
    pub buffer: TextBuffer,
}

/// The rows a group asks the layout for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rows {
    /// At most this many rows, fewer when the form runs out of room.
    Fixed(u16),
    /// This share of the body's rows.
    Percent(u16),
    /// Every row the other groups leave.
    Fill,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Group<F> {
    /// Wrapped text.
    Info(String),
    /// Wrapped text in the warning colour: a failure or a refusal.
    Warning(String),
    Radio(Radio<F>),
    Toggles(Vec<Toggle<F>>),
    LineInput(LineInput),
    /// A region the overlay draws itself (a list, a table, a calendar). The
    /// form gives it its rect in [`FormLayout::groups`] like any other group.
    Custom(Rows),
}

impl<F> Group<F> {
    /// Focus stops inside the group, top to bottom.
    fn stop_count(&self) -> usize {
        match self {
            Group::Radio(radio) if radio.is_focusable() => radio.choices.len(),
            Group::Toggles(toggles) => toggles.len(),
            Group::LineInput(_) => 1,
            _ => 0,
        }
    }
}

/// What a button does, which decides its icon and accent colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonRole {
    /// The action the form exists for. `Esc` never reaches it.
    Submit { is_destructive: bool },
    /// Another way out, between Submit and Cancel.
    Extra { is_destructive: bool },
    /// Closes the form with no side effect.
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button<F> {
    pub id: F,
    pub label: String,
    pub role: ButtonRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The `item`-th stop of group `group`.
    Item {
        group: usize,
        item: usize,
    },
    Button(usize),
}

/// Where the last draw put each part of the form, for mouse clicks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormLayout {
    pub area: Rect,
    /// One rect per group, in order, empty for a group left without room.
    pub groups: Vec<Rect>,
    pub items: Vec<(Focus, Rect)>,
    pub buttons: Vec<Rect>,
}

impl FormLayout {
    pub fn focus_at(&self, column: u16, row: u16) -> Option<Focus> {
        let position = ratatui::layout::Position::new(column, row);
        self.items
            .iter()
            .find(|(_, rect)| rect.contains(position))
            .map(|(focus, _)| *focus)
            .or_else(|| {
                self.buttons
                    .iter()
                    .position(|rect| rect.contains(position))
                    .map(Focus::Button)
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form<F> {
    pub groups: Vec<Group<F>>,
    pub buttons: Vec<Button<F>>,
    focus: Option<Focus>,
    /// Written by every draw.
    pub layout: Option<FormLayout>,
}

impl<F> Default for Form<F> {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new())
    }
}

impl<F> Form<F> {
    /// Focus starts on the first stop, a control before a button.
    pub fn new(groups: Vec<Group<F>>, buttons: Vec<Button<F>>) -> Self {
        let mut form = Self {
            groups,
            buttons,
            focus: None,
            layout: None,
        };
        let first_stop = form.stops().next();
        form.focus = first_stop;
        form
    }

    /// Swaps group `index`, keeping the focus wherever it still lands on a
    /// stop, so a group filled in late does not move the user.
    pub fn replace_group(&mut self, index: usize, group: Group<F>) {
        let Some(slot) = self.groups.get_mut(index) else {
            return;
        };
        *slot = group;
        let is_focus_kept = self
            .focus
            .is_some_and(|focus| self.stops().any(|stop| stop == focus));
        if !is_focus_kept {
            let first_stop = self.stops().next();
            self.focus = first_stop;
        }
    }

    pub fn focus(&self) -> Option<Focus> {
        self.focus
    }

    /// Ignores a stop the form does not have.
    pub fn set_focus(&mut self, focus: Focus) {
        if self.stops().any(|stop| stop == focus) {
            self.focus = Some(focus);
        }
    }

    pub fn focus_button_with(&mut self, role: ButtonRole) {
        if let Some(index) = self.buttons.iter().position(|b| b.role == role) {
            self.focus = Some(Focus::Button(index));
        }
    }

    pub fn focused_button(&self) -> Option<&Button<F>> {
        match self.focus? {
            Focus::Button(index) => self.buttons.get(index),
            Focus::Item { .. } => None,
        }
    }

    pub fn is_on_button(&self) -> bool {
        matches!(self.focus, Some(Focus::Button(_)))
    }

    /// Every focus stop: the groups' controls top to bottom, then the
    /// buttons left to right.
    pub fn stops(&self) -> impl Iterator<Item = Focus> + '_ {
        let items = self.groups.iter().enumerate().flat_map(|(group, g)| {
            (0..g.stop_count()).map(move |item| Focus::Item { group, item })
        });
        items.chain((0..self.buttons.len()).map(Focus::Button))
    }

    fn item_stops(&self) -> Vec<Focus> {
        self.stops()
            .filter(|stop| matches!(stop, Focus::Item { .. }))
            .collect()
    }

    /// `Tab` / `BackTab`: every stop in turn, stopping at either end.
    pub fn focus_next(&mut self, forward: bool) {
        let stops: Vec<Focus> = self.stops().collect();
        let Some(current) = self.focus.and_then(|f| stops.iter().position(|&s| s == f)) else {
            self.focus = stops.first().copied();
            return;
        };
        let target = if forward {
            (current + 1).min(stops.len() - 1)
        } else {
            current.saturating_sub(1)
        };
        self.focus = Some(stops[target]);
    }

    /// `j` / `k`: from control to control. Leaving the first or last one, or
    /// moving off the buttons, lands on Cancel or the first / last control.
    pub fn move_vertically(&mut self, down: bool) {
        let items = self.item_stops();
        if items.is_empty() {
            return;
        }
        let position = self.focus.and_then(|f| items.iter().position(|&s| s == f));
        let cancel = self
            .buttons
            .iter()
            .position(|b| b.role == ButtonRole::Cancel)
            .map(Focus::Button);
        self.focus = match (position, down) {
            (None, true) => items.first().copied(),
            (None, false) => items.last().copied(),
            (Some(i), true) if i + 1 < items.len() => Some(items[i + 1]),
            (Some(i), false) if i > 0 => Some(items[i - 1]),
            _ => cancel.or(self.focus),
        };
    }

    /// `h` / `l` on the button row, stopping at either end.
    pub fn move_between_buttons(&mut self, step: isize) {
        let Some(Focus::Button(index)) = self.focus else {
            return;
        };
        let last = self.buttons.len().saturating_sub(1);
        self.focus = Some(Focus::Button(index.saturating_add_signed(step).min(last)));
    }

    /// `Space`: selects the focused radio choice or flips the focused
    /// toggle. Returns whether a value changed.
    pub fn toggle_focused(&mut self) -> bool {
        let Some(Focus::Item { group, item }) = self.focus else {
            return false;
        };
        match self.groups.get_mut(group) {
            Some(Group::Radio(radio)) if item < radio.choices.len() => {
                let changed = radio.selected != item;
                radio.selected = item;
                changed
            }
            Some(Group::Toggles(toggles)) => match toggles.get_mut(item) {
                Some(toggle) => {
                    toggle.checked = !toggle.checked;
                    true
                }
                None => false,
            },
            _ => false,
        }
    }

    /// `Enter` and clicks: a focused button is pressed, a control toggled.
    pub fn activate(&mut self) -> Option<&F> {
        if self.is_on_button() {
            return self.focused_button().map(|button| &button.id);
        }
        self.toggle_focused();
        None
    }

    /// The selected choice of the first radio group.
    pub fn radio_value(&self) -> Option<&F> {
        self.groups.iter().find_map(|group| match group {
            Group::Radio(radio) => radio.selected_id(),
            _ => None,
        })
    }

    pub fn is_checked(&self, id: &F) -> bool
    where
        F: PartialEq,
    {
        self.groups.iter().any(|group| match group {
            Group::Toggles(toggles) => toggles.iter().any(|t| &t.id == id && t.checked),
            _ => false,
        })
    }

    pub fn line_input(&self) -> Option<&LineInput> {
        self.groups.iter().find_map(|group| match group {
            Group::LineInput(input) => Some(input),
            _ => None,
        })
    }

    pub fn line_input_mut(&mut self) -> Option<&mut LineInput> {
        self.groups.iter_mut().find_map(|group| match group {
            Group::LineInput(input) => Some(input),
            _ => None,
        })
    }

    /// The key footer for the focused control.
    pub fn key_hints(&self) -> &'static str {
        match self.focus {
            Some(Focus::Button(_)) if self.item_stops().is_empty() => {
                "h/l button · Enter select · Esc cancel"
            }
            Some(Focus::Button(_)) => "h/l button · j/k options · Enter select · Esc cancel",
            Some(Focus::Item { group, .. }) => match self.groups.get(group) {
                Some(Group::LineInput(_)) => "Type to edit · Enter submit · Esc cancel",
                Some(Group::Radio(_)) => "j/k move · Space pick · Tab next · Esc cancel",
                _ => "j/k move · Space toggle · Tab next · Esc cancel",
            },
            None => "Esc close",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Id {
        A,
        B,
        Flag,
        Submit,
        Cancel,
    }

    fn form() -> Form<Id> {
        Form::new(
            vec![
                Group::Info("body".to_string()),
                Group::Radio(Radio {
                    label: "Method".to_string(),
                    choices: vec![
                        Choice {
                            id: Id::A,
                            label: "A".to_string(),
                        },
                        Choice {
                            id: Id::B,
                            label: "B".to_string(),
                        },
                    ],
                    selected: 0,
                }),
                Group::Toggles(vec![Toggle {
                    id: Id::Flag,
                    label: "Flag".to_string(),
                    checked: false,
                }]),
            ],
            vec![
                Button {
                    id: Id::Submit,
                    label: "Go".to_string(),
                    role: ButtonRole::Submit {
                        is_destructive: false,
                    },
                },
                Button {
                    id: Id::Cancel,
                    label: "Cancel".to_string(),
                    role: ButtonRole::Cancel,
                },
            ],
        )
    }

    #[test]
    fn tab_walks_the_controls_then_the_buttons_and_stops_at_the_ends() {
        let mut form = form();
        let mut visited = vec![form.focus()];
        for _ in 0..5 {
            form.focus_next(true);
            visited.push(form.focus());
        }
        assert_eq!(
            visited,
            vec![
                Some(Focus::Item { group: 1, item: 0 }),
                Some(Focus::Item { group: 1, item: 1 }),
                Some(Focus::Item { group: 2, item: 0 }),
                Some(Focus::Button(0)),
                Some(Focus::Button(1)),
                Some(Focus::Button(1)),
            ]
        );
    }

    #[test]
    fn picking_a_radio_choice_deselects_the_other_and_reads_back_as_its_id() {
        let mut form = form();
        form.set_focus(Focus::Item { group: 1, item: 1 });
        assert!(form.toggle_focused());
        assert_eq!(form.radio_value(), Some(&Id::B));
        assert!(
            !form.toggle_focused(),
            "picking the picked choice changes nothing"
        );
    }

    #[test]
    fn a_radio_with_one_choice_takes_no_focus() {
        let form: Form<Id> = Form::new(
            vec![Group::Radio(Radio {
                label: String::new(),
                choices: vec![Choice {
                    id: Id::A,
                    label: "A".to_string(),
                }],
                selected: 0,
            })],
            vec![],
        );
        assert_eq!(form.stops().count(), 0);
        assert_eq!(form.radio_value(), Some(&Id::A));
    }

    #[test]
    fn leaving_the_controls_with_j_or_k_lands_on_cancel() {
        let mut form = form();
        form.move_vertically(false);
        assert_eq!(form.focused_button().map(|b| b.id), Some(Id::Cancel));
        form.move_vertically(true);
        assert_eq!(form.focus(), Some(Focus::Item { group: 1, item: 0 }));
        form.set_focus(Focus::Item { group: 2, item: 0 });
        form.move_vertically(true);
        assert_eq!(form.focused_button().map(|b| b.id), Some(Id::Cancel));
    }

    #[test]
    fn enter_presses_a_button_but_only_toggles_a_control() {
        let mut form = form();
        form.set_focus(Focus::Item { group: 2, item: 0 });
        assert_eq!(form.activate(), None);
        assert!(form.is_checked(&Id::Flag));
        form.focus_button_with(ButtonRole::Submit {
            is_destructive: false,
        });
        assert_eq!(form.activate(), Some(&Id::Submit));
    }

    #[test]
    fn a_focus_the_form_does_not_have_is_ignored() {
        let mut form = form();
        form.set_focus(Focus::Button(7));
        assert_eq!(form.focus(), Some(Focus::Item { group: 1, item: 0 }));
    }
}
