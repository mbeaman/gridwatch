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
everything else. Publishing them to `github.com/mbeaman/gridwatch/wiki` is a
separate, outward-facing step and has not been done. It needs two mechanical
changes, because a GitHub wiki is a different git repository:

1. **Links** lose their `.md` — a link written `[Installing](Installing.md)`
   here has to become `[Installing](Installing)` there.
2. **Images** must be committed into the wiki repo. A wiki page cannot render an
   SVG from `raw.githubusercontent.com`: GitHub serves it as `text/plain` and
   the image proxy will not draw it.
