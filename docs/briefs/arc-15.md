# Arc 15 brief — "what a tile draws with the room" (D65)

> Written 2026-09-09 with D65. **Fable was rate-limited, so an Opus session wrote the design**; read D65 first — it decides everything below and this brief only says where each piece goes. **Arc 15 changes no seam.** No new `View` variant, `Unit`, `Capability`, `Control`, `Manifest` field or `Detail` use, and no change to `ColWidth`, `Bounds`, `Series` or `Column`. If anything here seems to need one, **stop and escalate**. Build **15a, then 15b**, sequentially: 15b's net chart needs 15a's midpoint gridline for its zero line, and 15a churns every snapshot in the repo.

## What already exists (do not rebuild)

| thing | where | state |
|---|---|---|
| §4.6's drawing rule + `assert_grows_with_area` | `docs/ARCHITECTURE.md` §4.6, `ui/src/testkit.rs` | landed (D62) — this arc adds the *text* half and the per-leaf sweep, and weakens neither |
| the sparkline that holds across empty columns | `ui/src/renderer` | landed (D62 §1) — the same "fix it in the renderer" argument is what seams 1 and 3 rest on |
| `Series.label` | `ui/src/view.rs` | carried since arc 2b and **read by nothing in the renderer** — 15a draws it |
| `View::Empty` | `ui/src/view.rs` | exists; used under `Fill(1)` in the htop header. The sensors header spacer is this |
| `stack`'s constraint split | `ui/src/renderer/mod.rs` | the exact algorithm `layout::leaves` exposes — move it, do not re-derive it |
| the elastic-share fix (D57 amendment 19) | `ui/src/renderer/mod.rs` | landed — this arc **caps** what that shares out; keep its byte-identity below the content width |
| "appears when there is room" (`ADDED_AT`) | `components/src/disk/view.rs` | landed (arc 14) — copy it for the sensors gauge pane; `disk` already obeys D65 §1 |
| the tile finder + `coverage()` | `cli/tests/smoke.rs` | landed (D62) — extend the floors, do **not** rewrite the finder |

## 15a — the rule, the renderer and the instruments

**Seam 1 — the elastic cap (`ui/src/renderer/mod.rs`, `fn table`).** After `spare` is computed, cap each elastic column at the widest cell it holds, **measured over every row, never the visible page**. The leftover is not redistributed and not drawn. Two invariants to pin: a table whose content exceeds the rect renders **byte-identically** to today (the share is smaller than the cap, so the cap never binds), and a single-elastic table with short content ends at its content. Six tables change shape — `sensors`' `sensor`, `net`'s `iface`/`local`/`remote`, `audio`'s sink picker, `winamp`'s playlist. Four do not: htop, gpu, `disk`, `sources`.

**Seam 2 — `Command` is last.** htop's and gpu's `fit_columns` copy the user's `columns` order and only *append* `Command` when absent, so `columns = ["Command","PID"]` puts the elastic first and the drop order incoherent. Move it to the end unconditionally; one unit test per component with a reordered `columns`.

**Seam 3 — chart furniture (`ui/src/renderer/mod.rs`, `fn chart`).** Before any series: horizontal gridlines at the quarters of `bounds.y`, `Role::TextGhost`, glyph from the theme's tier. Height gate: `>= 8` rows → 25/50/75; `>= 4` → the midpoint alone; below → none. **Unlabelled** — `Bounds` carries no unit. Then, after the series, each `Series.label` at that series' newest point (rightmost, or leftmost when reversed), clipped, styled from the series' gradient at that point's height. Pure renderer work; no component and no `View` change.

**Seam 4 — `ui::layout::leaves(view, area)`.** The split `stack` computes, exposed, with `Stack` recursed into and every other variant a leaf, carrying whether the chain to the root contains a `Fill`. `renderer::stack` calls it so there is one implementation. **This is the one item near the contract line: additive, no signature changed.** If review vetoes it, the fallback is a testkit copy of the split, accepting the drift — say which was taken in the report.

