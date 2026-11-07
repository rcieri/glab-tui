use crate::config::Theme;
use crate::ui::render_ctx::RenderCtx;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, Clear},
};

/// Create a standard modal block with double border.
pub(crate) fn modal_block(title: &str, theme: &Theme) -> Block<'static> {
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
pub(crate) fn clear_area(f: &mut Frame, ctx: &RenderCtx<'_>, area: Rect) {
    f.render_widget(Clear, area);
    f.render_widget(
        Block::default().style(Style::default().bg(ctx.theme.bg)),
        area,
    );
}

/// Style of a dialog button, filled with its accent (or `border_focused`
/// without one) while focused.
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

/// Clear `area` and draw a modal frame over it. Returns the area inside the border.
pub(crate) fn modal_area(f: &mut Frame, ctx: &RenderCtx<'_>, title: &str, area: Rect) -> Rect {
    clear_area(f, ctx, area);
    let block = modal_block(title, &ctx.theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
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
