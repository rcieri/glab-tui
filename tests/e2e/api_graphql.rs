//! Checks the GraphQL queries the CLI mocks record (`gh api graphql -f
//! query=…`, `glab api graphql -f query=…`) against the field tables in
//! `tests/api-descriptions/<host>.graphql.json`: every selected field, every
//! argument name, and whether a field takes a selection must agree with the
//! table, aliases, inline fragments and fragment spreads included. Argument
//! values and variable types are not read.
//!
//! The table holds only the fields `scripts/api-descriptions/<host>.json`
//! lists, so a field it lacks is either absent from the schema or not listed
//! yet; the generator fails when a listed field is absent from the schema,
//! and the message names both causes. The generator lists fields from the
//! query root and the mutation root down through objects and interfaces only,
//! so a field of a union member is always reported, and a type condition is
//! not checked against the type it sits in. A field or argument the schema marks
//! deprecated is excused through the shared `Ledger`, by the key
//! `<host> graphql <Type>.<field> deprecated` or
//! `<host> graphql <Type>.<field>(<argument>) deprecated`, as finding or
//! exception. It names the type and field, never the response path, so the
//! number of aliases a query sends does not matter.
//!
//! What the mock printed for the call is checked against the same query and
//! table, one JSON document per page (`--paginate`; the one array
//! `--slurp` prints is not read): every key a field the query selects, every
//! selected field present unless `@include` / `@skip` or a type condition
//! other than the field's declared type may leave it out, and each value of
//! its field's type (`null` only where nullable, an array only for a list,
//! `Int` within 32 bits, a string for every enum and every scalar but `Int`,
//! `Float` and `Boolean`). A field under a type condition is read against the
//! condition's type, `__typename` must name the object's type where that is
//! an object type, and the top holds `data` or `errors`, and nothing but
//! `data`, `errors` (a non-empty list, required when `data` is null) and
//! `extensions`. Only the first operation of a document is read, the type of
//! an interface or union value is not resolved from its `__typename`, one
//! response key shared by different fields is not reported, and enum values
//! are not read. A reply to a query with problems is not read.

use crate::api_lists::Ledger;
use crate::api_requests::{Host, excused, type_of};
use graphql_parser::query::{
    self as gql, Definition, OperationDefinition, Selection, SelectionSet, TypeCondition,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::LazyLock;

/// Nesting deeper than this is reported instead of followed.
const MAX_DEPTH: usize = 32;

#[derive(Deserialize)]
struct FieldDef {
    #[serde(rename = "type")]
    result: TypeRef,
    #[serde(default)]
    args: Vec<String>,
    /// Argument name to deprecation reason.
    #[serde(default)]
    deprecated_args: BTreeMap<String, String>,
    #[serde(default)]
    deprecated: Option<String>,
}

/// A field's result type, read from the table's notation: `[Deployment!]!`
/// is a non-null list of non-null `Deployment` objects.
#[derive(Debug, Deserialize)]
#[serde(try_from = "String")]
enum TypeRef {
    Named(String),
    List(Box<TypeRef>),
    NonNull(Box<TypeRef>),
}

impl TryFrom<String> for TypeRef {
    type Error = String;

    fn try_from(notation: String) -> Result<Self, String> {
        TypeRef::parse(&notation)
    }
}

impl TypeRef {
    fn parse(notation: &str) -> Result<TypeRef, String> {
        if let Some(inner) = notation.strip_suffix('!') {
            return match TypeRef::parse(inner)? {
                TypeRef::NonNull(_) => Err(format!("`{notation}` is non-null twice")),
                of => Ok(TypeRef::NonNull(Box::new(of))),
            };
        }
        if let Some(inner) = notation
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            return Ok(TypeRef::List(Box::new(TypeRef::parse(inner)?)));
        }
        if !notation.is_empty()
            && notation
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            Ok(TypeRef::Named(notation.to_string()))
        } else {
            Err(format!("`{notation}` is not a GraphQL type"))
        }
    }

    /// The named type inside the wrappers.
    fn named(&self) -> &str {
        match self {
            TypeRef::Named(name) => name,
            TypeRef::List(of) | TypeRef::NonNull(of) => of.named(),
        }
    }
}

impl fmt::Display for TypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeRef::Named(name) => f.write_str(name),
            TypeRef::List(of) => write!(f, "[{of}]"),
            TypeRef::NonNull(of) => write!(f, "{of}!"),
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TypeKind {
    Object,
    Interface,
    Union,
    Scalar,
    Enum,
    Input,
}

impl fmt::Display for TypeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TypeKind::Object => "object",
            TypeKind::Interface => "interface",
            TypeKind::Union => "union",
            TypeKind::Scalar => "scalar",
            TypeKind::Enum => "enum",
            TypeKind::Input => "input",
        })
    }
}

#[derive(Deserialize)]
struct TypeDef {
    kind: TypeKind,
    #[serde(default)]
    fields: BTreeMap<String, FieldDef>,
}

impl TypeDef {
    fn is_leaf(&self) -> bool {
        matches!(self.kind, TypeKind::Scalar | TypeKind::Enum)
    }
}

#[derive(Deserialize)]
struct Table {
    query: String,
    mutation: Option<String>,
    types: BTreeMap<String, TypeDef>,
}

static GITHUB: LazyLock<Table> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../api-descriptions/github.graphql.json"))
        .expect("github.graphql.json is a field table")
});

static GITLAB: LazyLock<Table> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../api-descriptions/gitlab.graphql.json"))
        .expect("gitlab.graphql.json is a field table")
});

/// What is wrong with one selection, and where the query sent it.
struct Problem {
    /// The wording, which names the type and field but not the response path.
    message: String,
    /// The response path of the first occurrence, aliases as sent.
    path: String,
}

