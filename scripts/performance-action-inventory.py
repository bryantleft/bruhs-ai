"""Regenerate the source catalogue with `python3 scripts/performance-action-inventory.py`.

This does not establish measured coverage. Event bindings are search references,
not a Rust AST or an exhaustive semantic action list. Only target/ output is written.
"""
from collections import Counter
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--source-root', type=Path, default=Path(__file__).resolve().parent.parent)
parser.add_argument('--output', type=Path, help='Output path; defaults to SOURCE_ROOT/target/performance-action-inventory.json')
parser.add_argument('--snapshot-source', type=Path, help='New directory for exact source bytes as read; this is not an atomic workspace/build snapshot')
args = parser.parse_args()
ROOT = args.source_root.resolve()
sources = {}
source_bytes = {}
if args.snapshot_source:
    assert not args.snapshot_source.exists(), 'Preserve an existing source capture; use a new directory'
    args.snapshot_source.mkdir(parents=True)
items = []
RAW_STRING_START = re.compile(r'r(#+)?"')


def source(path):
    if path not in source_bytes:
        raw = (ROOT / path).read_bytes()
        source_bytes[path] = raw
        sources[path] = hashlib.sha256(raw).hexdigest()
        if args.snapshot_source:
            copied = args.snapshot_source / path
            copied.parent.mkdir(parents=True, exist_ok=True)
            copied.write_bytes(raw)
    return source_bytes[path].decode()


def add(kind, identity, path, text, at, **extra):
    items.append({"kind": kind, "id": identity, "source": path,
                  "line": text.count("\n", 0, at) + 1,
                  "measurement_status": "requires_evidence_mapping", **extra})


def balanced(text, start, opening="(", closing=")"):
    depth, quoted, escaped = 0, False, False
    index = start
    while index < len(text):
        char = text[index]
        if quoted:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                quoted = False
        elif text.startswith("//", index):
            end = text.find("\n", index)
            index = len(text) if end < 0 else end
            continue
        elif text.startswith("/*", index):
            level = 1
            index += 2
            while level and index < len(text):
                if text.startswith("/*", index):
                    level += 1
                    index += 2
                elif text.startswith("*/", index):
                    level -= 1
                    index += 2
                else:
                    index += 1
            assert level == 0, 'Unclosed block comment'
            continue
        elif char == 'r' and (raw := RAW_STRING_START.match(text, index)) is not None:
            end = text.find('"' + (raw[1] or ''), raw.end())
            assert end >= 0, 'Unclosed Rust raw string'
            index = end + 1 + len(raw[1] or '')
            continue
        elif char == '"':
            quoted = True
        elif char == opening:
            depth += 1
        elif char == closing:
            depth -= 1
            if depth == 0:
                return text[start:index + 1]
        index += 1
    raise ValueError(f"Unclosed {opening} at {start}")


def register_routes(path, full_text, body, offset=0, handler_prefix="", listener="main", **context):
    assert not re.search(r'\.(?:nest|nest_service|route_service|merge)\s*\(', body), "Catalogue nested routers explicitly"
    for match in re.finditer(r'\.route\s*\(', body):
        call = balanced(body, body.index("(", match.start()))
        expression = call[1:].split(",", 1)[0].strip()
        path_origin = {"path_expression": expression, "listener": listener, **context}
        if expression.startswith('"'):
            route_path = json.loads(expression)
        else:
            assert expression == "bruhs_models::api::live_events::PATH", (
                "Resolve this route path before publishing the catalogue", expression
            )
            api_path = "bruhs-models/src/api.rs"
            api = source(api_path)
            module = re.search(r'pub mod live_events\s*\{', api)
            assert module is not None, "Missing live_events route contract"
            contract = balanced(api, api.index("{", module.start()), "{", "}")
            constants = list(re.finditer(r'pub const PATH:\s*&str\s*=\s*"([^"]+)"\s*;', contract))
            assert len(constants) == 1, "Resolve the live_events PATH contract"
            route_path = constants[0][1]
            path_origin.update({"path_source": api_path,
                                "path_line": api.count("\n", 0, module.start()) + 1
                                + contract.count("\n", 0, constants[0].start())})
        assert isinstance(route_path, str) and route_path.startswith("/"), expression
        assert not re.search(r'\b(?:any|on|trace|connect)\s*\(', call), "Catalogue additional method routing explicitly"
        methods = re.findall(r'\b(get|post|put|patch|delete|options|head)\s*\(\s*([\w:]+|\|\|)', call)
        assert methods, call
        for verb, handler in methods:
            if handler == "||":
                handler = f"{handler_prefix}<inline closure>"
            elif "::" not in handler:
                handler = handler_prefix + handler
            identity = f"{verb.upper()} {route_path}"
            if listener != "main":
                identity = listener + ":" + identity
            add("http", identity, path, full_text, offset + match.start(), handler=handler,
                method=verb.upper(), route_path=route_path, **path_origin)


