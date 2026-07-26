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
    default: Option<serde_yaml::Value>,
    #[serde(rename = "type")]
    input_type: Option<String>,
    options: Option<Vec<String>>,
}

pub fn parse_workflow_inputs(yaml_path: &str) -> Option<Vec<WorkflowInput>> {
    let content = std::fs::read_to_string(yaml_path).ok()?;
    let wf: WorkflowTop = serde_yaml::from_str(&content).ok()?;
    let on = wf.on.or(wf.true_on)?;
    let inputs = on.workflow_dispatch?.inputs?;

    let result: Vec<WorkflowInput> = inputs
        .into_iter()
        .map(|(name, def)| {
            let description = def.description.unwrap_or_default();
            let required = def.required.unwrap_or(false);
            let default = def.default.map(|v| match v {
                serde_yaml::Value::String(s) => s,
                serde_yaml::Value::Bool(b) => b.to_string(),
                serde_yaml::Value::Number(n) => n.to_string(),
                _ => String::new(),
            });
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