struct Walk<'a, 'q> {
    host: &'a str,
    table: &'a Table,
    ledger: &'a Ledger,
    fragments: HashMap<&'q str, &'q gql::FragmentDefinition<'q, &'q str>>,
    /// The fragments whose selections are being walked, outermost first.
    spreading: Vec<&'q str>,
    problems: Vec<Problem>,
}

impl<'q> Walk<'_, 'q> {
    fn problem(&mut self, path: &str, message: String) {
        self.problems.push(Problem {
            message: format!("{}: graphql {message}", self.host),
            path: path.to_string(),
        });
    }

    /// Reports `what` as deprecated for `reason`, unless the ledger lists
    /// `<host> graphql <subject> deprecated`.
    fn deprecated(&mut self, path: &str, subject: &str, what: &str, reason: &str) {
        let key = format!("{} graphql {subject} deprecated", self.host);
        if !excused(self.ledger, &key) {
            self.problem(
                path,
                format!("{what} is deprecated: {reason} (list it as `{key}`)"),
            );
        }
    }

    fn not_in_table(&self, what: &str) -> String {
        let host = self.host;
        format!(
            "{what} is not in {host}.graphql.json: the schema lacks it, or \
             scripts/api-descriptions/{host}.json does not list it yet (the generator fails \
             when the schema lacks a listed field)"
        )
    }

    fn selection_set(
        &mut self,
        on: &str,
        set: &'q SelectionSet<'q, &'q str>,
        path: &str,
        depth: usize,
    ) {
        if depth > MAX_DEPTH {
            return self.problem(path, format!("selection nested deeper than {MAX_DEPTH}"));
        }
        for selection in &set.items {
            match selection {
                Selection::Field(field) => self.field(on, field, path, depth),
                Selection::InlineFragment(fragment) => {
                    let target = match &fragment.type_condition {
                        Some(TypeCondition::On(target)) => *target,
                        None => on,
                    };
                    if self.table.types.contains_key(target) {
                        self.selection_set(target, &fragment.selection_set, path, depth + 1);
                    } else {
                        let message = self.not_in_table(&format!("type `{target}`"));
                        self.problem(path, message);
                    }
                }
                Selection::FragmentSpread(spread) => {
                    let Some(definition) = self.fragments.get(spread.fragment_name).copied() else {
                        self.problem(path, format!("unknown fragment `{}`", spread.fragment_name));
                        continue;
                    };
                    if self.spreading.contains(&spread.fragment_name) {
                        self.problem(
                            path,
                            format!("fragment `{}` spreads itself", spread.fragment_name),
                        );
                        continue;
                    }
                    let TypeCondition::On(target) = definition.type_condition;
                    if !self.table.types.contains_key(target) {
                        let message = self.not_in_table(&format!("type `{target}`"));
                        self.problem(path, message);
                        continue;
                    }
                    self.spreading.push(spread.fragment_name);
                    self.selection_set(target, &definition.selection_set, path, depth + 1);
                    self.spreading.pop();
                }
            }
        }
    }

    fn field(&mut self, on: &str, field: &'q gql::Field<'q, &'q str>, parent: &str, depth: usize) {
        if field.name == "__typename" {
            return;
        }
        let shown = field.alias.unwrap_or(field.name);
        let path = joined(parent, shown);
        let name = field.name;
        let Some(definition) = self.table.types.get(on).and_then(|t| t.fields.get(name)) else {
            let message = self.not_in_table(&format!("field `{on}.{name}`"));
            return self.problem(&path, message);
        };
        if let Some(reason) = &definition.deprecated {
            self.deprecated(
                &path,
                &format!("{on}.{name}"),
                &format!("`{on}.{name}`"),
                reason,
            );
        }
        for (argument, _) in &field.arguments {
            if !definition
                .args
                .iter()
                .any(|known| known.as_str() == *argument)
            {
                self.problem(&path, format!("`{on}.{name}` has no argument `{argument}`"));
            } else if let Some(reason) = definition.deprecated_args.get(*argument) {
                self.deprecated(
                    &path,
                    &format!("{on}.{name}({argument})"),
                    &format!("argument `{argument}` of `{on}.{name}`"),
                    reason,
                );
            }
        }
        let result = definition.result.named();
        let Some(target) = self.table.types.get(result) else {
            let message = self.not_in_table(&format!("type `{result}` of `{on}.{name}`"));
            return self.problem(&path, message);
        };
        match (target.is_leaf(), field.selection_set.items.is_empty()) {
            (true, false) => self.problem(
                &path,
                format!(
                    "`{on}.{name}` is of kind {} (`{result}`), which takes no selection",
                    target.kind
                ),
            ),
            (false, true) => self.problem(
                &path,
                format!(
                    "`{on}.{name}` is of kind {} (`{result}`), which needs a selection",
                    target.kind
                ),
            ),
            (false, false) => self.selection_set(result, &field.selection_set, &path, depth + 1),
            (true, true) => {}
        }
    }
}

/// What is wrong with `query` against `table`, one message each, the same
/// type and field reported once however many aliases select it.
fn check_query(host: &str, table: &Table, query: &str, ledger: &Ledger) -> Vec<String> {
    let document = match gql::parse_query::<&str>(query) {
        Ok(document) => document,
        Err(e) => return vec![format!("{host}: graphql query does not parse: {e}")],
    };
    let fragments = fragments_of(&document);
    let mut walk = Walk {
        host,
        table,
        ledger,
        fragments,
        spreading: Vec::new(),
        problems: Vec::new(),
    };
    for definition in &document.definitions {
        let Definition::Operation(operation) = definition else {
            continue;
        };
        let (root, set) = root_of(table, operation);
        match root {
            Some(root) => walk.selection_set(root, set, "", 0),
            None => walk.problem("", "operation type is not in the schema".to_string()),
        }
    }
    walk.problems.sort_by(|a, b| a.message.cmp(&b.message));
    let mut seen = BTreeSet::new();
    walk.problems
        .into_iter()
        .filter(|problem| seen.insert(problem.message.clone()))
        .map(|problem| match problem.path.as_str() {
            "" => problem.message,
            path => format!("{} (at {path})", problem.message),
        })
        .collect()
}

