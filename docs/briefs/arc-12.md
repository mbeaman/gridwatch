# Arc 12 brief — "the wide terminal" (D62)

> Written by the Fable session that wrote D62 (2026-09-06), for an Opus session. Read D62 first: it decides everything below, and this brief only says where each piece goes. **Never change a seam** — `View`, `RenderCx`, `resample`, the `Component` trait and `Manifest` are all unchanged by this arc; if a fix seems to need one, stop and write the tier's name into your report instead.

## The bug, reproduced

```
cargo build --release
./target/release/gridwatch shot --size 480x135 | sed 's/\x1b\[[0-9;]*m//g' | sed -n 1,46p | cut -c1-240
```

Compare with `--size 250x70`. Three things are wrong, all inside components or the renderer, none in the layout engine:

1. `crates/ui/src/renderer/mod.rs::sparkline` skips a `None` column, so `htop::view::spark` and `gpu::view::util_spark` (one bucket per column over a fixed span) draw scattered ticks once there are more columns than samples. Visible at 250×70 already (every other column of the CPU tile's sparkline is blank).
2. `crates/components/src/htop/view.rs::geom` tries `tw` from 4 down to 1 and takes the first that fits, so at 240 columns a CCD half draws 86 cells, left-aligned.
3. `crates/components/src/gpu/mod.rs::band_rows` clamps the chart band to `BAND_MIN..=BAND_MAX` (4..=8); `crates/components/src/htop/view.rs::cores` clamps the header quarter to 8.

## The three fixes

**F1 — the renderer holds (D62 §1).** In `sparkline`, keep a `last: Option<f32>`; for a `None` column after the first `Some`, draw `last`. Columns before the first sample stay empty. Nothing else changes: not the gradient, not `max`, not the `offset` logic for a series wider than the area. `dump.rs`'s JSON description of the view is untouched (it describes the tree, not the paint). Add a renderer unit test next to the existing sparkline tests: `[Some(1), None, None, Some(0.5), None]` at width 5 draws five columns with heights 1, 1, 1, 0.5, 0.5, and `[None, None, Some(1)]` draws one.

**F2 — the bars fit (D62 §2).** In `geom`, replace the `(1..=4u16).rev()` search with the largest `tw ≥ 1` such that `g.width(cores) <= block_w` (closed form: `tw = (block_w − cores·inner − (cores−1)·outer) / (2·cores)`, floored at the `FLOOR` geometry when it comes out below 1). Then centre: `ccd_block` prefixes the values *and* the label row with `(width − g.width(cores)) / 2` empty cells (a zero-height bar and spaces respectively), so the ids stay under their pairs. Read `ccd_block` and the label-row builder fully before touching either — they share the geometry through `Geom`, and the PARITY row for the per-core layout is exercised by the htop tests.

**F3 — bands from the rect (D62 §3).** `band_rows`: below `TIER_PROCS`, `max(inner_height − HEADER_ROWS, BAND_MIN)`; at a table tier on the grid, `max(inner_height − HEADER_ROWS − 1 − table_rows, BAND_MIN)`; zoomed, `max(inner_height / 3, BAND_MIN)`. `band_rows` does not take `zoomed` today — thread it through from the two callers (`charts` is never zoomed-specific; `table` has `cx.zoomed`) and from `body_rows`, which already takes it. Delete `BAND_MAX` or keep it as the documented *floor* semantics only — no constant may cap. In htop's `cores`, `(h / 4).clamp(2, 8)` becomes `(h / 4).max(2)`. Then check what the taller header does to `two_column_panel`'s sparkline: it should simply be taller. `HEADER_ROWS` stays 8 (nvtop parity), and `power_trace` stays `Len(3)`.

## The rule and its test

**T1 — `assert_grows_with_area`** in `crates/ui/src/testkit.rs`, beside `assert_renders_everywhere`:

```rust
pub struct Growth { pub tier: usize, pub width: bool, pub height: bool }
pub fn assert_grows_with_area(mk: &dyn Fn() -> Box<dyn Component>, data: &Store, th: &Theme, growth: &[Growth])
```

For each entry: render at the tier's `min` (`render_component` with `zoomed = false`, or `true` for a `zoom_only` tier), count non-blank cells; render at double the width (same height) when `width`, and at double the height when `height`; assert each count is ≥ 1.5× the base, with a message naming the component, the tier and the two counts. The base render must actually land on the listed tier — assert the tier index `render_component` returns, so a doubled rect that steps *up* a tier is caught (double the rect but pass the listed tier's `view` preference, or double only within the tier's range; pick whichever `render_component` supports without a seam, and say which in the report).

**T2 — the sweep.** In `crates/components/tests/components.rs`, list the tiers per built-in component that hold a `Fill` drawing — at least: htop `cores` (width and height), gpu `charts` (width and height), pins `trend` (width), net `sparks`/`table` (width), audio `spectrum` (width and height), sensors' chart tier if it has one. Anything the sweep fails that is not F1–F3 gets fixed the same way (a floor, never a cap) or escalated by name in the report. Do not weaken the ratio to make a tier pass.

**T3 — the headless case.** In `crates/cli/tests/` (see how `pty.rs` and the shot tests spawn the binary), run `shot --size 480x135 --format cells` (or the plain form and strip ANSI), locate the CPU and GPU tiles from the layout's known placements at that size (`gridwatch layout`-style helpers exist in `app`; otherwise the top-left 6x3 is the CPU tile in the embedded default), and assert the non-blank fraction of each tile's inner rect is above a floor you measure *after* the fixes and set with margin (say 0.6× the measured value), with the measured value in a comment. Add a second size constant `MATT_TERMINAL: Option<(u16, u16)> = None` with a comment pointing at `MACHINE.md`; when Matt gives the size, the same assertion runs at it.

## Snapshots

Regenerate with `cargo insta` (or however `docs/TESTING.md` says) and **read the diff** before accepting: at 250×70 the CPU sparkline becomes continuous, the core bars at 122 columns already fit at `tw = 3` and should not move unless the centring shifts them by one, and the gpu band at 6x3 (inner 31 rows: 31 − 8 − 1 − 10 = 12 rows) grows from 8 to 12, taking rows from nothing — check the table still shows 10 rows. Any other change is a finding.

## Numbers

`PERFORMANCE.md`: run the criterion frame bench at 480×135 (the 250×70 one gives 513 µs uncached, P19) and write the number in the log with the date. It is not a gate; it is so the next person knows what a wide frame costs.

## Traps

1. **Do not forward-fill in `resample`** — the braille chart and the store tests rely on `None`, and a gap in a chart is honest (D62 §1).
2. **Do not widen the span with the width** — rejected in D62 §1; a 1x1 and a 6x3 of the same key must show the same window.
3. **`assert_renders_everywhere` sweeps up to `max.w + 4` only** — that is why this bug never tripped a test. Do not raise its ceiling (it is O(w·h) renders); the growth test is the right instrument.
4. **A held sparkline and a stale source**: the flat tail is correct — the `STALE` badge is the signal. Say so in the CHANGELOG.
5. **Centring changes the label row** — the htop parity tests compare the id row; update them for the offset, do not drop them.

## Gates

`scripts/gate.sh` green; commit before review; the review workflow from `docs/REVIEW.md` with the read-only guard in every agent prompt; scoped commits (`ui: …`, `htop: …`, `gpu: …`, `testkit: …`, `docs: …`); push `main` and watch CI. `v0.12.0` and the `Cargo.toml` version are Matt's.
