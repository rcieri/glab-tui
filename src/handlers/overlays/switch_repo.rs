use crate::app::App;
use crate::keybinding::KeyPress;
use crossterm::event::KeyCode;
use ratatui::widgets::ListState;

pub fn handle_switch_repo(app: &mut App, key_event: &KeyPress) -> bool {
    let is_switch_repo = (key_event.code == KeyCode::Char('s')
        && key_event
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL))
        || (key_event.code == KeyCode::Char('S')
            && key_event
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL))
        || crate::keybinding::keybinding_matches(
            &app.config.keybindings.global.switch_repo,
            key_event,
        );

    if is_switch_repo
        && app.text_input.is_none()
        && app.edit_menu.is_none()
        && app.selector.is_none()
        && !app.is_typing_search
        && !app.job_trace_searching
        && !app.diff_view.as_ref().is_some_and(|d| d.search_active)
    {
        let mut items = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut switch_repo_paths = std::collections::HashMap::new();
        let mut switch_repo_groups = std::collections::HashSet::new();

        // Pin the current context to the top so the active group (or the
        // group implied by the current repo) stays visible even before the
        // recent-group cache has been warmed.
        let current_group = match &app.scope {
            crate::scope::Scope::Group(g) => Some(g.clone()),
            crate::scope::Scope::Repository(r) => r.rsplit_once('/').map(|(g, _)| g.to_string()),
        };
        if let Some(g) = current_group {
            if !g.trim().is_empty() && seen.insert(g.clone()) {
                switch_repo_groups.insert(g.clone());
                items.push(g);
            }
        }

        // Then every group the user can reach — recently switched groups
        // plus the group implied by each cached repo's remote. Newly
        // discovered groups are persisted so the next opener doesn't need
        // the git/auth probes again. Persist in one batched write instead
        // of one read-modify-write per new group.
        let recent_groups: std::collections::HashSet<String> =
            crate::utils::cache::get_recent_groups()
                .into_iter()
                .collect();
        let mut new_groups: Vec<String> = Vec::new();
        for g in crate::utils::cache::get_available_groups() {
            if g.trim().is_empty() || !seen.insert(g.clone()) {
                continue;
            }
            switch_repo_groups.insert(g.clone());
            items.push(g.clone());
            if !recent_groups.contains(&g) {
                new_groups.push(g);
            }
        }
        crate::utils::cache::add_recent_groups(&new_groups);

        // Finally the repositories. A repo that shares a display name with
        // an already-listed group yields to the group.
        for repo in crate::utils::cache::get_switchable_repos() {
            if !seen.insert(repo.display.clone()) {
                continue;
            }
            switch_repo_paths.insert(repo.display.clone(), repo.absolute_path);
            items.push(repo.display);
        }
        app.switch_repo_paths = switch_repo_paths;
        app.switch_repo_groups = switch_repo_groups;

        app.selector = Some(crate::app::Selector {
            title: " Switch Repository / Group ".to_string(),
            all_items: items,
            selected_items: {
                let mut s = std::collections::HashSet::new();
                match &app.scope {
                    crate::scope::Scope::Group(g) => {
                        s.insert(g.clone());
                    }
                    crate::scope::Scope::Repository(_) => {
                        if let Ok(cwd) = std::env::current_dir() {
                            if let Some(name) = cwd.file_name().and_then(|n| n.to_str()) {
                                s.insert(name.to_string());
                            }
                        }
                    }
                }
                s
            },
            cursor_idx: 0,
            search_query: String::new(),
            is_filtering: false,
            is_loading: false,
            entity_iid: 0,
            entity_type: "app".to_string(),
            field_type: "switch_repo".to_string(),
            multi_select: false,
            state: {
                let mut s = ListState::default();
                s.select(Some(0));
                s
            },
            request: None,
        });
        return true;
    }
    false
}
