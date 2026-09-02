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
