#!/usr/bin/env python3
"""Generates tests/api-descriptions/ from the API descriptions pinned below.

For GitHub and GitLab it writes two files from the pinned REST description
and the pinned GraphQL schema:

  <host>.openapi.json  the operations glab-tui calls, each with its query
                       parameters and the 2xx responses cut to the fields
                       glab-tui reads (OpenAPI 3.1)
  <host>.graphql.json  the GraphQL fields glab-tui selects, with their types,
                       argument names and the deprecation of fields and
                       arguments

What to keep comes from scripts/api-descriptions/<host>.json: the operations
with the fields read from each response, the host rule, the corrections for
known errors in the upstream description, and the GraphQL fields. The e2e
suite reads only the generated files.

Every download is checked against the SHA-256 pinned here, so a moved commit
or changed bytes fail the run instead of changing the output. The output is
sorted and has no timestamps: the same pins and inputs give the same bytes.
To change a pin, edit SOURCES, run the script and commit the new files.

  python3 scripts/gen-api-descriptions.py [--check] [--spec-dir DIR]
                                          [--out DIR] [--inputs DIR]

--check writes nothing and exits 1 when a generated file differs from what
the pins and inputs produce (CI runs it). PyYAML is needed for GitLab's YAML.

Rules for the response schemas:
  * Every 2xx response is kept. 204 and 205 are marked bodyless.
  * An operation may name a response_content_type for a body the description
    types wrongly (a job log is text); it must read no fields.
  * Each component schema is emitted once, with the union of every field set
    read from it anywhere, under its description name.
  * On a level glab-tui reads, every property of the description stays and no
    other properties are allowed, unless the description lists none for it.
    required is the description's required list cut to the fields read.
  * Host rule all_nullable (GitLab, whose description marks no nullability)
    makes every property nullable.
Problems are collected and reported together: a field read but absent from
the description (unless listed in absent_ok), an absent_ok or spec_absent_ok
entry the description has caught up with, a correction the description
already contains, an operation listed twice, an operation or GraphQL field
the description lacks.
"""
import argparse
import copy
import hashlib
import json
import os
import re
import subprocess
import sys
from typing import Any, Callable

SCALARS = {"string", "integer", "number", "boolean"}
MARK = "x-component"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DOWNLOAD_TIMEOUT_SECONDS = 600

GITHUB_RAW = "https://raw.githubusercontent.com/{repo}/{commit}/{path}"
GITLAB_RAW = "https://gitlab.com/{repo}/-/raw/{commit}/{path}"


def source(template: str, repo: str, commit: str, path: str, sha256: str) -> dict[str, str]:
    return {"repo": repo, "commit": commit, "path": path, "sha256": sha256,
            "url": template.format(repo=repo, commit=commit, path=path)}


GITLAB_COMMIT = "a63260946aaf41f9effa51234e03f4e6693f6ae8"

SOURCES = {
    "github": {
        "rest": source(
            GITHUB_RAW, "github/rest-api-description", "4754642742db86eba8971da648439698b46ac2ec",
            "descriptions/api.github.com/api.github.com.2022-11-28.json",
            "58f44930b993908e70e5f3e944d48adea6393cd8b7668f66d2c41fe48e95e5f5"),
        "graphql": source(
            GITHUB_RAW, "github/docs", "1655ba930f2fdb17bf23a87f8e9847acc05018aa",
            "src/graphql/data/fpt/schema.docs.graphql",
            "d928ebbb2d08cba348c0c5713ab3c8756a39e838324089ff66fcddf1bab2c496"),
        "license": None,
    },
    "gitlab": {
        "rest": source(
            GITLAB_RAW, "gitlab-org/gitlab", GITLAB_COMMIT,
            "doc/api/openapi/openapi_v3.yaml",
            "458acad3e40be731212801caff84b3befedbfcec96567cf00df10fc745e9937c"),
        "graphql": source(
            GITLAB_RAW, "gitlab-org/gitlab", GITLAB_COMMIT,
            "public/-/graphql/introspection_result.json",
            "a524ad2e389c619790e9fb5e9b05e37275a29dc59d51860c405e6b50c5f0e7a0"),
        # gitlab-org/gitlab/LICENSE: doc/ is CC BY-SA 4.0, everything else MIT Expat.
        "license": {
            "rest": {
                "name": "CC BY-SA 4.0",
                "url": "https://creativecommons.org/licenses/by-sa/4.0/",
                "notice": (
                    "A modified excerpt of the GitLab API description doc/api/openapi/openapi_v3.yaml "
                    "in gitlab-org/gitlab, Copyright GitLab B.V., licensed under CC BY-SA 4.0. Cut to "
                    "the operations and fields glab-tui uses, with every property marked nullable and "
                    "the corrections listed in scripts/api-descriptions/gitlab.json applied."),
            },
            "graphql": {
                "name": "MIT",
                "url": f"https://gitlab.com/gitlab-org/gitlab/-/blob/{GITLAB_COMMIT}/LICENSE",
                "notice": (
                    "A modified excerpt of public/-/graphql/introspection_result.json in "
                    "gitlab-org/gitlab, Copyright GitLab B.V., licensed under the MIT license. Cut to "
                    "the types and fields glab-tui uses."),
            },
        },
    },
}


