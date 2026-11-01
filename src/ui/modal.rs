use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, Clear},
};

use super::diff::centered_rect_min;
use crate::config::{THEME, Theme};

/// Create a standard modal block with double border.
pub(crate) fn modal_block(title: &str) -> Block<'static> {
    let theme = THEME.read().unwrap();
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(theme.modal_border))
        .title(format!(" {title} "))
        .title_style(
            Style::default()
                .fg(theme.modal_border)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(theme.bg))
}

/// Clear an overlay area, repainting it with the theme background so overlays keep the
/// theme bg instead of falling back to the terminal default background.
pub(crate) fn clear_area(f: &mut Frame, area: Rect) {
    f.render_widget(Clear, area);
    f.render_widget(
        Block::default().style(Style::default().bg(THEME.read().unwrap().bg)),
        area,
    );
}

/// Style of a dialog button: filled with `accent` (or `border_focused` when it
/// has none) while focused, written in it otherwise.
pub(crate) fn button_style(theme: &Theme, is_focused: bool, accent: Option<Color>) -> Style {
    let (fg, bg) = match (is_focused, accent) {
        (true, Some(accent)) => (theme.bg, accent),
        (false, Some(accent)) => (accent, theme.bg),
        (true, None) => (theme.bg, theme.border_focused),
        (false, None) => (theme.text_normal, theme.bg),
    };
    let style = Style::default().fg(fg).bg(bg);
    if is_focused {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

/// Clear, size, and render a modal frame. Returns (inner_area, outer_area) for body content.
pub(crate) fn modal_area(
    f: &mut Frame,
    title: &str,
    percent_x: u16,
    percent_y: u16,
    min_w: u16,
    min_h: u16,
    size: Rect,
) -> (Rect, Rect) {
    let area = centered_rect_min(percent_x, percent_y, min_w, min_h, size);
    clear_area(f, area);
    let block = modal_block(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    (inner, area)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_focused_button_is_filled_and_bold_and_an_unfocused_one_is_written_in_its_accent() {
        let theme = Theme::default();
        let accent = theme.green;

        assert_eq!(
            button_style(&theme, true, Some(accent)),
            Style::default()
                .fg(theme.bg)
                .bg(accent)
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(
            button_style(&theme, false, Some(accent)),
            Style::default().fg(accent).bg(theme.bg)
        );
        assert_eq!(
            button_style(&theme, true, None),
            Style::default()
                .fg(theme.bg)
                .bg(theme.border_focused)
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(
            button_style(&theme, false, None),
            Style::default().fg(theme.text_normal).bg(theme.bg)
        );
    }
}
