"""Tests for gen-api-descriptions.py, on small synthetic specs (no network).

Run: python3 -m unittest discover -s scripts -p 'test_*.py'
"""
import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest
from typing import Any

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location(
    "gen_api_descriptions", os.path.join(HERE, "gen-api-descriptions.py"))
gen = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gen)

SOURCE = {
    "repo": "acme/api", "commit": "c" * 40, "path": "spec.json",
    "sha256": "0" * 64, "url": "https://example.invalid/spec.json",
}


def json_body(schema: dict[str, Any]) -> dict[str, Any]:
    return {"content": {"application/json": {"schema": schema}}}


def mini_spec() -> dict[str, Any]:
    return copy.deepcopy({
        "info": {"version": "1.0"},
        "paths": {
            "/items": {
                "get": {
                    "operationId": "items/list",
                    "parameters": [
                        {"$ref": "#/components/parameters/page"},
                        {"name": "state", "in": "query", "schema": {"type": "string", "enum": ["open", "closed"]}},
                    ],
                    "responses": {"200": json_body({"type": "array", "items": {"$ref": "#/components/schemas/Item"}})},
                },
                "post": {
                    "operationId": "items/create",
                    "requestBody": json_body({"$ref": "#/components/schemas/NewItem"}),
                    "responses": {
                        "201": json_body({"$ref": "#/components/schemas/Item"}),
                        "202": {"description": "accepted"},
                    },
                },
            },
            "/items/{id}": {
                "parameters": [{"name": "id", "in": "path", "required": True, "schema": {"type": "integer"}}],
                "get": {
                    "operationId": "items/get",
                    "responses": {"200": json_body({"$ref": "#/components/schemas/Item"})},
                },
                "delete": {"operationId": "items/delete", "responses": {"204": {"description": "gone"}}},
            },
            "/trace": {
                "get": {
                    "operationId": "getTrace",
                    "responses": {"200": json_body({"$ref": "#/components/schemas/Item"})},
                },
            },
            "/either": {
                "get": {
                    "operationId": "either",
                    "responses": {"200": json_body({"oneOf": [
                        {"$ref": "#/components/schemas/Item"},
                        {"$ref": "#/components/schemas/Owner"},
                    ]})},
                },
            },
            "/mixed": {
                "get": {
                    "operationId": "mixed",
                    "responses": {"200": json_body({"oneOf": [
                        {"type": "array", "items": {"$ref": "#/components/schemas/Item"}},
                        {"$ref": "#/components/schemas/Owner"},
                    ]})},
                },
            },
        },
        "components": {
            "parameters": {"page": {"name": "page", "in": "query", "schema": {"type": "integer"}}},
            "schemas": {
                "Item": {
                    "type": "object",
                    "required": ["id", "name"],
                    "properties": {
                        "id": {"type": "integer", "format": "int64"},
                        "name": {"type": "string"},
                        "owner": {"$ref": "#/components/schemas/Owner"},
                        "note": {"type": "string", "nullable": True},
                        "extra": {"type": "string"},
                    },
                },
                "NewItem": {
                    "required": ["name"],
                    "allOf": [
                        {"type": "object", "properties": {"name": {"type": "string", "description": "Its name."}}},
                        {"type": "object", "properties": {
                            "tags": {"type": "array", "items": {"type": "string"}},
                            "owner": {"$ref": "#/components/schemas/Owner"},
                            "note": {"type": "string", "nullable": True},
                            "meta": {"type": "object", "additionalProperties": {"type": "string"}},
                        }},
                    ],
                },
                "Owner": {
                    "type": "object",
                    "required": ["login"],
                    "properties": {"login": {"type": "string"}, "group_id": {"type": "string", "format": "uuid"}},
                },
            },
        },
    })


def mini_cfg(**over: Any) -> dict[str, Any]:
    cfg: dict[str, Any] = {
        "host_rule": "none",
        "corrections": [],
        "operations": [
            {"method": "get", "path": "/items", "code": ["a.rs:1"],
             "read": {"id": {}, "name": {}, "owner": {"login": {}}}},
            {"method": "post", "path": "/items", "code": ["a.rs:2"], "read": {}},
            {"method": "get", "path": "/items/{id}", "code": ["a.rs:3", "a.rs:4"], "read": {"id": {}}},
            {"method": "delete", "path": "/items/{id}", "code": ["a.rs:5"], "read": {}},
            {"method": "get", "path": "/trace", "code": ["a.rs:6"], "read": {},
             "response_content_type": "text/plain"},
        ],
    }
    cfg.update(over)
    return cfg


