use ratatui::style::Color;
use serde::Deserialize;
use std::{path::PathBuf, sync::LazyLock};

fn hex_to_color(s: &str) -> Option<Color> {
    let s = s.trim_start_matches('#');
    if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Color::Rgb(r, g, b))
    } else {
        None
    }
}

fn apply_color(field: &mut Color, override_val: &Option<String>) {
    if let Some(s) = override_val {
        if let Some(c) = hex_to_color(s) {
            *field = c;
        }
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color,
    pub border: Color,
    pub border_focused: Color,
    pub header_fg: Color,
    pub highlight_bg: Color,
    pub inactive_bg: Color,
    pub text_normal: Color,
    pub text_muted: Color,
    pub checked_bg: Color,
    pub green: Color,
    pub green_bg: Color,
    pub red: Color,
    pub red_bg: Color,
    pub blue: Color,
    pub blue_bg: Color,
    pub yellow: Color,
    pub yellow_bg: Color,
    pub purple: Color,
    pub purple_bg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            bg: Color::Rgb(18, 18, 20),
            border: Color::Rgb(80, 80, 88),
            border_focused: Color::Rgb(49, 191, 103),
            header_fg: Color::Rgb(49, 191, 103),
            highlight_bg: Color::Rgb(43, 43, 57),
            inactive_bg: Color::Rgb(49, 50, 68),
            text_normal: Color::Rgb(216, 222, 233),
            text_muted: Color::Rgb(130, 130, 138),
            checked_bg: Color::Rgb(28, 38, 55),
            green: Color::Rgb(49, 191, 103),
            green_bg: Color::Rgb(20, 45, 28),
            red: Color::Rgb(224, 73, 83),
            red_bg: Color::Rgb(50, 20, 25),
            blue: Color::Rgb(61, 139, 255),
            blue_bg: Color::Rgb(15, 35, 60),
            yellow: Color::Rgb(235, 180, 50),
            yellow_bg: Color::Rgb(45, 35, 15),
            purple: Color::Rgb(168, 122, 243),
            purple_bg: Color::Rgb(38, 25, 55),
        }
    }
}

impl Theme {
    fn apply_raw(&mut self, raw: &RawTheme) {
        apply_color(&mut self.bg, &raw.bg);
        apply_color(&mut self.border, &raw.border);
        apply_color(&mut self.border_focused, &raw.border_focused);
        apply_color(&mut self.header_fg, &raw.header_fg);
        apply_color(&mut self.highlight_bg, &raw.highlight_bg);
        apply_color(&mut self.inactive_bg, &raw.inactive_bg);
        apply_color(&mut self.text_normal, &raw.text_normal);
        apply_color(&mut self.text_muted, &raw.text_muted);
        apply_color(&mut self.checked_bg, &raw.checked_bg);
        apply_color(&mut self.green, &raw.green);
        apply_color(&mut self.green_bg, &raw.green_bg);
        apply_color(&mut self.red, &raw.red);
        apply_color(&mut self.red_bg, &raw.red_bg);
        apply_color(&mut self.blue, &raw.blue);
        apply_color(&mut self.blue_bg, &raw.blue_bg);
        apply_color(&mut self.yellow, &raw.yellow);
        apply_color(&mut self.yellow_bg, &raw.yellow_bg);
        apply_color(&mut self.purple, &raw.purple);
        apply_color(&mut self.purple_bg, &raw.purple_bg);
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct RawTheme {
    bg: Option<String>,
    border: Option<String>,
    border_focused: Option<String>,
    header_fg: Option<String>,
    highlight_bg: Option<String>,
    inactive_bg: Option<String>,
    text_normal: Option<String>,
    text_muted: Option<String>,
    checked_bg: Option<String>,
    green: Option<String>,
    green_bg: Option<String>,
    red: Option<String>,
    red_bg: Option<String>,
    blue: Option<String>,
    blue_bg: Option<String>,
    yellow: Option<String>,
    yellow_bg: Option<String>,
    purple: Option<String>,
    purple_bg: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FormattingConfig {
    pub date_format: Option<String>,
    pub release_name_trunc: Option<usize>,
    pub release_tag_width: Option<u16>,
    pub release_date_width: Option<u16>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct KeybindingConfig {
    pub create_issue: Option<String>,
    pub create_mr: Option<String>,
    pub create_release: Option<String>,
    pub create_milestone: Option<String>,
    pub refresh: Option<String>,
    pub help: Option<String>,
    pub quit: Option<String>,
    pub search: Option<String>,
    pub configure: Option<String>,
    pub next_tab: Option<String>,
    pub prev_tab: Option<String>,
    pub edit_entity: Option<String>,
    pub close_entity: Option<String>,
    pub open_in_browser: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct RawConfig {
    theme: RawTheme,
    formatting: FormattingConfig,
    keybindings: KeybindingConfig,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub theme: Theme,
    pub formatting: FormattingConfig,
    pub keybindings: KeybindingConfig,
}

impl Config {
    fn load_from_path(path: &std::path::Path) -> Self {
        let raw: RawConfig = match std::fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => RawConfig::default(),
        };

        let mut theme = Theme::default();
        theme.apply_raw(&raw.theme);

        Self {
            theme,
            formatting: raw.formatting,
            keybindings: raw.keybindings,
        }
    }

    fn config_path() -> PathBuf {
        if let Ok(path) = std::env::var("GLAB_TUI_CONFIG") {
            return PathBuf::from(path);
        }

        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());

        let xdg_config = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let mut p = PathBuf::from(&home);
                p.push(".config");
                p
            });

        let mut path = xdg_config;
        path.push("glab-tui");
        path.push("config.json");
        path
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        Self::load_from_path(&path)
    }
}

pub static THEME: LazyLock<Theme> = LazyLock::new(|| Config::load().theme);
