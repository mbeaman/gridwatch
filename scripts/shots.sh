#!/usr/bin/env bash
# Regenerate everything CI drift-checks (D33 executable specs, brief 2a task 5):
# the README SVG screenshots from `shot` (byte-deterministic: embedded config,
# virtual clock, seeded synth — D41), the metric catalogue (`docs/KEYS.md`)
# and the component manifests (`docs/COMPONENTS.md`).
#   scripts/shots.sh            # regenerate in place
#   scripts/shots.sh --check    # regenerate, then fail if git sees a diff
set -euo pipefail
cd "$(dirname "$0")/.."
GW="${GRIDWATCH_BIN:-}"
if [ -z "$GW" ]; then
  cargo build --release --quiet
  GW=target/release/gridwatch
fi
mkdir -p docs/img
"$GW" shot --format svg --size 250x70 --theme retrowave --page 1 > docs/img/overview-retrowave.svg
"$GW" shot --format svg --size 250x70 --theme modern    --page 1 > docs/img/overview-modern.svg
"$GW" shot --format svg --size 120x40 --theme retrowave --page 1 > docs/img/dense-120x40.svg
"$GW" keys > docs/KEYS.md
"$GW" component list > docs/COMPONENTS.md
if [ "${1:-}" = "--check" ]; then
  if ! git diff --exit-code --stat -- docs/img docs/KEYS.md docs/COMPONENTS.md; then
    echo "docs drift: regenerate with scripts/shots.sh and commit" >&2
    exit 1
  fi
  echo "docs in sync"
fi
