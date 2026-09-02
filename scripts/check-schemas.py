#!/usr/bin/env python3
"""Validate the shipped defaults and themes against schema/ (D33)."""
import json
import pathlib
import sys
import tomllib

import jsonschema

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

sys.exit(1 if FAILED else 0)
