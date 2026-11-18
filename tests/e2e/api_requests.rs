//! Checks the raw `gh api` / `glab api` calls the CLI mocks record against
//! the pinned API descriptions in `tests/api-descriptions/`: the endpoint
//! must exist, the method must be one the description gives it, every
//! query parameter must be one the operation takes, and the request body must
//! fit the operation's: no field the description lacks where it lists the
//! fields, and every value of a type it allows. The body is what the call read
//! with `--input -`, else its `-f` / `-F` fields when it is not a GET.
//! `ApiRequest::read` reads each call once and hands a GraphQL call to
//! `api_graphql`.
//!
//! What the description cannot confirm is excused through the shared
//! `Ledger`, by the keys `<host> <METHOD> <template>` (an operation the
//! description lacks, the ones the generator inputs mark `spec_absent_ok`) and
//! `<host> <METHOD> <template> query <name>` (a query parameter it does not
//! list) and `<host> <METHOD> <template> body <field>` (a body field it
//! lacks or types otherwise, `comments[].path` for a field of an array's
//! items; `body` alone for a body the operation does not take). All are
//! looked up as finding first, then as exception.

use crate::api_graphql;
use crate::api_lists::{Kind, Ledger};
use crate::cli_contract::recorded_calls;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::LazyLock;

/// Templates whose last parameter takes the rest of the path, slashes
/// included: the `{ref}` of GitHub's `git/refs/{ref}` is `heads/main`.
const REST_OF_PATH_TEMPLATES: [&str; 1] = ["/repos/{owner}/{repo}/git/refs/{ref}"];

/// Path parameters that hold a name the user chose, which may contain a slash
/// the call has to URL-encode: a path that fits a template only when such a
/// parameter holds several segments is excused by an `unencoded slash` entry.
const NAME_PARAMETERS: [&str; 2] = ["branch", "ref"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Host {
    GitHub,
    GitLab,
}

impl Host {
    pub fn name(self) -> &'static str {
        match self {
            Host::GitHub => "github",
            Host::GitLab => "gitlab",
        }
    }

    /// The flags of `api` that take a value, in both spellings.
    fn flags_with_value(self) -> &'static [&'static str] {
        match self {
            Host::GitHub => &[
                "--cache",
                "-F",
                "--field",
                "-H",
                "--header",
                "--hostname",
                "--input",
                "-q",
                "--jq",
                "-X",
                "--method",
                "-p",
                "--preview",
                "-f",
                "--raw-field",
                "-t",
                "--template",
            ],
            Host::GitLab => &[
                "-F",
                "--field",
                "--form",
                "-H",
                "--header",
                "--hostname",
                "--input",
                "-X",
                "--method",
                "--output",
                "--placeholder",
                "-f",
                "--raw-field",
            ],
        }
    }

    /// The path prefix the description's paths carry.
    fn path_prefix(self) -> &'static str {
        match self {
            Host::GitHub => "/",
            Host::GitLab => "/api/v4/",
        }
    }
}

/// What the description says about one (path template, method).
enum Operation {
    Described {
        query: BTreeSet<String>,
        /// The schema of the JSON request body, when the operation takes one.
        body: Option<Value>,
    },
    /// The generator inputs mark it `spec_absent_ok`: it exists, the
    /// description lacks it.
    Missing,
}

struct Description {
    /// Path template, then upper-case method.
    paths: BTreeMap<String, BTreeMap<String, Operation>>,
}

impl Description {
    fn parse(openapi: &str, generator_input: &str) -> Description {
        let openapi: Value = serde_json::from_str(openapi).expect("description is JSON");
        let mut paths: BTreeMap<String, BTreeMap<String, Operation>> = BTreeMap::new();
        for (template, methods) in openapi["paths"].as_object().into_iter().flatten() {
            for (method, operation) in methods.as_object().into_iter().flatten() {
                let query = operation["parameters"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|parameter| parameter["in"] == "query")
                    .filter_map(|parameter| parameter["name"].as_str())
                    .map(str::to_string)
                    .collect();
                let body = operation
                    .pointer("/requestBody/content/application~1json/schema")
                    .cloned();
                paths
                    .entry(template.clone())
                    .or_default()
                    .insert(method.to_uppercase(), Operation::Described { query, body });
            }
        }
        let input: Value = serde_json::from_str(generator_input).expect("input is JSON");
        for operation in input["operations"].as_array().into_iter().flatten() {
            if operation.get("spec_absent_ok").is_none() {
                continue;
            }
            let method = operation["method"].as_str().unwrap_or_default();
            let template = operation["path"].as_str().unwrap_or_default();
            paths
                .entry(template.to_string())
                .or_default()
                .entry(method.to_uppercase())
                .or_insert(Operation::Missing);
        }
        Description { paths }
    }

