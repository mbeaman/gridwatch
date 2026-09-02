> **Status: adopted 2026-09-01 (D46), after arc 1b.** How gridwatch is tested,
> and why the harness has the shape it has. §12 of `ARCHITECTURE.md` lists the
> test *kinds*; this file is the contract for what each layer must assert and
> the retrospective that made the contract necessary.

# Testing — what the harness must catch

## The retrospective that wrote this file

Arc 1b passed every gate: fmt, clippy, 54 tests, doc, MSRV, schemas, deny,
audit, two adversarial reviews (20 + 1 agents), and CI on a clean runner. Then
Matt ran the binary and hit two failures in the first thirty seconds:

1. **A non-tty exited silently.** `run_terminal` redirected stderr into the log
   file *before* opening the terminal, so crossterm's "No such device or
   address" went to a file nobody was looking at. The process looked like it
   did nothing.
2. **A too-small terminal drew a blank screen.** `overlay::too_small` had
   existed since arc 1a and nothing called it; `draw_frame` just returned.

Neither is subtle. Both survived because of the harness's shape, not because
an assertion was missed:

- **Nothing executed the real entry path.** Every test entered the app *below*
  `run_terminal` — `shot`, `shot_frame`, `run_loop<TestBackend>`. Terminal
  setup, the stderr redirect and the order between them had zero coverage, and
  the bug was an ordering bug in exactly that function. The perf runs used a
  `script` pty, which *is* a tty, and walked around it too.
- **Tests asserted the absence of crashes, never the presence of content.** The
  no-panic sweep covered every size from 0×0 up and asserted only "didn't
  panic". A blank frame passes that. Snapshots pin content, but only at the six
  real grid sizes, so the too-small branch was never pinned either.
- **Error visibility was never a test subject.** We tested that the log file
  exists; nobody tested that a startup failure reaches the person.
- **The reviews inherited the same blind spot**, because the review prompts
  told them to verify frames with `shot`. Nobody launched the binary.
- **Resize was believed, not tested.** The design handles it (a live pty
  resized six times mid-run re-derived mode, grid and tiers correctly), but no
  test drove `InputEvent::Resize` through the shell.

The harness was built for determinism and speed — headless, seeded, byte-exact
— and the properties it lost were the ones that only exist at the boundary with
a real terminal and a real person. Those are a different axis, and this file
adds it.

## The five layers

Every layer is a CI gate. The layers are cumulative: a defect should be caught
by the cheapest layer that *can* see it, and each higher layer exists for what
the ones below structurally cannot observe.

### A. Content oracles in the component sweeps (`gridwatch-ui` testkit)

`assert_never_panics` is now `assert_renders_everywhere`. For every size from
0×0 to the richest tier's minimum plus margin, it asserts:

- **No panic** (as before).
- **Non-blank when the rect fits tier 0 — on a full store and on an empty
  one.** A component handed at least its declared minimum must put at least
  one non-space glyph in the buffer; with no data that glyph is `—` or a
  "waiting" line. (The first draft of this rule said "with data", and a
  mutation check showed it let the arc-1b blank tile through.)
- **The tier's signature is present.** Each component supplies
  `signature(tier) -> &[&str]`: short strings that *must* appear whenever that
  tier is chosen with data in the store (htop: `CPU` at tier 0, `MEM`/`SWP` at
  `meters`, `CCD` at `cores`). This turns `Tier.adds` from a comment into a
  checked claim.
- **Below the minimum, only "no panic" is required** — the shell owns the chip
  there and never asks the component for a tier it cannot fit.
- **No fabricated data.** With an *empty* store, the buffer must not contain a
  digit followed by `%` — a dash, a "waiting" line, or blank is honest; `0%` is
  not.

### B. App-level size lattice and resize (`gridwatch-app` tests)

`shot_frame` over a lattice of terminal sizes (widths 1…300, heights 1…80,
coarse steps plus every threshold from §6: 20×3, 109×27, 131×37 and their
neighbours), both pages, all three themes, through one `frame_lint`:

- the frame is non-blank;
- below the shell's minimum the too-small notice is present and nothing else;
- above it, every placement either renders its component's content or a chip
  with a reason — never an empty framed box;
- the tab bar is present in configured mode and absent in dense.

