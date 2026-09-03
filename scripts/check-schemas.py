#!/usr/bin/env python3
"""Validate the shipped defaults and themes against schema/ (D33)."""
import json
import pathlib
import sys
import tomllib

import jsonschema
from referencing import Registry
from referencing.jsonschema import DRAFT202012

ROOT = pathlib.Path(__file__).resolve().parent.parent
FAILED = False


def check(schema_name: str, doc, label: str, quiet: bool = False) -> bool:
    global FAILED
    schema = json.loads((ROOT / "schema" / schema_name).read_text())
    try:
        jsonschema.Draft202012Validator(schema).validate(doc)
        if not quiet:
            print(f"ok   {label} vs {schema_name}")
        return True
    except jsonschema.ValidationError as e:
        FAILED = True
        print(f"FAIL {label} vs {schema_name}: {e.message} at {list(e.absolute_path)}")
        return False


def toml_doc(path: pathlib.Path):
    return tomllib.loads(path.read_text())


for theme in sorted((ROOT / "themes").glob("*.toml")):
    check("theme.schema.json", toml_doc(theme), f"themes/{theme.name}")

check("config.schema.json", toml_doc(ROOT / "crates/app/src/defaults/config.toml"), "defaults/config.toml")
check("layout.schema.json", toml_doc(ROOT / "crates/app/src/defaults/layout.toml"), "defaults/layout.toml")

# The scheme fixtures `gridwatch theme import` reads (D59 seam 2). Their own
# formats have no schema here — the check is that they are the shapes the
# importer's three parsers expect, so a fixture cannot rot into something the
# tests silently stop covering.
import_dir = ROOT / "fixtures/themes/import"
if import_dir.is_dir():
    seen = set()
    for path in sorted(import_dir.iterdir()):
        if not path.is_file():
            continue
        text = path.read_text()
        if "[colors.primary]" in text and "[colors.normal]" in text:
            seen.add("alacritty")
        elif "ansi = [" in text and "brights = [" in text:
            seen.add("wezterm")
        elif "base00" in text and "base0F" in text:
            seen.add("base16")
        else:
            FAILED = True
            print(f"FAIL fixtures/themes/import/{path.name}: not one of the three formats")
    missing = {"alacritty", "wezterm", "base16"} - seen
    if missing:
        FAILED = True
        print(f"FAIL fixtures/themes/import: no fixture for {', '.join(sorted(missing))}")
    else:
        print("ok   fixtures/themes/import covers alacritty, wezterm and base16")

# The `[[plugins]]` surface (§4.7, arc 8b). The good file must validate whole;
# every entry of the bad one must be refused **on its own** against the item
# subschema, so a file that happens to fail on its first entry cannot hide
# three that would have been accepted.
config_schema = json.loads((ROOT / "schema/config.schema.json").read_text())
plugin_item = config_schema["properties"]["plugins"]["items"]
plugin_ok = ROOT / "fixtures/config/plugins-ok.toml"
if plugin_ok.is_file():
    check("config.schema.json", toml_doc(plugin_ok), "fixtures/config/plugins-ok.toml")
plugin_bad = ROOT / "fixtures/config/plugins-bad.toml"
if plugin_bad.is_file():
    item_validator = jsonschema.Draft202012Validator(plugin_item)
    accepted = [
        e.get("id", f"#{i}")
        for i, e in enumerate(toml_doc(plugin_bad).get("plugins", []))
        if not list(item_validator.iter_errors(e))
    ]
    if accepted:
        FAILED = True
        for name in accepted:
            print(f"FAIL fixtures/config/plugins-bad.toml: [[plugins]] {name!r} was accepted")
    else:
        n = len(toml_doc(plugin_bad).get("plugins", []))
        print(f"ok   fixtures/config/plugins-bad.toml ({n} entries) are refused vs config.schema.json")

for layout in sorted((ROOT / "fixtures/layouts").glob("*.toml")):
    check("layout.schema.json", toml_doc(layout), f"fixtures/layouts/{layout.name}")

# Journals (§4.5, D47): every line of every fixture, the first one a header.
for journal in sorted((ROOT / "fixtures/journals").glob("*.jsonl")):
    lines = [l for l in journal.read_text().splitlines() if l.strip()]
    ok = all([
        check("journal.schema.json", json.loads(line), f"{journal.name}:{i + 1}", quiet=True)
        for i, line in enumerate(lines)
    ])
    first = json.loads(lines[0]) if lines else {}
    if "v" not in first:
        FAILED = True
        print(f"FAIL fixtures/journals/{journal.name}: first line is not a header")
    elif ok:
        print(f"ok   fixtures/journals/{journal.name} ({len(lines)} lines) vs journal.schema.json")

# The plugin protocol (§4.7, arc 8b): `exec.schema.json` references the
# manifest and view schemas, so it needs a registry rather than a bare
# validator. Every line of the good fixture must validate, and every line
# of the bad one must be refused — a schema that accepts nonsense is worse
# than no schema, because the host trusts it.
def exec_validator():
    names = ["exec", "manifest", "view"]
    docs = {n: json.loads((ROOT / "schema" / f"{n}.schema.json").read_text()) for n in names}
    registry = Registry().with_resources(
        [(f"{n}.schema.json", DRAFT202012.create_resource(d)) for n, d in docs.items()]
    )
    return jsonschema.Draft202012Validator(docs["exec"], registry=registry)


plugin_dir = ROOT / "fixtures/plugins"
if plugin_dir.is_dir():
    validator = exec_validator()
    for path in sorted(plugin_dir.glob("*.jsonl")):
        must_pass = path.name != "bad.jsonl"
        lines = [l for l in path.read_text().splitlines() if l.strip()]
        bad_lines = []
        for i, line in enumerate(lines):
            errors = list(validator.iter_errors(json.loads(line)))
            if bool(errors) == must_pass:
                bad_lines.append((i + 1, errors[0].message if errors else "accepted"))
        if bad_lines:
            FAILED = True
            for n, why in bad_lines:
                verb = "rejected" if must_pass else "accepted"
                print(f"FAIL fixtures/plugins/{path.name}:{n} {verb}: {why}")
        else:
            what = "validate" if must_pass else "are refused"
            print(f"ok   fixtures/plugins/{path.name} ({len(lines)} lines) {what} vs exec.schema.json")

sys.exit(1 if FAILED else 0)