    /// The templates `path` fits, the one with the most literal segments
    /// first. With `name_takes_rest`, a template's last placeholder may also
    /// hold several segments when it is one of `NAME_PARAMETERS`.
    fn templates_for(&self, path: &str, name_takes_rest: bool) -> Vec<&str> {
        let segments: Vec<&str> = path.split('/').collect();
        let mut fits: Vec<(usize, &str)> = self
            .paths
            .keys()
            .filter_map(|template| {
                literal_segments_matched(template, &segments, name_takes_rest)
                    .map(|n| (n, template.as_str()))
            })
            .collect();
        fits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
        fits.into_iter().map(|(_, template)| template).collect()
    }
}

/// How many literal segments of `template` equal those of `path`, or `None`
/// when `path` does not fit it.
fn literal_segments_matched(template: &str, path: &[&str], name_takes_rest: bool) -> Option<usize> {
    let segments: Vec<&str> = template.split('/').collect();
    let placeholder = |segment: &str| segment.starts_with('{') && segment.ends_with('}');
    let takes_rest = segments
        .last()
        .filter(|last| placeholder(last))
        .map(|last| &last[1..last.len() - 1])
        .is_some_and(|name| {
            REST_OF_PATH_TEMPLATES.contains(&template)
                || (name_takes_rest && NAME_PARAMETERS.contains(&name))
        });
    let fits_length = if takes_rest {
        path.len() >= segments.len()
    } else {
        path.len() == segments.len()
    };
    if !fits_length {
        return None;
    }
    let mut literals = 0;
    for (wanted, given) in segments.iter().zip(path) {
        if placeholder(wanted) {
            if given.is_empty() {
                return None;
            }
        } else if wanted == given {
            literals += 1;
        } else {
            return None;
        }
    }
    Some(literals)
}

static GITHUB: LazyLock<Description> = LazyLock::new(|| {
    Description::parse(
        include_str!("../api-descriptions/github.openapi.json"),
        include_str!("../../scripts/api-descriptions/github.json"),
    )
});

static GITLAB: LazyLock<Description> = LazyLock::new(|| {
    Description::parse(
        include_str!("../api-descriptions/gitlab.openapi.json"),
        include_str!("../../scripts/api-descriptions/gitlab.json"),
    )
});

/// A recorded `gh api` / `glab api` call, read once for the REST and the
/// GraphQL check.
pub enum ApiRequest<'a> {
    Rest(ApiCall),
    /// `query` is the value of the call's `query=` field, `None` when it
    /// sends none.
    Graphql {
        host: Host,
        query: Option<&'a str>,
    },
}

/// A REST call of `api` as the description sees it.
pub struct ApiCall {
    host: Host,
    method: String,
    /// The endpoint's path, with the host's prefix and without its query.
    path: String,
    query: Vec<String>,
    /// The `-f` / `-F` fields of a call that is not a GET, as JSON values.
    fields: Vec<(String, Value)>,
    reads_body_from_input: bool,
}