Plus **resize as a sequence**: one `Shell` fed `InputEvent::Resize` events
through `run_loop<TestBackend>` at 60×20 → 200×45 → 40×8 → 250×50 → 158×1 →
131×38, asserting the mode, the cpu tile's tier and the notice at each step.
Resize must never leave stale cells: the frame after a shrink contains nothing
from outside the new area.

### C. The binary under a pty (`gridwatch` CLI tests, `pty.rs`)

Spawn the built binary the way a person does, under util-linux `script`
(present on every Linux box and on the CI runner; no new dependency). Each test
reads the pty stream and asserts what the user would have seen:

1. **stdout not a tty** → exit 1 and the explanation on the *inherited* stderr
   within one second.
2. **One-row pty** → the too-small notice in the stream within two seconds.
3. **60×20 pty** → the cpu tile's text within one second (P18's first-frame
   gate as a test rather than a perf row).
4. **Resize mid-run** (`stty` on the pty at three sizes) → the tile's tier
   follows.
5. **`q`** → exit 0, the alternate-screen leave sequence in the stream, and
   `gridwatch.log` contains no `ERROR` line.

Focus events cannot be produced under `script`; P4/P21 remain a human row, and
they are the *only* one.

Arc 2a's review found the shape of a hidden-by-the-fixture defect worth naming: the store-level replay tests fed *monotone* synthetic entries while the recorded fixture was not monotone, so `apply_until` looked correct until `shot --replay --at 0.2` was run by hand. A fixture recorded by the real binary is the oracle; a synthetic one is a convenience. Arc 2a added three cases: C.6 `--demo` draws the process table (`Command` and the synthetic game row reach the terminal), C.7 `--replay fixtures/journals/torch-idle.jsonl --speed 0` reaches a frame and the sources tile reports the end of the journal, C.8 `--record FILE` writes a header and batch lines with the tables off. The testkit's `render_component` / `view_snapshot` now run the component's `tick` before `view`, as the shell does — a table tier derives its rows there, and a sweep that skipped it saw an honest but empty table.

Arc 2b added the GPU to every layer without a GPU in CI: the source's tier logic is written over a `Probe` seam (`sources/gpu/probe.rs`) so `crates/sources/tests/gpu.rs` drives the poller with a scripted fake — pruning after one `NotSupported`, the PCIe counter diff, the `last_seen` carry-forward, `InsufficientSize` keeping the previous rows, `GpuLost` aborting the tick, the plan's grids — and two `#[ignore]` live tests (`live_nvml_pass_is_inside_p11`, `live_call_costs`) print the real numbers on torch. The component's tests (`crates/components/tests/gpu.rs`) pin the tier per real grid size, the §8.1 row budget, gridwatch's drop order, the join with its last-known cache, nvtop's 30 s ENC/DEC timer on the store's clock, and the keys; the D46 sweep and the snapshot matrix include `gpu`; the testkit's `demo_store` feeds both synths. C.6 now asserts both tables (`GPU MEM`, `Both G+C`) under the pty, and the shell lattice zooms the gpu tile by double-click to check the joined columns arrive with no htop tile visible. The first live pass is the layer-D lesson of this arc: every unit test passed while P11 read 29 ms/s, because the device handle was fetched per call — only the ignored live test could see it.

