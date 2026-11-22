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
//! query root down through objects and interfaces only, so a field of the
//! mutation root or of a union member is always reported, and a type condition
//! is not checked against the type it sits in. A field or argument the schema marks
//! deprecated is excused through the shared `Ledger`, by the key
//! `<host> graphql <Type>.<field> deprecated` or
//! `<host> graphql <Type>.<field>(<argument>) deprecated`, as finding or
//! exception. It names the type and field, never the response path, so the
//! number of aliases a query sends does not matter.

use crate::api_lists::Ledger;
use crate::api_requests::{Host, excused};
use graphql_parser::query::{
    self as gql, Definition, OperationDefinition, Selection, SelectionSet, TypeCondition,
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::LazyLock;

/// Nesting deeper than this is reported instead of followed.
const MAX_DEPTH: usize = 32;

#[derive(Deserialize)]
struct FieldDef {
    /// The named result type, list and non-null wrappers removed.
    #[serde(rename = "type")]
    type_name: String,
    #[serde(default)]
    args: Vec<String>,
    /// Argument name to deprecation reason.
    #[serde(default)]
    deprecated_args: BTreeMap<String, String>,
    #[serde(default)]
    deprecated: Option<String>,
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
        let path = if parent.is_empty() {
            shown.to_string()
        } else {
            format!("{parent}.{shown}")
        };
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
        let result = &definition.type_name;
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
    let fragments = document
        .definitions
        .iter()
        .filter_map(|definition| match definition {
            Definition::Fragment(fragment) => Some((fragment.name, fragment)),
            Definition::Operation(_) => None,
        })
        .collect();
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
        let (root, set) = match operation {
            OperationDefinition::SelectionSet(set) => (Some(table.query.as_str()), set),
            OperationDefinition::Query(query) => (Some(table.query.as_str()), &query.selection_set),
            OperationDefinition::Mutation(mutation) => {
                (table.mutation.as_deref(), &mutation.selection_set)
            }
            OperationDefinition::Subscription(subscription) => (None, &subscription.selection_set),
        };
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

/// What is wrong with a GraphQL call to `host` sending `query`, one message
/// each.
pub fn problems(host: Host, query: Option<&str>, ledger: &Ledger) -> Vec<String> {
    let table = match host {
        Host::GitHub => &*GITHUB,
        Host::GitLab => &*GITLAB,
    };
    match query {
        Some(query) => check_query(host.name(), table, query, ledger),
        None => vec![format!(
            "{}: graphql call sends no `query=` field",
            host.name()
        )],
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
}
