> **Status: requirements, revision 2 (2026-08-30) — no measurements yet.** Revision 2 applied an adversarial verification pass: the wake-up and byte counters are now measured per thread and from `/proc/<pid>/io` (perf and strace are blocked unprivileged on torch), the NVML and `/proc`-scan ceilings were re-derived from the measured call costs, and two CPU ceilings were raised honestly (D26). The measured tables at the bottom are filled in per arc on torch; CI cannot see Ptyxis or the GPU, so these are manual gates that block an arc's commit like a failing test would.

# Performance requirements

gridwatch runs beside a game on the same GPU, next to a 60-fps compositor, on a machine whose owner will notice a fan curve. "Not a hog" therefore has to be stated for **four consumers**, not one:

1. **The gridwatch process** — CPU time, wake-ups, memory.
2. **The terminal it drives** — every byte we write makes Ptyxis parse, re-shape and re-upload glyph runs, and Ptyxis is a GPU client (verified: `nvidia-smi` lists `/usr/bin/ptyxis` as `C+G`, 44 MiB). A chatty TUI is a GPU load by proxy.
3. **The GPU driver we poll** — NVML calls execute inside the driver; some block for milliseconds (`pcie_throughput` 21 ms per direction); `nvidia-smi` forks a 27 MB process per sample.
4. **The buses we share** — the GPU's i2c bus (astral-watch's chip), PipeWire's graph (a capture stream can change the quantum for everyone).

Every number below is a **ceiling in a release build on torch**, measured over 60 s, with the game running for the "beside a game" rows. Comparators measured today: a live `htop` at 1.5 s costs 3.0 % of a core and writes ≈ 1 KB/s (16-colour SGR); `nvtop` idles at ~1 %.

## Budgets