impl<'a> ApiRequest<'a> {
    /// `None` for anything but a `gh api` / `glab api` call.
    pub fn read(argv: &[&'a str]) -> Option<ApiRequest<'a>> {
        let host = match argv {
            ["gh", "api", ..] => Host::GitHub,
            ["glab", "api", ..] => Host::GitLab,
            _ => return None,
        };
        let mut method = None;
        let mut fields = Vec::new();
        let mut graphql_query = None;
        let mut reads_body_from_input = false;
        let mut endpoint = None;
        let mut words = argv[2..].iter().copied();
        while let Some(word) = words.next() {
            let (flag, inline) = match word.split_once('=') {
                Some((flag, value)) if word.starts_with("--") => (flag, Some(value)),
                _ => (word, None),
            };
            let short_with_value = word
                .get(..2)
                .filter(|_| !word.starts_with("--") && word.len() > 2)
                .filter(|short| host.flags_with_value().contains(short));
            let (flag, inline) = match short_with_value {
                Some(short) => (short, Some(&word[2..])),
                None => (flag, inline),
            };
            if !flag.starts_with('-') {
                endpoint.get_or_insert(word);
                continue;
            }
            if !host.flags_with_value().contains(&flag) {
                continue;
            }
            let Some(value) = inline.or_else(|| words.next()) else {
                continue;
            };
            match flag {
                "-X" | "--method" => method = Some(value.to_uppercase()),
                "-f" | "--raw-field" => {
                    let (name, value) = value.split_once('=').unwrap_or((value, ""));
                    if name == "query" {
                        graphql_query.get_or_insert(value);
                    }
                    fields.push((name.to_string(), Value::String(value.to_string())));
                }
                "-F" | "--field" | "--form" => {
                    let (name, value) = value.split_once('=').unwrap_or((value, ""));
                    if name == "query" {
                        graphql_query.get_or_insert(value);
                    }
                    fields.push((name.to_string(), typed_field(value)));
                }
                "--input" => reads_body_from_input = true,
                _ => {}
            }
        }
        let endpoint = endpoint?;
        if endpoint == "graphql" {
            return Some(ApiRequest::Graphql {
                host,
                query: graphql_query,
            });
        }
        let has_body = reads_body_from_input || !fields.is_empty();
        let method = method.unwrap_or_else(|| if has_body { "POST" } else { "GET" }.to_string());
        let (path, query) = endpoint.split_once('?').unwrap_or((endpoint, ""));
        let mut query: Vec<String> = query
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                pair.split('=')
                    .next()
                    .unwrap_or_default()
                    .replace("%5B%5D", "[]")
            })
            .collect();
        if method == "GET" {
            query.extend(fields.drain(..).map(|(name, _)| name));
        }
        Some(ApiRequest::Rest(ApiCall {
            host,
            method,
            path: format!("{}{}", host.path_prefix(), path.trim_start_matches('/')),
            query,
            fields,
            reads_body_from_input,
        }))
    }
}

impl ApiCall {
    /// The request body: what `stdin` holds when the call reads `--input`,
    /// else its fields. `None` when it sends none, or reads a file the
    /// mocks do not record.
    fn body(&self, stdin: Option<&str>) -> Option<Result<Value, String>> {
        if self.reads_body_from_input {
            return stdin.map(|body| serde_json::from_str(body).map_err(|e| e.to_string()));
        }
        if self.fields.is_empty() {
            return None;
        }
        Some(Ok(Value::Object(self.fields.iter().cloned().collect())))
    }
}

/// A `-F` value as `gh api` and `glab api` send it: `true`, `false`, `null`
/// and integers as JSON literals, anything else as a string.
fn typed_field(value: &str) -> Value {
    match value {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        _ => value
            .parse::<i64>()
            .map_or_else(|_| Value::String(value.to_string()), Value::from),
    }
}

/// Where a body value does not fit its schema.
enum Mismatch {
    /// A field the schema does not list, on a level that lists its fields.
    Unknown { path: String },
    /// A value of a type the schema does not allow.
    Type {
        path: String,
        sent: &'static str,
        allowed: String,
    },
}

/// The mismatches of `value` against `schema`, the value found at `path`.
/// Of several branches, the one with the fewest mismatches is reported.
fn mismatches(schema: &Value, value: &Value, path: &str, out: &mut Vec<Mismatch>) {
    for key in ["oneOf", "anyOf"] {
        let Some(branches) = schema[key].as_array() else {
            continue;
        };
        let mut closest: Option<Vec<Mismatch>> = None;
        for branch in branches {
            let mut found = Vec::new();
            mismatches(branch, value, path, &mut found);
            if found.is_empty() {
                return;
            }
            if closest.as_ref().is_none_or(|best| found.len() < best.len()) {
                closest = Some(found);
            }
        }
        out.extend(closest.unwrap_or_default());
        return;
    }
    let allowed: Vec<&str> = match &schema["type"] {
        Value::String(one) => vec![one.as_str()],
        Value::Array(many) => many.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    if !allowed.is_empty() && !allowed.iter().any(|kind| is_of_type(value, kind)) {
        out.push(Mismatch::Type {
            path: path.to_string(),
            sent: type_of(value),
            allowed: allowed.join(" or "),
        });
        return;
    }
    match value {
        Value::Object(fields) => {
            let listed = schema["properties"].as_object();
            let extra = &schema["additionalProperties"];
            for (name, field) in fields {
                let field_path = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}.{name}")
                };
                match listed.and_then(|listed| listed.get(name)) {
                    Some(field_schema) => mismatches(field_schema, field, &field_path, out),
                    None if extra.is_object() => mismatches(extra, field, &field_path, out),
                    None if listed.is_some() && extra != &Value::Bool(true) => {
                        out.push(Mismatch::Unknown { path: field_path })
                    }
                    None => {}
                }
            }
        }
        Value::Array(items) if schema["items"].is_object() => {
            for item in items {
                mismatches(&schema["items"], item, &format!("{path}[]"), out);
            }
        }
        _ => {}
    }
}

