#!/usr/bin/env bash
# Publish wiki/ to github.com/mbeaman/gridwatch/wiki.
#
# A GitHub wiki is a *separate git repository*, which is why this is a script
# and not a copy: three things have to change on the way out.
#   1. Page links lose their `.md`     — [Installing](Installing.md) -> (Installing)
#   2. Links into docs/ become absolute — a wiki page cannot relative-link into
#      the code repo at all.
#   3. Images are copied in flat        — a wiki page cannot render an SVG from
#      raw.githubusercontent.com (served as text/plain; the proxy will not draw
#      it), so every image has to live in the wiki repo itself.
#
#   scripts/wiki-publish.sh            # convert, show the diff, push
#   scripts/wiki-publish.sh --dry-run  # convert into build/wiki-out and stop
#
# FIRST RUN: the wiki repo does not exist until somebody creates one page in
# the browser. Open https://github.com/mbeaman/gridwatch/wiki, click "Create
# the first page", save anything at all, then run this — it overwrites it.
set -euo pipefail
cd "$(dirname "$0")/.."
REPO="https://github.com/mbeaman/gridwatch"
BLOB="$REPO/blob/main"
DRY=""
[ "${1:-}" = "--dry-run" ] && DRY=1

if [ -n "$DRY" ]; then
  OUT="build/wiki-out"
  rm -rf "$OUT"; mkdir -p "$OUT"
else
  OUT="$(mktemp -d)"
  trap 'rm -rf "$OUT"' EXIT
  if ! git clone -q "$REPO.wiki.git" "$OUT" 2>/dev/null; then
    cat >&2 <<'MSG'
The wiki repository does not exist yet.

GitHub does not create <repo>.wiki.git until the first page is made through
the web interface, and there is no API for it. One click, once:

  1. https://github.com/mbeaman/gridwatch/wiki
  2. "Create the first page" — save anything, this script overwrites it
  3. re-run scripts/wiki-publish.sh

MSG
    exit 1
  fi
  find "$OUT" -maxdepth 1 -name '*.md' -delete
  rm -rf "$OUT/img"
fi

mkdir -p "$OUT/img"

# Every image any page references, copied in flat and renamed to its basename.
cp docs/img/wiki/*.svg "$OUT/img/"
cp docs/img/overview-*.svg "$OUT/img/"

# wiki/README.md describes the directory, not the product — it is not a page.
for src in wiki/*.md; do
  name="$(basename "$src")"
  [ "$name" = "README.md" ] && continue
  python3 - "$src" "$OUT/$name" "$BLOB" <<'PY'
import re, sys, pathlib
src, dst, blob = sys.argv[1], sys.argv[2], sys.argv[3]
t = pathlib.Path(src).read_text(encoding="utf-8")

# Match every `](target)` rather than label-plus-target: an image inside a
# link — [![alt](x.svg)](x.svg), which every tile page uses — nests brackets,
# and a label pattern only ever converts the inner one.
def fix(m):
    target = m.group(1)
    if target.startswith(("http://", "https://", "#")):
        return m.group(0)
    # Images live flat in the wiki repo under img/.
    if "docs/img/" in target:
        return f"](img/{target.rsplit('/', 1)[-1]})"
    # Anything else in docs/ or at the repo root can only be an absolute link.
    if target.startswith("../"):
        return f"]({blob}/{target[3:]})"
    # A sibling wiki page: drop the .md, keep any anchor.
    path, sep, anchor = target.partition("#")
    if path.endswith(".md"):
        return f"]({path[:-3]}{sep}{anchor})"
    return m.group(0)

# Code spans are not links: hide them, convert, put them back.
spans = []
def hide(m):
    spans.append(m.group(0))
    return f"\x00{len(spans) - 1}\x00"
t = re.sub(r'`[^`\n]*`', hide, t)
t = re.sub(r'\]\(([^)\s]+)\)', fix, t)
t = re.sub(r'\x00(\d+)\x00', lambda m: spans[int(m.group(1))], t)
pathlib.Path(dst).write_text(t, encoding="utf-8")
PY
done

# A sidebar, generated so it cannot drift from what is actually published.
cat > "$OUT/_Sidebar.md" <<'MSG'
### gridwatch

- [Home](Home)
- [Installing](Installing)
- [The tiles](Tiles)
  - [CPU](Tiles-CPU)
  - [GPU](Tiles-GPU)
  - [Disks](Tiles-Disks)
- [Configuring](Configuring)
- [Themes](Themes)
- [Keys](Keys)
- [Plugins](Plugins)
- [Troubleshooting](Troubleshooting)

---
Edited in the [code repo](https://github.com/mbeaman/gridwatch/tree/main/wiki),
not here — changes made in this wiki are overwritten on the next publish.
MSG

if [ -n "$DRY" ]; then
  echo "converted into $OUT — nothing pushed"
  exit 0
fi

# Read what we need from the code repo before leaving it.
REV="$(git rev-parse --short HEAD)"
cd "$OUT"
git add -A
if git diff --cached --quiet; then
  echo "wiki already up to date"
  exit 0
fi
git commit -q -m "Publish wiki/ from the code repo ($REV)"
git push -q origin HEAD
echo "wiki published to $REPO/wiki"