def is_read(read: dict[str, Any], parts: list[str]) -> bool:
    for part in parts:
        if part not in read:
            return False
        read = read[part]
    return True


class Problems(Exception):
    def __init__(self, items: list[str]) -> None:
        super().__init__("\n".join(items))
        self.items = list(items)


def render(doc: Any) -> str:
    return json.dumps(doc, indent=2, sort_keys=True) + "\n"


def canon(node: Any) -> str:
    return json.dumps(node, sort_keys=True)


def op_name(op_id: str) -> str:
    """A spec operationId as a bare name; GitHub's contain '/' and '-'."""
    return "".join(w[:1].upper() + w[1:] for w in re.split(r"[-_/]", op_id) if w)


def merge_reads(a: dict[str, Any], b: dict[str, Any]) -> dict[str, Any]:
    out = {k: copy.deepcopy(v) for k, v in a.items()}
    for k, v in b.items():
        out[k] = merge_reads(out.get(k, {}), v)
    return out


def ref_name(ref: str) -> str:
    prefix = "#/components/schemas/"
    if not ref.startswith(prefix):
        raise Problems([f"unsupported $ref {ref}"])
    return ref[len(prefix):]


Event = tuple[str, str, str, bool]
Listed = tuple[tuple[str, str], str, dict[str, Any], dict[str, Any], dict[str, Any]]


