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
| P11 | NVML: call time per second per device | **≤ 6 ms/s** — sum listed so the gate is auditable: fast tier 2 × ≈ 20 µs; slow tier ≈ 2.3 ms (`samples(Power)` ≈ 0.65 while a gpu tile is visible — D49, one batched PCIe-counter call 0.45, memory/enc/dec µs, fans %/RPM 6 × 0.85 ms ÷ 5 s ≈ 1.0); process rows ≈ 2.4 per pass (lists 0.2, utilisation 2.2) only at `Detail::Table` and on a 2 s grid (D49) → ≈ 1.2 ms/s; ≈ 4–5 ms/s with a `procs` tile visible on an idle card; never `pcie_throughput`; never a `NotSupported` field twice; `nvidia-smi` never spawned while NVML works | `tracing` span sums on the gpu thread, shown per call class in the `sources` tile |
| P12 | NVML: gridwatch is **never a GPU client** — NVML creates no context (verified: `nvidia-smi` and a running `nvtop` are absent from the process table) | 0 entries | `nvidia-smi \| grep -c gridwatch` = 0 (the full table lists graphics *and* compute clients; `--query-compute-apps` sees compute only), cross-checked by the gpu source filtering its own pid out of the v3 lists |
| P13 | Per-process GPU accounting only when shown | v3 lists and `process_utilization_stats` are called only while a gpu tile whose `demand(tier)` is `Table` or richer is visible | `sources` tile shows each source's demand level and detail |
| P14 | i2c (pins source) | **≤ 2 transactions/s** (500 ms block read, one transaction per sample), **≤ 1 %** of one core (the digest measured the root logger at 1.0 % on the older 36-transaction path; the block path is measured in arc 3 and recorded below); the interval is never configured below 500 ms | source transaction counter, `pidstat` |
| P15 | `/proc` scan | pid-level scan (`stat`, `statm`, dir `st_uid`, `cmdline` on first sight) **≤ 20 ms** wall per pass and **≤ 1 %** of one core amortised at its 3 s grid cadence (five file kinds measured 20 ms today for 665 pids; two kinds 10 ms); the `task/` walk (+30 ms) and htop's gated files (`smaps_rollup` alone 130 ms) only at `Detail::Columns` — a `full` tier, zoomed or `view = "full"`, with that column on | `tracing` span on the cpu thread, shown in the `sources` tile |
| P16 | Audio capture | `pw-record` child **≤ 1.5 %** at its ≈ 94 chunk writes/s (4096-byte chunks every ~10.6 ms regardless of `--latency`), DSP thread `gw-audio` (the io pump `gw-audio-io` and the stderr reader `gw-audio-err` beside it) **≤ 2 %** at 30 fps; PipeWire quantum **unchanged** at the default `latency = 1024` (no-game row only — the game pins the graph at 256); child killed **≤ 10 s** after the last visible audio tile | `pidstat -u -t -p <gridwatch> 1 60` (named threads), `pidstat -p <pw-record pid>`, `pw-top -b -n 1` / `pw-metadata -n settings 0` |
| P17 | Memory | RSS **≤ 60 MB** after 1 h (NVML maps ~20 MB — nvtop sits at 31 MB); store ≤ 32 MB (retention-capped); **no growth** between hour 1 and hour 24 (24 h replay at 100× speed) | `pidstat -r`, `/proc/<pid>/status` VmRSS |
| P18 | Startup | first frame **≤ 300 ms** (placeholder tiles); every source live **≤ 2 s**; NVML init, `detect_bus`, `pw-record` spawn and D-Bus connect never on the render thread | `--stats` prints both timestamps |
| P19 | Frame cost | draw + write **p95 ≤ 8 ms** at 250×70; **mean ≤ 3 ms** at the Overview's ≈ 2 frames/s and **≤ 1.3 ms** at 30 fps (render cache: only the animated tile re-renders; the whole-frame diff is ~0.3 ms); missed frames **< 1 %** | `F12` HUD (p50/p95, changed cells, bytes) |
| P20 | Effects (arc 4) | ≤ `budget_ms` (4 ms) per frame, area-scoped, ≤ 600 ms per event; the repeating alert pulse alone at 8 fps; `--no-effects` honours P6/P8 exactly | `F12` HUD effect column, `fx_us` in `--stats-log` |
| P22 | Plugin host (arc 8b) | with a plugin rendering at 1 Hz the host's two threads (`gw-plugins`, `gw-plugin-<id>`) are **below `pidstat`'s resolution** and there is **no measurable delta against the same page with no plugin configured** — that, rather than a number nobody can reproduce, is the gate; a plugin that floods costs the host **no more** — the reader takes at most `MAX_MSGS_PER_SEC` (500) messages/s from one plugin, so its pipe fills and the child blocks rather than either process spinning; the inbound queue is 64 deep and drops the oldest; a plugin over 50 % of a core for 10 s is stopped; at most 256 distinct metric names per plugin. Startup: `hello_ms` (2 s default) is added to P18's first frame **only when a plugin is configured**, and every plugin is waited on together | `pidstat -u -t` (the `gw-plugins` and `gw-plugin-<id>` threads are named), `pidstat -p <child>`, task-summed context switches |
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
| `theme/load retrowave` | parse + build a theme, WCAG gate included — what every `t` press pays | **26.5 µs** |

What they say about the ceilings above: P19 allows **8 ms p95** for a frame, and a *completely uncached* Overview costs 0.51 ms — which is why the render cache buys what it does, and why the live p50 is 0.04 ms (arc 8a's row: most frames are a blit). `Store::apply` at 2.5 µs means the data path is not the cost of anything; at the Overview's ~40 batches a second it is 0.1 ms of CPU per second. And `resample` costing *less* at more buckets is not a mistake in the table — the work is per point, and the per-bucket aggregation gets cheaper as the buckets get smaller.

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
