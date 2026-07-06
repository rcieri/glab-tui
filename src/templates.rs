pub fn list_templates(template_type: &str) -> Vec<(String, String)> {
    let mut templates: Vec<(String, String)> = Vec::new();

    let paths = if template_type == "issue" {
        vec![
            ".github/issue_template.md",
            ".github/ISSUE_TEMPLATE.md",
            ".gitlab/issue_template.md",
        ]
    } else {
        vec![
            ".github/pull_request_template.md",
            ".github/PULL_REQUEST_TEMPLATE.md",
            ".gitlab/merge_request_template.md",
        ]
    };

    for path in &paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            let name = std::path::Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string());
            if !templates.iter().any(|(n, _)| n == &name) {
                templates.push((name, content));
            }
        }
    }

    let dirs = if template_type == "issue" {
        vec![".github/ISSUE_TEMPLATE", ".gitlab/issue_templates"]
    } else {
        vec![
            ".github/PULL_REQUEST_TEMPLATE",
            ".gitlab/merge_request_templates",
        ]
    };

    for dir in &dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut md_files = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().map(|ext| ext == "md").unwrap_or(false) {
                    md_files.push(path);
                }
            }
            md_files.sort();
            for file_path in &md_files {
                if let Ok(content) = std::fs::read_to_string(file_path) {
                    let name = file_path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if !templates.iter().any(|(n, _)| n == &name) {
                        templates.push((name, content));
                    }
                }
            }
        }
    }

    templates
}

pub fn get_default_template(template_type: &str) -> Option<String> {
    let templates = list_templates(template_type);
    if let Some((_, content)) = templates.iter().find(|(n, _)| n == "default") {
        return Some(content.clone());
    }
    templates.into_iter().next().map(|(_, content)| content)
}