class Pruner:
    """Walks response schemas along the fields glab-tui reads."""

    def __init__(self, host: str, spec: dict[str, Any], rule: str) -> None:
        self.host = host
        self.spec = spec
        self.rule = rule
        self.problems: list[str] = []
        self.reads_by_component: dict[str, dict[str, Any]] | None = None
        self.is_collecting_reads = False
        self.is_reporting_missing_fields = False
        self.pending_branch_events: list[Event] | None = None
        self.site = ""
        self.absent_ok_by_level: dict[str, dict[str, str]] = {}

    def schema(self, name: str) -> dict[str, Any]:
        return self.spec["components"]["schemas"][name]

    def resolve(self, node: dict[str, Any]) -> tuple[str | None, dict[str, Any], bool]:
        """(component name or None, resolved schema, nullable flag)."""
        name = None
        nullable = bool(node.get("nullable"))
        if "$ref" in node:
            name = ref_name(node["$ref"])
            node = self.schema(name)
            nullable = nullable or bool(node.get("nullable"))
        if "allOf" in node:
            merged: dict[str, Any] = {"type": "object", "required": []}
            for sub in node["allOf"]:
                _, s, n = self.resolve(sub)
                if "properties" in s:
                    merged.setdefault("properties", {}).update(s["properties"])
                if s.get("additionalProperties"):
                    merged["additionalProperties"] = True
                merged["required"] += s.get("required", [])
                nullable = nullable or n
            node = merged
        return name, node, nullable

    @staticmethod
    def nullable(schema: dict[str, Any], nullable: bool) -> dict[str, Any]:
        """Express nullability the OAS 3.1 / JSON Schema way."""
        if not nullable or schema == {}:
            return schema
        t = schema.get("type")
        if t is None or MARK in schema:
            return {"anyOf": [schema, {"type": "null"}]}
        if isinstance(t, list):
            return schema if "null" in t else {**schema, "type": t + ["null"]}
        return {**schema, "type": [t, "null"]}

    def leaf(self, node: dict[str, Any]) -> tuple[dict[str, Any], bool]:
        _, s, nullable = self.resolve(node)
        t = s.get("type")
        if t in SCALARS and "oneOf" not in s and "anyOf" not in s:
            out = {"type": t}
            if t == "integer" and s.get("format") in ("int32", "int64"):
                out["format"] = s["format"]
        else:
            out = {}
        return out, nullable

    def walk(self, node: dict[str, Any], read: dict[str, Any] | None, where: str) -> tuple[dict[str, Any], bool]:
        """Inline schema for a field glab-tui reads, with its nullable flag.

        Components come back inline, marked with x-component, and are named
        once every operation is walked."""
        if not read:
            return self.leaf(node)
        name, s, nullable = self.resolve(node)
        if name is not None and self.reads_by_component is not None:
            if self.is_collecting_reads:
                self.reads_by_component[name] = merge_reads(self.reads_by_component.get(name, {}), read)
            read = self.reads_by_component.get(name, read)
        for key in ("oneOf", "anyOf"):
            if key in s:
                outer, self.pending_branch_events = self.pending_branch_events, []
                outs = []
                for b in s[key]:
                    o, n = self.walk(b, read, where)
                    outs.append(self.nullable(o, n))
                events, self.pending_branch_events = self.pending_branch_events, outer
                if outer is None:
                    self.report(events)
                else:
                    outer.extend(events)
                out = {key: outs}
                if name is not None:
                    out[MARK] = name
                return out, nullable
        if s.get("type") == "array":
            items, inull = self.walk(s["items"], read, where)
            return {"type": "array", "items": self.nullable(items, inull)}, nullable
        props = s.get("properties", {})
        self.check_fields(read, set(props), where)
        out_props = {}
        for field in sorted(props):
            sub, sub_null = self.walk(props[field], read.get(field), f"{where}.{field}")
            if self.rule == "all_nullable":
                sub_null = True
            out_props[field] = self.nullable(sub, sub_null)
        obj: dict[str, Any] = {"type": "object"}
        if "properties" in s:
            obj["properties"] = out_props
            if not s.get("additionalProperties"):
                obj["additionalProperties"] = False
        required = sorted(set(s.get("required", [])) & set(read))
        if required:
            obj["required"] = required
        if name is not None:
            obj[MARK] = name
        return obj, nullable

    def check_fields(self, read: dict[str, Any], props: set[str], where: str) -> None:
        if not self.is_reporting_missing_fields:
            return
        ok = self.absent_ok_by_level.get(where, {})
        events = [("read", where, f, f in props) for f in sorted(read)]
        events += [("ok", where, f, f in props) for f in sorted(ok)]
        if self.pending_branch_events is None:
            self.report(events)
        else:
            self.pending_branch_events.extend(events)

    def report(self, events: list[Event]) -> None:
        """Problems for the checked fields; a field counts as in the spec when
        any event for it, one per union branch, found it."""
        found: dict[tuple[str, str, str], bool] = {}
        for kind, where, field, present in events:
            found[(kind, where, field)] = found.get((kind, where, field), False) or present
        for (kind, where, field), present in found.items():
            if kind == "read" and not present and field not in self.absent_ok_by_level.get(where, {}):
                self.problems.append(f"{self.host}: {self.site}: {where}.{field}: read by glab-tui, not in the spec")
            elif kind == "ok" and present:
                self.problems.append(
                    f"{self.host}: {self.site}: {where}.{field}: absent_ok is stale, the spec has the field now")

    def response_schemas(self, spec_op: dict[str, Any]) -> dict[str, dict[str, Any] | None]:
        """{code: JSON schema or None} for every 2xx response."""
        out = {}
        for code, resp in sorted(spec_op["responses"].items(), key=lambda kv: str(kv[0])):
            if not str(code).startswith("2"):
                continue
            if "$ref" in resp:
                resp = self.spec["components"]["responses"][resp["$ref"].rsplit("/", 1)[-1]]
            out[str(code)] = resp.get("content", {}).get("application/json", {}).get("schema")
        return out

    def operation(self, spec_op: dict[str, Any], op: dict[str, Any], site: str) -> dict[str, tuple[str, Any]]:
        """{code: schema} for the operation's 2xx responses, walked."""
        self.site = site
        self.absent_ok_by_level = {}
        for dotted, why in op.get("absent_ok", {}).items():
            parts = dotted.split(".")
            if self.is_reporting_missing_fields and not is_read(op["read"], parts):
                self.problems.append(
                    f"{self.host}: {site}: absent_ok {dotted} names a field glab-tui does not read")
            where = ".".join(["response"] + parts[:-1])
            self.absent_ok_by_level.setdefault(where, {})[parts[-1]] = why
        out: dict[str, tuple[str, Any]] = {}
        for code, schema in self.response_schemas(spec_op).items():
            if op.get("response_content_type"):
                if op["read"]:
                    self.problems.append(
                        f"{self.host}: {site}: response_content_type is set, so no field can be read")
                out[code] = ("text", op["response_content_type"])
            elif schema is None:
                if op["read"] and self.is_reporting_missing_fields:
                    self.problems.append(f"{self.host}: {site}: {code} has no JSON body to read")
                out[code] = ("none", code)
            else:
                s, n = self.walk(schema, op["read"], "response")
                out[code] = ("json", self.nullable(s, n))
        return out

    def report_missing_fields(self, listed: list[Listed]) -> None:
        """Walks every operation with its own read set, reporting read fields
        the spec lacks."""
        self.is_reporting_missing_fields = True
        for _, label, op, _, spec_op in listed:
            self.operation(spec_op, op, label)
        self.is_reporting_missing_fields = False

    def unite_component_reads(self, listed: list[Listed]) -> None:
        """Fills reads_by_component with the union of the read sets reaching
        each component, so later walks emit every component in one shape."""
        self.reads_by_component = {}
        self.is_collecting_reads = True
        while True:
            before = canon(self.reads_by_component)
            for _, label, op, _, spec_op in listed:
                self.operation(spec_op, op, label)
            if canon(self.reads_by_component) == before:
                break
        self.is_collecting_reads = False