def registered_function(path, name, **context):
    text = source(path)
    declaration = re.search(r'pub fn ' + re.escape(name) + r'\s*\(', text)
    assert declaration is not None, (path, name)
    start = text.index("{", declaration.start())
    body = balanced(text, start, "{", "}")
    register_routes(path, text, body, start, **context)


path = "bruhs-server/src/main.rs"
text = source(path)
merges = list(re.finditer(r'\.merge\s*\(', text))
workbench = re.search(r'\bfn workbench_services\s*\(', text)
composed = []
if workbench is not None:
    start = text.index('{', workbench.start())
    helper = balanced(text, start, '{', '}')
    normalized = re.sub(r'\s+', '', helper)
    assert normalized.count('routes::delivery::router(delivery)') == 1
    assert normalized.count('.merge(routes::remote_hosts::router(hosts))') == 1
    assert not re.search(r'\.(?:route|nest|nest_service|route_service)\s*\(', helper), 'Resolve direct routes in workbench composition'
    expected = {
        '(workbench_services(delivery,remote_hosts.clone(),config.access.clone(),))',
        '(workbench_services(delivery,remote_hosts.clone(),config.access.clone()))',
        '(routes::remote_hosts::router(hosts))',
    }
    calls = [(merge, re.sub(r'\s+', '', balanced(text, text.index('(', merge.start())))) for merge in merges]
    assert len(calls) == 2 and all(call in expected for _, call in calls), 'Catalogue this merged workbench router explicitly'
    outer = [merge for merge, call in calls if call.startswith('(workbench_services(')]
    assert len(outer) == 1 and all(call != '(routes::remote_hosts::router(hosts))' or start <= merge.start() < start + len(helper) for merge, call in calls), 'Resolve workbench registration'
    for module in ['delivery', 'remote_hosts']:
        composed.append((module, outer[0], {'composition_source': path, 'composition_line': text.count('\n', 0, workbench.start()) + 1}))
else:
    for merge in merges:
        call = balanced(text, text.index("(", merge.start()))
        assert re.match(r'\(\s*routes::delivery::router\(delivery\)\.with_state\(', call), "Catalogue this merged router explicitly"
    assert len(merges) <= 1, "Resolve repeated delivery router composition"
    if merges:
        composed.append(('delivery', merges[0], {}))
assert not re.search(r'\.(?:nest|nest_service|route_service)\s*\(', text), "Catalogue nested routers explicitly"
router_calls = re.findall(r'\b(routes::\w+::(?:router|remote_router))\s*\(', text)
allowed = {'routes::coordination::remote_router'} | {f'routes::{module}::router' for module, _, _ in composed}
assert set(router_calls) <= allowed, "Catalogue this router constructor explicitly"
for module, _, _ in composed:
    assert router_calls.count(f'routes::{module}::router') == 1, 'Resolve repeated router registration'
# Remove only the explicitly resolved merge span from the top-level route scan.
direct_text = text
for merge in reversed(merges):
    end = text.index("(", merge.start()) + len(balanced(text, text.index("(", merge.start())))
    direct_text = direct_text[:merge.start()] + ' ' * (end - merge.start()) + direct_text[end:]
register_routes(path, text, direct_text)
for module, registration, context in composed:
    registered_function(f'bruhs-server/src/routes/{module}.rs', 'router', handler_prefix=f'routes::{module}::', registration_source=path, registration_line=text.count('\n',0,registration.start())+1, **context)
if 'routes::coordination::remote_router' in router_calls:
    assert router_calls.count('routes::coordination::remote_router') == 1
    guard = re.search(r'if let Ok\(bind\) = std::env::var\("BRUHS_TASK_BIND"\)\s*\{', text)
    assert guard is not None, "Resolve paired listener enablement"
    guard_body = balanced(text, text.index('{',guard.start()), '{', '}')
    assert 'routes::coordination::remote_router(coordination)' in guard_body and 'axum::serve(remote, remote_app)' in guard_body, "Resolve paired listener registration"
    registered_function('bruhs-server/src/routes/coordination.rs', 'remote_router', handler_prefix='routes::coordination::', listener='paired_task', enabled_by='BRUHS_TASK_BIND', registration_source=path, registration_line=text.count('\n',0,guard.start())+1)

