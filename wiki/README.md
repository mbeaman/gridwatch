# `wiki/` — the user-facing guide

`docs/` is the **spec**: architecture, decisions, performance ceilings, parity
tables, the roadmap. It is written for someone changing gridwatch.

`wiki/` is for someone **using** it. It answers "what is this number", "why is
that tile empty", "how do I add the one I want" — questions no file in `docs/`
holds, because a specification says what a thing *is* and a guide says what to
*do* with it.

**The rule that keeps the two from drifting:** a wiki page never restates
something a generated file already says. `docs/COMPONENTS.md` owns the tier
ladders, minimum sizes and footprints; `docs/KEYS.md` owns the metric
catalogue; `docs/KEYBINDINGS.md` owns every key. Those three are regenerated
from the binary and CI fails on drift. A wiki page **links** to them and spends
its own words on meaning instead.

## The screenshots are generated

Every tile picture under `docs/img/wiki/` comes from `scripts/wiki-shots.sh`,
which places each tile alone on a 12x6 grid in a sandbox config and shoots it at
160x44 — the smallest frame that stays out of dense mode, so what you see is the
tier a reader gets on a normal terminal rather than a degraded one. Regenerate
with:

```sh
scripts/wiki-shots.sh            # in place
scripts/wiki-shots.sh --check    # fail if git sees a diff (CI runs this)
```

Nothing here is pasted by hand. The README spent three arcs showing four shipped
tiles as "arrives in a later arc" for exactly that reason, and in September 2026
it was still naming ten tiles while its own generated screenshot showed the
eleventh.

**Not reachable from `shot`:** the zoom-only `full` tier of each tile, because
reaching it needs a keypress and `shot` renders one frame. Those tiers are
described in prose and marked as such.

## Publishing to the GitHub wiki

These pages live in the repo so they are reviewed, diffed and drift-checked like
everything else. `scripts/wiki-publish.sh` pushes them to
`github.com/mbeaman/gridwatch/wiki`:

```sh
scripts/wiki-publish.sh --dry-run   # convert into build/wiki-out and stop
scripts/wiki-publish.sh             # convert and push
```

It is a script rather than a copy because a GitHub wiki is a **separate git
repository**, and three things have to change on the way out:

1. **Page links lose their `.md`** — a link written `[Installing](Installing.md)`
   here has to become `[Installing](Installing)` there.
2. **Links into `docs/` become absolute.** A wiki page cannot relative-link into
   the code repo at all, so `../docs/ARCHITECTURE.md` becomes a `blob/main` URL.
3. **Images are copied in, flat, under `img/`.** A wiki page cannot render an SVG
   from `raw.githubusercontent.com` — it is served as `text/plain` and the image
   proxy will not draw it — so every picture has to live in the wiki repo itself.

`wiki/README.md` (this file) is about the directory rather than the product, so
it is not published. A `_Sidebar.md` is generated, and says on it that the wiki
is a copy: anyone who edits a page there loses it on the next publish.

**Before the first publish, somebody has to create one page in the browser.**
GitHub does not create `<repo>.wiki.git` until a page exists, and there is no
API for it — so the script stops with that instruction rather than a git error.
Open the repo's Wiki tab, click *Create the first page*, save anything; the
script overwrites it.