def apply_corrections(host: str, spec: dict[str, Any], corrections: list[dict[str, str]], problems: list[str]) -> None:
    for c in corrections:
        label = f"{c['schema']}.{c['field']}: {c['type']} ({c['upstream']}, entity {c['entity']})"
        try:
            field = spec["components"]["schemas"][c["schema"]]["properties"][c["field"]]
        except KeyError:
            problems.append(f"{host}: correction {label} names a field the spec does not have")
            continue
        if field.get("type") == c["type"]:
            problems.append(f"{host}: correction {label} is stale, the spec already says {c['type']}")
            continue
        field["type"] = c["type"]
        field.pop("format", None)


def compact_param_schema(s: dict[str, Any]) -> dict[str, Any]:
    out: dict[str, Any] = {}
    if "type" in s:
        out["type"] = s["type"]
    if "enum" in s:
        out["enum"] = s["enum"]
    if "items" in s:
        out["items"] = compact_param_schema(s["items"])
    for key in ("oneOf", "anyOf"):
        if key in s:
            out[key] = [compact_param_schema(b) for b in s[key]]
    return out


def parameters(spec: dict[str, Any], path_item: dict[str, Any], spec_op: dict[str, Any]) -> list[dict[str, Any]]:
    """The operation's parameters, path-level ones overridden by its own."""
    found: dict[tuple[str, str], dict[str, Any]] = {}
    for p in path_item.get("parameters", []) + spec_op.get("parameters", []):
        if "$ref" in p:
            p = spec["components"]["parameters"][p["$ref"].rsplit("/", 1)[-1]]
        entry = {"in": p["in"], "name": p["name"], "schema": compact_param_schema(p.get("schema", {}))}
        if p.get("required"):
            entry["required"] = True
        found[(p["in"], p["name"])] = entry
    return [found[k] for k in sorted(found)]


def name_components(
        walked: dict[tuple[str, str], dict[str, tuple[str, Any]]], problems: list[str], host: str,
) -> tuple[dict[tuple[str, str], dict[str, Any]], dict[str, Any]]:
    """Turn the marked inline components into $refs. Returns (responses by
    operation, schemas by name)."""
    shapes: dict[str, set[str]] = {}

    def collect(node: Any) -> None:
        if isinstance(node, dict):
            if MARK in node:
                shapes.setdefault(node[MARK], set()).add(canon(node))
            for v in node.values():
                collect(v)
        elif isinstance(node, list):
            for v in node:
                collect(v)

    for codes in walked.values():
        for kind, schema in codes.values():
            if kind == "json":
                collect(schema)
    for name, variants in sorted(shapes.items()):
        if len(variants) > 1:
            problems.append(f"{host}: {name} is read with {len(variants)} different shapes")
    schemas: dict[str, Any] = {}

    def rewrite(node: Any) -> Any:
        if isinstance(node, list):
            return [rewrite(v) for v in node]
        if not isinstance(node, dict):
            return node
        if MARK in node:
            schemas[node[MARK]] = {k: rewrite(v) for k, v in node.items() if k != MARK}
            return {"$ref": f"#/components/schemas/{node[MARK]}"}
        return {k: rewrite(v) for k, v in node.items()}

    out: dict[tuple[str, str], dict[str, Any]] = {}
    for key, codes in walked.items():
        responses: dict[str, Any] = {}
        for code, (kind, payload) in codes.items():
            if kind == "json":
                responses[code] = {"description": "OK",
                                   "content": {"application/json": {"schema": rewrite(payload)}}}
            elif kind == "text":
                responses[code] = {"description": "OK",
                                   "content": {payload: {"schema": {"type": "string"}}}}
            else:
                responses[code] = {"description": "No content"}
                if code in ("204", "205"):
                    responses[code]["x-bodyless"] = True
        out[key] = responses
    return out, schemas