for path, constructor, surface in [
    ("bruhs-web/src/components/command_bar.rs", "Action", "web"),
    ("bruhs-desktop/src/views/workspace/palette_ui.rs", "CommandAction", "desktop"),
]:
    text = source(path)
    for match in re.finditer(r'\b' + constructor + r'\s*\{\s*id:\s*(?:ActionId::(\w+)|"([^"]+)"),\s*group:\s*"([^"]+)",\s*label:\s*"([^"]+)"', text):
        add("palette", f"{surface}:{match[1] or match[2]}", path, text, match.start(), group=match[3], label=match[4])

path = "bruhs-models/src/models/ui.rs"
text = source(path)
start = text.index("pub enum AppView")
body = balanced(text, text.index("{", start), "{", "}")
for match in re.finditer(r'^    ([A-Z]\w*)(?:,|\s*\{)', body, re.M):
    add("view", match[1], path, text, text.index("{", start) + match.start())

path = "bruhs-models/src/logic/commands.rs"
text = source(path)
for match in re.finditer(r'SlashCommand\s*\{\s*name:\s*"([^"]+)"', text):
    add("slash", match[1], path, text, match.start())

path = "bruhs-models/src/models/automation.rs"
text = source(path)
start = text.index("pub const fn key(")
keys = balanced(text, text.index("{", start), "{", "}")
for match in re.finditer(r'Self::(\w+)\s*=>\s*"([^"]+)"', keys):
    for access in ["read", "write"]:
        add("mcp", f"bruhs_{access}_{match[2]}", path, text, text.index("{", start) + match.start(), domain=match[2], access=access)
start = text.index("pub enum WorkflowOperation")
body = balanced(text, text.index("{", start), "{", "}")
for match in re.finditer(r'^    ([A-Z]\w*)\s*\{', body, re.M):
    add("workflow", match[1], path, text, text.index("{", start) + match.start())

for path in ["bruhs-mcp/src/tools.rs", "bruhs-mcp/src/tools/workflow_schema.rs"]:
    text = source(path)
    # Parser diagnostics also contain bruhs_* labels; count tool declarations.
    for match in re.finditer(r'(?:\binfo\(\s*|\bname:\s*)"(bruhs_[a-z0-9_]+)"', text):
        if not any(i["kind"] == "mcp" and i["id"] == match[1] for i in items):
            add("mcp", match[1], path, text, match.start())

path = "bruhs-mcp/src/serve.rs"
text = source(path)
start = text.index("{", text.index("async fn handle_request("))
body = balanced(text, start, "{", "}")
for match in re.finditer(r'^[ \t]*"([^\"]+)"\s*=>', body, re.M):
    add("mcp_method", match[1], path, text, start + match.start(), transport="stdio")

paths = subprocess.check_output(["rg", "--files", "bruhs-web/src", "bruhs-desktop/src"], cwd=ROOT, text=True).splitlines()
for path in paths:
    if not path.endswith(".rs") or "/tests/" in path or path.endswith(("tests.rs", "performance_tests.rs")):
        continue
    text = source(path)
    # Inline test modules are not necessarily the last item in a Rust file.
    # Keep all search candidates rather than dropping later production controls.
    # Test-only references are resolved during semantic review, not guessed here.
    pattern = r'\bon:([A-Za-z_]\w*)\s*=' if path.startswith("bruhs-web") else r'\.on_(\w+)\s*\('
    for match in re.finditer(pattern, text):
        line = text.count("\n", 0, match.start()) + 1
        column = match.start() - text.rfind("\n", 0, match.start())
        add("binding_reference", f"{path}:{line}:{column}:{match[1]}", path, text, match.start(),
            event=match[1], column=column,
            classification="requires_semantic_and_test_context_review")