fn is_of_type(value: &Value, kind: &str) -> bool {
    match kind {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => true,
    }
}

/// The type of `value`, worded for a message.
fn type_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "an integer",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// What is wrong with the body a call sent to the operation `operation_key`,
/// whose body schema is `schema`, one message each.
fn body_problems(
    operation_key: &str,
    schema: Option<&Value>,
    body: Option<Result<Value, String>>,
    ledger: &Ledger,
) -> Vec<String> {
    let Some(body) = body else {
        return Vec::new();
    };
    let (host, operation) = operation_key.split_once(' ').unwrap_or_default();
    let Some(schema) = schema else {
        let key = format!("{operation_key} body");
        if excused(ledger, &key) {
            return Vec::new();
        }
        return vec![format!(
            "{host}: {operation} takes no request body (list it as `{key}`)"
        )];
    };
    let body = match body {
        Ok(body) => body,
        Err(error) => {
            return vec![format!(
                "{host}: {operation}: the body is not JSON: {error}"
            )];
        }
    };
    let mut found = Vec::new();
    mismatches(schema, &body, "", &mut found);
    found
        .into_iter()
        .filter_map(|mismatch| {
            let (path, problem) = match mismatch {
                Mismatch::Unknown { path } => (path, "is not in the description".to_string()),
                Mismatch::Type {
                    path,
                    sent,
                    allowed,
                } => (path, format!("is {sent}, the description says {allowed}")),
            };
            let (subject, key) = if path.is_empty() {
                ("the body".to_string(), format!("{operation_key} body"))
            } else {
                (
                    format!("the body field `{path}`"),
                    format!("{operation_key} body {path}"),
                )
            };
            (!excused(ledger, &key))
                .then(|| format!("{host}: {operation}: {subject} {problem} (list it as `{key}`)"))
        })
        .collect()
}

pub fn excused(ledger: &Ledger, key: &str) -> bool {
    [Kind::Finding, Kind::Exception]
        .into_iter()
        .any(|kind| ledger.excuse(kind, key).is_some())
}

/// What is wrong with the call `argv`, which read `stdin` as its body, one
/// message each.
fn check(argv: &[&str], stdin: Option<&str>, ledger: &Ledger) -> Vec<String> {
    match ApiRequest::read(argv) {
        Some(ApiRequest::Rest(call)) => rest_problems(&call, stdin, ledger),
        Some(ApiRequest::Graphql { host, query }) => api_graphql::problems(host, query, ledger),
        None => Vec::new(),
    }
}

/// What is wrong with the REST call `call`, which read `stdin` as its body,
/// one message each.
fn rest_problems(call: &ApiCall, stdin: Option<&str>, ledger: &Ledger) -> Vec<String> {
    let description = match call.host {
        Host::GitHub => &*GITHUB,
        Host::GitLab => &*GITLAB,
    };
    let host = call.host.name();
    let method = &call.method;
    let templates = description.templates_for(&call.path, false);
    if templates.is_empty() {
        // Only an entry in the lists says a path that fits a template only
        // with extra segments is a name holding a slash: structurally it is
        // no different from an endpoint the description lacks.
        let listed = description
            .templates_for(&call.path, true)
            .into_iter()
            .filter(|template| description.paths[*template].contains_key(method))
            .any(|template| {
                excused(
                    ledger,
                    &format!("{host} {method} {template} unencoded slash"),
                )
            });
        if listed {
            return Vec::new();
        }
        return vec![format!(
            "{host}: no endpoint of the description matches {method} {}",
            call.path
        )];
    }
    let Some((template, operation)) = templates
        .iter()
        .find_map(|template| Some((*template, description.paths[*template].get(method)?)))
    else {
        let allowed: BTreeSet<&str> = templates
            .iter()
            .flat_map(|template| description.paths[*template].keys().map(String::as_str))
            .collect();
        return vec![format!(
            "{host}: {method} {} is not in the description, which has {} there",
            templates[0],
            allowed.into_iter().collect::<Vec<_>>().join(", ")
        )];
    };
    let operation_key = format!("{host} {method} {template}");
    match operation {
        Operation::Missing if excused(ledger, &operation_key) => Vec::new(),
        Operation::Missing => vec![format!(
            "{host}: {method} {template} is not in the description (list it as `{operation_key}`)"
        )],
        Operation::Described { query, body } => call
            .query
            .iter()
            .map(|name| {
                // An array parameter is sent as `scope[]` and described as `scope`.
                let plain = name.strip_suffix("[]").unwrap_or(name);
                if query.contains(name) { name } else { plain }
            })
            .filter(|name| !query.contains(*name))
            .filter_map(|name| {
                let key = format!("{operation_key} query {name}");
                (!excused(ledger, &key)).then(|| {
                    format!(
                        "{host}: {method} {template} takes no query parameter `{name}` \
                         (list it as `{key}`)"
                    )
                })
            })
            .chain(body_problems(
                &operation_key,
                body.as_ref(),
                call.body(stdin),
                ledger,
            ))
            .collect(),
    }
}