def header(host: str, kind: str, source_pin: dict[str, str], license_: dict[str, str] | None) -> tuple[str, dict[str, str]]:
    description = (f"The {kind} API of {host}, cut to what glab-tui uses. Generated by "
                   "scripts/gen-api-descriptions.py, do not edit.")
    if license_:
        description += " " + license_["notice"]
    pin = {k: source_pin[k] for k in ("repo", "commit", "path", "sha256")}
    return description, pin


def build_openapi(
        host: str, cfg: dict[str, Any], spec: dict[str, Any], source_pin: dict[str, str],
        license_: dict[str, str] | None = None,
) -> dict[str, Any]:
    spec = copy.deepcopy(spec)
    problems: list[str] = []
    apply_corrections(host, spec, cfg.get("corrections", []), problems)
    pruner = Pruner(host, spec, cfg["host_rule"])
    seen: set[tuple[str, str]] = set()
    listed: list[Listed] = []
    for op in cfg["operations"]:
        key = (op["method"], op["path"])
        label = f"{op['method'].upper()} {op['path']}"
        if key in seen:
            problems.append(f"{host}: {label} listed twice: merge the read sets and list every call site in code")
            continue
        seen.add(key)
        item = spec["paths"].get(op["path"], {})
        in_spec = op["method"] in item
        if "spec_absent_ok" in op:
            if in_spec:
                problems.append(f"{host}: {label}: spec_absent_ok is stale, the spec has the operation now")
            continue
        if not in_spec:
            problems.append(f"{host}: {label}: not in the spec")
            continue
        listed.append((key, label, op, item, item[op["method"]]))
    pruner.report_missing_fields(listed)
    pruner.unite_component_reads(listed)
    walked = {key: pruner.operation(spec_op, op, label) for key, label, op, item, spec_op in listed}
    problems = pruner.problems + problems
    responses, schemas = name_components(walked, problems, host)
    paths: dict[str, dict[str, Any]] = {}
    op_ids: dict[str, str] = {}
    for key, label, op, item, spec_op in listed:
        op_id = op_name(spec_op["operationId"])
        if op_id in op_ids:
            problems.append(f"{host}: {label} and {op_ids[op_id]} both name the operation {op_id}")
        op_ids[op_id] = label
        paths.setdefault(op["path"], {})[op["method"]] = {
            "operationId": op_id,
            "x-read-at": op["code"],
            "parameters": parameters(spec, item, spec_op),
            "responses": responses[key],
        }
    if problems:
        raise Problems(sorted(set(problems)))
    description, pin = header(host, "REST", source_pin, license_)
    info = {
        "title": f"glab-tui pruned {host} API",
        "version": str(spec["info"]["version"]),
        "description": description,
        "x-source": pin,
        "x-host-rule": cfg["host_rule"],
        "x-required-rule": "spec required list cut to the fields read",
        "x-schema-names": "union",
    }
    if license_:
        info["license"] = {"name": license_["name"], "url": license_["url"]}
    return {"openapi": "3.1.0", "info": info, "paths": paths, "components": {"schemas": schemas}}


BUILTIN_SCALARS = ("String", "Int", "Float", "Boolean", "ID")
# What GraphQL's @deprecated directive gives as the reason when it names none.
DEFAULT_DEPRECATION_REASON = "No longer supported"


def deprecation_reason(reason: object) -> str:
    """The reason a schema gives for a deprecation, the default without one."""
    return reason if isinstance(reason, str) and reason else DEFAULT_DEPRECATION_REASON

TOKEN = re.compile(r'''
    (?P<ws>[\s,﻿]+) | (?P<comment>\#[^\n]*)
  | (?P<block>"""(?:\\"""|.)*?""") | (?P<string>"(?:\\.|[^"\\\n])*")
  | (?P<spread>\.\.\.) | (?P<punct>[!$&()\[\]{}:=@|])
  | (?P<name>[_A-Za-z][_0-9A-Za-z]*) | (?P<number>-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)
''', re.X | re.S)


def tokenize(text: str) -> list[tuple[str, str]]:
    pos = 0
    out: list[tuple[str, str]] = []
    while pos < len(text):
        m = TOKEN.match(text, pos)
        if m is None:
            raise Problems([f"GraphQL SDL: cannot read {text[pos:pos + 40]!r} at offset {pos}"])
        pos = m.end()
        if m.lastgroup not in ("ws", "comment"):
            out.append((m.lastgroup, m.group()))
    return out


