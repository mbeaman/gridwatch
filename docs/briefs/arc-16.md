# Arc 16 brief — "the instrument has to enumerate" (D67)

> Written 2026-09-13 with D67 by an Opus session; read D67 first — it decides everything below and this brief only says where each piece goes. **Arc 16 changes no seam.** No crate gains a public type, no `View` variant, no `Manifest` field, no `Detail` use, no component's `view` changes. If anything here seems to need one, **stop and escalate**. Build **16a, then 16b**, sequentially and for one reason: 16a moves the fixtures, and 16b writes the snapshots that measure them. Reverse the order and every baseline is written twice and means nothing the first time.

## What already exists (do not rebuild)

| thing | where | state |
|---|---|---|
| `every_registered_component()` | `components/tests/components.rs:282` | **landed in arc 15** with the comment "a new tile joins by being registered rather than by being remembered". This arc applies it to the test forty lines *above* it. Do not write a second one |
| `demo_store(seed, ticks)` | `ui/src/testkit.rs:17` | **one shared timeline** — every synth ticks together, and the only parameter is the tick count. There is no per-source fixture knob and you should not add one |
| `dump::cells(&buf)` | `ui/src/dump.rs` | landed — the styled cell dump the three existing cell snapshots use. Reuse verbatim |
| `render_component(...)` | `ui/src/testkit.rs` | landed — returns `(tier, buf)`; the cell snapshots call it |
| the `disk.info` Record's `partitions` | `store/src/demo/disk.rs:46` | partitions are **already named** there. What is missing is publishing them as *series*, which is the gap that hid the bug |
| `a_wide_terminal_fills_its_tiles` + `coverage()` | `cli/tests/smoke.rs` | landed (D62/D65) — its NETWORK floor moves when the synth does. Record the new number, do **not** re-fit it to flatter the tile |
| the five real grid sizes | `components/tests/components.rs`, `real_grid_sizes()` | landed — the snapshot sweep's size list. Unchanged by this arc |

## 16a — the fixtures

**Seam 1 — `demo::NetSynth` (`store/src/demo/net.rs`).** It publishes five connections and three interfaces while `conns()` *reports* `scanned: 103, attributed: 87`. Publish enough connections that the `conns` tier's cursor can be driven **past the fold** — that tier's band is at least 23 rows, so five never could, and D65 recorded the resulting pty case as unwritable with the shipped fixture. Publish enough interfaces that a filtered tile differs from an unfiltered one. Keep `attributed < scanned` (the existing assertion at `net.rs:423` is correct and stays), but make both consistent with what is actually published rather than describing a machine the fixture is not.

**Seam 2 — `demo::DiskSynth` (`store/src/demo/disk.rs`).** `DEVICES` is three whole drives. Publish **at least one partition as a device** — nine scalars under its own `{dev}` label, exactly as a drive gets — because a partition existing only as a string inside `disk.info` is what let every partition render as its parent drive with no test noticing. Then make **a device leave mid-run**: it is D61's risk row ("re-created from scratch after `max_age` with an empty chart") and nothing models it. Respect the source's own rules — the 16-device cap and the ordering drives-then-partitions-then-`extra` — so the fixture stays a thing the real source could have produced.

**Seam 3 — the survey (no code).** There are **eight** synths, not the seven `BACKLOG.md` says. For each of the other six — `CpuSynth`, `GpuSynth`, `PinsSynth`, `AudioSynth`, `SensorsSynth`, `MediaSynth` — one sentence: *what does this refuse to model?* Write the answers into the arc's ROADMAP status. Anything real becomes a `BACKLOG.md` item. **Do not fix them in this arc**; two synths is already the churn budget.

## 16b — the instruments

**Seam 4 — the view sweep enumerates (`components/tests/components.rs`, `view_snapshots_at_real_grid_sizes`).** Ten hand-written blocks become a loop over `every_registered_component()`. The blocks carry a per-component store choice *and a comment saying why*; both move onto a **per-kind tick count** table (D67 §5). `net` gains view snapshots at all five sizes, which it has never had.