/// A field a selection asks for, and the type whose field it is: the
/// object's type, or the type condition of the fragment it sits in.
/// `optional`: whether this selection may leave it out.
#[derive(Clone, Copy)]
struct Asked<'q> {
    on: &'q str,
    field: &'q gql::Field<'q, &'q str>,
    optional: bool,
}

/// The fields a reply object holds under one key, from every selection that
/// asks for it.
struct Wanted<'q> {
    fields: Vec<Asked<'q>>,
    /// Whether every selection asking for it may leave it out: under
    /// `@include` or `@skip`, or under a type condition other than the type
    /// of the object.
    optional: bool,
}

#[derive(Clone, Copy)]
struct ReplyField<'q> {
    on: &'q str,
    name: &'q str,
    ty: &'q TypeRef,
}

struct Reply<'q> {
    host: &'q str,
    table: &'q Table,
    fragments: HashMap<&'q str, &'q gql::FragmentDefinition<'q, &'q str>>,
    problems: Vec<String>,
}

fn is_conditional<'q>(directives: &[gql::Directive<'q, &'q str>]) -> bool {
    directives
        .iter()
        .any(|directive| matches!(directive.name, "include" | "skip"))
}

fn joined(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

/// Whether `value` is a value of the scalar `name`. A custom scalar is a
/// string, as every custom scalar in the tables is.
fn is_scalar(name: &str, value: &Value) -> bool {
    match name {
        "Int" => value.as_i64().is_some_and(|n| i32::try_from(n).is_ok()),
        "Float" => value.is_number(),
        "Boolean" => value.is_boolean(),
        _ => value.is_string(),
    }
}

impl<'q> Reply<'q> {
    fn problem(&mut self, message: String) {
        self.problems
            .push(format!("{}: graphql reply {message}", self.host));
    }

    /// Adds the keys `set` asks an object of type `on` for to `out`.
    fn collect(
        &self,
        on: &'q str,
        set: &'q SelectionSet<'q, &'q str>,
        optional: bool,
        depth: usize,
        out: &mut BTreeMap<&'q str, Wanted<'q>>,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        for selection in &set.items {
            match selection {
                Selection::Field(field) => {
                    let optional = optional || is_conditional(&field.directives);
                    let wanted = out
                        .entry(field.alias.unwrap_or(field.name))
                        .or_insert(Wanted {
                            fields: Vec::new(),
                            optional,
                        });
                    wanted.optional &= optional;
                    wanted.fields.push(Asked {
                        on,
                        field,
                        optional,
                    });
                }
                Selection::InlineFragment(fragment) => {
                    let target = match &fragment.type_condition {
                        Some(TypeCondition::On(target)) => *target,
                        None => on,
                    };
                    let optional = optional || target != on || is_conditional(&fragment.directives);
                    self.collect(target, &fragment.selection_set, optional, depth + 1, out);
                }
                Selection::FragmentSpread(spread) => {
                    if let Some(definition) = self.fragments.get(spread.fragment_name).copied() {
                        let TypeCondition::On(target) = definition.type_condition;
                        let optional =
                            optional || target != on || is_conditional(&spread.directives);
                        self.collect(target, &definition.selection_set, optional, depth + 1, out);
                    }
                }
            }
        }
    }

    /// Checks the reply object `fields`, of type `on`, against `sets`, each
    /// with whether the selection holding it may leave the object out.
    fn object(
        &mut self,
        on: &'q str,
        sets: &[(&'q SelectionSet<'q, &'q str>, bool)],
        fields: &serde_json::Map<String, Value>,
        path: &str,
        depth: usize,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut wanted = BTreeMap::new();
        for &(set, optional) in sets {
            self.collect(on, set, optional, depth, &mut wanted);
        }
        for (key, value) in fields {
            let here = joined(path, key);
            match wanted.get(key.as_str()) {
                Some(wanted) => self.field(on, wanted, value, &here, depth),
                None => self.problem(format!("field `{here}` is not in the query's selection")),
            }
        }
        for (key, wanted) in &wanted {
            if !wanted.optional && !fields.contains_key(*key) {
                let here = joined(path, key);
                self.problem(format!("lacks `{here}`, which the query selects"));
            }
        }
    }

    /// Checks `value`, held under one key of an object of type `on`.
    fn field(&mut self, on: &'q str, wanted: &Wanted<'q>, value: &Value, path: &str, depth: usize) {
        let asked = wanted.fields[0];
        let name = asked.field.name;
        if name == "__typename" {
            let table = self.table;
            match value.as_str() {
                None => {
                    let found = type_of(value);
                    self.problem(format!(
                        "field `{path}` is {found}, but `__typename` is `String!`"
                    ));
                }
                Some(named)
                    if named != on
                        && table
                            .types
                            .get(on)
                            .is_some_and(|t| matches!(t.kind, TypeKind::Object)) =>
                {
                    self.problem(format!(
                        "field `{path}` is {value}, but the object is of type `{on}`"
                    ));
                }
                Some(_) => {}
            }
            return;
        }
        let table = self.table;
        // The request check reports a field the table lacks.
        let Some(definition) = table.types.get(asked.on).and_then(|t| t.fields.get(name)) else {
            return;
        };
        let sets: Vec<_> = wanted
            .fields
            .iter()
            .map(|asked| (&asked.field.selection_set, asked.optional))
            .collect();
        let of = ReplyField {
            on: asked.on,
            name,
            ty: &definition.result,
        };
        self.value(of, &definition.result, &sets, value, path, depth);
    }

    /// Checks `value` against `ty`, the part of the field type `of.ty` it holds.
    fn value(
        &mut self,
        of: ReplyField<'q>,
        ty: &'q TypeRef,
        sets: &[(&'q SelectionSet<'q, &'q str>, bool)],
        value: &Value,
        path: &str,
        depth: usize,
    ) {
        let fits = match (ty, value) {
            (TypeRef::NonNull(_), Value::Null) => false,
            (TypeRef::NonNull(inner), _) => {
                return self.value(of, inner, sets, value, path, depth);
            }
            (_, Value::Null) => true,
            (TypeRef::List(item), Value::Array(items)) => {
                let here = format!("{path}[]");
                for item_value in items {
                    self.value(of, item, sets, item_value, &here, depth);
                }
                true
            }
            (TypeRef::List(_), _) => false,
            (TypeRef::Named(name), _) => match self.table.types.get(name.as_str()) {
                // The request check reports a type the table lacks.
                None => true,
                Some(definition) => match definition.kind {
                    TypeKind::Scalar => is_scalar(name, value),
                    TypeKind::Enum => value.is_string(),
                    TypeKind::Input => false,
                    TypeKind::Object | TypeKind::Interface | TypeKind::Union => {
                        match value.as_object() {
                            Some(fields) => {
                                self.object(name, sets, fields, path, depth + 1);
                                true
                            }
                            None => false,
                        }
                    }
                },
            },
        };
        if !fits {
            let found = match ty {
                TypeRef::Named(name) if name == "Int" && (value.is_i64() || value.is_u64()) => {
                    "an integer beyond 32 bits"
                }
                _ => type_of(value),
            };
            let ReplyField { on, name, ty } = of;
            self.problem(format!(
                "field `{path}` is {found}, but `{on}.{name}` is `{ty}`"
            ));
        }
    }
}

/// The fragment definitions of `document`, by name.
fn fragments_of<'q>(
    document: &'q gql::Document<'q, &'q str>,
) -> HashMap<&'q str, &'q gql::FragmentDefinition<'q, &'q str>> {
    document
        .definitions
        .iter()
        .filter_map(|definition| match definition {
            Definition::Fragment(fragment) => Some((fragment.name, fragment)),
            Definition::Operation(_) => None,
        })
        .collect()
}

/// The root type `table` gives `operation`, if any, and its selection.
fn root_of<'t, 'q>(
    table: &'t Table,
    operation: &'q OperationDefinition<'q, &'q str>,
) -> (Option<&'t str>, &'q SelectionSet<'q, &'q str>) {
    match operation {
        OperationDefinition::SelectionSet(set) => (Some(table.query.as_str()), set),
        OperationDefinition::Query(query) => (Some(table.query.as_str()), &query.selection_set),
        OperationDefinition::Mutation(mutation) => {
            (table.mutation.as_deref(), &mutation.selection_set)
        }
        OperationDefinition::Subscription(subscription) => (None, &subscription.selection_set),
    }
}

/// What is wrong with what a mock printed for a GraphQL call sending
/// `query`, one message each. `--paginate` prints one JSON document per page.
/// `query` must have passed the request check: a fragment that spreads itself
/// would be followed without end. A query that does not parse, or whose
/// root the table lacks, is left to the request check.
fn check_reply(host: &str, table: &Table, query: &str, stdout: &str) -> Vec<String> {
    let Ok(document) = gql::parse_query::<&str>(query) else {
        return Vec::new();
    };
    let operation = document
        .definitions
        .iter()
        .find_map(|definition| match definition {
            Definition::Operation(operation) => Some(operation),
            Definition::Fragment(_) => None,
        });
    let Some((Some(root), set)) = operation.map(|operation| root_of(table, operation)) else {
        return Vec::new();
    };
    let mut reply = Reply {
        host,
        table,
        fragments: fragments_of(&document),
        problems: Vec::new(),
    };
    for page in serde_json::Deserializer::from_str(stdout).into_iter::<Value>() {
        let page = match page {
            Ok(page) => page,
            Err(error) => {
                reply.problem(format!("is not JSON: {error}"));
                break;
            }
        };
        let Some(top) = page.as_object() else {
            reply.problem(format!("is {}, not an object", type_of(&page)));
            continue;
        };
        if !top.contains_key("data") && !top.contains_key("errors") {
            reply.problem("has neither `data` nor `errors`".to_string());
        }
        if top.get("data").is_some_and(Value::is_null) && !top.contains_key("errors") {
            reply.problem("has `data: null` but no `errors`".to_string());
        }
        for (key, value) in top {
            match (key.as_str(), value) {
                ("errors", Value::Array(errors)) if errors.is_empty() => {
                    reply.problem("field `errors` is an empty list".to_string());
                }
                ("data", Value::Null) | ("errors", Value::Array(_)) | ("extensions", _) => {}
                ("data", Value::Object(fields)) => {
                    reply.object(root, &[(set, false)], fields, "data", 0)
                }
                ("data", _) => {
                    let found = type_of(value);
                    reply.problem(format!("field `data` is {found}, but `data` is `{root}`"));
                }
                ("errors", _) => {
                    let found = type_of(value);
                    reply.problem(format!("field `errors` is {found}, not a list"));
                }
                _ => reply.problem(format!(
                    "field `{key}` is none of `data`, `errors` and `extensions`"
                )),
            }
        }
    }
    reply.problems.sort();
    reply.problems.dedup();
    reply.problems
}

/// What is wrong with a GraphQL call to `host` sending `query`, and with
/// `stdout`, what the mock printed for it, one message each.
pub fn problems(
    host: Host,
    query: Option<&str>,
    stdout: Option<&str>,
    ledger: &Ledger,
) -> Vec<String> {
    let table = match host {
        Host::GitHub => &*GITHUB,
        Host::GitLab => &*GITLAB,
    };
    let Some(query) = query else {
        return vec![format!(
            "{}: graphql call sends no `query=` field",
            host.name()
        )];
    };
    let found = check_query(host.name(), table, query, ledger);
    match stdout {
        Some(stdout) if found.is_empty() => check_reply(host.name(), table, query, stdout),
        _ => found,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_with_an_unknown_kind_does_not_load() {
        let table =
            r#"{"query": "Query", "mutation": null, "types": {"Query": {"kind": "objekt"}}}"#;
        assert!(serde_json::from_str::<Table>(table).is_err());
    }

    fn empty() -> Ledger {
        Ledger::with_lists(r#"{"exceptions": [], "findings": []}"#)
    }

    fn github(query: &str) -> Vec<String> {
        check_query("github", &GITHUB, query, &empty())
    }

    fn gitlab(query: &str) -> Vec<String> {
        check_query("gitlab", &GITLAB, query, &empty())
    }

    #[test]
    fn a_wrong_field_behind_an_alias_is_named_with_its_alias_path() {
        assert_eq!(
            github(
                r#"{ repository(owner: "o", name: "r") {
                pr_1: pullRequest(number: 1) { number titel }
            } }"#,
            ),
            [
                "github: graphql field `PullRequest.titel` is not in github.graphql.json: the schema lacks it, \
                 or scripts/api-descriptions/github.json does not list it yet (the generator \
                 fails when the schema lacks a listed field) (at repository.pr_1.titel)"
            ]
        );
    }

    #[test]
    fn the_message_names_both_causes_of_a_field_the_table_lacks() {
        assert_eq!(
            github("{ repository(owner: \"o\", name: \"r\") { bogus } }"),
            [
                "github: graphql field `Repository.bogus` is not in github.graphql.json: the schema lacks it, \
                 or scripts/api-descriptions/github.json does not list it yet (the generator \
                 fails when the schema lacks a listed field) (at repository.bogus)"
            ]
        );
    }

    #[test]
    fn the_same_type_and_field_is_reported_once_for_every_alias() {
        assert_eq!(
            github(
                r#"{ repository(owner: "o", name: "r") {
                pr_9: pullRequest(number: 9) { titel }
                pr_10: pullRequest(number: 10) { titel }
            } }"#,
            ),
            [
                "github: graphql field `PullRequest.titel` is not in github.graphql.json: the schema lacks it, \
                 or scripts/api-descriptions/github.json does not list it yet (the generator \
                 fails when the schema lacks a listed field) (at repository.pr_9.titel)"
            ]
        );
    }

    #[test]
    fn a_wrong_argument_name_is_refused() {
        assert_eq!(
            github(r#"{ repository(owner: "o", name: "r") { pullRequest(numbr: 1) { number } } }"#),
            [
                "github: graphql `Repository.pullRequest` has no argument `numbr` \
              (at repository.pullRequest)"
            ]
        );
    }

    #[test]
    fn a_selection_on_an_enum_or_scalar_is_refused() {
        assert_eq!(
            github(
                r#"{ repository(owner: "o", name: "r") {
                    pullRequest(number: 1) { state { value } }
                } }"#
            ),
            [
                "github: graphql `PullRequest.state` is of kind enum (`PullRequestState`), which takes \
              no selection (at repository.pullRequest.state)"
            ]
        );
        assert_eq!(
            github(
                r#"{ repository(owner: "o", name: "r") { pullRequest(number: 1) { number { x } } } }"#,
            ),
            [
                "github: graphql `PullRequest.number` is of kind scalar (`Int`), which takes no \
                 selection (at repository.pullRequest.number)"
            ]
        );
    }

    #[test]
    fn an_object_without_a_selection_is_refused() {
        assert_eq!(
            github(r#"{ repository(owner: "o", name: "r") { pullRequest(number: 1) } }"#),
            [
                "github: graphql `Repository.pullRequest` is of kind object (`PullRequest`), which \
                 needs a selection (at repository.pullRequest)"
            ]
        );
    }

    #[test]
    fn inline_fragments_and_spreads_are_walked() {
        assert_eq!(
            github(
                r#"{ repository(owner: "o", name: "r") {
                ... on Repository { pullRequest(number: 1) { number } bogus }
                ...Parts
            } }
            fragment Parts on Repository { issue(number: 2) { wrong } }"#,
            ),
            [
                "github: graphql field `Issue.wrong` is not in github.graphql.json: the schema lacks it, \
                 or scripts/api-descriptions/github.json does not list it yet (the generator \
                 fails when the schema lacks a listed field) (at repository.issue.wrong)",
                "github: graphql field `Repository.bogus` is not in github.graphql.json: the schema lacks it, \
                 or scripts/api-descriptions/github.json does not list it yet (the generator \
                 fails when the schema lacks a listed field) (at repository.bogus)"
            ]
        );
        assert_eq!(
            github(r#"{ repository(owner: "o", name: "r") { ... on Nope { x } ...Gone } }"#),
            [
                "github: graphql type `Nope` is not in github.graphql.json: the schema lacks it, \
                 or scripts/api-descriptions/github.json does not list it yet (the generator \
                 fails when the schema lacks a listed field) (at repository)",
                "github: graphql unknown fragment `Gone` (at repository)"
            ]
        );
    }

    #[test]
    fn a_fragment_spread_on_an_unknown_type_is_reported() {
        assert_eq!(
            github(
                r#"{ repository(owner: "o", name: "r") { ...F } } fragment F on Nope { __typename }"#
            ),
            [
                "github: graphql type `Nope` is not in github.graphql.json: the schema lacks it, \
                 or scripts/api-descriptions/github.json does not list it yet (the generator \
                 fails when the schema lacks a listed field) (at repository)"
            ]
        );
    }

    #[test]
    fn a_fragment_that_spreads_itself_is_reported() {
        assert_eq!(
            github(
                r#"{ repository(owner: "o", name: "r") { ...Loop } }
            fragment Loop on Repository { ...Loop ...Loop }"#,
            ),
            ["github: graphql fragment `Loop` spreads itself (at repository)"]
        );
    }

    #[test]
    fn typename_and_variables_pass() {
        assert_eq!(
            github(
                r#"query($after: String) { repository(owner: "o", name: "r") {
                    pullRequest(number: 1) {
                        reviewThreads(first: 100, after: $after) { __typename pageInfo { hasNextPage endCursor } }
                    }
                } }"#
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_query_that_does_not_parse_is_refused() {
        assert_eq!(
            github("{ repository("),
            [
                "github: graphql query does not parse: query parse error: Parse error at 1:14\n\
                 Unexpected end of input\nExpected Name\n"
            ]
        );
    }

    #[test]
    fn a_mutation_is_checked_against_the_mutation_root() {
        assert_eq!(
            github("mutation { frobnicate { x } }"),
            [
                "github: graphql field `Mutation.frobnicate` is not in github.graphql.json: the schema lacks it, \
                 or scripts/api-descriptions/github.json does not list it yet (the generator \
                 fails when the schema lacks a listed field) (at frobnicate)"
            ]
        );
    }

    #[test]
    fn a_deprecated_field_is_refused_unless_listed() {
        let query = r#"{ repository(owner: "o", name: "r") { pullRequest(number: 1) {
            reviewThreads(first: 1) { nodes { comments(first: 1) { nodes { databaseId } } } }
        } } }"#;
        assert_eq!(
            github(query),
            [
                "github: graphql `PullRequestReviewComment.databaseId` is deprecated: `databaseId` \
                 will be removed because it does not support 64-bit signed integer identifiers. \
                 Use `fullDatabaseId` instead. Removal on 2024-07-01 UTC. (list it as \
                 `github graphql PullRequestReviewComment.databaseId deprecated`) (at \
                 repository.pullRequest.reviewThreads.nodes.comments.nodes.databaseId)"
            ]
        );
        let listed = Ledger::with_lists(
            r#"{"exceptions": [
                {"key": "github graphql PullRequestReviewComment.databaseId deprecated",
                 "reason": "r"}], "findings": []}"#,
        );
        assert_eq!(
            check_query("github", &GITHUB, query, &listed),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_deprecated_argument_is_refused_unless_listed() {
        let query = r#"{ project(fullPath: "g/p") {
            runners(active: true) { nodes { id } }
        } }"#;
        assert_eq!(
            gitlab(query),
            [
                "gitlab: graphql argument `active` of `Project.runners` is deprecated: Renamed. \
                 Use `paused` instead. Deprecated in GitLab 14.8. (list it as \
                 `gitlab graphql Project.runners(active) deprecated`) (at project.runners)"
            ]
        );
        let listed = Ledger::with_lists(
            r#"{"exceptions": [
                {"key": "gitlab graphql Project.runners(active) deprecated",
                 "reason": "r"}], "findings": []}"#,
        );
        assert_eq!(
            check_query("gitlab", &GITLAB, query, &listed),
            Vec::<String>::new()
        );
    }

    #[test]
    fn gitlab_calls_are_checked_against_the_gitlab_table() {
        assert_eq!(
            gitlab(r#"{ project(fullPath: "o/r") { runners { nodes { id tagList } } } }"#),
            Vec::<String>::new()
        );
        assert_eq!(
            gitlab(r#"{ project(fullPath: "o/r") { pipelines(ids: []) { nodes { idd } } } }"#),
            [
                "gitlab: graphql field `Pipeline.idd` is not in gitlab.graphql.json: the schema lacks it, \
                 or scripts/api-descriptions/gitlab.json does not list it yet (the generator \
                 fails when the schema lacks a listed field) (at project.pipelines.nodes.idd)"
            ]
        );
        assert_eq!(
            gitlab(r#"{ project(path: "o/r") { runners { nodes { id } } } }"#),
            ["gitlab: graphql `Query.project` has no argument `path` (at project)"]
        );
    }

    const PULL_REQUEST: &str = r#"{ repository(owner: "o", name: "r") { pullRequest(number: 1) {
        number title isDraft stack { number entries { nodes { position } } }
    } } }"#;

    const STACKED: &str = r#"{"data": {"repository": {"pullRequest": {"number": 1, "title": "t",
        "isDraft": false, "stack": {"number": 7, "entries": {"nodes": [{"position": 1}]}}}}}}"#;

    fn reply(query: &str, stdout: &str) -> Vec<String> {
        check_reply("github", &GITHUB, query, stdout)
    }

    #[test]
    fn a_reply_that_fits_the_query_passes() {
        assert_eq!(reply(PULL_REQUEST, STACKED), Vec::<String>::new());
        let unstacked = STACKED.replace(
            r#"{"number": 7, "entries": {"nodes": [{"position": 1}]}}"#,
            "null",
        );
        assert_eq!(reply(PULL_REQUEST, &unstacked), Vec::<String>::new());
    }

    #[test]
    fn a_reply_field_outside_the_selection_is_refused() {
        let extra = STACKED.replace(r#""title": "t","#, r#""title": "t", "state": "OPEN","#);
        assert_eq!(
            reply(PULL_REQUEST, &extra),
            [
                "github: graphql reply field `data.repository.pullRequest.state` is not in the \
              query's selection"
            ]
        );
    }

    #[test]
    fn a_selected_field_missing_from_the_reply_is_refused() {
        let without_title = STACKED.replace(r#""title": "t","#, "");
        assert_eq!(
            reply(PULL_REQUEST, &without_title),
            [
                "github: graphql reply lacks `data.repository.pullRequest.title`, which the query \
              selects"
            ]
        );
    }

    #[test]
    fn null_for_a_non_null_field_is_refused() {
        let null_number = STACKED.replace(r#""number": 1,"#, r#""number": null,"#);
        assert_eq!(
            reply(PULL_REQUEST, &null_number),
            [
                "github: graphql reply field `data.repository.pullRequest.number` is null, but \
              `PullRequest.number` is `Int!`"
            ]
        );
        let null_position = STACKED.replace(r#""position": 1"#, r#""position": null"#);
        assert_eq!(
            reply(PULL_REQUEST, &null_position),
            ["github: graphql reply field \
              `data.repository.pullRequest.stack.entries.nodes[].position` is null, but \
              `PullRequestStackEntry.position` is `Int!`"]
        );
    }

    #[test]
    fn a_list_and_an_object_are_told_apart() {
        let object_nodes = STACKED.replace(r#"[{"position": 1}]"#, r#"{"position": 1}"#);
        assert_eq!(
            reply(PULL_REQUEST, &object_nodes),
            [
                "github: graphql reply field `data.repository.pullRequest.stack.entries.nodes` is \
              an object, but `PullRequestStackEntryConnection.nodes` is `[PullRequestStackEntry]`"
            ]
        );
        let listed_stack = STACKED.replace(
            r#""stack": {"number": 7, "entries": {"nodes": [{"position": 1}]}}"#,
            r#""stack": []"#,
        );
        assert_eq!(
            reply(PULL_REQUEST, &listed_stack),
            [
                "github: graphql reply field `data.repository.pullRequest.stack` is an array, but \
              `PullRequest.stack` is `PullRequestStack`"
            ]
        );
    }

    #[test]
    fn a_leaf_of_the_wrong_type_is_refused() {
        let wrong = STACKED
            .replace(r#""number": 1,"#, r#""number": "1","#)
            .replace(r#""isDraft": false"#, r#""isDraft": 0"#);
        assert_eq!(
            reply(PULL_REQUEST, &wrong),
            [
                "github: graphql reply field `data.repository.pullRequest.isDraft` is an \
                 integer, but `PullRequest.isDraft` is `Boolean!`",
                "github: graphql reply field `data.repository.pullRequest.number` is a string, \
                 but `PullRequest.number` is `Int!`"
            ]
        );
        assert_eq!(
            reply(
                r#"{ repository(owner: "o", name: "r") { pullRequest(number: 1) { state } } }"#,
                r#"{"data": {"repository": {"pullRequest": {"state": 1}}}}"#
            ),
            [
                "github: graphql reply field `data.repository.pullRequest.state` is an integer, \
              but `PullRequest.state` is `PullRequestState!`"
            ]
        );
    }

    #[test]
    fn a_custom_scalar_is_a_string() {
        let query = r#"{ project(fullPath: "o/r") { runners { nodes { contactedAt tagList } } } }"#;
        assert_eq!(
            check_reply(
                "gitlab",
                &GITLAB,
                query,
                r#"{"data": {"project": {"runners": {"nodes": [
                    {"contactedAt": "2026-01-01T00:00:00Z", "tagList": ["a"]}]}}}}"#
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            check_reply(
                "gitlab",
                &GITLAB,
                query,
                r#"{"data": {"project": {"runners": {"nodes": [
                    {"contactedAt": 5, "tagList": [null]}]}}}}"#
            ),
            [
                "gitlab: graphql reply field `data.project.runners.nodes[].contactedAt` is an \
                 integer, but `CiRunner.contactedAt` is `Time`",
                "gitlab: graphql reply field `data.project.runners.nodes[].tagList[]` is null, \
                 but `CiRunner.tagList` is `[String!]`"
            ]
        );
    }

    #[test]
    fn a_reply_is_keyed_by_alias() {
        let query =
            r#"{ repository(owner: "o", name: "r") { pr_1: pullRequest(number: 1) { number } } }"#;
        assert_eq!(
            reply(
                query,
                r#"{"data": {"repository": {"pr_1": {"number": 1}}}}"#
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            reply(
                query,
                r#"{"data": {"repository": {"pullRequest": {"number": 1}}}}"#
            ),
            [
                "github: graphql reply field `data.repository.pullRequest` is not in the query's \
                 selection",
                "github: graphql reply lacks `data.repository.pr_1`, which the query selects"
            ]
        );
    }

    #[test]
    fn a_field_under_include_may_be_left_out() {
        let query = r#"query($full: Boolean!) { repository(owner: "o", name: "r") {
            pullRequest(number: 1) @include(if: $full) { number }
        } }"#;
        assert_eq!(
            reply(query, r#"{"data": {"repository": {}}}"#),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_field_selected_again_under_include_keeps_its_sub_fields_optional() {
        let query = r#"query($full: Boolean!) { repository(owner: "o", name: "r") {
            pullRequest(number: 1) { number }
            pullRequest(number: 1) @include(if: $full) { number title }
        } }"#;
        assert_eq!(
            reply(
                query,
                r#"{"data": {"repository": {"pullRequest": {"number": 1}}}}"#
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            reply(query, r#"{"data": {"repository": {"pullRequest": {}}}}"#),
            [
                "github: graphql reply lacks `data.repository.pullRequest.number`, which the \
                 query selects"
            ]
        );
    }

    #[test]
    fn a_field_under_another_types_condition_may_be_left_out() {
        let query = r#"{ repository(owner: "o", name: "r") {
            issue(number: 2) { ... on PullRequest { isDraft } ... on Issue { title } __typename }
        } }"#;
        assert_eq!(
            reply(
                query,
                r#"{"data": {"repository": {"issue": {"title": "t", "__typename": "Issue"}}}}"#
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            reply(
                query,
                r#"{"data": {"repository": {"issue": {"__typename": 1}}}}"#
            ),
            [
                "github: graphql reply field `data.repository.issue.__typename` is an integer, \
                 but `__typename` is `String!`",
                "github: graphql reply lacks `data.repository.issue.title`, which the query \
                 selects"
            ]
        );
    }

    #[test]
    fn a_reply_holds_only_the_keys_of_a_graphql_response() {
        assert_eq!(
            reply(
                PULL_REQUEST,
                r#"{"data": null, "errors": [{"message": "m"}]}"#
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            reply(PULL_REQUEST, r#"{"errors": {}, "extra": 1}"#),
            [
                "github: graphql reply field `errors` is an object, not a list",
                "github: graphql reply field `extra` is none of `data`, `errors` and \
                 `extensions`"
            ]
        );
        assert_eq!(
            reply(PULL_REQUEST, "{}"),
            ["github: graphql reply has neither `data` nor `errors`"]
        );
        assert_eq!(
            reply(PULL_REQUEST, "[]"),
            ["github: graphql reply is an array, not an object"]
        );
    }

    #[test]
    fn every_page_of_a_paginated_reply_is_read() {
        let pages = format!("{STACKED}\n{}", STACKED.replace(r#""title": "t","#, ""));
        assert_eq!(
            reply(PULL_REQUEST, &pages),
            [
                "github: graphql reply lacks `data.repository.pullRequest.title`, which the query \
              selects"
            ]
        );
    }

    #[test]
    fn a_reply_that_is_not_json_is_refused() {
        assert_eq!(
            reply(PULL_REQUEST, "main"),
            ["github: graphql reply is not JSON: expected value at line 1 column 1"]
        );
    }

    #[test]
    fn a_field_under_a_type_condition_is_checked_against_that_type() {
        let query = r#"{ repository(owner: "o", name: "r") {
            issue(number: 2) { title ... on PullRequest { isDraft stack { number } } }
        } }"#;
        assert_eq!(
            reply(
                query,
                r#"{"data": {"repository": {"issue": {"title": "t", "isDraft": "x",
                    "stack": {"number": 7, "bogus": 1}}}}}"#
            ),
            [
                "github: graphql reply field `data.repository.issue.isDraft` is a string, but \
                 `PullRequest.isDraft` is `Boolean!`",
                "github: graphql reply field `data.repository.issue.stack.bogus` is not in the \
                 query's selection"
            ]
        );
    }

    #[test]
    fn a_typename_names_the_object_type() {
        let query = r#"{ repository(owner: "o", name: "r") { issue(number: 2) { __typename } } }"#;
        assert_eq!(
            reply(
                query,
                r#"{"data": {"repository": {"issue": {"__typename": "Nope"}}}}"#
            ),
            [
                "github: graphql reply field `data.repository.issue.__typename` is \"Nope\", but \
              the object is of type `Issue`"
            ]
        );
    }

    #[test]
    fn an_int_beyond_32_bits_is_refused() {
        let query =
            r#"{ repository(owner: "o", name: "r") { pullRequest(number: 1) { number } } }"#;
        assert_eq!(
            reply(
                query,
                r#"{"data": {"repository": {"pullRequest": {"number": 7785087816}}}}"#
            ),
            [
                "github: graphql reply field `data.repository.pullRequest.number` is an integer \
              beyond 32 bits, but `PullRequest.number` is `Int!`"
            ]
        );
    }

    #[test]
    fn errors_and_extensions_follow_the_response_format() {
        assert_eq!(
            reply(
                PULL_REQUEST,
                &STACKED.replacen('{', r#"{"extensions": {"cost": 1}, "#, 1)
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            reply(PULL_REQUEST, r#"{"data": null}"#),
            ["github: graphql reply has `data: null` but no `errors`"]
        );
        assert_eq!(
            reply(PULL_REQUEST, r#"{"data": null, "errors": []}"#),
            ["github: graphql reply field `errors` is an empty list"]
        );
    }

    #[test]
    fn the_reply_is_not_read_when_the_query_has_problems() {
        let looping = r#"{ repository(owner: "o", name: "r") { ...Loop } }
            fragment Loop on Repository { ...Loop ...Loop }"#;
        assert_eq!(
            problems(
                Host::GitHub,
                Some(looping),
                Some(r#"{"data": {"repository": {"extra": 1}}}"#),
                &empty()
            ),
            ["github: graphql fragment `Loop` spreads itself (at repository)"]
        );
    }

    #[test]
    fn a_type_notation_reads_its_wrappers() {
        for (notation, named) in [("[Deployment!]!", "Deployment"), ("[[String!]]", "String")] {
            let read = TypeRef::try_from(notation.to_string()).unwrap();
            assert_eq!(read.named(), named);
            assert_eq!(read.to_string(), notation);
        }
        for wrong in ["Deployment!!", "[Deployment", "", "Deploy ment"] {
            assert!(TypeRef::try_from(wrong.to_string()).is_err(), "{wrong}");
        }
    }
}