class SdlParser:
    """Reads the type definitions of a GraphQL SDL file into a field table."""

    KINDS = {"type": "object", "interface": "interface", "input": "input",
             "enum": "enum", "union": "union", "scalar": "scalar"}

    def __init__(self, text: str) -> None:
        self.toks = tokenize(text)
        self.i = 0

    def peek(self, offset: int = 0) -> tuple[str | None, str | None]:
        j = self.i + offset
        return self.toks[j] if j < len(self.toks) else (None, None)

    def take(self) -> tuple[str, str]:
        tok = self.toks[self.i]
        self.i += 1
        return tok

    def at(self, text: str) -> bool:
        return self.peek()[1] == text and self.peek()[0] in ("punct", "name", "spread")

    def expect(self, text: str) -> None:
        if not self.at(text):
            raise Problems([f"GraphQL SDL: expected {text!r}, found {self.peek()[1]!r} (token {self.i})"])
        self.i += 1

    def name(self) -> str:
        kind, text = self.take()
        if kind != "name":
            raise Problems([f"GraphQL SDL: expected a name, found {text!r} (token {self.i - 1})"])
        return text

    def description(self) -> None:
        if self.peek()[0] in ("string", "block"):
            self.i += 1

    def type_ref(self) -> str:
        if self.at("["):
            self.i += 1
            named = self.type_ref()
            self.expect("]")
        else:
            named = self.name()
        if self.at("!"):
            self.i += 1
        return named

    def value(self) -> str | None:
        kind, text = self.take()
        if text == "$":
            self.name()
        elif text == "[":
            while not self.at("]"):
                self.value()
            self.i += 1
        elif text == "{":
            while not self.at("}"):
                self.name()
                self.expect(":")
                self.value()
            self.i += 1
        elif kind == "string":
            return json.loads(text)
        elif kind == "block":
            return text[3:-3]
        return None

    def directives(self) -> dict[str, dict[str, str | None]]:
        found: dict[str, dict[str, str | None]] = {}
        while self.at("@"):
            self.i += 1
            name = self.name()
            args: dict[str, str | None] = {}
            if self.at("("):
                self.i += 1
                while not self.at(")"):
                    arg = self.name()
                    self.expect(":")
                    args[arg] = self.value()
                self.i += 1
            found[name] = args
        return found

    def input_values(self, close: str) -> tuple[list[str], dict[str, str]]:
        """Argument or input-field definitions up to the closing bracket;
        returns their names and the deprecation reason of each deprecated one."""
        names: list[str] = []
        deprecated: dict[str, str] = {}
        while not self.at(close):
            self.description()
            name = self.name()
            names.append(name)
            self.expect(":")
            self.type_ref()
            if self.at("="):
                self.i += 1
                self.value()
            directives = self.directives()
            if "deprecated" in directives:
                deprecated[name] = deprecation_reason(directives["deprecated"].get("reason"))
        self.i += 1
        return names, deprecated

    def field_definitions(self) -> dict[str, dict[str, Any]]:
        fields: dict[str, dict[str, Any]] = {}
        self.expect("{")
        while not self.at("}"):
            self.description()
            name = self.name()
            args: list[str] = []
            deprecated_args: dict[str, str] = {}
            if self.at("("):
                self.i += 1
                args, deprecated_args = self.input_values(")")
            self.expect(":")
            ty = self.type_ref()
            directives = self.directives()
            entry: dict[str, Any] = {"type": ty}
            if args:
                entry["args"] = args
            if deprecated_args:
                entry["deprecated_args"] = deprecated_args
            if "deprecated" in directives:
                entry["deprecated"] = deprecation_reason(directives["deprecated"].get("reason"))
            fields[name] = entry
        self.i += 1
        return fields

    def parse(self) -> tuple[str | None, str | None, dict[str, dict[str, Any]]]:
        types: dict[str, dict[str, Any]] = {}
        roots: dict[str, str | None] = {"query": "Query", "mutation": None}
        explicit_schema = False
        while self.i < len(self.toks):
            self.description()
            keyword = self.name()
            if keyword == "extend":
                raise Problems(["GraphQL SDL: type extensions are not handled"])
            if keyword == "schema":
                explicit_schema = True
                self.directives()
                self.expect("{")
                roots["mutation"] = None
                while not self.at("}"):
                    op = self.name()
                    self.expect(":")
                    roots[op] = self.name()
                self.i += 1
            elif keyword == "directive":
                self.expect("@")
                self.name()
                if self.at("("):
                    self.i += 1
                    self.input_values(")")
                if self.peek()[1] == "repeatable":
                    self.i += 1
                self.expect("on")
                if self.at("|"):
                    self.i += 1
                self.name()
                while self.at("|"):
                    self.i += 1
                    self.name()
            elif keyword in self.KINDS:
                name = self.name()
                entry: dict[str, Any] = {"kind": self.KINDS[keyword]}
                if keyword in ("type", "interface") and self.peek()[1] == "implements":
                    self.i += 1
                    if self.at("&"):
                        self.i += 1
                    self.name()
                    while self.at("&"):
                        self.i += 1
                        self.name()
                self.directives()
                if keyword in ("type", "interface"):
                    if self.at("{"):
                        fields = self.field_definitions()
                        if fields:
                            entry["fields"] = fields
                elif keyword == "input":
                    self.expect("{")
                    self.input_values("}")
                elif keyword == "enum":
                    self.expect("{")
                    while not self.at("}"):
                        self.description()
                        self.name()
                        self.directives()
                    self.i += 1
                elif keyword == "union":
                    self.expect("=")
                    if self.at("|"):
                        self.i += 1
                    self.name()
                    while self.at("|"):
                        self.i += 1
                        self.name()
                types[name] = entry
            else:
                raise Problems([f"GraphQL SDL: unexpected {keyword!r} (token {self.i - 1})"])
        if not explicit_schema and "Mutation" in types:
            roots["mutation"] = "Mutation"
        return roots["query"], roots["mutation"], types