| # | Requirement | Ceiling | Measured how |
|---|---|---|---|
| P1 | gridwatch CPU, Overview page, visualizer silent or absent, beside a game | **≤ 2 %** of one core — derived: pid-level scan 15 ms / 3 s ≈ 0.5 %, ≈ 2 frames/s × ≤ 3 ms ≈ 0.6 %, NVML ≈ 0.4 %, other pollers ≈ 0.3 % | `pidstat -u -p $(pidof gridwatch) 1 60` → avg `%CPU` |
| P2 | gridwatch CPU, Overview with the visualizer at 30 fps | **≤ 6 %** of one core (≈ 2 % DSP + ≤ 1.3 ms mean per frame × 30) | same |
| P3 | gridwatch CPU, Audio page (6x3 visualizer) at 60 fps opt-in | **≤ 10 %** of one core | same |
| P4 | gridwatch CPU while the terminal is **unfocused** or `space`-paused | **≤ 0.3 %** — `FocusLost` drops every source to `Hidden` *and* `Meters` (the process scan stops; only pins at 1 Hz and the heartbeat remain) | same; focus via `EnableFocusChange` |
| P5 | Wake-ups per second, Overview, silent audio | **≤ 40 /s** — derived: gpu 2 + pins 2 + net 1 + sensors 1 + cpu 0.7 + probes ≤ 3 + audio idle 2 + mpris ≤ 1 + watcher 1 + heartbeat 1 + render ≤ 4 + tokio timer ≤ 2 ≈ 21, headroom for channel wakes; requires the zero-poll `sleep_until` of §4.3 and `[perf] phase_ms = 250` alignment | Σ over `/proc/<pid>/task/*/status` of Δ`voluntary_ctxt_switches` over 60 s (the leader-only file under-counts ~80×; nonvoluntary switches are preemptions, not wake-ups) — or `pidstat -w -t -p <pid> 1 60` summing the TID rows' `cswch/s`; `sudo perf stat -e sched:sched_switch -p <pid>` only as a privileged cross-check (`perf_event_paranoid = 4`) |
| P6 | Bytes written to the terminal, Overview, silent audio | **≤ 25 KB/s** at ≈ 2 frames/s — byte model per changed cell: `MoveTo` ≤ 10 B only at run starts, a truecolor fg+bg pair 38 B only when the colour changes, glyph ≤ 3 B; ≈ 11 B/cell amortised *if* adjacent cells share LUT entries and one SGR covers a run (the mechanism, not a hope) | Δ`wchar` from `/proc/<gridwatch>/io` over 60 s with `--stats-log` off (`strace -p` is blocked by `ptrace_scope = 1`); `F12` HUD counter must agree within 5 % |
| P7 | Bytes written with the visualizer at 30 fps (4x2 tile) | **≤ 600 KB/s**; changed cells ≤ 2 500 / frame (an 80×20 spectrum touches ~1 500) | same + HUD |
| P7b | Audio page, 6x3 visualizer (122×31) at 60 fps | **measured, not gated** in arc 5 (up to ~230 k changed cells/s; upstream saw 24–40 fps at 200×50 all-cells-changing) — the gate is set from the measurement in D26's follow-up | same + HUD |
| P8 | Frames per second, static content | **≤ 1 + Σ(visible source cadences after coalescing)**: ≈ 2/s on the Overview silent, 1/s on a clock-only page, **0** for pages not shown; every frame must be caused by a generation change, an animation, an effect or the heartbeat | `F12` HUD frame counter and cause histogram |
| P9 | Load imposed on Ptyxis, Overview, silent | Ptyxis **≤ +1 %** CPU; `nvidia-smi pmon` `sm` column for Ptyxis is `-` or `0` in all 60 samples (delta against gridwatch paused) | `pidstat -p <ptyxis pid> 1 60`; `nvidia-smi pmon -s u -d 1 -c 60` with and without gridwatch |
| P10 | Load imposed on Ptyxis with the visualizer at 30 fps (4x2) | Ptyxis **≤ +4 %** CPU; `sm` ≤ 1 in ≥ 57 of 60 samples (pmon's resolution is one integer percent) | same |
| P10b | Audio page at 60 fps | **measured, not gated** in arc 5 (see P7b) | same |
| P11 | NVML: call time per second per device | **≤ 6 ms/s** — sum listed so the gate is auditable: fast tier 2 × ≈ 20 µs; slow tier ≈ 2.3 ms (`samples(Power)` 1.5 at `header`+, PCIe counters 0.3, memory/enc/dec µs, fans %/RPM 2.4 ms ÷ 5 s ≈ 0.5); process rows ≈ 2.0 (lists 0.35 incl. count + fetch, utilisation 1.7) only at `Detail::Table` → ≈ 4.3 ms/s with a `procs` tile visible; never `pcie_throughput`; never a `NotSupported` field twice; `nvidia-smi` never spawned while NVML works | `tracing` span sums on the gpu thread, shown per call class in the `sources` tile |
| P12 | NVML: gridwatch is **never a GPU client** — NVML creates no context (verified: `nvidia-smi` and a running `nvtop` are absent from the process table) | 0 entries | `nvidia-smi \| grep -c gridwatch` = 0 (the full table lists graphics *and* compute clients; `--query-compute-apps` sees compute only), cross-checked by the gpu source filtering its own pid out of the v3 lists |
| P13 | Per-process GPU accounting only when shown | v3 lists and `process_utilization_stats` are called only while a gpu tile whose `demand(tier)` is `Table` or richer is visible | `sources` tile shows each source's demand level and detail |
| P14 | i2c (pins source) | **≤ 2 transactions/s** (500 ms block read, one transaction per sample), **≤ 1 %** of one core (the digest measured the root logger at 1.0 % on the older 36-transaction path; the block path is measured in arc 3 and recorded below); the interval is never configured below 500 ms | source transaction counter, `pidstat` |
| P15 | `/proc` scan | pid-level scan (`stat`, `statm`, dir `st_uid`, `cmdline` on first sight) **≤ 20 ms** wall per pass and **≤ 1 %** of one core amortised at its 3 s grid cadence (five file kinds measured 20 ms today for 665 pids; two kinds 10 ms); the `task/` walk (+30 ms) and htop's gated files (`smaps_rollup` alone 130 ms) only at `Detail::Columns` — a `full` tier, zoomed or `view = "full"`, with that column on | `tracing` span on the cpu thread, shown in the `sources` tile |
| P16 | Audio capture | `pw-record` child **≤ 1.5 %** at its ≈ 94 chunk writes/s (4096-byte chunks every ~10.6 ms regardless of `--latency`), DSP thread `gridwatch-dsp` **≤ 2 %** at 30 fps; PipeWire quantum **unchanged** at the default `latency = 1024` (no-game row only — the game pins the graph at 256); child killed **≤ 10 s** after the last visible audio tile | `pidstat -u -t -p <gridwatch> 1 60` (named threads), `pidstat -p <pw-record pid>`, `pw-top -b -n 1` / `pw-metadata -n settings 0` |
| P17 | Memory | RSS **≤ 60 MB** after 1 h (NVML maps ~20 MB — nvtop sits at 31 MB); store ≤ 32 MB (retention-capped); **no growth** between hour 1 and hour 24 (24 h replay at 100× speed) | `pidstat -r`, `/proc/<pid>/status` VmRSS |
| P18 | Startup | first frame **≤ 300 ms** (placeholder tiles); every source live **≤ 2 s**; NVML init, `detect_bus`, `pw-record` spawn and D-Bus connect never on the render thread | `--stats` prints both timestamps |
| P19 | Frame cost | draw + write **p95 ≤ 8 ms** at 250×70; **mean ≤ 3 ms** at the Overview's ≈ 2 frames/s and **≤ 1.3 ms** at 30 fps (render cache: only the animated tile re-renders; the whole-frame diff is ~0.3 ms); missed frames **< 1 %** | `F12` HUD (p50/p95, changed cells, bytes) |
| P20 | Effects (arc 4) | ≤ `budget_ms` (4 ms) per frame, area-scoped, ≤ 600 ms per event; ambient CRT off by default; `--no-effects` honours P6/P8 exactly | `F12` HUD effect column |
| P21 | Unfocused throttle | on `FocusLost` every animated tile drops to `unfocused_fps` (default 2), every source but `always_on` ones goes `Hidden` / `Meters`; restored on `FocusGained` within one frame; VTE 0.84 implements focus reporting (DECSET 1004 → `CSI I`/`CSI O`, verified in `vte.cc`) and crossterm 0.29 maps it to `FocusGained`/`FocusLost` | `F12` HUD; confirmed interactively once in arc 1 |

## Showcase class — ceilings that apply only while a `class = "showcase"` theme is active **and the terminal is focused**

The `matrix` theme is the first theme that is *supposed* to spend resources: rain over the whole frame at 24 fps. It does not get an exemption; it gets its own ceilings, and it gets them only while you are looking at it. The moment the terminal loses focus (or `space` is pressed) the ambient layer freezes and **P4 applies unchanged** — a showcase theme costs nothing while you are in the game.

| # | Requirement | Ceiling | Measured how |
|---|---|---|---|
| S1 | gridwatch CPU, Overview under `matrix`, focused, beside a game | **≤ 15 %** of one core (rain painter + whole-frame diff + write at 24 fps) | `pidstat -u -t` |
| S2 | Bytes written / changed cells | **≤ 3 MB/s** averaged over a sweep cycle, ≤ 7 000 changed cells per frame — droplet cells ≈ density × 17 500 (≈ 3 500 at 0.20) plus cells whose fade crossed a LUT step (≤ ~5/s per lit cell, none once at the floor) plus re-lit updated cells; a sweep briefly touches most content cells | Δ`wchar`, `F12` HUD (droplet / fade / re-light histogram) |
| S3 | Load imposed on Ptyxis (the "spend GPU" allowance, explicit) | Ptyxis **≤ +15 %** CPU; `pmon` sm **≤ 5** in ≥ 57 of 60 samples | `pidstat`, `nvidia-smi pmon` |
| S4 | Frame time at the rain fps | draw + write **p95 ≤ 16 ms**; above it the governor steps down (fps 24 → 16 → 12 → 8, density × 0.75, gutters-only) and the HUD says so | `F12` HUD |
| S5 | Unfocused / paused under `matrix` | **= P4** (≤ 0.3 %, no frames): the rain is frozen, not slowed — measured with the game focused | `pidstat`, HUD frame counter = 0 |
| S6 | Readability floor | a tile with an active Warn/Crit alert, the focused tile, the banner/toasts and the key bar are always fully lit and never rained over; every content cell is re-lit at least once per `sweep_s`; a cell whose value changes is re-lit immediately; `V` re-lights the page, `L` locks everything lit | readability / sweep / re-light tests (§12) + eyeball in arc 4 |
| S7 | Memory | ambient state ≤ 1 MB — per-cell `lit_at` (u32 × 17 500 ≈ 70 KB), fixed-size droplet pool; no growth | `pidstat -r` |

Quiet-class themes (`modern`, `retrowave`, `mono`, `terminal`, `phosphor-*`) never run an ambient layer; their event effects stay inside P20.

## Mechanisms that pay for the budgets

| Budget | Mechanism in the design |
|---|---|
| P1, P4, P8 | Generation-gated redraws and frame coalescing (§5): a frame is drawn only when a source a *visible* component needs advanced, an animated visible tile is due, effects run, or the 1 Hz heartbeat fires, and several advances inside one frame slot yield one frame. Demand levels `Paused / Hidden / Visible / Focused` and `Detail` per source; `Hidden` cadences 2–3× slower; gpu fast tier 500 ms visible (nvtop itself refreshes at 1 s). |
| P1, P2, P3, P19 | **Render cache** (§5): an instance re-renders only when its `(source generations, tier, rect, theme, zoomed, focused, animation frame)` key changes; everything else is blitted. |
| P5 | **Zero-poll waits** (`SourceCtx::sleep_until` parks on the control receiver — no 200 ms stop-flag polling) and timer phase alignment on `[perf] phase_ms = 250`, so one wake-up serves several sources; the frame clock is the only sub-250 ms timer and only while something animates. |
| P6, P7, P9, P10 | ratatui's cell diff plus style-run reuse (one SGR per run, not per cell); gradients are 64-entry LUTs so adjacent cells share styles; animated regions are the tile's inner rect only; `Role::Bg` is painted once and then diffed away. |
| P11–P13 | NVML tiers (fast 500 ms ≈ 20 µs; slow 1 s; fans 5 s; power trace only when drawn); PCIe from byte-counter fields 197/198 (0.3 ms) never `pcie_throughput`; `NotSupported` pruning, `InsufficientSize` retried; process accounting gated on `Detail::Table`. |
| P14 | astral-watch's block read (one transaction per sample, validated once per chip); exporter preferred when the service runs; `redetect` only after 10 misses. |
| P15 | Pid-level scan at 3 s on the grid; htop's own gating rule for expensive files and the `task/` walk, reachable only through `Detail::Columns`. |
| P16 | `node.passive` capture on the sink monitor; `--latency 1024` keeps the graph quantum; the child is killed after 10 s hidden and respawned only on visibility. |
| P17 | `Retention { max_len 2400, max_age 10 min }` per scalar series, `Vector` series short, `Record` latest-only, alert ring 500, art cache 8 × ≤ 256 px. |
| P18 | Capability probe ≤ 200 ms of cheap checks; every source initialises on its own thread; placeholder tiles first. |
| P21, S5 | crossterm `EnableFocusChange` → `InputEvent::FocusLost/FocusGained` → the app rewrites `Demand` (level and detail), the frame clock, and freezes any ambient layer. |
| S1–S4 | The ambient layer is a post-render pass over the frame buffer (no component re-render, the render cache holds); the governor watches p95 frame time and bytes/s and degrades fps → density → gutters-only with a 30 s recovery. |

## Measurement protocol (`scripts/perf/measure.sh`, arc 1)

Everything below runs unprivileged on torch (`perf_event_paranoid = 4`, `ptrace_scope = 1` — `perf` and `strace -p` are sudo-only cross-checks).

1. Release build, `gridwatch --page 1 --theme retrowave` in Ptyxis at the usual window size; note `stty size`. Threads are named (`gridwatch-render`, `gridwatch-cpu`, `gridwatch-gpu`, `gridwatch-dsp`, …) so per-thread rows are readable.
2. 60 s samples: `pidstat -u -r -w -t -p <gridwatch> 1 60` (per-thread CPU and voluntary switches); `pidstat -u -p <ptyxis>,<pw-record> 1 60`; Σ Δ`voluntary_ctxt_switches` over `/proc/<gridwatch>/task/*/status`; Δ`wchar` from `/proc/<gridwatch>/io` (with `--stats-log` off, or subtract the log's bytes); `nvidia-smi pmon -s u -d 1 -c 60`; `pw-top -b -n 1` and `pw-metadata -n settings 0` (no-game row); the `F12` HUD's p50/p95/changed-cells/bytes and the `sources` tile's NVML ms/s and scan ms, dumped by `S`.
3. Repeat with gridwatch paused (`space`) to get the Ptyxis/GPU baseline; subtract. The paused baseline keeps the pins tile at 2 Hz; either hide it for the baseline or accept it.
4. Rows: Overview silent · Overview + viz 30 fps · Audio page 60 fps · unfocused · zoomed htop `full` · zoomed gpu `full` — each with and without the game; from arc 4 also Overview under `matrix` focused and unfocused. The "beside a game" rows need Matt to start the usual game first — agents never launch one (CLAUDE.md) — and the `game` column records which.
5. The script appends a dated table to this file; the arc report quotes it; a red cell blocks the commit until fixed or the ceiling is renegotiated in `DECISIONS.md`.

## Gates per arc

| Arc | Must be green before commit |
|---|---|
| 1 | P1, P4, P5, P6, P8, P18, P19, P21 (no GPU yet); P21 confirmed interactively once |
| 2 | + P11 (with the Overview's `procs` tile visible, sum shown), P12, P13, P15, P17 (1 h); zoomed gpu `full` shows USER/CPU/HOST MEM for the game (cpu detail raised through `demand`) |
| 3 | + P14 (block-path number recorded); alarm banner adds no steady-state cost |
| 4 | + P20; edit mode idle = P8; **S1–S7 under `matrix`** (focused, beside the game) and S5 = P4 with the game focused |
| 5 | + P2, P3, P7, P9, P10, P16; P7b/P10b measured and a gate proposed |
| 6 | Winamp marquee at 220 ms steps stays inside P6 (it is ~40 cells) |
| 7 | probes and connection table stay inside P1/P5 |
| 8 | zoomed `full` tiers inside P15 (`Detail::Columns`, `task/` walk with `H`) and P19 |
| 9 | P17 at 24 h; packaged binary re-measured |

## Measured (fill per arc)

| date | arc | theme class | page / state | game | gridwatch CPU | wake/s | KB/s | frames/s | Ptyxis Δ CPU | Ptyxis sm | NVML ms/s | scan ms | frame p50/p95 | RSS |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| — | — | — | — | — | — | — | — | — | — | — | — | — | — | — |
