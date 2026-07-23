use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, Clear},
};

use super::diff::centered_rect_min;
use crate::config::THEME;

/// Create a standard modal block with double border.
pub(crate) fn modal_block(title: &str) -> Block<'static> {
    let theme = THEME.read().unwrap();
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(theme.modal_border))
        .title(format!(" {} ", title))
        .title_style(
            Style::default()
                .fg(theme.modal_border)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(Color::Reset))
}

/// Clear, size, and render a modal frame. Returns the inner area for body content.
pub(crate) fn modal_area(
    f: &mut Frame,
    title: &str,
    percent_x: u16,
    percent_y: u16,
    min_w: u16,
    min_h: u16,
    size: Rect,
) -> Rect {
    let area = centered_rect_min(percent_x, percent_y, min_w, min_h, size);
    f.render_widget(Clear, area);
    let block = modal_block(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}
