use crate::app::App;
use crate::backend::BackendKind;
use crate::config::{Icons, Theme};
use ratatui::style::{Modifier, Style};

/// What every render function draws with, resolved once per frame from `App`
/// so no widget reads configuration on its own.
#[derive(Clone, Copy)]
pub(crate) struct RenderCtx<'a> {
    pub(crate) theme: Theme,
    pub(crate) icons: &'a Icons,
    pub(crate) kind: BackendKind,
}

impl<'a> RenderCtx<'a> {
    /// `icons` is passed in rather than borrowed from `app` so the context can
    /// live alongside the `&mut App` the render functions take.
    pub(crate) fn new(app: &App, icons: &'a Icons) -> Self {
        Self {
            theme: app.theme,
            icons,
            kind: app.kind(),
        }
    }

    pub(crate) fn highlight_style(&self) -> Style {
        Style::default().bg(self.theme.highlight_bg)
    }

    pub(crate) fn header_style(&self) -> Style {
        Style::default()
            .fg(self.theme.text_normal)
            .add_modifier(Modifier::BOLD)
    }
}

#[cfg(test)]
impl RenderCtx<'static> {
    /// The bundled theme and Nerd Font icons, for tests that render without an
    /// `App`.
    pub(crate) fn for_tests() -> Self {
        static ICONS: std::sync::LazyLock<Icons> = std::sync::LazyLock::new(Icons::nerd);
        Self {
            theme: Theme::default(),
            icons: &ICONS,
            kind: BackendKind::GitLab,
        }
    }
}
