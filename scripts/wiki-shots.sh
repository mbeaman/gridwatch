#!/usr/bin/env bash
# One screenshot per tile for `wiki/`, on the same principle as scripts/shots.sh:
# a picture pasted in by hand rots, a generated one cannot. Each tile is placed
# alone on a 12x6 grid in a sandbox config, so the shot shows that tile's
# richest *grid* tier — the zoom-only `full` tiers need a keypress and are not
# reachable from `shot`.
#   scripts/wiki-shots.sh            # regenerate in place
#   scripts/wiki-shots.sh --check    # regenerate, then fail if git sees a diff
set -euo pipefail
cd "$(dirname "$0")/.."
GW="${GRIDWATCH_BIN:-}"
if [ -z "$GW" ]; then
  cargo build --release --quiet
  GW=target/release/gridwatch
fi
mkdir -p docs/img/wiki
SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

# No [[components]] and no [sources.*]: a placement may name a `kind` directly,
# so every tile here is the shipped default of its kind with no options set —
# which is what a reader who has just installed gridwatch would see.
printf 'schema = 1\ntheme = "retrowave"\n' > "$SB/config.toml"

# The banner prints the page name, so it doubles as the caption.
shoot() { # kind, page label, WxH
  cat > "$SB/layout.toml" <<EOF
schema = 1

[grid]
columns = 12
rows = 6
gap = 1
borders = "each"
cell_aspect = 0.5
min_unit_inner = { cols = 8, rows = 3 }

[[pages]]
name = "$2"
hotkey = "1"
place = [ { kind = "$1", at = [0, 0], size = [12, 6] } ]
EOF
  "$GW" shot --config "$SB" --format svg --size "$3" > "docs/img/wiki/tile-$1.svg"
}

# 160x44 is the smallest frame that stays out of dense/stack mode with one tile
# filling the grid, so each of these is the tier a reader gets on a normal
# terminal rather than the degraded one.
shoot htop    "CPU"          160x44
shoot gpu     "GPU"          160x44
shoot disk    "Disks"        160x44
shoot net     "Network"      160x44
shoot pins    "12V-2x6 pins" 160x44
shoot sensors "Sensors"      160x44
shoot audio   "Audio"        160x44
shoot winamp  "Now playing"  160x44
shoot alerts  "Alerts"       160x44
shoot sources "Sources"      160x44
shoot clock   "Clock"        160x44

if [ "${1:-}" = "--check" ]; then
  # A regenerated file that is new (untracked) is drift too — `git diff` alone misses it.
  if ! git diff --exit-code --stat -- docs/img/wiki || git status --porcelain --untracked-files=all -- docs/img/wiki | grep -q '^??'; then
    echo "wiki screenshots drift: regenerate with scripts/wiki-shots.sh and commit" >&2
    exit 1
  fi
  echo "wiki screenshots in sync"
fi
