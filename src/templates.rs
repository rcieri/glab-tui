/// The template picker's entry that leaves the description empty.
pub const BLANK_TEMPLATE_CHOICE: &str = "-- (blank)";

/// Which description templates a template picker lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateKind {
    Issue,
    Mr,
}

impl TemplateKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Issue => "Issue",
            Self::Mr => "Merge Request",
        }
    }
}

pub fn list_templates(kind: TemplateKind) -> Vec<(String, String)> {
    let mut templates: Vec<(String, String)> = Vec::new();

    let dirs = match kind {
        TemplateKind::Issue => vec![".github/ISSUE_TEMPLATE", ".gitlab/issue_templates"],
        TemplateKind::Mr => vec![
            ".github/PULL_REQUEST_TEMPLATE",
            ".gitlab/merge_request_templates",
        ],
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
                    if !templates.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                        templates.push((name, content));
                    }
                }
            }
        }
    }

    // Only look for root-level template fallback if no directory templates exist
    if templates.is_empty() {
        let paths = match kind {
            TemplateKind::Issue => vec![
                ".github/issue_template.md",
                ".github/ISSUE_TEMPLATE.md",
                ".gitlab/issue_template.md",
            ],
            TemplateKind::Mr => vec![
                ".github/pull_request_template.md",
                ".github/PULL_REQUEST_TEMPLATE.md",
                ".gitlab/merge_request_template.md",
            ],
        };

        for path in &paths {
            if let Ok(content) = std::fs::read_to_string(path) {
                let name = std::path::Path::new(path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string());
                if !templates.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                    templates.push((name, content));
                }
            }
        }
    }

    templates
}

pub fn get_default_template(kind: TemplateKind) -> Option<String> {
    let templates = list_templates(kind);
    if let Some((_, content)) = templates
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("default"))
    {
        return Some(content.clone());
    }
    templates.into_iter().next().map(|(_, content)| content)
}