**Seam 5 — the two assertions (`ui/src/testkit.rs`).** `assert_every_drawing_grows`: walk `leaves`, and for each leaf that is `Bars | Sparkline | Chart | Gauge | Segmented` **and** has a `Fill` in its chain, render it alone at its rect and at double, requiring the existing ratio. Exempt, each with a reason named at the call site: `Len`-only chains, `View::Custom`, and text leaves. `assert_tables_end_at_their_content`: at four times each tier's `min`, every `View::Table` leaf's summed drawn width must not exceed its natural width plus one pad per column — this catches a *component* handing the renderer a `Fixed(999)`, which the renderer cap cannot.

**Tests for 15a.** Renderer unit tests for the six width cases and the three gridline regimes; both new assertions run over **every** component in the registry, not a list; the existing `assert_grows_with_area` calls stay unchanged and green. Regenerate snapshots with `scripts/shots.sh` and **land the churn in its own commit**.

## 15b — what each tile draws with the room

**Seam 6 — `net` (`components/src/net/view.rs`).** (1) The interface band becomes `Constraint::Len(min(rows + 1, body * 2 / 5))`, not `Fill(2)`. (2) **Both scroll viewports come from the band the table was given** — today the connection table computes its viewport from the tile's inner height (39 rows against a 23-row band, so a cursor past row 35 is off-screen) and the interface table from its share minus one. Pass each band's row count in; a pty case drives `↓` past the fold on both and asserts the selected row is drawn. (3) The **mirrored chart**: per shown interface (busiest four), two `Series` over rx and tx — rx as `+v`, tx as `−v` — `Bounds.y = (−m, +m)` where `m` is the max over both, window = the run's age capped at the tile's span (**never a fixed span**, D62 amendment 1). `Fill(1)` at `table`, and at `conns` a `Fill(1)` above the connection table's `Fill(3)` once the inner height reaches 20.

**Seam 7 — `gpu` (`components/src/gpu/view.rs`).** `spec_column` becomes a strip: chunk the rows into `width / (SPEC_COLUMN_W + 1)` side-by-side `KeyValue`s in a `Stack{H}`, placed as `Len(⌈rows / cols⌉)` **under** the chart, gated on the existing width threshold. The chart then gets the full width. **The option keeps the name `spec_column`** — D63's config-audit test has a row for it and will fail on a rename. Separately, `body_rows`/`band_rows` reserve `min(table_rows, rows the table has)`.

**Seam 8 — `sensors` (`components/src/sensors/view.rs`).** When the width exceeds the table's natural width by a threshold, the tier becomes `Stack{H, [(Len(natural), table), (Fill(1), gauges)]}` where `gauges` is a `Stack{V}` of one `Len(1)` `View::Empty` header spacer plus one `Len(1)` `View::Gauge` per row (value = the `heat` the table sorts by, `text` = the percentage, gradient `Temp`). The `of lim` column **stays** — it is the sort key, and an invisible sort key is a puzzle (the arc-5b review's finding); the gauge is its picture, not its replacement.

**Seam 9 — the two "nothing" verdicts, written down.** `audio` and `sources` get **no code change**; §8 and §8.1 are already corrected. Do not "improve" either.

**Tests for 15b.** `snapshot_matrix!`; `assert_renders_everywhere` with `signature(tier)`; both new 15a assertions over the changed components; the growth sweep re-run with **the net `table` tier's height axis newly excluded and its ratio named in the comment** (a braille line lights ~1 cell per column however tall — the gpu `charts` exclusion verbatim) and its **width** axis newly asserted. `a_wide_terminal_fills_its_tiles` gains NETWORK and SENSORS and raises GPU, floors at 0.6× the measured value, **with a comment saying the NETWORK number is fixture-shaped** (the synth publishes a handful of connections against torch's 109 sockets and 9 interfaces, measured 2026-09-09). **No floor for SOURCES, AUDIO or PINS.** A pty case at a very wide size confirming the sensors bars line up with their rows and the gpu spec strip prints its last row.

**Docs for 15b.** The D65 ARCHITECTURE and PARITY edits are already applied; 15b adds `PERFORMANCE.md`'s re-taken P19 pair, regenerates `COMPONENTS.md`, and writes `CHANGELOG.md`, `PLAN.md` and `BACKLOG.md` (the `P2` struck as pulled, plus five new entries: the NetworkIO sum, the `sources` cadence/demand seam, the sensors per-row sparklines, the synths' happy-path bias, and PINS at 120×40 as an undiagnosed observation).

