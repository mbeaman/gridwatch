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
| P5 | Wake-ups per second, Overview, silent audio | **≤ 40 /s** — derived: gpu 2 + pins 2 + net 1 + sensors 1 + cpu 0.7 + probes ≤ 3 + audio idle 2 + mpris ≤ 1 + watcher 1 + heartbeat 1 + render ≤ 4 + tokio timer ≤ 2 ≈ 21, headroom for channel wakes; requires the zero-poll `sleep_until` of §4.3 and each source's own **cadence alignment** — `next_deadline` is the next multiple of that source's period from the epoch, so sources sharing a cadence wake together (D63 retired `[perf] phase_ms`, which named a 250 ms grid that was never built; arc 7a measured 33 wake-ups/s with every source live, so the row holds without one) | Σ over `/proc/<pid>/task/*/status` of Δ`voluntary_ctxt_switches` over 60 s (the leader-only file under-counts ~80×; nonvoluntary switches are preemptions, not wake-ups) — or `pidstat -w -t -p <pid> 1 60` summing the TID rows' `cswch/s`; `sudo perf stat -e sched:sched_switch -p <pid>` only as a privileged cross-check (`perf_event_paranoid = 4`) |
| P6 | Bytes written to the terminal, Overview, silent audio | **≤ 25 KB/s** at ≈ 2 frames/s — byte model per changed cell: `MoveTo` ≤ 10 B only at run starts, a truecolor fg+bg pair 38 B only when the colour changes, glyph ≤ 3 B; ≈ 11 B/cell amortised *if* adjacent cells share LUT entries and one SGR covers a run (the mechanism, not a hope) | Δ`wchar` from `/proc/<gridwatch>/io` over 60 s with `--stats-log` off (`strace -p` is blocked by `ptrace_scope = 1`); `F12` HUD counter must agree within 5 % |
| P7 | Bytes written with the visualizer at 30 fps (4x2 tile) | **≤ 600 KB/s**; changed cells ≤ 2 500 / frame (an 80×20 spectrum touches ~1 500) | same + HUD |
| P7b | Audio page, 6x3 visualizer (122×31) at 60 fps | **measured, not gated** in arc 5 (up to ~230 k changed cells/s; upstream saw 24–40 fps at 200×50 all-cells-changing) — the gate is set from the measurement in D26's follow-up | same + HUD |
| P8 | Frames per second, static content | **≤ 1 + Σ(visible source cadences after coalescing)**: ≈ 2/s on the Overview silent, 1/s on a clock-only page, **0** for pages not shown; every frame must be caused by a generation change, an animation, an effect or the heartbeat | `F12` HUD frame counter and cause histogram |
| P9 | Load imposed on Ptyxis, Overview, silent | Ptyxis **≤ +1 %** CPU; `nvidia-smi pmon` `sm` column for Ptyxis is `-` or `0` in all 60 samples (delta against gridwatch paused) | `pidstat -p <ptyxis pid> 1 60`; `nvidia-smi pmon -s u -d 1 -c 60` with and without gridwatch |
| P10 | Load imposed on Ptyxis with the visualizer at 30 fps (4x2) | Ptyxis **≤ +4 %** CPU; `sm` ≤ 1 in ≥ 57 of 60 samples (pmon's resolution is one integer percent) | same |
| P10b | Audio page at 60 fps | **measured, not gated** in arc 5 (see P7b) | same |
| P11 | NVML: call time per second per device | **≤ 6 ms/s** — sum listed so the gate is auditable: fast tier 2 × ≈ 20 µs; slow tier ≈ 2.3 ms (`samples(Power)` ≈ 0.65 while a gpu tile is visible — D49, one batched PCIe-counter call 0.45, memory/enc/dec µs, fans %/RPM 6 × 0.85 ms ÷ 5 s ≈ 1.0); process rows ≈ 2.4 per pass (lists 0.2, utilisation 2.2) only at `Detail::Table` and on a 2 s grid (D49) → ≈ 1.2 ms/s; ≈ 4–5 ms/s with a `procs` tile visible on an idle card; never `pcie_throughput`; never a `NotSupported` field twice; `nvidia-smi` never spawned while NVML works | `tracing` span sums on the gpu thread, shown per call class in the `sources` tile |
| P12 | NVML: gridwatch is **never a GPU client** — NVML creates no context (verified: `nvidia-smi` and a running `nvtop` are absent from the process table) | 0 entries | `nvidia-smi \| grep -c gridwatch` = 0 (the full table lists graphics *and* compute clients; `--query-compute-apps` sees compute only), cross-checked by the gpu source filtering its own pid out of the v3 lists |
| P13 | Per-process GPU accounting only when shown | v3 lists and `process_utilization_stats` are called only while a gpu tile whose `demand(tier)` is `Table` or richer is visible | `sources` tile shows each source's demand level and detail |
| P14 | i2c (pins source) | **≤ 2 transactions/s** (500 ms block read, one transaction per sample), **≤ 1 %** of one core (the digest measured the root logger at 1.0 % on the older 36-transaction path; the block path is measured in arc 3 and recorded below); the interval is never configured below 500 ms | source transaction counter, `pidstat` |
| P15 | `/proc` scan | pid-level scan (`stat`, `statm`, dir `st_uid`, `cmdline` on first sight) **≤ 20 ms** wall per pass and **≤ 1 %** of one core amortised at its 3 s grid cadence (five file kinds measured 20 ms today for 665 pids; two kinds 10 ms); the `task/` walk (budgeted +30 ms, **measured +7.6 ms** in arc 10b — 5.4 ms without, 13.0 ms with, 1 807 thread rows on top of 629 processes) and htop's gated files (`smaps_rollup` alone 130 ms) only at `Detail::Columns` — a `full` tier, zoomed or `view = "full"`, with that column on, and the walk additionally only while `H` is on | `tracing` span on the cpu thread, shown in the `sources` tile |
| P16 | Audio capture | `pw-record` child **≤ 1.5 %** at its ≈ 94 chunk writes/s (4096-byte chunks every ~10.6 ms regardless of `--latency`), DSP thread `gw-audio` (the io pump `gw-audio-io` and the stderr reader `gw-audio-err` beside it) **≤ 2 %** at 30 fps; PipeWire quantum **unchanged** at the default `latency = 1024` (no-game row only — the game pins the graph at 256); child killed **≤ 10 s** after the last visible audio tile | `pidstat -u -t -p <gridwatch> 1 60` (named threads), `pidstat -p <pw-record pid>`, `pw-top -b -n 1` / `pw-metadata -n settings 0` |
| P17 | Memory | RSS **≤ 60 MB** after 1 h (NVML maps ~20 MB — nvtop sits at 31 MB) at the shipped `[store] history = "10m"`, and at the `1h` ceiling; the store's own bytes are **reported, not capped** — `Store::footprint()` on the `F12` HUD and as `store_bytes` in `--stats-log` (D63 retired `[store] max_mb`: there was no byte accounting behind it and no honest policy for exceeding it); **no growth** between hour 1 and hour 24 (24 h replay at 100× speed) | `pidstat -r`, `/proc/<pid>/status` VmRSS; `store_bytes` from `--stats-log` |
| P18 | Startup | first frame **≤ 300 ms** (placeholder tiles); every source live **≤ 2 s**; NVML init, `detect_bus`, `pw-record` spawn and D-Bus connect never on the render thread | `--stats` prints both timestamps |
| P19 | Frame cost | draw + write **p95 ≤ 8 ms** at 250×70; **mean ≤ 3 ms** at the Overview's ≈ 2 frames/s and **≤ 1.3 ms** at 30 fps (render cache: only the animated tile re-renders; the whole-frame diff is ~0.3 ms); missed frames **< 1 %** | `F12` HUD (p50/p95, changed cells, bytes) |
| P20 | Effects (arc 4) | ≤ `budget_ms` (4 ms) per frame, area-scoped, ≤ 600 ms per event; the repeating alert pulse alone at 8 fps; `--no-effects` honours P6/P8 exactly | `F12` HUD effect column, `fx_us` in `--stats-log` |
| P22 | Plugin host (arc 8b) | with a plugin rendering at 1 Hz the host's two threads (`gw-plugins`, `gw-plugin-<id>`) are **below `pidstat`'s resolution** and there is **no measurable delta against the same page with no plugin configured** — that, rather than a number nobody can reproduce, is the gate; a plugin that floods costs the host **no more** — the reader takes at most `MAX_MSGS_PER_SEC` (500) messages/s from one plugin, so its pipe fills and the child blocks rather than either process spinning; the inbound queue is 64 deep and drops the oldest; a plugin over 50 % of a core for 10 s is stopped; at most 256 distinct metric names per plugin. Startup: `hello_ms` (2 s default) is added to P18's first frame **only when a plugin is configured**, and every plugin is waited on together | `pidstat -u -t` (the `gw-plugins` and `gw-plugin-<id>` threads are named), `pidstat -p <child>`, task-summed context switches |
| P23 | Disk source (arc 14) | one `/proc/diskstats` read per tick ≤ **1 ms** wall and **≤ 0.2 %** of one core at the shipped 1 s cadence, with `disk.scan_ms` as the evidence; device **classification is once per device name**, not per tick (D64 §5), so the first pass is allowed to cost more than the steady state and is measured separately; the source never raises `Detail`, so P13 and P15 are untouched | `disk.scan_ms` in the store and on the `sources` tile's NOTE column; `pidstat -u -t` for the `gw-disk` thread |
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
| P5 | **Zero-poll waits** (`SourceCtx::sleep_until` parks on the control receiver — no 200 ms stop-flag polling) and **per-source cadence alignment** (`next_deadline` is the next multiple of that source's own period from the epoch, so sources sharing a cadence wake together — there is no separate phase grid, D63); the frame clock is the only sub-250 ms timer and only while something animates. |
| P6, P7, P9, P10 | ratatui's cell diff plus style-run reuse (one SGR per run, not per cell); gradients are 64-entry LUTs so adjacent cells share styles; animated regions are the tile's inner rect only; `Role::Bg` is painted once and then diffed away. |
| P11–P13 | NVML tiers (fast 500 ms ≈ 20 µs; slow 1 s; fans 5 s; power trace only when drawn); PCIe from byte-counter fields 197/198 (0.3 ms) never `pcie_throughput`; `NotSupported` pruning, `InsufficientSize` retried; process accounting gated on `Detail::Table`. |
| P14 | astral-watch's block read (one transaction per sample, validated once per chip); exporter preferred when the service runs; `redetect` only after 10 misses. |
| P15 | Pid-level scan at 3 s on the grid; htop's own gating rule for expensive files and the `task/` walk, reachable only through `Detail::Columns`. |
| P16 | `node.passive` capture on the sink monitor; `--latency 1024` keeps the graph quantum; the child is killed after 10 s hidden and respawned only on visibility. |
| P17 | `Retention::for_history([store] history)` per scalar series — `max_age` is the history (`1m`–`1h`, shipped `10m`) and `max_len` is `max_age / 250 ms` (2 400 at the default, 14 400 at `1h`); `Vector` series short, `Record` latest-only, alert ring 500, art cache 8 × ≤ 256 px. `Ring::new` preallocates `min(max_len, 4096)` slots, so a series costs 38 KB at the default and saturates at 64 KB from `history ≥ 17m`. |
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
| 3 | + P14 (block-path number recorded — by hand, the live pass opens `/dev/i2c-*`); alarm banner adds no steady-state cost |
| 4 | + P20; edit mode idle = P8; **S1–S7 under `matrix`** (focused, beside the game) and S5 = P4 with the game focused — 4b measured S2/S4/S7 and the no-game S1 under `--demo`; S1 beside the game, S3, S5 and S6's eyeball are owed |
| 5 | + P2, P3, P7, P9, P10, P16; P7b/P10b measured and a gate proposed |
| 6 | Winamp marquee at 220 ms steps stays inside P6 (it is ~40 cells) |
| 7 | probes and connection table stay inside P1/P5 |
| 8 | zoomed `full` tiers inside P15 (`Detail::Columns`, `task/` walk with `H`) and P19; **P22** for the plugin host, taken with the example plugin and with one that floods |
| 9 | P17 at 24 h; packaged binary re-measured |

## Measured (fill per arc)

| date | arc | theme class | page / state | game | gridwatch CPU | wake/s | KB/s | frames/s | Ptyxis Δ CPU | Ptyxis sm | NVML ms/s | scan ms | frame p50/p95 | RSS |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| — | — | — | — | — | — | — | — | — | — | — | — | — | — | — |
| 2026-08-31 | 1 | quiet | arc1a demo overview 250x70 focused | ? | 0.00% | 4 | 0 KB/s | ? | 0.63% | 0.0 | ? | ? | ? | 10092 kB |
| 2026-08-31 | 1 | quiet | arc1a demo overview 250x70 UNFOCUSED | ? | 0.00% | 4 | 0 KB/s | ? | 0.60% | 0.0 | ? | ? | ? | 10112 kB |
| 2026-08-31 | 1 | quiet | arc1a POST-FIX demo overview 250x70 focused | ? | 0.00% | 4 | 0 KB/s | ? | 0.65% | 0.0 | ? | ? | ? | 7928 kB |
| 2026-08-31 | 1b | quiet | **live** overview 250x70, cpu tile focused (pty) | no | 0.38% | 6 | 3.4 KB/s | 2.00 | — | — | n/a | 0.29 | 0.69 / 1.10 ms | 10504 kB |
| 2026-08-31 | 1b | quiet | demo overview 250x70 (pty) — synth jitters every core each tick | no | 0.20% | 6 | 24.8 KB/s | 1.9 | — | — | n/a | n/a | 0.84 / 1.45 ms | 10856 kB |
| 2026-09-01 | 2a | quiet | **live** pid-level scan, release, 635 pids / 395 kthreads (`cpu::procs`) | no | — | — | — | — | — | — | n/a | **5.42 mean / 6.32 worst** | — | — |
| 2026-09-01 | 2a | quiet | **live** overview 250x70, cpu tile focused, process table on (pty, review measurement) | no | 0.82% | 6 | 3.33 KB/s | 2.00 | — | — | n/a | 5.4 | 1.14 / 1.58 ms | 11708 kB |
| 2026-09-01 | 2a | quiet | demo overview 250x70 (pty, review measurement) | no | 0.27% | 6 | 18.6 KB/s | 2.00 | — | — | n/a | n/a | 1.26 / 1.69 ms | 10600 kB |
| 2026-09-01 | 2a | quiet | **live** overview 250x70 with `--record` (pty, review measurement) | no | 0.73% | 8 | 3.68 KB/s | 2.00 | — | — | n/a | 5.4 | 1.02 / 1.38 ms | 12000 kB |
| 2026-09-02 | 2b | quiet | **live** overview 250x70, 60 min, nothing focused (pty, idle torch, RSS only) | no | — | — | — | — | — | — | — | — | — | 37100 kB → 40108 kB at 20 min → 40284 kB at 60 min |
| 2026-09-01 | 2b | quiet | **live** overview 250x70, cpu + gpu tiles at `procs`, nothing focused (pty, idle torch, `--stats-log`) | no | 1.50% (render 0.28 · gw-cpu 0.58 · gw-gpu 0.67) | 8 | 8.2 KB/s | 2.1 | — | — | ≈ 7.1 in this run's fixture (before the 2 s process grid); **4.26** re-measured (fast 0.04 + slow 2.55 + procs 1.67) | 5.4 | 1.31 / 1.67 ms | 38972 kB → 41956 kB at 19 min |
| 2026-09-02 | 3b | quiet | demo overview 250x70 (pty, `--demo`, three synths), **no active alert**, 15 s window 3–18 s | no | 0.40% | 14 | 28.9 KB/s (HUD 29.4) | 2.00 (30 data, 0 beat) | — | — | n/a | n/a | 1.69 / 2.09 ms | 13040 kB |
| 2026-09-02 | 3b | quiet | demo overview 250x70 (pty, `--demo`), **Crit banner active and pulsing**, 15 s window 24–39 s | no | 0.40% | 14 | 29.6 KB/s (HUD 29.5) | 2.00 (30 data, 0 beat) | — | — | n/a | n/a | 1.71 / 2.15 ms | 13040 kB |
| 2026-09-02 | 4a | quiet | demo overview 250x70 (pty, `--demo`), **edit mode idle** (`e` pressed, no further keys), 40 s window | no | 0.42% | 13 | 29.4 KB/s (HUD 29.6) | 2.00 (80 data, 0 beat) | — | — | n/a | n/a | — / 2.13 ms | — |
| 2026-09-02 | 4b | **showcase** | demo overview 250x70 (pty, `--demo --theme matrix`), focused, no game — the rain at 24 fps, 40 s window; after the review's in-place rewrite (the first build measured 6.57 %, 612 KB/s, p95 4.4 ms, 16.5 MB) | no | 4.88% | 33 | 659 KB/s (HUD 705) | 23.8 (anim) | — | — | n/a | n/a | — / 3.09 ms · fx ≤ 8 µs | 13436 kB |
| 2026-09-02 | 4b | quiet | demo overview 250x70 (pty, `--demo --no-effects`), 40 s window | no | 0.43% | 13 | 29.2 KB/s (HUD 29.0) | 2.00 (80 data, 0 anim) | — | — | n/a | n/a | — / 2.19 ms | 14332 kB |
| 2026-09-02 | 4b | quiet | demo overview 250x70 (pty, `--demo`, retrowave with its `[effects]`: startup sweep, focus fade, the alert pulse at 8 fps while the banner is up 22–50 s), 40 s window | no | 0.90% | 16 | 30.6 KB/s (HUD 30.5) | 5.83 avg (8 while the pulse runs) | — | — | n/a | n/a | — / 2.20 ms · fx ≤ 42 µs | 13708 kB |
| 2026-09-02 | 5a | quiet | demo overview 250x70 (pty, `--demo`), the 4x2 audio tile animating at its 30 fps beside the four quiet tiles, 40 s window (**P2**) | no | **4.72%** (render 4.52 · gw-audio 0.18) | 69 | 85.5 KB/s (HUD 85.0) | 29.0 (29.0 anim) | — | — | n/a | n/a | 1.49 / 2.02 ms | 14120 kB |
| 2026-09-02 | 5a | quiet | demo Audio page 250x70 (pty, `--demo --page 2`), the 6x3 spectrum (122×31) at 30 fps, 40 s window (**P7b**) | no | **3.37%** (render 3.22 · gw-audio 0.17) | 66 | 145 KB/s (HUD 148) | 30.0 (30.0 anim) | — | — | n/a | n/a | 1.03 / 1.56 ms | 14204 kB |
| 2026-09-02 | 5a | quiet | demo Audio page 250x70 (pty, `--demo --page 2 --fps 60` with the tile's `fps = 60` and `[sources.audio] fps = 60`), 40 s window (**P3**) | no | **6.05%** (render 5.85 · gw-audio 0.20) | 95 | 155 KB/s (HUD 155) | 59.0 (59.0 anim) | — | — | n/a | n/a | 0.94 / 1.45 ms | 13540 kB |
| 2026-09-02 | 5a | — | **live** `pw-record` capture for 60 s on the idle desktop, **nothing playing** (the silence path; `--record` of `torch-audio.jsonl`), the pins source pinned to `exporter` at a dead port so `/dev/i2c-*` stayed closed (**P16**) | no | gridwatch 0.1 · gw-audio 0.0 · gw-audio-io 0.0 · `pw-record` 0.0 (`ps -L` at 20 s and 40 s) | — | — | 2 Hz data (silence) | — | — | n/a | n/a | — | — |
| 2026-09-02 | 5b | quiet | **live** overview 250x70 (pty), every source incl. the new `sensors` and the audio source on an idle sink, 40 s window (**P1/P5/P6**) | no | **1.95%** (render 0.35 · gw-cpu 0.65 · gw-gpu 0.83 · gw-sensors **0.05** · gw-audio 0.00) | 28 | 3.8 KB/s | 2.17 | — | — | — | — | — / 2.20 ms | 44260 kB |
| 2026-09-02 | 6 | quiet | demo Audio page 250x70 (pty, `--demo --page 2`), the winamp tile animating at 10 fps beside the audio tile at 30, 40 s window (**P1/P5/P6/P19**) | no | **3.95%** (render 3.75 · gw-audio 0.18 · gw-mpris 0.00) | 68 | 198 KB/s | 29.9 (29.9 anim) | — | — | n/a | n/a | — / 1.63 ms | 17480 kB |
| 2026-09-02 | 7a | quiet | **live** overview 250x70 (pty), every source incl. the new `net` with its connection scan, 40 s window (**P1/P5/P6/P13**) | no | **2.67%** (render 0.42 · gw-gpu 0.97 · gw-cpu 0.65 · gw-net **0.52** · gw-sensors 0.07) | 33 | 4.1 KB/s | 2.15 | — | — | — | scan 3.3 ms | — / 2.20 ms | 49828 kB |
| 2026-09-02 | 7b | — | **P18** rules cost: ten rules over a batch of 40 scalars, in `Store::apply` (`ten_rules_cost_microseconds_per_batch`) | no | — | — | — | — | — | — | — | — | **24 µs** release / 149 µs debug per batch | — |
| 2026-09-02 | 7 (post-review) | quiet | **live** overview 250x70 (pty), every source, the probes now on their own thread, 40 s window (**P1/P5/P6/P13**) | no | **2.60%** (render 0.43 · gw-gpu 0.88 · gw-cpu 0.65 · gw-net 0.53 · gw-net-probe **0.00** · gw-sensors 0.07) | 33 | 3.9 KB/s | 2.17 | — | — | — | — | — / 2.25 ms | 50468 kB |
| 2026-09-02 | 8a | — | **P15 with the gated files** — the pid scan plus one `/proc/<pid>/io` per process, 638 rows, release (`live_scan_with_the_gated_files_is_inside_p15`) | no | — | — | — | — | — | — | — | scan **3.5 ms** mean / 4.1 ms worst (173 of 638 readable) | — | — |
| 2026-09-02 | 8a (post-review) | quiet | **P19** — one render of the zoomed htop `full` tier, 632 rows, tree on, release (`zoomed_full_tier_render_is_inside_p19`); and the live frame cost with that tier drawing | no | — | — | — | 18.2 | — | — | — | — | **0.04 ms** p50 / 0.05 ms p95 live · **605 µs** per tile render | — |









| 2026-09-02 | 8b | quiet | live Overview 250x70 (pty, release), the clock slot replaced by a chip for a plugin that is **not** configured — the control for the three rows below | no | **1.27%** (gw-pins 0.78 · gw-gpu 0.33 · render 0.07) | 31 | 0.6 KB/s | 1.4 | — | — | n/a | n/a | 0.74 / 1.54 ms | 47524 kB |
| 2026-09-02 | 8b | quiet | the same Overview with `plugins/examples/weather.py` in that slot, rendering at 1 Hz (**P22**) | no | **0.95%** (gw-pins 0.78 · render 0.15 · **gw-plugins 0.00 · gw-plugin-weather 0.00**; the python3 child 0.00) | 36 | 0.6 KB/s | 2.3 | — | — | n/a | n/a | 0.65 / 1.51 ms | 44168 kB |
| 2026-09-02 | 8b | quiet | a plugin writing samples in a loop, **before** the read-rate budget existed (**P22, failing**) | no | **62.05%** (gw-plugin-flood 55.53 · gw-plugins 5.22 · gw-pins 0.83 · render 0.45; the python3 child **99.95%**) | **578457** | 0.7 KB/s | 4.2 | — | — | n/a | n/a | 0.49 / 0.98 ms | 56716 kB |
| 2026-09-02 | 8b | quiet | the same flooding plugin **after** the budget (**P22** ✓) | no | **0.97%** (gw-pins 0.85 · render 0.07 · **gw-plugins 0.00 · gw-plugin-flood 0.00**; the python3 child **0.10%**) | 35 | 0.6 KB/s | 1.4 | — | — | n/a | n/a | 0.69 / 1.51 ms | 43764 kB |
| 2026-09-07 | 11b | — | **P18** the retention sweep with D61's **per-label** pass, 400 labelled series, against a control that never sweeps (`the_retention_sweep_stays_inside_the_batch_budget`, `cargo test -p gridwatch-store --release --test store the_retention_sweep -- --nocapture`) | no | — | — | — | — | — | — | — | — | **7.0 µs** amortised per batch (release, 6.2–10.2 µs over eight runs) ≈ **70 µs per sweep**, one sweep per ten batches in the fixture; 35 µs amortised debug on a 124 µs control | — |
| 2026-09-07 | 11b | — | **P18** D61's uncatalogued cap, on the only path it touches — a **new series** (`the_cap_costs_a_catalogue_miss_on_a_new_series_and_nothing_after`, same command) | no | — | — | — | — | — | — | — | — | new uncatalogued series **447 ns**, new catalogued series **351 ns**, push to an existing series **110 ns**; the catalogue miss alone **120 ns** (release, 2 000 series, `max_len = 16`) | — |

## Benches (arc 9a, D59 seam 3) — the layer under the ceilings

The rows above gate the **product**: CPU, wake-ups and bytes on a real run, measured with `pidstat`. These gate nothing. They are the four functions every one of those ceilings assumes a cost for, isolated with criterion so a regression in one is legible instead of arriving as "the dashboard got slower". `scripts/gate.sh` does not run them on purpose — a timing assertion on a machine that is also running a game is a flake generator, and a red build meaning "the box was busy" teaches people to ignore red builds.

```console
$ cargo bench -p gridwatch-app
```

| bench | what it is | torch, 2026-09-02 (release, idle, no game) |
|---|---|---|
| `store/apply/cpu batch` | one cpu batch — ~40 scalars plus the process table — through `Store::apply`, rules engine included, over a store already holding a minute of history | **2.48 µs** |
| `store/resample/60` · `/120` · `/240` | a ten-minute window into a chart's buckets | **0.93 µs** · 0.69 · 0.61 |
| `render/frame/250x70 configured` | the **whole** Overview solved, ticked, viewed, rendered and diffed with nothing cached | **513 µs** |
| `render/frame/120x40 dense` | the same page in dense mode | **267 µs** |
| `render/frame/480x135 wide` | the same page on the wide terminal D62 was reported from — 3.7x the cells of 250x70 | **1.02 ms** (2026-09-06, after arc 12; 250x70 re-read 527 µs in the same run) · **1.09 ms** (2026-09-09, after arc 15; see below) |
| `theme/load retrowave` | parse + build a theme, WCAG gate included — what every `t` press pays | **26.5 µs** |

What they say about the ceilings above: P19 allows **8 ms p95** for a frame, and a *completely uncached* Overview costs 0.51 ms — which is why the render cache buys what it does, and why the live p50 is 0.04 ms (arc 8a's row: most frames are a blit). `Store::apply` at 2.5 µs means the data path is not the cost of anything; at the Overview's ~40 batches a second it is 0.1 ms of CPU per second. And `resample` costing *less* at more buckets is not a mistake in the table — the work is per point, and the per-bucket aggregation gets cheaper as the buckets get smaller.

**Arc 12 (2026-09-06).** The wide row is not a gate; it is so the next person
knows what a frame that really fills 480x135 costs. Uncached it is **1.02 ms**
against P19's 8 ms p95, or 2.0x the 250x70 frame for 3.7x the cells — sublinear
because the chrome, the layout solve and the fixed-height header blocks do not
grow with the rect. The 250x70 bench read **527 µs** in the same run against
the 513 µs recorded on 2026-09-02; the 2.7 % difference is not attributed —
two runs four days apart on a shared machine, with the arc-12 drawings now
filling columns they used to leave blank, and nothing here separates the two.

**Arc 15 (2026-09-09) — P19 re-taken uncached at both sizes, as an A/B on one machine in one sitting.** The new drawing work is the chart gridlines (at most three `set_string` runs of `width` cells per chart), one `set_stringn` per series label, one `View::Gauge` row per shown sensor, and a gpu chart band that is now the full tile width instead of 24 cells narrower. The control is the **same benchmark on `75aa940`**, the commit before the arc, built and run in a scratch worktree minutes before the after-run:

| `cargo bench -p gridwatch-app` | before (`75aa940`) | after arc 15 | change |
|---|---|---|---|
| `render/frame/250x70 configured` | **521.9 µs** | **549.5 µs** | +5.3 % |
| `render/frame/480x135 wide` | **1.0400 ms** | **1.0876 ms** | +4.6 % |

Five per cent for the whole arc, and an uncached 480×135 frame is **1.09 ms against P19's 8 ms p95** — 13 % of the ceiling, with the render cache meaning most live frames are a blit (0.04 ms p50, arc 8a). **Nothing here is superlinear**: the same +5 % at both sizes is what a per-chart constant plus a per-row constant looks like.

**Why the A/B and not the recorded 1.02 ms.** A first "before" run, taken on this tree straight after a release build, read **753.8 µs / 1.4596 ms** — 43 % over the arc-12 numbers, for reasons that have nothing to do with the code. Comparing the after-run to *that* would have claimed a 26 % **improvement** from an arc that only adds drawing. Two runs minutes apart on a quiet machine are the comparison; two runs days apart are not. (Arc 12 recorded the same hazard from the other side: its 250x70 re-read 527 µs against the 513 µs of four days earlier and declined to attribute the difference.)

**P2, P4, P5, P13, P15 and P17 are unchanged and asserted so, not re-measured**: arc 15 touches no source, no cadence and no `Detail` — `grep -n 'Detail::' crates/components/src/{net,gpu,sensors}` is identical to the arc-14 tree, and the only new work is inside `view` and the renderer.

Re-take them on a machine change and put the new column here rather than overwriting: the point is the comparison.

**Arc 8b notes (2026-09-02) — P22, and the row that failed first.** Same protocol (release binary, `script` pty at 250×70, an idle torch with no game, per-thread `pidstat`, task-summed context switches, Δ`wchar`). The Overview was measured with and without a plugin in the clock's slot, so the two rows differ only by the plugin.

**The host is free at 1 Hz — and the honest form of that claim is a comparison, not a number.** With `plugins/examples/weather.py` rendering once a second, both host threads read **0.00 %** at `pidstat`'s resolution and so does the python3 child. The whole process reads 0.95 % against the control's 1.27 %, which is *lower* with the plugin than without it: the difference is `gw-gpu`, idle in one run and not the other. So the row is not "the host costs 0.1 %" — that is below what this instrument can see — it is "no measurable delta against the same page with no plugin configured", which is what the two rows above show and what a re-take can check. It adds **+5 wake-ups/s**, **no measurable bytes**, and **+0.9 frames/s** — one frame for the sample it publishes, which is a generation change and therefore a frame P8 allows.

**A flooding plugin failed the row, and hard.** The first measurement of a plugin writing samples in a loop cost **62 % of a core** — its reader thread 55.5, the host thread 5.2 — with **578 000 wake-ups a second** and 13 MB of RSS the queue had grown. D58 seam 7 had specified "a 64-message inbound queue that drops oldest rather than growing" and none of it was implemented: every parsed line went down an unbounded channel as fast as the child could write. The fix is two bounds, and the one that matters is the **read-rate budget**: the reader takes at most `MAX_MSGS_PER_SEC` (500) messages a second from one plugin and otherwise stops reading, so the pipe fills and the child blocks in `write`. After it, the host is **0.97 %** — the control's own figure — both plugin threads are 0.00 %, the child is **0.10 %**, and the wake-ups are **35**. The queue (64 deep, drop-oldest) is the second bound, for a burst rather than a flood.

**And one that spins.** The budget is no answer to a plugin that burns a core without writing, so the host reads each child's `utime + stime` once a second and stops one holding 50 % of a core for ten seconds. Watched by hand: a Python `while True` child is gone 11 s after start, with `spin: stopped: 100% of a core for 10 s (the ceiling is 50%)` in the log and a `Crit` toast on screen. `RLIMIT_CPU` is still underneath it, but its 600 s default is ten minutes of a core beside a game.

**P18 with plugins.** The first frame is still 1–6 ms without plugins, and a configured plugin adds up to its `hello_ms` (2 s default) — every plugin is spawned before any is waited on, so N plugins cost the longest wait rather than the sum, and one that never answers is left running as a source with its tile chipped. P18's ceiling is measured on the default config, which configures none.

**Numbering.** The arc-8 brief called this row "P20"; P20 has been the effects budget since arc 4 and P21 the unfocused throttle, so the plugin host is **P22** (D58 amendment 17, in the same spirit as arc 6's P12/P19 correction).


**Arc 8a post-review note (2026-09-02).** The review found the tree's per-row depth being recomputed inside `view` — a fresh map over every row, once per row, which at torch's 638 processes is ~407 000 inserts a frame — against §8.1's "`view` never sorts". The filter and the tree order now run in `tick` and `view` reads an index list, and the depth is one pass over the set. Measured after the fix: **605 µs** for one render of the zoomed `full` tier at 632 rows with the tree on, and **0.04 ms p50 / 0.05 ms p95** for the whole frame in a live 30 s run with that tier drawing (the render cache means most frames are blits, which is the point of it). P19's ceiling is 8 ms p95 for the frame.

**Arc 8a notes (2026-09-02).** The gated pass — htop's `H` and its I/O screen — costs **3.5 ms mean, 4.1 ms worst** over 638 processes, against P15's 12 ms ceiling and the 6.05 ms the plain pid-level pass measures in the same run. (The gated number is lower because the plain pass ran first and warmed the dentry cache; the honest reading is that opening one more small file per process is not what makes this scan expensive.) 173 of 638 `/proc/<pid>/io` files were readable as the user, and the tile marks the other 465 `n/a` rather than drawing zeroes. Both are behind `Detail::Columns`, which only the zoomed `full` tier asks for, and only once a person presses `H` or switches to the I/O screen. The executor thread is idle unless an action is queued and never touches the render thread; the confirm bar is a line of text.

**Arc 7 post-review note (2026-09-02).** Moving the latency probes off the source thread (D57 amendment 22) cost nothing measurable: the new `gw-net-probe` thread rounds to **0.00 %** — it spends its life blocked on a channel or in a socket timeout — and the process total came down slightly, to **2.60 %** with every source live. Wake-ups are unchanged at 33/s. What the change bought is that a silent probe target can no longer delay a rate sample: inline, two of them held the collector for up to 1.8 s a tick.

**Arc 7b notes (2026-09-02).** The rules engine is name-indexed and sees only the scalars a batch carried, so its cost scales with *matches*, not with the store: ten rules against a forty-sample batch cost **24 µs** in release (149 µs in a debug build, which is what the gate measures). At the sensors source's one batch a second that is 0.002 % of a core, and the test fails above 0.5 ms a batch. A store with no rules does no work at all — the engine is skipped by an `is_empty` check before the samples are even collected, which is why the shipped `config.toml` keeps its four examples commented out.

**Arc 7a notes (2026-09-02).** The net source costs **0.52 %** of one core with the connection table visible: one `/proc/net/dev` read a second, sysfs link attributes every two seconds, and the `/proc/*/fd` scan every two seconds — the scan itself reports **3.3 ms** on the `sources` tile beside `87/103 attributed` (the digest measured ~10 ms in Python for the same work). That is inside **P13**'s 1 % and its ≤ 10 ms scan. The whole process sits at 2.67 % with every source live, 33 wake-ups/s and 4.1 KB/s — P1, P5 and P6 hold with one more source than any earlier row. RSS is 49.8 MB (the connection table and the inode map are the biggest allocation this dashboard makes). **Owed to Matt:** the Wi-Fi row (`wlp7s0` is down on torch, so SSID/dBm/bitrate have no live measurement), P9/P10 in Ptyxis, and the public-IP path (off by default, and asking the internet is his call).

**Arc 6 notes (2026-09-02).** The Audio page with the winamp tile and the audio tile both animating costs **3.95 %** of one core at 29.9 fps — the whole page's frame rate is the audio tile's 30, and the winamp tile's own 10 fps only decides how often *it* re-renders (the cache's animation term, D55 seam 5). p95 1.63 ms, 68 wake-ups/s, 198 KB/s, RSS 17.5 MB, and the `gw-mpris` thread at **0.00 %**: under `--demo` it is the synth, and live it sleeps on the bus between property signals. **The brief's "P12/P19" was a mislabel** — P12 is the NVML row; the winamp tile's cost lands in P1/P5/P6/P19, recorded above (D56 amendment). **Owed to Matt:** the live Firefox pass (a real player's controls, a track change, stream mode, the art fetch's wall time), P9/P10 in Ptyxis, and the tag.

**Arc 5b notes (2026-09-02).** The sensors source's own cost is **0.05 %** of one core at 1 Hz over torch's fifteen hwmon inputs (nine chips), and its hwmon walk reports `walk 0.4–0.6 ms` on the `sources` tile — inside the ≤ 1 ms row the brief asked for. Its thread makes ≈ 14 voluntary context switches a second: one per blocking sysfs read (the NVMe inputs are SMART log pages, `spd5118` an SMBus transaction), which is inherent to reading them at 1 Hz. **A wake-up regression the measurement caught:** the 5a review's "re-check the sink every 5 s" (D55 amendment 12) spawned `pw-dump` and parsed ≈ 280 KB on the audio source's thread, costing **≈ 435 wake-ups/s** on that thread alone — P5 is 40/s for the whole process. The re-check is now 60 s and only runs while the target is `auto` (a pinned sink cannot change under us); the audio thread is back to 2.0 wake-ups/s and the whole process to **28/s** with every source live. **P1 with the live pins source is still owed** (this row ran with `[sources.pins] source = "exporter"` at a dead port so no agent opened `/dev/i2c-*`).

**Arc 5a notes (2026-09-02).** Same protocol (release binary, `script` pty at 250×70, an idle torch with no game, `--stats-log`). **P2** with the 4x2 visualizer animating on the Overview: 4.72 % of one core at 29 fps (every frame animation-caused; the DSP thread 0.18 %), 69 wake-ups/s, 85 KB/s, p95 2.0 ms — inside the 6 % ceiling. **P7b** (the Audio page's 122×31 spectrum at 30 fps): 3.37 %, 145 KB/s, p95 1.56 ms, ≈ 370 changed cells per frame (the HUD's per-frame figure) — under P7's 600 KB/s and 2 500 cells. **P3** (60 fps): 6.05 % at 59 fps (render 5.85 · DSP 0.20), 95 wake-ups/s, 155 KB/s, p95 1.45 ms, ≈ 240 changed cells per frame — inside the 10 % ceiling. `--fps 60` alone does **not** raise the rate: the tile animates at its own `fps` option and the source publishes at `[sources.audio] fps` — the shell takes the max of the animating tiles capped by `fps_max` (seam 5), so the 60 fps row sets both. **P16, silence only:** with nothing playing the `pw-record` child, `gw-audio` and `gw-audio-io` all sat at 0.0 % over 60 s (`node.passive` delivers nothing while the sink is idle, the DSP runs at 2 Hz on zeros), the graph's quantum stayed 0 in `pw-top -b -n 1` (idle graph; the "unchanged at 1024 while playing" half is owed), and the child was killed within a second of `q`. **Owed to Matt:** P16 with sound (the child's 94 chunks/s and the DSP at 30 fps on real frames), the "reacts to Firefox/game audio within 30 ms" row, and P9/P10 in Ptyxis. The kill-after-10-s-hidden path is unit-tested (`supervise::Policy`), not measured live.

**Arc 4b notes (2026-09-02).** Same protocol (release binary, `script` pty at 250×70, an idle torch with no game, `--stats-log`). **Under `matrix`** the rain ran at its full 24 fps for the whole window with the governor never stepping: after the review's in-place rewrite **4.88 %** of a core (**S1's no-game half**; the beside-the-game row is Matt's; the first build was 6.57 %), 659 KB/s (**S2** ≤ 3 MB/s ✓; the sweep second peaks near 1.6 MB/s), frame p95 3.1 ms (**S4** ≤ 16 ms ✓), RSS 13.4 MB against 14.3 MB for the `--no-effects` run — the layer draws in place from three per-cell vectors (≈ 300 KB at 250×70) and keeps no frame-sized buffer (**S7** ✓). At 400×100 the review measured 22.5 frames/s, p95 6.9 ms, 1.1 MB/s (sweep seconds 3.0 MB/s), ≈ 10 % of a core, and the governor had no reason to step — the brief's "engages at 400×100" does not reproduce on torch and is recorded as such (D54). S3 (Ptyxis CPU and `pmon`), S5 with the game focused and S6's eyeball row are owed to Matt; the pause-freeze half of S5 is the shell test. **P20:** `--no-effects` reproduces the 3b/4a demo rows to the digit (2.00 frames/s, 13 wake-ups, 0.43 %, 29 KB/s); under retrowave with its hooks the painter costs ≤ 42 µs per frame against the 4 ms budget, the watchdog never trips, the startup sweep and the focus fade are ≤ 600 ms events, and the repeating alert pulse — measured first at the full 30 fps: **2.65 % of a core while a banner is up, over P1** — is now drawn at 8 fps when nothing else animates (`effects::PULSE_FPS`), which brings a run with the banner up for 28 of 40 s to 0.90 %, 16 wake-ups and 30.6 KB/s. The heartbeat reverse is off while a theme declares the pulse (D54).

**Arc 4a note (2026-09-02).** Idle edit mode = P8: with `e` pressed and no further keys the same 40 s protocol shows the same 2.00 frames/s (all data-caused), 13 wake-ups/s and 0.42 % as the 3b demo rows — the dotted grid and the edit key bar are drawn inside the frames the data already causes and add no cause of their own.

**Arc 3b notes (2026-09-02).** The banner's steady state, measured the same way (release binary under a `script` pty at 250×70, `run --demo --stats-log`, an idle torch with no game; the synth raises `pins/overload` at 21.5 s and resolves it at 50 s): the two 15 s windows before and during the active Crit alert show **the same 2.00 frames/s** (every frame data-caused, zero heartbeat frames — the demo's 500 ms synths always arrive first), **the same 14 wake-ups/s and 0.40 % CPU**, and bytes 28.9 → 29.6 KB/s: the pulse re-styles one 250-cell row once a second (≈ 0.7 KB/s), which is the "+1 row/s" the brief predicted — the banner causes **no extra frames**. The demo's 29 KB/s is the three synths jittering every value every tick (arc 2a's demo row was 18.6 KB/s with one synth); the P6 gate is the live row. **Owed to a human:** P1/P5/P6 with the *live* pins source (it opens `/dev/i2c-*`) and `doctor`'s live table, like P14. The watcher adds one wake-up per second (P5's `watcher 1`), inside the 14 measured.

**Arc 1b notes (2026-08-31).** Release binary under a `script` pty sized 250×70
(`sleep N | script -qec "stty rows 70 cols 250; gridwatch run --stats-log …"
/dev/null`), a 45 s window after a 12 s settle, on an **idle torch with no game
running**. Per-thread voluntary context switches from
`/proc/<pid>/task/*/status`; bytes from `/proc/<pid>/io` `wchar` **minus the
stats log's own growth**; frames, frame times and both P18 timestamps from
`--stats-log` (the F12 HUD shows the same numbers).

| gate | ceiling | measured | verdict |
|---|---|---|---|
| P1 | ≤ 2 % of one core | 0.38 % | ✓ **on an idle box** — the "beside a game" row is owed |
| P5 | ≤ 40 wake-ups/s | 6 /s | ✓ |
| P6 | ≤ 25 KB/s, HUD within 5 % of Δ`wchar` | 3.4 KB/s; HUD 3.37 KB/s vs Δ`wchar`−log 3.43 KB/s = **1.99 %** | ✓ (brief task 4's cross-check) |
| P8 | every frame caused; ≈ 2/s on the Overview | **2.00 /s** over the 45 s window; every frame in the whole ~57 s run had a cause — 112 data, 1 heartbeat, 0 animated | ✓ — the cpu tile is *focused*, so its source runs the 500 ms cadence |
| P18 | first frame ≤ 300 ms; every source live ≤ 2 s | **≈ 14 ms** end to end (13 ms exec → first bytes on the terminal, over three runs, + 1 ms shell → first drawn frame) and **252 ms** | ✓ |
| P19 | p95 ≤ 8 ms, mean ≤ 3 ms | p50 0.69 ms, p95 1.10 ms | ✓ |
| P17 | RSS ≤ 60 MB | 10.5 MB | ✓ |
| P4, P21 | unfocused ≤ 0.3 %; focus reporting | — | **owed**: a pty sends no focus events |
| P15 (arc 2a) | pid-level scan ≤ 20 ms wall, ≤ 1 % of a core amortised | **5.42 ms mean, 6.32 ms worst** over 10 passes (635 pids); 6.3 ms / 3 s = 0.21 % of a core at the grid cadence, 0.42 % focused | ✓ — `sys.scan_ms` carries the number; the `sources` tile prints it (`scan 5.4 ms`) |
| P1, P5, P6, P8, P19 re-taken (arc 2a, process table on the Overview) | as above | live 0.82 % · 6 wake/s · 3.33 KB/s (HUD vs Δ`wchar` 0.3 %) · 2.00 fps · p95 1.58 ms; demo 0.27 % / 18.6 KB/s (down from 24.8: the table steals rows from the jittering core bars); `--record` 0.73 %, p95 1.38 ms | ✓ — the +0.44 pp over arc 1b is the scan (0.37 pp) plus ~0.07 pp for the table's derive/view/fingerprint |
| P9, P10 | Ptyxis Δ CPU and `pmon sm` | — | **owed**: needs the real terminal beside the game |

- **With `--record` the naive P6 recipe reads the journal's own disk writes** (≈ 11 KB/s) in Δ`wchar`: subtract the journal file's growth as the recipe already subtracts the stats log's.
- **`first_frame_ms` is measured from `Shell::new`, not from `exec`** — everything
  before it (config, theme, capability probe, source spawn, terminal setup) is
  covered by the separate 13 ms exec→first-bytes measurement, and the two legs
  are added above rather than one being passed off as the other.
- **Two defects the P18 measurement found and this arc fixed:** the frame loop
  parked on input for up to 250 ms *before* drawing (first frame 251 ms → 1 ms),
  and both cpu sources waited for their first cadence boundary before sampling,
  so with the demand still `Hidden` the first batch landed at 3.0 s (→ 252 ms,
  and the first *delta* now arrives a whole period earlier).
- **The demo row is the pathological case, not the product**: `demo::CpuSynth`
  moves all 32 cores ±8 % every tick, so ~660 cells change per frame against the
  live source's ~95. It lands at 24.8 KB/s, just inside P6 — that is the
  headroom a genuinely busy machine has under the `cores` tier, and the honest
  reason to re-take P6 with the game running.
- **The instrument cost something, and now it is measured and off by default**
  (arc 10a, D60). `--stats-log` used to turn on changed-cell accounting, which
  clones the frame buffer and compares 17 500 cells per frame; every row taken
  from a stats log therefore measured the product *plus its instrument*. The
  counter and the diff are separate now: `--stats-log` counts frames, times
  them and attributes every redraw for free, and the diff runs only for the
  `F12` HUD (a person is looking at the number) or `--stats-log --stats-cells`.
  `changed_cells` is `null` rather than `0` when it did not run.
  **What it was costing**, from a paired 40 s window on the Overview with the
  visualizer animating (release binary, `script` pty at 250×70, `--demo
  --no-effects`, the two runs back to back and reporting the *same* 1 698
  frames, so the difference is like-for-like):

  | | frame p50 | frame p95 | CPU |
  |---|---|---|---|
  | `--stats-log --stats-cells` | 1 072 µs | 1 599 µs | 3.60 % |
  | `--stats-log` | 903 µs | 1 455 µs | 3.00 % |
  | **the diff** | **+169 µs/frame** | +144 µs | **+0.60 pp** |

  169 µs per frame is 0.47 % of a core at 28 fps, which agrees with the 0.60 pp
  measured (the clone's allocator work is the rest). So it scales with the
  frame rate and barely touches the rows that matter most: at P4's 2 fps it is
  0.03 % of a core, so **P4's 0.3 % row was never meaningfully inflated** — but
  **P2's 4.72 % arc-5a row was**, by roughly 0.6 pp of instrument, putting the
  product itself nearer 4.1 %. This box was not idle during the pair (Firefox
  and a game-shaped process were running), so these are not idle-torch absolute
  rows; the *difference* is what they are for, and the identical frame counts
  are why it holds.
- **The retention sweep** (arc 10b, D60; per-label since arc 11, D61) runs on
  a **retention boundary in store time** — `max_age / 10`, floored at 10 s —
  not per `apply`, because the sweep walks every series. Measured against a
  control that never sweeps, on a store of **400 labelled series** (torch runs
  about 150 with every source live): arc 10b's prune-only sweep cost **2.3 µs**
  a batch on top of the store's own 127 µs for the same 400 samples in a debug
  build. Arc 11's per-label sweep costs **7.0 µs** a batch amortised in
  release (6.2–10.2 µs over eight runs) and **35 µs** debug on a 124 µs
  control — about fifteen times arc 10b's, because before it can evict
  anything it builds the newest `Ts` per `(domain, label)` over every labelled
  series, cloning a `Label` per entry. Amortised is the misleading half: the
  fixture sweeps once per ten batches, so **one sweep of 400 series costs
  ≈ 70 µs**, and the shipped cadence is one sweep per 40 cpu batches (60 s of
  store time at a 1.5 s cadence) over ~150 series. Still microseconds a batch
  against the suite's 500 µs ceiling, and P18 is untouched by it. Store time
  rather than the wall clock is what keeps arc 2a's determinism test
  meaningful: a replay evicts at exactly the message the live run did.
  It **prunes without deleting, and deletes only a dead label**. Pruning keeps
  a scalar ring's newest point: the first version emptied the ring and dropped
  the entry, on the reasoning that a chart's window is `max_age` so an empty
  ring holds nothing renderable — true of charts, false of `Store::last`,
  which is how every tile reads a current value. Six catalogued scalars are
  published **once** (`sensor.max_c`, `sensor.crit_c`, `net.speed_mbps` and
  the three static gpu clocks), so they vanished eleven minutes into any
  default run. Keeping one point is 16 bytes per dead series against 38 KB, so
  that leak is closed 2 400-fold. Removal arrived with D61 and is **per
  `(domain, label)`, never per series**, and only for a key the catalogue
  marks `Dynamic` (`docs/KEYS.md`'s `labels` column): the same six
  publish-once scalars survive beside a sibling that keeps arriving, and go
  only when nothing in their domain has published to their label for
  `max_age`.
- **D61's uncatalogued cap** touches exactly one path: the `Entry::Vacant` arm
  of `Store::apply`, where a series is *created*. A push to a series that
  already exists pays **110 ns** and none of the cap; creating a catalogued
  labelled series costs **351 ns**, and creating an uncatalogued (plugin) one
  **447 ns** — the ≈ 96 ns difference is the catalogue miss that cannot
  short-circuit (**120 ns** measured alone, over ~100 rows) minus the
  catalogued lookup that can, plus the `Box<str>` domain key and the `HashMap`
  entry. Release, 2 000 series, `max_len = 16` so `Ring::new`'s 38 KB
  allocation does not swamp what is being measured. The design note guessed
  "tens of nanoseconds, a `HashMap` increment"; the increment is the cheap
  half and the catalogue walk is the rest. Note for the record that the cap
  calls `key::lookup` on **every** series creation, catalogued or not — the
  roadmap's "the cap looks up nothing unless a series is created for an
  uncatalogued name" is true of the `HashMap`, not of the lookup; the
  steady-state claim it stands for holds, because no series creation happens
  in a steady state.
- **Scan cost:** a full meters pass (`/proc/stat` + `meminfo` + `loadavg` +
  `uptime` + 3 PSI files + one `/proc` readdir + 32 `scaling_cur_freq` + 3
  k10temp inputs) is **0.29 ms mean, 0.35 ms worst** over 20 runs — 0.06 % of a
  core at the focused 500 ms cadence
  (`cargo test -p gridwatch-sources --release --test cpu -- --ignored`).
- **View cost:** the `cores` tier at 122×31 costs 0.051 ms to build and render
  and 0.079 ms to fingerprint for the render cache (release, 500 runs) — inside
  §13's 0.3 ms view budget, so `ui::view::fingerprint`'s note about a
  hand-rolled walker stays unclaimed.
**Arc 2b notes (2026-09-01).** Same protocol: release binary under a `script`
pty at 250×70, idle torch, no game, nothing focused (both 6x3 tiles at their
`procs` tiers, the gpu source on its 500 ms visible fast tier).

| gate | ceiling | measured | verdict |
|---|---|---|---|
| P11 | ≤ 6 ms/s NVML per device, per-class sum | **4.26 ms/s** = fast 0.04 + slow 2.55 + procs 1.67 over 30 s with process rows on (`live_nvml_pass_is_inside_p11`, release, idle card); the `sources` tile prints the same three numbers live, averaged over the 2 s process grid | ✓ — three passes to get here: **29 ms/s** with the device handle fetched per call (≈ 6 ms each, D49 §5); **4.78** over 10 s with one handle; then the 63 s fixture averaged **≈ 7.1** once fan seconds were included (slow 3.95 + procs 3.13) — the two PCIe fields became one batched call and the process rows moved to a 2 s grid (D49 §12) |
| P11 per call (`live_call_costs`, idle 5090 in P8) | — | utilization 0.65 ms · POWER_INSTANT 0.92 ms · PCIe counters 0.45 ms · fan % 0.90 / RPM 0.81 ms per fan · samples(Power) 0.65 ms · process lists 0.11 + 0.07 ms · process utilisation 2.20 ms; everything else < 12 µs | recorded — the digest's sub-µs fast tier was measured under load; an idle card in P8 is the slow case for the fast tier |
| P12 | never a GPU client | `nvidia-smi \| grep -c gridwatch` = **0** during the live run; own pid filtered from the v3 lists (test); NVML does start one internal thread in the process (`cuda0000280000b`, 0.00 % CPU, no context — the process table still shows nothing) | ✓ |
| P13 | process accounting only at `Detail::Table` | the poller's `Plan` gates the lists and `process_utilization_stats` on `detail >= Table` (`tiers_publish_their_own_keys_and_nothing_more`); `gpu.nvml_ms{procs}` reads 0 with no table tier visible | ✓ |
| P15 | pid-level scan ≤ 20 ms | unchanged from 2a (5.4 ms); the gpu tile's `demand` raises it with no htop tile visible, at the same cadence | ✓ |
| P17 | RSS ≤ 60 MB after 1 h, no growth | 20-minute run: **39.0 → 42.0 MB**, flat for the last three minutes. **One-hour run** (release, pty, idle, nothing focused): **37.1 MB** at 1 min → **40.1 MB** at 20 min → **40.2–40.3 MB** from minute 30 to minute 60 — the growth is the series rings filling to their 10-minute retention plus the allocator settling, then nothing (NVML maps ≈ 27 MB: 2a's process was 11.7 MB). The 24 h replay proxy is still owed — the 63 s fixture at 100× is a second, not a day | ✓ at one hour (40.3 of 60 MB, no growth between minute 20 and minute 60); the 24 h row is owed |
| P1, P5, P6, P8, P19 re-taken with both tables | as above | **1.50 %** of a core (render 0.28, gw-cpu 0.58, gw-gpu 0.67 — the gpu thread is the P11 time plus ioctl overhead, and this is the idle card's *expensive* fast tier) · 8 wake/s · 8.2 KB/s (up from 3.3: the gpu tile redraws every 500 ms and its braille band and power trace change cells) · 2.1 fps · p50 1.31 / p95 1.67 ms | ✓ — P1 has 0.5 pp of headroom on the idle box; the beside-a-game row (where the fast tier is cheap and the bytes are not) is owed with the game fixture |
| P9, P10, P4, P21 | Ptyxis Δ CPU / `pmon sm`; focus | — | **owed**: a pty is not Ptyxis; the run above was not in the real terminal |

- **The gpu thread's time is all *system* time** (0.67 % sys, 0.00 % user): NVML is ioctls into the driver, so P11's wall-clock sum understates the CPU it costs by roughly the ratio 0.67 % / 0.48 % — the kernel side of each call. P1 is the ceiling that catches it, and it holds.
**Arc 3a notes (2026-09-02).** The pins source's P14 evidence is `pins.read_ms`
(published every sample) and the `sources` tile's `N tx/s · read N ms` note;
the interval is clamped to ≥ 500 ms in `pins::clamp_interval`, so the source
cannot exceed 2 transactions/s by construction. **The live number is owed to a
human**: an agent never opens `/dev/i2c-*` (MACHINE.md), so the pass that
records it is run by hand, beside the root `astral-watch log` that is still
running on torch:

```
cargo test -p gridwatch-sources --release --test pins live_pins -- --ignored --nocapture
```

| gate | ceiling | measured | verdict |
|---|---|---|---|
| P14 | ≤ 2 i2c transactions/s, ≤ 1 % of a core | the interval floor (≥ 500 ms) bounds *reads* to 2/s; a **plausible** read is one block transaction, but a deeply idle GPU makes astral-watch re-probe bytewise (36 transactions) on every implausible reading — the review's finding — so the source backs off to 5 s after three misses while the chip answers zeros (≤ 7.2 tx/s worst, 0.4 tx/s steady) and returns to the cadence on the first good sample; the live pass gates on the mean read cost (block ≈ 4 ms vs bytewise ≈ 33 ms) as the transaction proxy. The read cost, the misses beside the root logger and the thread's CPU are **owed** — run the command above and paste its `P14:` line here | owed (a human's row); the idle case is bounded by construction |
| banner steady state | no redraws when no alert is active; one row per second while a Crit alert is active | the banner is drawn inside `draw_frame` from the store's active set — no timer, no extra frame cause; the pulse rides the 1 Hz heartbeat frame that fires anyway | ✓ by construction; the `redraw_heartbeat` counter in `--stats-log` is the check |

- **Still owed by a human on torch** (all need the real terminal, not a pty):
  P4 and P21 (focus events), P9/P10 (Ptyxis Δ CPU and `pmon sm`), and every row
  re-taken **at Matt's actual window size** (D42's open `stty size` item) **with
  the game running** — P1's and P6's ceilings are both specified beside a game.

**Arc 13 notes (2026-09-07, D63).** `[store] history` is live, so P17's requirement row now has two operating points rather than one, and the store's bytes are readable at last (`Store::footprint()` on the `F12` HUD; `store_bytes` in `--stats-log`, sampled at the same 1 Hz tick as the rest of the object, never per frame).

**P18 is stated, not re-measured.** `Retention::for_history(600 s)` is field-identical to `Retention::default()` — `max_len` 2 400, `max_age` 600 s, `max_uncatalogued` 512 — and a store test pins it, so every arc-10b/11b sweep and cap number above stands unchanged at the shipped `history = "10m"`. Nothing about the sweep's cadence moves under a different history either: `sweep_every` reads `max_age` alone.

**P17 at the `1h` ceiling is owed to Matt.** The row wants a 60-minute **release** run under a pty at 250×70 on an idle torch with `history = "1h"` and `--stats-log`: RSS at 1/20/40/60 min and `store_bytes` at the end, beside the arc-3b/5a rows, plus a 20-minute run at the default confirming `store_bytes` under 6 MB. What follows is **not** that row — it is a pair of short **debug** runs whose only purpose is to show the plumbing works and to size the preallocation question in `BACKLOG.md`.

| what | `history = "10m"` | `history = "1h"` |
|---|---|---|
| `store_bytes` after 110 s (from the app's own `--stats-log`) | **572 480** | **572 512** |
| `VmData` (heap reserved) after 40 s | **75 648 kB** | **82 256 kB** |
| `VmRSS` after 40 s | **69 408 kB** | **69 852 kB** |

Three things to read from it. (1) **The two histories hold the same points at 110 s**, which is what they should: neither retention has bitten, so the byte figure is identical to within one sample and the plumbing is doing nothing clever. (2) **The preallocation costs address space, not resident memory, until the points arrive.** `Ring::new` reserves `min(max_len, 4096)` slots, so `1h` reserves **6 608 kB** more heap than `10m` — and only **444 kB** of that is resident after 40 s, because untouched pages of a reserved `VecDeque` never fault in. D63's "≈ 10 MB before a point arrives … lands RSS near 50 of the 60 MB budget" is therefore the *reservation*, not the RSS; what the 60-minute run will actually show is the points arriving. (3) These are **debug** binaries (p50 14.9 ms a frame, ten times the release figures in the rows above), so the absolute RSS is not comparable to any row here — only the delta between the two runs is, since everything else about them is identical.

*Recorded rather than reported: an earlier pass at this measurement read RSS from the wrong pid — `pgrep -f "gridwatch run --stats-log …"` matches the `script` wrapper as well as the binary, and `head -1` takes the wrapper. Its 7.5 MB figures are discarded. The `store_bytes` numbers above come from gridwatch's own stats log and were never affected.*

---

## Arc 14 (2026-09-09, D64) — the disk source and tile

Release build, pty at 250×70, torch idle (**no game**), the shipped Overview
with the `disk` tile in the clock's slot, `[sources.pins] source = "exporter"`
so nothing opens `/dev/i2c-*` (CLAUDE.md forbids an agent that bus; the pins
source reports `Unavailable` and costs 0.1 wake-ups/s instead of 2).

| gate | ceiling | measured | verdict |
|---|---|---|---|
| **P23** (new) | pass ≤ 1 ms, ≤ 0.2 % of a core at 1 s | **0.28–0.35 ms** steady state (`disk.scan_ms`, release, torch's 64-line/4 205-byte file) → **0.032 %** of one core at 1 s. The **first** pass is **6.5–7.0 ms**: it classifies all 64 diskstats names through sysfs, once, and never again — a device is classified on first sight and forgotten when it leaves diskstats, so a second pass 200 ms later costs the steady-state figure. Ceiling for comparison: the Python upper bound in the brief was 69.5 µs to read *and* parse, so the Rust pass is dominated by neither | ✓ |
| P5 | ≤ 40 wake-ups/s | **34.0 /s** live at 250×70 with the shipped Overview, the disk tile visible and a **silent** sink — the row's own condition. `gw-sensors` 13.5 · render 4.0 · `gw-watch` 4.0 · `gw-gpu` 3.2 · `gw-mpris` 2.2 · **`gw-audio` 2.0** · `gw-cpu` 2.0 · `gw-net-probe` 1.5 · `gw-net` 1.0 · **`gw-disk` 0.50** · `gw-pins` 0.1. The whole disk tile costs half a wake-up a second | ✓ |
| P1 | ≤ 2 % of one core, silent | **1.75 %** over 40 s, live at 250×70 with the shipped Overview and the disk tile visible, `pw-dump` showing **zero running nodes** before and after the run. The arc's first pass recorded **2.57 %** and signed it off against **P2** on the grounds that "the shipped Overview carries the `viz` tile, so the audio DSP is live at 30 fps" — the same retracted premise as the 69.6 /s wake-up figure (see the P5 note): what raises the DSP is *sound*, not visibility, so the applicable ceiling was always P1's and 2.57 % was over it. Re-taken with the sink confirmed idle, it is inside | ✓ |
| P6 | ≤ 25 kB/s | **10.6 kB/s** (Δ`wchar` over 60 s, the stats log's own 13 kB subtracted; the recorded typescript agrees at ≈ 12 kB/s including startup) | ✓ — but the HUD cross-check disagrees, below |
| P8 | ≈ 2 frames/s on the Overview, every frame caused | **2.10 /s**; 126 data-caused redraws, 0 animated, 0 heartbeat over the window | ✓ |
| P19 | p95 ≤ 8 ms, mean ≤ 3 ms | p50 **1.63 ms**, p95 **2.04 ms** | ✓ |
| P17 | RSS ≤ 60 MB | **47.8 MB**; `store_bytes` **340 kB** held. Arithmetic for the disk keys alone: 3 drives × 9 scalars × 38 400 B ≈ **1.0 MB** reserved; `partitions = true` on torch (12 devices) ≈ **4.2 MB**; the 16-device cap ≈ **5.5 MB**; **all 64 diskstats lines ≈ 22 MB** — which is the whole reason the device rule and the cap exist | ✓ |
| P13, P15 | unchanged | asserted rather than measured: `no_tier_ever_raises_detail` walks every `disk` tier and requires `Detail::Meters`, so the pid-level scan and the gated columns are untouched by this arc | ✓ |
| cross-check | `iostat -x` beside the tile | idle: both read zero on `nvme1n1` and `nvme2n1` and the same trickle on `nvme0n1`. Loaded read side: one bounded `dd … iflag=direct bs=1M count=512` — see below. **The write-side row is Matt's**: an agent does not write half a gigabyte to his boot drive | partial |

**P5 holds, and the disk tile costs half a wake-up a second.** The arc's first pass recorded this row as **over** at 69.6 /s and escalated two causes; re-measured during the review, **both readings were of the wrong thing** and the row is inside its ceiling at **34.0 /s**.

1. **The 69.6 /s pass had sound playing.** P5's row is titled *"Overview, silent audio"* and its derivation budgets `audio idle 2`. Under silence the DSP does exactly that: `gw-audio` books **2.00 /s**, because the source publishes at `fps` only while the input is above the floor and at 2 Hz otherwise (D55, and arc 5a's P16 row measured the same path at 0.0 % of a core). The first pass saw `gw-audio` at 37–40 /s, which is the 30 fps *sound-playing* path — a correct number for a condition this row excludes. The claim that "the shipped Overview places the `viz` tile, so the DSP runs at 30 fps whenever page 1 is on screen" is wrong: what raises it is sound, not visibility. **A demo run is not a substitute either** — `--demo`'s audio synth is never silent, and measured the same way it gives **72.9 /s** with `gw-audio` at 30.4 /s. Anyone re-taking this row must confirm the sink is idle first.
2. **`gw-sensors` at 13.5 /s on a 1 s cadence is real, and it is the instrument.** Σ Δ`voluntary_ctxt_switches` — the metric this file's own protocol prescribes (§ measurement, step 2) — counts **every** voluntary yield, including a blocking sysfs read, not only a timer wake. The sensors source reads ~40 hwmon files a second, so it books ~13 switches per pass. The number is true; what it measures is not only what "wake-ups per second" suggests, and the same overcount applies to every source that reads several files per tick. The budget's `sensors 1` is counting ticks. **Recorded, not fixed** — reconciling the derivation with the instrument is a `PERFORMANCE.md` question of its own, and nothing about it changes what the program does.

*Method, so this is reproducible: release binary under a `script` pty at 250×70, `XDG_CONFIG_HOME` pointing at the shipped default plus `[sources.pins] source = "exporter"` (an agent must not open `/dev/i2c-*`), Σ Δ`voluntary_ctxt_switches` over `/proc/<pid>/task/*/status` across a 10 s window starting 10 s after launch. Resolve the pid with `pgrep -x gridwatch` — `pgrep -f` matches the `script` wrapper, which is how an earlier pass in this project reported a wrapper's RSS as the program's.*

Neither is arc 14's to fix, and neither is fixed here.

**The P6 HUD cross-check disagrees by 2×** — Δ`wchar` 10.6 kB/s against the
stats log's own `bytes` counter at 4.9 kB/s over the same window, where P6
requires them to agree within 5 %. The recorded typescript (839 472 B over
70 s) sides with `wchar`. Nothing in arc 14 touches the byte accounting, and
the row is inside its ceiling by either number, so this is **recorded, not
chased**: it wants a run with `--stats-log` off and the HUD read by eye.

**The read-side cross-check, and what it says about `BUSY`.** One bounded
`dd if=<an existing 1 GB file> of=/dev/null iflag=direct bs=1M count=512` moved
537 MB in 0.082 s. `iostat -x 1` caught it on `nvme0n1` in one sample:

```
Device      r/s     rkB/s  r_await rareq-sz  aqu-sz  %util
nvme0n1  4609.00 524292.00    0.08   113.75    0.38   7.30
```

**524 MB/s at 7.3 % util** — the same argument D64 makes, from the other
direction: field 13 is the share of wall time the queue was non-empty, and a
drive that answers in 80 µs empties its queue between requests however much
work it is doing. `aqu-sz 0.38` is the honest number, and it is what the tile
draws as `Q`. This is the **read** side only, run **once** (an agent does not
benchmark a disk beside a game); the tile's own row under that load was not
captured cleanly in the same second, so **the loaded tile-versus-`iostat`
comparison is owed to Matt** along with the write side.

**Still owed to Matt on this arc:** the write-side `iostat` cross-check under
real load, every row above re-taken beside the game, and P5 with the pins
source on its real backend (`auto`, which opens `/dev/i2c-*`).
