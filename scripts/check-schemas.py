#!/usr/bin/env python3
"""Validate the shipped defaults and themes against schema/ (D33)."""
import json
import pathlib
import sys
import tomllib

import jsonschema

ROOT = pathlib.Path(__file__).resolve().parent.parent
FAILED = False


def check(schema_name: str, doc, label: str) -> None:
    global FAILED
    schema = json.loads((ROOT / "schema" / schema_name).read_text())
    try:
        jsonschema.validate(doc, schema)
        print(f"ok   {label} vs {schema_name}")
    except jsonschema.ValidationError as e:
        FAILED = True
        print(f"FAIL {label} vs {schema_name}: {e.message} at {list(e.absolute_path)}")


def toml_doc(path: pathlib.Path):
    return tomllib.loads(path.read_text())


for theme in sorted((ROOT / "themes").glob("*.toml")):
    check("theme.schema.json", toml_doc(theme), f"themes/{theme.name}")

check("config.schema.json", toml_doc(ROOT / "crates/app/src/defaults/config.toml"), "defaults/config.toml")
check("layout.schema.json", toml_doc(ROOT / "crates/app/src/defaults/layout.toml"), "defaults/layout.toml")

for layout in sorted((ROOT / "fixtures/layouts").glob("*.toml")):
    check("layout.schema.json", toml_doc(layout), f"fixtures/layouts/{layout.name}")

sys.exit(1 if FAILED else 0)