def build(cfg: dict[str, Any] | None = None, spec: dict[str, Any] | None = None, host: str = "acme") -> dict[str, Any]:
    return gen.build_openapi(host, cfg or mini_cfg(), spec or mini_spec(), SOURCE)


def problems_of(cfg: dict[str, Any] | None = None, spec: dict[str, Any] | None = None) -> list[str]:
    try:
        build(cfg, spec)
    except gen.Problems as e:
        return e.items
    return []


class RestTests(unittest.TestCase):
    def test_two_runs_are_byte_identical(self) -> None:
        a = gen.render(build())
        b = gen.render(build())
        self.assertEqual(a, b)
        self.assertTrue(a.endswith("\n"))

    def test_every_2xx_response_is_kept(self) -> None:
        op = build()["paths"]["/items"]["post"]
        self.assertEqual(sorted(op["responses"]), ["201", "202"])

    def test_204_is_marked_bodyless(self) -> None:
        resp = build()["paths"]["/items/{id}"]["delete"]["responses"]["204"]
        self.assertTrue(resp["x-bodyless"])
        self.assertNotIn("content", resp)

    def test_content_type_override_replaces_the_json_schema(self) -> None:
        resp = build()["paths"]["/trace"]["get"]["responses"]["200"]
        self.assertEqual(list(resp["content"]), ["text/plain"])

    def test_override_with_a_read_set_is_a_problem(self) -> None:
        cfg = mini_cfg()
        cfg["operations"][4]["read"] = {"id": {}}
        self.assertEqual(problems_of(cfg), [
            "acme: GET /trace: response_content_type is set, so no field can be read"])

    def test_operation_id_is_sanitized(self) -> None:
        doc = build()
        self.assertEqual(doc["paths"]["/items"]["get"]["operationId"], "ItemsList")
        self.assertEqual(doc["paths"]["/trace"]["get"]["operationId"], "GetTrace")

    def test_call_sites_are_listed_per_operation(self) -> None:
        doc = build()
        self.assertEqual(doc["paths"]["/items/{id}"]["get"]["x-read-at"], ["a.rs:3", "a.rs:4"])

    def test_parameters_include_referenced_and_path_level_ones(self) -> None:
        doc = build()
        names = [(p["in"], p["name"]) for p in doc["paths"]["/items"]["get"]["parameters"]]
        self.assertEqual(names, [("query", "page"), ("query", "state")])
        state = doc["paths"]["/items"]["get"]["parameters"][1]
        self.assertEqual(state["schema"], {"type": "string", "enum": ["open", "closed"]})
        path_params = doc["paths"]["/items/{id}"]["get"]["parameters"]
        self.assertEqual(path_params, [{"in": "path", "name": "id", "required": True, "schema": {"type": "integer"}}])

    def test_closed_world_keeps_unread_spec_properties(self) -> None:
        item = build()["components"]["schemas"]["Item"]
        self.assertEqual(sorted(item["properties"]), ["extra", "id", "name", "note", "owner"])
        self.assertIs(item["additionalProperties"], False)

    def test_required_is_spec_required_intersected_with_read(self) -> None:
        item = build()["components"]["schemas"]["Item"]
        self.assertEqual(item["required"], ["id", "name"])
        cfg = mini_cfg()
        cfg["operations"][0]["read"] = {"id": {}}
        cfg["operations"][2]["read"] = {"id": {}}
        self.assertEqual(build(cfg)["components"]["schemas"]["Item"]["required"], ["id"])

    def test_nullable_is_expressed_in_json_schema_terms(self) -> None:
        item = build()["components"]["schemas"]["Item"]
        self.assertEqual(item["properties"]["note"]["type"], ["string", "null"])

    def test_all_nullable_rule(self) -> None:
        item = build(mini_cfg(host_rule="all_nullable"))["components"]["schemas"]["Item"]
        self.assertEqual(item["properties"]["id"]["type"], ["integer", "null"])

    def test_union_names_keep_the_spec_name_when_read_depth_differs(self) -> None:
        cfg = mini_cfg()
        cfg["operations"][2]["read"] = {"id": {}, "owner": {"login": {}, "group_id": {}}}
        doc = build(cfg)
        self.assertEqual(sorted(doc["components"]["schemas"]), ["Item", "Owner"])
        item = doc["paths"]["/items/{id}"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]
        self.assertEqual(item, {"$ref": "#/components/schemas/Item"})

    def test_one_of_branches_are_walked_each(self) -> None:
        cfg = mini_cfg()
        cfg["operations"].append({"method": "get", "path": "/either", "code": ["a.rs:7"], "read": {"login": {}}})
        branches = build(cfg)["paths"]["/either"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["oneOf"]
        self.assertEqual(len(branches), 2)

    def test_a_field_missing_below_a_union_is_an_error(self) -> None:
        cfg = mini_cfg()
        cfg["operations"].append(
            {"method": "get", "path": "/either", "code": ["a.rs:7"], "read": {"owner": {"nope": {}}}})
        self.assertEqual(problems_of(cfg), [
            "acme: GET /either: response.owner.nope: read by glab-tui, not in the spec"])

    def test_absent_ok_below_a_union_goes_stale_once_a_branch_has_the_field(self) -> None:
        cfg = mini_cfg()
        cfg["operations"].append({"method": "get", "path": "/either", "code": ["a.rs:7"],
                                  "read": {"login": {}}, "absent_ok": {"login": "not documented"}})
        self.assertEqual(problems_of(cfg), [
            "acme: GET /either: response.login: absent_ok is stale, the spec has the field now"])

    def test_a_field_one_union_branch_has_is_accepted(self) -> None:
        cfg = mini_cfg()
        cfg["operations"].append(
            {"method": "get", "path": "/either", "code": ["a.rs:7"], "read": {"owner": {"login": {}}, "login": {}}})
        self.assertEqual(problems_of(cfg), [])

    def test_reads_through_an_array_branch_are_checked_against_its_items(self) -> None:
        cfg = mini_cfg()
        op = {"method": "get", "path": "/mixed", "code": ["a.rs:7"], "read": {"name": {}}}
        cfg["operations"].append(op)
        self.assertEqual(problems_of(cfg), [])
        op["read"] = {"nope": {}}
        self.assertEqual(problems_of(cfg), [
            "acme: GET /mixed: response.nope: read by glab-tui, not in the spec"])

    def test_a_correction_the_spec_already_has_fails_and_is_named(self) -> None:
        spec = mini_spec()
        cfg = mini_cfg(corrections=[{
            "schema": "Owner", "field": "group_id", "type": "string",
            "upstream": "acme/api#1", "entity": "Owner"}])
        self.assertEqual(problems_of(cfg, spec), [
            "acme: correction Owner.group_id: string (acme/api#1, entity Owner) is stale, the spec already says string"])

    def test_a_correction_changes_the_type_and_drops_the_format(self) -> None:
        cfg = mini_cfg(corrections=[{
            "schema": "Owner", "field": "group_id", "type": "integer",
            "upstream": "acme/api#1", "entity": "Owner"}])
        cfg["operations"][0]["read"]["owner"]["group_id"] = {}
        owner = build(cfg)["components"]["schemas"]["Owner"]
        self.assertEqual(owner["properties"]["group_id"], {"type": "integer"})

    def test_a_correction_for_a_missing_field_fails(self) -> None:
        cfg = mini_cfg(corrections=[{
            "schema": "Owner", "field": "nope", "type": "integer",
            "upstream": "acme/api#1", "entity": "Owner"}])
        self.assertEqual(problems_of(cfg), [
            "acme: correction Owner.nope: integer (acme/api#1, entity Owner) names a field the spec does not have"])

    def test_the_same_method_and_path_twice_is_an_error(self) -> None:
        cfg = mini_cfg()
        cfg["operations"].append(dict(cfg["operations"][0]))
        self.assertEqual(problems_of(cfg), [
            "acme: GET /items listed twice: merge the read sets and list every call site in code"])

    def test_an_operation_the_spec_lacks_is_an_error(self) -> None:
        cfg = mini_cfg()
        cfg["operations"].append({"method": "get", "path": "/nope", "code": ["a.rs:9"], "read": {}})
        self.assertEqual(problems_of(cfg), ["acme: GET /nope: not in the spec"])

    def test_spec_absent_ok_accepts_a_missing_operation_and_goes_stale(self) -> None:
        cfg = mini_cfg()
        cfg["operations"].append({"method": "get", "path": "/nope", "code": ["a.rs:9"], "read": {},
                                  "spec_absent_ok": "not documented"})
        self.assertNotIn("/nope", build(cfg)["paths"])
        cfg["operations"][-1]["path"] = "/either"
        self.assertEqual(problems_of(cfg), [
            "acme: GET /either: spec_absent_ok is stale, the spec has the operation now"])

    def test_a_read_field_the_spec_lacks_is_an_error(self) -> None:
        cfg = mini_cfg()
        cfg["operations"][0]["read"]["owner"]["email"] = {}
        self.assertEqual(problems_of(cfg), [
            "acme: GET /items: response.owner.email: read by glab-tui, not in the spec"])

    def test_absent_ok_accepts_the_field_and_goes_stale(self) -> None:
        cfg = mini_cfg()
        cfg["operations"][0]["read"]["cache"] = {}
        cfg["operations"][0]["absent_ok"] = {"cache": "cache-only field"}
        build(cfg)
        cfg["operations"][0]["read"]["extra"] = {}
        cfg["operations"][0]["absent_ok"]["extra"] = "wrong"
        self.assertEqual(problems_of(cfg), [
            "acme: GET /items: response.extra: absent_ok is stale, the spec has the field now"])

    def test_absent_ok_for_a_field_not_read_is_an_error(self) -> None:
        cfg = mini_cfg()
        cfg["operations"][0]["absent_ok"] = {"bogus": "x", "owner.bogus": "y"}
        self.assertEqual(problems_of(cfg), [
            "acme: GET /items: absent_ok bogus names a field glab-tui does not read",
            "acme: GET /items: absent_ok owner.bogus names a field glab-tui does not read",
        ])

    def test_a_read_set_on_a_bodyless_response_is_an_error(self) -> None:
        cfg = mini_cfg()
        cfg["operations"][3]["read"] = {"id": {}}
        self.assertEqual(problems_of(cfg), ["acme: DELETE /items/{id}: 204 has no JSON body to read"])

    def test_every_problem_is_reported_not_only_the_first(self) -> None:
        cfg = mini_cfg()
        cfg["operations"][0]["read"]["zzz"] = {}
        cfg["operations"].append({"method": "get", "path": "/nope", "code": ["a.rs:9"], "read": {}})
        self.assertEqual(problems_of(cfg), [
            "acme: GET /items: response.zzz: read by glab-tui, not in the spec",
            "acme: GET /nope: not in the spec",
        ])

    def test_the_source_pin_is_part_of_the_output(self) -> None:
        a = gen.build_openapi("acme", mini_cfg(), mini_spec(), SOURCE)
        moved = dict(SOURCE, commit="d" * 40)
        b = gen.build_openapi("acme", mini_cfg(), mini_spec(), moved)
        self.assertNotEqual(gen.render(a), gen.render(b))
        self.assertEqual(a["info"]["x-source"]["commit"], "c" * 40)
        self.assertEqual(a["info"]["x-source"]["sha256"], "0" * 64)

    def test_a_license_notice_goes_into_the_header(self) -> None:
        lic = {"name": "CC BY-SA 4.0", "url": "https://example.invalid/l", "notice": "Excerpt of acme/api."}
        doc = gen.build_openapi("acme", mini_cfg(), mini_spec(), SOURCE, lic)
        self.assertEqual(doc["info"]["license"], {"name": "CC BY-SA 4.0", "url": "https://example.invalid/l"})
        self.assertIn("Excerpt of acme/api.", doc["info"]["description"])
        self.assertNotIn("license", build()["info"])

    def test_an_object_the_description_gives_no_properties_stays_open(self) -> None:
        spec = mini_spec()
        spec["components"]["schemas"]["Item"]["properties"]["target"] = {"type": "object"}
        cfg = mini_cfg()
        cfg["operations"][0]["read"]["target"] = {"title": {}}
        cfg["operations"][0]["absent_ok"] = {"target.title": "the description gives target no fields"}
        target = build(cfg, spec)["components"]["schemas"]["Item"]["properties"]["target"]
        self.assertEqual(target, {"type": "object"})

    def test_an_all_of_of_free_form_parts_stays_open(self) -> None:
        spec = mini_spec()
        spec["components"]["schemas"]["Free"] = {"type": "object"}
        spec["components"]["schemas"]["Item"]["properties"]["target"] = {
            "allOf": [{"$ref": "#/components/schemas/Free"}]}
        cfg = mini_cfg()
        cfg["operations"][0]["read"]["target"] = {"title": {}}
        cfg["operations"][0]["absent_ok"] = {"target.title": "the description gives target no fields"}
        target = build(cfg, spec)["components"]["schemas"]["Item"]["properties"]["target"]
        self.assertEqual(target, {"type": "object"})

    def test_an_object_that_allows_other_properties_stays_open(self) -> None:
        spec = mini_spec()
        spec["components"]["schemas"]["Item"]["properties"]["target"] = {
            "type": "object", "properties": {"title": {"type": "string"}}, "additionalProperties": True}
        cfg = mini_cfg()
        cfg["operations"][0]["read"]["target"] = {"title": {}}
        target = build(cfg, spec)["components"]["schemas"]["Item"]["properties"]["target"]
        self.assertEqual(target, {"type": "object", "properties": {"title": {"type": "string"}}})

    def test_an_object_with_an_empty_property_list_stays_closed(self) -> None:
        spec = mini_spec()
        spec["components"]["schemas"]["Item"]["properties"]["target"] = {"type": "object", "properties": {}}
        cfg = mini_cfg()
        cfg["operations"][0]["read"]["target"] = {"title": {}}
        cfg["operations"][0]["absent_ok"] = {"target.title": "the description gives target no fields"}
        target = build(cfg, spec)["components"]["schemas"]["Item"]["properties"]["target"]
        self.assertEqual(target, {"type": "object", "properties": {}, "additionalProperties": False})

    def test_a_request_body_is_kept_whole_and_inline(self) -> None:
        body = build()["paths"]["/items"]["post"]["requestBody"]
        self.assertEqual(body, json_body({
            "type": "object",
            "properties": {
                "meta": {"type": "object", "additionalProperties": {"type": "string"}},
                "name": {"type": "string"},
                "note": {"type": ["string", "null"]},
                "owner": {"type": "object", "properties": {
                    "group_id": {"type": "string"}, "login": {"type": "string"}},
                    "additionalProperties": False},
                "tags": {"type": "array", "items": {"type": "string"}},
            },
            "additionalProperties": False,
        }))

    def test_a_request_body_branch_carries_the_properties_beside_it(self) -> None:
        spec = mini_spec()
        spec["paths"]["/items"]["post"]["requestBody"] = json_body({
            "type": "object",
            "properties": {"reviewers": {"type": "array", "items": {"type": "string"}}},
            "anyOf": [{"required": ["reviewers"]}, {"required": ["team_reviewers"]}],
        })
        body = build(spec=spec)["paths"]["/items"]["post"]["requestBody"]["content"]["application/json"]["schema"]
        branch = {"type": "object", "properties": {"reviewers": {"type": "array", "items": {"type": "string"}}},
                  "additionalProperties": False}
        self.assertEqual(body["anyOf"], [branch, branch])

    def test_a_request_body_branch_that_refers_to_itself_stops_at_the_loop(self) -> None:
        spec = mini_spec()
        spec["components"]["schemas"]["Body"] = {
            "properties": {"a": {"type": "string"}}, "oneOf": [{"$ref": "#/components/schemas/Body"}]}
        spec["paths"]["/items"]["post"]["requestBody"] = json_body({"$ref": "#/components/schemas/Body"})
        body = gen.request_body(spec, spec["paths"]["/items"]["post"])
        self.assertEqual(body["content"]["application/json"]["schema"]["oneOf"], [{}])

    def test_a_request_body_merge_leaves_the_description_untouched(self) -> None:
        spec = mini_spec()
        spec["components"]["schemas"]["Base"] = {"properties": {"a": {"type": "string"}}}
        spec["components"]["schemas"]["Body"] = {
            "properties": {"b": {"type": "string"}}, "allOf": [{"$ref": "#/components/schemas/Base"}]}
        spec["paths"]["/items"]["post"]["requestBody"] = json_body({"$ref": "#/components/schemas/Body"})
        gen.request_body(spec, spec["paths"]["/items"]["post"])
        self.assertEqual(spec["components"]["schemas"]["Body"]["properties"], {"b": {"type": "string"}})

    def test_an_operation_without_a_request_body_gets_none(self) -> None:
        self.assertNotIn("requestBody", build()["paths"]["/items/{id}"]["delete"])

    def test_a_request_body_that_refers_to_itself_stops_at_the_loop(self) -> None:
        spec = mini_spec()
        spec["components"]["schemas"]["Owner"]["properties"]["parent"] = {"$ref": "#/components/schemas/Owner"}
        owner = build(spec=spec)["paths"]["/items"]["post"]["requestBody"]["content"]["application/json"][
            "schema"]["properties"]["owner"]
        self.assertEqual(owner["properties"]["parent"], {})



SDL = '''
"""The schema."""
schema { query: Query }

"A repo."
type Query {
  "Look up"
  repository(owner: String!, name: String! = "x"): Repository
  viewer: User!
}

type Repository implements Node {
  deployments(first: Int, environments: [String!] @deprecated(reason: "Use envs."), orderBy: Order = {field: A, direction: DESC} @deprecated): DeploymentConnection!
  number: Int @deprecated(reason: "Use fullNumber.")
  state: State @deprecated(reason: "")
}

interface Node { id: ID! }
type DeploymentConnection { nodes: [Deployment] }
type Deployment { databaseId: Int latestStatus: String }
type User { login: String! }
input Order { field: String direction: String }
enum State { OPEN CLOSED }
union Thing = Repository | User
scalar URI
directive @deprecated(reason: String = "No longer supported") on FIELD_DEFINITION | ARGUMENT_DEFINITION
'''

GRAPHQL_CFG = {"graphql": {"repository": {"deployments": {"nodes": {"databaseId": {}}}, "number": {}, "state": {}}}}


class GraphqlTests(unittest.TestCase):
    def test_sdl_table_has_kinds_args_and_deprecation(self) -> None:
        t = gen.table_from_sdl(SDL)
        self.assertEqual((t["query"], t["mutation"]), ("Query", None))
        self.assertEqual(t["types"]["State"]["kind"], "enum")
        self.assertEqual(t["types"]["Thing"]["kind"], "union")
        self.assertEqual(t["types"]["Node"]["kind"], "interface")
        self.assertEqual(t["types"]["Order"]["kind"], "input")
        self.assertEqual(t["types"]["URI"]["kind"], "scalar")
        f = t["types"]["Repository"]["fields"]
        self.assertEqual(f["deployments"]["args"], ["first", "environments", "orderBy"])
        self.assertEqual(f["deployments"]["type"], "DeploymentConnection!")
        self.assertEqual(t["types"]["DeploymentConnection"]["fields"]["nodes"]["type"], "[Deployment]")
        self.assertEqual(t["types"]["Query"]["fields"]["repository"]["type"], "Repository")
        self.assertEqual(f["number"]["deprecated"], "Use fullNumber.")
        self.assertEqual(f["deployments"]["deprecated_args"],
                         {"environments": "Use envs.", "orderBy": "No longer supported"})
        self.assertEqual(f["state"]["deprecated"], "No longer supported")
        self.assertNotIn("deprecated_args", t["types"]["Query"]["fields"]["repository"])
        self.assertEqual(t["types"]["Query"]["fields"]["repository"]["args"], ["owner", "name"])

    def test_sdl_keeps_nested_list_wrappers(self) -> None:
        t = gen.table_from_sdl("type Query { grid: [[String!]]! }")
        self.assertEqual(t["types"]["Query"]["fields"]["grid"]["type"], "[[String!]]!")

    def test_introspection_gives_the_same_table_as_sdl(self) -> None:
        def named(n: str, kind: str = "OBJECT") -> dict[str, Any]:
            return {"kind": kind, "name": n, "ofType": None}

        def wrapped(inner: dict[str, Any], *kinds: str) -> dict[str, Any]:
            for k in reversed(kinds):
                inner = {"kind": k, "name": None, "ofType": inner}
            return inner

        def field(name: str, ty: dict[str, Any], args: tuple[str, ...] | list[str] = (), dep: str | None = None,
                  dep_args: dict[str, str] | None = None) -> dict[str, Any]:
            dep_args = dep_args or {}
            return {"name": name, "args": [{"name": a, "type": named("String", "SCALAR"),
                                            "isDeprecated": a in dep_args, "deprecationReason": dep_args.get(a)}
                                           for a in args],
                    "type": ty, "isDeprecated": dep is not None, "deprecationReason": dep}

        intro = {"data": {"__schema": {
            "queryType": {"name": "Query"}, "mutationType": None,
            "types": [
                {"kind": "OBJECT", "name": "Query", "fields": [
                    field("repository", named("Repository"), ["owner"])]},
                {"kind": "OBJECT", "name": "Repository", "fields": [
                    field("tags", wrapped(named("String", "SCALAR"), "NON_NULL", "LIST", "NON_NULL"), ["first", "active"], "old", {"active": None})]},
                {"kind": "ENUM", "name": "State", "fields": None, "enumValues": [{"name": "OPEN"}]},
                {"kind": "INPUT_OBJECT", "name": "Order", "fields": None, "inputFields": []},
                {"kind": "SCALAR", "name": "String", "fields": None},
            ]}}}
        t = gen.table_from_introspection(intro)
        self.assertEqual((t["query"], t["mutation"]), ("Query", None))
        self.assertEqual(t["types"]["State"], {"kind": "enum"})
        self.assertEqual(t["types"]["Order"], {"kind": "input"})
        tags = t["types"]["Repository"]["fields"]["tags"]
        self.assertEqual(tags, {"type": "[String!]!", "deprecated": "old", "args": ["first", "active"],
                                "deprecated_args": {"active": "No longer supported"}})

    def test_pruned_table_keeps_only_listed_fields_and_the_types_they_name(self) -> None:
        doc = gen.build_graphql("acme", GRAPHQL_CFG, gen.table_from_sdl(SDL), SOURCE)
        types = doc["types"]
        self.assertEqual(sorted(types), [
            "Deployment", "DeploymentConnection", "Int", "Query", "Repository", "State"])
        self.assertEqual(sorted(types["Repository"]["fields"]), ["deployments", "number", "state"])
        self.assertEqual(types["Repository"]["fields"]["deployments"]["args"],
                         ["first", "environments", "orderBy"])
        self.assertEqual(types["Repository"]["fields"]["number"]["deprecated"], "Use fullNumber.")
        self.assertEqual(types["State"], {"kind": "enum"})
        self.assertEqual(types["DeploymentConnection"]["fields"]["nodes"]["type"], "[Deployment]")
        self.assertNotIn("viewer", types["Query"]["fields"])
        self.assertEqual(doc["query"], "Query")

    def test_a_listed_field_the_schema_lacks_is_an_error_with_its_path(self) -> None:
        cfg = {"graphql": {"repository": {"deployments": {"nodes": {"databseId": {}}}}}}
        with self.assertRaises(gen.Problems) as ctx:
            gen.build_graphql("acme", cfg, gen.table_from_sdl(SDL), SOURCE)
        self.assertEqual(ctx.exception.items, [
            "acme: Deployment.databseId: listed as selected (repository.deployments.nodes.databseId), "
            "not in the schema"])

    def test_a_selection_below_a_leaf_is_an_error(self) -> None:
        cfg = {"graphql": {"repository": {"state": {"x": {}}}}}
        with self.assertRaises(gen.Problems) as ctx:
            gen.build_graphql("acme", cfg, gen.table_from_sdl(SDL), SOURCE)
        self.assertEqual(ctx.exception.items, [
            "acme: repository.state: fields are listed below State, of kind enum"])

    def test_graphql_output_is_deterministic_and_carries_the_pin(self) -> None:
        t = gen.table_from_sdl(SDL)
        a = gen.render(gen.build_graphql("acme", GRAPHQL_CFG, t, SOURCE))
        b = gen.render(gen.build_graphql("acme", GRAPHQL_CFG, t, SOURCE))
        self.assertEqual(a, b)
        self.assertEqual(json.loads(a)["source"]["commit"], "c" * 40)

    MUTATION_SDL = '''
type Query { viewer: User }
type User { login: String! }
type Mutation {
  markFileAsViewed(input: MarkInput!): MarkPayload
  deleteEverything: Boolean
}
input MarkInput { path: String! }
type MarkPayload { clientMutationId: String }
'''

    def test_mutation_fields_are_kept_from_the_mutation_root(self) -> None:
        cfg = {"graphql": {"viewer": {"login": {}}},
               "graphql_mutation": {"markFileAsViewed": {"clientMutationId": {}}}}
        doc = gen.build_graphql("acme", cfg, gen.table_from_sdl(self.MUTATION_SDL), SOURCE)
        self.assertEqual(doc["mutation"], "Mutation")
        self.assertEqual(doc["types"]["Mutation"]["fields"],
                         {"markFileAsViewed": {"type": "MarkPayload", "args": ["input"]}})
        self.assertEqual(doc["types"]["MarkPayload"]["fields"]["clientMutationId"]["type"], "String")

    def test_a_listed_mutation_field_the_schema_lacks_is_an_error(self) -> None:
        cfg = {"graphql_mutation": {"markFileAsSeen": {}}}
        with self.assertRaises(gen.Problems) as ctx:
            gen.build_graphql("acme", cfg, gen.table_from_sdl(self.MUTATION_SDL), SOURCE)
        self.assertEqual(ctx.exception.items, [
            "acme: Mutation.markFileAsSeen: listed as selected (mutation.markFileAsSeen), not in the schema"])

    def test_mutation_fields_without_a_mutation_root_are_an_error(self) -> None:
        cfg = {"graphql_mutation": {"markFileAsViewed": {}}}
        with self.assertRaises(gen.Problems) as ctx:
            gen.build_graphql("acme", cfg, gen.table_from_sdl(SDL), SOURCE)
        self.assertEqual(ctx.exception.items, [
            "acme: graphql_mutation is listed, but the schema has no mutation root"])


class RunTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = self.tmp.name
        self.spec_dir = os.path.join(root, "specs")
        self.out_dir = os.path.join(root, "out")
        self.in_dir = os.path.join(root, "in")
        for d in (self.spec_dir, self.in_dir):
            os.makedirs(d)
        rest = json.dumps(mini_spec()).encode()
        self.rest_file = f"acme-rest-{hashlib.sha256(rest).hexdigest()}.json"
        self.write(os.path.join(self.spec_dir, self.rest_file), rest)
        self.write(os.path.join(self.spec_dir, f"acme-graphql-{hashlib.sha256(SDL.encode()).hexdigest()}.graphql"),
                   SDL.encode())
        cfg = mini_cfg()
        cfg.update(GRAPHQL_CFG)
        self.write(os.path.join(self.in_dir, "acme.json"), json.dumps(cfg).encode())
        self.sources = {"acme": {
            "rest": dict(SOURCE, path="spec.json", sha256=hashlib.sha256(rest).hexdigest()),
            "graphql": dict(SOURCE, path="schema.graphql", sha256=hashlib.sha256(SDL.encode()).hexdigest()),
            "license": None,
        }}

    @staticmethod
    def write(path: str, data: bytes) -> None:
        with open(path, "wb") as f:
            f.write(data)

    @staticmethod
    def read(path: str) -> bytes:
        with open(path, "rb") as f:
            return f.read()

    def run_gen(self, *flags: str) -> int:
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return gen.run(["--spec-dir", self.spec_dir, "--out", self.out_dir, "--inputs", self.in_dir, *flags],
                           self.sources)

    def test_a_run_writes_one_file_per_spec_and_a_second_run_is_identical(self) -> None:
        self.assertEqual(self.run_gen(), 0)
        names = sorted(os.listdir(self.out_dir))
        self.assertEqual(names, ["acme.graphql.json", "acme.openapi.json"])
        first = {n: self.read(os.path.join(self.out_dir, n)) for n in names}
        self.assertEqual(self.run_gen(), 0)
        second = {n: self.read(os.path.join(self.out_dir, n)) for n in names}
        self.assertEqual(first, second)

    def test_check_passes_on_current_output_and_fails_on_a_changed_pin(self) -> None:
        self.assertEqual(self.run_gen(), 0)
        self.assertEqual(self.run_gen("--check"), 0)
        self.sources["acme"]["rest"]["commit"] = "e" * 40
        self.assertEqual(self.run_gen("--check"), 1)

    def test_check_names_the_stale_file_and_does_not_rewrite_it(self) -> None:
        self.assertEqual(self.run_gen(), 0)
        path = os.path.join(self.out_dir, "acme.openapi.json")
        self.write(path, b"{}\n")
        self.assertEqual(self.run_gen("--check"), 1)
        self.assertEqual(self.read(path), b"{}\n")

    def test_check_fails_on_a_missing_output_file(self) -> None:
        self.assertEqual(self.run_gen("--check"), 1)

    def test_a_changed_spec_byte_fails_loudly_with_both_hashes(self) -> None:
        path = os.path.join(self.spec_dir, self.rest_file)
        self.write(path, self.read(path) + b" ")
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            self.assertEqual(gen.run(["--spec-dir", self.spec_dir, "--out", self.out_dir, "--inputs", self.in_dir],
                                     self.sources), 1)
        self.assertIn(self.rest_file, err.getvalue())
        self.assertIn(self.sources["acme"]["rest"]["sha256"], err.getvalue())
        self.assertFalse(os.path.exists(self.out_dir))

    def test_each_source_carries_its_own_license(self) -> None:
        self.sources["acme"]["license"] = {
            "rest": {"name": "CC BY-SA 4.0", "url": "https://example.invalid/a", "notice": "Rest excerpt."},
            "graphql": {"name": "MIT", "url": "https://example.invalid/m", "notice": "GraphQL excerpt."}}
        self.assertEqual(self.run_gen(), 0)
        rest = json.loads(self.read(os.path.join(self.out_dir, "acme.openapi.json")))
        graphql = json.loads(self.read(os.path.join(self.out_dir, "acme.graphql.json")))
        self.assertEqual(rest["info"]["license"]["name"], "CC BY-SA 4.0")
        self.assertEqual(graphql["license"]["name"], "MIT")
        self.assertIn("GraphQL excerpt.", graphql["description"])
        self.assertNotIn("Rest excerpt.", graphql["description"])

    def test_a_problem_in_the_inputs_writes_nothing_and_exits_1(self) -> None:
        cfg = mini_cfg()
        cfg.update(GRAPHQL_CFG)
        cfg["operations"][0]["read"]["zzz"] = {}
        self.write(os.path.join(self.in_dir, "acme.json"), json.dumps(cfg).encode())
        self.assertEqual(self.run_gen(), 1)
        self.assertFalse(os.path.exists(self.out_dir))


if __name__ == "__main__":
    unittest.main()
