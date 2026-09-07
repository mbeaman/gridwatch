# Session journal

> One entry per working session, newest first. This is the only document that records **what a session was like** — what was tried, what the reviews caught, what turned out to be wrong, and what is owed to Matt when it ends. The others answer different questions: `DECISIONS.md` says *why a choice was made*, `CHANGELOG.md` says *what shipped*, `ROADMAP.md` says *what an arc promised*, `PLAN.md` says *where the project stands right now*. A journal entry is allowed to record a dead end, a wrong assumption or a near miss, because those are the things no other file has a place for.
>
> **Write the entry before the session ends**, while the work is still in context, and commit it with the session's last commit. Lead with what changed for Matt and why it matters — not an inventory of edits (Matt, 2026-09-07). Keep it to what a reader six months from now would need.

---

## 2026-09-06 → 07 — the wide-terminal bug, and the source-option seams

**Models:** Fable 5.1 throughout (decisions, briefs, review judging); Opus 5 for both implementations and the six review lenses. **Shipped:** arcs 12 and 11, 28 commits, `8547c6d → 95977af`, CI green on both pushes. **Nothing tagged** — as always, that is Matt's.

### What changed for Matt

gridwatch fills a wide terminal now. That was his report ("appears broken if the console resolution is too wide") and it is fixed, confirmed in a real terminal rather than a buffer. And a misspelled source option in `config.toml` finally says so instead of being ignored forever.

### The thing worth remembering

**Neither arc was really about the bug it was named for.**

Arc 12 looked like three components hard-coding a drawing size. The actual problem was that nothing in the project *stopped* a fourth. So the fix was a rule in `ARCHITECTURE.md` §4.6 — a drawing inside a `Fill` band scales with its rect, and a constant is only ever a floor — plus `assert_grows_with_area` to enforce it per tier. That test then caught four more instances, **two of which the arc itself had just shipped**: a pins sparkline that capped its own buckets, and a GPU power trace that was fifty columns wide on any terminal. Without the test those ship silently and Matt rediscovers them on the next resize.

The lesson that generalises: **a growth test that counts cells per *tier* misses a capped drawing that is a small share of that tier's ink.** The sweep passed pins and gpu while both were still broken; the ux and user-path lenses found them by looking at rendered frames. Cell-count oracles need a per-drawing companion, or a human looking at the screen.

Arc 11 was the same shape one level down: a config that could be quietly wrong. The seams (`SourceDef.options`, `KeyMeta.labels`) had landed on 2026-09-05; this session built the breadth and reviewed it.

### What the reviews caught that the builds did not

Six adversarial lenses ran across the two arcs (correctness, spec-drift, ux-theme, user-path ×2). The pattern held from earlier arcs: **the implementation session's own report was accurate about what it built and blind to what it had broken.**

- **Arc 12** shipped four fresh instances of the defect it existed to fix (above), plus a zoomed GPU tile that showed *fewer* table rows than the grid below 30 rows — a plain "a third of the body" rule inverts under a short terminal.
- **Arc 11** shipped a reload that called a plugin's `[sources.<id>]` table "no such source in this build" while `config check` on the same file called it fine. Cause: the re-check read the plugin ids that had *started*, and `[[plugins]]` is restart-only, so a plugin added in the very save being checked can never be in that set. This is the first-plugin flow — the most likely time anyone meets it.
- **Toast truncation had a shape assumption.** An earlier review chose "keep the tail" because a parse error leads with `file:line:col` and ends with the reason. Arc 11's messages are the opposite shape — they lead with the source and the key. At 80 columns the network warning showed *neither*, just a bare list of option names. Toasts elide the middle now, which serves both shapes.
- **`config check` claimed to have read a value it had not.** `refresh_ms = "fast"` was echoed back as though accepted while the source silently kept its default. The report now says it checks option *names*; the real fix is a seam and is in `BACKLOG.md`.

### Two things that were decided rather than built

**The four §9 option-name collisions stay, and the naming call is Matt's** (D61 review amendment R6). `[sources.audio] fps` vs the audio and winamp tiles' `fps`; `[sources.sensors] chips`; `[sources.mpris] art`. All real — the name is valid on both sides, so no check can catch a person who writes one in the wrong file. Not renamed because the harm is the mildest kind (in all four cases the wrong-file write produces roughly the intended *effect*, differing in cost rather than outcome), because `[sources.audio] fps` is D35 decision 6 which Matt named himself, and because the churn would land on D55 and `PERFORMANCE.md` rows that must keep their original words. §9 records the exemptions; the test pins them in both directions. Candidates if he wants them gone: `analysis_fps`, `sample`, `fetch_art`.

*A near miss worth recording:* the first instinct was to rename all three on a consistency argument — "this arc exists to end silent wrongness, so leave no ambiguity". A second read reversed it, on the strength of the severity analysis that was already in hand and had been argued past. **The consistency argument was aesthetic; the severity analysis was evidence.**

**The rename was not the only overreach caught.** A ticked roadmap box claimed "a replay determinism test crosses a sweep boundary that evicts". It does not: every journal fixture spans about 62 s of store time against a 600 s retention, so a replay evicts nothing. The store-level determinism test does cross an evicting sweep; the *journal* one cannot without a fixture longer than ten minutes, which does not exist. A falsely ticked box is worse than an unticked one, because the next session reads the roadmap to decide what is left.

### Owed to Matt after this session

- **His real Ptyxis window size.** `MATT_TERMINAL` in `crates/cli/tests/smoke.rs` is a `None` slot waiting for it, and `MACHINE.md` still says unknown. The moment it is filled in, the wide-terminal assertions run at his actual size.
- **The naming call** above.
- **Everything already owed**: every tag from `v0.1.0`, the `Cargo.toml` version (still `0.1.0` after twelve arcs, so an installed binary lies about itself), the game fixture, the Ptyxis and i2c rows.

### Where to pick up

The roadmap is empty again. `BACKLOG.md` gained two items that need a Fable session because each changes a Rust contract: **typed source options** (`from_table` returning `Result` — a wrongly *typed* value is still silent today) and **the dead `[store]` section** (`history = "1h"` silently gets ten minutes, and two further spec claims ride on the unimplemented `max_mb`). The `disk` component remains the one genuinely missing vertical.