Arc 3a added the pins source and the alert overlay the same way: the source's loop runs over a `PinsBackend` seam so `crates/sources/tests/pins.rs` drives the `Sampler` with a scripted chip — the seam keys, the real astral-watch `Lifecycle` through a 3-of-5 raise and a 20-clean resolve, telemetry loss with the redetect at ten misses — and one `#[ignore]` live pass that **a human runs** (it opens `/dev/i2c-*`, which an agent must not). The component's tests pin the tier per rect, tui.rs's balance classes, the limit line's role-only styling, the `Command::Source` the `+` key emits, the scripted overload reaching every tier, and the `full` header with and without the gpu source. The shell test `the_alert_banner_is_on_every_page_and_acknowledges` covers the banner on both pages, the heartbeat pulse, `a`, the re-raise, the `Resolved` toast, the Warn-only chip and the `A` overlay; the replay test finds the banner on page 2 at the fixture's scripted instant; pty case C.10 waits for the banner under `--demo` and opens the overlay. The lattice caught two overlay defects before they shipped: the banner stealing the only tile's row at 15×7 (it now yields below one tile's height) and toasts covering that tile (they yield below six body rows).

Arc 3b's layers: the loader v2 is pinned in `crates/ui/tests/ui.rs` (`inherits` key by key, chains refused, per-kind overrides, the WCAG gate on the brief's known pairs — 21:1 and 4.54:1 — and the `terminal` palette by name; swatch snapshots for the two new built-ins); the shell tests (`crates/app/tests/shell.rs`) drive `reload_from_texts` — an inverted htop sort survives a same-file reload, changed options rebuild exactly that instance, a broken file keeps the state and the toast names `config.toml:2:`, a one-page layout pulls the shell back to page 1 — the theme following the config unless locked, a `[components.htop]` override reaching the rendered cells, the `STALE Ns` badge appearing per source at 3 × its cadence on the virtual clock and vanishing under pause, and the chip's reason-and-fix lines with an empty `CapSet`; the watcher's decision (`watch::judge`) is a unit test without a thread. Pty cases: **C.12** starts `--demo`, writes `config.toml` with another theme and sees the reload toast and the new theme name inside three seconds, breaks the file and sees `kept the old config … config.toml:3:`, writes `layout.toml` and sees its toast, presses `T`; **C.13** runs `gridwatch doctor --offline` and checks the rows. `--offline` is deliberate: the live probes open `/dev/i2c-*`, which no test on torch may do — the live doctor table is a human's run, like P14.