/// Every problem of the `gh api` / `glab api` calls recorded in
/// `<log_path>.argv.d/`, REST and GraphQL.
pub fn problems_in_log(log_path: &Path) -> Vec<String> {
    let ledger = Ledger::shipped();
    let mut problems: Vec<String> = recorded_calls(log_path)
        .iter()
        .flat_map(|call| {
            let argv: Vec<&str> = call.argv.iter().map(String::as_str).collect();
            check(&argv, call.stdin.as_deref(), &ledger)
        })
        .collect();
    problems.sort();
    problems.dedup();
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_lists::Ledger;

    fn problems(argv: &[&str]) -> Vec<String> {
        problems_with(
            argv,
            &Ledger::with_lists(r#"{"exceptions": [], "findings": []}"#),
        )
    }

    fn problems_with(argv: &[&str], ledger: &Ledger) -> Vec<String> {
        check(argv, None, ledger)
    }

    #[test]
    fn an_endpoint_the_description_lacks_is_refused() {
        assert_eq!(
            problems(&["gh", "api", "repos/o/r/frobnicate"]),
            ["github: no endpoint of the description matches GET /repos/o/r/frobnicate"]
        );
        assert_eq!(
            problems(&["glab", "api", "projects/o%2Fr/frobnicate"]),
            [
                "gitlab: no endpoint of the description matches GET /api/v4/projects/o%2Fr/frobnicate"
            ]
        );
    }

    #[test]
    fn a_method_the_endpoint_does_not_have_is_refused() {
        assert_eq!(
            problems(&["gh", "api", "-X", "POST", "repos/o/r/branches"]),
            [
                "github: POST /repos/{owner}/{repo}/branches is not in the description, which has GET there"
            ]
        );
    }

    #[test]
    fn a_field_or_input_turns_the_default_method_into_post() {
        let github = "github: POST /repos/{owner}/{repo}/branches is not in the description, \
                      which has GET there";
        let gitlab = "gitlab: POST /api/v4/projects/{id}/repository/branches/{branch} is not in \
                      the description, which has DELETE, GET there";
        for (argv, expected) in [
            (
                &["gh", "api", "repos/o/r/branches", "-f", "a=b"][..],
                github,
            ),
            (&["gh", "api", "repos/o/r/branches", "-F", "a=b"], github),
            (&["gh", "api", "repos/o/r/branches", "--input", "-"], github),
            (
                &[
                    "glab",
                    "api",
                    "projects/o%2Fr/repository/branches/x",
                    "-f",
                    "a=b",
                ],
                gitlab,
            ),
        ] {
            assert_eq!(problems(argv), [expected], "{argv:?}");
        }
    }

    #[test]
    fn a_query_parameter_the_operation_does_not_take_is_refused() {
        assert_eq!(
            problems(&["gh", "api", "repos/o/r/branches?bogus=1&per_page=5"]),
            [
                "github: GET /repos/{owner}/{repo}/branches takes no query parameter `bogus` \
              (list it as `github GET /repos/{owner}/{repo}/branches query bogus`)"
            ]
        );
        assert_eq!(
            problems(&[
                "gh",
                "api",
                "-X",
                "GET",
                "repos/o/r/branches",
                "-f",
                "bogus=1"
            ]),
            [
                "github: GET /repos/{owner}/{repo}/branches takes no query parameter `bogus` \
              (list it as `github GET /repos/{owner}/{repo}/branches query bogus`)"
            ]
        );
    }

    #[test]
    fn calls_the_description_accepts_pass() {
        for argv in [
            &[
                "gh",
                "api",
                "--paginate",
                "/repos/o/r/pulls/3/comments?per_page=100",
            ][..],
            &["gh", "api", "user", "--jq", ".login"],
            &[
                "gh",
                "api",
                "-X",
                "PATCH",
                "repos/o/r/milestones/2",
                "-f",
                "state=closed",
            ],
            &[
                "gh",
                "api",
                "-X",
                "DELETE",
                "repos/o/r/git/refs/heads/feat%2Fx",
            ],
            &["gh", "pr", "list", "--json", "number"],
            &[
                "glab",
                "api",
                "--paginate",
                "projects/o%2Fr/pipelines/7/jobs?scope[]=failed&per_page=100",
            ],
            &[
                "glab",
                "api",
                "groups/o%2Fg/issues?state=all&per_page=100&page=2",
            ],
            &["glab", "api", "-X", "POST", "projects/o%2Fr/jobs/3/play"],
            &["glab", "api", "todos?state=done"],
        ] {
            assert_eq!(problems(argv), Vec::<String>::new(), "{argv:?}");
        }
    }

    #[test]
    fn a_path_parameter_holding_an_unencoded_slash_passes_only_when_listed() {
        let call = [
            "glab",
            "api",
            "-X",
            "DELETE",
            "/projects/o%2Fr/repository/branches/feature/x",
        ];
        assert_eq!(
            problems(&call),
            ["gitlab: no endpoint of the description matches DELETE \
              /api/v4/projects/o%2Fr/repository/branches/feature/x"]
        );
        let listed = Ledger::with_lists(
            r#"{"exceptions": [
                {"key": "gitlab DELETE /api/v4/projects/{id}/repository/branches/{branch} unencoded slash",
                 "reason": "r"}], "findings": []}"#,
        );
        assert_eq!(problems_with(&call, &listed), Vec::<String>::new());
        assert_eq!(
            problems(&[
                "glab",
                "api",
                "-X",
                "DELETE",
                "/projects/o%2Fr/repository/branches/feature%2Fx"
            ]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_unencoded_slash_entry_covers_its_template_and_an_unlisted_path_is_unknown() {
        let listed = Ledger::with_lists(
            r#"{"exceptions": [
                {"key": "github GET /repos/{owner}/{repo}/commits/{ref} unencoded slash",
                 "reason": "r"}], "findings": []}"#,
        );
        assert_eq!(
            problems_with(&["gh", "api", "repos/o/r/commits/abc/frobnicate"], &listed),
            Vec::<String>::new(),
            "the list entry covers a name holding a slash, which this path may be"
        );
        assert_eq!(
            problems(&["gh", "api", "repos/o/r/commits/abc/frobnicate"]),
            [
                "github: no endpoint of the description matches GET /repos/o/r/commits/abc/frobnicate"
            ]
        );
    }

    #[test]
    fn only_git_refs_takes_the_rest_of_the_path() {
        assert_eq!(
            problems(&[
                "gh",
                "api",
                "-X",
                "DELETE",
                "repos/o/r/git/refs/heads/feature/x"
            ]),
            Vec::<String>::new()
        );
        for call in [
            "repos/o/r/commits/abc/frobnicate",
            "repos/o/r/commits/abc/check-runs",
        ] {
            assert_eq!(
                problems(&["gh", "api", call]),
                [format!(
                    "github: no endpoint of the description matches GET /{call}"
                )],
                "{call} must not fit commits/{{ref}}"
            );
        }
    }

    #[test]
    fn a_flag_value_is_not_taken_for_the_endpoint() {
        assert_eq!(
            problems(&[
                "gh",
                "api",
                "-H",
                "Accept: x",
                "--hostname",
                "h",
                "repos/o/r/frobnicate"
            ]),
            ["github: no endpoint of the description matches GET /repos/o/r/frobnicate"]
        );
    }

    #[test]
    fn an_operation_the_description_lacks_is_excused_by_its_list_entry_only() {
        let call = ["gh", "api", "-X", "POST", "repos/o/r/actions/jobs/9/cancel"];
        assert_eq!(
            problems(&call),
            [
                "github: POST /repos/{owner}/{repo}/actions/jobs/{job_id}/cancel is not in the description \
              (list it as `github POST /repos/{owner}/{repo}/actions/jobs/{job_id}/cancel`)"
            ]
        );
        let listed = Ledger::with_lists(
            r#"{"exceptions": [], "findings": [
                {"key": "github POST /repos/{owner}/{repo}/actions/jobs/{job_id}/cancel",
                 "reason": "r", "issue": "https://github.com/rcieri/glab-tui/issues/781"}]}"#,
        );
        assert_eq!(problems_with(&call, &listed), Vec::<String>::new());
    }

    #[test]
    fn a_listed_query_parameter_passes() {
        let listed = Ledger::with_lists(
            r#"{"exceptions": [
                {"key": "gitlab POST /api/v4/projects/{id}/repository/branches query branch",
                 "reason": "r"}], "findings": []}"#,
        );
        assert_eq!(
            problems_with(
                &[
                    "glab",
                    "api",
                    "-X",
                    "POST",
                    "projects/o%2Fr/repository/branches?branch=x"
                ],
                &listed
            ),
            Vec::<String>::new()
        );
    }

    fn problems_with_body(argv: &[&str], body: &str) -> Vec<String> {
        check(
            argv,
            Some(body),
            &Ledger::with_lists(r#"{"exceptions": [], "findings": []}"#),
        )
    }

    const CREATE_REF: [&str; 5] = ["gh", "api", "repos/o/r/git/refs", "--input", "-"];

    #[test]
    fn a_body_field_the_description_lacks_is_refused() {
        assert_eq!(
            problems_with_body(
                &CREATE_REF,
                r#"{"ref": "refs/heads/x", "sha": "a1", "frob": 1}"#
            ),
            [
                "github: POST /repos/{owner}/{repo}/git/refs: the body field `frob` is not in the \
                 description (list it as `github POST /repos/{owner}/{repo}/git/refs body frob`)"
            ]
        );
    }

    #[test]
    fn a_body_field_of_the_wrong_type_is_refused() {
        assert_eq!(
            problems_with_body(&CREATE_REF, r#"{"ref": 7, "sha": "a1"}"#),
            [
                "github: POST /repos/{owner}/{repo}/git/refs: the body field `ref` is an integer, \
                 the description says string (list it as `github POST /repos/{owner}/{repo}/git/refs body ref`)"
            ]
        );
    }

    #[test]
    fn a_field_inside_an_array_of_the_body_is_named_by_its_path() {
        let review = ["gh", "api", "repos/o/r/pulls/2/reviews", "--input", "-"];
        assert_eq!(
            problems_with_body(
                &review,
                r#"{"event": "COMMENT", "comments": [{"path": "a", "body": "b", "frob": 1}]}"#
            ),
            [
                "github: POST /repos/{owner}/{repo}/pulls/{pull_number}/reviews: the body field \
                 `comments[].frob` is not in the description (list it as \
                 `github POST /repos/{owner}/{repo}/pulls/{pull_number}/reviews body comments[].frob`)"
            ]
        );
    }

    #[test]
    fn fields_given_as_flags_are_the_body() {
        let milestone = |field: &'static str, value: &'static str| {
            check(
                &[
                    "gh",
                    "api",
                    "repos/o/r/milestones",
                    "-f",
                    "title=t",
                    field,
                    value,
                ],
                None,
                &Ledger::with_lists(r#"{"exceptions": [], "findings": []}"#),
            )
        };
        assert_eq!(milestone("-f", "due_on=5"), Vec::<String>::new());
        assert_eq!(
            milestone("-F", "due_on=5"),
            [
                "github: POST /repos/{owner}/{repo}/milestones: the body field `due_on` is an \
                 integer, the description says string (list it as \
                 `github POST /repos/{owner}/{repo}/milestones body due_on`)"
            ]
        );
        assert_eq!(
            milestone("-f", "frob=x"),
            [
                "github: POST /repos/{owner}/{repo}/milestones: the body field `frob` is not in \
                 the description (list it as `github POST /repos/{owner}/{repo}/milestones body frob`)"
            ]
        );
    }

    #[test]
    fn a_body_to_an_operation_that_takes_none_is_refused() {
        assert_eq!(
            problems_with_body(
                &[
                    "gh",
                    "api",
                    "-X",
                    "PATCH",
                    "notifications/threads/1",
                    "--input",
                    "-"
                ],
                "{}"
            ),
            [
                "github: PATCH /notifications/threads/{thread_id} takes no request body \
                 (list it as `github PATCH /notifications/threads/{thread_id} body`)"
            ]
        );
    }

    #[test]
    fn a_body_that_is_not_json_is_refused() {
        assert_eq!(
            problems_with_body(&CREATE_REF, "ref=x"),
            [
                "github: POST /repos/{owner}/{repo}/git/refs: the body is not JSON: \
              expected value at line 1 column 1"
            ]
        );
    }

    #[test]
    fn a_listed_body_field_passes() {
        let listed = Ledger::with_lists(
            r#"{"exceptions": [
                {"key": "github POST /repos/{owner}/{repo}/git/refs body frob", "reason": "r"}],
                "findings": []}"#,
        );
        assert_eq!(
            check(
                &CREATE_REF,
                Some(r#"{"ref": "x", "sha": "a", "frob": 1}"#),
                &listed
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_recorded_call_is_reported_once() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("cli_calls.log");
        let records = dir.path().join("cli_calls.log.argv.d");
        std::fs::create_dir(&records).unwrap();
        for name in ["a", "b"] {
            std::fs::write(
                records.join(name),
                "gh\u{1f}api\u{1f}repos/o/r/frobnicate\u{1f}",
            )
            .unwrap();
        }
        assert_eq!(problems_in_log(&log_path).len(), 1);
    }

    /// The query a GraphQL call sends; panics for any other call.
    fn graphql_query<'a>(argv: &[&'a str]) -> Option<&'a str> {
        match ApiRequest::read(argv) {
            Some(ApiRequest::Graphql { query, .. }) => query,
            _ => panic!("{argv:?} is not read as a GraphQL call"),
        }
    }

    #[test]
    fn the_query_is_read_from_every_spelling_of_the_field_flag() {
        let q = "{ viewer { login } }";
        let value = format!("query={q}");
        for argv in [
            vec!["gh", "api", "graphql", "-f", value.as_str()],
            vec!["gh", "api", "graphql", "--raw-field", value.as_str()],
            vec!["gh", "api", "graphql", "-F", value.as_str()],
            vec!["gh", "api", "graphql", "--field", value.as_str()],
            vec!["glab", "api", "graphql", "-f", value.as_str()],
        ] {
            assert_eq!(graphql_query(&argv), Some(q), "{argv:?}");
        }
        let inline = [
            format!("--raw-field={value}"),
            format!("--field={value}"),
            format!("-f{value}"),
            format!("-F{value}"),
        ];
        for word in &inline {
            assert_eq!(
                graphql_query(&["gh", "api", "graphql", word]),
                Some(q),
                "{word}"
            );
        }
        assert_eq!(
            graphql_query(&["gh", "api", "--paginate", "graphql", "-f", &value]),
            Some(q)
        );
    }

    #[test]
    fn a_call_is_graphql_only_when_its_endpoint_is() {
        for argv in [
            &["gh", "api", "repos/o/r", "-f", "q=graphql"][..],
            &["gh", "api", "-H", "graphql", "repos/o/r"],
        ] {
            assert!(
                matches!(ApiRequest::read(argv), Some(ApiRequest::Rest(_))),
                "{argv:?}"
            );
        }
        assert!(ApiRequest::read(&["gh", "pr", "list"]).is_none());
        assert!(ApiRequest::read(&["git", "api", "graphql"]).is_none());
    }

    #[test]
    fn a_graphql_call_without_a_query_is_reported() {
        assert_eq!(
            problems(&["gh", "api", "graphql", "-f", "other=1"]),
            ["github: graphql call sends no `query=` field"]
        );
    }

    #[test]
    fn a_recorded_graphql_call_is_reported_once() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("cli_calls.log");
        let records = dir.path().join("cli_calls.log.argv.d");
        std::fs::create_dir(&records).unwrap();
        for name in ["a", "b"] {
            std::fs::write(
                records.join(name),
                "glab\u{1f}api\u{1f}graphql\u{1f}-f\u{1f}query={ currentUser { nope } }\u{1f}",
            )
            .unwrap();
        }
        assert_eq!(
            problems_in_log(&log_path),
            [
                "gitlab: graphql field `CurrentUser.nope` is not in gitlab.graphql.json: the \
              schema lacks it, or scripts/api-descriptions/gitlab.json does not list it yet \
              (the generator fails when the schema lacks a listed field) (at currentUser.nope)"
            ]
        );
    }
}