def table_from_sdl(text: str) -> dict[str, Any]:
    query, mutation, types = SdlParser(text).parse()
    for builtin in BUILTIN_SCALARS:
        types.setdefault(builtin, {"kind": "scalar"})
    return {"query": query, "mutation": mutation, "types": types}


INTROSPECTION_KINDS = {"OBJECT": "object", "INTERFACE": "interface", "UNION": "union",
                       "SCALAR": "scalar", "ENUM": "enum", "INPUT_OBJECT": "input"}


def named_type(ref: dict[str, Any]) -> str:
    while ref.get("ofType"):
        ref = ref["ofType"]
    return ref["name"]


def table_from_introspection(doc: dict[str, Any]) -> dict[str, Any]:
    schema = doc["data"]["__schema"]
    types: dict[str, dict[str, Any]] = {}
    for t in schema["types"]:
        entry: dict[str, Any] = {"kind": INTROSPECTION_KINDS[t["kind"]]}
        fields: dict[str, dict[str, Any]] = {}
        for f in t.get("fields") or []:
            field: dict[str, Any] = {"type": named_type(f["type"])}
            if f["args"]:
                field["args"] = [a["name"] for a in f["args"]]
            deprecated_args = {a["name"]: deprecation_reason(a.get("deprecationReason"))
                               for a in f["args"] if a.get("isDeprecated")}
            if deprecated_args:
                field["deprecated_args"] = deprecated_args
            if f.get("isDeprecated"):
                field["deprecated"] = deprecation_reason(f.get("deprecationReason"))
            fields[f["name"]] = field
        if fields:
            entry["fields"] = fields
        types[t["name"]] = entry
    mutation = (schema.get("mutationType") or {}).get("name")
    return {"query": schema["queryType"]["name"], "mutation": mutation, "types": types}


def build_graphql(
        host: str, cfg: dict[str, Any], table: dict[str, Any], source_pin: dict[str, str],
        license_: dict[str, str] | None = None,
) -> dict[str, Any]:
    """The table cut to the fields listed in cfg["graphql"] (a nested dict
    from the query root, like an operation's read set)."""
    types = table["types"]
    problems: list[str] = []
    used: dict[str, dict[str, Any]] = {}  # type name -> {field: definition}
    kinds: dict[str, str] = {}  # type name -> kind

    def walk(type_name: str, read: dict[str, Any], path: list[str]) -> None:
        kinds[type_name] = types[type_name]["kind"]
        for field in sorted(read):
            here = ".".join(path + [field])
            definition = types[type_name].get("fields", {}).get(field)
            if definition is None:
                problems.append(f"{host}: {type_name}.{field}: listed as selected ({here}), not in the schema")
                continue
            used.setdefault(type_name, {})[field] = definition
            target = definition["type"]
            kinds[target] = types[target]["kind"]
            if read[field]:
                if kinds[target] in ("object", "interface"):
                    walk(target, read[field], path + [field])
                else:
                    problems.append(f"{host}: {here}: fields are listed below {target}, of kind {kinds[target]}")

    walk(table["query"], cfg.get("graphql", {}), [])
    if problems:
        raise Problems(sorted(set(problems)))
    out_types: dict[str, Any] = {}
    for name in sorted(kinds):
        entry: dict[str, Any] = {"kind": kinds[name]}
        if name in used:
            entry["fields"] = {f: used[name][f] for f in sorted(used[name])}
        out_types[name] = entry
    description, pin = header(host, "GraphQL", source_pin, license_)
    doc = {"description": description, "source": pin, "query": table["query"],
           "mutation": table["mutation"], "types": out_types}
    if license_:
        doc["license"] = {"name": license_["name"], "url": license_["url"]}
    return doc


