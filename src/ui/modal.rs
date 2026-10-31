use crate::config::Theme;
use crate::ui::render_ctx::RenderCtx;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
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

/// Clear and render a modal frame over `area`. Returns (inner_area, outer_area) for body content.
pub(crate) fn modal_area(
    f: &mut Frame,
    ctx: &RenderCtx<'_>,
    title: &str,
    area: Rect,
) -> (Rect, Rect) {
    clear_area(f, ctx, area);
    let block = modal_block(title, &ctx.theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    (inner, area)
}