Arc 4a's layers: `unit_at` is pinned against `solve` by a proptest over both grid modes and every body size (every cell of every tile maps back into its placement; the ghost rect equals the outer rect); `crates/app/src/edit.rs` unit-tests the undo/redo cap, the key decoding (both spellings of `Ctrl-h`/`Ctrl-j`), the picker's ordering and filter, and the drag's proposed geometry; `crates/app/src/save.rs` unit-tests comment survival outside `place`, a removed and an added placement, the rebuilt array on a page-count change, the re-parse refusal and the atomic write; the shell tests drive every key path through `handle_input` and read the key bar (`EDIT · cpu @ (0,0) 6×3`), the red ghost after a collision, discard restoring a byte-identical frame, the picker adding a `kind:sources` tile, `save_layout_to` writing a commented file that reloads to the same pages with the watcher's hash observed on the ignore sender (the slot's `judge` is the 3b unit test), the mouse drag/corner paths, and — from the 4a review — the ghost on the attempted rect, gutter-only dots, the deferred page change, the reload re-baseline and the scrolling picker. `crates/app/src/edit.rs` also carries the proptest that the drag paths never yield an overlapping or out-of-grid page. Pty **C.15** narrows and moves the cpu tile under `--demo`, presses `w`, reads `at = [2, 0]` from the sandbox's `layout.toml`, checks `config.toml` was never written and that the watcher did not reload the app's own write.

Arc 4b's layers: `crates/app/src/effects.rs` unit-tests each hook building from retrowave's spec, staying inside its area, a bounded one finishing and the pulse repeating, and the watchdog tripping on a sustained overrun; `crates/app/src/flourish.rs` pins the empty-run computation; `crates/app/src/ambient.rs` carries the §12.2 list — determinism (two layers from the same theme fed the same molds are byte-identical frame for frame), readability (a pinned rect is the mold with no rain glyph over it), fade (a lit cell walks to the floor and stops), sweep (every content cell reaches full light at least once per `sweep_s`), re-light (a changed mold cell is lit next frame), composition (with the pool emptied every unlit content cell is dark; under `L` every content cell is the mold) and the governor's step-down/recovery; `crates/ui/tests/ui.rs` pins the parsed tables, the quiet-theme `[ambient]` refusal, effect validation and bounding, `contrast.autofix`, and the two new swatches; the shell tests render `--theme matrix` (rain glyphs present, the banner text present through a sweep with the overload active, pause freezes two frames identical, `V` and `L`). Pty **C.16** runs `--demo --theme matrix` and waits for katakana within two seconds and the banner text near 22 s; **C.17** runs `--no-effects --demo` and checks no effects notice and a clean exit.

Arc 5a's audio vertical: `crates/sources/src/audio/dsp.rs` carries the §12.1 tests (a full-scale 1 kHz sine lights its band ≥ 0.97 and reads −3.01 dBFS RMS, a bin-centred 52.7 Hz sine leaves exactly one dominant band below 100 Hz, silence is all zeros and a short history pads, the Hann `2/Σw` identity reads 1.0 for a bin-centred sine, the tilt and the config clamps, the peak hold's 1.5 s and 20 dB/s); `capture.rs` pins the verified `pw-record` command line (with `stdbuf -o0 … --latency 256` and `node.dont-fallback` under their options) and pumps a generated f32 stream through the ring across odd-sized reads and a full ring; `sink.rs` parses a `pw-dump` excerpt shaped like torch's (serials, never node ids; the `default` metadata's props at the top level); `supervise.rs` drives the `Policy` (spawn on the first visible demand, kill 10 s after hidden, respawn on EOF with the 250 ms → 5 s backoff, never on "no data") and the `Silence` rule (250 ms without a frame, 500 ms below the floor, back at once) on fixed instants; `mod.rs` runs the publisher over a ring of a 1 kHz tone and then silence, and the `#[ignore]` `live_pw_record_delivers_frames` captures two seconds on torch (a safe read-only probe). `crates/components/src/audio/ballistics.rs` tests the Winamp falloff and peak schedule and cava's integral/monstercat/gravity at 30 and 60 fps; `crates/components/tests/audio.rs` pins the tier per real grid size, the keys, the sink picker's `Domain` command (the boxed `SetSink` downcasts), `Animated` with sound and `Redraw::No` once silent and settled, the empty and `Unavailable` tiles, and the mirrored bar layout; the component snapshot matrix gained the audio tile at every grid size. The shell tests' staleness rows count the audio tile (its cadence follows `fps` and the silence rule). `crates/app/tests/replay.rs` replays `torch-audio.jsonl` (the silence path) deterministically. Arc 5b's sensors vertical: `crates/sources/src/sensors/hwmon.rs` walks `fixtures/hwmon/torch/` (copied from torch's `/sys/class/hwmon`) and pins the inventory — nine chips with `nvme#2`/`nvme#3`/`spd5118#2` numbered by name, the attribute-less `asus` skipped, `nvme:Sensor 1`'s `65261850` m°C `max` dropped as a sentinel, an unlabelled input falling back to its stem, and the unit divisions (m°C, mV, µW); `rapl.rs` drives the three states and the wrapping `Δenergy` on a temp tree; `sensors/mod.rs` samples the fixture (fifteen readings, the thresholds and the inventory once, nothing republished on an unchanged re-walk) and parses the options. `crates/components/tests/sensors.rs` pins the tier per real grid size, the hottest rule (the smallest margin to `max`, so an 80 °C NVMe outranks a 59 °C Tctl with no max), the over-max/over-crit marks, the `o` sort and the chip filter, the `full` tier's RAPL/PSI/gpu rows with and without the gpu source, and the honest empty tile; the component snapshot matrix gained the sensors tile. Pty **C.20** runs `--demo` and waits for a chip name on the Overview, then runs `doctor --offline` and checks the hwmon row (the walk is a sysfs read, so it runs even offline).

Pty **C.18** runs `--demo --page 2` (with and without `--no-effects`), waits for the `Hz` axis, checks bar glyphs keep arriving over 500 ms and that the stats log counts animation frames; **C.19** runs `--demo --fps 60 --page 2` and exits cleanly.

### D. Error visibility is a contract (§11)

Any failure **before** the alternate screen is entered must be printed to the
inherited stderr — the log file does not exist yet from the user's point of
view. Any failure **after** must be logged *and* surfaced in the UI where a
surface exists: a source failure is a status the `sources` tile shows and, on
transition to `Unavailable`, a toast. Layer C pins the first half; a shell test
that injects an `Unavailable` status and asserts the toast pins the second.

### E. The review process

`docs/REVIEW.md` Template A gains a mandatory **`user-path`** lens: launch the
binary in a pty and try to break it — no tty, one row, narrow, resize, quit,
kill a source, corrupt the config — and report *what the user saw*. `shot` is a
supplement for the `ux-theme` lens, never its only instrument. The gate
checklist runs `cargo test -p gridwatch --test pty`, and the arc report includes
that test's transcript, so "I ran it like you would" is evidence rather than a
claim.

## What still needs a person

Only what the machine physically cannot do: focus reporting from a real
terminal (P4/P21), the Ptyxis glyph check, the README screenshot, and the perf
rows beside the game. Everything else that reached a human in arc 1b now has a
gate.