def sha256_of(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def fetch(host: str, kind: str, pin: dict[str, str], spec_dir: str) -> str:
    """The pinned file in spec_dir, downloaded when missing, checked either way."""
    ext = os.path.splitext(pin["path"])[1]
    path = os.path.join(spec_dir, f"{host}-{kind}-{pin['sha256']}{ext}")
    if not os.path.exists(path):
        os.makedirs(spec_dir, exist_ok=True)
        partial = path + ".partial"
        result = subprocess.run(
            ["curl", "-fsSL", "--retry", "3", "--max-time", str(DOWNLOAD_TIMEOUT_SECONDS),
             "-o", partial, pin["url"]],
            capture_output=True, text=True)
        if result.returncode != 0:
            raise Problems([f"{host}: download of {pin['url']} failed: {result.stderr.strip()}"])
        os.replace(partial, path)
    digest = sha256_of(path)
    if digest != pin["sha256"]:
        raise Problems([
            f"{os.path.basename(path)}: sha256 is {digest}, but {pin['sha256']} is pinned for "
            f"{pin['url']} (delete the file to download it again)"])
    return path


def read_text(path: str) -> str:
    with open(path, encoding="utf-8") as f:
        return f.read()


def load_spec(path: str) -> Any:
    if path.endswith((".yaml", ".yml")):
        import yaml
        with open(path, "rb") as f:
            return yaml.load(f, Loader=getattr(yaml, "CSafeLoader", yaml.SafeLoader))
    with open(path, "rb") as f:
        return json.load(f)


def generate(host: str, pins: dict[str, Any], inputs_dir: str, spec_dir: str) -> dict[str, str]:
    """{file name: text} for one host."""
    with open(os.path.join(inputs_dir, f"{host}.json")) as f:
        cfg = json.load(f)
    rest = load_spec(fetch(host, "rest", pins["rest"], spec_dir))
    graphql_path = fetch(host, "graphql", pins["graphql"], spec_dir)
    if graphql_path.endswith(".json"):
        table = table_from_introspection(load_spec(graphql_path))
    else:
        table = table_from_sdl(read_text(graphql_path))
    licenses = pins["license"]
    problems: list[str] = []
    files: dict[str, str] = {}
    build: Callable[..., dict[str, Any]]
    for name, build, args in (
        (f"{host}.openapi.json", build_openapi, (rest, pins["rest"], licenses and licenses["rest"])),
        (f"{host}.graphql.json", build_graphql, (table, pins["graphql"], licenses and licenses["graphql"])),
    ):
        try:
            files[name] = render(build(host, cfg, *args))
        except Problems as e:
            problems += e.items
    if problems:
        raise Problems(problems)
    return files


def run(argv: list[str], sources: dict[str, dict[str, Any]]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", action="store_true",
                        help="write nothing, exit 1 when a generated file is not current")
    parser.add_argument("--spec-dir", default=os.path.join(ROOT, "target", "api-descriptions"))
    parser.add_argument("--out", default=os.path.join(ROOT, "tests", "api-descriptions"))
    parser.add_argument("--inputs", default=os.path.join(ROOT, "scripts", "api-descriptions"))
    args = parser.parse_args(argv)
    files: dict[str, str] = {}
    problems: list[str] = []
    for host in sorted(sources):
        try:
            files.update(generate(host, sources[host], args.inputs, args.spec_dir))
        except Problems as e:
            problems += e.items
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 1
    if args.check:
        stale: list[str] = []
        for name, text in sorted(files.items()):
            path = os.path.join(args.out, name)
            if not os.path.exists(path):
                stale.append(f"{name}: missing")
            elif read_text(path) != text:
                stale.append(f"{name}: differs from what the pins and inputs generate")
        if stale:
            print("\n".join(stale), file=sys.stderr)
            print("run scripts/gen-api-descriptions.py and commit the result", file=sys.stderr)
            return 1
        return 0
    os.makedirs(args.out, exist_ok=True)
    for name, text in sorted(files.items()):
        with open(os.path.join(args.out, name), "w", encoding="utf-8") as f:
            f.write(text)
        print(f"wrote {os.path.join(args.out, name)}")
    return 0


if __name__ == "__main__":
    sys.exit(run(sys.argv[1:], SOURCES))