## Numbers

- **P19** re-taken uncached at 250×70 **and** 480×135 with the gridlines and gauges on. The control is arc 12's **1.02 ms** for an uncached 480×135 frame against an 8 ms p95. The new work is bounded: ≤ 4 × width cells per chart, one `set_string` per series label, one `Gauge` row per sensor. If 480×135 passes 2 ms, say so and find out why — nothing here is superlinear.
- **The coverage table, before and after**, at 480×135 and 250×70, all seven tiles, in the report and in the test comment. **Expect NETWORK's and SENSORS' column fractions to move in both directions** — the table narrows (down) and the chart or bars fill (up). Record both halves, not the net.
- **No source, cadence or `Detail` change**, so P2, P4, P5, P13, P15 and P17 are unchanged and asserted so, not re-measured.

## Traps — do not rediscover

1. **Fixing a stretch lowers the coverage numbers.** Do not re-fit a floor to a number you just improved, and do not widen anything to raise one.
2. **Gridlines go under the series** — the braille mask is written after, so ink wins a contested cell. That is correct.
3. **Four gridlines in a four-row band is a box of dashes.** Honour the height gate.
4. **The mirrored chart's zero line *is* the midpoint gridline.** 15a first.
5. **The mirrored chart will fail the growth sweep's height axis.** Name the exclusion with its measured ratio; do not invent ink.
6. **Measure an elastic column over every row, not the page** — a column that changes width as you scroll is worse than one that stretches.
7. **`fit_columns` copies the user's order.** `Command` last, unconditionally.
8. **`spec_column` keeps its name.** D63's config audit walks every leaf key of the shipped default.
9. **The gpu chart band moves when the process list changes length.** Accepted; no hysteresis.
10. **Do not touch the demo synths.** A fixture change churns every net snapshot and is its own backlog item — say what the NETWORK floor is measuring instead.
11. **The chart window is the run's age capped at the tile's span**, never a fixed span (D62 amendment 1; D64 trap 8 caught it coming back in a new component).
12. **Six tables and every chart churn at once.** 15a's snapshot commit lands alone.

## Escalate rather than improvise

A new `View` variant, `Unit`, `Capability`, `Control`, `Manifest` field or `Detail` use. Any change to `ColWidth`, `Bounds`, `Series` or `Column`. Any change to `SourceOverview` — that is the `sources` tile's demand column, and it is a Fable session's. A `Component::blank_budget(tier)` or any other API for "this rect may be empty" (D65 §9 rejects it). A new `Role`. A theme `[widgets]` key for the gridlines. Numbering the chart's y axis.

## Gates

`scripts/gate.sh` green (`scripts/shots.sh` regenerates `KEYS.md` **and `COMPONENTS.md`**; the drift step fails until both are). The pty suite occasionally flakes under parallel load — re-run once before treating a pty failure as real. Commit before review; the review workflow from `docs/REVIEW.md` with the read-only guard in every agent prompt, and a lens that renders **every** component tier at 480×135 and 250×70 and answers D65 §9's three questions in order. Scoped commits (`ui:`, `components:`, `docs:`), 15a's before 15b's so each half reverts alone; push `main` and watch CI. `v0.15.0` and the `Cargo.toml` version are Matt's.

## Done when

**15a:** no `View::Table` anywhere in the registry draws a column wider than its widest cell at four times its tier's minimum; a single-elastic table below its content width renders byte-identically to `main`; a chart of eight rows or more carries three gridlines and each series is labelled at its newest point; `assert_every_drawing_grows` runs over every component with each exemption named at its call site. **15b:** at 480×135 the `net` tile draws a mirrored rx/tx chart in the rows its interface band no longer takes, `↓` past the fold keeps the selected connection on screen, the `gpu` spec strip prints every row with the chart at full width, and the `sensors` bars sit on their rows with the table ending at its content; `audio` and `sources` are unchanged and *named* in §8; the before/after coverage table for all seven tiles at both sizes is in the report and in the test comment. **Matt's rows:** the tag, the version bump, his real Ptyxis size for `MATT_TERMINAL`, and everything already owed.