for surface, path, marker, pattern in [
    ("desktop", "bruhs-desktop/src/views/workspace/palette_ui.rs", "match action_id {",
     r'^[ \t]*("[^"]+"|_)[ \t]*=>'),
    ("web", "bruhs-web/src/components/command_bar.rs", "let execute_action = move |id: ActionId| {",
     r'^[ \t]*(ActionId::\w+(?:\s*\|\s*ActionId::\w+)*)[ \t]*=>'),
]:
    text = source(path)
    start = text.index("{", text.index(marker))
    body = balanced(text, start, "{", "}")
    arms = list(re.finditer(pattern, body, re.M))
    dispatches = {}
    for index, arm in enumerate(arms):
        if arm[1] == "_":
            continue
        names = [arm[1].strip('"')] if surface == "desktop" else re.findall(r'ActionId::(\w+)', arm[1])
        end = arms[index + 1].start() if index + 1 < len(arms) else len(body)
        span = body[arm.start():end]
        line = text.count("\n", 0, start + arm.start()) + 1
        for name in names:
            assert name not in dispatches, (surface, name)
            dispatches[name] = {
                "source": path, "line": line,
                "owner": "Workspace::execute_command_action" if surface == "desktop" else "CommandBar execute_action closure",
                "text_span_sha256": hashlib.sha256(span.encode()).hexdigest(),
            }
    catalogue = [item for item in items if item["kind"] == "palette" and item["id"].startswith(surface + ":")]
    catalogue_names = {item["id"].split(":", 1)[1] for item in catalogue}
    assert catalogue_names == set(dispatches), {
        "surface": surface,
        "missing_dispatch": sorted(catalogue_names - dispatches.keys()),
        "uncatalogued_dispatch": sorted(dispatches.keys() - catalogue_names),
    }
    for item in catalogue:
        item["dispatch"] = dispatches[item["id"].split(":", 1)[1]]

identities = [(i["kind"], i["id"]) for i in items]
assert len(identities) == len(set(identities)), "Duplicate catalogue identity"
evidence_path = ROOT / "docs/performance-action-evidence.json"
if evidence_path.exists():
    evidence = json.loads(evidence_path.read_text())
    assert evidence["version"] == 1
    entries = {(item["kind"], item["id"]): item for item in items}
    reviewed = set()
    for entry in evidence["entries"]:
        identity = (entry["kind"], entry["id"])
        assert identity in entries, identity
        assert entry["evidence_id"] not in reviewed, entry["evidence_id"]
        assert entry["status"] == "partial_variant_evidence", entry["status"]
        reviewed.add(entry["evidence_id"])
        changed = []
        for name, expected in entry["source_sha256"].items():
            path = (ROOT / name).resolve()
            assert path.is_relative_to(ROOT), name
            if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
                changed.append(name)
        missing = [name for name in entry["artifacts"] if not (ROOT / name).is_file()]
        item = entries[identity]
        if changed or item["measurement_status"] != "evidence_source_changed":
            item["measurement_status"] = "evidence_source_changed" if changed else "partial_variant_evidence"
        item.setdefault("evidence", []).append({
            "evidence_id": entry["evidence_id"],
            "ledger": str(evidence_path.relative_to(ROOT)),
            "variant": entry["variant"], "boundaries": entry["boundaries"],
            "report": entry["report"], "remaining": entry["remaining"],
            "changed_sources": changed, "missing_local_artifacts": missing,
        })
    correction_ids = set()
    for correction in evidence.get("corrections", []):
        assert correction["id"] not in correction_ids, correction["id"]
        correction_ids.add(correction["id"])
        affected = set(correction["affected_evidence_ids"])
        assert affected and affected <= reviewed, correction["id"]
        missing = [name for name in correction["artifacts"] if not (ROOT / name).is_file()]
        for item in items:
            for mapped in item.get("evidence", []):
                if mapped["evidence_id"] in affected:
                    mapped.setdefault("corrections", []).append({
                        "id": correction["id"], "effect": correction["effect"],
                        "artifacts": correction["artifacts"],
                        "missing_local_artifacts": missing,
                    })
result = {
    "purpose": "Coverage discovery; no automatic latency acceptance or promotion of existing evidence",
    "limitations": ["Binding references and palette dispatch spans are text searches, not a Rust AST or transitive call graph. Entire files are scanned because production code may follow inline tests; comments, test-only callbacks and lifecycle hooks require semantic classification. Desktop discovery includes global app/dock callbacks outside views; columns are 1-based character offsets", "HTTP discovery covers explicit route methods; implicit HEAD, CORS, fallback and parameter branches require separate review. Unknown path expressions and nested routers require catalogue updates", "Parameter branches, context menus, subscriptions, shortcuts, errors and lifecycle variants need semantic review", "Native bridge and standalone MCP modes require separate evidence", "Previous partial measurements remain in the audit report; entries require explicit evidence mapping"],
    "counts": dict(Counter(i["kind"] for i in items)),
    "source_sha256": sources, "items": items,
}
assert all(hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == digest for name, digest in sources.items()), 'Source changed during discovery; rerun the capture'
output = args.output or ROOT / "target/performance-action-inventory.json"
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
print(json.dumps({"counts": result["counts"], "sources": len(sources), "palette_dispatches": sum("dispatch" in item for item in items), "output": str(output)}, indent=2))
