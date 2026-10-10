#![allow(clippy::all)]
#![allow(unused_variables)]
#![allow(unused_assignments)]

pub mod app;
pub mod backend;
pub mod cli;
pub mod config;
pub mod custom_commands;
pub mod domain;
pub mod editor;
pub mod entity_editor;
pub mod event;
pub mod fetch;
pub mod git_helpers;
pub mod handlers;
pub mod keybinding;
pub mod scope;
pub mod templates;
pub mod ui;
pub mod utils;

pub type AppTerminal = ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>;

pub use editor::*;
pub use entity_editor::*;
pub use fetch::spawn_fetch_repo_attributes;
pub use fetch::{spawn_refresh_active_tab, spawn_refresh_all_tabs};
pub use git_helpers::*;
pub use keybinding::keybinding_matches;
pub use templates::*;