**Seam 5 — the cell snapshot means what it says (`rendered_cells_snapshot_modern_only`).** Its comment claims "one per component" and it covers three of eleven, so arc 15's gridlines and series labels are pinned in cells for no chart at all. Add a cell snapshot for every charting tile — gpu, disk, net, sensors, pins, audio — **at its charting tier**, not at whatever size happens to be first. Then the comment is true; fix it if it still is not.

**Seam 6 — the two documents that name tiles.** A test asserting every registered kind is named in `README.md` and in `wiki/Tiles.md`. It is a **check, not a generator** (D67 §3): the per-tile prose is the value. Match on something stable — the kind in backticks, as both documents already write it — and make the failure message say which kind and which file, because the person who trips it is adding a component and has not read this brief.

## Numbers

- **11** registered components; **10** are in each of the two hand-written lists and **`net` is in neither**; **3** have cell snapshots (clock, sources, htop ×2); **0** charts have one.
- **8** demo synths (`BACKLOG.md` says seven — corrected by this arc): cpu, gpu, pins, audio, sensors, media, net, disk.
- **64** snapshot files today under `components/tests/snapshots/`.
- Tick counts in use: **3** (`demo_store(42, 3)` — clock, sources, audio) and **40** (`demo_store(42, 40)` — htop, gpu, pins, alerts, sensors, winamp, disk). One tick is 1.5 s, so 3 ticks is 4.5 s and 40 is 60 s.
- The windows those two numbers exist for: the audio synth is **silent for the first 1.5 s**; the pins synth's scripted overload **raises at 21.5 s and resolves at 50 s**.
- `NetSynth` reports `scanned: 103, attributed: 87` against **5** published connections and **3** interfaces.

## Traps — do not rediscover

1. **One store for everything is the trap that makes this arc look done and the suite worse.** `demo_store` is a single timeline; hand every component the same tick count and audio snapshots its silence, pins and alerts snapshot a log with no overload in it, and htop's sparkline is three samples in one bucket. Those snapshots then get accepted and pin emptiness forever. The per-kind tick count is not a nicety.
2. **A generous default defeats the point.** If an unlisted kind falls back to 40 ticks, every future component passes while possibly showing nothing. State the default, and make a component with animation or a scripted event name its own count.
3. **Write net's snapshots *after* the synth change.** They do not exist, so there is no baseline to preserve — and if you create one against the old fixture you will accept it twice.
4. **The NETWORK coverage floor will move.** That is the fixture getting richer, not the tile getting better. Record the new number with a sentence saying which, and do not let it drift upward as a reflex.
5. **A partition is not a drive.** The source publishes drives first, then partitions, then `extra`, within a 16-device cap. A fixture that ignores that ordering tests a source that does not exist.
6. **`assert_renders_everywhere` above the snapshot function is the same hand-written list** (`components.rs:73–82`) — the same ten kinds in the same order, **also missing `net`**. So `net` has never been in *either* sweep. It is in scope for seam 4 and is two lines from the same fix.

## Escalate rather than improvise

Any of these means stop: a snapshot that cannot be made deterministic; a component whose `view` you want to change to make it snapshot cleanly (the instrument is wrong, not the tile); a need for a per-source fixture parameter in `demo_store`; a check in seam 6 that wants the README's prose restructured to be matchable.

## Gates

`scripts/gate.sh` green. `replaying_a_fixture_twice_is_byte_identical` unchanged. No P-row moves and the arc says so; **P17 re-taken under `--demo`** because a richer `NetSynth` puts more series in the demo store. Every accepted snapshot of an animated or scripted tile is **lit** — checked, not assumed.

## Done when

Registering a twelfth component and running the suite fails in **four** places — view snapshot, cell snapshot, `README.md`, `wiki/Tiles.md` — rather than passing silently. That is the arc's whole thesis and the one thing to verify by hand before reporting.
