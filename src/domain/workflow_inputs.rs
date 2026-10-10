use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct WorkflowInput {
    pub name: String,
    pub description: String,
    pub required: bool,
    pub default: Option<String>,
    pub input_type: WorkflowInputType,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowInputType {
    String,
    Choice,
    Boolean,
    Environment,
}

#[derive(Deserialize)]
struct WorkflowTop {
    on: Option<WorkflowOn>,
    #[serde(rename = "true")]
    true_on: Option<WorkflowOn>,
}

#[derive(Deserialize)]
struct WorkflowOn {
    workflow_dispatch: Option<WorkflowDispatch>,
}

#[derive(Deserialize)]
struct WorkflowDispatch {
    inputs: Option<HashMap<String, WorkflowInputDef>>,
}

#[derive(Deserialize)]
struct WorkflowInputDef {
    description: Option<String>,
    required: Option<bool>,
    default: Option<InputDefault>,
    #[serde(rename = "type")]
    input_type: Option<String>,
    options: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum InputDefault {
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Unsupported(serde::de::IgnoredAny),
}

impl InputDefault {
    fn into_text(self) -> String {
        match self {
            InputDefault::Bool(b) => b.to_string(),
            InputDefault::Integer(n) => n.to_string(),
            InputDefault::Float(n) => n.to_string(),
            InputDefault::Text(s) => s,
            InputDefault::Unsupported(_) => String::new(),
        }
    }
}

pub fn parse_workflow_inputs(yaml_path: &str) -> Option<Vec<WorkflowInput>> {
    let content = std::fs::read_to_string(yaml_path).ok()?;
    let wf: WorkflowTop = serde_saphyr::from_str(&content)
        .inspect_err(|e| eprintln!("workflow YAML parse error for {}: {}", yaml_path, e))
        .ok()?;
    let on = wf.on.or(wf.true_on)?;
    let inputs = on.workflow_dispatch?.inputs?;

    let result: Vec<WorkflowInput> = inputs
        .into_iter()
        .map(|(name, def)| {
            let description = def.description.unwrap_or_default();
            let required = def.required.unwrap_or(false);
            let default = def.default.map(InputDefault::into_text);
            let input_type = match def.input_type.as_deref() {
                Some("choice") => WorkflowInputType::Choice,
                Some("boolean") => WorkflowInputType::Boolean,
                Some("environment") => WorkflowInputType::Environment,
                _ => WorkflowInputType::String,
            };
            let options = def.options.unwrap_or_default();

            WorkflowInput {
                name,
                description,
                required,
                default,
                input_type,
                options,
            }
        })
        .collect();

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `workflow_dispatch` block with a single choice input, in the shape
    /// GitHub emits. Kept inline so the parser's tests do not depend on which
    /// workflow files happen to exist in this repository.
    const CHOICE_WORKFLOW: &str = r#"
name: Prepare Release
on:
  workflow_dispatch:
    inputs:
      version_increment:
        description: "Version increment"
        required: true
        default: patch
        type: choice
        options:
          - patch
          - minor
          - major
"#;

    fn write_workflow(contents: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workflow.yml");
        std::fs::write(&path, contents).unwrap();
        let path_str = path.to_str().unwrap().to_string();
        (dir, path_str)
    }

    #[test]
    fn parses_a_choice_input_with_all_its_fields() {
        let (_dir, path) = write_workflow(CHOICE_WORKFLOW);
        let inputs = parse_workflow_inputs(&path).expect("workflow_dispatch inputs should parse");
        assert_eq!(inputs.len(), 1, "expected 1 input, got {}", inputs.len());

        let version = &inputs[0];
        assert_eq!(version.name, "version_increment");
        assert_eq!(version.description, "Version increment");
        assert!(version.required);
        assert_eq!(version.default, Some("patch".to_string()));
        assert_eq!(version.input_type, WorkflowInputType::Choice);
        assert_eq!(version.options, vec!["patch", "minor", "major"]);
    }

    #[test]
    fn untyped_input_defaults_to_string_and_is_optional() {
        let (_dir, path) = write_workflow(
            r#"
on:
  workflow_dispatch:
    inputs:
      tag:
        description: "Tag to build"
"#,
        );
        let inputs = parse_workflow_inputs(&path).expect("inputs should parse");
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].input_type, WorkflowInputType::String);
        assert!(!inputs[0].required);
        assert_eq!(inputs[0].default, None);
        assert!(inputs[0].options.is_empty());
    }

    #[test]
    fn non_string_defaults_are_rendered_as_text_and_jobs_are_ignored() {
        let (_dir, path) = write_workflow(
            r#"
on:
  workflow_dispatch:
    inputs:
      dry_run:
        type: boolean
        default: true
      retries:
        default: 3
      ratio:
        default: 0.5
      targets:
        default: [a, b]
jobs:
  build:
    runs-on: ubuntu-latest
    steps: &steps
      - uses: actions/checkout@v4
      - run: echo "${{ inputs.dry_run }}"
  again:
    steps: *steps
"#,
        );
        let inputs = parse_workflow_inputs(&path).expect("inputs should parse");
        let mut defaults: Vec<(String, Option<String>)> = inputs
            .into_iter()
            .map(|input| (input.name, input.default))
            .collect();
        defaults.sort();
        assert_eq!(
            defaults,
            vec![
                ("dry_run".to_string(), Some("true".to_string())),
                ("ratio".to_string(), Some("0.5".to_string())),
                ("retries".to_string(), Some("3".to_string())),
                ("targets".to_string(), Some(String::new())),
            ]
        );
    }

    #[test]
    fn workflow_without_dispatch_inputs_yields_none() {
        let (_dir, path) = write_workflow("name: CI\non:\n  push:\n    branches: [main]\n");
        assert!(parse_workflow_inputs(&path).is_none());
    }

    #[test]
    fn missing_file_yields_none() {
        let (dir, _) = write_workflow("");
        let absent = dir.path().join("does-not-exist.yml");
        assert!(parse_workflow_inputs(absent.to_str().unwrap()).is_none());
    }
}
